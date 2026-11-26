//! Restart and shut down (Settings → SYSTEM, and its "Restart to update" row), and restarting
//! SwarmPlay itself into a new version, asked first: a
//! controller's ✕ is pressed by habit, and turning the TV's PC off by mistake isn't something
//! to undo with ○. The card opens on Cancel; once confirmed it shows what's happening and takes
//! no more input until the PC goes.

use crate::app::*;
use crate::audio::{self, Sound};
use crate::os::system;
use crate::settings::SId;
use tr::tr;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Power {
    Restart,
    ShutDown,
    /// Restart into the downloaded system update.
    Update,
    /// Not the PC's: delete installed games (src/games.rs, what `games_view.doomed` holds). The
    /// same card asks, on Cancel first.
    Delete,
    /// Not the PC's either: SwarmPlay closes and opens again (into the update it installed). The
    /// card says what stops meanwhile.
    App,
    /// Install a repack (src/repack_ui.rs): a button per drive (`power_choices`), then Cancel. It
    /// opens on the first drive: nothing is lost by pressing it.
    Repack,
}

/// The Settings row that asks for it.
fn power_row(what: Power) -> SId {
    match what {
        Power::Restart => SId::Restart,
        Power::ShutDown => SId::ShutDown,
        Power::Update => SId::SystemUpdate,
        Power::Delete => SId::Rescan,
        Power::App => SId::AppUpdate,
        Power::Repack => SId::Rescan,
    }
}

#[derive(Default)]
pub struct PowerAsk {
    pub what: Option<Power>,
    /// The version the update brings (Power::Update, Power::App).
    pub next: String,
    /// 0: restart / shut down, 1: Cancel.
    pub sel: usize,
    /// Confirmed: waiting for the PC to restart or shut down.
    pub busy: bool,
}

/// Where Cancel is: last, after the one button (two buttons), or after every drive (a repack).
fn cancel_at(app: &App) -> usize {
    if app.power.what == Some(Power::Repack) { app.power_choices.len() } else { 1 }
}

impl App {
    /// Ask before restarting or shutting down. A running game is closed first, so its progress
    /// is saved: the card doesn't open while one runs.
    pub fn power_ask(&mut self, what: Power) {
        if !matches!(what, Power::Delete | Power::Repack) && !self.live.is_empty() {
            return self.note_setting(power_row(what), &tr!("A game is running: close it first, so its progress is saved"), 2);
        }
        audio::play(Sound::Select);
        let next = if what == Power::App { self.upd.installed.clone() } else { self.sys.update.as_ref().and_then(|u| u.staged.clone()) }.unwrap_or_default();
        self.power = PowerAsk { what: Some(what), next, sel: 0, busy: false };
        self.power.sel = if what == Power::Repack { 0 } else { cancel_at(self) };
        self.push_overlay(Overlay::Power, Z_POWER, 0);
        self.power_push();
    }

    pub fn act_power(&mut self, a: Act) {
        if self.power.busy {
            return;
        }
        match a {
            Act::Left | Act::Up if self.power.sel > 0 => self.power.sel -= 1,
            Act::Right | Act::Down if self.power.sel < cancel_at(self) => self.power.sel += 1,
            Act::Confirm => return self.power_choose(self.power.sel),
            Act::Back => return self.back(),
            _ => return,
        }
        audio::play(Sound::Move);
        self.power_push();
    }

    /// A button clicked with the mouse (negative: the backdrop, which cancels).
    pub fn click_power(&mut self, idx: i32) {
        if self.power.busy {
            return;
        }
        if idx < 0 {
            return self.back();
        }
        self.power.sel = (idx as usize).min(cancel_at(self));
        self.power_choose(self.power.sel);
    }

    fn power_choose(&mut self, sel: usize) {
        let Some(what) = self.power.what else { return self.back() };
        if sel == cancel_at(self) {
            return self.back();
        }
        if what == Power::Repack {
            let base = self.power_choices.get(sel).map(|(p, _)| p.clone());
            self.back();
            if let Some(base) = base { self.repack_start(base); }
            return;
        }
        if what == Power::Delete {
            self.back();
            return self.delete_games();
        }
        audio::play(Sound::Select);
        self.power.busy = true;
        self.power_push();
        if what == Power::App {
            // A moment for "Restarting…" to show: stopping the downloads holds the window up.
            return slint::Timer::single_shot(std::time::Duration::from_millis(120), || with_app(|app| {
                app.app_restart();
                // Still here: it couldn't start the new one (app_restart said why).
                app.power.busy = false;
                if app.overlay == Overlay::Power { app.back(); }
            }));
        }
        let restart = what != Power::ShutDown;
        std::thread::spawn(move || {
            let result = if restart { system::restart() } else { system::shut_down() };
            let _ = slint::invoke_from_event_loop(move || with_app(move |app| match result {
                Err(e) => app.power_failed(restart, &e),
                // The session ends within moments. If the PC is still here long after, something
                // held it up: give the screen back rather than leave it waiting forever.
                Ok(()) => slint::Timer::single_shot(std::time::Duration::from_secs(30), move || with_app(move |app| {
                    if app.power.busy {
                        app.power_failed(restart, &tr!("It didn't happen. Try again in a moment."));
                    }
                })),
            }));
        });
    }

