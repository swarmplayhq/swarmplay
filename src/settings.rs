//! Settings sheet: sections on the left, the chosen one's rows on the right; controller-friendly
//! editing, instant save.

use crate::app::*;
use crate::audio::{self, Sound};
use crate::config::{PRESENT_MODES, RESOLUTIONS};
use crate::util;
use crate::{SettingData, SettingSection};
use tr::tr;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SId {
    Header,
    // System (AzrakOS only; see os/system_ui.rs)
    Internet,
    WifiNet(u16),
    WifiMore,
    BtPair,
    BtFound(u16),
    BtDevice(u16),
    SoundOutput,
    SystemUpdate,
    InstallOs,
    Restart,
    ShutDown,
    // Games
    /// A game folder (src/storage.rs), and the row that adds one with the folder picker.
    GameDir(u16),
    GameDirAdd,
    InstallDir,
    DownloadDir,
    Rescan,
    // Playing
    Fullscreen,
    ReturnOnExit,
    Controls,
    ThisPc,
    // Downloads
    VpnOnly,
    Vpn,
    VpnAdd,
    SeedCompleted,
    Downloads,
    // Updates
    AppUpdate,
    KytyUpdate,
    ShadUpdate,
    AutoUpdate,
    KytyRollback,
    ShadRollback,
    // Notifications (src/toast.rs)
    NotifPopups,
    NotifSecs,
    NotifSound,
    NotifGames,
    NotifDesktop,
    NotifDnd,
    NotifCat(u8),
    // Controllers in Windows games (src/wine_pads.rs)
    WineSdlPads,
    // In-game overlay (src/overlay_ui.rs)
    OverlayHelper,
    OverlayPause,
    // Storage (src/storage.rs)
    StoragePlace(u16),
    /// A line of a drive's legend: games, downloads, media, other.
    StoragePart(u16, u8),
    StorageAdd,
    // Sound (src/sound.rs)
    SoundVolume,
    SoundMute,
    SoundOut,
    // Theme (src/theme.rs)
    ThemeWallpaper,
    ThemeImage,
    ThemeAccent,
    // Launcher
    Display,
    Language,
    /// The name on the avatar (src/onboard.rs types it).
    Name,
    /// The avatar's picture (src/avatar.rs).
    Avatar,
    AvatarColor,
    /// The first-run setup again (src/onboard.rs).
    RunSetup,
    /// Who's playing (src/profiles.rs).
    SwitchUser,
    RemoveUser,
    Sounds,
    Share,
    // Advanced
    Emulator,
    Resolution,
    Present,
    Amd,
    Extra,
    Refresh,
    Quit,
}

/// The avatar's colour as Settings shows it ("Same as accent" when it has none of its own).
pub fn avatar_label(key: &str) -> String {
    if key.is_empty() { tr!("Same as accent") } else { crate::theme::accent_label(key) }
}

fn row(kind: i32, label: &str) -> SettingData {
    SettingData { kind, label: label.into(), ..Default::default() }
}

/// What Settings shows about VPNs, to notice when one connects or drops.
pub fn vpn_signature() -> String {
    crate::vpn::list().iter().map(|v| format!("{}:{} ", v.name, v.connected)).collect()
}

/// A download's progress on its row, which already names the app: "Downloading KytyPS5 0.4 · 45%"
/// → "0.4 · 45%" (from the version on, in any language); without it, the whole text.
fn from_version(progress: &str, version: Option<&str>) -> String {
    match version.filter(|v| !v.is_empty()).and_then(|v| progress.find(v)) {
        Some(i) => progress[i..].to_string(),
        None => progress.to_string(),
    }
}

/// Where a row went after Settings was rebuilt: the same row and name; else the same kind of row
/// with that name (a device that moved in the list); else the same row.
fn same_row(row: (SId, &str), rows: &[(SId, &str)]) -> Option<usize> {
    let kind = std::mem::discriminant(&row.0);
    rows.iter().position(|r| *r == row)
        .or_else(|| rows.iter().position(|r| std::mem::discriminant(&r.0) == kind && r.1 == row.1))
        .or_else(|| rows.iter().position(|r| r.0 == row.0))
}


