//! The Downloads page: every download from looking it up to Play, newest first, over the selected
//! one's key art. Downloads start from the Download & install panel (setup_ui.rs) and install by
//! themselves when they finish; installing by hand stays for downloads from before that.
use crate::app::*;
use crate::audio::{self, Sound};
use crate::downloads::{Job, State};
use crate::installer::{Record, State as Install};
use crate::present::req_url;
use crate::setup_ui::size_label;
use crate::{ActionData, DlStat, DlTab};
use slint::{Model, ModelRc, VecModel};
use tr::tr;

/// The list (right): a row's height, and a group heading's above its first row (design px).
const LIST_ROW: f32 = 104.0;
const LIST_HEADING: f32 = 44.0;

/// Tile width and gap on the shelf, and the extra gap before each group (design units).
const TILE: f32 = 288.0;
const GAP: f32 = 20.0;
const GROUP_GAP: f32 = 40.0;
/// The shelf's groups' names, in order: in progress, needs you, ready to play.
fn group_label(group: usize) -> String {
    match group {
        0 => tr!("In progress"),
        1 => tr!("Needs you"),
        _ => tr!("Ready to play"),
    }
}

/// How one download looks on the page.
struct Desc {
    group: i32,
    kind: i32,
    status: String,
    big: String,
    unit: String,
    stats: Vec<(String, String)>,
    spark: bool,
    note: String,
    dl: f32,
    inst: f32,
    ready: bool,
    dl_label: String,
    inst_label: String,
    bar: i32,
    /// (id, label, icon)
    actions: Vec<(&'static str, String, &'static str)>,
    line: String,
    tile: f32,
    /// Downloading or verifying right now.
    working: bool,
}

/// A menu item's icon for a download action (the page's own buttons carry theirs).
fn icon_of(id: &str) -> &'static str {
    match id {
        "pause" => "pause", "resume" | "play_install" | "play" => "play", "cancel" | "discard" | "cancel_install" => "close",
        "remove" | "delete" => "trash", "retry" => "retry", "share" => "share", "stop_seed" => "unshare",
        "install" => "disk", "locate" => "folder", "info" => "info", "vpn" => "gear", _ => "check",
    }
}

fn mbps(v: f64) -> String { format!("{} MB/s", crate::i18n::num(v, 1)) }

fn pct(f: f32) -> String { format!("{:.0}", (f * 100.0).clamp(0.0, 100.0).floor()) }

/// "Added 4 minutes ago", "Added yesterday", "Added Oct 30, 2026".
fn added_label(added: u64) -> String {
    if added == 0 { return String::new(); }
    let ago = (crate::util::now_secs() as i64 - added as i64).max(0);
    match ago {
        0..60 => tr!("Added just now"),
        60..3600 => { let m = ago / 60; tr!("Added {n} minute ago" | "Added {n} minutes ago" % m) }
        3600..86400 => { let h = ago / 3600; tr!("Added {n} hour ago" | "Added {n} hours ago" % h) }
        _ => {
            // The day's words, in English or already in the player's language.
            let day = crate::util::fmt_last_played(added as f64);
            if day == "Today" || day == tr!("Today") { tr!("Added today") }
            else if day == "Yesterday" || day == tr!("Yesterday") { tr!("Added yesterday") }
            else { tr!("Added {day}", day) }
        }
    }
}

/// The top bar ring's arc for `frac` (0–1), on a 36×36 box: a circle of radius 15 from the top,
/// clockwise.
fn ring_path(frac: f32) -> String {
    let f = frac.clamp(0.0, 1.0);
    if f >= 0.999 { return "M 18 3 A 15 15 0 0 1 18 33 A 15 15 0 0 1 18 3".into(); }
    let a = f.max(0.01) * std::f32::consts::TAU;
    let (x, y) = (18.0 + 15.0 * a.sin(), 18.0 - 15.0 * a.cos());
    format!("M 18 3 A 15 15 0 {} 1 {x:.2} {y:.2}", if f > 0.5 { 1 } else { 0 })
}

/// A name short enough for the top bar.
fn short_name(name: &str) -> String {
    if name.chars().count() <= 26 { return name.into(); }
    format!("{}…", name.chars().take(24).collect::<String>().trim_end())
}

/// The speed line: the last minutes of download speed, scaled to its own peak, on a 400×160 box.
fn spark_path(speeds: &std::collections::VecDeque<f32>) -> String {
    if speeds.len() < 2 { return String::new(); }
    let peak = speeds.iter().copied().fold(0.1f32, f32::max);
    let step = 400.0 / (speeds.len() - 1) as f32;
    speeds.iter().enumerate().map(|(i, v)| {
        format!("{}{:.1} {:.1}", if i == 0 { "M " } else { " L " }, i as f32 * step, 150.0 - v / peak * 140.0)
    }).collect()
}

