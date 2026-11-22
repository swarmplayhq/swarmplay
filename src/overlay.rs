//! The in-game overlay's link to the desktop: which desktop this is, and how a window gets above
//! a running game, gets focus, and gives it back. A normal Wayland program can do none of that,
//! so each desktop has its own way:
//!
//! - GNOME: the `swarmplay@m1theusr` Shell extension (extras/gnome-shell), over D-Bus. It puts the
//!   overlay on the game's own screen and above the game — not full screen: Mutter (43 and later)
//!   paints an opaque black backdrop behind a full-screen Wayland window, which would hide the
//!   game, while a window the size of the screen, raised and given the focus, shows it live behind
//!   the panel (src/overlay_ui.rs sizes it).
//! - KDE Plasma: a small KWin script, loaded through KWin's D-Bus for each step.
//! - AzrakOS (its own Gamescope): Gamescope's overlay window property.
//! - Steam's Game Mode: Steam owns the PS button and has its own overlay; ours stays off.
//! - X11 sessions: the window manager honours "keep above" and window activation.
//!
//! Every call runs a short command (gdbus, xdotool, xprop); callers keep them off the UI thread.

use std::path::Path;
use tr::tr;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// The overlay window's title: how the helpers find it.
pub const OVERLAY_TITLE: &str = "SwarmPlay Overlay";
/// The launcher's own window title.
pub const MAIN_TITLE: &str = "SwarmPlay";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Desktop {
    Gnome,
    Kde,
    /// AzrakOS: Gamescope without Steam.
    Gamescope,
    /// Steam's Game Mode (Bazzite-Deck, SteamOS): Steam runs the overlay.
    SteamGameMode,
    X11,
    Other,
}

impl Desktop {
    pub fn label(self) -> String {
        match self {
            Self::Gnome => "GNOME".into(),
            Self::Kde => "KDE Plasma".into(),
            Self::Gamescope => "Gamescope".into(),
            Self::SteamGameMode => tr!("Steam Game Mode"),
            Self::X11 => "X11".into(),
            Self::Other => tr!("this desktop"),
        }
    }
}

fn detect(var: impl Fn(&str) -> Option<String>) -> Desktop {
    let gamescope = var("GAMESCOPE_WAYLAND_DISPLAY").is_some();
    let steam = var("SteamGamepadUI").is_some() || var("SteamDeck").is_some_and(|v| v == "1") && gamescope
        || var("SteamAppId").is_some() && gamescope;
    if gamescope && steam { return Desktop::SteamGameMode; }
    if gamescope { return Desktop::Gamescope; }
    let current = var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_ascii_uppercase();
    let wayland = var("WAYLAND_DISPLAY").is_some();
    if current.contains("GNOME") { return Desktop::Gnome; }
    if current.contains("KDE") { return Desktop::Kde; }
    if !wayland && var("DISPLAY").is_some() { return Desktop::X11; }
    Desktop::Other
}

pub fn desktop() -> Desktop {
    static D: OnceLock<Desktop> = OnceLock::new();
    *D.get_or_init(|| detect(|k| std::env::var(k).ok().filter(|v| !v.is_empty())))
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

// ------------------------------------------------------------------ GNOME

const GNOME_UUID: &str = "swarmplay@m1theusr";
const GNOME_EXTENSION_JS: &str = include_str!("../extras/gnome-shell/swarmplay@m1theusr/extension.js");
const GNOME_METADATA: &str = include_str!("../extras/gnome-shell/swarmplay@m1theusr/metadata.json");

fn gnome(method: &str, args: &[&str]) -> Option<String> {
    let mut a = vec!["call", "--session", "--timeout", "3", "--dest", "org.swarmplay.Shell", "--object-path", "/org/swarmplay/Shell",
        "--method"];
    let m = format!("org.swarmplay.Shell.{method}");
    a.push(&m);
    a.extend_from_slice(args);
    run("gdbus", &a)
}

/// What the overlay window is: the panel (the screen's size, over the game) or a notification's
/// card (in the top right corner, the game's input going on).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Screen,
    Corner,
}

impl Shape {
    /// What the GNOME helper calls it.
    fn name(self) -> &'static str {
        match self {
            Self::Screen => "screen",
            Self::Corner => "corner",
        }
    }
}

/// The helper's version this binary needs (extras/gnome-shell/swarmplay@m1theusr/metadata.json).
/// 4: the panel is raised, not made full screen (the game shows through it), on the game's screen.
const GNOME_HELPER_VERSION: u32 = 4;

/// Where the helper stands on this desktop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Helper {
    /// Ready: the overlay goes above games.
    Ready,
    /// GNOME: the extension isn't installed (or enabled) yet.
    Missing,
    /// GNOME: installed, waiting for the next login.
    NeedsLogin,
    /// Steam's Game Mode: Steam's overlay is used.
    Steam,
    /// No way to stay above a game here (the launcher still comes to the front).
    Unsupported,
}

