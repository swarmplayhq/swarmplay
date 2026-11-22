//! Choosing a drive, then a folder on it, with a controller, as a console's storage screens:
//! no paths to type. Opened from Settings → Storage for "Add a place", "Install new games to"
//! and "Save downloads to".
//!
//! First the drives: the home folder, every drive mounted where removable and extra drives go
//! (/run/media, /media, /mnt, /var/mnt), and drives plugged in but not mounted yet (lsblk),
//! which ✕ mounts (udisksctl). Then the folders on the chosen one: "Use this folder" first, then
//! its folders to go into; ○ goes up, and back to the drives from a drive's top.

use crate::app::*;
use crate::audio::{self, Sound};
use crate::settings::SId;
use crate::{util, PickRow};
use std::path::{Path, PathBuf};
use tr::tr;

/// What the folder chosen is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickFor {
    Place,
    Install,
    Download,
}

impl PickFor {
    fn title(self) -> String {
        match self { PickFor::Place => tr!("Add a place"), PickFor::Install => tr!("Install new games to"), PickFor::Download => tr!("Save downloads to") }
    }
    fn row(self) -> SId {
        match self { PickFor::Place => SId::StorageAdd, PickFor::Install => SId::InstallDir, PickFor::Download => SId::DownloadDir }
    }
}

/// A drive on the first list.
#[derive(Clone, Debug, PartialEq)]
pub struct Drive {
    pub name: String,
    /// Where it's mounted (None: plugged in, not mounted yet).
    pub root: Option<PathBuf>,
    /// The device, to mount it (/dev/sdb1).
    pub device: String,
    pub free: u64,
    pub total: u64,
    pub removable: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum Entry {
    Drive(usize),
    Use,
    Folder(PathBuf),
}

pub struct Pick {
    pub purpose: PickFor,
    drives: Vec<Drive>,
    /// The drive gone into and the folder shown (None: the drives).
    at: Option<(usize, PathBuf)>,
    entries: Vec<Entry>,
    /// What went wrong last (can't write there, couldn't mount).
    note: String,
    /// A drive being mounted.
    busy: Option<usize>,
}

/// Filesystems a drive for games can have.
const FS: [&str; 9] = ["ext4", "btrfs", "xfs", "ntfs", "ntfs3", "exfat", "vfat", "f2fs", "ext3"];
/// Where other drives get mounted.
const MOUNTS: [&str; 4] = ["/run/media/", "/media/", "/mnt/", "/var/mnt/"];

/// The drives mounted where extra drives go, from /proc/self/mountinfo's lines.
pub fn mounted(mountinfo: &str) -> Vec<(PathBuf, String)> {
    let mut out: Vec<(PathBuf, String)> = Vec::new();
    for l in mountinfo.lines() {
        let f: Vec<&str> = l.split(' ').collect();
        let Some(sep) = f.iter().position(|x| *x == "-") else { continue };
        let (Some(mp), Some(fs), Some(src)) = (f.get(4), f.get(sep + 1), f.get(sep + 2)) else { continue };
        let mp = mp.replace("\\040", " ");
        if !MOUNTS.iter().any(|m| mp.starts_with(m)) || !FS.contains(fs) { continue; }
        if out.iter().any(|(p, _)| p.as_os_str() == mp.as_str()) { continue; }
        out.push((PathBuf::from(mp), src.to_string()));
    }
    out
}

/// Drives plugged in but not mounted, from `lsblk -J -b -o PATH,LABEL,SIZE,FSTYPE,MOUNTPOINTS,RM,TYPE`.
pub fn unmounted(lsblk: &str) -> Vec<Drive> {
    fn walk(v: &serde_json::Value, out: &mut Vec<Drive>) {
        let kids = v.get("children").and_then(|c| c.as_array());
        let fs = v.get("fstype").and_then(|x| x.as_str()).unwrap_or("");
        let free = v.get("mountpoints").and_then(|m| m.as_array()).is_none_or(|m| m.iter().all(|x| x.is_null()));
        let size = v.get("size").and_then(|s| s.as_u64().or_else(|| s.as_str().and_then(|t| t.parse().ok()))).unwrap_or(0);
        let kind = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
        if (kind == "part" || kind == "disk" && kids.is_none()) && FS.contains(&fs) && free && size >= 1 << 30 {
            let label = v.get("label").and_then(|x| x.as_str()).filter(|l| !l.is_empty());
            let removable = v.get("rm").and_then(|x| x.as_bool().or_else(|| x.as_str().map(|s| s == "1"))).unwrap_or(false);
            out.push(Drive {
                name: label.map(str::to_string).unwrap_or_else(|| tr!("{} drive", crate::storage::size(size))),
                root: None,
                device: v.get("path").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                free: 0,
                total: size,
                removable,
            });
        }
        for k in kids.into_iter().flatten() { walk(k, out); }
    }
    let mut out = Vec::new();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(lsblk) {
        for d in v.get("blockdevices").and_then(|b| b.as_array()).into_iter().flatten() { walk(d, &mut out); }
    }
    out
}

/// "Mounted /dev/sdb1 at /run/media/m/SSD" → where.
pub fn mounted_at(out: &str) -> Option<PathBuf> {
    let at = out.split(" at ").nth(1)?.trim().trim_end_matches('.');
    (!at.is_empty()).then(|| PathBuf::from(at))
}

fn space(path: &Path) -> (u64, u64) {
    use std::ffi::CString;
    let Ok(c) = CString::new(path.as_os_str().as_encoded_bytes()) else { return (0, 0) };
    let mut st = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(c.as_ptr(), st.as_mut_ptr()) } != 0 { return (0, 0); }
    let st = unsafe { st.assume_init() };
    (st.f_bavail as u64 * st.f_frsize as u64, st.f_blocks as u64 * st.f_frsize as u64)
}

