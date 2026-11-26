//! Repacks on Home (src/repack.rs): each one found on a drive shows as a console shows a disc put
//! in, its cover at the front of the row with the disc peeking out. Install asks where (a button per
//! drive, with its free space), then checks the files and installs it with the disc spinning and a
//! bar on the cover; the island says how far. Once installed it's an ordinary game in the Library.

use crate::app::*;
use crate::audio::{self, Sound};
use crate::repack::Repack;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tr::tr;

/// A repack found, with its catalog release when one has its name (for its art and details).
#[derive(Clone, Debug)]
pub struct RepackV {
    pub r: Repack,
    pub cat: Option<usize>,
}

/// The repack being installed.
pub struct RepackJob {
    pub dir: PathBuf,
    pub name: String,
    /// 0: checking its files, 1: installing.
    pub phase: u8,
    pub frac: f32,
    pub cancel: Arc<AtomicBool>,
}

impl App {
    /// Where repacks are looked for: the downloads folder, the game folders, the top of every drive
    /// the settings know, and the home's Downloads.
    fn repack_roots(&self) -> Vec<PathBuf> {
        let c = self.cfg.lock().unwrap();
        // The drives in the settings (Storage), not every one mounted.
        let mut roots = vec![crate::util::expand_home(&c.download_dir)];
        roots.extend(c.game_dir_paths());
        roots.extend(c.drives.iter().map(|d| PathBuf::from(&d.mount)));
        roots.extend(c.storage.iter().map(|p| crate::util::expand_home(p)));
        roots.push(crate::util::expand_home("~/Downloads"));
        roots.sort();
        roots.dedup();
        roots
    }

    /// Look for repacks, in the background (reading installers to tell Inno ones takes a moment).
    pub fn repack_scan(&mut self) {
        let roots = self.repack_roots();
        std::thread::spawn(move || {
            let found = crate::repack::scan(&roots);
            let _ = slint::invoke_from_event_loop(move || with_app(move |app| app.repacks_found(found)));
        });
    }

    fn repacks_found(&mut self, found: Vec<Repack>) {
        if found.len() != self.repacks.len() { crate::log!("repacks: {} on the drives", found.len()); }
        // One already installed (the Library has a PC game of that name) is a disc already copied.
        let installed: std::collections::HashSet<String> = self.locals.iter()
            .filter(|l| l.l.platform == crate::platform::Platform::Pc).map(|l| crate::util::norm(&l.name)).collect();
        let index = CatalogIndex::new(self.games.iter().map(|g| (&g.g, g.name.as_str())));
        let list: Vec<RepackV> = found.into_iter()
            .filter(|r| !installed.contains(&crate::util::norm(&r.name)) || self.repack_job.as_ref().is_some_and(|j| j.dir == r.dir))
            .map(|r| {
                let probe = crate::library::LocalGame { name: r.name.clone(), platform: crate::platform::Platform::Pc, ..Default::default() };
                RepackV { cat: index.find(&probe), r }
            }).collect();
        // Said once each: a new one found.
        for v in &list {
            let key = format!("repack:{}", v.r.dir.display());
            if self.repack_told.insert(v.r.dir.clone()) && !self.boot_first_scan {
                let place = crate::storage::drive_name(&crate::storage::mount_point(&v.r.dir));
                self.notify(crate::toast::Cat::Installs, crate::toast::INFO, "download", &tr!("{} is ready to install", v.r.name),
                    &tr!("A repack on {} · on Home", place), crate::toast::Action::None, &key, false);
            }
        }
        self.boot_first_scan = false;
        let same = list.len() == self.repacks.len() && list.iter().zip(&self.repacks).all(|(a, b)| a.r == b.r && a.cat == b.cat);
        if same { return; }
        self.repacks = list;
        self.build_row();
        self.push_row();
        self.push_hero();
        if self.view == 0 && self.row_repack(self.sel).is_some() { self.show_row_background(self.sel, true); }
    }

