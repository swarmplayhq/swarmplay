//! Repacks waiting to be installed: a folder with an Inno Setup installer and the `.bin` archives it
//! unpacks (FitGirl's, DODI's and the like), as downloaded. Home shows each one as a console shows
//! a disc put in; Install copies it like one: the files are checked against the repack's own MD5
//! list, then its installer runs silently under Proton/Wine, in the prefix the game will play in,
//! and the game joins the Library.
//!
//! What it takes to run these installers unattended on Wine:
//! - `/VERYSILENT /SUPPRESSMSGBOXES /NOCANCEL /NORESTART /SP- /DIR=…`: no wizard; every component the
//!   download holds is installed (FitGirl's "selective" languages are the ones downloaded).
//! - The unpacker starts helpers (`rzw`, `rz`, `mpz`…) by name alone. Windows finds them in the
//!   installer's temporary folder (the current one); Wine doesn't look there, the unpacking fails
//!   and the repack reports "archive data corrupted". They're copied into the prefix's `C:\windows`
//!   as soon as they're unpacked, with the libraries they load beside them (`mpz` needs
//!   `MpzSlimmer.dll` for a repack's MP3s), and taken out again after.
//! - It opens its maker's site at the end: Wine's browser is switched off for it.
//! - It reports success even when unpacking failed (it deletes what it wrote): the result is checked
//!   by its size, and progress is how far its processes have read into the `.bin` files.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tr::tr;

/// A repack folder, ready to install.
#[derive(Clone, Debug, PartialEq)]
pub struct Repack {
    pub dir: PathBuf,
    pub setup: PathBuf,
    /// The game's name, from the folder's ("God of War [FitGirl Repack]" → "God of War").
    pub name: String,
    /// Who made it, when the folder says ("FitGirl", "DODI"), else "".
    pub maker: String,
    /// The archives, in name order, with their sizes.
    pub bins: Vec<(PathBuf, u64)>,
    /// The repack's MD5 list, when it has one.
    pub md5: Option<PathBuf>,
    /// Languages of FitGirl's "selective" files downloaded ("brazilian", "english"…).
    pub languages: Vec<String>,
}

impl Repack {
    /// The archives' size: what's read while installing.
    pub fn size(&self) -> u64 {
        self.bins.iter().map(|(_, s)| s).sum()
    }

    /// Brazilian Portuguese voices or text are in it.
    pub fn brazilian(&self) -> bool {
        self.languages.iter().any(|l| l == "brazilian")
    }
}

/// The folder's repack, if it is one: a `setup.exe` made with Inno Setup and `.bin` files beside it.
pub fn find(dir: &Path) -> Option<Repack> {
    let mut setup = None;
    let mut bins = Vec::new();
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() { continue; }
        if name == "setup.exe" {
            setup = Some(entry.path());
        } else if name.ends_with(".bin") && meta.len() > 0 {
            bins.push((entry.path(), meta.len()));
        }
    }
    let setup = setup?;
    // Archives worth installing (not a stray 1 KB file), and an installer that is Inno Setup.
    if bins.iter().map(|(_, s)| s).sum::<u64>() < 16 << 20 || !is_inno(&setup) {
        return None;
    }
    bins.sort();
    let folder = dir.file_name()?.to_string_lossy().into_owned();
    let (name, maker) = repack_name(&folder);
    let languages = bins.iter().filter_map(|(p, _)| {
        let stem = p.file_stem()?.to_string_lossy().to_ascii_lowercase();
        stem.strip_prefix("fg-selective-").map(str::to_string)
    }).collect();
    Some(Repack { dir: dir.to_path_buf(), setup, name, maker, bins, md5: md5_file(dir), languages })
}

/// Repacks in these folders and the folders inside them (a drive's top, the downloads folder).
pub fn scan(roots: &[PathBuf]) -> Vec<Repack> {
    let mut found: Vec<Repack> = Vec::new();
    let mut look = |dir: &Path| {
        if found.iter().any(|r| r.dir == dir) { return; }
        if let Some(r) = find(dir) { found.push(r); }
    };
    for root in roots {
        look(root);
        let Ok(rd) = std::fs::read_dir(root) else { continue };
        let mut children: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        children.sort();
        for c in children.iter().filter(|c| !c.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))) {
            look(c);
        }
    }
    found.sort_by_key(|r| r.name.to_lowercase());
    found
}

