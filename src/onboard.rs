//! The first-run setup, as a console's: full screen, one question a page, the controller all it
//! takes. A new install goes through it before the start screen (the catalog and emulators come
//! in meanwhile); Settings → Launcher → Set up again opens it any time.
//!
//! Each page shows Settings' own rows (the language, Wi-Fi, the theme, sound, folders…) and
//! changes them the way Settings does, so the two never disagree and nothing is set twice. What
//! a page opens (the folder picker, a Wi-Fi password, the name's keyboard) comes back to it.

use crate::app::*;
use crate::audio::{self, Sound};
use crate::i18n::Lang;
use crate::settings::SId;
use crate::{ActionData, OnboardData, SettingData};
use slint::{Model, VecModel};
use std::rc::Rc;
use std::time::Duration;
use tr::tr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// "Press any button": the controller (or keyboard) to be used.
    Press = 0,
    Language = 1,
    Controller = 2,
    Internet = 3,
    /// Who's playing?: the name and the avatar's colour.
    Profile = 4,
    Look = 5,
    Sound = 6,
    Storage = 7,
    Playing = 8,
    Notify = 9,
    Done = 10,
}

/// The pages a setup goes through. A controller already there needs no Controller page, nor
/// does a desktop that's online need Internet; AzrakOS always has both (it pairs and joins
/// Wi-Fi itself). Opened from Settings, there's nothing to press first.
pub fn steps(on_os: bool, online: bool, has_pad: bool, first_run: bool) -> Vec<Step> {
    use Step::*;
    let mut v = if first_run { vec![Press, Language] } else { vec![Language] };
    if on_os || !has_pad {
        v.push(Controller);
    }
    if on_os || !online {
        v.push(Internet);
    }
    v.extend([Profile, Look, Sound, Storage, Playing, Notify, Done]);
    v
}

/// SWARMPLAY_ONBOARD=all: every page, whatever is connected (screenshots).
fn every_page() -> bool {
    std::env::var_os("SWARMPLAY_ONBOARD").is_some_and(|v| v == "all")
}

/// The pages for what's there now.
fn pages(online: bool, has_pad: bool, first_run: bool) -> Vec<Step> {
    if every_page() {
        return steps(true, false, false, first_run);
    }
    steps(crate::os::system::on_os(), online, has_pad, first_run)
}

/// Settings' rows a page shows.
fn wants(step: Step, id: SId) -> bool {
    use SId::*;
    match step {
        Step::Controller => matches!(id, BtPair | BtFound(_) | BtDevice(_)),
        Step::Internet => matches!(id, Internet | WifiNet(_) | WifiMore),
        Step::Profile => matches!(id, Name | AvatarColor),
        Step::Look => matches!(id, ThemeWallpaper | ThemeAccent),
        Step::Sound => matches!(id, SoundVolume | SoundOut | SoundOutput | Sounds),
        Step::Storage => matches!(id, InstallDir | DownloadDir),
        Step::Playing => matches!(id, Fullscreen | WineSdlPads | OverlayHelper),
        Step::Notify => matches!(id, NotifPopups | NotifGames | AutoUpdate),
        _ => false,
    }
}

/// A line on the page.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Line {
    /// One of Settings' rows.
    Setting(SId),
    /// A language to choose.
    Lang(Lang),
    /// Something to read (a controller found, the summary at the end): never selected.
    Info,
}

/// A button under the lines.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Button {
    Next,
    Finish,
}

pub struct Onboard {
    steps: Vec<Step>,
    at: usize,
    lines: Vec<Line>,
    buttons: Vec<Button>,
    /// Opened at startup (the start screen comes after; Some(first run)) or from Settings (None).
    first_run: Option<bool>,
    online: bool,
    pads: Vec<String>,
    /// Bumped on every page change: the window builds the new page (and its entrance) afresh.
    page: i32,
    /// Which way the last page change went (1 forward, -1 back): the page slides in from there.
    dir: i32,
    /// Controllers and the network, read every couple of seconds while it's open.
    _watch: slint::Timer,
    /// The page's rows and buttons, changed in place: a row that changes isn't made again (its
    /// entrance would play again).
    rows: Rc<VecModel<SettingData>>,
    actions: Rc<VecModel<ActionData>>,
}