    /// Each repack's catalog release again (the catalog came in after they were found, or changed):
    /// their art and details come from it.
    pub fn repack_relink(&mut self) {
        if self.repacks.is_empty() { return; }
        let index = CatalogIndex::new(self.games.iter().map(|g| (&g.g, g.name.as_str())));
        let mut changed = false;
        for v in &mut self.repacks {
            let probe = crate::library::LocalGame { name: v.r.name.clone(), platform: crate::platform::Platform::Pc, ..Default::default() };
            let cat = index.find(&probe);
            if cat != v.cat { v.cat = cat; changed = true; }
        }
        if changed && self.view == 0 {
            self.push_row();
            if self.row_repack(self.sel).is_some() {
                self.push_hero();
                self.show_row_background(self.sel, true);
            }
        }
    }

    /// The repack row item `i` is, if it is one.
    pub fn row_repack(&self, i: usize) -> Option<usize> {
        match self.row.get(i) { Some(RowItem::Repack(r)) => Some(*r), _ => None }
    }

    /// How far a repack's install is (below 0: not installing).
    pub fn repack_progress(&self, r: usize) -> f32 {
        match (&self.repack_job, self.repacks.get(r)) {
            // Checking the files is the first tenth, unpacking the rest.
            (Some(j), Some(v)) if j.dir == v.r.dir => if j.phase == 0 { j.frac * 0.1 } else { 0.1 + j.frac * 0.9 },
            _ => -1.0,
        }
    }

    /// The line under a repack's name on Home.
    pub fn repack_sub(&self, r: usize) -> String {
        let Some(v) = self.repacks.get(r) else { return String::new() };
        match &self.repack_job {
            Some(j) if j.dir == v.r.dir && j.phase == 0 => tr!("Checking its files · {}%", (j.frac * 100.0) as u32),
            Some(j) if j.dir == v.r.dir => tr!("Copying · {}%", (self.repack_progress(r) * 100.0) as u32),
            _ => {
                let size = crate::storage::size(v.r.size());
                let what = if v.r.maker.is_empty() { tr!("Repack") } else { tr!("{} repack", v.r.maker) };
                let dub = if v.r.brazilian() { format!(" · {}", tr!("Brazilian Portuguese")) } else { String::new() };
                format!("{what} · {size}{dub}")
            }
        }
    }

    /// The hero beside a repack: what it is, and Install (or Cancel while it installs).
    pub fn repack_hero(&self, r: usize, h: &mut crate::HeroData) {
        let Some(v) = self.repacks.get(r) else { return };
        let mk = |id, label: &str, icon, primary| ActionDef { id, label: label.into(), icon, primary, danger: false, round: false };
        let busy = self.repack_job.as_ref().is_some_and(|j| j.dir == v.r.dir);
        let place = crate::storage::drive_name(&crate::storage::mount_point(&v.r.dir));
        h.kicker = if busy { tr!("INSTALLING") } else { tr!("READY TO INSTALL") }.into();
        h.kicker_live = busy;
        h.desc = if busy {
            tr!("It installs on its own: its installer runs out of sight. You can keep using SwarmPlay, even play.")
        } else {
            tr!("A repack on {}. Install checks its files, then installs it without its installer's windows, like copying a disc.", place)
        }.into();
        let mut chips = vec![crate::ChipData { icon: "disk".into(), text: crate::storage::size(v.r.size()).into(), gold: false, dot: 0, stars: 0.0 }];
        if !v.r.maker.is_empty() {
            chips.push(crate::ChipData { icon: "tag".into(), text: tr!("{} repack", v.r.maker).into(), gold: false, dot: 0, stars: 0.0 });
        }
        if v.r.brazilian() {
            chips.push(crate::ChipData { icon: "sound".into(), text: tr!("Brazilian Portuguese").into(), gold: true, dot: 0, stars: 0.0 });
        }
        h.chips = slint::ModelRc::new(slint::VecModel::from(chips));
        let actions = if busy { vec![ActionDef { danger: true, ..mk("repack_cancel", &tr!("Cancel install"), "close", true) }] }
            else { vec![mk("repack_install", &tr!("Install"), "download", true)] };
        h.actions = slint::ModelRc::new(slint::VecModel::from(actions.iter().map(|a| crate::ActionData {
            id: a.id.into(), label: a.label.clone().into(), icon: a.icon.into(), primary: a.primary, danger: a.danger, round: a.round,
        }).collect::<Vec<_>>()));
    }

