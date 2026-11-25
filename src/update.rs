//! Azrak Launcher self-update from its own GitHub releases.
//!
//! The launcher checks at start and every 6 hours. A newer release is downloaded in the
//! background, verified (SHA-256 from GitHub), test-run, and swapped in place of the running
//! binary (Linux keeps the running program alive until it exits). The new version then starts
//! on the next launch, or right away with **Restart now** (never while a game is running).

use crate::app::*;
use crate::util::{http_json, now_secs};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Upstream: where the game catalog is published.
pub const REPO: &str = "AzrakHQ/azrak-launcher";
/// Where this build updates itself from: SwarmPlay's own repo, so an upstream release never replaces it
/// (copies from before the move asked m1theusr/azrak-launcher; GitHub redirects that here).
/// `AZRAK_UPDATE_REPO` overrides it.
const UPDATE_REPO: &str = "swarmplayhq/swarmplay";
const ASSET: &str = "swarmplay-linux-x86_64.tar.gz";
/// How often a running SwarmPlay asks for a new version (GitHub allows 60 asks an hour).
const CHECK_INTERVAL: f64 = 30.0 * 60.0;

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

/// Version of the running binary. `AZRAK_PRETEND_VERSION` exists for testing updates.
/// The menu icon install.sh puts in place.
/// The menu icon's name (the data folders keep upstream's "azrak": see util::APP_NAME).
pub const ICON_NAME: &str = "swarmplay";
pub const ICON: &[u8] = include_bytes!("../assets/swarmplay.svg");

/// Put the current menu icon in place of an older one. Automatic updates replace only the
/// binary, so without this an updated launcher keeps the icon of the version first installed.
/// Only an icon that install.sh put in the user's data folder is touched.
pub fn refresh_menu_icon() {

    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| crate::util::expand_home("~/.local/share"));
    let hicolor = data.join("icons").join("hicolor");
    let path = hicolor.join("scalable").join("apps").join(format!("{ICON_NAME}.svg"));
    match std::fs::read(&path) {
        Ok(old) if old != ICON => {}
        _ => return,
    }
    if let Err(e) = crate::util::atomic_write(&path, ICON) {
        crate::log!("menu icon update failed: {e}");
        return;
    }
    crate::log!("menu icon updated");
    // Desktops read icons through GTK's cache for that folder; rebuild it so the new one shows.
    let _ = Command::new("gtk-update-icon-cache")
        .args(["-q", "-f", "-t"])
        .arg(&hicolor)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

