//! Settings → Storage: the places games download and install to, each with how much room it
//! has; new ones can be added (an SSD, a second drive), and any becomes where new games go.
//!
//! A place added by its folder P keeps SwarmPlay's things in P/SwarmPlay/Downloads and
//! P/SwarmPlay/Games (the latter is also a game folder, so what's installed there is found).
//!
//! As on a console, each drive shows its size, a bar split by what fills it (games, downloads,
//! screenshots, everything else) and its free space; the sizes are counted in the background
//! when Storage is opened (at most once a minute).

use crate::app::*;
use crate::settings::SId;
use crate::{util, SettingData};
use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tr::tr;

/// What fills a drive, counted: games, downloads, screenshots (bytes on the disk).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    pub games: u64,
    pub downloads: u64,
    pub media: u64,
}

/// The last count (when, and per drive), and whether one is under way.
static USAGE: Mutex<Option<(f64, HashMap<u64, Usage>)>> = Mutex::new(None);
static COUNTING: AtomicBool = AtomicBool::new(false);
const RECOUNT_SECS: f64 = 60.0;

/// The legend under a drive's bar: its colour (in the bar too), as a console's.
pub const PARTS: [&str; 4] = ["Games", "Downloads", "Media gallery", "Other"];

/// A line of the legend (PARTS' name), as shown.
pub fn part_label(k: usize) -> String {
    match PARTS[k.min(PARTS.len() - 1)] {
        "Games" => tr!("Games"),
        "Downloads" => tr!("Downloads"),
        "Media gallery" => tr!("Media gallery"),
        _ => tr!("Other"),
    }
}

/// Bytes `path` takes on the disk, not crossing into other drives nor following links.
pub fn du(path: &Path) -> u64 {
    let Ok(top) = std::fs::symlink_metadata(path) else { return 0 };
    let dev = top.dev();
    let mut total = 0u64;
    let mut todo = vec![path.to_path_buf()];
    while let Some(dir) = todo.pop() {
        let Ok(meta) = std::fs::symlink_metadata(&dir) else { continue };
        if meta.dev() != dev { continue; }
        total += meta.blocks() * 512;
        if !meta.is_dir() { continue; }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() { todo.push(e.path()); }
            else if let Ok(m) = e.metadata() { if m.dev() == dev { total += m.blocks() * 512; } }
        }
    }
    total
}

/// The paths with none of their parents also there (nothing counted twice).
fn outermost(mut paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths.sort();
    paths.dedup();
    let all = paths.clone();
    paths.retain(|p| !all.iter().any(|q| q != p && p.starts_with(q)));
    paths
}

/// Where screenshots go (src/overlay_ui.rs).
pub fn media_dir() -> PathBuf {
    std::env::var_os("XDG_PICTURES_DIR").map(PathBuf::from).unwrap_or_else(|| util::expand_home("~/Pictures")).join("SwarmPlay")
}

fn dev_of(path: &Path) -> u64 {
    existing(path).and_then(|p| std::fs::metadata(p).ok()).map(|m| m.dev()).unwrap_or(0)
}

/// One place: where its downloads and installs go, and how much room its drive has.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub name: String,
    pub download: String,
    pub install: String,
    pub free: u64,
    pub total: u64,
    pub default: bool,
    /// Added in Storage (not the built-in place): it can be taken off the list.
    pub added: Option<String>,
    pub dev: u64,
}

/// The nearest folder of `path` that exists.
fn existing(path: &Path) -> Option<&Path> {
    path.ancestors().find(|p| p.is_dir())
}

/// Free and total bytes on the drive holding `path`.
fn space(path: &Path) -> Option<(u64, u64)> {
    use std::ffi::CString;
    let dir = existing(path)?;
    let c = CString::new(dir.as_os_str().as_encoded_bytes()).ok()?;
    let mut st = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(c.as_ptr(), st.as_mut_ptr()) } != 0 { return None; }
    let st = unsafe { st.assume_init() };
    let unit = st.f_frsize as u64;
    Some((st.f_bavail as u64 * unit, st.f_blocks as u64 * unit))
}

