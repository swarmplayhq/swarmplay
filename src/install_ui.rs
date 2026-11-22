//! Installation is explicit and separate from downloading. The worker never runs payload code.
use crate::{app::*, downloads, installer, util};
use anyhow::Context;
use tr::tr;
use std::path::{Path, PathBuf};

/// Where a finished download installs: where it was set to go while that folder is there,
/// otherwise where new games go now (a drive's folder is gone when the drive was unplugged, or
/// mounted somewhere else since). Never a folder holding the download (a drive's top): its games
/// go in its SwarmPlay/Games, as Storage puts them. A drive that isn't connected says so, rather
/// than filling the system drive under its name.
fn destination(wanted: Option<&Path>, default: &Path, source: &Path) -> anyhow::Result<PathBuf> {
    let chosen = wanted.filter(|p| p.is_dir()).unwrap_or(default);
    anyhow::ensure!(crate::storage::drive_connected(chosen),
        tr!("The drive for new games isn't connected ({}): connect it, or choose another in Settings → Storage", util::display_path(&chosen.to_string_lossy())));
    let real = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    Ok(if real(source).starts_with(real(chosen)) { chosen.join("SwarmPlay").join("Games") } else { chosen.to_path_buf() })
}

impl App {
    pub fn download_for_target(&self, target: Target) -> Option<downloads::Job> {
        let game = &self.games.get(target.game?)?.g;
        let hash = downloads::normalize_magnet(&game.magnet).ok().map(|(key, _)| key);
        self.downloads.snapshot().into_iter().find(|job| job.topic == game.id || hash.as_ref() == Some(&job.key))
    }

    pub fn confirm_install(&mut self) {
        let Some(key) = self.install_pending.clone() else { return; };
        let destination = self.ui().get_download_path().trim().to_owned();
        match self.start_install(&key, Some(util::expand_home(&destination))) {
            Ok(()) => {
                { let mut cfg = self.cfg.lock().unwrap(); cfg.install_dir = destination; cfg.save(); }
                self.install_pending = None;
                self.push_installs();
                self.open_downloads(Some(&key));
            }
            Err(error) => self.ui().set_download_error(format!("{error:#}").into()),
        }
    }

    /// Install a finished download into `wanted` (a folder), or where new games go (see
    /// `destination`). A download whose files were moved is looked for first.
    pub fn start_install(&mut self, key: &str, wanted: Option<PathBuf>) -> anyhow::Result<()> {
        let (default, bases) = { let c = self.cfg.lock().unwrap(); (util::expand_home(&c.install_dir), crate::storage::download_bases(&c)) };
        self.downloads.relocate(&bases);
        let job = self.downloads.job(key).filter(|job| job.state == downloads::State::Complete).with_context(|| tr!("The download isn't finished"))?;
        anyhow::ensure!(job.folder.is_dir(), tr!("Its files aren't where they were downloaded ({}): press Find the files", util::display_path(&job.folder.to_string_lossy())));
        let destination = destination(wanted.as_deref(), &default, &job.folder)?;
        let ids = self.games.iter().find(|game| game.g.id == job.topic).map(|game| {
            std::iter::once(game.g.title_id.clone()).chain(game.g.title_ids.clone())
                .filter(|id| crate::psn::valid_title_id(id)).collect()
        }).unwrap_or_default();
        let game = self.games.iter().find(|game| game.g.id == job.topic);
        let platform = game.map(|game| game.g.platform).or(job.platform).unwrap_or_default();
        // PC games are named after the game (their folder and Library name), not the release.
        let name = match game { Some(game) if platform == crate::platform::Platform::Pc => game.name.clone(), _ => job.name };
        let skip = self.downloads.left_out(key)?;
        let request = installer::Request { key: key.into(), topic: job.topic, name,
            source: job.folder, destination, expected_ids: ids, platform, skip };
        self.installer.start(request)?;
        self.install_failures.remove(key);
        self.install_states.insert(key.into(), installer::State::Inspecting);
        Ok(())
    }

    /// Install pressed (Downloads or the Game Hub): where it was set to install, or where new
    /// games go now; no folder to type. One that can't start waits for Install again, saying why.
    pub fn install_download(&mut self, key: &str) {
        let to = self.downloads.job(key).and_then(|job| job.install_to);
        // Another one installing: this one goes next, by itself (an old record would stop that).
        // Pressed in the Game Hub: what happened is said under its buttons too.
        let game = self.downloads.job(key).and_then(|job| self.games.iter().position(|game| game.g.id == job.topic))
            .map(|g| self.game_key(crate::app::Target { game: Some(g), local: self.games[g].local }));
        if self.installer.snapshot().iter().any(|record| record.state.active()) {
            let _ = self.installer.forget(key);
            self.install_failures.remove(key);
            crate::audio::play(crate::audio::Sound::Select);
            // Its row says "Waiting to install".
            if let Some(game) = &game { self.note_game_brief(game, &tr!("Waiting to install: it starts once the game installing now is done"), false); }
            return self.push_downloads();
        }
        match self.start_install(key, to) {
            Ok(()) => { crate::audio::play(crate::audio::Sound::Select); self.push_installs(); }
            // Its row says "Couldn't install" and why.
            Err(error) => {
                crate::audio::play(crate::audio::Sound::Error);
                if let Some(game) = &game { self.note_game_brief(game, &tr!("Couldn't install: {}", format!("{error:#}")), true); }
                self.install_failures.insert(key.into(), format!("{error:#}"));
                self.downloads.set_install_to(key, None);
            }
        }
    }