pub fn current_version() -> String {
    std::env::var("AZRAK_PRETEND_VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string())
}

fn parse(v: &str) -> Vec<u64> {
    v.trim().trim_start_matches('v').split(['.', '-', '+']).take(3).map(|p| p.parse().unwrap_or(0)).collect()
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    parse(candidate) > parse(current)
}

pub fn latest_release() -> Result<Release, String> {
    let repo = std::env::var("AZRAK_UPDATE_REPO").unwrap_or_else(|_| UPDATE_REPO.to_string());
    let v = http_json(&format!("https://api.github.com/repos/{repo}/releases/latest"))?;
    let tag = v["tag_name"].as_str().ok_or_else(|| tr::tr!("no releases found"))?;
    let assets = v["assets"].as_array().cloned().unwrap_or_default();
    let asset = assets.iter().find(|a| a["name"] == ASSET).ok_or_else(|| tr::tr!("this release has no Linux build yet"))?;
    let mut sha256 = asset["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).unwrap_or("").to_string();
    if sha256.is_empty() {
        // Older releases: a separate .sha256 file.
        if let Some(u) = assets.iter().find(|a| a["name"] == format!("{ASSET}.sha256")).and_then(|a| a["browser_download_url"].as_str()) {
            if let Ok(b) = crate::util::http_get(u) {
                sha256 = String::from_utf8_lossy(&b).split_whitespace().next().unwrap_or("").to_string();
            }
        }
    }
    Ok(Release {
        version: tag.trim_start_matches('v').to_string(),
        url: asset["browser_download_url"].as_str().unwrap_or("").to_string(),
        size: asset["size"].as_u64().unwrap_or(0),
        sha256,
    })
}

/// The installed binary, if the launcher may replace it itself.
pub fn replaceable_exe() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // Replaced already by an update waiting for a restart: Linux names it "… (deleted)", the
    // path is still where the newer one goes.
    let exe = PathBuf::from(exe.to_string_lossy().trim_end_matches(" (deleted)")).canonicalize().map_err(|e| e.to_string())?;
    if exe.components().any(|c| c.as_os_str() == "target") && exe.parent().is_some_and(|p| p.ends_with("release") || p.ends_with("debug")) {
        return Err(tr::tr!("running from a source build (update with git pull + ./install.sh)"));
    }
    let dir = exe.parent().ok_or_else(|| tr::tr!("no install folder"))?;
    let c = std::ffi::CString::new(dir.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())?;
    if unsafe { libc::access(c.as_ptr(), libc::W_OK) } != 0 {
        return Err(tr::tr!("{} is not writable (installed system-wide? re-run the install command with sudo)", dir.display()));
    }
    Ok(exe)
}

/// Download, verify, test and swap in the new binary. Returns the installed version.
pub fn install(rel: &Release, exe: &Path, progress: &dyn Fn(String, f32)) -> Result<(), String> {
    let dir = exe.parent().ok_or_else(|| tr::tr!("no install folder"))?;
    // Work next to the binary so the final rename stays on one filesystem.
    let work = dir.join(".azrak-update");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let cleanup = || {
        let _ = std::fs::remove_dir_all(&work);
    };
    let archive = work.join(ASSET);
    let last = std::cell::Cell::new(101u64);
    let res = crate::kyty::download_file(&rel.url, rel.size, &rel.sha256, &archive, &|done, total| {
        let pct = if total > 0 { done * 100 / total } else { 0 };
        if pct != last.get() {
            last.set(pct);
            progress(tr::tr!("Downloading SwarmPlay {} · {}%", rel.version, pct), pct as f32 / 100.0);
        }
    });
    if let Err(e) = res {
        cleanup();
        return Err(e);
    }
    progress(tr::tr!("Installing SwarmPlay {}…", rel.version), 1.0);
    let ok = Command::new("tar").arg("-xzf").arg(&archive).arg("-C").arg(&work).status().is_ok_and(|s| s.success());
    let new_bin = work.join("swarmplay-linux-x86_64").join("swarmplay");
    if !ok || !new_bin.is_file() {
        cleanup();
        return Err(tr::tr!("could not unpack the update"));
    }
    // The new binary must run here and report the expected version.
    let out = Command::new(&new_bin).arg("--version").env_remove("AZRAK_PRETEND_VERSION").output();
    let reported = out.ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    if !reported.ends_with(&rel.version) {
        cleanup();
        return Err(tr::tr!("the new version does not run on this system ({})", if reported.is_empty() { tr::tr!("no output") } else { reported }));
    }
    let _ = std::fs::set_permissions(&new_bin, std::fs::Permissions::from_mode(0o755));
    // Keep the old binary for a manual rollback, then swap atomically.
    let _ = std::fs::copy(exe, dir.join("azrak.previous"));
    let staged = dir.join(".azrak.new");
    std::fs::rename(&new_bin, &staged).map_err(|e| e.to_string())?;
    let r = std::fs::rename(&staged, exe).map_err(|e| e.to_string());
    cleanup();
    r?;
    crate::log!("SwarmPlay updated to {}", rel.version);
    Ok(())
}

// ---------------------------------------------------------------------------- in the app

/// The downloads going on when SwarmPlay restarted itself, for the new one to go on with.
const RESUME_ENV: &str = "SWARMPLAY_RESUME_DOWNLOADS";

#[derive(Default)]
pub struct AppUpdate {
    pub latest: Option<Release>,
    pub checking: bool,
    pub busy: bool,
    pub progress: String,
    pub error: String,
    /// Installed on disk, waiting for a restart.
    pub installed: Option<String>,
    pub last_check: f64,
    /// The version a notification already told about (once per version).
    pub told: String,
    /// How far the download is (0–1), for the top bar's button.
    pub fraction: f32,
    /// The top bar's button as last opened ("kind version"): it opens again only for news.
    pub shown: String,
    /// Bumped each time it opens: a close timer from an older opening leaves it alone.
    pub open_seq: u64,
}

/// How long the top bar's button stays open to say what's new, as a phone's island does.
pub const ISLAND_OPEN: Duration = Duration::from_millis(4200);

fn post(f: impl FnOnce(&mut App) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || with_app(f));
}