fn describe(job: &Job, install: Option<&Record>, failure: Option<&String>, queued_behind: Option<&str>, sharing_on: bool) -> Desc {
    let mut d = Desc { group: 0, kind: 0, status: String::new(), big: String::new(), unit: String::new(), stats: vec![], spark: false,
        note: String::new(), dl: job.progress(), inst: 0.0, ready: false, dl_label: tr!("Downloading"),
        inst_label: tr!("Install"), bar: 0, actions: vec![],
        line: String::new(), tile: job.progress(), working: matches!(job.state, State::Checking | State::Downloading) };
    let size = size_label(job.total);
    let progress = |d: &mut Desc, f: f32| { d.big = pct(f); d.unit = "%".into(); };
    match job.state {
        State::Resolving => {
            d.status = tr!("Getting the file list");
            d.line = tr!("Getting the file list");
            // Looked up again after it had started: cancelling keeps what it has.
            d.actions = vec![(if crate::downloads::unstarted(job) { "discard" } else { "cancel" }, tr!("Cancel"), "close")];
        }
        State::Ready => {
            (d.group, d.kind) = (1, 1);
            d.stats = vec![(size.clone(), tr!("{n} file" | "{n} files" % job.file_count))];
            if job.starting {
                d.status = tr!("Starting the download");
                d.line = tr!("Starting");
            } else {
                d.status = tr!("Waiting for you to choose its files");
                d.line = tr!("Choose its files");
                // It couldn't start where (or with what) the player chose.
                d.note = crate::downloads::error_text(&job.error);
                d.actions = vec![("choose", tr!("Choose files"), "check"), (if crate::downloads::unstarted(job) { "discard" } else { "remove" }, tr!("Remove"), "trash")];
            }
        }
        State::Checking | State::Downloading => {
            // Resuming: the engine verifies what's on disk before downloading the rest (once it
            // reports the check; until then it looks as it did).
            // A new download has nothing on disk to verify: it looks like downloading.
            let verifying = job.state == State::Checking && job.check_total > 0 && job.done > 0;
            d.status = if verifying { tr!("Verifying what's already downloaded") } else { tr!("Downloading") };
            if verifying { d.dl_label = tr!("Verifying"); (d.kind, d.bar) = (4, 4); }
            // Never shown as done (a ✓) before the download goes on.
            let checked = if job.check_total > 0 { (job.checked as f32 / job.check_total as f32).min(0.99) } else { 0.0 };
            // Verifying, the number and the bars are the check's; then back to what's downloaded.
            progress(&mut d, if verifying { checked } else { job.progress() });
            if verifying { (d.dl, d.tile) = (checked, checked); }
            if job.state == State::Downloading {
                d.stats.push((mbps(job.speed), tr!("Download speed")));
                d.spark = true;
                if !job.eta.is_empty() { d.stats.push((job.eta.clone(), tr!("Time left"))); }
            }
            d.stats.push((tr!("{} of {}", size_label(job.done), size), tr!("Downloaded")));
            d.line = if verifying { tr!("Verifying · {}%", pct(checked)) }
                else if job.eta.is_empty() { format!("{}%", pct(job.progress())) } else { tr!("{}% · {} left", pct(job.progress()), job.eta) };
            d.actions = vec![("pause", tr!("Pause"), "pause"), ("cancel", tr!("Cancel"), "close")];
        }
        State::Paused => {
            (d.group, d.kind, d.bar) = (1, 1, 1);
            d.dl_label = tr!("Paused");
            progress(&mut d, job.progress());
            d.stats = vec![(tr!("{} of {}", size_label(job.done), size), tr!("Downloaded")),
                (size_label(job.total.saturating_sub(job.done)), tr!("Still to download"))];
            if job.waiting_vpn {
                d.status = tr!("Paused until your VPN connects");
                d.stats.push((tr!("By itself"), tr!("Resumes")));
                d.line = tr!("Waiting for your VPN");
                // Still being looked up when the VPN went: nothing to keep, so Cancel forgets it.
                let cancel = if job.total == 0 && job.done == 0 { "discard" } else { "cancel" };
                d.actions = vec![("vpn", tr!("VPN settings"), "gear"), (cancel, tr!("Cancel"), "close")];
            } else {
                d.status = tr!("Paused");
                d.line = tr!("Paused · {}%", pct(job.progress()));
                d.actions = vec![("resume", tr!("Resume"), "play"), ("cancel", tr!("Cancel"), "close")];
            }
        }
        State::Failed => {
            (d.group, d.kind, d.bar) = (1, 2, 2);
            progress(&mut d, job.progress());
            d.status = tr!("Stopped");
            d.dl_label = tr!("Stopped");
            d.note = crate::downloads::error_text(&job.error);
            d.stats = vec![(tr!("{} of {}", size_label(job.done), size), tr!("Downloaded, and kept"))];
            d.line = tr!("Stopped");
            // A release it can never download or install: nothing to retry, only to remove.
            let hopeless = job.done == 0 && matches!(job.problem, crate::downloads::Problem::Release | crate::downloads::Problem::Unsupported);
            d.actions = if hopeless { vec![(if crate::downloads::unstarted(job) { "discard" } else { "remove" }, tr!("Remove"), "trash")] } else { vec![("retry", tr!("Retry"), "retry"), ("delete", tr!("Delete…"), "trash")] };
        }
        State::Cancelled => {
            (d.group, d.kind, d.bar) = (1, 1, 1);
            progress(&mut d, job.progress());
            d.status = tr!("Cancelled · what's downloaded is kept");
            d.dl_label = tr!("Cancelled");
            d.line = tr!("Cancelled");
            d.actions = vec![("retry", tr!("Download again"), "retry"), ("delete", tr!("Delete…"), "trash")];
        }
        State::Complete => {
            d.dl = 1.0;
            d.dl_label = tr!("Downloaded");
            let sharing = job.seeding || job.verifying;
            // Sharing goes on and off; Delete (which asks what to delete) sits beside it.
            let share = if sharing { ("stop_seed", tr!("Stop sharing"), "unshare") } else { ("share", tr!("Share again"), "share") };
            let remove = ("delete", tr!("Delete…"), "trash");
            // Why it isn't sharing (its files changed, say), or a delete that didn't happen; an
            // install problem says more below.
            d.note = crate::downloads::error_text(&job.error);
            let failed = match install {
                // The latest attempt's reason (one that couldn't even start is newer than the record).
                Some(record) if record.state == Install::Failed => Some(failure.cloned().unwrap_or_else(|| record.error.clone())),
                Some(record) if record.state == Install::Installed || record.state.active() => None,
                _ => failure.cloned(),
            };
            match install.map(|r| r.state) {
                Some(state) if state.active() => {
                    let record = install.unwrap();
                    progress(&mut d, record.progress());
                    d.status = tr!("Installing");
                    d.inst = record.progress();
                    d.inst_label = tr!("Installing");
                    d.stats = vec![(record.state.label().into(), tr!("Now")),
                        (tr!("{} of {}", size_label(record.done), size_label(record.total.max(record.done))), tr!("Unpacked"))];
                    d.line = tr!("Installing · {}%", pct(record.progress()));
                    d.tile = record.progress();
                    d.actions = vec![("cancel_install", tr!("Cancel install"), "close")];
                }
                Some(Install::Installed) => {
                    (d.group, d.kind, d.bar) = (2, 3, 3);
                    d.status = tr!("Ready to play");
                    (d.inst, d.ready) = (1.0, true);
                    d.inst_label = tr!("Installed");
                    // Before sharing again, its files are verified first.
                    d.stats = if job.verifying {
                        vec![(size.clone(), tr!("Size")), (tr!("Verifying"), tr!("Before sharing"))]
                    } else if job.seeding {
                        vec![(tr!("{n} player" | "{n} players" % job.peers), tr!("Connected to you")),
                            (mbps(job.upload_speed), tr!("Sharing speed")), (size, tr!("Size"))]
                    } else {
                        vec![(size, tr!("Size")), (tr!("Not sharing"), if sharing_on { tr!("Share it again any time") } else { tr!("Sharing is off in Settings") })]
                    };
                    d.line = if job.verifying { tr!("Ready · verifying") } else if job.seeding { tr!("Ready · sharing") } else { tr!("Ready to play") };
                    d.tile = 1.0;
                    d.actions = vec![("play_install", tr!("Play"), "play"), share, remove];
                }
                _ if failed.is_some() => {
                    (d.group, d.kind, d.bar) = (1, 2, 2);
                    d.status = tr!("Couldn't install");
                    d.note = failed.unwrap_or_default();
                    d.inst_label = tr!("Install failed");
                    d.line = tr!("Couldn't install");
                    d.actions = vec![("install", tr!("Retry install"), "retry"), share, remove];
                }
                Some(_) => {
                    (d.group, d.kind) = (1, 1);
                    d.status = tr!("Install cancelled");
                    d.line = tr!("Install cancelled");
                    d.actions = vec![("install", tr!("Install"), "disk"), share, remove];
                }
                None if job.install_to.is_some() => {
                    d.status = tr!("Waiting to install");
                    d.inst_label = tr!("Install · next in line");
                    d.stats = vec![(queued_behind.map_or(tr!("Next"), |name| tr!("After {name}", name)), tr!("Installs by itself")), (size, tr!("Downloaded"))];
                    d.line = tr!("Installs next");
                    d.tile = 1.0;
                    d.actions = vec![share, remove];
                }
                None => {
                    (d.group, d.kind) = (1, 1);
                    d.status = tr!("Downloaded · not installed yet");
                    d.stats = vec![(size, tr!("Downloaded"))];
                    d.line = tr!("Not installed");
                    d.tile = 1.0;
                    d.actions = vec![("install", tr!("Install"), "disk"), share, remove];
                }
            }
        }
    }
    // Its files aren't where it downloaded them, and weren't found where SwarmPlay looks (the
    // places in Storage, every drive): found with the folder picker, rather than downloading it
    // all again or a "couldn't install" that never says why.
    let installed = install.is_some_and(|r| r.state == Install::Installed);
    if job.done > 0 && !job.waiting_vpn && !install.is_some_and(|r| r.state.active())
        && matches!(job.state, State::Paused | State::Failed | State::Cancelled | State::Complete) && !job.folder.is_dir() {
        let locate = ("locate", tr!("Find the files"), "folder");
        if installed {
            d.note = tr!("Its download isn't where it was: find it to share it again");
            d.actions = vec![("play_install", tr!("Play"), "play"), locate, ("delete", tr!("Delete…"), "trash")];
        } else {
            (d.group, d.kind, d.bar) = (1, 2, 2);
            d.status = tr!("Its files were moved");
            d.line = tr!("Files moved");
            let was = job.folder.parent().unwrap_or(&job.folder);
            d.note = tr!("They aren't in {} any more: find where they went", crate::util::display_path(&was.to_string_lossy()));
            let keep = d.actions.iter().find(|a| matches!(a.0, "delete" | "remove" | "cancel" | "discard")).cloned();
            d.actions = std::iter::once(locate).chain(keep).collect();
        }
    }
    d
}