/// Where the drive holding `path` is mounted (the longest mount point above it).
fn mount_point(path: &Path) -> String {
    let path = existing(path).and_then(|p| p.canonicalize().ok()).unwrap_or_else(|| path.to_path_buf());
    let mounts = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    mounts.lines().filter_map(|l| l.split(' ').nth(4)).map(|m| m.replace("\\040", " "))
        .filter(|m| path.starts_with(m)).max_by_key(|m| m.len()).unwrap_or_else(|| "/".into())
}

/// A drive's name for people: the system's own drive, or the removable one's label.
pub fn drive_name(mount: &str) -> String {
    let internal = ["/", "/var", "/home", "/var/home", "/sysroot", "/usr"];
    if internal.contains(&mount) { return tr!("Internal storage"); }
    Path::new(mount).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| mount.to_string())
}

/// Whether the drive `path` is on is there: a path where extra drives get mounted (/run/media,
/// /mnt…) is only on one if a drive is mounted above it. Otherwise writing there would fill the
/// system drive (or a folder in memory) under the name of a drive that's gone.
pub fn drive_connected(path: &Path) -> bool {
    let on_drive = |p: &str| crate::folder_pick::MOUNTS.iter().any(|m| p.starts_with(m));
    !on_drive(&path.to_string_lossy()) || on_drive(&mount_point(path))
}

/// Where downloads can be: the folder they go to now, the built-in one, each place added in
/// Storage (its SwarmPlay/Downloads and its top) and each drive mounted now (the same, at its
/// top). A download moved between them, or on a drive mounted somewhere else since, is found
/// there again (downloads::find_moved).
pub fn download_bases(cfg: &crate::config::Config) -> Vec<PathBuf> {
    let mounts = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    let tops = cfg.storage.iter().map(|f| util::expand_home(f))
        .chain(crate::folder_pick::mounted(&mounts).into_iter().map(|(root, _)| root));
    let mut bases = vec![util::expand_home(&cfg.download_dir), util::expand_home("~/Downloads/Azrak")];
    for top in tops {
        bases.push(PathBuf::from(under(&top.to_string_lossy()).0));
        bases.push(top);
    }
    let mut seen = std::collections::HashSet::new();
    bases.retain(|b| b.is_dir() && seen.insert(b.clone()));
    bases
}

/// A place added by its folder: SwarmPlay's downloads and games inside it.
fn under(folder: &str) -> (String, String) {
    let base = Path::new(folder.trim_end_matches('/')).join("SwarmPlay");
    (base.join("Downloads").to_string_lossy().into_owned(), base.join("Games").to_string_lossy().into_owned())
}

/// A size as a console writes it: 0 kB, 512 MB, 2.02 GB, 260.5 GB, 1.2 TB (with a decimal comma
/// in Portuguese).
pub fn size(bytes: u64) -> String {
    use crate::i18n::num;
    let k = bytes as f64 / 1024.0;
    let (m, g) = (k / 1024.0, k / 1024.0 / 1024.0);
    if g >= 1024.0 { format!("{} TB", num(g / 1024.0, 1)) }
    else if g >= 100.0 { format!("{} GB", num(g, 1)) }
    else if g >= 1.0 { format!("{} GB", num(g, 2)) }
    else if m >= 1.0 { format!("{} MB", num(m, 0)) }
    else { format!("{} kB", num(k, 0)) }
}

/// The bar's parts, as shares of the whole drive: games, downloads, media, other (what's used
/// but none of those).
pub fn shares(u: &Usage, free: u64, total: u64) -> [f32; 4] {
    if total == 0 { return [0.0; 4]; }
    let used = total.saturating_sub(free);
    let ours = (u.games + u.downloads + u.media).min(used);
    let other = used - ours;
    let f = |b: u64| (b as f64 / total as f64) as f32;
    [f(u.games.min(used)), f(u.downloads.min(used.saturating_sub(u.games))), f(u.media.min(used.saturating_sub(u.games + u.downloads))), f(other)]
}