/// Ask once (a D-Bus round trip at most): off the UI thread.
pub fn helper() -> Helper {
    match desktop() {
        Desktop::Gnome => {
            // An older helper is replaced with the binary's copy; the session keeps the code it
            // loaded (GNOME has no way to reload an extension on Wayland), so the next login loads
            // it. Switching it off and on again only makes it run the same old code.
            let version = gnome("Version", &[]).map_or(0, |v| extension_version(&v));
            if version >= GNOME_HELPER_VERSION { return Helper::Ready; }
            if version > 0 {
                let _ = install_gnome_helper();
                return Helper::NeedsLogin;
            }
            let installed = dirs_home().map(|h| h.join(".local/share/gnome-shell/extensions").join(GNOME_UUID).join("extension.js").is_file()).unwrap_or(false);
            if installed { Helper::NeedsLogin } else { Helper::Missing }
        }
        Desktop::Kde => if run("gdbus", &["introspect", "--session", "--dest", "org.kde.KWin", "--object-path", "/Scripting"]).is_some() { Helper::Ready } else { Helper::Unsupported },
        Desktop::Gamescope | Desktop::X11 => Helper::Ready,
        Desktop::SteamGameMode => Helper::Steam,
        Desktop::Other => Helper::Unsupported,
    }
}

/// `gdbus` prints the answer as `('2',)`: its digits are the helper's version.
fn extension_version(answer: &str) -> u32 {
    answer.chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0)
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(std::path::PathBuf::from)
}