    fn power_failed(&mut self, restart: bool, why: &str) {
        self.power.busy = false;
        if self.overlay == Overlay::Power {
            self.back();
        }
        let what = if restart { tr!("Couldn't restart") } else { tr!("Couldn't shut down") };
        // On the row that was pressed (Restart to update is a System row).
        let id = self.power.what.map(power_row).unwrap_or(if restart { SId::Restart } else { SId::ShutDown });
        self.note_setting(id, &format!("{what}: {why}"), 2);
    }

    /// What restarting SwarmPlay does to what's going on: downloads pause and go on by themselves
    /// after (update.rs passes them to the new one), an installation stops and has to begin again.
    fn app_restart_words(&self) -> String {
        let downloads = self.downloads.snapshot().iter().filter(|j| j.state.active()).count();
        let installing = self.installer.snapshot().iter().any(|r| r.state.active());
        restart_words(&self.power.next, downloads, installing)
    }

    fn power_push(&self) {
        let ui = self.ui();
        let p = &self.power;
        let delete = (p.what == Some(Power::Delete)).then(|| self.delete_words());
        let (title, text, yes) = match (p.what, p.busy) {
            (Some(Power::Delete), _) => { let (t, x) = delete.unwrap_or_default(); (t, x, tr!("Delete")) }
            (Some(Power::Repack), _) => { let (t, x) = self.repack_card_words(); (t, x, String::new()) }
            (Some(Power::App), false) => (tr!("Restart SwarmPlay?"), self.app_restart_words(), tr!("Restart")),
            (Some(Power::App), true) => (tr!("Restarting SwarmPlay…"), tr!("It's back in a moment."), String::new()),
            (Some(Power::ShutDown), false) => (tr!("Shut down now?"), tr!("The PC turns off. Its power button turns it on again."), tr!("Shut down")),
            (Some(Power::ShutDown), true) => (tr!("Shutting down…"), tr!("See you soon."), String::new()),
            (Some(Power::Update), false) => (tr!("Restart and update?"),
                tr!("AzrakOS {} is downloaded. The PC restarts into it and comes back here.", p.next), tr!("Restart and update")),
            (_, false) => (tr!("Restart now?"), tr!("The PC restarts and comes back here."), tr!("Restart")),
            (_, true) => (tr!("Restarting…"), tr!("The PC comes back here in a moment."), String::new()),
        };
        ui.set_power_title(title.into());
        ui.set_power_text(text.into());
        let buttons: Vec<slint::SharedString> = if p.what == Some(Power::Repack) {
            self.power_choices.iter().map(|(_, words)| words.into()).chain(std::iter::once(tr!("Cancel").into())).collect()
        } else { vec![yes.into(), tr!("Cancel").into()] };
        ui.set_power_buttons(std::rc::Rc::new(slint::VecModel::from(buttons)).into());
        ui.set_power_stack(p.what == Some(Power::Repack));
        let installing = p.what == Some(Power::App) && self.installer.snapshot().iter().any(|r| r.state.active());
        ui.set_power_danger(matches!(p.what, Some(Power::ShutDown | Power::Delete)) || installing);
        ui.set_power_icon(match p.what { Some(Power::Delete) => "trash", Some(Power::App) => "retry", Some(Power::Repack) => "download", _ => "" }.into());
        ui.set_power_sel(p.sel as i32);
        ui.set_power_busy(p.busy);
    }
}

fn restart_words(version: &str, downloads: usize, installing: bool) -> String {
    let mut words = vec![if version.is_empty() { tr!("SwarmPlay closes and opens again in a moment.") }
        else { tr!("SwarmPlay {} is installed: it closes and opens again in the new version.", version) }];
    if downloads > 0 {
        words.push(tr!("{n} download pauses and picks up again by itself after." | "{n} downloads pause and pick up again by themselves after." % downloads));
    }
    if installing { words.push(tr!("The game being installed stops and has to install again from the start.")); }
    words.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restarting_says_what_stops() {
        assert!(restart_words("1.35.0", 0, false).contains("1.35.0"));
        assert!(!restart_words("", 0, false).contains("download"));
        assert!(restart_words("1.35.0", 2, false).contains("2 downloads pause"));
        assert!(restart_words("1.35.0", 1, true).contains("1 download pauses") && restart_words("", 0, true).contains("install again"));
    }
}