impl App {
    /// Every place, the one new games go to first; one per drive.
    pub fn storage_places(&self) -> Vec<Place> {
        let cfg = self.cfg.lock().unwrap().clone();
        let mut pairs = vec![(cfg.download_dir.clone(), cfg.install_dir.clone(), None)];
        pairs.push(("~/Downloads/Azrak".into(), "~/Games/Azrak".into(), None));
        pairs.extend(cfg.storage.iter().map(|f| { let (d, i) = under(f); (d, i, Some(f.clone())) }));
        let mut places: Vec<Place> = Vec::new();
        for (download, install, added) in pairs {
            let path = util::expand_home(&install);
            let dev = dev_of(&path);
            if let Some(p) = places.iter_mut().find(|p| p.dev == dev) {
                // The one new games go to stands for its drive; an added folder on it can still go.
                if p.added.is_none() && added.is_some() && !p.default { p.added = added; }
                continue;
            }
            let (free, total) = space(&path).unwrap_or((0, 0));
            let default = install == cfg.install_dir;
            let added = added.filter(|_| !default);
            places.push(Place { name: drive_name(&mount_point(&path)), download, install, free, total, default, added, dev });
        }
        places
    }

    /// Count what fills each drive, in the background, if the last count is old.
    fn storage_count(&self, places: &[Place]) {
        let fresh = USAGE.lock().unwrap().as_ref().is_some_and(|(at, _)| util::now_secs() - at < RECOUNT_SECS);
        if fresh || COUNTING.swap(true, Ordering::SeqCst) { return; }
        let cfg = self.cfg.lock().unwrap().clone();
        let games: Vec<PathBuf> = self.locals.iter().map(|l| l.l.path.clone()).collect();
        let mut downloads: Vec<PathBuf> = places.iter().map(|p| util::expand_home(&p.download)).collect();
        downloads.push(util::expand_home(&cfg.download_dir));
        std::thread::Builder::new().name("storage-count".into()).spawn(move || {
            let mut by: HashMap<u64, Usage> = HashMap::new();
            for g in outermost(games) { let d = dev_of(&g); by.entry(d).or_default().games += du(&g); }
            for d in outermost(downloads) { let dev = dev_of(&d); by.entry(dev).or_default().downloads += du(&d); }
            let m = media_dir();
            by.entry(dev_of(&m)).or_default().media += du(&m);
            *USAGE.lock().unwrap() = Some((util::now_secs(), by));
            COUNTING.store(false, Ordering::SeqCst);
            let _ = slint::invoke_from_event_loop(|| crate::app::with_app(|app| {
                if app.overlay == Overlay::Settings { app.refresh_settings(); }
            }));
        }).ok();
    }

    /// Something was added or removed: the next look counts again.
    pub fn storage_recount() {
        if let Some((at, _)) = USAGE.lock().unwrap().as_mut() { *at = 0.0; }
    }