/// `model` holds `items` after this, changing only the rows that differ when the count is the same.
fn update<T: Clone + PartialEq + 'static>(model: &VecModel<T>, items: Vec<T>) {
    if model.row_count() != items.len() {
        return model.set_vec(items);
    }
    for (i, item) in items.into_iter().enumerate() {
        if model.row_data(i).as_ref() != Some(&item) {
            model.set_row_data(i, item);
        }
    }
}

impl Onboard {
    fn step(&self) -> Step {
        self.steps[self.at]
    }

    /// Where the highlight can go: the lines that can be chosen, then the buttons.
    fn stops(&self) -> Vec<usize> {
        let lines = self.lines.iter().enumerate().filter(|(_, l)| **l != Line::Info).map(|(i, _)| i);
        lines.chain((0..self.buttons.len()).map(|b| self.lines.len() + b)).collect()
    }
}

impl App {
    /// Open the setup. `first_run`: at startup (Some), from Settings (None).
    pub fn onboard_open(&mut self, first_run: Option<bool>) {
        let pads = crate::gamepad::connected();
        let online = !self.net_down;
        let watch = slint::Timer::default();
        watch.start(slint::TimerMode::Repeated, Duration::from_secs(2), || with_app(|app| app.onboard_watch()));
        self.ob = Some(Onboard {
            steps: pages(online, !pads.is_empty(), first_run.is_some()),
            at: 0,
            lines: Vec::new(),
            buttons: Vec::new(),
            first_run,
            online,
            pads,
            page: 1,
            dir: 1,
            _watch: watch,
            rows: Rc::new(VecModel::default()),
            actions: Rc::new(VecModel::default()),
        });
        // The rows' state as it is now (as opening Settings does).
        self.sys_poll();
        self.sound_polled = 0.0;
        self.sound_poll();
        self.ov_check_helper();
        self.build_settings();
        self.push_overlay(Overlay::Onboard, Z_ONBOARD, 0);
        self.ui().set_onboard_open(true);
        if first_run.is_none() {
            audio::play(Sound::Open);
        }
        self.onboard_push();
        self.onboard_focus_first();
        // Online or not, for the Internet page: asked of the network itself, not guessed.
        std::thread::spawn(|| {
            let up = crate::util::network_up();
            let _ = slint::invoke_from_event_loop(move || with_app(move |app| app.onboard_online(up)));
        });
    }

    fn onboard_online(&mut self, up: bool) {
        let Some(ob) = self.ob.as_mut() else { return };
        if ob.online != up {
            ob.online = up;
            self.onboard_restep();
        }
    }

    /// Every 2 s while it's open: a controller plugged in (or paired) shows, and pages come and go
    /// with it (never the one on screen).
    fn onboard_watch(&mut self) {
        let Some(ob) = self.ob.as_mut() else { return };
        let pads = crate::gamepad::connected();
        if pads == ob.pads {
            return;
        }
        let more = pads.len() > ob.pads.len();
        ob.pads = pads;
        if more && self.overlay == Overlay::Onboard {
            audio::play(Sound::Notify);
        }
        self.onboard_restep();
    }

    /// The pages again (a controller or the network changed), staying on the one shown.
    fn onboard_restep(&mut self) {
        let Some(ob) = self.ob.as_mut() else { return };
        let now = ob.step();
        let next = pages(ob.online, !ob.pads.is_empty(), ob.first_run.is_some());
        if let Some(at) = next.iter().position(|s| *s == now) {
            ob.steps = next;
            ob.at = at;
        }
        if self.in_setup() {
            self.onboard_push();
        }
    }

