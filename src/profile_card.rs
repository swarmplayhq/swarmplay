//! The profile card, as a console's: pressing your avatar at the top opens who's playing (the
//! picture, the name, a game running or not, games and hours played) and where to go from there:
//! your profile, switching user, the Media gallery, Power. The Control Center stays on Guide (F1).

use crate::app::*;
use crate::audio::{self, Sound};
use tr::tr;

/// The card's lines, in order.
const ITEMS: [&str; 4] = ["profile", "switch", "gallery", "power"];

impl App {
    pub fn open_profile_card(&mut self) {
        audio::play(Sound::Select);
        let playing = self.live.first().map(|s| s.name.clone());
        let hours: f64 = self.locals.iter().map(|l| self.playtime(&l.l).total).sum::<f64>() / 3600.0;
        let games = self.locals.len();
        let status = match playing { Some(name) => tr!("Playing {}", name), None => tr!("Online") };
        let played = if hours >= 1.0 { tr!("{} h played", hours.floor() as u64) } else { tr!("Under an hour played") };
        let stats = format!("{} · {}", tr!("{n} game" | "{n} games" % games), played);
        let labels = [tr!("Profile · name, picture and colour"), tr!("Switch user"), tr!("Media gallery"), tr!("Power")];
        let ui = self.ui();
        ui.set_profile_status(status.into());
        ui.set_profile_playing(!self.live.is_empty());
        ui.set_profile_stats(stats.into());
        ui.set_profile_items(model(labels.into_iter().map(slint::SharedString::from).collect()));
        self.push_overlay(Overlay::ProfileCard, Z_PROFILE_CARD, 0);
    }

    pub fn act_profile_card(&mut self, a: Act) {
        let i = self.idx.max(0);
        match a {
            Act::Up if i > 0 => self.move_focus(Z_PROFILE_CARD, i - 1),
            Act::Down if (i as usize) + 1 < ITEMS.len() => self.move_focus(Z_PROFILE_CARD, i + 1),
            Act::Confirm => self.profile_choose(i as usize),
            Act::Back | Act::Options => self.back_soft(),
            _ => {}
        }
    }

    pub fn click_profile_card(&mut self, idx: i32) {
        if idx < 0 { return self.back_soft(); }
        if self.idx == idx { self.profile_choose(idx as usize); } else { self.move_focus(Z_PROFILE_CARD, idx); }
    }

    fn profile_choose(&mut self, i: usize) {
        let Some(item) = ITEMS.get(i).copied() else { return };
        self.back();
        match item {
            "profile" => self.open_setting(crate::settings::SId::Name),
            "switch" => { audio::play(Sound::Select); self.switch_user(); }
            "gallery" => if !self.open_gallery() {
                self.toast(&tr!("No screenshots yet"), &tr!("Share takes one in a game"), "info", crate::toast::INFO);
            },
            _ => self.open_power_menu(),
        }
    }
}