impl App {
    /// Sections in the order players need them; expert options folded under Advanced.
    pub fn build_settings(&mut self) {
        let cfg = self.cfg.lock().unwrap().clone();
        let mut rows: Vec<(SId, SettingData)> = Vec::new();
        let header = |rows: &mut Vec<(SId, SettingData)>, label: &str| rows.push((SId::Header, row(0, label)));

        rows.extend(self.system_rows());
        header(&mut rows, "GAMES");
        rows.extend(self.game_folder_rows());

        header(&mut rows, "STORAGE");
        rows.extend(self.storage_rows().0);

        header(&mut rows, "PLAYING");
        let mut r = row(2, &tr!("Play games in fullscreen"));
        r.on = cfg.fullscreen;
        rows.push((SId::Fullscreen, r));
        let mut r = row(2, &tr!("Return to the launcher when a game closes"));
        r.on = cfg.return_on_exit;
        rows.push((SId::ReturnOnExit, r));
        let connected = !crate::gamepad::connected().is_empty();
        let mut r = row(4, &tr!("Controller & keyboard"));
        r.value = crate::gamepad::status().into();
        r.value_kind = if connected { 1 } else { 0 };
        r.hint = if connected { tr!("Your controller works in games with no setup") } else { tr!("Games play best with a controller · open to see the keyboard keys") }.into();
        rows.push((SId::Controls, r));
        let mut r = row(2, &tr!("Controller in Windows games"));
        r.on = cfg.wine_sdl_pads;
        r.hint = tr!("Shows any controller (DualShock, DualSense…) to Windows games as an Xbox one · takes effect at the next start").into();
        rows.push((SId::WineSdlPads, r));
        rows.push((SId::ThisPc, self.this_pc_row()));
        rows.extend(self.overlay_rows(&cfg));

        header(&mut rows, "SOUND");
        rows.extend(self.sound_rows(cfg.sounds));

        header(&mut rows, "NOTIFICATIONS");
        rows.extend(self.notify_rows(&cfg));

        header(&mut rows, "DOWNLOADS");
        // VPN: the toggle keeps downloads off every other connection; the row below manages the
        // VPN itself through NetworkManager, or just reports one that another app connected.
        self.vpn_seen = vpn_signature();
        let connected = crate::vpn::active(&cfg.vpn_interface);
        let nm = crate::vpn::nm_available();
        let saved = if nm { crate::vpn::nm_saved() } else { Vec::new() };
        let mut r = row(2, &tr!("Download only through a VPN"));
        r.on = cfg.vpn_only;
        (r.hint, r.hint_kind) = match (&connected, cfg.vpn_only) {
            (_, false) => (tr!("Hides your IP address from other players · uses your own VPN").into(), 0),
            (Some(iface), true) => (tr!("Protected · downloads only use {iface}", iface).into(), 1),
            (None, true) => (tr!("Downloads wait until your VPN is connected").into(), 2),
        };
        rows.push((SId::VpnOnly, r));
        let mut r = row(4, &tr!("Your VPN"));
        let active_saved = saved.iter().find(|v| v.active);
        (r.value, r.value_kind, r.hint) = if !self.vpn_busy.is_empty() {
            (self.vpn_busy.clone().into(), 0, "".into())
        } else if let Some(v) = active_saved {
            (tr!("{} · Connected", v.name).into(), 1, tr!("Press to disconnect").into())
        } else if let Some(iface) = &connected {
            (tr!("{} · Connected", iface).into(), 1, tr!("Connected by your VPN app").into())
        } else if let Some(v) = saved.first() {
            (tr!("{} · Connect", v.name).into(), 3, tr!("Not connected").into())
        } else if nm {
            (tr!("Add one").into(), 3, tr!("From your VPN provider: a WireGuard .conf or OpenVPN .ovpn file").into())
        } else {
            (tr!("None").into(), 0, tr!("Connect your VPN app, and downloads use it").into())
        };
        rows.push((SId::Vpn, r));
        // Without a file dialog, its path is typed here (even for the first one).
        let dialog = crate::vpn::has_file_dialog();
        if nm && (!saved.is_empty() || !dialog) {
            let mut r = row(if dialog { 4 } else { 1 }, &tr!("Add a VPN from a file"));
            r.value = if dialog { "WireGuard .conf · OpenVPN .ovpn".into() } else { "".into() };
            r.hint = if dialog { tr!("Download the file from your VPN provider's website") } else { tr!("Type the path of a WireGuard .conf or OpenVPN .ovpn file") }.into();
            rows.push((SId::VpnAdd, r));
        }
        let mut r = row(2, &tr!("Keep sharing finished downloads"));
        r.on = cfg.seed_after_download;
        r.hint = tr!("Helps other players download faster · uploads at most 128 KiB/s").into();
        rows.push((SId::SeedCompleted, r));
        // No controller button opens Downloads (the top bar's icon does), so a controller player
        // just presses this row; Ctrl+D is only worth showing to keyboard players.
        let mut r = row(4, &tr!("Open downloads"));
        r.value = if self.pad_hints { tr!("Open").into() } else { tr!("Open · Ctrl+D").into() };
        rows.push((SId::Downloads, r));

        header(&mut rows, "UPDATES");
        // Azrak Launcher
        let cur = crate::update::current_version();
        let latest = self.upd.latest.as_ref().map(|r| r.version.clone());
        let mut r = row(4, "SwarmPlay");
        r.hint = tr!("This app").into();
        let (value, kind) = if let Some(v) = &self.upd.installed {
            (tr!("Restart to finish · {v}", v), 3)
        } else if self.upd.busy {
            (from_version(&self.upd.progress, latest.as_deref()), 0)
        } else if self.upd.checking {
            (tr!("Checking…"), 0)
        } else if self.app_update_available() {
            (latest.map(|l| tr!("Update to {l}", l)).unwrap_or_default(), 3)
        } else if latest.is_some() {
            (tr!("{cur} · Up to date", cur), 1)
        } else {
            (cur.to_string(), 0)
        };
        (r.value, r.value_kind) = (value.into(), kind);
        if !self.upd.error.is_empty() {
            (r.hint, r.hint_kind) = (self.upd.error.clone().into(), 2);
        }
        rows.push((SId::AppUpdate, r));

        // KytyPS5
        let ok = cfg.emulator_ok();
        let managed = self.kyty_managed();
        let kyty_state = crate::kyty::load_state();
        let mut r = row(4, "KytyPS5");
        r.hint = tr!("Runs PS5 games").into();
        let (value, kind) = if self.kyty.busy {
            (from_version(&self.kyty.progress, self.kyty.latest.as_ref().map(|l| crate::kyty::pretty(&l.tag)).as_deref()), 0)
        } else if self.kyty.checking {
            (tr!("Checking…"), 0)
        } else if !managed && !ok {
            (tr!("Install"), 3)
        } else if !managed {
            r.hint = tr!("Runs PS5 games · you use your own build{} · switching keeps your saves",
                self.kyty.version.strip_prefix("Self-built").map(|v| format!(" ({})", v.trim())).unwrap_or_default()).into();
            (tr!("Switch to official builds"), 3)
        } else if self.kyty_update_available() {
            (self.kyty.latest.as_ref().map(|l| tr!("Update to {l}", l = crate::kyty::pretty(&l.tag))).unwrap_or_default(), 3)
        } else if self.kyty.latest.is_some() {
            (tr!("{cur} · Up to date", cur = crate::kyty::pretty(&kyty_state.installed)), 1)
        } else {
            (crate::kyty::pretty(&kyty_state.installed), 0)
        };
        (r.value, r.value_kind) = (value.into(), kind);
        if !self.kyty.error.is_empty() {
            (r.hint, r.hint_kind) = (self.kyty.error.clone().into(), 2);
        }
        rows.push((SId::KytyUpdate, r));

        // shadPS4
        let shad_state = crate::shad::load_state();
        let shad_installed = crate::shad::installed();
        let mut r = row(4, "shadPS4");
        r.hint = if shad_installed {
            tr!("Runs PS4 games")
        } else if cfg.shad_auto_update {
            tr!("Runs PS4 games · installs by itself in the background")
        } else {
            tr!("Runs PS4 games · installs when you first play one")
        }
        .into();
        let (value, kind) = if self.shad.busy {
            (self.shad.progress.clone(), 0)
        } else if !shad_installed {
            (tr!("Install"), 3)
        } else if shad_state.latest == shad_state.installed {
            (tr!("{cur} · Up to date", cur = crate::shad::pretty(&shad_state.installed)), 1)
        } else {
            (crate::shad::pretty(&shad_state.installed), 0)
        };
        (r.value, r.value_kind) = (value.into(), kind);
        if !self.shad.error.is_empty() {
            (r.hint, r.hint_kind) = (self.shad.error.clone().into(), 2);
        }
        rows.push((SId::ShadUpdate, r));

        let mut r = row(2, &tr!("Update automatically"));
        r.on = cfg.app_auto_update && cfg.kyty_auto_update && cfg.shad_auto_update;
        r.hint = tr!("Checks every few hours · updates wait until you close your game").into();
        rows.push((SId::AutoUpdate, r));
        // Rollbacks only when there is something to go back to.
        let mut rollback_hint = tr!("For when an update breaks a game");
        if managed && !kyty_state.previous.is_empty() && crate::kyty::root().join("versions").join(&kyty_state.previous).is_dir() {
            let mut r = row(4, &tr!("Go back to the previous KytyPS5"));
            r.value = crate::kyty::pretty(&kyty_state.previous).into();
            r.hint = std::mem::take(&mut rollback_hint).into();
            rows.push((SId::KytyRollback, r));
        }
        if !shad_state.previous.is_empty() && crate::shad::root().join("versions").join(&shad_state.previous).is_dir() {
            let mut r = row(4, &tr!("Go back to the previous shadPS4"));
            r.value = crate::shad::pretty(&shad_state.previous).into();
            r.hint = rollback_hint.into();
            rows.push((SId::ShadRollback, r));
        }

        header(&mut rows, "THEME");
        rows.extend(self.theme_rows());

        header(&mut rows, "LAUNCHER");
        let mut r = row(3, &tr!("Language"));
        r.value = crate::i18n::current().name().into();
        rows.push((SId::Language, r));
        let mut r = row(4, &tr!("Your name"));
        r.value = self.player_name().into();
        r.hint = tr!("On your avatar and the start screen").into();
        rows.push((SId::Name, r));
        let mut r = row(4, &tr!("Avatar"));
        r.value = if cfg.avatar_image.is_empty() { tr!("Your initial") } else { tr!("A picture") }.into();
        r.hint = tr!("A game's art, your account's picture or one of yours").into();
        rows.push((SId::Avatar, r));
        let mut r = row(3, &tr!("Avatar colour"));
        r.value = avatar_label(&cfg.avatar_color).into();
        rows.push((SId::AvatarColor, r));
        let mut r = row(3, &tr!("Display"));
        r.value = match self.monitors.iter().find(|m| m.name == cfg.monitor) {
            Some(m) => format!("{} · {}×{}", m.name, m.w, m.h),
            None => tr!("Primary display"),
        }
        .into();
        rows.push((SId::Display, r));
        let unshared = self.my_results.unshared().len();
        let mut r = row(4, &tr!("Share your game ratings"));
        (r.value, r.value_kind) = if unshared > 0 { (tr!("{unshared} to share", unshared).into(), 3) } else { (tr!("Nothing new").into(), 0) };
        r.hint = if self.my_results.games.is_empty() {
            tr!("Rate a game from its Options menu after you play it, then share it here").into()
        } else {
            tr!("Tells the emulator teams which games work on Linux · needs a free GitHub account").into()
        };
        rows.push((SId::Share, r));
        let mut r = row(4, &tr!("Switch user"));
        r.hint = tr!("Back to who's playing · everyone has their own name, avatar, theme and language").into();
        rows.push((SId::SwitchUser, r));
        if cfg.profiles.len() > 1 {
            let mut r = row(4, &tr!("Remove this user"));
            r.danger = true;
            r.hint = tr!("{}'s name, avatar and theme go · games and downloads stay", self.player_name()).into();
            rows.push((SId::RemoveUser, r));
        }
        let mut r = row(4, &tr!("Set up again"));
        r.hint = tr!("The first-run setup: language, controller, look, sound, where games go").into();
        rows.push((SId::RunSetup, r));

        // Its own section: out of the way without a switch to show it.
        header(&mut rows, "ADVANCED");
        rows.extend(self.storage_folder_rows());
        {
            let mut r = row(1, &tr!("KytyPS5 location"));
            r.value = util::display_path(&cfg.emulator).into();
            (r.hint, r.hint_kind) = if ok { (tr!("Found").into(), 1) } else { (tr!("Nothing runnable at this path").into(), 2) };
            rows.push((SId::Emulator, r));
            let mut r = row(3, &tr!("Resolution (PS5 games)"));
            r.value = format!("{} × {}", cfg.width, cfg.height).into();
            rows.push((SId::Resolution, r));
            let mut r = row(3, &tr!("Present mode (PS5 games)"));
            r.value = crate::config::present_label(&cfg.present_mode).into();
            r.hint = tr!("Try V-Sync if the picture tears").into();
            rows.push((SId::Present, r));
            let mut r = row(2, &tr!("AMD CPU patches (PS5 games)"));
            r.on = cfg.amd_cpu;
            r.hint = tr!("Only for AMD processors").into();
            rows.push((SId::Amd, r));
            let mut r = row(1, &tr!("Extra KytyPS5 arguments"));
            r.value = cfg.extra_args.clone().into();
            r.hint = tr!("For example --vblank-frequency 60 --tessellation").into();
            rows.push((SId::Extra, r));

            let mut r = row(4, &tr!("Check for new releases"));
            r.value = if self.syncing {
                tr!("Reloading…").into()
            } else {
                let snapshot = self.games.first().map(|game| util::fmt_date(util::parse_iso_date(&game.g.peers_observed))).unwrap_or_default();
                let snapshot = if snapshot.is_empty() { tr!("date unknown") } else { snapshot };
                tr!("{n} release · {date}" | "{n} releases · {date}" % self.games.len(), date = snapshot).into()
            };
            r.hint = tr!("Checks by itself every 6 hours · the list is updated daily, with seeder counts from that day").into();
            rows.push((SId::Refresh, r));
        }

        header(&mut rows, "");
        if crate::os::system::on_os() {
            // The launcher is the whole screen here: quitting it would only start it again.
            rows.push((SId::Restart, row(4, &tr!("Restart"))));
            let mut r = row(4, &tr!("Shut down"));
            r.danger = true;
            rows.push((SId::ShutDown, r));
        } else {
            let mut r = row(4, &tr!("Quit SwarmPlay"));
            r.value = "Ctrl+Q".into();
            r.danger = true;
            rows.push((SId::Quit, r));
        }

        // Each header starts a section (the left column); the last, unnamed one holds Quit (or
        // Restart and Shut down), under the sections.
        let mut titles: Vec<String> = Vec::new();
        let mut cur = -1;
        for (_, r) in rows.iter_mut() {
            if r.kind == 0 {
                cur = if r.label.is_empty() { -1 } else { titles.push(r.label.to_string()); titles.len() as i32 - 1 };
            }
            r.section = cur;
        }
        titles.push("ABOUT".into());
        self.settings_sections = titles.iter().map(|t| self.section_info(t)).collect();
        self.settings_headers = titles;
        self.settings_section = self.settings_section.min(self.settings_sections.len() - 1);
        // What a row just did, in place of its hint (see App::note_setting): on the row of that
        // name in a list (a network, a device). What just happened (good or bad) covers a problem
        // the row shows itself; a passing remark doesn't.
        for (id, r) in rows.iter_mut() {
            let found = self.setting_notes.iter().find(|(nid, (_, _, label))| note_row(**nid, label, *id, &r.label));
            if let Some((_, (note, note_kind, _))) = found {
                if r.hint_kind != 2 || *note_kind != 0 { (r.hint, r.hint_kind) = (note.into(), *note_kind); }
            }
        }
        self.settings_ids = rows.iter().map(|(id, _)| *id).collect();
        self.settings_rows = rows.into_iter().map(|(_, r)| r).collect();
    }

