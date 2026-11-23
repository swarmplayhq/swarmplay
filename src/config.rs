//! User settings, stored in ~/.config/azrak/config.json (same format as v1).

use crate::util::{atomic_write, config_dir, expand_home};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    pub emulator: String,
    pub game_dirs: Vec<String>,
    pub download_dir: String,
    pub install_dir: String,
    /// Continue sharing completed downloads until stopped or the launcher exits.
    pub seed_after_download: bool,
    /// Torrent upload limit in MB/s (src/downloads.rs `UPLOAD_LIMITS`); 0: unlimited.
    pub upload_limit_mb: u32,
    pub library_compact: bool,
    pub fullscreen: bool,
    pub width: u32,
    pub height: u32,
    pub present_mode: String,
    pub amd_cpu: bool,
    pub extra_args: String,
    pub sounds: bool,
    /// Interface sounds' volume: 1 Low, 2 Medium, 3 High (audio::LEVELS).
    pub sounds_level: u8,
    pub return_on_exit: bool,
    /// Output name (e.g. "DP-2"); empty = primary display.
    pub monitor: String,
    /// Keep the launcher-managed KytyPS5 on the latest official build.
    pub kyty_auto_update: bool,
    /// Install shadPS4 in the background and keep it on the latest official release.
    pub shad_auto_update: bool,
    /// Install new Azrak Launcher releases automatically (applied on the next start).
    pub app_auto_update: bool,
    /// Download (and seed) only through a VPN; stop when it isn't connected.
    pub vpn_only: bool,
    /// The VPN's network interface to use ("" = any connected VPN).
    pub vpn_interface: String,
    /// Library searches that found something, newest first (shown before typing on the
    /// on-screen keyboard).
    pub recent_searches: Vec<String>,
    /// The Library's sort (an index into app::SORTS) and status filter ("All", "Installed",
    /// "Playable"), as last chosen.
    pub library_sort: usize,
    pub library_status: String,
    /// Settings → Theme (src/theme.rs): the wallpaper, the picture for "Your image" and the
    /// accent colour, by name.
    pub theme_wallpaper: String,
    pub theme_image: String,
    pub theme_accent: String,
    /// Settings → Storage's other places (folders on other drives); see src/storage.rs.
    pub storage: Vec<String>,
    /// The drives those folders (and the game folders) are on, by their filesystem's UUID and
    /// where each was mounted: one mounted somewhere else since takes its folders along
    /// (storage::follow_drives).
    pub drives: Vec<KnownDrive>,
    /// The in-game overlay (src/overlay_ui.rs): notifications over games, and pausing the game
    /// while the panel is up.
    pub overlay_toasts: bool,
    pub overlay_pause: bool,
    /// Windows games get controllers through SDL, as XInput ones (src/wine_pads.rs).
    pub wine_sdl_pads: bool,
    /// The controller native PC games see when they could see it twice (src/pc_pads.rs):
    /// "Automatic", "PlayStation" or "Xbox".
    pub pc_pad: String,
    /// Settings → Notifications (src/toast.rs): pop-ups at all, how long they stay, their sound,
    /// during games ("All", "Important only", "None"), desktop notifications in the background,
    /// Do not disturb until (Unix seconds), kinds turned off (by name), the catalog digest's day.
    pub notify_popups: bool,
    pub notify_secs: u32,
    pub notify_sound: bool,
    pub notify_games: String,
    pub notify_desktop: bool,
    pub notify_dnd_until: f64,
    pub notify_off: Vec<String>,
    pub last_digest_day: i64,
    /// The interface's language (src/i18n.rs): "en", "pt_BR", or empty for the system's.
    pub language: String,
    /// The first-run setup (src/onboard.rs) was gone through. A settings file from before it
    /// existed counts as set up: someone who already uses the launcher never sees it by surprise.
    pub setup_done: bool,
    /// The name on the avatar and the start screen (empty: the system's, see app::user_name).
    pub user_name: String,
    /// The avatar's colour, one of theme::ACCENTS' names (empty: the accent colour).
    pub avatar_color: String,
    /// The avatar's picture (a square PNG made by src/avatar.rs; empty: the initial on its colour).
    pub avatar_image: String,
    /// Who uses this launcher (the start screen asks who's playing). The fields above that a
    /// profile has (name, avatar, theme, language) are the one playing now's; `profiles` keeps
    /// everyone's, the one playing's brought up to date on every save.
    pub profiles: Vec<Profile>,
    /// Which of `profiles` is playing.
    pub profile: usize,
}

/// Someone who plays here: their name, avatar, theme and language. Games, downloads and the rest
/// of the settings are everyone's.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Profile {
    pub user_name: String,
    pub avatar_color: String,
    pub avatar_image: String,
    pub theme_wallpaper: String,
    pub theme_image: String,
    pub theme_accent: String,
    pub language: String,
}