impl App {
    /// Download pressed: the Download & install panel, or the download if it's already under way.
    pub fn prepare_download(&mut self, target: Target) {
        if let Some(job) = self.download_for_target(target).filter(|job| job.state != State::Ready) {
            self.open_downloads(Some(&job.key));
            return;
        }
        self.open_setup(target);
    }

    pub fn open_downloads(&mut self, key: Option<&str>) {
        // Already open under what's shown (Downloads → VPN settings → Open downloads): back to
        // it, not a second one on top.
        while self.overlay != Overlay::Downloads && self.stack.iter().any(|(overlay, _, _)| *overlay == Overlay::Downloads) {
            // What's being typed in Settings is kept, as when clicking away from it.
            if self.overlay == Overlay::Settings && self.edit_index >= 0 {
                let text = self.ui().get_edit_text().to_string();
                self.finish_edit(Some(text));
            }
            self.back();
        }
        // Looked at: the ring stops asking about what stopped.
        let needs = self.ring_needs(&self.downloads.snapshot(), &self.installer.snapshot());
        self.ring_seen.extend(needs);
        self.ring_done.clear();
        // A new visit: a game started from a row last time isn't news on it now.
        if self.overlay != Overlay::Downloads { self.played_from.clear(); }
        if self.overlay != Overlay::Downloads {
            self.push_overlay(Overlay::Downloads, Z_DOWNLOADS, 0);
            self.dl_shelf = 0.0;
        }
        let ui = self.ui();
        ui.set_download_consent(false);
        ui.set_download_is_install(false);
        ui.set_download_focus_path(false);
        ui.set_download_editing(false);
        if let Some(key) = key {
            self.dl_sel = Some(key.into());
            self.dl_filter = 0;
        }
        self.dl_btn = -1;
        self.push_downloads();
        ui.invoke_focus_root();
    }

    /// The Options button on a download: every action it has, in the Options menu over the page.
    fn download_more(&mut self, key: &str, sel: usize) {
        let Some(row) = self.download_model.row_data(sel) else { return };
        // Menu items keep a static id: the actions a download can have.
        const IDS: [&str; 17] = ["pause", "resume", "cancel", "discard", "remove", "delete", "retry", "choose", "share", "stop_seed",
            "play_install", "install", "cancel_install", "locate", "info", "vpn", "play"];
        let items: Vec<ActionDef> = row.actions.iter().filter_map(|a| {
            let id = IDS.iter().copied().find(|i| *i == a.id.as_str())?;
            Some(ActionDef { id, label: a.label.to_string(), icon: icon_of(id), primary: false, danger: a.danger, round: false })
        }).collect();
        if items.is_empty() { return; }
        audio::play(Sound::Select);
        self.menu_target = None;
        self.menu_actions = items;
        self.ui().set_menu_anchor(0);
        self.push_overlay(Overlay::Menu, Z_MENU, 0);
        self.menu_download = Some(key.to_string());
        self.push_menu(&row.title);
    }

    /// Only installing by hand asks for confirmation now (downloads start from the panel).
    pub fn confirm_download(&mut self) {
        if self.overlay != Overlay::Downloads || !self.ui().get_download_consent() { return; }
        if self.ui().get_download_is_install() { self.confirm_install(); }
    }