    /// Row `i` by what it is (its id and name), to find it again after the rows are rebuilt.
    pub fn settings_row(&self, i: i32) -> Option<(SId, String)> {
        let i = usize::try_from(i).ok()?;
        Some((*self.settings_ids.get(i)?, self.settings_rows.get(i)?.label.to_string()))
    }

    /// Where that row is now; none for a network or device that left the list (what took its
    /// place isn't what was pressed).
    pub fn settings_row_now(&self, row: &(SId, String)) -> Option<usize> {
        let rows: Vec<(SId, &str)> = self.settings_ids.iter().zip(&self.settings_rows).map(|(id, r)| (*id, r.label.as_str())).collect();
        same_row((row.0, &row.1), &rows).filter(|&i| note_row(row.0, &row.1, rows[i].0, rows[i].1))
    }

    /// What row `i` said last time goes (it's being pressed again).
    fn forget_setting_note(&mut self, i: usize) {
        let (Some(id), Some(r)) = (self.settings_ids.get(i).copied(), self.settings_rows.get(i)) else { return };
        let label = r.label.to_string();
        self.setting_notes.retain(|nid, (_, _, l)| !note_row(*nid, l, id, &label));
    }

    /// A section's name in the left column, what it's for, and a note beside it (what needs you).
    fn section_info(&self, header: &str) -> SettingSection {
        let label = section_title(header);
        let (sub, (note, note_kind)) = match header {
            "SYSTEM" => (tr!("Internet, Bluetooth and updates"), (String::new(), 0)),
            "GAMES" => (tr!("Where your games live"), (String::new(), 0)),
            "PLAYING" => (tr!("What happens when you start a game"), match crate::gamepad::connected().first() {
                Some(name) => (name.clone(), 0),
                None => (String::new(), 0),
            }),
            "DOWNLOADS" => (tr!("How games are downloaded and shared"), {
                let active = self.downloads.snapshot().iter().filter(|j| j.state.active()).count();
                if active > 0 { (tr!("{active} active", active), 1) } else { (String::new(), 0) }
            }),
            "UPDATES" => (tr!("The launcher and its emulators"), {
                let ready = [self.app_update_available(), self.kyty_update_available()].iter().filter(|r| **r).count();
                // Installed and waiting: a restart is what's needed.
                if self.upd.installed.is_some() { (tr!("Restart"), 2) }
                else if ready > 0 { (tr!("{ready} ready", ready), 2) } else { (String::new(), 0) }
            }),
            "THEME" => (tr!("Wallpaper and accent colour"), (String::new(), 0)),
            "STORAGE" => (tr!("Where games download and install, and the room left"), match self.storage_places().into_iter().find(|p| p.default) {
                Some(p) if p.total > 0 => (tr!("{gb} GB free", gb = crate::i18n::num(p.free as f64 / (1u64 << 30) as f64, 0)), if p.free < 50 << 30 { 2 } else { 0 }),
                _ => (String::new(), 0),
            }),
            "NOTIFICATIONS" => (tr!("What lets you know, and when"), if cfg_dnd(self) { (tr!("Do not disturb"), 2) } else { (String::new(), 0) }),
            "SOUND" => (tr!("Volume and where sound goes"), if self.sound.muted { (tr!("Muted"), 2) } else { (self.sound.volume.map(|v| format!("{v}%")).unwrap_or_default(), 0) }),
            "LAUNCHER" => (tr!("Language, you, display and your game ratings"), (String::new(), 0)),
            "ADVANCED" => (tr!("Folders of your own, KytyPS5 location and video options, the game catalog"), (String::new(), 0)),
            _ => (String::new(), (String::new(), 0)),
        };
        let sub = if header == "ABOUT" { format!("SwarmPlay {}", crate::update::current_version()) } else { sub };
        // Its icon on the first page (ui/theme.slint Icons.get).
        let icon = match header {
            "SYSTEM" => "wifi", "GAMES" => "grid", "PLAYING" => "gamepad", "STORAGE" => "disk", "SOUND" => "sound", "NOTIFICATIONS" => "bell", "DOWNLOADS" => "download",
            "UPDATES" => "retry", "THEME" => "palette", "LAUNCHER" => "gear", "ADVANCED" => "log", _ => "info",
        };
        SettingSection { label: label.into(), sub: sub.into(), note: note.into(), note_kind, icon: icon.into() }
    }