/// A drive SwarmPlay keeps folders on: its filesystem's UUID, and where it was last mounted.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct KnownDrive {
    pub uuid: String,
    pub mount: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            emulator: String::new(),
            game_dirs: vec!["~/Games/Azrak".into()],
            download_dir: "~/Downloads/Azrak".into(),
            install_dir: "~/Games/Azrak".into(),
            seed_after_download: true,
            upload_limit_mb: crate::downloads::UPLOAD_DEFAULT_MB,
            library_compact: false,
            fullscreen: true,
            width: 1920,
            height: 1080,
            present_mode: "Mailbox".into(),
            amd_cpu: false,
            extra_args: String::new(),
            sounds: true,
            sounds_level: 2,
            return_on_exit: true,
            monitor: String::new(),
            kyty_auto_update: true,
            shad_auto_update: true,
            app_auto_update: true,
            vpn_only: false,
            vpn_interface: String::new(),
            recent_searches: Vec::new(),
            library_sort: 0,
            library_status: "All".into(),
            theme_wallpaper: "Game art".into(),
            theme_image: String::new(),
            theme_accent: "Blue".into(),
            storage: Vec::new(),
            drives: Vec::new(),
            overlay_toasts: true,
            overlay_pause: false,
            wine_sdl_pads: true,
            pc_pad: "Automatic".into(),
            notify_popups: true,
            notify_secs: 6,
            notify_sound: true,
            notify_games: "Important only".into(),
            notify_desktop: true,
            notify_dnd_until: 0.0,
            notify_off: Vec::new(),
            last_digest_day: 0,
            language: String::new(),
            setup_done: false,
            user_name: String::new(),
            avatar_color: String::new(),
            avatar_image: String::new(),
            profiles: Vec::new(),
            profile: 0,
        }
    }
}

pub const RESOLUTIONS: [(u32, u32); 5] = [(1280, 720), (1600, 900), (1920, 1080), (2560, 1440), (3840, 2160)];
pub const PRESENT_MODES: [(&str, &str); 3] =
    [("Mailbox", "Mailbox (low latency)"), ("Fifo", "Fifo (V-Sync)"), ("Immediate", "Immediate (uncapped)")];

/// A present mode (the config's key, PRESENT_MODES' first column) as Settings shows it; an
/// unknown one, as it is.
pub fn present_label(key: &str) -> String {
    present_text(key).unwrap_or_else(|| key.to_string())
}

fn present_text(key: &str) -> Option<String> {
    Some(match key {
        "Mailbox" => tr::tr!("Mailbox (low latency)"),
        "Fifo" => tr::tr!("Fifo (V-Sync)"),
        "Immediate" => tr::tr!("Immediate (uncapped)"),
        _ => return None,
    })
}

/// A settings file written before the first-run setup existed (it has no `setup_done`).
fn from_before_setup(json: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(json).is_ok_and(|v| v.get("setup_done").is_none())
}

impl Config {
    pub fn path() -> PathBuf {
        config_dir().join("config.json")
    }

    pub fn load() -> Config {
        let bytes = std::fs::read(Self::path()).ok();
        let mut cfg: Config = bytes.as_deref().and_then(|b| serde_json::from_slice(b).ok()).unwrap_or_default();
        if bytes.as_deref().is_some_and(from_before_setup) {
            cfg.setup_done = true;
        }
        // Every settings file has its first profile (made from the fields when it's older).
        cfg.keep_profile();
        cfg.one_update_switch();
        cfg.dedup_game_dirs();
        if cfg.emulator.is_empty() || !cfg.emulator_path().is_file() {
            let managed = crate::kyty::managed_emulator();
            if managed.is_file() {
                cfg.emulator = managed.to_string_lossy().into_owned();
            } else if let Some(found) = detect_emulator() {
                cfg.emulator = found.to_string_lossy().into_owned();
            }
        }
        cfg
    }

    /// The same game folder written two ways (~/Games and /home/me/Games, or with a trailing
    /// /) is one: scanned once and shown once.
    pub(crate) fn dedup_game_dirs(&mut self) {
        let mut seen = std::collections::HashSet::new();
        self.game_dirs.retain(|d| !d.trim().is_empty() && seen.insert(expand_home(d.trim().trim_end_matches('/'))));
    }

    /// Settings has one "Update automatically" switch for the launcher, KytyPS5 and shadPS4.
    /// Older settings files set them separately (and have no shadPS4 entry, which defaults to
    /// on): if any was turned off, all are off, so the switch shows what actually happens.
    fn one_update_switch(&mut self) {
        let all = self.app_auto_update && self.kyty_auto_update && self.shad_auto_update;
        (self.app_auto_update, self.kyty_auto_update, self.shad_auto_update) = (all, all, all);
    }