impl App {
    pub fn app_update_start(&mut self) {
        self.app_update_check(false);
        let t = slint::Timer::default();
        t.start(slint::TimerMode::Repeated, Duration::from_secs(5 * 60), || {
            with_app(|app| {
                if now_secs() - app.upd.last_check > CHECK_INTERVAL {
                    app.app_update_check(false);
                }
            })
        });
        std::mem::forget(t);
    }

    /// Updates wants you: an update is ready, or installed and waiting for a restart. Shown as a
    /// dot on the top bar's gear and beside Updates in Settings until it's done.
    pub fn updates_need_you(&self) -> bool {
        self.upd.installed.is_some() || self.app_update_available() || self.kyty_update_available()
    }

    pub fn app_update_available(&self) -> bool {
        self.upd.installed.is_none() && self.upd.latest.as_ref().is_some_and(|r| is_newer(&r.version, &current_version()))
    }

    /// One newer still than the update installed and waiting for a restart: it's got the same
    /// way (the restart then goes straight into it).
    fn newer_than_installed(&self, rel: &Release) -> bool {
        self.upd.installed.as_ref().is_some_and(|v| is_newer(&rel.version, v))
    }

    pub fn app_update_check(&mut self, manual: bool) {
        if self.upd.checking || self.upd.busy {
            return;
        }
        self.upd.checking = true;
        self.upd.error.clear();
        self.kyty_refresh_settings();
        std::thread::spawn(move || {
            let res = latest_release();
            post(move |app| {
                app.upd.checking = false;
                app.upd.last_check = now_secs();
                match res {
                    Ok(rel) => {
                        app.upd.latest = Some(rel.clone());
                        // Said once, wherever you are: a new SwarmPlay is out.
                        if app.app_update_available() && app.upd.told != rel.version {
                            app.upd.told = rel.version.clone();
                            let auto = app.cfg.lock().unwrap().app_auto_update;
                            let sub = if auto { tr::tr!("Downloading it now") } else { tr::tr!("Settings → Updates to get it") };
                            app.notify(crate::toast::Cat::Updates, crate::toast::INFO, "download", &tr::tr!("SwarmPlay {} is out", rel.version), &sub,
                                crate::toast::Action::Settings("UPDATES".into()), "app-update", false);
                        }
                        // Otherwise Settings says it: "Update to …" on the row and "1 ready"
                        // beside Updates, or "Up to date".
                        if (app.app_update_available() || app.newer_than_installed(&rel)) && (manual || app.cfg.lock().unwrap().app_auto_update) {
                            app.app_update_install(rel);
                        }
                    }
                    Err(e) => {
                        crate::log!("launcher update check failed: {e}");
                        // A background check that fails (offline) stays in the log; Settings
                        // shows errors only for a check you asked for.
                        if manual {
                            app.upd.error = tr::tr!("Update check failed: {e}", e);
                        }
                    }
                }
                app.kyty_refresh_settings();
            });
        });
    }