    /// The launcher's version and whether it's up to date, under the sections.
    fn version_line(&self) -> String {
        let cur = crate::update::current_version();
        if self.upd.installed.is_some() { tr!("SwarmPlay {cur} · restart to update", cur) }
        else if self.app_update_available() { tr!("SwarmPlay {cur} · update ready", cur) }
        else if self.upd.latest.is_some() { tr!("SwarmPlay {cur} · up to date", cur) }
        else { format!("SwarmPlay {cur}") }
    }

    /// The rows under the sections (Quit, or Restart and Shut down), by row number.
    fn settings_footer(&self) -> Vec<usize> {
        self.settings_rows.iter().enumerate().filter(|(_, r)| r.kind != 0 && r.section < 0).map(|(i, _)| i).collect()
    }

    /// The rows of the chosen section that can have focus, by row number.
    fn section_rows(&self) -> Vec<usize> {
        let s = self.settings_section as i32;
        self.settings_rows.iter().enumerate().filter(|(_, r)| r.kind != 0 && r.section == s).map(|(i, _)| i).collect()
    }

    /// Focus a row, showing its section.
    pub fn focus_setting_row(&mut self, i: usize) {
        if let Some(s) = self.settings_rows.get(i).map(|r| r.section).filter(|s| *s >= 0) {
            self.set_settings_root(false);
            self.settings_section = s as usize;
            self.set_focus(Z_SETTINGS, i as i32);
            self.push_settings_nav();
            self.scroll_settings();
        }
    }

    /// The left column: section `i`, or a row under them (Quit) past the last section. Silent:
    /// callers play the sound that fits (Move, or Back from the rows).
    pub fn focus_settings_nav(&mut self, i: usize) {
        self.settings_direct = false;
        let n = self.settings_sections.len();
        if i < n && i != self.settings_section {
            self.settings_section = i;
            self.ui().set_settings_y(0.0);
        }
        self.set_focus(Z_SETTINGS_NAV, i as i32);
        self.push_settings_nav();
    }

    /// The notes beside the sections ("1 active", "1 ready") as they are now.
    pub fn refresh_settings_notes(&mut self) {
        let sections: Vec<SettingSection> = self.settings_headers.iter().map(|h| self.section_info(h)).collect();
        if sections != self.settings_sections {
            self.settings_sections = sections;
            self.ui().set_settings_sections(model(self.settings_sections.clone()));
        }
    }

    /// The glass behind the sheet: the art behind it, blurred (none over the dimmed Library).
    pub fn push_settings_glass(&mut self, make: bool) {
        // Over Home only: over another page (Downloads) the art behind isn't Home's.
        let over_home = self.stack.last().is_some_and(|(overlay, _, _)| *overlay == Overlay::None);
        let Ok(glass) = (if over_home { self.bg_glass(make) } else { Ok(None) }) else { return };
        let ui = self.ui();
        ui.set_settings_has_glass(glass.is_some());
        ui.set_settings_glass(glass.unwrap_or_default());
    }

    /// The first page (the list of sections, Quit under them) or a section's page.
    pub fn set_settings_root(&mut self, on: bool) {
        self.settings_root = on;
        self.ui().set_settings_root(on);
    }

    pub fn act_settings_nav(&mut self, a: Act) {
        let n = self.settings_sections.len();
        let footer = self.settings_footer();
        let i = self.idx.max(0) as usize;
        // A section's page lists the sections only; the first page has Quit under them too.
        let last = if self.settings_root { n + footer.len() } else { n };
        match a {
            // Back from a section's page is its first page; from there it closes.
            Act::Back if !self.settings_root => { audio::play(Sound::Back); self.set_settings_root(true); }
            Act::Back => self.back_soft(),
            Act::Up if i > 0 => { audio::play(Sound::Move); self.focus_settings_nav(i - 1); }
            Act::Down if i + 1 < last => { audio::play(Sound::Move); self.focus_settings_nav(i + 1); }
            Act::Right | Act::Confirm if i < n => {
                audio::play(Sound::Select);
                match self.section_rows().first() {
                    Some(&j) => self.focus_setting_row(j),
                    // About: its page, nothing to choose on it.
                    None => self.set_settings_root(false),
                }
            }
            Act::Confirm => {
                if let Some(&j) = footer.get(i - n) { self.settings_activate(j); }
            }
            _ => {}
        }
    }

    pub fn act_settings_rows(&mut self, a: Act) {
        let rows = self.section_rows();
        let pos = rows.iter().position(|&j| j as i32 == self.idx);
        let kind = self.settings_rows.get(self.idx.max(0) as usize).map_or(0, |r| r.kind);
        let to_nav = |app: &mut App| { audio::play(Sound::Back); let s = app.settings_section; app.focus_settings_nav(s); };
        match a {
            // Back leaves a section's page for the first page; Back there closes Settings.
            // Opened onto a row from elsewhere (VPN settings on Downloads), Back goes straight
            // back there.
            Act::Back if self.settings_direct => self.back_soft(),
            Act::Back => { to_nav(self); self.set_settings_root(true); }
            Act::Up | Act::Down => {
                let next = match (pos, a) {
                    (Some(p), Act::Up) => p.checked_sub(1),
                    (Some(p), _) => Some(p + 1),
                    (None, _) => Some(0),
                };
                if let Some(&j) = next.and_then(|p| rows.get(p)) {
                    self.move_focus(Z_SETTINGS, j as i32);
                    self.scroll_settings();
                }
            }
            // Left and Right step a choice (its ‹ › show it); anywhere else Left goes back to the
            // sections. A switch changes with ✕ only (Left never turns one off by surprise).
            Act::Left | Act::Right if kind == 3 => self.settings_change(self.idx as usize, if a == Act::Left { -1 } else { 1 }),
            Act::Left => to_nav(self),
            Act::Confirm => self.settings_activate(self.idx as usize),
            // □ on a place that was added, or a game folder: off the list.
            Act::Trailer => match self.settings_ids.get(self.idx.max(0) as usize).copied() {
                Some(SId::StoragePlace(n)) => self.storage_remove(n as usize),
                Some(SId::GameDir(n)) => self.game_dir_remove(n as usize),
                _ => {}
            },
            _ => {}
        }
    }