    /// Show the page as it is now (also after its rows changed behind it).
    pub fn onboard_push(&mut self) {
        let Some(ob) = self.ob.as_ref() else { return };
        let step = ob.step();
        let name = self.player_name();
        let mut rows: Vec<SettingData> = Vec::new();
        let mut lines: Vec<Line> = Vec::new();
        let info = |label: String, value: String, icon: &str| SettingData {
            kind: 7, label: label.into(), value: value.into(), icon: icon.into(), value_kind: 1, ..Default::default()
        };
        match step {
            Step::Language => {
                for l in Lang::ALL {
                    let on = l == crate::i18n::current();
                    rows.push(SettingData { kind: 4, label: l.name().into(), icon: if on { "check".into() } else { "".into() }, value_kind: 1, on, ..Default::default() });
                    lines.push(Line::Lang(l));
                }
            }
            Step::Controller if !crate::os::system::on_os() => {
                for p in &ob.pads {
                    rows.push(info(p.clone(), tr!("Connected"), "check"));
                    lines.push(Line::Info);
                }
            }
            Step::Internet if !crate::os::system::on_os() => {
                let (value, kind, icon) = if ob.online { (tr!("Connected"), 1, "check") } else { (tr!("Not connected"), 2, "alert") };
                rows.push(SettingData { value_kind: kind, ..info(tr!("Internet"), value, icon) });
                lines.push(Line::Info);
            }
            Step::Done => {
                let cfg = self.cfg.lock().unwrap().clone();
                for (label, value, icon) in [
                    (tr!("Language"), crate::i18n::current().name().to_string(), "web"),
                    (tr!("Your name"), name.clone(), "user"),
                    (tr!("Wallpaper"), crate::theme::wallpaper_label(&cfg.theme_wallpaper), "palette"),
                    (tr!("Accent colour"), crate::theme::accent_label(&cfg.theme_accent), "palette"),
                    (tr!("New games go to"), crate::util::display_path(&cfg.install_dir), "folder"),
                ] {
                    rows.push(SettingData { value_kind: 0, ..info(label, value, icon) });
                    lines.push(Line::Info);
                }
            }
            _ => {
                for (id, r) in self.settings_ids.iter().zip(&self.settings_rows) {
                    if wants(step, *id) {
                        rows.push(r.clone());
                        lines.push(Line::Setting(*id));
                    }
                }
            }
        }
        let (title, sub) = match step {
            Step::Press => (tr!("Welcome to SwarmPlay"), tr!("Connect your controller and press any button")),
            Step::Language => (tr!("Choose your language"), tr!("You can change it any time in Settings")),
            Step::Controller if crate::os::system::on_os() => (tr!("Connect a controller"),
                tr!("Plug it in with a cable, or hold its PS and Share buttons until the light flashes and pair it here")),
            Step::Controller => (tr!("Connect a controller"),
                tr!("Plug it in with a cable, or pair it in your system's Bluetooth settings. A keyboard works too.")),
            Step::Internet if crate::os::system::on_os() => (tr!("Connect to the internet"), tr!("Games, their pictures and updates come from the internet")),
            Step::Internet => (tr!("Connect to the internet"),
                tr!("Games, their pictures and updates come from the internet · connect in your system's settings, SwarmPlay notices")),
            Step::Profile => (tr!("Who's playing?"), tr!("Your name and colour, on your avatar and the start screen")),
            Step::Look => (tr!("Choose your look"), tr!("A wallpaper and an accent colour · they show right away")),
            Step::Sound => (tr!("Sound"), tr!("How loud, where it plays, and the interface's own sounds")),
            Step::Storage => (tr!("Where games go"), tr!("New games and downloads go here · more places can be added in Settings → Storage")),
            Step::Playing => (tr!("Playing"), tr!("How games open, and controllers in Windows games")),
            Step::Notify => (tr!("Notifications and updates"), tr!("What tells you something, and keeping everything up to date")),
            Step::Done => (tr!("All set, {}", name), tr!("Here's what you chose · everything can be changed in Settings")),
        };
        let buttons: Vec<Button> = match step {
            Step::Press => Vec::new(),
            Step::Done => vec![Button::Finish],
            _ => vec![Button::Next],
        };
        let actions: Vec<ActionData> = buttons.iter().map(|b| {
            let label = match b {
                // Nothing connected yet: going on is skipping it.
                Button::Next if step == Step::Controller && ob.pads.is_empty() => tr!("Skip"),
                Button::Next if step == Step::Internet && !ob.online => tr!("Skip"),
                Button::Next => tr!("Setup" => "Next"),
                Button::Finish if ob.first_run.is_some() => tr!("Start playing"),
                Button::Finish => tr!("Done"),
            };
            ActionData { id: "next".into(), label: label.into(), icon: "".into(), primary: true, danger: false, round: false }
        }).collect();
        // The progress line counts the questions: not "press a button".
        let counted: Vec<&Step> = ob.steps.iter().filter(|s| **s != Step::Press).collect();
        let index = counted.iter().position(|s| **s == step).map_or(0, |p| p as i32 + 1);
        let data = OnboardData {
            step: step as i32,
            index,
            count: counted.len() as i32,
            title: title.into(),
            subtitle: sub.into(),
            rows: ob.rows.clone().into(),
            buttons: ob.actions.clone().into(),
            from_settings: ob.first_run.is_none(),
        };
        let (page, dir) = (ob.page, ob.dir);
        update(&ob.rows, rows);
        update(&ob.actions, actions);
        if let Some(ob) = self.ob.as_mut() {
            ob.lines = lines;
            ob.buttons = buttons;
        }
        let ui = self.ui();
        ui.set_onboard(data);
        ui.set_onboard_dir(dir);
        // A new page: a new model, so the window makes the page again and plays its entrance.
        if ui.get_onboard_page().row_count() != 1 || ui.get_onboard_page().row_data(0) != Some(page) {
            ui.set_onboard_page(model(vec![page]));
        }
        // Rows can go (a network out of reach): the highlight stays on something there.
        if self.overlay == Overlay::Onboard && self.zone == Z_ONBOARD {
            let stops = self.ob.as_ref().map(|o| o.stops()).unwrap_or_default();
            if !stops.is_empty() && !stops.contains(&(self.idx.max(0) as usize)) {
                let last = *stops.last().unwrap_or(&0);
                self.set_focus(Z_ONBOARD, last as i32);
            }
        }
    }