/// Inno Setup keeps its setup data after this mark (in the first tens of MB of the program).
fn is_inno(setup: &Path) -> bool {
    const MARK: &[u8] = b"Inno Setup Setup Data (";
    let Ok(file) = std::fs::File::open(setup) else { return false };
    let mut bytes = Vec::new();
    if file.take(64 << 20).read_to_end(&mut bytes).is_err() { return false; }
    bytes.windows(MARK.len()).any(|w| w == MARK)
}

/// "God of War [FitGirl Repack]" → ("God of War", "FitGirl"); "Elden.Ring-DODI" → ("Elden Ring", "DODI").
pub fn repack_name(folder: &str) -> (String, String) {
    let lower = folder.to_ascii_lowercase();
    let maker = if lower.contains("fitgirl") { "FitGirl" } else if lower.contains("dodi") { "DODI" }
        else if lower.contains("elamigos") { "ElAmigos" } else if lower.contains("kaos") { "KaOs" } else { "" };
    // Dots and underscores for spaces ("Elden.Ring-DODI"), the maker's tag dropped.
    let spaced = if folder.contains(' ') { folder.to_string() } else { folder.replace(['.', '_'], " ") };
    let tagless = regex::Regex::new(r"(?i)[\s\-–]*\(?\[?\s*(fitgirl|dodi|elamigos|kaos)(\s+repacks?)?\s*\]?\)?").unwrap()
        .replace_all(&spaced, "").into_owned();
    let tagless = regex::Regex::new(r"(?i)[\s\-]*\brepack\b").unwrap().replace_all(&tagless, "").into_owned();
    let name = crate::pcgame::clean_name(tagless.trim());
    let name = if name.trim().is_empty() { folder.to_string() } else { name.trim().to_string() };
    (name, maker.to_string())
}

/// The repack's MD5 list: an `MD5` folder's `.md5` file, or one beside the archives.
fn md5_file(dir: &Path) -> Option<PathBuf> {
    let in_dir = |d: &Path| std::fs::read_dir(d).ok()?.flatten().map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("md5")));
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path())
        .find(|p| p.is_dir() && p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("md5")))
        .and_then(|d| in_dir(&d)).or_else(|| in_dir(dir))
}

/// The list's lines ("d2a7…5 *..\fg-01.bin"), for the files that are there: a selective file left
/// out of the download isn't missing.
pub fn md5_entries(list: &Path, dir: &Path) -> Vec<(PathBuf, String)> {
    let Ok(text) = std::fs::read(list) else { return Vec::new() };
    String::from_utf8_lossy(&text).lines().filter_map(|line| {
        let line = line.trim();
        if line.starts_with(';') || line.is_empty() { return None; }
        let (hash, name) = line.split_once(char::is_whitespace)?;
        let name = name.trim().trim_start_matches('*').replace('\\', "/");
        let file = name.rsplit('/').next()?;
        (hash.len() == 32 && hash.bytes().all(|b| b.is_ascii_hexdigit()) && !file.is_empty())
            .then(|| (dir.join(file), hash.to_ascii_lowercase()))
    }).filter(|(p, _)| p.is_file()).collect()
}

/// Check the archives against the repack's MD5 list. `progress(done, total)` in bytes. A damaged
/// file is named: getting it again from the torrent mends it.
pub fn verify(r: &Repack, progress: &dyn Fn(u64, u64), cancel: &AtomicBool) -> Result<(), String> {
    let Some(list) = &r.md5 else { return Ok(()) };
    let entries = md5_entries(list, &r.dir);
    let total: u64 = entries.iter().filter_map(|(p, _)| p.metadata().ok()).map(|m| m.len()).sum();
    let mut done = 0u64;
    for (path, want) in entries {
        let mut file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
        let mut md5 = Md5::new();
        let mut buf = vec![0u8; 4 << 20];
        loop {
            if cancel.load(Ordering::Relaxed) { return Err(tr!("Cancelled")); }
            let n = file.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 { break; }
            md5.update(&buf[..n]);
            done += n as u64;
            progress(done, total);
        }
        if md5.hex() != want {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            return Err(tr!("{} is damaged: check the download again in your torrent app", name));
        }
    }
    Ok(())
}