    /// The left column's state (which section, its title, the rows under the sections).
    pub fn push_settings_nav(&mut self) {
        let ui = self.ui();
        let s = self.settings_sections.get(self.settings_section).cloned().unwrap_or_default();
        ui.set_settings_section(self.settings_section as i32);
        ui.set_settings_title(s.label);
        ui.set_settings_sub(s.sub);
    }

    /// Open Settings with this row selected.
    pub fn open_setting(&mut self, id: SId) {
        self.open_settings();
        if let Some(i) = self.settings_ids.iter().position(|s| *s == id) {
            self.focus_setting_row(i);
            // Opened onto this row from elsewhere: Back returns there.
            self.settings_direct = true;
        }
    }

    pub fn push_settings(&mut self) {
        let footer: Vec<SettingData> = self.settings_footer().into_iter().map(|i| self.settings_rows[i].clone()).collect();
        self.push_settings_glass(true);
        let ui = self.ui();
        ui.set_settings(model(self.settings_rows.clone()));
        ui.set_settings_sections(model(self.settings_sections.clone()));
        ui.set_settings_footer(model(footer));
        ui.set_settings_version(self.version_line().into());
        ui.set_edit_index(self.edit_index);
        self.push_settings_nav();
    }

    pub fn scroll_settings(&mut self) {
        // Estimated row heights matching app.slint (layout units). Text follows the text scale,
        // which on small windows is larger than the layout scale.
        let r = (self.scale.max(crate::present::TEXT_SCALE_MIN) / self.scale).max(1.0);
        let (mut y, mut target) = (0.0, 0.0);
        for i in self.section_rows() {
            if i as i32 == self.idx {
                target = y;
            }
            let row = &self.settings_rows[i];
            // Hints wrap at about 95 characters (the column's width at 17 px), or about 55 beside
            // a long value (a folder takes up to half the row).
            // Text grows faster than the column on small windows (r), so fewer fit on a line.
            let per_line = (if row.kind == 1 || row.value.chars().count() > 24 { 55.0 } else { 95.0 } / r).max(10.0) as usize;
            let hint_lines = row.hint.chars().count().div_ceil(per_line) as f32;
            // A drive (its bar) and its legend's lines have heights of their own (storage.rs).
            if row.kind == 5 { y += 80.0 + 48.0 * r + 4.0; continue; }
            if row.kind == 6 { y += if row.hint.is_empty() { 62.0 } else { 46.0 + 42.0 * r } + 4.0; continue; }
            let h = 32.0 + 26.0 * r + if hint_lines > 0.0 { 5.0 + hint_lines * 22.0 * r } else { 0.0 } + if self.edit_index == i as i32 { 60.0 * r } else { 0.0 };
            y += f32::max(h, 84.0) + 10.0;
        }
        // The rows' window on the sheet (as tall as the window allows): under the title, above
        // the button hints. A little room to spare at the end, as the heights are estimates.
        let (_, h) = self.logical_size();
        let view = f32::min(960.0, h - 40.0) - 48.0 - (42.0 + 24.0) * r - 8.0 - 26.0 - 26.0 * r - 48.0;
        let max = (y + 24.0 - view).max(0.0);
        self.ui().set_settings_y(-(target - view * 0.35).clamp(0.0, max) * self.scale);
    }

    /// The language changed: everything already handed to the window is written again in it (the
    /// window's own texts switched by themselves). Panels that are closed build in it when opened;
    /// the clock, downloads, installs and the battery are rewritten by the tick anyway.
    pub fn relocalize(&mut self) {
        // Settings first (it's where the language is chosen): its rows, the sections' titles,
        // subtitles and notes, the version line, the title of the page that's open. The selection
        // stays on the Language row (same_row finds a row by its id once its name changed).
        self.refresh_settings();
        self.push_settings_nav();
        // Home's row and hero, the Library (sort names, genres, the list's date), the Game Hub.
        self.push_all();
        self.push_welcome();
        self.push_downloads();
        self.push_ring();
        // The pop-ups, the Control Center's last notification, the list.
        self.push_notes();
        self.push_controller_status();
        // The games' descriptions and genres in the new language (Portuguese from the BR store),
        // fetched in the background for whatever isn't cached yet in it.
        self.start_enrich();
    }

    pub(crate) fn save_cfg(&mut self, f: impl FnOnce(&mut crate::config::Config)) {
        let mut c = self.cfg.lock().unwrap();
        f(&mut c);
        c.save();
    }

    pub fn refresh_settings(&mut self) {
        // Not while a row is being typed in: new rows would make its field again (and select its
        // text). Finishing the edit rebuilds them.
        if self.edit_index >= 0 { return; }
        // Rows come and go (the Wi-Fi list, devices found or paired): the selection stays on the
        // same row, not at the same place, or the next press would land on a stranger.
        let before: Vec<(SId, String)> = self.settings_ids.iter().zip(&self.settings_rows).map(|(id, r)| (*id, r.label.to_string())).collect();
        self.build_settings();
        self.push_settings();
        // The setup shows these same rows: a network found, a device paired, a note, all there too.
        if self.in_setup() {
            self.onboard_push();
        }
        if self.overlay != Overlay::Settings || self.zone != Z_SETTINGS {
            return;
        }
        let after: Vec<(SId, &str)> = self.settings_ids.iter().zip(&self.settings_rows).map(|(id, r)| (*id, r.label.as_str())).collect();
        // A row that's gone (a network once the list closes) leaves the selection on the nearest
        // row above it that's still there (Internet).
        let Some(i) = before.iter().take(self.idx as usize + 1).rev()
            .filter(|(id, _)| *id != SId::Header)
            .find_map(|(id, label)| same_row((*id, label), &after)) else { return };
        if i as i32 != self.idx {
            self.focus_setting_row(i);
        }
    }

    /// Select a row of Settings while it's open (e.g. Internet once a network is joined).
    pub fn select_setting(&mut self, id: SId) {
        if self.overlay != Overlay::Settings {
            return;
        }
        if let Some(i) = self.settings_ids.iter().position(|s| *s == id) {
            self.focus_setting_row(i);
        }
    }