    pub fn app_update_install(&mut self, rel: Release) {
        if self.upd.busy {
            return;
        }
        let exe = match replaceable_exe() {
            Ok(p) => p,
            Err(e) => {
                crate::log!("launcher update {} available, not installed: {e}", rel.version);
                self.upd.error = tr::tr!("Can't update automatically: {e}", e);
                self.kyty_refresh_settings();
                return;
            }
        };
        self.upd.busy = true;
        self.upd.error.clear();
        self.kyty_refresh_settings();
        std::thread::spawn(move || {
            let last = std::sync::Mutex::new(String::new());
            let res = install(&rel, &exe, &|p, f| {
                let mut l = last.lock().unwrap();
                if *l != p {
                    *l = p.clone();
                    post(move |app| {
                        app.upd.progress = p.clone();
                        app.upd.fraction = f;
                        if !app.boot.active {
                            app.set_status(&p, true);
                        }
                        app.kyty_refresh_settings();
                    });
                }
            });
            post(move |app| {
                app.upd.busy = false;
                app.upd.fraction = 0.0;
                app.upd.progress.clear();
                app.set_status("", false);
                match res {
                    Ok(()) => {
                        app.upd.installed = Some(rel.version.clone());
                        app.notify(crate::toast::Cat::Updates, crate::toast::DONE, "check", &tr::tr!("SwarmPlay {} is ready", rel.version), &tr::tr!("Restart SwarmPlay to finish"),
                            crate::toast::Action::Restart, "app-update", true);
                        crate::log!("launcher update {} installed", rel.version);
                    }
                    Err(e) => {
                        crate::log!("launcher update failed: {e}");
                        app.upd.error = tr::tr!("Update failed: {e}", e);
                    }
                }
                app.kyty_refresh_settings();
            });
        });
    }

    /// The top bar's update button, left of Search (ui/app.slint's `upd-*`): 0 none, 1 a new
    /// version is out (press: get it), 2 getting it, 3 ready (press: restart into it). When there's
    /// news it opens leftwards to say it, then closes back to its icon. Called every tick.
    pub fn push_update(&mut self) {
        let latest = self.upd.latest.as_ref().map(|r| r.version.clone()).unwrap_or_default();
        let (kind, version) = match &self.upd.installed {
            _ if self.upd.busy => (2, latest),
            Some(v) => (3, v.clone()),
            None if self.app_update_available() => (1, latest),
            None => (0, String::new()),
        };
        let text = match kind {
            1 => tr::tr!("SwarmPlay {} is out", version),
            2 => tr::tr!("Updating to {} · {}%", version, (self.upd.fraction * 100.0).round() as u32),
            3 => tr::tr!("SwarmPlay {} is ready · restart", version),
            _ => String::new(),
        };
        let ui = self.ui();
        ui.set_upd_kind(kind);
        ui.set_upd_text(text.into());
        ui.set_upd_ring(crate::download_ui::ring_path(self.upd.fraction).into());
        // Gone (nothing newer after all) while chosen: Search takes the focus.
        if kind == 0 && self.zone == Z_TOP && self.idx == 7 { self.set_focus(Z_TOP, 5); }
        // News is said where it's seen: not under the start screen or a page over the bar (it
        // waits for them to go).
        let key = format!("{kind} {version}");
        // One island says one thing at a time: the downloads' news first, then this.
        if kind == 0 || key == self.upd.shown || self.boot.active || self.overlay != Overlay::None || ui.get_ring_open() { return; }
        self.upd.shown = key;
        self.upd.open_seq += 1;
        let seq = self.upd.open_seq;
        ui.set_upd_open(true);
        slint::Timer::single_shot(ISLAND_OPEN, move || with_app(move |app| {
            if app.upd.open_seq == seq { app.ui().set_upd_open(false); }
        }));
    }