fn writable(path: &Path) -> bool {
    use std::ffi::CString;
    CString::new(path.as_os_str().as_encoded_bytes()).is_ok_and(|c| unsafe { libc::access(c.as_ptr(), libc::W_OK) } == 0)
}

/// Every drive to choose from, the home folder first.
fn drives() -> Vec<Drive> {
    let home = util::expand_home("~");
    let (free, total) = space(&home);
    let mut list = vec![Drive { name: tr!("Home folder"), root: Some(home), device: String::new(), free, total, removable: false }];
    for (root, device) in mounted(&std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default()) {
        let (free, total) = space(&root);
        // Too small for a game (a boot stick's own little partition).
        if total < 1 << 30 { continue; }
        let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| root.to_string_lossy().into_owned());
        let removable = root.starts_with("/run/media") || root.starts_with("/media");
        list.push(Drive { name, root: Some(root), device, free, total, removable });
    }
    let lsblk = std::process::Command::new("lsblk").args(["-J", "-b", "-o", "PATH,LABEL,SIZE,FSTYPE,MOUNTPOINTS,RM,TYPE"])
        .stderr(std::process::Stdio::null()).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    for d in unmounted(&lsblk) {
        if !list.iter().any(|l| l.device == d.device) { list.push(d); }
    }
    list
}

/// The folders in `dir` to go into: not hidden, by name.
fn folders(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.path()).collect();
    v.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    v.truncate(500);
    v
}

fn free_line(d: &Drive) -> String {
    use crate::storage::size;
    if d.total == 0 { String::new() } else { tr!("{free} free of {total}", free = size(d.free), total = size(d.total)) }
}

impl App {
    /// Open the picker for this purpose, on the drives.
    pub fn open_folder_pick(&mut self, purpose: PickFor) {
        audio::play(Sound::Select);
        self.pick = Some(Pick { purpose, drives: drives(), at: None, entries: Vec::new(), note: String::new(), busy: None });
        self.push_overlay(Overlay::Folders, Z_PICK, 0);
        self.push_pick();
    }