    pub fn settings_change(&mut self, i: usize, dir: i32) {
        let Some(id) = self.settings_ids.get(i).copied() else { return };
        // Pressed again: what it said last time goes.
        self.forget_setting_note(i);
        if self.system_change(id, dir) {
            return;
        }
        if self.sound_change(id, dir) {
            return;
        }
        // A switch clicks on or off; a choice ticks.
        let kind = self.settings_rows.get(i).map_or(0, |r| r.kind);
        let heard = if kind == 2 { if dir > 0 { Sound::ToggleOn } else { Sound::ToggleOff } } else { Sound::Move };
        if self.notify_change(id, dir) {
            audio::play(heard);
            return self.refresh_settings();
        }
        if self.theme_change(id, dir) {
            audio::play(Sound::Move);
            return self.refresh_settings();
        }
        match id {
            SId::Resolution => {
                let cur = { let c = self.cfg.lock().unwrap(); (c.width, c.height) };
                let pos = RESOLUTIONS.iter().position(|r| *r == cur).unwrap_or(2) as i32;
                let (w, h) = RESOLUTIONS[(pos + dir).rem_euclid(RESOLUTIONS.len() as i32) as usize];
                self.save_cfg(|c| {
                    c.width = w;
                    c.height = h;
                });
            }
            SId::Present => {
                let cur = self.cfg.lock().unwrap().present_mode.clone();
                let pos = PRESENT_MODES.iter().position(|(k, _)| *k == cur).unwrap_or(0) as i32;
                let next = PRESENT_MODES[(pos + dir).rem_euclid(PRESENT_MODES.len() as i32) as usize].0.to_string();
                self.save_cfg(|c| c.present_mode = next);
            }
            SId::AvatarColor => {
                // The accent's own colour first ("Same as accent"), then each colour.
                let names: Vec<&str> = std::iter::once("").chain(crate::theme::ACCENTS.iter().map(|(n, _)| *n)).collect();
                let cur = self.cfg.lock().unwrap().avatar_color.clone();
                let pos = names.iter().position(|n| *n == cur).unwrap_or(0) as i32;
                let next = names[(pos + dir).rem_euclid(names.len() as i32) as usize].to_string();
                self.save_cfg(|c| c.avatar_color = next);
                self.push_user();
            }
            SId::Language => {
                use crate::i18n::Lang;
                let pos = Lang::ALL.iter().position(|l| *l == crate::i18n::current()).unwrap_or(0) as i32;
                let next = Lang::ALL[(pos + dir).rem_euclid(Lang::ALL.len() as i32) as usize];
                self.save_cfg(|c| c.language = next.code().into());
                crate::i18n::set_lang(next);
                self.relocalize();
            }
            SId::Display => {
                let names: Vec<String> = std::iter::once(String::new()).chain(self.monitors.iter().map(|m| m.name.clone())).collect();
                let cur = self.cfg.lock().unwrap().monitor.clone();
                let pos = names.iter().position(|n| *n == cur).unwrap_or(0) as i32;
                let next = names[(pos + dir).rem_euclid(names.len() as i32) as usize].clone();
                self.save_cfg(|c| c.monitor = next.clone());
                self.move_to_monitor(&next);
            }
            SId::VpnOnly => {
                let on = dir > 0;
                let vpn = { let c = self.cfg.lock().unwrap(); on.then(|| c.vpn_interface.clone()) };
                if let Err(error) = self.downloads.set_vpn(vpn) {
                    self.note_setting(SId::VpnOnly, &tr!("Couldn't change it: {error}", error), 2);
                    return;
                }
                self.save_cfg(|c| c.vpn_only = on);
                self.push_downloads();
            }
            SId::SeedCompleted => {
                let on = dir > 0;
                if let Err(error) = self.downloads.set_seed_after_download(on) {
                    self.note_setting(SId::SeedCompleted, &tr!("Couldn't change it: {error}", error), 2);
                    return;
                }
                self.save_cfg(|c| c.seed_after_download = on);
                self.push_downloads();
            }
            SId::Fullscreen | SId::Amd | SId::ReturnOnExit | SId::AutoUpdate | SId::OverlayPause | SId::WineSdlPads => {
                let on = dir > 0;
                self.save_cfg(|c| match id {
                    SId::WineSdlPads => c.wine_sdl_pads = on,
                    SId::OverlayPause => c.overlay_pause = on,
                    SId::Fullscreen => c.fullscreen = on,
                    SId::Amd => c.amd_cpu = on,
                    SId::ReturnOnExit => c.return_on_exit = on,
                    SId::AutoUpdate => (c.app_auto_update, c.kyty_auto_update, c.shad_auto_update) = (on, on, on),
                    _ => {}
                });
                if id == SId::AutoUpdate && on {
                    self.kyty_check(false);
                    self.shad_tick();
                }
            }
            _ => return,
        }
        audio::play(heard);
        self.refresh_settings();
    }

    pub fn settings_activate(&mut self, i: usize) {
        let Some(id) = self.settings_ids.get(i).copied() else { return };
        // Pressed again: what it said last time goes.
        self.forget_setting_note(i);
        if self.system_activate(id, i) {
            return;
        }
        match id {
            SId::StoragePlace(n) => self.storage_use(n as usize),
            SId::StoragePart(n, k) => self.storage_part(n as usize, k as usize),
            SId::StorageAdd => self.open_folder_pick(crate::folder_pick::PickFor::Place),
            SId::GameDir(n) => self.game_dir_pressed(n as usize),
            SId::GameDirAdd => self.open_folder_pick(crate::folder_pick::PickFor::GameFolder),
            SId::InstallDir => self.open_folder_pick(crate::folder_pick::PickFor::Install),
            SId::DownloadDir => self.open_folder_pick(crate::folder_pick::PickFor::Download),
            SId::NotifPopups | SId::NotifSound | SId::NotifDesktop | SId::NotifCat(_) => {
                let on = self.settings_rows[i].on;
                self.settings_change(i, if on { -1 } else { 1 });
            }
            SId::NotifSecs | SId::NotifGames | SId::NotifDnd => self.settings_change(i, 1),
            SId::OverlayHelper => self.overlay_helper_pressed(),
            SId::Name => self.open_name_keys(),
            SId::Avatar => self.open_avatar_pick(),
            SId::SwitchUser => { audio::play(Sound::Select); self.switch_user(); }
            SId::RemoveUser => self.remove_user_pressed(),
            SId::RunSetup => { audio::play(Sound::Select); self.onboard_open(None); }
            SId::Emulator | SId::Extra | SId::ThemeImage => {
                let cfg = self.cfg.lock().unwrap().clone();
                let text = match id {
                    SId::ThemeImage if cfg.theme_image.is_empty() => "~/Pictures/".into(),
                    SId::ThemeImage => cfg.theme_image,
                    SId::Emulator => cfg.emulator,
                    SId::Extra => cfg.extra_args,
                    _ => String::new(),
                };
                audio::play(Sound::Select);
                self.edit_index = i as i32;
                let ui = self.ui();
                ui.set_edit_text(text.into());
                ui.set_edit_index(i as i32);
            }
            SId::Fullscreen | SId::Amd | SId::ReturnOnExit | SId::AutoUpdate | SId::SeedCompleted | SId::VpnOnly | SId::SoundMute
                | SId::OverlayPause | SId::WineSdlPads => {
                let on = self.settings_rows[i].on;
                self.settings_change(i, if on { -1 } else { 1 });
            }
            SId::Resolution | SId::Present | SId::Display | SId::Language | SId::AvatarColor | SId::ThemeWallpaper | SId::ThemeAccent | SId::SoundOut => self.settings_change(i, 1),
            // A press turns it up a step (◂ ▸ go both ways).
            SId::SoundVolume => self.settings_change(i, 1),
            // A press goes round Off, Low, Medium, High.
            SId::Sounds => self.settings_change(i, 2),
            SId::KytyUpdate => {
                audio::play(Sound::Select);
                if self.kyty.busy || self.kyty.checking {
                    return;
                }
                let managed = self.kyty_managed();
                if !managed {
                    self.kyty_switch_to_managed();
                } else if self.kyty_update_available() {
                    if let Some(rel) = self.kyty.latest.clone() {
                        self.kyty_install(rel, true);
                    }
                } else {
                    self.kyty_check(true);
                }
                self.refresh_settings();
            }
            SId::AppUpdate => {
                audio::play(Sound::Select);
                if self.upd.installed.is_some() {
                    self.app_restart();
                } else if self.upd.busy || self.upd.checking {
                    return;
                } else if self.app_update_available() {
                    if let Some(rel) = self.upd.latest.clone() {
                        self.app_update_install(rel);
                    }
                } else {
                    self.app_update_check(true);
                }
                self.refresh_settings();
            }
            SId::KytyRollback => {
                audio::play(Sound::Select);
                self.kyty_rollback();
                self.refresh_settings();
            }
            SId::ShadUpdate => {
                audio::play(Sound::Select);
                // Installs when shadPS4 isn't there yet, otherwise updates it if there's a newer release.
                self.shad_check(true);
            }
            SId::ShadRollback => {
                audio::play(Sound::Select);
                self.shad_rollback();
            }
            SId::Refresh => {
                audio::play(Sound::Select);
                self.start_sync(true);
                self.refresh_settings();
            }
            SId::Rescan => {
                audio::play(Sound::Select);
                self.rescan_library();
                let n = self.locals.len();
                let found = tr!("{n} game found" | "{n} games found" % n);
                self.note_setting(SId::Rescan, &found, 1);
                self.toast(&tr!("Game folders scanned"), &found, "folder", crate::toast::DONE);
            }
            SId::Quit => {
                crate::app::quit("Settings → Quit SwarmPlay");
            }
            SId::Downloads => self.open_downloads(None),
            SId::Vpn => {
                audio::play(Sound::Select);
                if !self.vpn_busy.is_empty() || !crate::vpn::nm_available() {
                    return;
                }
                let saved = crate::vpn::nm_saved();
                if let Some(v) = saved.iter().find(|v| v.active) {
                    let name = v.name.clone();
                    self.vpn_task(SId::Vpn, &tr!("Disconnecting…"), move || crate::vpn::nm_down(&name));
                } else if crate::vpn::active("").is_some() {
                    self.note_setting(SId::Vpn, &tr!("Connected by your VPN app: switch it off there"), 0);
                } else if let Some(v) = saved.first() {
                    let name = v.name.clone();
                    self.vpn_task(SId::Vpn, &tr!("Connecting…"), move || crate::vpn::nm_up(&name));
                } else {
                    self.vpn_add(i);
                }
            }
            SId::VpnAdd => {
                audio::play(Sound::Select);
                self.vpn_add(i);
            }
            SId::Share => self.share_results(),
            SId::Controls => self.open_controls(),
            SId::ThisPc => {
                audio::play(Sound::Select);
                self.pc_check();
                self.refresh_settings();
            }
            _ => {}
        }
    }