    pub fn download_action(&mut self, key: &str, action: &str) {
        if self.overlay != Overlay::Downloads || self.ui().get_download_consent() { return; }
        let jobs = self.downloads.snapshot();
        let Some(job) = jobs.iter().find(|job| job.key == key) else { return };
        let install = self.installer.snapshot().into_iter().find(|record| record.key == key);
        // Nothing that changes the download while it's being installed from.
        let touches = !matches!(action, "info" | "cancel_install" | "play_install" | "vpn" | "stop_seed" | "share" | "choose");
        self.dl_notes.remove(key);
        self.dl_page_note.clear();
        // A game started from this row: what it says isn't this press's news. (From another
        // row: that row keeps saying it.)
        self.played_from.remove(key);
        if touches && install.as_ref().is_some_and(|record| record.state.active()) {
            return self.note_download(key, &tr!("It's installing: cancel the installation first"));
        }
        match action {
            "info" => {
                if let Some(path) = install.as_ref().filter(|record| record.state == Install::Installed).and_then(|record| record.path.clone()) {
                    self.open_games(None, Some(&path));
                }
            }
            "install" => self.install_download(key),
            "locate" => self.open_folder_pick(crate::folder_pick::PickFor::Locate(key.to_string())),
            "cancel_install" => {
                if let Err(error) = self.installer.cancel(key) { self.note_download_quiet(key, tr!("Couldn't cancel it: {error}", error)); }
            }
            "play_install" => {
                self.push_installs();
                if let Some(path) = install.as_ref().filter(|record| record.state == Install::Installed).and_then(|record| record.path.as_ref()) {
                    if let Some(local) = self.locals.iter().position(|game| &game.l.path == path) {
                        // A game that can't start says why here, on its row (not behind this page).
                        let id = self.locals[local].l.id.clone();
                        self.played_from.insert(key.to_string(), id);
                        self.run_action("play", Target { game: self.locals[local].cat, local: Some(local) });
                    } else { self.note_download(key, &tr!("Its folder is missing: check Game folders in Settings")); }
                }
                return;
            }
            // On Your VPN (✕ connects it), not on the switch (✕ there would turn protection off).
            "vpn" => { self.open_setting(crate::settings::SId::Vpn); return; }
            "delete" => return self.open_delete(key),
            "discard" => {
                if let Err(error) = self.downloads.discard(key) { self.note_download_quiet(key, tr!("Couldn't do that: {error}", error)); }
                else { audio::play(Sound::Select); }
            }
            "choose" => {
                let game = self.games.iter().position(|game| game.g.id == job.topic);
                let target = Target { game, local: game.and_then(|g| self.games[g].local) };
                self.open_setup_with(target, Some(job.clone()));
                return;
            }
            _ => {
                if let Err(error) = self.downloads.action(key, action) { self.note_download_quiet(key, tr!("Couldn't do that: {error}", error)); }
                else { audio::play(Sound::Select); }
            }
        }
        self.push_downloads();
    }

    /// What pressing something on a download just did, on its row until the next press there.
    pub fn note_download(&mut self, key: &str, text: &str) {
        self.note_download_quiet(key, text);
        self.push_downloads();
    }

    /// The same, for the page to show on its next push.
    pub(crate) fn note_download_quiet(&mut self, key: &str, text: impl AsRef<str>) {
        self.dl_notes.insert(key.to_string(), (crate::app::one_line(text.as_ref()), true));
    }