    /// The rows for what's shown, to the screen.
    fn push_pick(&mut self) {
        let Some(p) = self.pick.as_mut() else { return };
        let mut rows = Vec::new();
        let mut entries = Vec::new();
        // The first drive is always the home folder (drives()); its name is translated.
        let is_home = |d: &Drive| d.name == tr!("Home folder");
        let (title, path) = match &p.at {
            None => {
                for (i, d) in p.drives.iter().enumerate() {
                    let mut sub = match (&d.root, p.busy == Some(i)) {
                        (_, true) => tr!("Mounting…"),
                        (None, _) => tr!("Not mounted · {} · ✕ mounts it", crate::storage::size(d.total)),
                        (Some(_), _) => free_line(d),
                    };
                    if d.removable && d.root.is_some() { sub = tr!("Removable · {sub}", sub); }
                    rows.push(PickRow {
                        label: d.name.clone().into(), sub: sub.into(),
                        icon: if is_home(d) { "user".into() } else { "disk".into() },
                        kind: 0, has_bar: d.root.is_some() && d.total > 0,
                        bar: if d.total > 0 { 1.0 - d.free as f32 / d.total as f32 } else { 0.0 },
                        warn: d.root.is_some() && d.total > 0 && d.free < 20 << 30,
                    });
                    entries.push(Entry::Drive(i));
                }
                (tr!("{} · choose a drive", p.purpose.title()), String::new())
            }
            Some((di, dir)) => {
                let d = &p.drives[*di];
                let top = d.root.as_deref() == Some(dir.as_path());
                let can = writable(dir);
                let (label, sub) = match p.purpose {
                    PickFor::Place => (if top && !is_home(d) { tr!("Use this drive") } else { tr!("Use this folder") }, tr!("Games and downloads go in {}", util::display_path(&dir.join("SwarmPlay").to_string_lossy()))),
                    PickFor::Install => (tr!("Install new games here"), util::display_path(&dir.to_string_lossy())),
                    PickFor::Download => (tr!("Save downloads here"), util::display_path(&dir.to_string_lossy())),
                };
                rows.push(PickRow {
                    label: label.into(),
                    sub: if can { sub.into() } else { tr!("Can't write here · choose a folder of yours").into() },
                    icon: "check".into(), kind: 1, has_bar: false, bar: 0.0, warn: !can,
                });
                entries.push(Entry::Use);
                for f in folders(dir) {
                    let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let sub = if name == "SwarmPlay" { tr!("SwarmPlay's games and downloads") } else { String::new() };
                    rows.push(PickRow { label: name.into(), sub: sub.into(), icon: "folder".into(), kind: 2, has_bar: false, bar: 0.0, warn: false });
                    entries.push(Entry::Folder(f));
                }
                let rel = dir.strip_prefix(d.root.as_deref().unwrap_or(Path::new("/"))).map(|r| r.to_string_lossy().into_owned()).unwrap_or_default();
                let path = if rel.is_empty() { d.name.clone() } else { format!("{} › {}", d.name, rel.replace('/', " › ")) };
                (p.purpose.title(), format!("{path}   ·   {}", free_line(d)))
            }
        };
        p.entries = entries;
        let note = p.note.clone();
        let step = if p.at.is_some() { 1 } else { 0 };
        let ui = self.ui();
        ui.set_pick_title(title.into());
        ui.set_pick_path(path.into());
        ui.set_pick_note(note.into());
        ui.set_pick_step(step);
        ui.set_pick_rows(model(rows));
    }

    pub fn act_folder_pick(&mut self, a: Act) {
        let Some(p) = self.pick.as_ref() else { return self.back_soft() };
        let n = p.entries.len();
        let i = self.idx.max(0) as usize;
        match a {
            Act::Up if i > 0 => self.move_focus(Z_PICK, i as i32 - 1),
            Act::Down if i + 1 < n => self.move_focus(Z_PICK, i as i32 + 1),
            Act::First => self.move_focus(Z_PICK, 0),
            Act::Last if n > 0 => self.move_focus(Z_PICK, n as i32 - 1),
            Act::PageUp => self.move_focus(Z_PICK, i.saturating_sub(8) as i32),
            Act::PageDown if n > 0 => self.move_focus(Z_PICK, (i + 8).min(n - 1) as i32),
            Act::Confirm => self.pick_enter(i),
            // ▸ goes into a folder (or a drive), ◂ up.
            Act::Right if matches!(p.entries.get(i), Some(Entry::Folder(_) | Entry::Drive(_))) => self.pick_enter(i),
            // △: this folder, wherever the selection is.
            Act::Search if p.at.is_some() => self.pick_enter(0),
            Act::Back | Act::Left => self.pick_up(a == Act::Back),
            _ => {}
        }
    }