    pub fn save(&self) {
        let mut this = self.clone();
        this.keep_profile();
        if let Ok(json) = serde_json::to_vec_pretty(&this) {
            if let Err(e) = atomic_write(&Self::path(), &json) {
                crate::log!("could not save config: {e}");
            }
        }
    }

    /// The one playing, as the fields hold them now.
    pub fn profile_now(&self) -> Profile {
        Profile {
            user_name: self.user_name.clone(),
            avatar_color: self.avatar_color.clone(),
            avatar_image: self.avatar_image.clone(),
            theme_wallpaper: self.theme_wallpaper.clone(),
            theme_image: self.theme_image.clone(),
            theme_accent: self.theme_accent.clone(),
            language: self.language.clone(),
        }
    }

    /// The one playing's entry in `profiles` as the fields hold it (the first profile is made here
    /// from a settings file older than profiles).
    pub fn keep_profile(&mut self) {
        if self.profiles.is_empty() {
            self.profile = 0;
            self.profiles.push(Profile::default());
        }
        self.profile = self.profile.min(self.profiles.len() - 1);
        let now = self.profile_now();
        self.profiles[self.profile] = now;
    }

    /// Someone else plays now: their name, avatar, theme and language in the fields.
    pub fn use_profile(&mut self, i: usize) {
        self.keep_profile();
        let Some(p) = self.profiles.get(i).cloned() else { return };
        self.profile = i;
        self.user_name = p.user_name;
        self.avatar_color = p.avatar_color;
        self.avatar_image = p.avatar_image;
        self.theme_wallpaper = p.theme_wallpaper;
        self.theme_image = p.theme_image;
        self.theme_accent = p.theme_accent;
        self.language = p.language;
    }

    /// A new profile, with the one playing's theme and language to start from; it plays now.
    pub fn add_profile(&mut self, name: &str) -> usize {
        self.keep_profile();
        let p = Profile { user_name: name.to_string(), avatar_color: String::new(), avatar_image: String::new(), ..self.profile_now() };
        self.profiles.push(p);
        let i = self.profiles.len() - 1;
        self.use_profile(i);
        i
    }

    /// Someone leaves (never the last one); the first profile plays then.
    pub fn remove_profile(&mut self, i: usize) {
        self.keep_profile();
        if self.profiles.len() < 2 || i >= self.profiles.len() {
            return;
        }
        self.profiles.remove(i);
        self.profile = usize::MAX;
        self.use_first();
    }

    fn use_first(&mut self) {
        let p = self.profiles[0].clone();
        self.profile = 0;
        self.user_name = p.user_name;
        self.avatar_color = p.avatar_color;
        self.avatar_image = p.avatar_image;
        self.theme_wallpaper = p.theme_wallpaper;
        self.theme_image = p.theme_image;
        self.theme_accent = p.theme_accent;
        self.language = p.language;
    }

    /// "Download only through a VPN": `Some(preferred interface)` ("" = any connected VPN).
    pub fn download_vpn(&self) -> Option<String> {
        self.vpn_only.then(|| self.vpn_interface.clone())
    }

    pub fn emulator_path(&self) -> PathBuf {
        expand_home(self.emulator.trim())
    }

    pub fn emulator_ok(&self) -> bool {
        let p = self.emulator_path();
        std::fs::metadata(&p).map(|m| m.is_file() && is_executable(&m)).unwrap_or(false)
    }

    pub fn game_dir_paths(&self) -> Vec<PathBuf> {
        self.game_dirs.iter().filter(|d| !d.trim().is_empty()).map(|d| expand_home(d.trim())).collect()
    }
}

fn is_executable(m: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    m.permissions().mode() & 0o111 != 0
}

#[cfg(test)]
mod tests {

    #[test]
    fn profiles_keep_their_own_name_theme_and_language() {
        let mut c = Config { user_name: "Ana".into(), theme_accent: "Teal".into(), language: "pt_BR".into(), ..Default::default() };
        c.keep_profile();
        assert_eq!(c.profiles.len(), 1);
        let bia = c.add_profile("Bia");
        assert_eq!((bia, c.user_name.as_str(), c.theme_accent.as_str()), (1, "Bia", "Teal"), "a new one starts from the theme in use");
        c.theme_accent = "Pink".into();
        c.language = "en".into();
        c.use_profile(0);
        assert_eq!((c.user_name.as_str(), c.theme_accent.as_str(), c.language.as_str()), ("Ana", "Teal", "pt_BR"));
        c.use_profile(1);
        assert_eq!((c.user_name.as_str(), c.theme_accent.as_str(), c.language.as_str()), ("Bia", "Pink", "en"));
        c.remove_profile(1);
        assert_eq!((c.profiles.len(), c.profile, c.user_name.as_str()), (1, 0, "Ana"));
        c.remove_profile(0);
        assert_eq!(c.profiles.len(), 1, "never the last one");
    }