    pub fn settings_commit_text(&mut self, i: usize, text: String) {
        let Some(id) = self.settings_ids.get(i).copied() else { return };
        let t = text.trim().to_string();
        match id {
            SId::Emulator => {
                self.save_cfg(|c| c.emulator = t);
                // The row's hint says whether it's runnable (Found, or Nothing runnable at this path).
                self.kyty_refresh_version();
            }
            SId::Extra => self.save_cfg(|c| c.extra_args = t),
            // Left as it opened: nothing chosen.
            SId::ThemeImage if t == "~/Pictures/" => {}
            SId::ThemeImage => self.theme_image_typed(&t),
            // Left as it opened: nothing chosen. A folder: not the file yet.
            SId::VpnAdd if t == "~/Downloads/" => {}
            SId::VpnAdd if util::expand_home(&t).is_dir() => {
                self.note_setting(SId::VpnAdd, &tr!("Type the file itself, like ~/Downloads/wg0.conf"), 0);
            }
            SId::VpnAdd if !t.is_empty() => {
                let file = util::expand_home(&t);
                self.vpn_task(SId::VpnAdd, &tr!("Adding…"), move || crate::vpn::nm_import(&file).map(|_| ()));
            }
            _ => {}
        }
    }

    /// "This PC": whether PS5 and PS4 games run here, the specs, and what holds it back.
    fn this_pc_row(&self) -> SettingData {
        use crate::pc::Verdict;
        use crate::platform::Platform;
        let mut r = row(4, &tr!("This PC"));
        let Some(specs) = self.pc.as_ref().map(|pc| pc.specs()).filter(|_| !self.pc_checking) else {
            (r.value, r.hint) = (tr!("Checking…").into(), tr!("Reads the processor, memory and graphics card").into());
            return r;
        };
        r.hint = specs.summary().into();
        let (ps5, ps4) = (self.pc_verdict(Platform::Ps5).cloned(), self.pc_verdict(Platform::Ps4).cloned());
        let (Some(ps5), Some(ps4)) = (ps5, ps4) else {
            (r.value, r.hint_kind) = (tr!("Check again").into(), 2);
            r.hint = tr!("{} · the graphics check didn't finish", r.hint).into();
            return r;
        };
        let lower = |v: &Verdict| v.headline().to_lowercase();
        r.value = if ps5 == ps4 {
            tr!("PS5 and PS4 games {}", lower(&ps5)).replace("games won't start", "games won't start here")
        } else {
            tr!("PS5: {} · PS4: {}", lower(&ps5), lower(&ps4))
        }
        .into();
        r.value_kind = if ps5 == Verdict::Good && ps4 == Verdict::Good { 1 } else if matches!((&ps5, &ps4), (Verdict::No(_), _) | (_, Verdict::No(_))) { 2 } else { 0 };
        // The first thing holding it back, in words.
        if let Some((v, p)) = [(&ps5, Platform::Ps5), (&ps4, Platform::Ps4)].into_iter().find(|(v, _)| matches!(v, Verdict::No(_)))
            .or_else(|| [(&ps5, Platform::Ps5), (&ps4, Platform::Ps4)].into_iter().find(|(v, _)| matches!(v, Verdict::Slow(_))))
        {
            r.hint = format!("{} · {}", crate::pc_ui::explain(v, p), r.hint).into();
        }
        r
    }

    /// Add a VPN from a provider's file: the desktop's file dialog, or typing the path.
    fn vpn_add(&mut self, i: usize) {
        if !self.vpn_busy.is_empty() {
            return;
        }
        if !crate::vpn::has_file_dialog() {
            // Typed on Add a VPN from a file (pressed on Your VPN: there, just below).
            let Some(i) = self.settings_ids.iter().position(|s| *s == SId::VpnAdd) else { return };
            self.focus_setting_row(i);
            self.edit_index = i as i32;
            let ui = self.ui();
            ui.set_edit_text("~/Downloads/".into());
            ui.set_edit_index(i as i32);
            return;
        }
        // Said on the row pressed: Add a VPN, or Your VPN when there's none yet.
        let id = self.settings_ids.get(i).copied().unwrap_or(SId::VpnAdd);
        self.vpn_task(id, &tr!("Choose the file…"), || match crate::vpn::choose_file() {
            Ok(Some(file)) => crate::vpn::nm_import(&file).map(|_| ()),
            Ok(None) => Ok(()),
            Err(()) => Err(tr!("There's no file dialog on this PC")),
        });
    }

    /// Run a NetworkManager step off the UI thread. Done, the VPN rows show the new state; an
    /// error shows on row `id`.
    fn vpn_task(&mut self, id: SId, busy: &str, job: impl FnOnce() -> Result<(), String> + Send + 'static) {
        self.vpn_busy = busy.into();
        self.refresh_settings();
        std::thread::spawn(move || {
            let result = job();
            let _ = slint::invoke_from_event_loop(move || {
                with_app(move |app| {
                    app.vpn_busy.clear();
                    if let Err(error) = result {
                        app.note_setting(id, &tr!("Didn't change: {error}", error), 2);
                        audio::play(Sound::Error);
                    }
                    if app.overlay == Overlay::Settings {
                        app.refresh_settings();
                    }
                })
            });
        });
    }