    /// Up a folder; from a drive's top to the drives; from the drives, closed (Back only).
    fn pick_up(&mut self, can_close: bool) {
        let Some(p) = self.pick.as_mut() else { return };
        p.note.clear();
        match p.at.take() {
            None => {
                if can_close { self.pick = None; self.back_soft(); }
                return;
            }
            Some((di, dir)) => {
                let top = p.drives[di].root.clone().unwrap_or_default();
                let focus = if dir != top {
                    let parent = dir.parent().map(Path::to_path_buf).unwrap_or(top);
                    // The folder just left stays chosen in its parent.
                    let k = folders(&parent).iter().position(|f| *f == dir).map_or(0, |k| k + 1);
                    p.at = Some((di, parent));
                    k
                } else {
                    di
                };
                audio::play(Sound::Back);
                self.push_pick();
                self.set_focus(Z_PICK, focus as i32);
            }
        }
    }

    fn pick_enter(&mut self, i: usize) {
        let Some(p) = self.pick.as_mut() else { return };
        let Some(e) = p.entries.get(i).cloned() else { return };
        p.note.clear();
        match e {
            Entry::Drive(_) if p.busy.is_some() => {}
            Entry::Drive(di) => match p.drives[di].root.clone() {
                Some(root) => {
                    audio::play(Sound::Select);
                    p.at = Some((di, root));
                    self.push_pick();
                    self.set_focus(Z_PICK, 0);
                }
                None => {
                    // Mount it, then go in.
                    audio::play(Sound::Select);
                    p.busy = Some(di);
                    let device = p.drives[di].device.clone();
                    self.push_pick();
                    std::thread::spawn(move || {
                        let out = std::process::Command::new("udisksctl").args(["mount", "-b", &device])
                            .stdin(std::process::Stdio::null()).output();
                        let res = match out {
                            Ok(o) if o.status.success() => mounted_at(&String::from_utf8_lossy(&o.stdout)).ok_or_else(|| tr!("it didn't say where")),
                            Ok(o) => Err(String::from_utf8_lossy(&o.stderr).lines().last().map(|l| l.trim().to_string()).unwrap_or_else(|| tr!("it failed"))),
                            Err(e) => Err(e.to_string()),
                        };
                        let _ = slint::invoke_from_event_loop(move || with_app(move |app| app.pick_mounted(di, res)));
                    });
                }
            },
            Entry::Folder(f) => {
                if let Some((_, dir)) = p.at.as_mut() { *dir = f; }
                audio::play(Sound::Move);
                self.push_pick();
                self.set_focus(Z_PICK, 0);
            }
            Entry::Use => {
                let Some((_, dir)) = p.at.clone() else { return };
                if !writable(&dir) {
                    p.note = tr!("Can't write there: choose a folder of yours, or the drive's own top");
                    audio::play(Sound::Back);
                    return self.push_pick();
                }
                let purpose = p.purpose;
                self.pick = None;
                self.back_soft();
                self.pick_chosen(purpose, &dir);
                // Back on Storage once the picker has gone: on the row it was opened from.
                slint::Timer::single_shot(std::time::Duration::from_millis(220), move || with_app(move |app| app.select_setting(purpose.row())));
            }
        }
    }

    fn pick_mounted(&mut self, di: usize, res: Result<PathBuf, String>) {
        let Some(p) = self.pick.as_mut() else { return };
        p.busy = None;
        match res {
            Ok(root) => {
                let (free, total) = space(&root);
                let d = &mut p.drives[di];
                d.root = Some(root.clone());
                (d.free, d.total) = (free, total);
                p.at = Some((di, root));
                audio::play(Sound::Select);
                self.push_pick();
                if self.overlay == Overlay::Folders { self.set_focus(Z_PICK, 0); }
            }
            Err(e) => {
                p.note = tr!("Couldn't mount it: {e}", e);
                audio::play(Sound::Back);
                self.push_pick();
            }
        }
    }

