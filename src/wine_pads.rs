//! Controllers in Windows games. Wine hands Sony controllers (DualShock 4, DualSense) to games
//! raw, through hidraw, as Proton does: games made for them (Sony's ports, most big ones) read
//! them natively, with PlayStation buttons. A game that only knows XInput may then see none;
//! through SDL instead, Wine shows every controller as an XInput one (Settings → Playing →
//! Controller in Windows games, off by default). It isn't the default because SDL's copy keeps
//! the Sony controller's ids but not its reports: a game made for the real one reads it wrong
//! (Ghost of Tsushima: a stick held up). Proton gets the same through PROTON_PREFER_SDL.

use std::path::{Path, PathBuf};

/// The winebus settings in a prefix's system.reg. Newer Wine keeps them in ControlSet001 (and
/// CurrentControlSet links there); prefixes made by older Wine (5.x, in Kron4ek's repacks) keep
/// a real CurrentControlSet, which is the one their Wine reads.
const SECTIONS: [&str; 2] = [r"[System\\ControlSet001\\Services\\winebus]", r"[System\\CurrentControlSet\\Services\\winebus]"];
const VALUES: [(&str, &str); 2] = [("\"DisableHidraw\"", "dword:00000001"), ("\"Enable SDL\"", "dword:00000001")];
/// Wine's own way, in a prefix SwarmPlay set to SDL before: hidraw for Sony controllers again.
const NATIVE: [(&str, &str); 1] = [("\"DisableHidraw\"", "dword:00000000")];
/// Left in a prefix the first time SwarmPlay changes it: the original, and the mark of a prefix
/// it set.
const BACKUP: &str = "system.reg.swarmplay-backup";

/// Wine prefixes in a game's folder (a repack's game/prefix, say): folders with a system.reg
/// and a drive_c, a few levels down at most.
pub fn prefixes_in(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
        if dir.join("system.reg").is_file() && dir.join("drive_c").is_dir() {
            out.push(dir.to_path_buf());
            return;
        }
        if depth == 0 { return; }
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            // Not into the game's own files (drive_c of another prefix, big data folders).
            if p.is_dir() && !e.file_type().is_ok_and(|t| t.is_symlink()) { walk(&p, depth - 1, out); }
        }
    }
    let mut out = Vec::new();
    walk(dir, 3, &mut out);
    out
}

/// `text` (a system.reg) with controllers going through SDL; None when it already does.
fn with_sdl(text: &str) -> Option<String> {
    with_values(text, &VALUES)
}

/// `text` with `values` in its winebus key (added when it has none); None when already so.
fn with_values(text: &str, values: &[(&str, &str)]) -> Option<String> {
    let has = |section: &str| text.lines().any(|l| l.starts_with(section));
    let present: Vec<&str> = SECTIONS.into_iter().filter(|s| has(s)).collect();
    if present.is_empty() {
        // No winebus key yet: add it where this prefix's Wine keeps its services (read at the next start).
        let old = !text.contains(r"[System\\ControlSet001\\") && text.contains(r"[System\\CurrentControlSet\\");
        let mut out = text.trim_end().to_string();
        out.push_str(&format!("\n\n{} 0\n", SECTIONS[old as usize]));
        for (k, v) in values { out.push_str(&format!("{k}={v}\n")); }
        return Some(out);
    }
    let mut out = text.to_string();
    let mut changed = false;
    for section in present {
        if let Some(new) = set_in(&out, section, values) { out = new; changed = true; }
    }
    changed.then_some(out)
}

/// `text` with `values` set in its `section`, which it has; None: already so.
fn set_in(text: &str, section: &str, values: &[(&str, &str)]) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.iter().position(|l| l.starts_with(section))?;
    let end = lines[start + 1..].iter().position(|l| l.trim().is_empty() || l.starts_with('[')).map_or(lines.len(), |n| start + 1 + n);
    let mut body: Vec<String> = lines[start + 1..end].iter().map(|l| l.to_string()).collect();
    let mut changed = false;
    for (k, v) in values {
        let want = format!("{k}={v}");
        match body.iter().position(|l| l.starts_with(&format!("{k}="))) {
            Some(i) if body[i] == want => {}
            Some(i) => { body[i] = want; changed = true; }
            None => { body.push(want); changed = true; }
        }
    }
    if !changed { return None; }
    let mut out: Vec<String> = lines[..=start].iter().map(|l| l.to_string()).collect();
    out.extend(body);
    out.extend(lines[end..].iter().map(|l| l.to_string()));
    Some(out.join("\n") + "\n")
}