/// Install the repack into `dest`, with `prefix` as its Wine prefix (the one the game plays in).
/// `progress(done, total)`: how far the installer has read into the archives. Returns once the
/// game is in `dest`, with its manifest written.
pub fn install(r: &Repack, dest: &Path, prefix: &Path, topic: i64, progress: &dyn Fn(u64, u64), cancel: &AtomicBool) -> Result<(), String> {
    let (runner, label) = crate::pcgame::windows_runner()
        .ok_or_else(|| tr!("Install umu-launcher (Proton) or Wine to install Windows games"))?;
    std::fs::create_dir_all(prefix).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dest.parent().unwrap_or(dest)).map_err(|e| e.to_string())?;
    let log = crate::util::cache_dir().join("logs");
    let _ = std::fs::create_dir_all(&log);
    let log = log.join(format!("repack-{}.log", crate::pcgame::folder_name(&r.name)));
    let windows = |p: &Path| format!("Z:{}", p.display());
    let mut command = Command::new(&runner);
    command.arg(&r.setup)
        .args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NOCANCEL", "/NORESTART", "/SP-"])
        .arg(format!("/DIR={}", windows(dest))).arg(format!("/LOG={}", windows(&log)))
        .current_dir(&r.dir).env("WINEPREFIX", prefix)
        // Its maker's site, opened at the end: not from here.
        .env("WINEDLLOVERRIDES", "winebrowser.exe=d").env("WINEDEBUG", "-all")
        .env_remove("MANGOHUD").env_remove("MANGOHUD_CONFIGFILE")
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .process_group(0);
    if label == "Proton" {
        command.env("GAMEID", "umu-default").env("PROTONPATH", "GE-Proton");
    }
    use std::os::unix::process::CommandExt;
    let mut child = command.spawn().map_err(|e| tr!("Couldn't start the installer: {e}", e))?;
    crate::log!("repack: installing {} into {}", r.name, dest.display());
    let total = r.size();
    let mut read: HashMap<PathBuf, u64> = HashMap::new();
    let mut copied: Vec<PathBuf> = Vec::new();
    let mut last_move = Instant::now();
    let mut last_done = 0u64;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? { break status; }
        if cancel.load(Ordering::Relaxed) {
            stop(&runner, label, prefix, &mut child);
            cleanup(&copied);
            return Err(tr!("Cancelled"));
        }
        copied.extend(place_helpers(prefix));
        for (bin, pos) in reading(prefix, r) {
            let seen = read.entry(bin).or_default();
            *seen = (*seen).max(pos);
        }
        let done = read.values().sum::<u64>().min(total);
        if done != last_done { last_done = done; last_move = Instant::now(); }
        progress(done, total);
        // Reading stopped partway for minutes: the unpacker failed and its message box waits.
        if done > 0 && done < total * 97 / 100 && last_move.elapsed() > Duration::from_secs(240) {
            stop(&runner, label, prefix, &mut child);
            cleanup(&copied);
            return Err(tr!("The repack's installer stopped while unpacking (see {})", log.display()));
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    cleanup(&copied);
    crate::log!("repack: installer for {} exited ({status})", r.name);
    // It says it succeeded either way: what's there tells. (Unpacked games are bigger than their
    // compressed archives; a failed unpack leaves a few MB, or nothing.)
    let size = dir_size(dest);
    if size < total / 2 {
        return Err(tr!("The repack couldn't be unpacked (see {})", log.display()));
    }
    write_manifest(r, dest, topic)
}

/// The game's manifest, so the Library lists it and Play starts the right program.
fn write_manifest(r: &Repack, dest: &Path, topic: i64) -> Result<(), String> {
    let mut files = Vec::new();
    collect_programs(dest, "", 0, &mut files);
    let (launch, kind, alternatives) = crate::pcgame::choose(&files, &r.name)
        .ok_or_else(|| tr!("Installed, but no program to start was found in {}", dest.display()))?;
    let m = crate::pcgame::Manifest { version: 1, name: r.name.clone(), topic, launch, kind, alternatives };
    let json = serde_json::to_vec_pretty(&m).map_err(|e| e.to_string())?;
    std::fs::write(dest.join(crate::pcgame::MANIFEST), json).map_err(|e| e.to_string())
}

fn collect_programs(dir: &Path, prefix: &str, depth: usize, out: &mut Vec<(String, crate::pcgame::Probe)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() && depth < 4 {
            collect_programs(&entry.path(), &rel, depth + 1, out);
        } else if kind.is_file() && name.to_ascii_lowercase().ends_with(".exe") {
            out.push((rel, crate::pcgame::probe_file(&entry.path())));
        }
    }
}