    pub fn push_downloads(&mut self) {
        let jobs = self.downloads.snapshot();
        self.dl_notes.retain(|key, _| jobs.iter().any(|job| &job.key == key));
        let installs = self.installer.snapshot();
        let mut changed = false;
        for job in &jobs {
            let old = self.download_states.insert(job.key.clone(), job.state);
            changed |= old != Some(job.state);
            if old.is_some_and(|o| o.active()) && old != Some(job.state) {
                match job.state {
                    State::Complete if job.install_to.is_some() => self.notify(crate::toast::Cat::Downloads, crate::toast::INFO, "download", &tr!("Downloaded"), &tr!("{} · installing now", job.name),
                        crate::toast::Action::Downloads(job.key.clone()), &format!("dl:{}", job.key), false),
                    State::Complete => self.notify(crate::toast::Cat::Downloads, crate::toast::INFO, "download", &tr!("Downloaded"), &tr!("{} · ready to install", job.name),
                        crate::toast::Action::Downloads(job.key.clone()), &format!("dl:{}", job.key), false),
                    State::Failed => self.notify(crate::toast::Cat::Downloads, crate::toast::FAILED, "alert", &tr!("Download stopped"), &job.name,
                        crate::toast::Action::Downloads(job.key.clone()), &format!("dl:{}", job.key), true),
                    _ => {}
                }
            }
            // A finished one installs by itself (or waits on its row); a stopped one turns the
            // ring amber.
            let speeds = self.dl_speeds.entry(job.key.clone()).or_default();
            if job.state == State::Downloading {
                speeds.push_back(job.speed as f32);
                if speeds.len() > 90 { speeds.pop_front(); }
            } else if job.state != State::Checking { speeds.clear(); }
        }
        // A download that went away (a lookup the panel forgot) changes the Game Hub's button too.
        let before = self.download_states.len();
        self.download_states.retain(|key, _| jobs.iter().any(|job| &job.key == key));
        changed |= self.download_states.len() != before;
        self.dl_speeds.retain(|key, _| jobs.iter().any(|job| &job.key == key));
        if changed && self.overlay == Overlay::Hub { self.push_hub(); }
        if self.overlay == Overlay::Setup { self.push_setup(); }
        if self.overlay != Overlay::Downloads { return; }

        // Newest first within each group: in progress, needs you, ready to play.
        let sharing_on = self.cfg.lock().unwrap().seed_after_download;
        let mut rows: Vec<(usize, Desc)> = jobs.iter().enumerate().map(|(i, job)| {
            let install = installs.iter().find(|record| record.key == job.key);
            let behind = installs.iter().find(|record| record.state.active() && record.key != job.key).map(|record| record.name.as_str());
            (i, describe(job, install, self.install_failures.get(&job.key), behind, sharing_on))
        }).collect();
        let counts = [rows.len(), rows.iter().filter(|r| r.1.group == 0).count(), rows.iter().filter(|r| r.1.group == 1).count(), rows.iter().filter(|r| r.1.group == 2).count()];
        rows.retain(|(_, d)| self.dl_filter == 0 || d.group == self.dl_filter as i32 - 1);
        rows.sort_by_key(|(i, d)| (d.group, std::cmp::Reverse((jobs[*i].added, *i))));
        let sel = self.dl_sel.as_ref().and_then(|key| rows.iter().position(|(i, _)| &jobs[*i].key == key)).unwrap_or(0);
        self.dl_sel = rows.get(sel).map(|(i, _)| jobs[*i].key.clone());
        let actions = rows.get(sel).map_or(0, |(_, d)| d.actions.len() + 1) as i32;
        if self.dl_btn >= actions { self.dl_btn = actions - 1; }

        // Keep the selected tile in view.
        let (w, _) = self.logical_size();
        let x_of = |n: usize| n as f32 * (TILE + GAP) + (1..=n).filter(|&k| rows[k].1.group != rows[k - 1].1.group).count() as f32 * GROUP_GAP;
        if !rows.is_empty() {
            let x = x_of(sel);
            let view = (w - 240.0).max(TILE);
            if x < self.dl_shelf { self.dl_shelf = x; }
            if x + TILE > self.dl_shelf + view { self.dl_shelf = x + TILE - view; }
        } else { self.dl_shelf = 0.0; }

        self.dl_keys.clear();
        let mut glass_key = None;
        let scale = self.scale;
        let mut data = Vec::with_capacity(rows.len());
        // The list's rows, under their group headings (design px).
        let mut list_y = 0.0f32;
        for (n, (i, d)) in rows.iter().enumerate() {
            let job = &jobs[*i];
            let game = self.games.iter().position(|game| game.g.id == job.topic);
            let target = Target { game, local: game.and_then(|g| self.games[g].local) };
            let info = game.and_then(|_| self.target_info(target));
            let want = |app: &mut App, r: Option<crate::present::ImgReq>, prio: u8| -> Option<slint::Image> {
                let r = r?;
                app.dl_keys.insert(r.key.clone());
                app.images.want(&r.key, r.src.clone(), r.w, r.crop, prio)
            };
            // The same picture as the big art (usually on disk already), decoded small.
            let tile_req = info.as_ref().filter(|i| !i.hub.is_empty()).map(|i| crate::present::ImgReq {
                key: format!("{}#tile", i.hub), src: crate::images::Src::Url(crate::psn::sized(&i.hub, 1920)), w: 640, crop: 0.0,
            });
            let tile = want(self, tile_req.or_else(|| game.and_then(|_| self.cover_req(target))), crate::images::prio::TILE);
            let (art, logo) = if n == sel {
                let bg = game.and_then(|_| self.target_bg_url(target));
                glass_key = bg.as_ref().map(|r| r.key.clone());
                (want(self, bg, crate::images::prio::HERO),
                 want(self, info.as_ref().and_then(|i| req_url(&i.logo, 960, 0.0)), crate::images::prio::HERO))
            } else { (None, None) };
            let (logo_w, logo_h) = logo.as_ref().map(|img| {
                let s = img.size();
                let k = (560.0 / s.width.max(1) as f32).min(140.0 / s.height.max(1) as f32);
                (s.width as f32 * k * scale, s.height as f32 * k * scale)
            }).unwrap_or((0.0, 0.0));
            let first = n == 0 || rows[n - 1].1.group != d.group;
            let mut actions: Vec<ActionData> = d.actions.iter().enumerate().map(|(k, (id, label, icon))| ActionData {
                // Delete is an icon beside the others until it's focused (it's never the main thing to do).
                id: (*id).into(), label: label.as_str().into(), icon: (*icon).into(), primary: k == 0, danger: *id == "delete", round: *id == "delete" && k >= 2,
            }).collect();
            // Installed: where it is and how big, on the installed games' list (no file manager).
            if installs.iter().any(|r| r.key == job.key && r.state == Install::Installed) {
                actions.push(ActionData { id: "info".into(), label: tr!("Information").into(), icon: "info".into(), primary: false, danger: false, round: true });
            }
            // What a press there did, else what the game started from it says now.
            let pressed = self.dl_notes.get(&job.key).cloned()
                .or_else(|| self.played_from.get(&job.key).and_then(|game| self.game_notes.get(game).cloned()));
            data.push(crate::DownloadData {
                key: job.key.clone().into(), title: job.name.clone().into(),
                has_art: art.is_some(), art: art.unwrap_or_default(),
                has_tile_art: tile.is_some(), tile_art: tile.unwrap_or_default(),
                has_logo: logo.is_some(), logo: logo.unwrap_or_default(), logo_w, logo_h,
                status: d.status.clone().into(), kind: d.kind, when: added_label(job.added).into(),
                big: d.big.clone().into(), big_unit: d.unit.clone().into(),
                stats: ModelRc::new(VecModel::from(d.stats.iter().map(|(v, l)| DlStat { value: v.clone().into(), label: l.clone().into() }).collect::<Vec<_>>())),
                spark: if d.spark { self.dl_speeds.get(&job.key).map(spark_path).unwrap_or_default() } else { String::new() }.into(),
                // What pressing something on it just did, else what it needs.
                note: pressed.as_ref().map(|(text, _)| text.clone()).unwrap_or_else(|| d.note.clone()).into(),
                // What a press there couldn't do: in amber, whatever the download's own colour.
                // A finished or ready download's own error is a problem too (its files changed, a
                // delete refused, it couldn't start with what was chosen).
                note_bad: pressed.as_ref().map_or(matches!(job.state, State::Complete | State::Ready) && !job.error.is_empty(), |(_, bad)| *bad),
                dl_frac: d.dl, inst_frac: d.inst, ready: d.ready, dl_label: d.dl_label.clone().into(), inst_label: d.inst_label.clone().into(), bar_kind: d.bar,
                actions: ModelRc::new(VecModel::from(actions)),
                line: d.line.clone().into(), tile_frac: d.tile, working: d.working,
                group: d.group, group_first: first, group_label: group_label(d.group as usize).into(),
                row_y: { if first { list_y += LIST_HEADING; } let y = list_y; list_y += LIST_ROW; y },
            });
        }
        let glass = glass_key.and_then(|key| self.frost_of(&key));
        let tabs: Vec<DlTab> = [tr!("All"), group_label(0), group_label(1), group_label(2)].iter().enumerate().map(|(k, label)| DlTab {
            label: label.into(), count: counts[k].to_string().into(), warn: k == 2 && counts[2] > 0,
        }).collect();
        let (down, up) = jobs.iter().fold((0.0, 0.0), |(d, u), job| (d + job.speed, u + job.upload_speed));
        let vpn = { let cfg = self.cfg.lock().unwrap(); cfg.download_vpn().map(|preferred| crate::vpn::active(&preferred)) };
        let ui = self.ui();
        // Rebuilding the model every second would restart its images; only changed rows are set.
        if self.download_model.row_count() != data.len() {
            self.download_model.set_vec(data);
        } else {
            for (n, row) in data.into_iter().enumerate() {
                let old = self.download_model.row_data(n);
                // Every field (images compare by their pixels' address, so it's cheap); the
                // two models by their rows.
                let same = old.as_ref().is_some_and(|old| {
                    let mut old = old.clone();
                    let rows_same = old.stats.iter().eq(row.stats.iter()) && old.actions.iter().eq(row.actions.iter());
                    (old.stats, old.actions) = (row.stats.clone(), row.actions.clone());
                    rows_same && old == row
                });
                if !same { self.download_model.set_row_data(n, row); }
            }
        }
        // Set only when they change: new models rebuild their rows (and lose hover).
        if !ui.get_download_tabs().iter().eq(tabs.iter().cloned()) { ui.set_download_tabs(ModelRc::new(VecModel::from(tabs))); }
        ui.set_download_filter(self.dl_filter as i32);
        ui.set_download_sel(sel as i32);
        ui.set_download_focus_btn(self.dl_btn);
        ui.set_download_shelf_x(self.dl_shelf * scale);
        ui.set_download_down(mbps(down).into());
        ui.set_download_up(mbps(up).into());
        let (vpn_text, vpn_kind) = match vpn { None => (String::new(), 0), Some(Some(_)) => (tr!("VPN on"), 1), Some(None) => (tr!("Waiting for your VPN"), 2) };
        ui.set_download_delete_open(self.dl_delete.is_some());
        ui.set_download_delete_title(self.dl_delete.as_ref().and_then(|(key, _)| jobs.iter().find(|job| &job.key == key)).map(|job| job.name.clone()).unwrap_or_default().into());
        let choices = self.delete_choices();
        if !ui.get_download_delete_choices().iter().eq(choices.iter().cloned()) { ui.set_download_delete_choices(ModelRc::new(VecModel::from(choices))); }
        ui.set_download_delete_sel(self.dl_delete_sel as i32);
        ui.set_download_has_glass(glass.is_some());
        ui.set_download_glass(glass.unwrap_or_default());
        ui.set_download_has_any(!jobs.is_empty());
        ui.set_download_page_note(self.dl_page_note.as_str().into());
        ui.set_download_empty_title(match self.dl_filter { 0 => tr!("Nothing downloading"), 1 => tr!("Nothing in progress"), 2 => tr!("Nothing needs you"), _ => tr!("Nothing ready to play yet") }.into());
        ui.set_download_vpn(vpn_text.into());
        ui.set_download_vpn_kind(vpn_kind);
        ui.set_zone(Z_DOWNLOADS);
    }