    pub fn repack_actions(&self, r: usize) -> Vec<ActionDef> {
        let mk = |id, label: &str, icon, primary| ActionDef { id, label: label.into(), icon, primary, danger: false, round: false };
        let busy = self.repacks.get(r).is_some_and(|v| self.repack_job.as_ref().is_some_and(|j| j.dir == v.r.dir));
        if busy { vec![ActionDef { danger: true, ..mk("repack_cancel", &tr!("Cancel install"), "close", true) }] }
        else { vec![mk("repack_install", &tr!("Install"), "download", true)] }
    }

    /// Install pressed: where to? A button for each game folder's drive with room.
    pub fn repack_ask(&mut self, r: usize) {
        let Some(v) = self.repacks.get(r) else { return };
        if self.repack_job.is_some() {
            return self.toast(&tr!("One install at a time"), &tr!("{} is installing", self.repack_job.as_ref().map(|j| j.name.clone()).unwrap_or_default()), "alert", crate::toast::FAILED);
        }
        let need = v.r.size() as f64 / 1e9;
        let (dirs, install) = { let c = self.cfg.lock().unwrap(); (c.game_dir_paths(), crate::util::expand_home(&c.install_dir)) };
        let mut choices: Vec<(PathBuf, String, f64)> = Vec::new();
        for d in std::iter::once(install).chain(dirs) {
            if !crate::storage::drive_connected(&d) { continue; }
            let mount = crate::storage::mount_point(&d);
            if choices.iter().any(|(p, _, _)| crate::storage::mount_point(p) == mount) { continue; }
            let free = crate::pc::free_gb(&d).or_else(|| d.parent().and_then(crate::pc::free_gb)).unwrap_or(0.0);
            choices.push((d, crate::storage::drive_name(&mount), free));
        }
        // Room for at least what's read (an unpacked game is bigger still).
        choices.retain(|(_, _, free)| *free > need);
        if choices.is_empty() {
            return self.toast(&tr!("No drive has room for it"), &tr!("It needs more than {} free", crate::storage::size(v.r.size())), "alert", crate::toast::FAILED);
        }
        self.power_choices = choices.iter().map(|(p, name, free)| (p.clone(), tr!("Install on {} · {} free", name, crate::storage::size((*free * 1e9) as u64)))).collect();
        self.power_repack = r;
        self.power_ask(crate::os::power::Power::Repack);
    }

    /// The card's words: what it is and what happens.
    pub fn repack_card_words(&self) -> (String, String) {
        let Some(v) = self.repacks.get(self.power_repack) else { return (String::new(), String::new()) };
        let mut text = tr!("SwarmPlay checks its files, then installs it out of sight: no installer windows.");
        if v.r.brazilian() { text = format!("{text} {}", tr!("Brazilian Portuguese included.")); }
        (tr!("Install {}?", v.r.name), text)
    }