/// The unpacker's helpers and their libraries, copied where Wine looks for programs started by
/// name (see the module's notes); nothing already there is replaced. Returns what it put there, to
/// take out after.
fn place_helpers(prefix: &Path) -> Vec<PathBuf> {
    let temp = prefix.join("drive_c/users/steamuser/AppData/Local/Temp");
    let temp = if temp.is_dir() { temp } else { prefix.join(format!("drive_c/users/{}/AppData/Local/Temp", whoami())) };
    let windows = prefix.join("drive_c/windows");
    let mut placed = Vec::new();
    let Ok(rd) = std::fs::read_dir(&temp) else { return placed };
    for dir in rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
        // The installer's own folder: it holds the unpacker.
        if !dir.join("unarc.dll").is_file() && !dir.join("ISDone.dll").is_file() { continue; }
        let Ok(files) = std::fs::read_dir(&dir) else { continue };
        for f in files.flatten().map(|e| e.path()) {
            let Some(name) = f.file_name() else { continue };
            let lower = name.to_string_lossy().to_ascii_lowercase();
            if !lower.ends_with(".exe") && !lower.ends_with(".dll") { continue; }
            let to = windows.join(name);
            if !to.exists() && std::fs::copy(&f, &to).is_ok() {
                placed.push(to);
            }
        }
    }
    placed
}

fn whoami() -> String {
    std::env::var("USER").unwrap_or_else(|_| "steamuser".into())
}

fn cleanup(copied: &[PathBuf]) {
    for f in copied { let _ = std::fs::remove_file(f); }
}

/// How far the installer's processes are into each archive: the read position of every open
/// `.bin` of this repack, in processes running in this prefix.
fn reading(prefix: &Path, r: &Repack) -> Vec<(PathBuf, u64)> {
    let want = format!("WINEPREFIX={}", prefix.display());
    let mut out = Vec::new();
    let Ok(procs) = std::fs::read_dir("/proc") else { return out };
    for p in procs.flatten() {
        let pid = p.file_name();
        if !pid.to_string_lossy().bytes().all(|b| b.is_ascii_digit()) { continue; }
        let base = p.path();
        let Ok(env) = std::fs::read(base.join("environ")) else { continue };
        if !env.split(|b| *b == 0).any(|v| v == want.as_bytes()) { continue; }
        let Ok(fds) = std::fs::read_dir(base.join("fd")) else { continue };
        for fd in fds.flatten() {
            let Ok(target) = std::fs::read_link(fd.path()) else { continue };
            if !r.bins.iter().any(|(b, _)| *b == target) { continue; }
            let pos = std::fs::read_to_string(base.join("fdinfo").join(fd.file_name())).ok()
                .and_then(|t| t.lines().find_map(|l| l.strip_prefix("pos:").map(|v| v.trim().parse::<u64>().ok())).flatten());
            if let Some(pos) = pos { out.push((target, pos)); }
        }
    }
    out
}

/// End the installer and everything it started in its prefix.
fn stop(runner: &Path, label: &str, prefix: &Path, child: &mut std::process::Child) {
    unsafe { libc::kill(-(child.id() as i32), libc::SIGTERM); }
    let mut kill = Command::new(runner);
    kill.arg("wineserver").arg("-k").env("WINEPREFIX", prefix)
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    if label == "Proton" { kill.env("GAMEID", "umu-default").env("PROTONPATH", "GE-Proton"); }
    let _ = kill.status();
    let _ = child.kill();
    let _ = child.wait();
}

fn dir_size(dir: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    rd.flatten().map(|e| match e.file_type() {
        Ok(t) if t.is_dir() => dir_size(&e.path()),
        Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
        _ => 0,
    }).sum()
}

// ------------------------------------------------------------------ MD5 (RFC 1321)

struct Md5 { state: [u32; 4], buf: Vec<u8>, len: u64 }

impl Md5 {
    fn new() -> Self { Md5 { state: [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476], buf: Vec::with_capacity(64), len: 0 } }

    fn update(&mut self, mut data: &[u8]) {
        self.len += data.len() as u64;
        if !self.buf.is_empty() {
            let take = (64 - self.buf.len()).min(data.len());
            self.buf.extend_from_slice(&data[..take]);
            data = &data[take..];
            if self.buf.len() == 64 { let b = std::mem::take(&mut self.buf); self.block(&b); }
        }
        let mut chunks = data.chunks_exact(64);
        for c in &mut chunks { self.block(c); }
        self.buf.extend_from_slice(chunks.remainder());
    }