    /// The folder chosen, for what it was for.
    fn pick_chosen(&mut self, purpose: PickFor, dir: &Path) {
        let shown = util::display_path(&dir.to_string_lossy());
        let path = dir.to_string_lossy().into_owned();
        match purpose {
            PickFor::Place => {
                let dev = |p: &Path| { use std::os::unix::fs::MetadataExt; std::fs::metadata(p).map(|m| m.dev()).unwrap_or(0) };
                if self.storage_places().iter().any(|p| p.dev == dev(dir) && p.added.is_none()) {
                    // Already a place: a folder on it becomes where new games go instead.
                    self.note_setting(SId::StorageAdd, &tr!("That drive is already in the list · use Install new games to for a folder on it"), 0);
                } else {
                    self.storage_add(&path);
                }
            }
            PickFor::Install => {
                if let Err(e) = std::fs::create_dir_all(dir) { return self.note_setting(purpose.row(), &tr!("Not changed: {e}", e), 2); }
                {
                    let mut c = self.cfg.lock().unwrap();
                    c.install_dir = path.clone();
                    if !c.game_dir_paths().iter().any(|d| d == dir) { c.game_dirs.push(path); }
                    c.save();
                }
                audio::play(Sound::Select);
                self.note_setting(purpose.row(), &tr!("New games are set up in {shown} now", shown), 1);
            }
            PickFor::Download => {
                self.cfg.lock().unwrap().download_dir = path;
                self.cfg.lock().unwrap().save();
                audio::play(Sound::Select);
                self.note_setting(purpose.row(), &tr!("Downloads go to {shown} now", shown), 1);
            }
        }
        Self::storage_recount();
        self.refresh_settings();
    }

    /// A row clicked (or the ‹ at the top: up).
    pub fn click_folder_pick(&mut self, idx: i32) {
        if idx < 0 { return self.pick_up(true); }
        if self.idx == idx { self.pick_enter(idx as usize); } else { self.move_focus(Z_PICK, idx); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_extra_drives_in_mountinfo() {
        let info = "\
29 1 0:26 /root / rw - btrfs /dev/nvme0n1p3 rw
64 29 0:26 /home /var/home rw - btrfs /dev/nvme0n1p3 rw
90 29 8:17 / /run/media/m/Games\\040SSD rw,nosuid - ext4 /dev/sdb1 rw
91 29 8:33 / /var/mnt/data rw - ntfs3 /dev/sdc1 rw
92 29 0:50 / /run/media/m/iso ro - iso9660 /dev/sr0 ro
93 29 0:51 / /mnt/tmp rw - tmpfs tmpfs rw";
        let m = mounted(info);
        assert_eq!(m, vec![(PathBuf::from("/run/media/m/Games SSD"), "/dev/sdb1".into()), (PathBuf::from("/var/mnt/data"), "/dev/sdc1".into())]);
    }

    #[test]
    fn finds_drives_not_mounted() {
        let j = r#"{"blockdevices":[
          {"path":"/dev/nvme0n1","label":null,"size":1000204886016,"fstype":null,"mountpoints":[null],"rm":false,"type":"disk","children":[
            {"path":"/dev/nvme0n1p1","label":"EFI","size":629145600,"fstype":"vfat","mountpoints":["/boot/efi"],"rm":false,"type":"part"},
            {"path":"/dev/nvme0n1p3","label":"fedora","size":998000000000,"fstype":"btrfs","mountpoints":["/var/home","/"],"rm":false,"type":"part"}]},
          {"path":"/dev/sdb","label":null,"size":256060514304,"fstype":null,"mountpoints":[null],"rm":true,"type":"disk","children":[
            {"path":"/dev/sdb1","label":"Games","size":256059465728,"fstype":"exfat","mountpoints":[null],"rm":true,"type":"part"}]},
          {"path":"/dev/sdc1","label":"","size":500107862016,"fstype":"ext4","mountpoints":[null],"rm":"0","type":"part"}]}"#;
        let d = unmounted(j);
        assert_eq!(d.len(), 2);
        assert_eq!((d[0].name.as_str(), d[0].device.as_str(), d[0].removable), ("Games", "/dev/sdb1", true));
        assert_eq!(d[1].name, "465.8 GB drive");
        assert!(!d[1].removable);
    }

    #[test]
    fn reads_where_udisks_mounted_it() {
        assert_eq!(mounted_at("Mounted /dev/sdb1 at /run/media/m/Games.\n"), Some(PathBuf::from("/run/media/m/Games")));
        assert_eq!(mounted_at("Mounted /dev/sdb1 at /run/media/m/My SSD\n"), Some(PathBuf::from("/run/media/m/My SSD")));
        assert_eq!(mounted_at("nothing"), None);
    }
}