/// Set every prefix under `dir` (or `dir` itself) to hand controllers through SDL. Only while
/// the game isn't running (Wine keeps the registry in memory and writes it back on exit).
pub fn prefer_sdl(dir: &Path) {
    for prefix in prefixes_in(dir) {
        let reg = prefix.join("system.reg");
        let Ok(text) = std::fs::read_to_string(&reg) else { continue };
        if let Some(new) = with_sdl(&text) {
            let backup = prefix.join(BACKUP);
            if !backup.exists() { let _ = std::fs::copy(&reg, &backup); }
            match crate::util::atomic_write(&reg, new.as_bytes()) {
                Ok(()) => crate::log!("controllers through SDL in {}", prefix.display()),
                Err(e) => crate::log!("couldn't set controllers through SDL in {}: {e}", prefix.display()),
            }
        }
    }
}

/// Every prefix under `dir` that SwarmPlay set to SDL back to Wine's own way (Sony controllers
/// natively, through hidraw). Prefixes it never changed are left as their game came.
pub fn prefer_native(dir: &Path) {
    for prefix in prefixes_in(dir).into_iter().filter(|p| p.join(BACKUP).is_file()) {
        let reg = prefix.join("system.reg");
        let Ok(text) = std::fs::read_to_string(&reg) else { continue };
        let Some(new) = native(&text) else { continue };
        match crate::util::atomic_write(&reg, new.as_bytes()) {
            Ok(()) => crate::log!("controllers natively again in {}", prefix.display()),
            Err(e) => crate::log!("couldn't set controllers natively in {}: {e}", prefix.display()),
        }
    }
}

/// `text` with hidraw on again, where it was turned off; None when it isn't.
fn native(text: &str) -> Option<String> {
    text.contains("\"DisableHidraw\"=dword:00000001").then(|| with_values(text, &NATIVE)).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns_hidraw_off_and_sdl_on_in_the_winebus_key() {
        let reg = "WINE REGISTRY Version 2\n\n[System\\\\ControlSet001\\\\Services\\\\winebus] 1794966133\n#time=1dd\n\"Description\"=\"Wine HID bus driver\"\n\"Enable SDL\"=dword:00000000\n\n[Software] 1\n\"x\"=\"y\"\n";
        let out = with_sdl(reg).unwrap();
        assert!(out.contains("\"Enable SDL\"=dword:00000001"));
        assert!(out.contains("\"DisableHidraw\"=dword:00000001"));
        assert!(!out.contains("\"Enable SDL\"=dword:00000000"));
        assert!(out.contains("[Software] 1\n\"x\"=\"y\""), "the rest stays as it was");
        assert_eq!(with_sdl(&out), None, "already set: left alone");
    }

    #[test]
    fn an_older_wines_current_control_set_is_the_one_set() {
        // Wine 5.x prefixes: a real CurrentControlSet; a ControlSet001 key there is never read.
        let reg = "WINE REGISTRY Version 2\n\n[System\\\\CurrentControlSet\\\\Services\\\\winebus] 1795442974\n#time=1dd\n\"Start\"=dword:00000003\n\n[System\\\\CurrentControlSet\\\\Services\\\\winehid] 1\n\"x\"=\"y\"\n";
        let out = with_sdl(reg).unwrap();
        assert!(out.contains("[System\\\\CurrentControlSet\\\\Services\\\\winebus] 1795442974\n#time=1dd\n\"Start\"=dword:00000003\n\"DisableHidraw\"=dword:00000001\n\"Enable SDL\"=dword:00000001"));
        assert!(!out.contains("ControlSet001"));
        assert_eq!(with_sdl(&out), None);
        let fresh = with_sdl("WINE REGISTRY Version 2\n\n[System\\\\CurrentControlSet\\\\Control] 1\n").unwrap();
        assert!(fresh.contains("[System\\\\CurrentControlSet\\\\Services\\\\winebus] 0\n"), "no key yet: added where that Wine reads");
    }

    #[test]
    fn going_back_turns_hidraw_on_again_and_keeps_sdl() {
        let set = with_sdl("WINE REGISTRY Version 2\n").unwrap();
        let back = native(&set).unwrap();
        assert!(back.contains("\"DisableHidraw\"=dword:00000000\n\"Enable SDL\"=dword:00000001"));
        assert_eq!(native(&back), None, "already native: left alone");
        assert_eq!(native("WINE REGISTRY Version 2\n"), None, "never set: nothing to undo");
    }

    #[test]
    fn adds_the_key_when_the_prefix_has_none() {
        let out = with_sdl("WINE REGISTRY Version 2\n").unwrap();
        assert!(out.contains("[System\\\\ControlSet001\\\\Services\\\\winebus] 0\n\"DisableHidraw\"=dword:00000001\n\"Enable SDL\"=dword:00000001"));
    }
}