    /// The downloads the shelf shows, in its order (after the filter).
    fn shelf_keys(&self) -> Vec<String> {
        self.download_model.iter().map(|row| row.key.to_string()).collect()
    }

    pub fn select_download(&mut self, index: usize, mouse: bool) {
        let keys = self.shelf_keys();
        let Some(key) = keys.get(index) else { return };
        if self.dl_sel.as_deref() != Some(key.as_str()) { audio::play(Sound::Move); }
        self.dl_sel = Some(key.clone());
        if mouse || self.dl_btn >= 0 { self.dl_btn = -1; }
        self.push_downloads();
    }

    pub fn filter_downloads(&mut self, filter: usize) {
        if filter > 3 { return; }
        if filter != self.dl_filter { audio::play(Sound::Tab); }
        self.dl_filter = filter;
        self.dl_btn = -1;
        self.push_downloads();
    }

    /// Delete asks what to delete: only what this download has.
    fn open_delete(&mut self, key: &str) {
        let Some(job) = self.downloads.job(key) else { return };
        let installed = self.installer.snapshot().into_iter().any(|r| r.key == key && r.state == Install::Installed);
        let files = job.folder.is_dir();
        let mut ids = vec![];
        if installed && files { ids.push("both"); }
        if installed { ids.push("game"); }
        if files { ids.push("download"); }
        ids.extend(["forget", "cancel"]);
        self.dl_delete_sel = ids.len() - 1;
        self.dl_delete = Some((key.into(), ids));
        audio::play(Sound::Select);
        self.push_downloads();
    }

    fn delete_choices(&self) -> Vec<crate::DlChoice> {
        let Some((key, ids)) = &self.dl_delete else { return vec![] };
        let job = self.downloads.job(key);
        let install = self.installer.snapshot().into_iter().find(|r| &r.key == key);
        let dl = job.as_ref().map_or(0, |job| job.done.max(if job.state == State::Complete { job.total } else { 0 }));
        let game = install.as_ref().map_or(0, |r| r.total);
        let sharing = job.as_ref().is_some_and(|job| job.seeding || job.verifying);
        ids.iter().map(|id| {
            let (label, detail, danger) = match *id {
                "both" => (tr!("Delete the game and its download"), tr!("Frees {}. It leaves the Library and this list.", size_label(dl + game)), true),
                "game" => (tr!("Delete the installed game"), if sharing {
                    tr!("Frees {}. The download stays and keeps sharing, and Install puts the game back.", size_label(game))
                } else {
                    tr!("Frees {}. The download stays, and Install puts the game back.", size_label(game))
                }, true),
                "download" => (tr!("Delete the download"), if install.as_ref().is_some_and(|r| r.state == Install::Installed) {
                    tr!("Frees {}. The game stays installed; sharing it stops.", size_label(dl)) } else { tr!("Frees {}.", size_label(dl)) }, true),
                "forget" => (tr!("Remove from Downloads only"), tr!("Keeps every file. It just leaves this list."), false),
                _ => (tr!("Cancel"), String::new(), false),
            };
            crate::DlChoice { label: label.into(), detail: detail.into(), danger }
        }).collect()
    }

    /// A choice on the Delete dialog (-1: closed without one).
    pub fn delete_choose(&mut self, index: i32) {
        let Some((key, ids)) = self.dl_delete.take() else { return };
        let choice = usize::try_from(index).ok().and_then(|i| ids.get(i).copied()).unwrap_or("cancel");
        let name = self.downloads.job(&key).map(|job| job.name).unwrap_or_default();
        // It may have started installing while the dialog was open.
        self.dl_notes.remove(&key);
        if choice != "cancel" && self.installer.snapshot().iter().any(|r| r.key == key && r.state.active()) {
            return self.note_download(&key, &tr!("It's installing: cancel the installation before deleting it"));
        }
        // Before anything goes: the download's folder must be its own to delete.
        if matches!(choice, "both" | "download") {
            if let Err(error) = self.downloads.check_delete(&key) {
                return self.note_download(&key, &tr!("Couldn't delete it: {}", format!("{error:#}")));
            }
        }
        if matches!(choice, "both" | "game") {
            // Never under a running game.
            let path = self.installer.snapshot().into_iter().find(|r| r.key == key).and_then(|r| r.path);
            let running = path.as_ref().and_then(|p| self.locals.iter().position(|l| &l.l.path == p)).is_some_and(|l| self.session_for_local(l).is_some());
            if running {
                return self.note_download(&key, &tr!("Quit the game first: it's running"));
            }
        }
        if choice == "cancel" { audio::play(Sound::Back); return self.push_downloads(); }
        // First, so the install queue never (re)installs it while (or after) anything goes.
        let install_to = self.downloads.job(&key).and_then(|job| job.install_to);
        self.downloads.set_install_to(&key, None);
        // The download first (it can still be refused), then the game: a refusal leaves both.
        let queued = match choice {
            "both" | "download" => self.downloads.action(&key, "delete"),
            "forget" => self.downloads.action(&key, "remove"),
            _ => Ok(()),
        };
        if let Err(error) = queued {
            self.downloads.set_install_to(&key, install_to);
            return self.note_download(&key, &tr!("Couldn't delete it: {}", format!("{error:#}")));
        }
        // Both: the game goes once the download is gone (the worker can still refuse it).
        if choice == "both" {
            self.pending_uninstall = Some((key.clone(), name.clone(), install_to));
            audio::play(Sound::Select);
            self.dl_btn = -1;
            return self.push_downloads();
        }
        if choice == "game" {
            match self.installer.uninstall(&key) {
                Ok(_) => {
                    self.install_states.remove(&key);
                    self.rescan_library();
                }
                Err(error) => {
                    self.downloads.set_install_to(&key, install_to);
                    return self.note_download(&key, &tr!("Couldn't delete the game: {}", format!("{error:#}")));
                }
            }
        }
        if matches!(choice, "download" | "forget") {
            // An install that failed goes with it: downloaded again, it installs afresh.
            let _ = self.installer.forget(&key);
            self.install_failures.remove(&key);
        }
        audio::play(Sound::Select);
        // Its row changes (or goes a moment later, once the worker has deleted the download).
        self.dl_btn = -1;
        self.push_downloads();
    }