    /// A page's first highlight: the language in use, else the first row, else its button.
    fn onboard_focus_first(&mut self) {
        let Some(ob) = self.ob.as_ref() else { return };
        let stops = ob.stops();
        let current = ob.lines.iter().position(|l| *l == Line::Lang(crate::i18n::current()));
        let first = current.or_else(|| stops.first().copied()).unwrap_or(0);
        self.set_focus(Z_ONBOARD, first as i32);
    }

    /// To the next page (or the previous: -1).
    fn onboard_go(&mut self, dir: i32) {
        let Some(ob) = self.ob.as_mut() else { return };
        let at = ob.at as i32 + dir;
        if at < 0 || at >= ob.steps.len() as i32 {
            return;
        }
        // Back to "press a button" is no question to go back to.
        if dir < 0 && ob.steps[at as usize] == Step::Press {
            return;
        }
        ob.at = at as usize;
        ob.dir = dir;
        ob.page += 1;
        let from = ob.steps[(at - dir) as usize];
        audio::play(if dir < 0 { Sound::Back } else if from == Step::Press { Sound::Start } else { Sound::Select });
        // Each page with its rows as they are now.
        self.build_settings();
        self.onboard_push();
        self.onboard_focus_first();
    }

    pub fn act_onboard(&mut self, a: Act) {
        let Some(ob) = self.ob.as_ref() else { return self.back() };
        let step = ob.step();
        // The first page: any button (or key) goes on.
        if step == Step::Press {
            if matches!(a, Act::Confirm | Act::Up | Act::Down | Act::Left | Act::Right | Act::Options | Act::Search | Act::Trailer) {
                self.onboard_go(1);
            }
            return;
        }
        let stops = ob.stops();
        let here = self.idx.max(0) as usize;
        let pos = stops.iter().position(|s| *s == here);
        let line = ob.lines.get(here).copied();
        match a {
            Act::Up | Act::Down => {
                let next = match (pos, a) {
                    (Some(p), Act::Up) => p.checked_sub(1),
                    (Some(p), _) => Some(p + 1),
                    (None, _) => Some(0),
                };
                if let Some(&s) = next.and_then(|p| stops.get(p)) {
                    self.move_focus(Z_ONBOARD, s as i32);
                }
            }
            // ◂ ▸ step a choice, as in Settings.
            Act::Left | Act::Right => {
                if let Some(Line::Setting(id)) = line {
                    let kind = self.settings_ids.iter().position(|s| *s == id).and_then(|i| self.settings_rows.get(i)).map_or(0, |r| r.kind);
                    if kind == 3 {
                        self.onboard_change(id, if a == Act::Left { -1 } else { 1 });
                    }
                }
            }
            Act::Confirm => self.onboard_press(here),
            // R2 / Options: the next page from anywhere on it.
            Act::PageDown | Act::Options => self.onboard_next(),
            Act::Back => {
                if ob.at == 0 || (ob.at == 1 && ob.steps[0] == Step::Press) {
                    // From Settings, Back on its first page leaves; at startup there's nothing behind.
                    if ob.first_run.is_none() {
                        self.onboard_close();
                    }
                } else {
                    self.onboard_go(-1);
                }
            }
            _ => {}
        }
    }

    fn onboard_next(&mut self) {
        if self.ob.as_ref().is_some_and(|o| o.step() == Step::Done) {
            self.onboard_finish();
        } else {
            self.onboard_go(1);
        }
    }