    #[test]
    fn a_settings_file_from_before_the_setup_counts_as_set_up() {
        assert!(from_before_setup(br#"{"fullscreen": true}"#));
        assert!(!from_before_setup(br#"{"setup_done": false}"#));
        assert!(!from_before_setup(b"not json"));
        let old: Config = serde_json::from_slice(br#"{"fullscreen": true}"#).unwrap();
        assert!(old.user_name.is_empty() && old.avatar_color.is_empty());
    }

    use super::*;

    #[test]
    fn every_present_mode_has_a_label() {
        for (key, label) in PRESENT_MODES { assert_eq!(present_text(key).as_deref(), Some(label)); }
        assert_eq!(present_label("Other"), "Other");
    }

    #[test]
    fn an_older_config_reads_with_the_system_language() {
        let old: Config = serde_json::from_str(r#"{"fullscreen": false}"#).unwrap();
        assert_eq!(old.language, "", "no language saved: the system's");
    }

    #[test]
    fn a_game_folder_written_two_ways_is_kept_once() {
        let home = expand_home("~").to_string_lossy().into_owned();
        let mut cfg = Config { game_dirs: vec!["~/Games/Azrak".into(), format!("{home}/Games/Azrak/"), "".into(), "/mnt/ssd/Games".into()], ..Default::default() };
        cfg.dedup_game_dirs();
        assert_eq!(cfg.game_dirs, vec!["~/Games/Azrak".to_string(), "/mnt/ssd/Games".into()]);
    }

    #[test]
    fn an_older_update_opt_out_turns_every_automatic_update_off() {
        let mut old: Config = serde_json::from_str(r#"{"kyty_auto_update": false, "app_auto_update": false}"#).unwrap();
        assert!(old.shad_auto_update, "a missing shadPS4 entry defaults to on");
        old.one_update_switch();
        assert!(!old.app_auto_update && !old.kyty_auto_update && !old.shad_auto_update);
        let mut partial: Config = serde_json::from_str(r#"{"app_auto_update": false}"#).unwrap();
        partial.one_update_switch();
        assert!(!partial.kyty_auto_update && !partial.shad_auto_update);
        let mut fresh = Config::default();
        fresh.one_update_switch();
        assert!(fresh.app_auto_update && fresh.kyty_auto_update && fresh.shad_auto_update);
    }

    #[test]
    fn seeding_defaults_on_for_old_configs_and_preserves_explicit_opt_out() {
        let old: Config = serde_json::from_str(r#"{"sounds":false}"#).unwrap();
        assert!(old.seed_after_download);
        let opted_out: Config = serde_json::from_str(r#"{"seed_after_download":false}"#).unwrap();
        assert!(!opted_out.seed_after_download);
        let restored: Config = serde_json::from_slice(&serde_json::to_vec(&opted_out).unwrap()).unwrap();
        assert!(!restored.seed_after_download);
    }

    #[test]
    fn the_library_view_starts_as_before_and_is_kept_once_chosen() {
        let old: Config = serde_json::from_str(r#"{"sounds":false}"#).unwrap();
        assert_eq!((old.library_sort, old.library_status.as_str()), (0, "All"));
        let mut chosen = old;
        chosen.library_sort = 7;
        chosen.library_status = "Playable".into();
        let restored: Config = serde_json::from_slice(&serde_json::to_vec(&chosen).unwrap()).unwrap();
        assert_eq!((restored.library_sort, restored.library_status.as_str()), (7, "Playable"));
    }
}

/// Look for kyty_emulator in PATH and a few common build locations.
pub fn detect_emulator() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|d| d.join("kyty_emulator")));
    }
    let home = expand_home("~");
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf()));
    let mut roots = vec![home.clone(), home.join("projects"), home.join("Games"), home.join("Applications"), home.join("src")];
    if let Some(d) = &exe_dir {
        roots.extend(d.ancestors().take(4).map(|p| p.to_path_buf()));
    }
    for root in roots {
        for sub in ["KytyPS5", "kyty", "Kyty"] {
            for build in ["_Build/release-linux", "_Build/linux/install", "_Build/linux", "build", ""] {
                candidates.push(root.join(sub).join(build).join("kyty_emulator"));
            }
        }
        // One level deeper, e.g. ~/projects/PS5/KytyPS5
        if let Ok(rd) = std::fs::read_dir(&root) {
            for e in rd.flatten().take(200) {
                let p = e.path();
                if p.is_dir() {
                    for build in ["KytyPS5/_Build/release-linux", "KytyPS5/_Build/linux/install"] {
                        candidates.push(p.join(build).join("kyty_emulator"));
                    }
                }
            }
        }
    }
    candidates.into_iter().find(|p| std::fs::metadata(p).map(|m| m.is_file() && is_executable(&m)).unwrap_or(false))
}