    /// The top bar's update button pressed: get the new version, or restart into it.
    pub fn update_pressed(&mut self) {
        match (&self.upd.installed, self.upd.latest.clone()) {
            (Some(_), _) if !self.live.is_empty() => {
                self.toast(&tr::tr!("Finish your game first"), &tr::tr!("SwarmPlay restarts into the new version once no game is running"), "alert", crate::toast::FAILED);
            }
            (Some(_), _) => self.power_ask(crate::os::power::Power::App),
            (None, Some(rel)) if !self.upd.busy && self.app_update_available() => {
                crate::audio::play(crate::audio::Sound::Select);
                self.app_update_install(rel);
            }
            // Getting it already: Settings → Updates shows how it goes.
            _ => {
                self.open_settings();
                if let Some(i) = self.settings_headers.iter().position(|h| h == "UPDATES") {
                    self.set_settings_root(false);
                    self.focus_settings_nav(i);
                }
            }
        }
        self.push_update();
    }

    /// Downloads the previous one was getting when it restarted into this one: on again.
    pub fn resume_after_restart(&self) {
        let Ok(keys) = std::env::var(RESUME_ENV) else { return };
        for key in keys.split(',').filter(|k| !k.is_empty()) {
            match self.downloads.action(key, "resume") {
                Ok(()) => crate::log!("download {key} goes on after the restart"),
                Err(e) => crate::log!("couldn't resume {key} after the restart: {e}"),
            }
        }
    }

    /// Start the new version in place of this one (keeps the same window position/flags).
    pub fn app_restart(&mut self) {
        if !self.live.is_empty() {
            self.note_setting(crate::settings::SId::AppUpdate, &tr::tr!("Finish your game first: the launcher can restart once no game is running"), 2);
            return;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        // current_exe() of a replaced binary reads "…/azrak (deleted)": use the path itself.
        let exe = PathBuf::from(exe.to_string_lossy().trim_end_matches(" (deleted)"));
        // Drop --monitor: after a display change the saved setting must win.
        let mut args: Vec<String> = Vec::new();
        let mut it = std::env::args().skip(1);
        while let Some(a) = it.next() {
            if a == "--monitor" {
                it.next();
            } else {
                args.push(a);
            }
        }
        crate::log!("restarting into the new version");
        // Downloads going on now go on in the new one (a launcher opened anew leaves them paused).
        let going: Vec<String> = self.downloads.snapshot().into_iter().filter(|j| j.state.active()).map(|j| j.key).collect();
        self.installer.shutdown();
        self.downloads.shutdown();
        use std::os::unix::process::CommandExt;
        let mut command = Command::new(&exe);
        command.args(&args).env_remove("AZRAK_PRETEND_VERSION").env_remove(RESUME_ENV);
        if !going.is_empty() { command.env(RESUME_ENV, going.join(",")); }
        let err = command.exec();
        let (seed, vpn, upload, bases) = { let c = self.cfg.lock().unwrap(); (c.seed_after_download, c.download_vpn(), c.upload_limit_mb, crate::storage::download_bases(&c)) };
        self.downloads = crate::downloads::Manager::load(seed, vpn, upload, &bases);
        self.installer = crate::installer::Manager::load();
        self.push_downloads();
        self.note_setting(crate::settings::SId::AppUpdate, &tr::tr!("Couldn't restart: {err}", err), 2);
        // Moving to another display restarts too: said where that was pressed.
        // (The note's words up to the display's name, in the language they were written in.)
        let moving = tr::tr!("Moving to {label}: the launcher restarts there", label = '\u{1}');
        let moving = moving.split('\u{1}').next().unwrap_or_default().to_string();
        if self.setting_notes.get(&crate::settings::SId::Display).is_some_and(|(note, _, _)| note.starts_with(&moving)) {
            self.note_setting(crate::settings::SId::Display, &tr::tr!("Couldn't restart: {err}", err), 2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn versions() {
        assert!(is_newer("1.2.0", "1.1.9"));
        assert!(is_newer("v1.10.0", "1.9.3"));
        assert!(!is_newer("1.2.0", "1.2.0"));
        assert!(!is_newer("1.1.0", "1.2.0"));
    }
}