    fn hex(mut self) -> String {
        let bits = self.len.wrapping_mul(8);
        let mut tail = std::mem::take(&mut self.buf);
        tail.push(0x80);
        while tail.len() % 64 != 56 { tail.push(0); }
        tail.extend_from_slice(&bits.to_le_bytes());
        for c in tail.chunks(64) { self.block(c); }
        self.state.iter().flat_map(|w| w.to_le_bytes()).map(|b| format!("{b:02x}")).collect()
    }

    fn block(&mut self, c: &[u8]) {
        const S: [u32; 64] = [7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20,
            4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21];
        let k = |i: usize| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32;
        let m: Vec<u32> = c.chunks(4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect();
        let [mut a, mut b, mut cc, mut d] = self.state;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & cc) | (!b & d), i),
                1 => ((d & b) | (!d & cc), (5 * i + 1) % 16),
                2 => (b ^ cc ^ d, (3 * i + 5) % 16),
                _ => (cc ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k(i)).wrapping_add(m[g]);
            a = d; d = cc; cc = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        for (s, v) in self.state.iter_mut().zip([a, b, cc, d]) { *s = s.wrapping_add(v); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md5(data: &[u8]) -> String { let mut m = Md5::new(); m.update(data); m.hex() }

    #[test]
    fn md5_matches_the_standard() {
        assert_eq!(md5(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        let long: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        let mut split = Md5::new();
        split.update(&long[..37]); split.update(&long[37..500]); split.update(&long[500..]);
        assert_eq!(split.hex(), md5(&long), "fed in pieces, the same");
        assert_eq!(md5(b"The quick brown fox jumps over the lazy dog"), "9e107d9d372bb6826bd81d3542a419d6");
    }

    #[test]
    fn names_come_from_the_folder() {
        assert_eq!(repack_name("God of War [FitGirl Repack]"), ("God of War".into(), "FitGirl".into()));
        assert_eq!(repack_name("Elden.Ring-DODI").0, "Elden Ring");
        assert_eq!(repack_name("Hades [DODI Repack]"), ("Hades".into(), "DODI".into()));
        assert_eq!(repack_name("Some Game").1, "");
    }

    #[test]
    fn a_folder_with_an_inno_installer_and_archives_is_a_repack() {
        let dir = std::env::temp_dir().join(format!("swarmplay-repack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let r = dir.join("Celeste [FitGirl Repack]");
        std::fs::create_dir_all(r.join("MD5")).unwrap();
        let mut setup = vec![0u8; 4096];
        setup.extend_from_slice(b"Inno Setup Setup Data (5.5.0) (u)");
        std::fs::write(r.join("setup.exe"), &setup).unwrap();
        std::fs::write(r.join("fg-01.bin"), vec![1u8; 17 << 20]).unwrap();
        std::fs::write(r.join("fg-selective-brazilian.bin"), b"pt").unwrap();
        std::fs::write(r.join("MD5/fitgirl-bins.md5"), format!("; comment\r\n{} *..\\fg-selective-brazilian.bin\r\n0123456789abcdef0123456789abcdef *..\\fg-selective-english.bin\r\n", md5(b"pt"))).unwrap();
        std::fs::create_dir_all(dir.join("Not a repack")).unwrap();
        std::fs::write(dir.join("Not a repack/setup.exe"), b"MZ plain").unwrap();
        let found = scan(&[dir.clone()]);
        assert_eq!(found.len(), 1, "{found:?}");
        let rp = &found[0];
        assert_eq!(rp.name, "Celeste");
        assert!(rp.brazilian() && rp.md5.is_some());
        let entries = md5_entries(rp.md5.as_ref().unwrap(), &rp.dir);
        assert_eq!(entries.len(), 1, "a selective file not downloaded isn't missing");
        assert!(verify(rp, &|_, _| {}, &AtomicBool::new(false)).is_ok());
        std::fs::write(r.join("fg-selective-brazilian.bin"), b"PT").unwrap();
        assert!(verify(rp, &|_, _| {}, &AtomicBool::new(false)).unwrap_err().contains("fg-selective-brazilian.bin"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
