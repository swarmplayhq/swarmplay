//! Control Center: a panel that rises from the bottom over whatever is on screen, opened with the
//! controller's Guide button (or F1). It gathers what used to need a trip to a corner: the game
//! that's running (back to it, or close it), the downloads, and quick ways to the Library,
//! Settings, the controls and the power button.
//!
//! Two rows of items. The top row: the running game's two buttons (when one runs), then the
//! downloads card. The bottom row: Library, Settings, Controls, Power. `items()` is their order,
//! which ui/app.slint mirrors from `running.active`.

use crate::app::*;
use crate::audio::{self, Sound};

/// Overlays the Control Center doesn't open over: something is starting, or a question is waiting.
const BUSY: [Overlay; 6] = [Overlay::Launch, Overlay::OsInstall, Overlay::WifiJoin, Overlay::Power, Overlay::Setup, Overlay::SearchKeys];

impl App {
    /// What the panel holds, in focus order; the first `top` are the top row.
    fn cc_items(&self) -> (Vec<&'static str>, usize) {
        let mut items = Vec::new();
        if !self.live.is_empty() {
            items.extend(["resume", "stop"]);
        }
        items.push("downloads");
        items.push("notifications");
        let top = items.len();
        items.extend(["library", "settings", "controls", "power"]);
        (items, top)
    }

    /// Guide (or F1): open the panel, or close it when it's open.
    pub fn toggle_control_center(&mut self) {
        if self.overlay == Overlay::ControlCenter {
            return self.back_soft();
        }
        if self.boot.active || BUSY.contains(&self.overlay) || self.in_setup() {
            return;
        }
        // It opens over the screens underneath, not over another panel: those close first.
        for _ in 0..8 {
            if self.overlay == Overlay::None { break; }
            self.back();
        }
        // A running game's "Back to game" comes first: Guide, then Enter, returns to it.
        self.push_overlay(Overlay::ControlCenter, Z_CONTROL_CENTER, 0);
        audio::play(Sound::Open);
    }

    pub fn act_control_center(&mut self, a: Act) {
        let (items, top) = self.cc_items();
        let i = (self.idx.max(0) as usize).min(items.len() - 1);
        if let Some(to) = step(items.len(), top, i, a) {
            return self.move_focus(Z_CONTROL_CENTER, to as i32);
        }
        match a {
            Act::Back => self.back_soft(),
            Act::Confirm => {
                let item = items[i];
                // The panel slides away first, then what's chosen opens over the screen underneath.
                audio::play(Sound::Select);
                self.ui().set_closing(Overlay::ControlCenter as i32);
                slint::Timer::single_shot(std::time::Duration::from_millis(170), move || with_app(move |app| {
                    if app.overlay == Overlay::ControlCenter { app.back(); }
                    app.ui().set_closing(0);
                    match item {
                        "resume" => app.resume_game(None),
                        "stop" => app.stop_game(None),
                        "downloads" => { let key = app.ring_key.clone(); app.open_downloads(key.as_deref()) }
                        "notifications" => app.open_notifications(None),
                        "library" => app.switch_view(1, true),
                        "settings" => app.open_settings(),
                        "controls" => app.open_controls(),
                        _ => app.open_power_menu(),
                    }
                }));
            }
            _ => {}
        }
    }
}

impl App {
    /// Power, as a console's: a small card of choices by the corner, not straight off.
    pub fn open_power_menu(&mut self) {
        let mk = |id: &'static str, label: &str| ActionDef { id, label: label.into(), icon: "power", primary: false, danger: false, round: false };
        self.menu_actions = if crate::os::system::on_os() {
            vec![mk("power_off", &tr::tr!("Turn off")), mk("power_restart", &tr::tr!("Restart")), mk("app_restart", &tr::tr!("Restart SwarmPlay"))]
        } else {
            vec![mk("app_restart", &tr::tr!("Restart SwarmPlay")), mk("quit", &tr::tr!("Quit SwarmPlay"))]
        };
        self.menu_target = None;
        audio::play(Sound::Select);
        self.ui().set_menu_anchor(2);
        self.push_overlay(Overlay::Menu, Z_MENU, 0);
        self.push_menu(&tr::tr!("Power"));
    }

    /// A choice on the Power card.
    pub fn power_choice(&mut self, id: &str) {
        match id {
            "power_off" => self.power_ask(crate::os::power::Power::ShutDown),
            "power_restart" => self.power_ask(crate::os::power::Power::Restart),
            "app_restart" => self.power_ask(crate::os::power::Power::App),
            _ => { crate::app::quit("Power → Quit SwarmPlay"); }
        }
    }
}

/// Where focus goes among `len` items whose first `top` are the top row: across a row, and up
/// and down between the rows. None: it stays.
pub(crate) fn step(len: usize, top: usize, i: usize, a: Act) -> Option<usize> {
    let upper = i < top;
    match a {
        Act::Left if i > 0 && (upper || i > top) => Some(i - 1),
        Act::Right if i + 1 < len && (!upper || i + 1 < top) => Some(i + 1),
        Act::Down if upper => Some(top),
        Act::Up if !upper => Some(0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::step;
    use crate::app::Act;

    #[test]
    fn focus_stays_in_its_row_and_moves_between_rows() {
        // A game running: Back to game, Close game, Downloads | Library, Settings, Controls, Power.
        let (n, top) = (7, 3);
        assert_eq!(step(n, top, 0, Act::Right), Some(1));
        assert_eq!(step(n, top, 2, Act::Right), None, "the top row ends at Downloads");
        assert_eq!(step(n, top, 3, Act::Left), None, "the bottom row starts at Library");
        assert_eq!(step(n, top, 1, Act::Down), Some(3));
        assert_eq!(step(n, top, 5, Act::Up), Some(0), "up lands on Back to game");
        assert_eq!(step(n, top, 6, Act::Right), None);
        assert_eq!(step(n, top, 4, Act::Confirm), None);
        // No game: Downloads | Library, Settings, Controls, Power.
        let (n, top) = (5, 1);
        assert_eq!(step(n, top, 0, Act::Down), Some(1));
        assert_eq!(step(n, top, 4, Act::Up), Some(0));
        assert_eq!(step(n, top, 0, Act::Right), None);
        assert_eq!(step(n, top, 0, Act::Up), None);
    }
}
