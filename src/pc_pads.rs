//! Controllers in native Linux PC games. Feral's ports read a Sony controller themselves
//! (`EnableLibScePad` in their preferences: PlayStation buttons), while a repack's Steam emulator
//! (Goldberg's `steam_settings/controller`) hands the same controller to the game again, as an
//! Xbox one: the game sees two controllers and its button prompts flip between them. Before such
//! a game starts, one of the two is turned off (Settings → Playing → Controller in PC games).

use std::path::{Path, PathBuf};

/// The choices, as saved: follow the connected controller, or always one kind.
pub const KINDS: [&str; 3] = ["Automatic", "PlayStation", "Xbox"];
/// Goldberg's controller folder while it's turned off (renamed back for Xbox).
const OFF: &str = "controller.swarmplay-off";

/// A choice (KINDS' value), as shown.
pub fn label(key: &str) -> String {
    match key {
        "PlayStation" => "PlayStation".into(),
        "Xbox" => "Xbox".into(),
        _ => tr::tr!("Automatic"),
    }
}

/// Set a native game up for the chosen controller kind. Only Feral's ports: they're the ones with
/// both paths; any other game keeps its emulator's controller (it may be its only one).
pub fn prepare(game_dir: &Path, kind: &str) {
    let Some(prefs) = feral_preferences(game_dir) else { return };
    let playstation = match kind {
        "PlayStation" => true,
        "Xbox" => false,
        _ => match crate::gamepad::sony_first() { Some(sony) => sony, None => return },
    };
    if let Ok(text) = std::fs::read_to_string(&prefs) {
        if let Some(new) = with_sce_pad(&text, playstation) {
            match crate::util::atomic_write(&prefs, new.as_bytes()) {
                Ok(()) => crate::log!("controller: {} buttons in {}", if playstation { "PlayStation" } else { "Xbox" }, prefs.display()),
                Err(e) => crate::log!("couldn't set the controller in {}: {e}", prefs.display()),
            }
        }
    }
    for settings in steam_settings_in(game_dir) {
        let (from, to) = if playstation { ("controller", OFF) } else { (OFF, "controller") };
        let (from, to) = (settings.join(from), settings.join(to));
        // Real folders only, and never over one that's there already.
        let is_dir = |p: &Path| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir());
        if is_dir(&from) && std::fs::symlink_metadata(&to).is_err() {
            match std::fs::rename(&from, &to) {
                Ok(()) => crate::log!("controller: Steam emulator's controller {} in {}", if playstation { "off" } else { "on" }, settings.display()),
                Err(e) => crate::log!("couldn't rename {}: {e}", from.display()),
            }
        }
    }
}

/// A Feral port's preferences file, from its `config/game-settings.sh` (the game's own launcher
/// reads `$HOME/.local/share/<FERAL_PREFERENCES_DIR>/preferences`). None: not a Feral port.
fn feral_preferences(game_dir: &Path) -> Option<PathBuf> {
    let settings = find(game_dir, 3, &|d| d.join("config/game-settings.sh").is_file())?.join("config/game-settings.sh");
    let text = std::fs::read_to_string(settings).ok()?;
    let value = |key: &str| text.lines().find_map(|l| {
        let v = l.trim().strip_prefix(key)?.strip_prefix('=')?;
        Some(v.split('#').next()?.trim().trim_matches('"').to_string())
    }).filter(|v| !v.is_empty());
    let dir = value("FERAL_PREFERENCES_DIR").or_else(|| value("FERAL_GAME_NAME_FULL").map(|n| format!("feral-interactive/{n}")))?;
    // The game's files are a download: a plain folder name under ~/.local/share, nothing else.
    let rel = Path::new(&dir);
    if rel.is_absolute() || rel.components().any(|c| !matches!(c, std::path::Component::Normal(_))) { return None; }
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    Some(home.join(".local/share").join(rel).join("preferences"))
}

/// `text` (Feral preferences) with its own Sony controller support on or off; None: no change
/// (already so, or a game without the setting).
fn with_sce_pad(text: &str, on: bool) -> Option<String> {
    const KEY: &str = "<value name=\"EnableLibScePad\" type=\"integer\">";
    let start = text.find(KEY)? + KEY.len();
    let end = start + text[start..].find("</value>")?;
    let want = if on { "1" } else { "0" };
    if text[start..end].trim() == want { return None; }
    Some(format!("{}{want}{}", &text[..start], &text[end..]))
}

/// Goldberg's `steam_settings` folders in a game (beside its steamclient), a few levels down.
fn steam_settings_in(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            if !e.file_type().is_ok_and(|t| t.is_dir()) { continue; }
            let p = e.path();
            if e.file_name() == "steam_settings" { out.push(p); } else if depth > 0 { walk(&p, depth - 1, out); }
        }
    }
    let mut out = Vec::new();
    walk(dir, 3, &mut out);
    out
}

/// The first folder at most `depth` levels under `dir` (or `dir`) that `hit` accepts; symlinks
/// aren't followed.
fn find(dir: &Path, depth: u32, hit: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    if hit(dir) { return Some(dir.to_path_buf()); }
    if depth == 0 { return None; }
    let mut subs: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()).collect();
    subs.sort();
    subs.iter().find_map(|d| find(d, depth - 1, hit))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PREFS: &str = "<registry>\n <key name=\"Input\">\n  <value name=\"Combat_Aim_1\" type=\"integer\">301</value>\n  <value name=\"EnableLibScePad\" type=\"integer\">1</value>\n </key>\n</registry>\n";

    #[test]
    fn feral_sony_support_turns_off_for_xbox_and_back() {
        let off = with_sce_pad(PREFS, false).unwrap();
        assert!(off.contains("\"EnableLibScePad\" type=\"integer\">0</value>"));
        assert_eq!(off.len(), PREFS.len());
        assert_eq!(with_sce_pad(&off, true).unwrap(), PREFS);
        assert_eq!(with_sce_pad(PREFS, true), None, "already on: the file isn't rewritten");
        assert_eq!(with_sce_pad("<registry/>", false), None, "a game without the setting is left alone");
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("azrak-pc-pads-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_feral_port_is_found_by_its_settings_and_its_preferences_stay_under_home() {
        let d = tmp("feral");
        std::fs::create_dir_all(d.join("game/config")).unwrap();
        std::fs::write(d.join("game/config/game-settings.sh"),
            "FERAL_GAME_NAME_FULL=\"Some Game\" # Long form name\nFERAL_PREFERENCES_DIR=\"feral-interactive/Some Game\" # Preferences\n").unwrap();
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
        assert_eq!(feral_preferences(&d), Some(home.join(".local/share/feral-interactive/Some Game/preferences")));
        std::fs::write(d.join("game/config/game-settings.sh"), "FERAL_PREFERENCES_DIR=\"../../.config/x\"\n").unwrap();
        assert_eq!(feral_preferences(&d), None, "a path out of ~/.local/share is refused");
        assert_eq!(feral_preferences(&tmp("other")), None, "not a Feral port");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn steam_settings_folders_are_found_a_few_levels_down() {
        let d = tmp("goldberg");
        std::fs::create_dir_all(d.join("game/steam_settings/controller")).unwrap();
        assert_eq!(steam_settings_in(&d), vec![d.join("game/steam_settings")]);
        let _ = std::fs::remove_dir_all(&d);
    }
}