    /// Switch to another display. The UI scale is fixed per window at startup, so the
    /// launcher restarts itself on the new display (about a second).
    pub fn move_to_monitor(&mut self, name: &str) {
        let label = if name.is_empty() { tr!("the primary display") } else { name.to_string() };
        if !self.live.is_empty() {
            self.note_setting(SId::Display, &tr!("Moves to {label} after your game: the launcher can't restart while one is running", label), 0);
            return;
        }
        self.note_setting(SId::Display, &tr!("Moving to {label}: the launcher restarts there", label), 0);
        slint::Timer::single_shot(std::time::Duration::from_millis(600), || with_app(|app| app.app_restart()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_follows_its_row() {
        let rows = [(SId::Header, "SYSTEM"), (SId::Internet, "Internet"), (SId::BtPair, "Pair"),
            (SId::BtDevice(0), "DualSense"), (SId::BtDevice(1), "Xbox")];
        // The Wi-Fi list closed above it: still Pair, not whatever took its place.
        assert_eq!(same_row((SId::BtPair, "Pair"), &rows), Some(2));
        // A device that moved in the list is followed by its name.
        assert_eq!(same_row((SId::BtDevice(0), "Xbox"), &rows), Some(4));
        // A network row that's gone has no place.
        assert_eq!(same_row((SId::WifiNet(3), "Office"), &rows), None);
        // The language changed: a row's name is new, its id still finds it.
        assert_eq!(same_row((SId::Internet, "Internet (old name)"), &rows), Some(1));
    }

    #[test]
    fn progress_shows_from_the_version() {
        assert_eq!(from_version("Downloading SwarmPlay 1.30.0 · 45%", Some("1.30.0")), "1.30.0 · 45%");
        assert_eq!(from_version("Baixando o SwarmPlay 1.30.0 · 45%", Some("1.30.0")), "1.30.0 · 45%");
        assert_eq!(from_version("Installing SwarmPlay…", Some("1.30.0")), "Installing SwarmPlay…");
        assert_eq!(from_version("Checking", None), "Checking");
    }

    #[test]
    fn every_section_has_a_title() {
        for key in ["SYSTEM", "GAMES", "STORAGE", "PLAYING", "SOUND", "NOTIFICATIONS", "DOWNLOADS", "UPDATES", "THEME", "LAUNCHER", "ADVANCED", "ABOUT"] {
            let t = section_title(key);
            assert!(!t.is_empty());
            // In English the title is the key, written as a title.
            assert_eq!(t.to_uppercase(), key);
        }
    }
}

/// Whether a note left on row `noted` (named `noted_label` then) belongs on row `id` (named
/// `label`): the same row, or in a list (a network, a device), the one of that name now.
fn note_row(noted: SId, noted_label: &str, id: SId, label: &str) -> bool {
    match (noted, id) {
        (SId::WifiNet(_), SId::WifiNet(_)) | (SId::BtFound(_), SId::BtFound(_)) | (SId::BtDevice(_), SId::BtDevice(_)) =>
            !noted_label.is_empty() && noted_label == label,
        _ => noted == id,
    }
}

/// A section's title in the left column, from its key (the key stays English: it's what
/// section_info, the icons and deep links match on).
fn section_title(key: &str) -> String {
    match key {
        "SYSTEM" => tr!("System"),
        "GAMES" => tr!("Games"),
        "STORAGE" => tr!("Storage"),
        "PLAYING" => tr!("Playing"),
        "SOUND" => tr!("Sound"),
        "NOTIFICATIONS" => tr!("Notifications"),
        "DOWNLOADS" => tr!("Downloads"),
        "UPDATES" => tr!("Updates"),
        "THEME" => tr!("Theme"),
        "LAUNCHER" => tr!("Launcher"),
        "ADVANCED" => tr!("Advanced"),
        "ABOUT" => tr!("About"),
        other => {
            let mut chars = other.chars();
            chars.next().into_iter().chain(chars.flat_map(|c| c.to_lowercase())).collect()
        }
    }
}

/// Do not disturb is on now.
fn cfg_dnd(app: &App) -> bool {
    app.cfg.lock().unwrap().notify_dnd_until > crate::util::now_secs()
}

impl App {
    /// Settings → Notifications.
    fn notify_rows(&self, cfg: &crate::config::Config) -> Vec<(SId, SettingData)> {
        use crate::toast::{CATS, DND};
        let mut rows = Vec::new();
        let mut r = row(2, &tr!("Pop-up notifications"));
        r.on = cfg.notify_popups;
        r.hint = tr!("Small cards at the top right · everything is kept in the list (Control Center)").into();
        rows.push((SId::NotifPopups, r));
        let mut r = row(3, &tr!("Show them for"));
        r.value = crate::toast::duration_label(cfg.notify_secs).into();
        rows.push((SId::NotifSecs, r));
        let mut r = row(2, &tr!("Sound"));
        r.on = cfg.notify_sound;
        rows.push((SId::NotifSound, r));
        let mut r = row(3, &tr!("During games"));
        r.value = crate::toast::during_games_label(&cfg.notify_games).into();
        r.hint = tr!("Important ones: failures, a game ready to play, a low battery").into();
        rows.push((SId::NotifGames, r));
        let mut r = row(2, &tr!("When SwarmPlay is in the background"));
        r.on = cfg.notify_desktop;
        r.hint = tr!("As your desktop's notifications, with Open to go there").into();
        rows.push((SId::NotifDesktop, r));
        let mut r = row(3, &tr!("Do not disturb"));
        let now = crate::util::now_secs();
        r.value = if cfg.notify_dnd_until <= now { crate::toast::dnd_label(DND[0]).into() }
            else if cfg.notify_dnd_until >= f64::MAX / 2.0 { crate::toast::dnd_label(DND[3]).into() }
            else { tr!("Until {}", crate::util::fmt_clock_at(cfg.notify_dnd_until)).into() };
        r.hint = tr!("Nothing pops up but the important ones; the list keeps everything").into();
        rows.push((SId::NotifDnd, r));
        for (i, (_, name)) in CATS.iter().enumerate() {
            let mut r = row(2, &crate::toast::cat_label(name));
            r.on = !cfg.notify_off.iter().any(|o| o == name);
            rows.push((SId::NotifCat(i as u8), r));
        }
        rows
    }

    /// Left/right (or a press) on a Notifications row. False: not one.
    fn notify_change(&mut self, id: SId, dir: i32) -> bool {
        use crate::toast::{CATS, DND, DURATIONS, DURING_GAMES};
        let on = dir > 0;
        let step = |list: &[&str], cur: &str| -> String {
            let p = list.iter().position(|x| *x == cur).unwrap_or(0) as i32;
            list[(p + dir).rem_euclid(list.len() as i32) as usize].to_string()
        };
        match id {
            SId::NotifPopups => self.save_cfg(|c| c.notify_popups = on),
            SId::NotifSound => self.save_cfg(|c| c.notify_sound = on),
            SId::NotifDesktop => self.save_cfg(|c| c.notify_desktop = on),
            SId::NotifSecs => self.save_cfg(|c| {
                let p = DURATIONS.iter().position(|d| *d == c.notify_secs).unwrap_or(1) as i32;
                c.notify_secs = DURATIONS[(p + dir).rem_euclid(DURATIONS.len() as i32) as usize];
            }),
            SId::NotifGames => { let cur = self.cfg.lock().unwrap().notify_games.clone(); let next = step(&DURING_GAMES, &cur); self.save_cfg(|c| c.notify_games = next); }
            SId::NotifDnd => {
                let now = crate::util::now_secs();
                let cur = { let u = self.cfg.lock().unwrap().notify_dnd_until; if u <= now { 0 } else if u >= f64::MAX / 2.0 { 3 } else if u - now <= 3600.0 { 1 } else { 2 } };
                let next = (cur + dir).rem_euclid(DND.len() as i32);
                let until = match next {
                    0 => 0.0,
                    1 => now + 3600.0,
                    // Until tomorrow, 8:00 local.
                    2 => { let tm = crate::util::local_time(); let since_midnight = (tm.tm_hour * 3600 + tm.tm_min * 60 + tm.tm_sec) as f64; now - since_midnight + 86400.0 + 8.0 * 3600.0 }
                    _ => f64::MAX,
                };
                self.save_cfg(|c| c.notify_dnd_until = until);
            }
            SId::NotifCat(i) => {
                let name = CATS[i as usize].1.to_string();
                self.save_cfg(|c| { c.notify_off.retain(|o| *o != name); if !on { c.notify_off.push(name); } });
            }
            _ => return false,
        }
        true
    }
}

