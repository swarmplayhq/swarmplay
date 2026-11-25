//! Notifications, as a console's. Every notice goes through `App::notify`: it's kept in a list
//! (7 days, 100 at most, saved between starts), and, by the rules in Settings → Notifications,
//! also shown: in the launcher as a stack of small cards at the top right (up to three, the newest
//! on top), over a game through the overlay, or as a desktop notification when SwarmPlay is in
//! the background. Each one can lead somewhere (play, the download, the log, restart to update);
//! the list opens from the Control Center, or with PS while one shows.

use crate::app::*;
use crate::audio::{self, Sound};
use crate::{NoteRow, ToastData};
use serde::{Deserialize, Serialize};
use tr::tr;

/// How a notification is coloured (ui: 0 info, 1 done, 2 needs a look, 3 failed).
pub const INFO: i32 = 0;
pub const DONE: i32 = 1;
pub const WARN: i32 = 2;
pub const FAILED: i32 = 3;

const KEEP_SECS: f64 = 7.0 * 86400.0;
const KEEP_MAX: usize = 100;
const STACK: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cat { Downloads, Installs, Games, Updates, Catalog, System }

/// The kinds, as Settings → Notifications names them. The names are also what the config keeps
/// (notify_off), so they stay English; `cat_label` is what's shown.
pub const CATS: [(Cat, &str); 6] = [
    (Cat::Downloads, "Downloads"), (Cat::Installs, "Installs"), (Cat::Games, "Games"),
    (Cat::Updates, "Updates"), (Cat::Catalog, "New in the catalog"), (Cat::System, "Controller & system"),
];