    /// "Delete the game and its download", second half: once the worker has deleted the download,
    /// the game goes; if it refused, the game stays (and installs by itself again as before).
    pub fn settle_pending_uninstall(&mut self) {
        let Some((key, name, install_to)) = self.pending_uninstall.clone() else { return };
        match self.downloads.job(&key) {
            None => {
                self.pending_uninstall = None;
                // Started meanwhile: it stays (never deleted under a running game).
                let path = self.installer.snapshot().into_iter().find(|r| r.key == key).and_then(|r| r.path);
                // The download's row is gone: what happened to the game shows on the game.
                let local = path.as_ref().and_then(|p| self.locals.iter().position(|l| &l.l.path == p));
                // On the Downloads page too, at its foot (the row it was on is gone).
                let note = |app: &mut App, text: &str| {
                    if let Some(l) = local { let id = app.locals[l].l.id.clone(); app.note_game(&id, text, true) }
                    app.dl_page_note = crate::app::one_line(&format!("{name}: {text}"));
                    app.push_downloads();
                };
                if local.is_some_and(|l| self.session_for_local(l).is_some()) {
                    crate::log!("{name}: download deleted; the game is running, so it stays");
                    return note(self, &tr!("Its download is deleted; the game was running, so it stays"));
                }
                match self.installer.uninstall(&key) {
                    Ok(_) => {
                        self.install_states.remove(&key);
                        self.rescan_library();
                    }
                    Err(error) => {
                        crate::log!("{name}: download deleted, game not: {error:#}");
                        note(self, &tr!("Its download is deleted, but the game couldn't be: {}", format!("{error:#}")));
                    }
                }
            }
            Some(job) if job.error.starts_with(crate::downloads::NOT_DELETED) => {
                self.pending_uninstall = None;
                self.downloads.set_install_to(&key, install_to);
                self.note_download(&key, &tr!("Couldn't delete its download, so the game stays: {}", crate::downloads::error_text(&job.error)));
            }
            Some(_) => {}
        }
    }

    /// Downloads that need the player: stopped ones, and finished ones that couldn't install.
    fn ring_needs(&self, jobs: &[Job], installs: &[Record]) -> Vec<String> {
        jobs.iter().filter(|job| match job.state {
            State::Failed => job.total > 0,
            State::Complete => self.install_failures.contains_key(&job.key)
                || installs.iter().any(|r| r.key == job.key && r.state == Install::Failed),
            _ => false,
        }).map(|job| job.key.clone()).collect()
    }