    pub fn storage_rows(&self) -> (Vec<(SId, SettingData)>, Vec<Place>) {
        let cfg = self.cfg.lock().unwrap().clone();
        let row = |kind: i32, label: &str| SettingData { kind, label: label.into(), ..Default::default() };
        let places = self.storage_places();
        if self.overlay == Overlay::Settings { self.storage_count(&places); }
        let usage = USAGE.lock().unwrap().as_ref().map(|(_, m)| m.clone());
        let mut rows = Vec::new();
        for (i, p) in places.iter().enumerate() {
            // The drive: its name, its size, the bar, its free space.
            let mut r = row(5, &p.name);
            let u = usage.as_ref().map(|m| m.get(&p.dev).copied().unwrap_or_default());
            if p.total > 0 {
                r.value = tr!("Size: {}", size(p.total)).into();
                r.value2 = tr!("Free space: {}", size(p.free)).into();
                r.value_kind = if p.free < 50 << 30 { 2 } else { 0 };
                let sh = match &u {
                    Some(u) => shares(u, p.free, p.total),
                    // Not counted yet: all of it "other" for now.
                    None => [0.0, 0.0, 0.0, 1.0 - p.free as f32 / p.total as f32],
                };
                r.segs = crate::app::model(sh.to_vec());
            } else {
                r.value = tr!("Not connected").into();
                r.value_kind = 2;
            }
            (r.hint, r.hint_kind) = if p.default {
                (tr!("New games go here · {}", util::display_path(&p.install)).into(), 1)
            } else {
                (tr!("Press to put new games here · {}", util::display_path(&p.install)).into(), 0)
            };
            r.removable = p.added.is_some();
            rows.push((SId::StoragePlace(i as u16), r));
            if p.total == 0 { continue; }
            // What fills it, each with its colour.
            let used = p.total.saturating_sub(p.free);
            for k in 0..PARTS.len() {
                let mut r = row(6, &part_label(k));
                r.dot = k as i32 + 1;
                r.value = match &u {
                    None => tr!("Counting…").into(),
                    Some(u) => size(match k { 0 => u.games, 1 => u.downloads, 2 => u.media, _ => used.saturating_sub(u.games + u.downloads + u.media) }).into(),
                };
                rows.push((SId::StoragePart(i as u16, k as u8), r));
            }
        }
        let mut r = row(4, &tr!("Add a place"));
        r.hint = tr!("Another drive (an SSD, a USB drive) or a folder on one: games can download and install there").into();
        rows.push((SId::StorageAdd, r));
        let mut r = row(4, &tr!("Install new games to"));
        r.value = util::display_path(&cfg.install_dir).into();
        r.hint = tr!("Downloaded games are set up here when you press Install").into();
        rows.push((SId::InstallDir, r));
        let mut r = row(4, &tr!("Save downloads to"));
        r.value = util::display_path(&cfg.download_dir).into();
        r.hint = tr!("Each download gets its own folder").into();
        rows.push((SId::DownloadDir, r));
        (rows, places)
    }

    /// A line of the legend pressed: where those things are.
    pub fn storage_part(&mut self, place: usize, part: usize) {
        let id = SId::StoragePart(place as u16, part as u8);
        match part {
            0 => {
                crate::audio::play(crate::audio::Sound::Select);
                while self.overlay != Overlay::None { let before = self.overlay; self.back(); if self.overlay == before { break; } }
                self.switch_view(1, true);
            }
            1 => self.open_downloads(None),
            2 => {
                let dir = media_dir();
                if dir.is_dir() {
                    crate::audio::play(crate::audio::Sound::Select);
                    let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
                } else {
                    self.note_setting(id, &tr!("No screenshots yet · Share takes one in a game"), 0);
                }
            }
            _ => self.note_setting(id, &tr!("Everything else on this drive: the system, apps, your own files"), 0),
        }
    }

    /// □ on a place that was added: off the list (nothing on it is deleted).
    pub fn storage_remove(&mut self, i: usize) {
        let Some(place) = self.storage_places().get(i).cloned() else { return };
        let Some(folder) = place.added else {
            return self.note_setting(SId::StoragePlace(i as u16), &tr!("This one stays: it's where SwarmPlay started"), 0);
        };
        {
            let mut c = self.cfg.lock().unwrap();
            c.storage.retain(|f| *f != folder);
            c.save();
        }
        crate::audio::play(crate::audio::Sound::Back);
        self.toast(&tr!("Taken off the list"), &tr!("{} · its games and downloads are still there", place.name), "disk", crate::toast::DONE);
        Self::storage_recount();
        self.refresh_settings();
    }