    /// Downloads that install by themselves: when nothing is installing, the oldest finished one
    /// that hasn't been installed yet starts. One that failed waits for Retry install.
    pub fn auto_install(&mut self) {
        let installs = self.installer.snapshot();
        // Not while one installs, or while the player is choosing where one goes.
        if installs.iter().any(|record| record.state.active()) || self.install_pending.is_some() { return; }
        let mut jobs = self.downloads.snapshot();
        jobs.sort_by_key(|job| job.added);
        let next = jobs.into_iter().find(|job| job.state == downloads::State::Complete && job.install_to.is_some()
            && !installs.iter().any(|record| record.key == job.key) && !self.install_failures.contains_key(&job.key)
            // Not while the player is choosing what to delete of it.
            && self.dl_delete.as_ref().is_none_or(|(key, _)| *key != job.key));
        let Some(job) = next else { return };
        if let Err(error) = self.start_install(&job.key, job.install_to.clone()) {
            self.install_failures.insert(job.key.clone(), format!("{error:#}"));
            // It waits for Install by hand from now on (not tried again at every start); the
            // ring and its row say so.
            self.downloads.set_install_to(&job.key, None);
        } else {
            self.push_installs();
        }
    }

    pub fn push_installs(&mut self) {
        let records = self.installer.snapshot();
        let mut changed = false;
        let mut register = false;
        for record in &records {
            let previous = self.install_states.insert(record.key.clone(), record.state);
            if previous == Some(record.state) { continue; }
            changed = true;
            // Seen going (not just found that way at start): a notification says how it ended.
            if previous.is_some_and(|p| p.active()) {
                match record.state {
                    installer::State::Installed => self.notify(crate::toast::Cat::Installs, crate::toast::DONE, "play", &tr!("Ready to play"), &record.name,
                        record.path.as_ref().map_or(crate::toast::Action::Downloads(record.key.clone()), |p| crate::toast::Action::Play(p.to_string_lossy().into_owned())),
                        &format!("dl:{}", record.key), true),
                    installer::State::Failed => self.notify(crate::toast::Cat::Installs, crate::toast::FAILED, "alert", &tr!("Couldn't install"), &record.name,
                        crate::toast::Action::Downloads(record.key.clone()), &format!("dl:{}", record.key), true),
                    _ => {}
                }
            }
            if record.state == installer::State::Installed {
                if let Some(path) = record.path.as_ref().filter(|path| crate::library::read_param(path).is_some()) {
                    if let Some(parent) = path.parent() {
                        let mut cfg = self.cfg.lock().unwrap();
                        let folder = parent.to_string_lossy().into_owned();
                        if !cfg.game_dir_paths().iter().any(|existing| existing == parent) {
                            cfg.game_dirs.push(folder); cfg.save();
                        }
                        register = true;
                    }
                }
            }
            // A failed one: the ring and its row say so.
        }
        self.install_states.retain(|key, _| records.iter().any(|record| &record.key == key));
        if register { self.rescan_library(); }
        if changed && self.overlay == Overlay::Hub { self.push_hub(); }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_where_it_was_set_to_while_that_folder_is_there() {
        let temp = tempfile::tempdir().unwrap();
        let (wanted, default, source) = (temp.path().join("ssd/Games"), temp.path().join("home/Games"), temp.path().join("dl/torrent-x"));
        std::fs::create_dir_all(&wanted).unwrap();
        assert_eq!(destination(Some(&wanted), &default, &source).unwrap(), wanted);
        // Its drive mounted somewhere else since (or unplugged): where new games go now.
        std::fs::remove_dir_all(temp.path().join("ssd")).unwrap();
        assert_eq!(destination(Some(&wanted), &default, &source).unwrap(), default);
        assert_eq!(destination(None, &default, &source).unwrap(), default);
    }

    #[test]
    fn never_installs_into_a_folder_holding_the_download() {
        let temp = tempfile::tempdir().unwrap();
        let drive = temp.path().join("drive");
        let source = drive.join("torrent-x");
        std::fs::create_dir_all(&source).unwrap();
        assert_eq!(destination(Some(&drive), &drive, &source).unwrap(), drive.join("SwarmPlay").join("Games"));
    }

    #[test]
    fn a_drive_that_isnt_connected_is_said_not_written_under_its_name() {
        let gone = Path::new("/run/media/nobody-here/unplugged/SwarmPlay/Games");
        let error = destination(None, gone, Path::new("/tmp/torrent-x")).unwrap_err();
        assert!(format!("{error}").contains("isn't connected"));
        assert!(crate::storage::drive_connected(Path::new("/tmp/anything")), "not where drives get mounted");
    }
}