/// Put the GNOME extension in place and turn it on (GNOME loads it at the next login on Wayland).
pub fn install_gnome_helper() -> Result<(), String> {
    let dir = dirs_home().ok_or_else(|| tr!("no home folder"))?.join(".local/share/gnome-shell/extensions").join(GNOME_UUID);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("extension.js"), GNOME_EXTENSION_JS).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("metadata.json"), GNOME_METADATA).map_err(|e| e.to_string())?;
    // Enabling may fail until GNOME has seen it (a login): the setting still lists it then.
    if run("gnome-extensions", &["enable", GNOME_UUID]).is_none() {
        let list = run("gsettings", &["get", "org.gnome.shell", "enabled-extensions"]).unwrap_or_default();
        if !list.contains(GNOME_UUID) {
            let inner = list.trim().trim_start_matches("@as").trim().trim_start_matches('[').trim_end_matches(']').trim().to_string();
            let new = if inner.is_empty() { format!("['{GNOME_UUID}']") } else { format!("[{inner}, '{GNOME_UUID}']") };
            run("gsettings", &["set", "org.gnome.shell", "enabled-extensions", &new]).ok_or_else(|| tr!("couldn't turn it on"))?;
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ KDE (KWin scripts)

/// Run a few lines of KWin script once: load, run, unload.
fn kwin(body: &str) -> bool {
    let Some(cache) = dirs_home().map(|h| h.join(".cache/azrak")) else { return false };
    let _ = std::fs::create_dir_all(&cache);
    let path = cache.join("swarmplay-kwin.js");
    if std::fs::write(&path, body).is_err() { return false; }
    let name = "swarmplay";
    let script = |method: &str, args: &[&str]| {
        let mut a = vec!["call", "--session", "--dest", "org.kde.KWin", "--object-path", "/Scripting", "--method"];
        let m = format!("org.kde.kwin.Scripting.{method}");
        a.push(&m);
        a.extend_from_slice(args);
        run("gdbus", &a)
    };
    let _ = script("unloadScript", &[name]);
    let Some(id) = script("loadScript", &[&path.to_string_lossy(), name]) else { return false };
    let id: String = id.chars().filter(|c| c.is_ascii_digit()).collect();
    let ok = run("gdbus", &["call", "--session", "--dest", "org.kde.KWin", "--object-path", &format!("/Scripting/Script{id}"), "--method", "org.kde.kwin.Script.run"]).is_some();
    let _ = script("unloadScript", &[name]);
    ok
}

fn js_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

// ------------------------------------------------------------------ X11 / Gamescope

fn x_windows_named(title: &str) -> Vec<String> {
    run("xdotool", &["search", "--name", &format!("^{title}$")]).map(|s| s.split_whitespace().map(String::from).collect()).unwrap_or_default()
}

// ------------------------------------------------------------------ what callers use

/// The overlay window over everything: `focus` also gives it the keyboard and the mouse (the
/// panel); without, it only shows (a notification) and the game keeps them.
pub fn raise_overlay(focus: bool, shape: Shape) -> bool {
    match desktop() {
        Desktop::Gnome => gnome("Raise", &[&format!("'{OVERLAY_TITLE}'"), if focus { "true" } else { "false" }, &format!("'{}'", shape.name())]).is_some(),
        Desktop::Kde => {
            let place = match shape {
                Shape::Screen => "w.fullScreen = true;",
                // A notification's card: the top right corner of the screen it is on.
                Shape::Corner => "const g = w.output.geometry ?? workspace.activeScreen.geometry; \
                    w.frameGeometry = Qt.rect(g.x + g.width - w.frameGeometry.width, g.y, w.frameGeometry.width, w.frameGeometry.height);",
            };
            kwin(&format!(
                "for (const w of workspace.windowList()) {{ if (w.caption === {t}) {{ w.keepAbove = true; w.skipTaskbar = true; {p} {f} }} }}",
                t = js_str(OVERLAY_TITLE), p = place, f = if focus { "workspace.activeWindow = w;" } else { "" }))
        }
        Desktop::Gamescope => {
            // Gamescope draws a window with this property above the game, and gives it input.
            for w in x_windows_named(OVERLAY_TITLE) {
                run("xprop", &["-id", &w, "-f", "STEAM_OVERLAY", "32c", "-set", "STEAM_OVERLAY", "1"]);
            }
            true
        }
        // (The window asked to stay on top itself: src/overlay_ui.rs.)
        Desktop::X11 => {
            let ws = x_windows_named(OVERLAY_TITLE);
            if focus { for w in &ws { run("xdotool", &["windowactivate", w]); } }
            !ws.is_empty()
        }
        Desktop::SteamGameMode | Desktop::Other => false,
    }
}

/// Back to the game (process `pid` or one of its group), the overlay hidden.
pub fn focus_game(pid: u32) -> bool {
    match desktop() {
        Desktop::Gnome => {
            let pids: Vec<String> = std::iter::once(pid).chain(crate::sessions::group_members(pid)).map(|p| p.to_string()).collect();
            gnome("FocusPids", &[&format!("[{}]", pids.join(","))]).is_some()
        }
        Desktop::Kde => {
            let pids: Vec<String> = std::iter::once(pid).chain(crate::sessions::group_members(pid)).map(|p| p.to_string()).collect();
            kwin(&format!("const p = [{}]; for (const w of workspace.windowList()) {{ if (p.includes(w.pid) && w.normalWindow) {{ workspace.activeWindow = w; break; }} }}", pids.join(",")))
        }
        _ => crate::sessions::activate_pid_window(pid),
    }
}

/// The launcher's own window to the front (Home, and what opens from there).
pub fn focus_main() -> bool {
    match desktop() {
        Desktop::Gnome => gnome("Focus", &[&format!("'{MAIN_TITLE}'")]).is_some(),
        Desktop::Kde => kwin(&format!("for (const w of workspace.windowList()) {{ if (w.caption === {t}) {{ workspace.activeWindow = w; }} }}", t = js_str(MAIN_TITLE))),
        _ => {
            crate::sessions::show_launcher();
            true
        }
    }
}

/// A screenshot of what's on screen, saved as `path` (a PNG).
pub fn screenshot(path: &Path) -> Result<(), String> {
    if let Some(dir) = path.parent() { std::fs::create_dir_all(dir).map_err(|e| e.to_string())?; }
    let p = path.to_string_lossy().into_owned();
    let ok = match desktop() {
        Desktop::Gnome => gnome("Screenshot", &[&format!("'{p}'")]).is_some_and(|r| r.contains("true")),
        Desktop::Kde => run("spectacle", &["-b", "-n", "-f", "-o", &p]).is_some(),
        Desktop::X11 | Desktop::Gamescope => run("import", &["-window", "root", &p]).is_some(),
        _ => false,
    };
    if ok && path.is_file() { Ok(()) } else { Err(tr!("screenshots aren't available on {}", desktop().label())) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn tells_the_desktops_apart() {
        assert_eq!(detect(env(&[("XDG_CURRENT_DESKTOP", "GNOME"), ("WAYLAND_DISPLAY", "wayland-0")])), Desktop::Gnome);
        assert_eq!(detect(env(&[("XDG_CURRENT_DESKTOP", "KDE"), ("WAYLAND_DISPLAY", "wayland-0")])), Desktop::Kde);
        assert_eq!(detect(env(&[("GAMESCOPE_WAYLAND_DISPLAY", "gamescope-0")])), Desktop::Gamescope, "AzrakOS");
        assert_eq!(detect(env(&[("GAMESCOPE_WAYLAND_DISPLAY", "gamescope-0"), ("SteamGamepadUI", "1")])), Desktop::SteamGameMode);
        assert_eq!(detect(env(&[("DISPLAY", ":0"), ("XDG_CURRENT_DESKTOP", "XFCE")])), Desktop::X11);
        assert_eq!(detect(env(&[("WAYLAND_DISPLAY", "wayland-1"), ("XDG_CURRENT_DESKTOP", "sway")])), Desktop::Other);
    }

    #[test]
    fn reads_the_gnome_helper_version() {
        assert_eq!(extension_version("('3',)"), 3);
        assert_eq!(extension_version("('10',)"), 10);
        assert_eq!(extension_version(""), 0, "no answer, no version");
    }

    #[test]
    fn needs_the_helper_this_binary_ships() {
        let version = GNOME_METADATA.split("\"version\"").nth(1).and_then(|v| v.split(':').nth(1)).map(|v| extension_version(v)).unwrap_or(0);
        assert_eq!(version, GNOME_HELPER_VERSION, "extras/gnome-shell/swarmplay@m1theusr/metadata.json");
    }

    #[test]
    fn kwin_strings_are_quoted() {
        assert_eq!(js_str(r#"a "b" \c"#), r#""a \"b\" \\c""#);
    }
}