/// Where a notification leads.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Action {
    #[default]
    None,
    /// That download, on the Downloads page.
    Downloads(String),
    /// The game installed at this folder: play it.
    Play(String),
    /// A file to open (a log, a screenshot).
    File(String),
    /// Restart SwarmPlay (an update is waiting).
    Restart,
    /// A Settings section, by its header ("UPDATES", "STORAGE").
    Settings(String),
    /// The Library, newest first.
    Library,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Note {
    pub id: u64,
    pub cat: Cat,
    pub kind: i32,
    pub icon: String,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub action: Action,
    /// Same key: replaces the earlier one (the same download, the same battery).
    #[serde(default)]
    pub key: String,
    pub time: f64,
    #[serde(default)]
    pub read: bool,
    /// Shown even in Do not disturb and during games (when "Important only").
    #[serde(default)]
    pub important: bool,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Store {
    pub notes: Vec<Note>,
    next: u64,
}

impl Store {
    fn path() -> std::path::PathBuf {
        crate::util::config_dir().join("notifications.json")
    }

    pub fn load() -> Store {
        let mut s: Store = std::fs::read(Self::path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        s.prune(crate::util::now_secs());
        s
    }

    pub fn save(&self) {
        if let Ok(j) = serde_json::to_vec(self) { let _ = crate::util::atomic_write(&Self::path(), &j); }
    }

    /// Keep `n` (newest first); one with its key goes. Returns its id.
    pub fn add(&mut self, mut n: Note, now: f64) -> u64 {
        if !n.key.is_empty() { self.notes.retain(|o| o.key != n.key); }
        self.next += 1;
        n.id = self.next;
        n.time = now;
        self.notes.insert(0, n);
        self.prune(now);
        self.next
    }

    fn prune(&mut self, now: f64) {
        self.notes.retain(|n| now - n.time < KEEP_SECS);
        self.notes.truncate(KEEP_MAX);
    }

    pub fn unread(&self) -> usize {
        self.notes.iter().filter(|n| !n.read).count()
    }

    pub fn get(&self, id: u64) -> Option<&Note> {
        self.notes.iter().find(|n| n.id == id)
    }
}

/// Where a notification shows, besides the list.
#[derive(Debug, PartialEq, Eq)]
pub enum Show { Popup, Overlay, Desktop, ListOnly }

/// The rules in Settings → Notifications, as a pure choice (tested below).
pub struct Rules {
    pub popups: bool,
    /// 0 all, 1 important only, 2 none.
    pub games: u8,
    pub desktop: bool,
    pub dnd_until: f64,
    pub off: Vec<Cat>,
}

pub fn route(r: &Rules, n: &Note, now: f64, in_game: bool, background: bool) -> Show {
    if !r.popups || r.off.contains(&n.cat) { return Show::ListOnly; }
    if now < r.dnd_until && !n.important { return Show::ListOnly; }
    if in_game {
        return match r.games { 0 => Show::Overlay, 1 if n.important => Show::Overlay, _ => Show::ListOnly };
    }
    if background { if r.desktop { Show::Desktop } else { Show::ListOnly } } else { Show::Popup }
}

/// "now", "5 min", "2 h", "yesterday", "3 days".
pub fn age(now: f64, t: f64) -> String {
    let s = (now - t).max(0.0);
    if s < 60.0 { tr!("now") } else if s < 3600.0 { tr!("{} min", (s / 60.0) as u64) }
    else if s < 86400.0 { tr!("{} h", (s / 3600.0) as u64) } else if s < 2.0 * 86400.0 { tr!("yesterday") }
    else { let d = (s / 86400.0) as u64; tr!("{n} day" | "{n} days" % d) }
}

/// Games' names → the setting's words for "during games". These, DND's and the kinds' names are
/// the config's values (English keys); the `*_label` functions are what Settings shows.
pub const DURING_GAMES: [&str; 3] = ["All", "Important only", "None"];
pub const DURATIONS: [u32; 3] = [4, 6, 10];
pub const DND: [&str; 4] = ["Off", "For 1 hour", "Until tomorrow", "Until turned off"];

/// A kind of notification (CATS' name), as shown.
pub fn cat_label(name: &str) -> String {
    cat_text(name).unwrap_or_else(|| name.to_string())
}

fn cat_text(name: &str) -> Option<String> {
    Some(match name {
        "Downloads" => tr!("Downloads"),
        "Installs" => tr!("Installs"),
        "Games" => tr!("Games"),
        "Updates" => tr!("Updates"),
        "New in the catalog" => tr!("New in the catalog"),
        "Controller & system" => tr!("Controller & system"),
        _ => return None,
    })
}

/// "During games" (DURING_GAMES' value), as shown.
pub fn during_games_label(key: &str) -> String {
    during_games_text(key).unwrap_or_else(|| key.to_string())
}

fn during_games_text(key: &str) -> Option<String> {
    Some(match key {
        "All" => tr!("Notifications" => "All"),
        "Important only" => tr!("Important only"),
        "None" => tr!("Notifications" => "None"),
        _ => return None,
    })
}

/// Do not disturb (DND's value), as shown.
pub fn dnd_label(key: &str) -> String {
    dnd_text(key).unwrap_or_else(|| key.to_string())
}

fn dnd_text(key: &str) -> Option<String> {
    Some(match key {
        "Off" => tr!("Off"),
        "For 1 hour" => tr!("For 1 hour"),
        "Until tomorrow" => tr!("Until tomorrow"),
        "Until turned off" => tr!("Until turned off"),
        _ => return None,
    })
}

/// How long a pop-up stays (DURATIONS' value).
pub fn duration_label(secs: u32) -> String {
    tr!("{n} second" | "{n} seconds" % secs)
}

impl App {
    fn rules(&self) -> Rules {
        let c = self.cfg.lock().unwrap();
        Rules {
            popups: c.notify_popups,
            games: DURING_GAMES.iter().position(|g| *g == c.notify_games).unwrap_or(1) as u8,
            desktop: c.notify_desktop,
            dnd_until: c.notify_dnd_until,
            off: CATS.iter().filter(|(_, n)| c.notify_off.iter().any(|o| o == n)).map(|(k, _)| *k).collect(),
        }
    }

    /// A simple notice about SwarmPlay itself (no place to lead to).
    pub fn toast(&mut self, title: &str, sub: &str, icon: &str, kind: i32) {
        // The same message again takes the old one's place, in the list as in the stack.
        self.notify(Cat::System, kind, icon, title, sub, Action::None, &format!("sys:{title}"), false);
    }

    /// Keep a notification and show it where the rules say.
    #[allow(clippy::too_many_arguments)]
    pub fn notify(&mut self, cat: Cat, kind: i32, icon: &str, title: &str, body: &str, action: Action, key: &str, important: bool) {
        let now = crate::util::now_secs();
        let note = Note { id: 0, cat, kind, icon: icon.into(), title: title.into(), body: body.into(), action, key: key.into(), time: now, read: false, important };
        let id = self.notes.add(note.clone(), now);
        self.notes.save();
        self.toast_seq += 1;
        let in_game = self.in_game();
        let background = !in_game && !crate::display::window_has_focus(&self.ui());
        let show = route(&self.rules(), &note, now, in_game, background);
        crate::log!("notification: {title} · {:?}", show);
        let sound = self.cfg.lock().unwrap().notify_sound;
        match show {
            Show::Popup => {
                self.popups.retain(|(i, _)| *i != id);
                self.popups.insert(0, (id, now));
                self.popups.truncate(STACK);
                if sound { audio::play(Sound::Notify); }
            }
            Show::Overlay => {
                let data = ToastData { title: title.into(), sub: body.into(), icon: icon.into(), kind, id: id as i32, time: tr!("now").into() };
                let seq = self.toast_seq;
                self.ov_toast(&data, seq);
                if sound { audio::play(Sound::Notify); }
            }
            Show::Desktop => self.desktop_notice(id),
            Show::ListOnly => {}
        }
        self.push_notes();
    }

    /// The stack, the Control Center's card and the list, as they are now.
    pub fn push_notes(&mut self) {
        let now = crate::util::now_secs();
        let secs = self.cfg.lock().unwrap().notify_secs.max(2) as f64;
        self.popups.retain(|(_, t)| now - t < secs);
        let toasts: Vec<ToastData> = self.popups.iter().filter_map(|(id, _)| self.notes.get(*id)).map(|n| ToastData {
            title: n.title.clone().into(), sub: n.body.clone().into(), icon: n.icon.clone().into(), kind: n.kind, id: n.id as i32, time: age(now, n.time).into(),
        }).collect();
        let ui = self.ui();
        ui.set_toasts(model(toasts));
        ui.set_notif_count(self.notes.unread() as i32);
        ui.set_notif_last(self.notes.notes.first().map(|n| n.title.clone()).unwrap_or_else(|| tr!("Nothing new")).into());
        if self.overlay == Overlay::Notifications { self.push_note_rows(); }
    }

    fn push_note_rows(&mut self) {
        let now = crate::util::now_secs();
        let today = now - (now % 86400.0);
        let mut last_group = 0;
        let rows: Vec<NoteRow> = self.notes.notes.iter().map(|n| {
            let group = if n.time >= today { 1 } else { 2 };
            let header = if group != last_group { last_group = group; if group == 1 { tr!("TODAY") } else { tr!("EARLIER") } } else { String::new() };
            NoteRow { group: header.into(), title: n.title.clone().into(), body: n.body.clone().into(), age: age(now, n.time).into(),
                kind: n.kind, icon: n.icon.clone().into(), unread: !n.read, has_action: n.action != Action::None }
        }).collect();
        self.ui().set_notes(model(rows));
    }

    /// The list of notifications, `focus` (a notification's id) chosen.
    pub fn open_notifications(&mut self, focus: Option<u64>) {
        let i = focus.and_then(|id| self.notes.notes.iter().position(|n| n.id == id)).unwrap_or(0);
        audio::play(Sound::Open);
        self.popups.clear();
        self.push_overlay(Overlay::Notifications, Z_NOTES, i as i32);
        self.push_note_rows();
        // Seen: no longer counted as new.
        for n in &mut self.notes.notes { n.read = true; }
        self.notes.save();
        self.push_notes();
    }

    pub fn act_notifications(&mut self, a: Act) {
        let n = self.notes.notes.len();
        // The rows, then "Clear all" when there's any.
        let last = if n > 0 { n } else { 0 };
        let i = self.idx.max(0) as usize;
        match a {
            Act::Up if i > 0 => self.move_focus(Z_NOTES, i as i32 - 1),
            Act::Down if i < last => self.move_focus(Z_NOTES, i as i32 + 1),
            Act::Back => self.back_soft(),
            // □: this one goes.
            Act::Trailer if i < n => {
                self.notes.notes.remove(i);
                self.notes.save();
                audio::play(Sound::Back);
                let to = i.min(self.notes.notes.len()) as i32;
                self.set_focus(Z_NOTES, to);
                self.push_notes();
                self.push_note_rows();
            }
            Act::Confirm if i == n && n > 0 => {
                self.notes.notes.clear();
                self.notes.save();
                audio::play(Sound::Back);
                self.set_focus(Z_NOTES, 0);
                self.push_notes();
                self.push_note_rows();
            }
            Act::Confirm if i < n => {
                let action = self.notes.notes[i].action.clone();
                if action != Action::None { self.back(); self.run_note_action(&action); }
            }
            _ => {}
        }
    }

    /// A card in the stack clicked: where it leads.
    pub fn popup_clicked(&mut self, id: u64) {
        self.popups.retain(|(i, _)| *i != id);
        if let Some(action) = self.notes.get(id).map(|n| n.action.clone()) {
            if action == Action::None { self.open_notifications(Some(id)); } else { self.run_note_action(&action); }
        }
        self.push_notes();
    }

    /// The newest card in the stack, for PS ("press PS to see it").
    pub fn popup_showing(&self) -> Option<u64> {
        self.popups.first().map(|(id, _)| *id)
    }

    pub fn run_note_action(&mut self, action: &Action) {
        match action {
            Action::None => {}
            Action::Downloads(key) => self.open_downloads(Some(key)),
            Action::Play(folder) => {
                let found = self.locals.iter().position(|l| l.l.path.to_string_lossy() == folder.as_str()
                    || l.l.path.starts_with(folder) || std::path::Path::new(folder).starts_with(&l.l.path));
                match found {
                    Some(l) if self.session_for_local(l).is_some() => self.resume_game(None),
                    Some(l) => self.launch(l),
                    None => self.switch_view(0, true),
                }
            }
            Action::File(path) => { let _ = std::process::Command::new("xdg-open").arg(path).spawn(); }
            Action::Restart => self.power_ask(crate::os::power::Power::App),
            Action::Settings(header) => {
                self.open_settings();
                if let Some(i) = self.settings_headers.iter().position(|h| h == header) {
                    self.set_settings_root(false);
                    self.focus_settings_nav(i);
                }
            }
            Action::Library => self.switch_view(1, true),
        }
    }

    /// SwarmPlay in the background: the desktop's own notification, with Open leading back.
    fn desktop_notice(&mut self, id: u64) {
        let Some(n) = self.notes.get(id).cloned() else { return };
        let has_action = n.action != Action::None;
        std::thread::spawn(move || {
            let mut cmd = std::process::Command::new("notify-send");
            cmd.args(["-a", "SwarmPlay", "-i", crate::update::ICON_NAME, &n.title, &n.body]);
            if has_action { cmd.arg(format!("--action=open={}", tr!("Open"))); }
            let Ok(out) = cmd.output() else { return };
            if String::from_utf8_lossy(&out.stdout).trim() == "open" {
                let _ = slint::invoke_from_event_loop(move || with_app(move |app| {
                    std::thread::spawn(|| { crate::overlay::focus_main(); });
                    app.run_note_action(&n.action);
                }));
            }
        });
    }

    /// Every 10 s, from the tick: what's worth a notification on its own (the controller, room
    /// left, a download that stopped moving, the VPN).
    pub fn notify_watch(&mut self) {
        let now = crate::util::now_secs();
        if now - self.watch_at < 10.0 { return; }
        let first = self.watch_at == 0.0;
        self.watch_at = now;
        // Controllers that came and went (not the ones there at the start).
        let pads = crate::gamepad::connected();
        if !first {
            let came: Vec<String> = pads.iter().filter(|p| !self.pads_seen.contains(p)).cloned().collect();
            for p in &came {
                self.notify(Cat::System, DONE, "gamepad", &tr!("Controller connected"), p, Action::None, &format!("pad:{p}"), false);
            }
            for p in self.pads_seen.clone().iter().filter(|p| !pads.contains(p)) {
                self.notify(Cat::System, INFO, "gamepad", &tr!("Controller disconnected"), p, Action::None, &format!("pad:{p}"), false);
            }
        }
        self.pads_seen = pads;
        // A controller's battery low: once a charge.
        if let Some(b) = crate::battery::controllers().first().copied() {
            if b.percent <= 15 && !b.plugged && !self.pad_low_told {
                self.pad_low_told = true;
                self.notify(Cat::System, WARN, "gamepad", &tr!("Controller battery {}%", b.percent), &tr!("Charge it soon"), Action::None, "pad-battery", true);
            } else if b.plugged || b.percent > 20 {
                self.pad_low_told = false;
            }
        }
        // Little room where new games go: once a day.
        let day = (now / 86400.0) as i64;
        if day != self.low_space_day {
            if let Some(p) = self.storage_places().into_iter().find(|p| p.default && p.total > 0 && p.free < 20 << 30) {
                self.low_space_day = day;
                let gb = crate::i18n::num(p.free as f64 / (1u64 << 30) as f64, 0);
                self.notify(Cat::System, WARN, "disk", &tr!("Little room left for games"), &tr!("{name} · {gb} GB free", name = p.name, gb),
                    Action::Settings("STORAGE".into()), "low-space", false);
            }
        }
        // Downloads that stopped moving for 10 minutes, or wait for the VPN.
        for j in self.downloads.snapshot() {
            if j.state == crate::downloads::State::Downloading {
                let e = self.dl_moved.entry(j.key.clone()).or_insert((j.done, now, false));
                if e.0 != j.done { *e = (j.done, now, false); }
                else if now - e.1 > 600.0 && !e.2 {
                    e.2 = true;
                    self.notify(Cat::Downloads, WARN, "download", &tr!("Download stuck"), &tr!("{} · no one is sharing it right now", j.name),
                        Action::Downloads(j.key.clone()), &format!("stuck:{}", j.key), false);
                }
            } else {
                self.dl_moved.remove(&j.key);
            }
            if j.waiting_vpn && self.vpn_told.insert(j.key.clone()) {
                self.notify(Cat::Downloads, WARN, "lock", &tr!("Downloads paused"), &tr!("The VPN disconnected · they go on when it's back"),
                    Action::Downloads(j.key.clone()), "vpn", true);
            } else if !j.waiting_vpn {
                self.vpn_told.remove(&j.key);
            }
        }
    }

    /// Once a day, at the start: what's new in the catalog.
    pub fn notify_digest(&mut self) {
        let day = (crate::util::now_secs() / 86400.0) as i64;
        if self.cfg.lock().unwrap().last_digest_day == day { return; }
        let mut names: Vec<&str> = self.games.iter().filter(|g| g.is_new).map(|g| g.name.as_str()).collect();
        names.sort();
        names.dedup();
        if names.is_empty() { return; }
        let body = names.iter().take(3).copied().collect::<Vec<_>>().join(" · ");
        let n = names.len();
        { let mut c = self.cfg.lock().unwrap(); c.last_digest_day = day; c.save(); }
        self.notify(Cat::Catalog, INFO, "sparkle", &tr!("{n} new game in the Store" | "{n} new games in the Store" % n), &body, Action::Library, "digest", false);
    }

    /// The stack's cards go when their time is up (from the tick).
    pub fn notes_tick(&mut self) {
        if !self.popups.is_empty() { self.push_notes(); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(cat: Cat, key: &str, important: bool) -> Note {
        Note { id: 0, cat, kind: INFO, icon: String::new(), title: "t".into(), body: String::new(), action: Action::None, key: key.into(), time: 0.0, read: false, important }
    }

    #[test]
    fn keeps_newest_first_replaces_by_key_and_forgets_old_ones() {
        let mut s = Store::default();
        let a = s.add(note(Cat::Downloads, "dl:1", false), 1000.0);
        s.add(note(Cat::Downloads, "dl:2", false), 1001.0);
        let c = s.add(note(Cat::Downloads, "dl:1", false), 1002.0);
        assert_eq!(s.notes.len(), 2, "the same download replaces its notification");
        assert_eq!(s.notes[0].id, c);
        assert!(s.get(a).is_none());
        assert_eq!(s.unread(), 2);
        s.add(note(Cat::System, "", false), 1002.0 + KEEP_SECS + 1.0);
        assert_eq!(s.notes.len(), 1, "a week later only the new one is left");
        for i in 0..150 { s.add(note(Cat::System, "", false), 2e6 + i as f64); }
        assert_eq!(s.notes.len(), KEEP_MAX);
    }

    #[test]
    fn the_rules_decide_where_it_shows() {
        let r = Rules { popups: true, games: 1, desktop: true, dnd_until: 0.0, off: vec![Cat::Catalog] };
        assert_eq!(route(&r, &note(Cat::Downloads, "", false), 10.0, false, false), Show::Popup);
        assert_eq!(route(&r, &note(Cat::Downloads, "", false), 10.0, false, true), Show::Desktop, "in the background");
        assert_eq!(route(&r, &note(Cat::Downloads, "", false), 10.0, true, false), Show::ListOnly, "a game: important only");
        assert_eq!(route(&r, &note(Cat::Installs, "", true), 10.0, true, false), Show::Overlay);
        assert_eq!(route(&r, &note(Cat::Catalog, "", false), 10.0, false, false), Show::ListOnly, "kind turned off");
        let dnd = Rules { dnd_until: 100.0, ..r };
        assert_eq!(route(&dnd, &note(Cat::Downloads, "", false), 10.0, false, false), Show::ListOnly, "do not disturb");
        assert_eq!(route(&dnd, &note(Cat::Installs, "", true), 10.0, false, false), Show::Popup, "important gets through");
    }

    #[test]
    fn ages_read_like_a_console() {
        assert_eq!(age(100.0, 90.0), "now");
        assert_eq!(age(1000.0, 400.0), "10 min");
        assert_eq!(age(10000.0, 2000.0), "2 h");
        assert_eq!(age(200000.0, 100000.0), "yesterday");
        assert_eq!(age(400000.0, 100000.0), "3 days");
    }

    #[test]
    fn every_choice_has_a_label() {
        // Every key has its own label (none falls through to the raw key, which would stay
        // English when translated); in English a label is its key.
        for (_, name) in CATS { assert_eq!(cat_text(name).as_deref(), Some(name)); }
        for k in DURING_GAMES { assert_eq!(during_games_text(k).as_deref(), Some(k)); }
        for k in DND { assert_eq!(dnd_text(k).as_deref(), Some(k)); }
        for s in DURATIONS { assert_eq!(duration_label(s), format!("{s} seconds")); }
    }
}