    /// ✕ (or a click) on line or button `i`.
    fn onboard_press(&mut self, i: usize) {
        let Some(ob) = self.ob.as_ref() else { return };
        if i >= ob.lines.len() {
            return self.onboard_next();
        }
        match ob.lines[i] {
            Line::Lang(l) => {
                if l != crate::i18n::current() {
                    self.cfg.lock().unwrap().language = l.code().into();
                    self.cfg.lock().unwrap().save();
                    crate::i18n::set_lang(l);
                    // Everything already in the window, in the new language (this page too).
                    self.relocalize();
                    audio::play(Sound::Select);
                    self.onboard_push();
                } else {
                    // The one in use, chosen again: on to the next question.
                    self.onboard_go(1);
                }
            }
            Line::Setting(id) => {
                if let Some(k) = self.settings_ids.iter().position(|s| *s == id) {
                    self.settings_activate(k);
                    self.onboard_push();
                }
            }
            Line::Info => {}
        }
    }

    fn onboard_change(&mut self, id: SId, dir: i32) {
        if let Some(k) = self.settings_ids.iter().position(|s| *s == id) {
            self.settings_change(k, dir);
            self.onboard_push();
        }
    }

    pub fn click_onboard(&mut self, idx: i32) {
        if self.ob.as_ref().is_some_and(|o| o.step() == Step::Press) {
            return self.onboard_go(1);
        }
        if idx < 0 {
            return;
        }
        if self.idx != idx {
            let stops = self.ob.as_ref().map(|o| o.stops()).unwrap_or_default();
            if !stops.contains(&(idx as usize)) {
                return;
            }
            self.set_focus(Z_ONBOARD, idx);
        }
        self.onboard_press(idx as usize);
    }

    pub fn hover_onboard(&mut self, idx: i32) {
        let stops = self.ob.as_ref().map(|o| o.stops()).unwrap_or_default();
        if idx >= 0 && idx != self.idx && stops.contains(&(idx as usize)) {
            self.move_focus(Z_ONBOARD, idx);
        }
    }

    /// "Start playing": set up for good, then the start screen (at startup) or Settings again.
    fn onboard_finish(&mut self) {
        let Some(ob) = self.ob.take() else { return };
        {
            let mut c = self.cfg.lock().unwrap();
            c.setup_done = true;
            c.save();
        }
        audio::play(Sound::Welcome);
        self.onboard_pop();
        match ob.first_run {
            Some(first) => self.boot_start(first),
            None => self.refresh_settings(),
        }
    }

    /// Left from Settings before the end: what was changed stays changed.
    fn onboard_close(&mut self) {
        self.ob = None;
        audio::play(Sound::Back);
        self.onboard_pop();
        self.refresh_settings();
    }

    /// Off the screen, back to what was under it.
    fn onboard_pop(&mut self) {
        self.ui().set_onboard_open(false);
        let (ov, zone, idx) = self.stack.pop().unwrap_or((Overlay::None, Z_ROW, 0));
        self.overlay = ov;
        self.ui().set_overlay(ov as i32);
        self.set_focus(zone, idx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Step::*;

    #[test]
    fn a_desktop_online_with_a_controller_asks_neither_for_one() {
        let s = steps(false, true, true, true);
        assert_eq!(s[..3], [Press, Language, Profile]);
        assert_eq!(s.last(), Some(&Done));
    }

    #[test]
    fn azrakos_offline_asks_for_both() {
        let s = steps(true, false, false, true);
        assert_eq!(s[..4], [Press, Language, Controller, Internet]);
        // The same on AzrakOS with them: it pairs and joins Wi-Fi itself.
        assert_eq!(steps(true, true, true, true)[..4], [Press, Language, Controller, Internet]);
        // A desktop without them asks for each.
        assert_eq!(steps(false, false, false, true)[..4], [Press, Language, Controller, Internet]);
    }

    #[test]
    fn from_settings_there_is_nothing_to_press_first() {
        assert_eq!(steps(false, true, true, false)[0], Language);
    }

    #[test]
    fn each_page_shows_its_own_rows() {
        assert!(wants(Step::Look, SId::ThemeAccent));
        assert!(!wants(Step::Look, SId::ThemeImage), "a path to type has no place in the setup");
        assert!(wants(Step::Internet, SId::WifiNet(3)));
        assert!(!wants(Step::Sound, SId::SoundMute));
        assert!(wants(Step::Profile, SId::Name));
    }
}
