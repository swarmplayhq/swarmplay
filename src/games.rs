//! Installed games, as a console's Storage → Games: every game on the drives (or on one drive),
//! biggest first, each with its size. ✕ says what can be done with it (Play, Move to another
//! drive, Delete), △ picks several, □ deletes the one selected or the ones picked. Deleting asks
//! first (the Power card); moving copies the game to the other drive in the background and
//! deletes it here only once the copy is whole, so a move that fails or is cancelled loses nothing.
//!
//! Opened from Settings → Games, a drive's "Games" line in Storage, and a game's Information.

use crate::app::*;
use crate::audio::{self, Sound};
use crate::{util, GameRowData};
use anyhow::{bail, ensure, Context, Result};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tr::tr;

/// Each game folder's size on its drive, counted in the background (storage::du).
static SIZES: Mutex<Option<HashMap<PathBuf, u64>>> = Mutex::new(None);
static COUNTING: AtomicBool = AtomicBool::new(false);

/// The menu's "Move to" choices, one per other drive (a menu item's id is static).
pub const MOVE_IDS: [&str; 6] = ["move0", "move1", "move2", "move3", "move4", "move5"];

/// A game being copied to another drive (one at a time).
pub struct Moving {
    pub from: PathBuf,
    pub name: String,
    pub drive: String,
    pub done: Arc<AtomicU64>,
    pub total: u64,
    pub cancel: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct GamesView {
    /// One drive's games (its device number), or every drive's.
    pub drive: Option<u64>,
    /// The games shown, by folder, biggest first.
    pub list: Vec<PathBuf>,
    /// Picked with △, to delete together.
    pub picked: HashSet<PathBuf>,
    pub moving: Option<Moving>,
    /// The rows' image keys, to put art in as it arrives.
    pub keys: Vec<String>,
    /// The Power card's "Delete?": which games.
    pub doomed: Vec<PathBuf>,
}

fn dev_of(p: &Path) -> u64 { std::fs::metadata(p).map_or(0, |m| m.dev()) }

pub fn size_of(p: &Path) -> Option<u64> { SIZES.lock().unwrap().as_ref().and_then(|m| m.get(p).copied()) }

/// Count the sizes not known yet, in the background; the list is shown again as they come.
fn count_sizes(paths: Vec<PathBuf>) {
    let todo: Vec<PathBuf> = paths.into_iter().filter(|p| size_of(p).is_none()).collect();
    if todo.is_empty() || COUNTING.swap(true, Ordering::AcqRel) { return; }
    std::thread::spawn(move || {
        for p in todo {
            let bytes = crate::storage::du(&p);
            SIZES.lock().unwrap().get_or_insert_with(HashMap::new).insert(p, bytes);
            let _ = slint::invoke_from_event_loop(|| with_app(|app| if app.overlay == Overlay::Games { app.games_push(); }));
        }
        COUNTING.store(false, Ordering::Release);
    });
}

/// A game folder that can be deleted: a real folder (not a link), strictly inside one of the game
/// folders (never one of them, nor anything above them), and a game the Library lists. What
/// SwarmPlay installed itself goes through the installer's own uninstall instead.
pub fn deletable(path: &Path, game_dirs: &[PathBuf]) -> Result<PathBuf> {
    let meta = std::fs::symlink_metadata(path).with_context(|| tr!("It isn't there any more"))?;
    ensure!(meta.is_dir() && !meta.file_type().is_symlink(), tr!("{} isn't a game's folder", path.display()));
    let real = path.canonicalize()?;
    let inside = game_dirs.iter().filter_map(|d| d.canonicalize().ok()).any(|d| real != d && real.starts_with(&d));
    ensure!(inside, tr!("{} isn't in a game folder", path.display()));
    ensure!(crate::library::is_game(&real), tr!("{} isn't a game's folder", path.display()));
    Ok(real)
}

/// Delete a folder made sure of (`deletable`): hidden at once, so the Library never lists it while
/// it goes, then deleted in the background (finished at the next start if SwarmPlay closes first).
/// Links inside are deleted, never followed.
fn delete_folder(real: &Path) -> Result<()> {
    let parent = real.parent().context("No parent folder")?;
    let doomed = parent.join(util::deleting_name(&util::sha1_hex(&real.to_string_lossy())[..12]));
    std::fs::rename(real, &doomed).with_context(|| tr!("Delete {}", real.display()))?;
    let target = doomed.clone();
    util::delete_in_background(doomed, move || Ok(std::fs::remove_dir_all(&target)?));
    Ok(())
}

/// Copy `from` (a folder) to `to` (made here, not there before): files with their permissions and
/// times, links as links (never followed), nothing else (sockets, devices). `done` counts the bytes
/// copied; `cancel` stops it between chunks.
pub fn copy_tree(from: &Path, to: &Path, done: &AtomicU64, cancel: &AtomicBool) -> Result<()> {
    let meta = std::fs::symlink_metadata(from)?;
    ensure!(meta.is_dir() && !meta.file_type().is_symlink(), "{} isn't a folder", from.display());
    std::fs::create_dir(to)?;
    let mut buf = vec![0u8; 8 << 20];
    let mut todo = vec![(from.to_path_buf(), to.to_path_buf())];
    let mut dirs = Vec::new();
    while let Some((src, dst)) = todo.pop() {
        for entry in std::fs::read_dir(&src)? {
            if cancel.load(Ordering::Acquire) { bail!(tr!("Cancelled")); }
            let entry = entry?;
            let (s, d) = (entry.path(), dst.join(entry.file_name()));
            let m = std::fs::symlink_metadata(&s)?;
            let ft = m.file_type();
            if ft.is_symlink() {
                std::os::unix::fs::symlink(std::fs::read_link(&s)?, &d)?;
            } else if ft.is_dir() {
                std::fs::create_dir(&d)?;
                dirs.push((d.clone(), m.clone()));
                todo.push((s, d));
            } else if ft.is_file() {
                let mut input = std::fs::File::open(&s)?;
                let mut output = std::fs::OpenOptions::new().write(true).create_new(true).open(&d)?;
                loop {
                    if cancel.load(Ordering::Acquire) { bail!(tr!("Cancelled")); }
                    let n = input.read(&mut buf)?;
                    if n == 0 { break; }
                    output.write_all(&buf[..n])?;
                    done.fetch_add(n as u64, Ordering::Relaxed);
                }
                output.set_permissions(std::fs::Permissions::from_mode(m.mode() & 0o7777))?;
                if let Ok(t) = m.modified() { let _ = output.set_modified(t); }
            }
        }
    }
    // Folders last: their times change as things go in.
    for (d, m) in dirs.into_iter().rev() {
        let _ = std::fs::set_permissions(&d, std::fs::Permissions::from_mode(m.mode() & 0o7777));
        if let (Ok(t), Ok(f)) = (m.modified(), std::fs::File::open(&d)) { let _ = f.set_modified(t); }
    }
    let _ = std::fs::set_permissions(to, std::fs::Permissions::from_mode(meta.mode() & 0o7777));
    Ok(())
}

impl App {
    /// Open the list: one drive's games or all, with `focus` selected.
    pub fn open_games(&mut self, drive: Option<u64>, focus: Option<&Path>) {
        audio::play(Sound::Select);
        self.games_view.drive = drive;
        self.games_view.picked.clear();
        self.games_list();
        let i = focus.and_then(|f| self.games_view.list.iter().position(|p| p == f)).unwrap_or(0) as i32;
        if self.overlay == Overlay::Games { self.set_focus(Z_GAMES, i); } else { self.push_overlay(Overlay::Games, Z_GAMES, i); }
        self.games_push();
    }