    /// Press on a place: new games go there from now on.
    pub fn storage_use(&mut self, i: usize) {
        let Some(place) = self.storage_places().get(i).cloned() else { return };
        if place.default {
            return self.note_setting(SId::StoragePlace(i as u16), &tr!("New games already go here"), 0);
        }
        for dir in [&place.download, &place.install] {
            if let Err(e) = std::fs::create_dir_all(util::expand_home(dir)) {
                return self.note_setting(SId::StoragePlace(i as u16), &tr!("Can't use it: {e}", e), 2);
            }
        }
        {
            let mut c = self.cfg.lock().unwrap();
            c.download_dir = place.download.clone();
            c.install_dir = place.install.clone();
            if !c.game_dir_paths().iter().any(|d| *d == util::expand_home(&place.install)) { c.game_dirs.push(place.install.clone()); }
            c.save();
        }
        crate::audio::play(crate::audio::Sound::Select);
        self.toast(&tr!("New games go here now"), &place.name, "disk", crate::toast::DONE);
        self.refresh_settings();
    }

    /// "Add a place": the folder typed.
    pub fn storage_add(&mut self, text: &str) {
        let folder = text.trim().trim_end_matches('/').to_string();
        let path: PathBuf = util::expand_home(&folder);
        if folder.is_empty() { return; }
        if !path.is_dir() {
            return self.note_setting(SId::StorageAdd, &tr!("Not added: no folder there (is the drive connected?)"), 2);
        }
        let (download, install) = under(&folder);
        if let Err(e) = std::fs::create_dir_all(util::expand_home(&install)).and_then(|_| std::fs::create_dir_all(util::expand_home(&download))) {
            return self.note_setting(SId::StorageAdd, &tr!("Not added: can't write there ({e})", e), 2);
        }
        {
            let mut c = self.cfg.lock().unwrap();
            if !c.storage.contains(&folder) { c.storage.push(folder); }
            if !c.game_dir_paths().iter().any(|d| *d == util::expand_home(&install)) { c.game_dirs.push(install); }
            c.save();
        }
        self.note_setting(SId::StorageAdd, &tr!("Added · press it in the list to put new games there"), 1);
        Self::storage_recount();
        self.refresh_settings();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_drives_and_places_folders_under_swarmplay() {
        assert_eq!(drive_name("/"), "Internal storage");
        assert_eq!(drive_name("/var/home"), "Internal storage");
        assert_eq!(drive_name("/run/media/m/Games SSD"), "Games SSD");
        assert_eq!(under("/run/media/m/SSD/"), ("/run/media/m/SSD/SwarmPlay/Downloads".into(), "/run/media/m/SSD/SwarmPlay/Games".into()));
    }

    #[test]
    fn every_part_has_a_label() {
        for (k, p) in PARTS.iter().enumerate() { assert_eq!(part_label(k), *p); }
    }

    #[test]
    fn sizes_read_as_on_a_console() {
        assert_eq!(size(0), "0 kB");
        assert_eq!(size(512 << 20), "512 MB");
        assert_eq!(size((2.02 * (1u64 << 30) as f64) as u64), "2.02 GB");
        assert_eq!(size((260.5 * (1u64 << 30) as f64) as u64), "260.5 GB");
    }

    #[test]
    fn the_bar_splits_what_is_used() {
        let g = 1u64 << 30;
        let u = Usage { games: 30 * g, downloads: 10 * g, media: 0 };
        let s = shares(&u, 40 * g, 100 * g);
        assert_eq!(s, [0.3, 0.1, 0.0, 0.2]);
        // Counted more than is used (a drive filled since): never past the used part.
        let s = shares(&Usage { games: 90 * g, downloads: 0, media: 0 }, 40 * g, 100 * g);
        assert_eq!(s[0], 0.6);
        assert_eq!(s[3], 0.0);
    }

    #[test]
    fn counts_nothing_twice() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(outermost(vec![p("/a/b"), p("/a"), p("/c"), p("/a/b/c")]), vec![p("/a"), p("/c")]);
    }

    #[test]
    fn du_counts_a_folder() {
        let dir = std::env::temp_dir().join(format!("swarmplay-du-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/f"), vec![1u8; 100_000]).unwrap();
        assert!(du(&dir) >= 100_000);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