    /// The Control Center's Downloads card, saying what's true: the download under way and how
    /// far, else how many need you (stopped, ready to install, couldn't install), else how many
    /// are paused; only then nothing.
    fn push_cc_downloads(&self, jobs: &[Job], installs: &[Record]) {
        let installed = |j: &Job| installs.iter().any(|r| r.key == j.key && r.state == Install::Installed);
        let active: Vec<&Job> = jobs.iter().filter(|j| j.state.active()).collect();
        let needs = jobs.iter().filter(|j| match j.state {
            State::Failed => j.total > 0,
            State::Complete => !installed(j) && !installs.iter().any(|r| r.key == j.key && r.state.active()),
            State::Ready => !j.starting,
            _ => false,
        }).count();
        let paused = jobs.iter().filter(|j| j.state == State::Paused).count();
        let (title, sub, frac, kind) = match active.first() {
            Some(j) => {
                let more = if active.len() > 1 { tr!(" · {} more", active.len() - 1) } else { String::new() };
                let left = if j.eta.is_empty() { String::new() } else { tr!("{} left", j.eta) };
                let extra = if needs > 0 { tr!("{n} needs you" | "{n} need you" % needs) } else { String::new() };
                (format!("{} · {}%{more}", j.name, pct(j.progress())), [left, extra].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · "), j.progress(), 0)
            }
            None if needs > 0 => (tr!("{n} needs you" | "{n} need you" % needs), tr!("Open Downloads"), -1.0, 3),
            None if paused > 0 => (tr!("{n} paused" | "{n} paused" % paused), tr!("Open Downloads"), -1.0, 0),
            None => (tr!("Nothing downloading"), tr!("Open Downloads"), -1.0, 0),
        };
        let ui = self.ui();
        ui.set_cc_dl_title(title.into());
        ui.set_cc_dl_sub(if sub.is_empty() { tr!("Open Downloads") } else { sub }.into());
        ui.set_cc_dl_frac(frac);
        ui.set_cc_dl_kind(kind);
    }

    /// The top bar's ring: what needs you (until you've looked), else what's downloading, else
    /// what's installing, else what waits for the VPN. Gone when nothing is happening (the
    /// Downloads icon is there then).
    pub fn push_ring(&mut self) {
        let jobs = self.downloads.snapshot();
        let installs = self.installer.snapshot();
        self.push_cc_downloads(&jobs, &installs);
        let needs = self.ring_needs(&jobs, &installs);
        self.ring_seen.retain(|key| needs.contains(key));
        // Seen as it happens on the Downloads page: nothing to ask about after.
        if self.overlay == Overlay::Downloads { self.ring_seen.extend(needs.iter().cloned()); }
        let unseen: Vec<&Job> = jobs.iter().filter(|job| needs.contains(&job.key) && !self.ring_seen.contains(&job.key)).collect();
        let active: Vec<&Job> = jobs.iter().filter(|job| job.state.active()).collect();
        // What finished since the last look (nothing else says so): ready until Downloads opens.
        // (One that's downloaded and about to install by itself isn't ready yet.)
        let waits = |job: &&Job| job.state == State::Complete && job.install_to.is_some()
            && !installs.iter().any(|r| r.key == job.key) && !self.install_failures.contains_key(&job.key);
        let busy: std::collections::HashSet<String> = active.iter().map(|job| job.key.clone())
            .chain(jobs.iter().filter(waits).map(|job| job.key.clone()))
            .chain(installs.iter().filter(|r| r.state.active()).map(|r| r.key.clone())).collect();
        for key in self.ring_busy.difference(&busy) {
            if !self.ring_done.contains(key) { self.ring_done.push(key.clone()); }
        }
        self.ring_busy = busy;
        if self.overlay == Overlay::Downloads { self.ring_done.clear(); }
        self.ring_done.retain(|key| !needs.contains(key) && !self.ring_busy.contains(key)
            && jobs.iter().any(|job| &job.key == key && job.state == State::Complete));
        let installed = |key: &str| installs.iter().any(|r| r.key == key && r.state == Install::Installed);
        // What the ring says (its words, then a percentage), and the download it opens: worked
        // out together, so pressing it opens what it says.
        let none = || -> Option<String> { None };
        let (kind, words, suffix, frac, key) = if let Some(job) = unseen.first() {
            let what = if job.state == State::Failed { tr!("Download stopped") } else { tr!("Couldn't install") };
            (3, if unseen.len() > 1 { tr!("{} need you", unseen.len()) } else { what }, String::new(), job.progress(), Some(job.key.clone()))
        } else if !active.is_empty() {
            // One that's verifying shows how far the check is.
            let verifying = active.len() == 1 && active[0].state == State::Checking && active[0].check_total > 0 && active[0].done > 0;
            let (done, total) = active.iter().fold((0u64, 0u64), |(d, t), job| if verifying { (d + job.checked, t + job.check_total) } else { (d + job.done, t + job.total) });
            // A check never reads done (100%) before the download goes on, as on the page.
            let frac = if total > 0 { (done as f32 / total as f32).min(if verifying { 0.99 } else { 1.0 }) } else { 0.0 };
            let one = (active.len() == 1).then(|| active[0].key.clone());
            let suffix = format!(" · {}%", pct(frac));
            let (words, suffix) = if active.len() > 1 { (tr!("{n} download" | "{n} downloads" % active.len()), suffix) }
                else if verifying { (tr!("Verifying"), suffix) }
                // Just pressed Download: being looked up.
                else if active[0].state == State::Resolving { (tr!("Getting the file list"), String::new()) }
                else { (short_name(&active[0].name), suffix) };
            (if verifying { 4 } else { 1 }, words, suffix, frac, one)
        } else if let Some(record) = installs.iter().find(|r| r.state.active()) {
            (2, tr!("Installing"), format!(" · {}%", pct(record.progress())), record.progress(), Some(record.key.clone()))
        } else if let Some(job) = jobs.iter().find(|job| job.waiting_vpn && job.state != State::Complete) {
            (3, tr!("Waiting for your VPN"), String::new(), job.progress(), Some(job.key.clone()))
        } else if let Some(key) = self.ring_done.last().cloned() {
            let all = self.ring_done.iter().all(|key| installed(key));
            let words = match (self.ring_done.len(), all) {
                (1, true) => tr!("Ready to play"),
                (1, false) => tr!("Downloaded"),
                (n, true) => tr!("{n} ready to play", n),
                (n, false) => tr!("{n} ready", n),
            };
            (5, words, String::new(), 1.0, Some(key))
        } else {
            (0, String::new(), String::new(), 0.0, none())
        };
        self.ring_key = key;
        // A focus that was on the ring stays on Downloads (the icon in its place). The
        // percentage's width is with every digit a 0, so it doesn't jiggle as numbers change.
        let sizer: String = suffix.chars().map(|c| if c.is_ascii_digit() { '0' } else { c }).collect();
        let ui = self.ui();
        ui.set_ring_kind(kind);
        ui.set_ring_label(words.into());
        ui.set_ring_suffix(suffix.into());
        ui.set_ring_sizer(sizer.into());
        ui.set_ring_path(ring_path(frac).into());
        // The ring went (nothing's happening): its place in the top bar goes too.
        if kind == 0 && self.zone == Z_TOP && self.idx == 4 && self.overlay == Overlay::None {
            self.set_focus(Z_TOP, 3);
        }
    }

    /// The top bar's glass: the art behind it, blurred (none over the dimmed Library). Also when
    /// the art changes, so it never shows the one before.
    pub fn push_bar_glass(&mut self, make: bool) {
        let Ok(glass) = self.bg_glass(make) else { return };
        let ui = self.ui();
        ui.set_bar_has_glass(glass.is_some());
        ui.set_bar_glass(glass.unwrap_or_default());
    }

    pub fn act_downloads(&mut self, action: Act) {
        if let Some((_, ids)) = &self.dl_delete {
            let n = ids.len();
            match action {
                Act::Up if self.dl_delete_sel > 0 => { self.dl_delete_sel -= 1; audio::play(Sound::Move); }
                Act::Down if self.dl_delete_sel + 1 < n => { self.dl_delete_sel += 1; audio::play(Sound::Move); }
                Act::Confirm => return self.delete_choose(self.dl_delete_sel as i32),
                Act::Back => return self.delete_choose(-1),
                _ => return,
            }
            return self.push_downloads();
        }
        if self.ui().get_download_consent() {
            if action == Act::Back {
                self.install_pending = None;
                self.ui().set_download_consent(false);
                self.push_downloads();
                return;
            }
            // Enter while typing only finishes editing; another explicit confirmation is required.
            if action == Act::Confirm { self.confirm_download(); }
            return;
        }
        let keys = self.shelf_keys();
        let sel = self.dl_sel.as_ref().and_then(|key| keys.iter().position(|k| k == key)).unwrap_or(0);
        let buttons = self.download_model.row_data(sel).map_or(0, |row| row.actions.row_count()) as i32;
        match action {
            Act::Back if self.dl_btn >= 0 => { self.dl_btn = -1; audio::play(Sound::Back); }
            Act::Back => return self.back_soft(),
            Act::TabPrev => return self.filter_downloads((self.dl_filter + 3) % 4),
            Act::TabNext => return self.filter_downloads((self.dl_filter + 1) % 4),
            _ if keys.is_empty() => return,
            // The chosen one's buttons (left): ◂ ▸ between them; past the last, back to the list.
            Act::Left if self.dl_btn > 0 => { self.dl_btn -= 1; audio::play(Sound::Move); }
            Act::Right if self.dl_btn >= 0 && self.dl_btn + 1 < buttons => { self.dl_btn += 1; audio::play(Sound::Move); }
            Act::Right if self.dl_btn >= 0 => { self.dl_btn = -1; audio::play(Sound::Move); }
            Act::Left | Act::Up | Act::Down if self.dl_btn >= 0 => {}
            // The list (right), as a console's: ▴ ▾ go through it, ✕ does what matters now, the
            // Options button has the rest, ◂ goes to the buttons.
            Act::Up => return self.select_download(sel.saturating_sub(1), false),
            Act::Down => return self.select_download((sel + 1).min(keys.len() - 1), false),
            Act::First => return self.select_download(0, false),
            Act::Last => return self.select_download(keys.len() - 1, false),
            Act::Left if buttons > 0 => { self.dl_btn = 0; audio::play(Sound::Move); }
            Act::Confirm if self.dl_btn < 0 => {
                if let Some(id) = self.download_model.row_data(sel).and_then(|row| row.actions.row_data(0)).map(|a| a.id.to_string()) {
                    return self.download_action(&keys[sel], &id);
                }
            }
            Act::Options if self.dl_btn < 0 => return self.download_more(&keys[sel].clone(), sel),
            Act::Confirm if self.dl_btn >= 0 => {
                if let Some(id) = self.download_model.row_data(sel).and_then(|row| row.actions.row_data(self.dl_btn as usize)).map(|a| a.id.to_string()) {
                    return self.download_action(&keys[sel], &id);
                }
            }
            _ => return,
        }
        self.push_downloads();
    }
}