    /// The games to show, biggest first (not counted yet: last, by name).
    fn games_list(&mut self) {
        let drive = self.games_view.drive;
        let mut list: Vec<(PathBuf, String)> = self.locals.iter().map(|l| (l.l.path.clone(), l.name.clone()))
            .filter(|(p, _)| drive.is_none_or(|d| dev_of(p) == d)).collect();
        list.sort_by(|a, b| size_of(&b.0).cmp(&size_of(&a.0)).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
        self.games_view.list = list.into_iter().map(|(p, _)| p).collect();
        count_sizes(self.games_view.list.clone());
    }

    fn local_at(&self, p: &Path) -> Option<usize> { self.locals.iter().position(|l| l.l.path == p) }

    /// The list to the screen (again as sizes, art and a move's progress come in).
    pub fn games_push(&mut self) {
        if self.overlay != Overlay::Games { return; }
        // Sizes counted since: biggest first again, the same game selected.
        let selected = self.games_view.list.get(self.idx.max(0) as usize).cloned();
        let before = self.games_view.list.clone();
        self.games_list();
        if before != self.games_view.list {
            if let Some(i) = selected.and_then(|s| self.games_view.list.iter().position(|p| *p == s)) { self.idx = i as i32; }
        }
        let places = self.storage_places();
        let drive_of = |p: &Path| { let d = dev_of(p); places.iter().find(|pl| pl.dev == d).map(|pl| pl.name.clone()).unwrap_or_default() };
        let mut rows = Vec::new();
        let mut keys = Vec::new();
        let mut total = 0u64;
        for p in self.games_view.list.clone() {
            let Some(l) = self.local_at(&p) else { continue };
            let r = self.tile_req(RowItem::Local(l));
            keys.push(r.as_ref().map(|r| r.key.clone()).unwrap_or_default());
            let img = self.get_img(&r, crate::images::prio::TILE);
            let size = size_of(&p);
            total += size.unwrap_or(0);
            let platform = self.locals[l].l.platform.label();
            let (sub, busy, bar) = match &self.games_view.moving {
                Some(m) if m.from == p => {
                    let f = if m.total > 0 { m.done.load(Ordering::Relaxed) as f32 / m.total as f32 } else { 0.0 };
                    (tr!("Moving to {} · {}%", m.drive, (f * 100.0).floor() as u32), true, f.clamp(0.0, 1.0))
                }
                _ => (format!("{platform} · {}", drive_of(&p)), false, 0.0),
            };
            rows.push(GameRowData {
                image: img.clone().unwrap_or_default(), loaded: img.is_some(), label: self.locals[l].name.clone().into(), sub: sub.into(),
                size: size.map_or("…".into(), crate::storage::size).into(), picked: self.games_view.picked.contains(&p), busy, bar,
            });
        }
        self.games_view.keys = keys;
        let title = match self.games_view.drive {
            Some(d) => places.iter().find(|pl| pl.dev == d).map(|pl| pl.name.clone()).unwrap_or_else(|| tr!("Games")),
            None => tr!("Installed games"),
        };
        let n = rows.len();
        let picked = self.games_view.picked.len();
        let sub = if picked > 0 { tr!("{n} picked · □ deletes them · ○ lets go" | "{n} picked · □ deletes them · ○ lets go" % picked) }
            else { tr!("{} game · {}" | "{} games · {}" % n, n, crate::storage::size(total)) };
        let ui = self.ui();
        ui.set_games_title(title.into());
        ui.set_games_sub(sub.into());
        ui.set_games_picking(picked > 0);
        ui.set_games_rows(model(rows));
        if self.idx >= n as i32 { self.idx = n as i32 - 1; }
    }

    /// Art for a row came in.
    pub fn games_image(&mut self, key: &str) {
        if self.overlay == Overlay::Games && self.games_view.keys.iter().any(|k| k == key) { self.games_push(); }
    }

    pub fn act_games(&mut self, a: Act) {
        let n = self.games_view.list.len() as i32;
        let i = self.idx.max(0);
        match a {
            Act::Up if i > 0 => self.move_focus(Z_GAMES, i - 1),
            Act::Down if i + 1 < n => self.move_focus(Z_GAMES, i + 1),
            Act::First if n > 0 => self.move_focus(Z_GAMES, 0),
            Act::Last if n > 0 => self.move_focus(Z_GAMES, n - 1),
            Act::PageUp => self.move_focus(Z_GAMES, (i - 8).max(0)),
            Act::PageDown if n > 0 => self.move_focus(Z_GAMES, (i + 8).min(n - 1)),
            Act::Confirm | Act::Options => self.games_options(i as usize),
            // △ picks it (or lets it go).
            Act::Search => {
                let Some(p) = self.games_view.list.get(i as usize).cloned() else { return };
                if !self.games_view.picked.remove(&p) { self.games_view.picked.insert(p); }
                audio::play(Sound::Move);
                self.games_push();
            }
            // □ deletes the ones picked, or this one.
            Act::Trailer => {
                let doomed: Vec<PathBuf> = if self.games_view.picked.is_empty() { self.games_view.list.get(i as usize).cloned().into_iter().collect() }
                    else { self.games_view.list.iter().filter(|p| self.games_view.picked.contains(*p)).cloned().collect() };
                self.ask_delete(doomed);
            }
            Act::Back if !self.games_view.picked.is_empty() => {
                self.games_view.picked.clear();
                audio::play(Sound::Back);
                self.games_push();
            }
            Act::Back => self.back_soft(),
            _ => {}
        }
    }

    /// A row clicked: selected, or (already selected) its options.
    pub fn click_games(&mut self, idx: i32) {
        if idx < 0 { return self.back_soft(); }
        if self.idx == idx { self.games_options(idx as usize); } else { self.move_focus(Z_GAMES, idx); }
    }

    /// ✕ on a game: Play, Move to each other drive, Delete (the Options menu, over the list).
    fn games_options(&mut self, i: usize) {
        let Some(l) = self.games_view.list.get(i).and_then(|p| self.local_at(p)) else { return };
        let t = Target { game: self.locals[l].cat, local: Some(l) };
        let items = self.game_care_items(t, true);
        let title = self.locals[l].name.clone();
        self.menu_target = Some(t);
        self.menu_actions = items;
        audio::play(Sound::Select);
        self.ui().set_menu_anchor(0);
        self.push_overlay(Overlay::Menu, Z_MENU, 0);
        self.push_menu(&title);
    }

    /// What can be done with an installed game: Play (from the list), Move to each other drive
    /// with room for it, Delete.
    pub fn game_care_items(&self, t: Target, play: bool) -> Vec<ActionDef> {
        let mk = |id: &'static str, label: String, icon: &'static str| ActionDef { id, label, icon, primary: false, danger: false, round: false };
        let mut items = Vec::new();
        let Some(l) = t.local else { return items };
        let path = &self.locals[l].l.path;
        let running = self.live.iter().any(|s| Path::new(&s.path) == path);
        if play && !running { items.push(mk("play", tr!("Play"), "play")); }
        if self.games_view.moving.is_none() && !running {
            let here = dev_of(path);
            let size = size_of(path).unwrap_or(0);
            for (k, place) in self.storage_places().into_iter().filter(|p| p.dev != here && p.total > 0).take(MOVE_IDS.len()).enumerate() {
                let label = if place.free > size + (1 << 30) { tr!("Move to {} · {} free", place.name, crate::storage::size(place.free)) }
                    else { tr!("Move to {} · not enough room", place.name) };
                items.push(mk(MOVE_IDS[k], label, "disk"));
            }
        }
        if let Some(m) = self.games_view.moving.as_ref().filter(|m| m.from == *path) {
            items.push(mk("move_cancel", tr!("Cancel moving to {}", m.drive), "close"));
        }
        if !running { items.push(ActionDef { danger: true, ..mk("delete", tr!("Delete"), "trash") }); }
        items
    }

    /// The menu's choices for an installed game (run_action): Delete, a Move, Cancel moving,
    /// Information.
    pub fn game_care_action(&mut self, id: &str, t: Target) -> bool {
        let Some(l) = t.local else { return false };
        let path = self.locals[l].l.path.clone();
        match id {
            "delete" => self.ask_delete(vec![path]),
            "info" => self.open_games(None, Some(&path)),
            "move_cancel" => if let Some(m) = &self.games_view.moving { m.cancel.store(true, Ordering::Release); },
            _ => match MOVE_IDS.iter().position(|m| *m == id) {
                Some(k) => {
                    let here = dev_of(&path);
                    let place = self.storage_places().into_iter().filter(|p| p.dev != here && p.total > 0).nth(k);
                    if let Some(place) = place { self.start_move(&path, place); }
                }
                None => return false,
            },
        }
        true
    }

    /// The Power card asks before deleting.
    pub fn ask_delete(&mut self, doomed: Vec<PathBuf>) {
        let doomed: Vec<PathBuf> = doomed.into_iter().filter(|p| !self.live.iter().any(|s| Path::new(&s.path) == p)).collect();
        if doomed.is_empty() { return audio::play(Sound::Error); }
        self.games_view.doomed = doomed;
        self.power_ask(crate::os::power::Power::Delete);
    }

    /// The card's title and words for deleting what `ask_delete` was given.
    pub fn delete_words(&self) -> (String, String) {
        let doomed = &self.games_view.doomed;
        let size: u64 = doomed.iter().filter_map(|p| size_of(p)).sum();
        let frees = if size > 0 { tr!("Frees {}.", crate::storage::size(size)) } else { String::new() };
        match &doomed[..] {
            [one] => {
                let name = self.local_at(one).map(|l| self.locals[l].name.clone()).unwrap_or_default();
                (tr!("Delete {}?", name), format!("{frees} {}", tr!("It leaves your games. Download or install it again to play it.")).trim().to_string())
            }
            many => (tr!("Delete {} games?", many.len()), format!("{frees} {}", tr!("They leave your games. Download or install them again to play them.")).trim().to_string()),
        }
    }

    /// Confirmed on the card: each game goes (what SwarmPlay installed, by its installer).
    pub fn delete_games(&mut self) {
        let doomed = std::mem::take(&mut self.games_view.doomed);
        let dirs = self.cfg.lock().unwrap().game_dir_paths();
        let records = self.installer.snapshot();
        let (mut gone, mut freed, mut failed) = (0, 0u64, Vec::new());
        for p in &doomed {
            let name = self.local_at(p).map(|l| self.locals[l].name.clone()).unwrap_or_else(|| p.display().to_string());
            let own = records.iter().find(|r| r.state == crate::installer::State::Installed && r.path.as_deref() == Some(p.as_path()));
            let result = match own {
                Some(r) => self.installer.uninstall(&r.key).map(|_| ()),
                None => deletable(p, &dirs).and_then(|real| delete_folder(&real)),
            };
            match result {
                Ok(()) => { gone += 1; freed += size_of(p).unwrap_or(0); self.games_view.picked.remove(p); }
                Err(e) => failed.push(format!("{name}: {e:#}")),
            }
        }
        self.rescan_library();
        Self::storage_recount();
        if gone > 0 {
            audio::play(Sound::Back);
            let what = if gone == 1 { tr!("Game deleted") } else { tr!("{} games deleted", gone) };
            self.toast(&what, &tr!("{} freed", crate::storage::size(freed)), "trash", crate::toast::DONE);
        }
        if let Some(why) = failed.first() {
            self.toast(&tr!("Couldn't delete it"), why, "alert", crate::toast::FAILED);
        }
        if self.overlay == Overlay::Games { self.games_push(); }
    }

    /// Copy a game to another drive's games folder, then delete it here. The copy is hidden until
    /// whole; the old folder goes only once the new one is in place.
    fn start_move(&mut self, from: &Path, place: crate::storage::Place) {
        let size = size_of(from).unwrap_or_else(|| crate::storage::du(from));
        if self.games_view.moving.is_some() { return; }
        if place.free <= size + (1 << 30) {
            audio::play(Sound::Error);
            return self.toast(&tr!("Not enough room on {}", place.name), &tr!("It needs {}", crate::storage::size(size)), "disk", crate::toast::FAILED);
        }
        let root = util::expand_home(&place.install);
        let Some(name) = from.file_name().map(|n| n.to_os_string()) else { return };
        let to = root.join(&name);
        if std::fs::symlink_metadata(&to).is_ok() {
            audio::play(Sound::Error);
            return self.toast(&tr!("Couldn't move it"), &tr!("{} already has a folder of that name", place.name), "alert", crate::toast::FAILED);
        }
        let game = self.local_at(from).map(|l| self.locals[l].name.clone()).unwrap_or_default();
        let done = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        self.games_view.moving = Some(Moving { from: from.to_path_buf(), name: game.clone(), drive: place.name.clone(), done: done.clone(), total: size, cancel: cancel.clone() });
        audio::play(Sound::Select);
        self.toast(&tr!("Moving {}", game), &tr!("To {} · you can keep playing other games", place.name), "disk", crate::toast::INFO);
        let from = from.to_path_buf();
        std::thread::spawn(move || {
            let hidden = root.join(format!(".azrak-moving-{}", name.to_string_lossy()));
            let result = std::fs::create_dir_all(&root).map_err(anyhow::Error::from)
                .and_then(|_| copy_tree(&from, &hidden, &done, &cancel))
                .and_then(|_| {
                    ensure!(std::fs::symlink_metadata(&to).is_err(), "{} appeared while copying", to.display());
                    Ok(std::fs::rename(&hidden, &to)?)
                });
            if result.is_err() && std::fs::symlink_metadata(&hidden).is_ok() {
                let doomed = hidden.clone();
                util::delete_in_background(hidden, move || Ok(std::fs::remove_dir_all(&doomed)?));
            }
            let _ = slint::invoke_from_event_loop(move || with_app(move |app| app.move_finished(&from, &to, result)));
        });
        if self.overlay == Overlay::Games { self.games_push(); }
    }

    fn move_finished(&mut self, from: &Path, to: &Path, result: Result<()>) {
        let Some(m) = self.games_view.moving.take() else { return };
        match result {
            Ok(()) => {
                // What's kept about it follows it: the installer's record (so Delete deletes it
                // there), its play time, its Windows prefix.
                if let Err(e) = self.installer.moved(from, to) { crate::log!("moved {}: record not updated: {e:#}", m.name); }
                self.sessions.moved(&from.to_string_lossy(), &to.to_string_lossy());
                let prefixes = util::data_dir().join("prefixes");
                let id = |p: &Path| util::sha1_hex(&p.to_string_lossy())[..12].to_string();
                if prefixes.join(id(from)).is_dir() { let _ = std::fs::rename(prefixes.join(id(from)), prefixes.join(id(to))); }
                if let Some(root) = to.parent() {
                    let root = root.to_string_lossy().into_owned();
                    self.save_cfg(|c| if !c.game_dir_paths().iter().any(|d| *d == Path::new(&root)) { c.game_dirs.push(root) });
                }
                // The old copy goes (it's still a game in a game folder: made sure of first).
                let dirs = self.cfg.lock().unwrap().game_dir_paths();
                if let Err(e) = deletable(from, &dirs).and_then(|real| delete_folder(&real)) {
                    crate::log!("moved {}: the old copy stays: {e:#}", m.name);
                }
                if let Some(bytes) = size_of(from) { SIZES.lock().unwrap().get_or_insert_with(HashMap::new).insert(to.to_path_buf(), bytes); }
                self.rescan_library();
                Self::storage_recount();
                audio::play(Sound::Select);
                self.notify(crate::toast::Cat::Installs, crate::toast::DONE, "disk", &tr!("Moved to {}", m.drive), &m.name,
                    crate::toast::Action::Play(to.to_string_lossy().into_owned()), &format!("move:{}", m.name), true);
            }
            Err(e) => {
                let cancelled = m.cancel.load(Ordering::Acquire);
                if cancelled { self.toast(&tr!("Move cancelled"), &tr!("{} stays where it was", m.name), "disk", crate::toast::INFO); }
                else { self.toast(&tr!("Couldn't move {}", m.name), &format!("{e:#}"), "alert", crate::toast::FAILED); }
            }
        }
        if self.overlay == Overlay::Games { self.games_push(); }
    }

    /// Every second while a move runs: its progress on the list.
    pub fn games_tick(&mut self) {
        if self.games_view.moving.is_some() && self.overlay == Overlay::Games { self.games_push(); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("start.sh"), "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(dir.join("start.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn only_a_game_inside_a_game_folder_can_be_deleted() {
        let temp = tempfile::tempdir().unwrap();
        let games = temp.path().join("Games");
        game(&games.join("Some Game"));
        let dirs = vec![games.clone()];
        assert!(deletable(&games.join("Some Game"), &dirs).is_ok());
        assert!(deletable(&games, &dirs).is_err(), "never the game folder itself");
        assert!(deletable(temp.path(), &dirs).is_err(), "nor anything above it");
        std::fs::create_dir_all(games.join("Notes")).unwrap();
        assert!(deletable(&games.join("Notes"), &dirs).is_err(), "a folder that isn't a game");
        game(&temp.path().join("Elsewhere"));
        std::os::unix::fs::symlink(temp.path().join("Elsewhere"), games.join("Linked")).unwrap();
        assert!(deletable(&games.join("Linked"), &dirs).is_err(), "a link, never what it points to");
        assert!(deletable(&temp.path().join("Elsewhere"), &dirs).is_err(), "outside the game folders");
    }

    #[test]
    fn deleting_hides_the_folder_at_once_and_spares_what_links_point_to() {
        let temp = tempfile::tempdir().unwrap();
        let games = temp.path().join("Games");
        game(&games.join("Gone"));
        let precious = temp.path().join("precious.txt");
        std::fs::write(&precious, "keep").unwrap();
        std::os::unix::fs::symlink(&precious, games.join("Gone/link")).unwrap();
        let real = deletable(&games.join("Gone"), &[games.clone()]).unwrap();
        delete_folder(&real).unwrap();
        assert!(!games.join("Gone").exists(), "gone from the list at once");
        for _ in 0..100 { if std::fs::read_dir(&games).unwrap().count() == 0 { break; } std::thread::sleep(std::time::Duration::from_millis(20)); }
        assert_eq!(std::fs::read_dir(&games).unwrap().count(), 0, "deleted in the background");
        assert_eq!(std::fs::read_to_string(&precious).unwrap(), "keep");
    }

    #[test]
    fn a_copy_keeps_files_links_and_permissions_and_can_be_cancelled() {
        let temp = tempfile::tempdir().unwrap();
        let from = temp.path().join("from");
        game(&from);
        std::fs::create_dir_all(from.join("data/deep")).unwrap();
        std::fs::write(from.join("data/deep/save.bin"), vec![7u8; 100_000]).unwrap();
        std::os::unix::fs::symlink("../start.sh", from.join("data/run")).unwrap();
        let (done, cancel) = (AtomicU64::new(0), AtomicBool::new(false));
        copy_tree(&from, &temp.path().join("to"), &done, &cancel).unwrap();
        let to = temp.path().join("to");
        assert_eq!(std::fs::read(to.join("data/deep/save.bin")).unwrap(), vec![7u8; 100_000]);
        assert_eq!(std::fs::read_link(to.join("data/run")).unwrap(), Path::new("../start.sh"));
        assert_eq!(std::fs::metadata(to.join("start.sh")).unwrap().mode() & 0o777, 0o755);
        assert_eq!(done.load(Ordering::Relaxed), 100_000 + "#!/bin/sh\n".len() as u64);
        cancel.store(true, Ordering::Release);
        assert!(copy_tree(&from, &temp.path().join("again"), &done, &cancel).is_err());
    }
}