    /// A drive chosen on the card: install there.
    pub fn repack_start(&mut self, base: PathBuf) {
        let Some(v) = self.repacks.get(self.power_repack).cloned() else { return };
        let dest = base.join(crate::pcgame::folder_name(&v.r.name));
        if crate::pcgame::Manifest::read(&dest).is_some() {
            return self.toast(&tr!("Already installed"), &dest.display().to_string(), "alert", crate::toast::FAILED);
        }
        // The prefix it'll play in (sessions.rs: prefixes/<the game's id>), so what its installer
        // set up there is there for the game.
        let id = crate::util::sha1_hex(&dest.to_string_lossy())[..12].to_string();
        let prefix = crate::util::data_dir().join("prefixes").join(id);
        let topic = v.cat.map(|c| self.games[c].g.id).unwrap_or(0);
        let cancel = Arc::new(AtomicBool::new(false));
        self.repack_job = Some(RepackJob { dir: v.r.dir.clone(), name: v.r.name.clone(), phase: 0, frac: 0.0, cancel: cancel.clone() });
        audio::play(Sound::Select);
        self.repack_changed();
        let r = v.r.clone();
        std::thread::spawn(move || {
            let last = std::sync::Mutex::new((u8::MAX, -1.0f32, std::time::Instant::now()));
            let report = |phase: u8, done: u64, total: u64| {
                let frac = if total == 0 { 0.0 } else { (done as f64 / total as f64).min(0.999) as f32 };
                let mut l = last.lock().unwrap();
                // A few times a second, whatever the files do.
                if l.0 == phase && (frac - l.1).abs() < 0.002 && l.2.elapsed().as_millis() < 700 { return; }
                *l = (phase, frac, std::time::Instant::now());
                let _ = slint::invoke_from_event_loop(move || with_app(move |app| app.repack_progressed(phase, frac)));
            };
            let result = crate::repack::verify(&r, &|d, t| report(0, d, t), &cancel)
                .and_then(|()| crate::repack::install(&r, &dest, &prefix, topic, &|d, t| report(1, d, t), &cancel))
                .map(|()| dest);
            let _ = slint::invoke_from_event_loop(move || with_app(move |app| app.repack_done(result)));
        });
    }

    fn repack_progressed(&mut self, phase: u8, frac: f32) {
        let Some(j) = &mut self.repack_job else { return };
        j.phase = phase;
        j.frac = frac;
        self.repack_changed();
    }

    /// The row, the line under the name and the island, for a change in the install.
    fn repack_changed(&mut self) {
        if self.view == 0 {
            self.push_row();
            if self.row_repack(self.sel).is_some() { self.push_hero(); }
        }
        self.push_ring();
    }

    fn repack_done(&mut self, result: Result<PathBuf, String>) {
        let Some(job) = self.repack_job.take() else { return };
        match result {
            Ok(dest) => {
                crate::log!("repack: {} installed at {}", job.name, dest.display());
                audio::play(Sound::Notify);
                self.notify(crate::toast::Cat::Installs, crate::toast::DONE, "check", &tr!("{} is ready to play", job.name),
                    &tr!("Installed from its repack"), crate::toast::Action::Play(dest.display().to_string()), &format!("repack-done:{}", job.dir.display()), true);
                self.rescan_library();
            }
            Err(e) => {
                crate::log!("repack: {} failed: {e}", job.name);
                if !job.cancel.load(Ordering::Relaxed) {
                    audio::play(Sound::Error);
                    self.notify(crate::toast::Cat::Installs, crate::toast::FAILED, "alert", &tr!("Couldn't install {}", job.name), &e,
                        crate::toast::Action::None, &format!("repack-failed:{}", job.dir.display()), true);
                }
            }
        }
        self.repack_scan();
        self.repack_changed();
    }

    /// Cancel install pressed.
    pub fn repack_cancel(&mut self) {
        if let Some(j) = &self.repack_job {
            j.cancel.store(true, Ordering::Relaxed);
            audio::play(Sound::Back);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn checking_is_the_first_tenth() {
        // repack_progress's split: the bar never goes back when unpacking starts.
        let at = |phase: u8, frac: f32| if phase == 0 { frac * 0.1 } else { 0.1 + frac * 0.9 };
        assert!(at(0, 1.0) <= at(1, 0.0));
        assert!((at(1, 1.0) - 1.0).abs() < 1e-6);
    }
}
