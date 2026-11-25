//! The performance overlay: the frame rate, and as much more as asked, drawn over a game by
//! MangoHud (the overlay most Linux gaming systems ship; Bazzite and SteamOS have it). SwarmPlay
//! loads it into every game it starts, pointed at a config file of its own; MangoHud watches that
//! file and reloads it when it changes, so the in-game panel (PS → FPS) or Settings → Playing
//! switches levels while the game runs. Off is MangoHud's `no_display`: loaded, drawing nothing,
//! so it can come back without restarting the game.
//!
//! Vulkan games (DXVK and VKD3D under Proton/Wine, shadPS4, KytyPS5) take it through its Vulkan
//! layer (`MANGOHUD=1`); native PC games also start through the `mangohud` wrapper, for OpenGL ones.

use crate::util::config_dir;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The levels, as saved in the config: nothing, the frame rate alone, a line of the main numbers,
/// everything.
pub const LEVELS: [&str; 4] = ["Off", "Simple", "Detailed", "Full"];

/// A level (LEVELS' value), as shown.
pub fn label(key: &str) -> String {
    match key {
        "Simple" => tr::tr!("Simple"),
        "Detailed" => tr::tr!("Detailed"),
        "Full" => tr::tr!("Full"),
        _ => tr::tr!("Off"),
    }
}

/// A level's place in LEVELS (an unknown one is Off).
pub fn index(key: &str) -> usize {
    LEVELS.iter().position(|l| *l == key).unwrap_or(0)
}

/// The level `dir` steps from `key`, going round.
pub fn step(key: &str, dir: i32) -> &'static str {
    LEVELS[(index(key) as i32 + dir).rem_euclid(LEVELS.len() as i32) as usize]
}

/// MangoHud is installed: its Vulkan layer is where the loader looks for implicit layers.
pub fn available() -> bool {
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| crate::util::expand_home("~/.local/share"));
    let dirs = std::env::var("XDG_DATA_DIRS").ok().filter(|d| !d.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    std::iter::once(data).chain(dirs.split(':').map(PathBuf::from)).chain([PathBuf::from("/etc")])
        .any(|d| has_layer(&d.join("vulkan/implicit_layer.d")))
}

fn has_layer(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|rd| rd.flatten().any(|e| {
        let name = e.file_name().to_string_lossy().to_ascii_lowercase();
        name.starts_with("mangohud") && name.ends_with(".json")
    }))
}

/// SwarmPlay's MangoHud config (the user's own ~/.config/MangoHud stays theirs, for other programs).
pub fn config_path() -> PathBuf {
    config_dir().join("mangohud.conf")
}

/// What MangoHud draws at a level. MangoHud shows the frame rate, its graph and the CPU and GPU
/// load on its own, so a level turns off what it doesn't want as well as on what it does.
pub fn config_text(level: &str) -> String {
    let body: &[&str] = match level {
        "Simple" => &["fps_only", "fps", "cpu_stats=0", "gpu_stats=0", "frametime=0", "frame_timing=0"],
        "Detailed" => &["horizontal", "hud_compact", "table_columns=20", "fps", "frametime", "frame_timing=1",
            "cpu_stats", "cpu_temp", "gpu_stats", "gpu_temp", "ram", "vram"],
        "Full" => &["gpu_name", "gpu_stats", "gpu_temp", "gpu_core_clock", "gpu_mem_clock", "gpu_power", "gpu_fan", "vram",
            "cpu_stats", "cpu_temp", "cpu_power", "cpu_mhz", "core_load", "core_bars", "ram", "swap", "procmem",
            "fps", "fps_metrics=avg,0.01,0.001", "frametime", "frame_timing=1", "throttling_status",
            "resolution", "present_mode", "refresh_rate", "vulkan_driver", "engine_version", "wine", "gamemode", "time"],
        _ => &["no_display"],
    };
    // In the corner notifications don't use, in the panel's colours, numbers easy to read.
    let look = ["legacy_layout=0", "position=top-left", "offset_x=16", "offset_y=16", "round_corners=10",
        "background_color=0f1424", "background_alpha=0.6", "font_size=22", "text_outline"];
    let mut text = String::from("# Written by SwarmPlay (Settings → Playing → Performance overlay): changes here are replaced.\n");
    for line in look.iter().chain(body) {
        text.push_str(line);
        text.push('\n');
    }
    text
}

/// The config file, at a level. Rewritten in place, not replaced: MangoHud watches the file itself
/// (inotify), and a write is what it reloads on.
pub fn write(level: &str) {
    let path = config_path();
    let text = config_text(level);
    if std::fs::read_to_string(&path).is_ok_and(|t| t == text) { return; }
    let _ = std::fs::create_dir_all(config_dir());
    match std::fs::write(&path, text) {
        Ok(()) => crate::log!("performance overlay: {level}"),
        Err(e) => crate::log!("couldn't write {}: {e}", path.display()),
    }
}

/// A game's command with MangoHud in it, at the chosen level (when it's installed).
pub fn prepare(command: &mut Command, level: &str) {
    if !available() { return; }
    write(level);
    command.env("MANGOHUD", "1").env("MANGOHUD_CONFIGFILE", config_path())
        // Settings in the environment would win over the file, and the panel couldn't change them.
        .env_remove("MANGOHUD_CONFIG");
}

/// A native program, started through the `mangohud` wrapper when there is one: Vulkan games take the
/// layer from `MANGOHUD=1` alone, OpenGL ones need the wrapper's library.
pub fn program(target: &Path) -> Command {
    match available().then(|| crate::pcgame::which("mangohud")).flatten() {
        Some(wrapper) => {
            let mut c = Command::new(wrapper);
            c.arg(target);
            c
        }
        None => Command::new(target),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_go_round_both_ways() {
        assert_eq!(step("Off", 1), "Simple");
        assert_eq!(step("Full", 1), "Off");
        assert_eq!(step("Off", -1), "Full");
        assert_eq!(step("Something old", 1), "Simple", "an unknown level counts as Off");
    }

    #[test]
    fn each_level_says_what_it_shows() {
        let lines = |l: &str| config_text(l).lines().map(String::from).collect::<Vec<_>>();
        assert!(lines("Off").contains(&"no_display".into()));
        assert!(lines("Simple").contains(&"fps_only".into()) && !lines("Simple").contains(&"no_display".into()));
        assert!(lines("Detailed").contains(&"horizontal".into()) && lines("Detailed").contains(&"gpu_temp".into()));
        assert!(lines("Full").contains(&"fps_metrics=avg,0.01,0.001".into()) && !lines("Full").contains(&"horizontal".into()));
        for l in LEVELS { assert!(config_text(l).lines().all(|l| l.starts_with('#') || !l.contains(' ')), "one option per line: {l}"); }
    }

    #[test]
    fn every_level_has_a_label() {
        for l in LEVELS { assert!(!label(l).is_empty()); }
        assert_eq!(index("Full"), 3);
    }
}
