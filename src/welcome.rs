//! Welcome: Home's first tile, as on a console. In place of a game's hero it shows six widgets,
//! three by two, about SwarmPlay itself: the controller, the space left for games, the
//! downloads, what's new in the catalog, your games, and the emulators. Each opens where that
//! is looked after.
//!
//! Widgets refresh every two seconds while the Welcome is on screen (a download's bar moves, a
//! controller is paired), from what's cheap to read: /proc and /sys, one statvfs, the emulators'
//! small state files.

use tr::tr;
use crate::app::*;
use crate::audio::{self, Sound};
use crate::{util, WidgetData};
use slint::Model;

/// The widgets, in focus order (three a row).
const WIDGETS: [&str; 6] = ["controller", "storage", "downloads", "news", "games", "emulators"];
const COLS: usize = 3;

/// The Fact colours (ui/theme.slint).
const GOOD: i32 = 1;
const WARN: i32 = 2;

impl App {
    fn on_welcome(&self) -> bool {
        self.view == 0 && self.row.get(self.sel) == Some(&RowItem::Welcome)
    }

    /// Show the widgets when the Welcome is the tile selected (and nothing otherwise).
    pub fn push_welcome(&mut self) {
        let on = self.on_welcome();
        let ui = self.ui();
        ui.set_home_welcome(on);
        if on {
            // In place: new rows would come in again (their entrance) every refresh.
            let widgets = self.welcome_widgets();
            let shown = ui.get_welcome();
            if shown.row_count() == widgets.len() {
                for (i, w) in widgets.into_iter().enumerate() { shown.set_row_data(i, w); }
            } else {
                ui.set_welcome(model(widgets));
            }
        }
        self.welcome_read = util::now_secs();
    }

    /// From the tick: the widgets stay current while they're on screen.
    pub fn welcome_tick(&mut self) {
        if self.on_welcome() && util::now_secs() - self.welcome_read >= 2.0 {
            self.push_welcome();
        }
    }

    fn welcome_widgets(&self) -> Vec<WidgetData> {
        WIDGETS.iter().map(|id| match *id {
            "controller" => self.controller_widget(),
            "storage" => self.storage_widget(),
            "downloads" => self.downloads_widget(),
            "news" => self.news_widget(),
            "games" => self.games_widget(),
            _ => self.emulators_widget(),
        }).collect()
    }

    fn controller_widget(&self) -> WidgetData {
        let pads = crate::gamepad::connected();
        let battery = crate::battery::controllers().into_iter().next();
        let Some(name) = pads.first() else {
            return widget(Icons::Gamepad, &tr!("Controller"), &tr!("None connected"), &tr!("Pair one over Bluetooth or plug it in"), -1.0, 0);
        };
        let more = if pads.len() > 1 { format!(" · {}", tr!("{n} connected" | "{n} connected" % pads.len())) } else { String::new() };
        match battery {
            Some(b) => widget(
                Icons::Gamepad, &tr!("Controller"),
                &if b.plugged { tr!("{}% · charging", b.percent) } else { format!("{}%", b.percent) },
                &format!("{name}{more}"),
                b.percent as f32 / 100.0,
                if b.plugged || b.percent > 20 { GOOD } else { WARN },
            ),
            None => widget(Icons::Gamepad, &tr!("Controller"), &tr!("Connected"), &format!("{name}{more}"), -1.0, GOOD),
        }
    }

    fn storage_widget(&self) -> WidgetData {
        let dir = self.cfg.lock().unwrap().install_dir.clone();
        match crate::downloads::free_space(&util::expand_home(&dir)) {
            Some(free) => {
                let gb = free as f64 / (1u64 << 30) as f64;
                let value = if gb >= 1024.0 { tr!("{} TB free", crate::i18n::num(gb / 1024.0, 1)) } else { tr!("{} GB free", crate::i18n::num(gb, 0)) };
                widget(Icons::Folder, &tr!("Storage"), &value, &tr!("For games in {dir}", dir), -1.0, if gb < 50.0 { WARN } else { 0 })
            }
            None => widget(Icons::Folder, &tr!("Storage"), &tr!("Unknown"), &tr!("Can't read {dir}", dir), -1.0, WARN),
        }
    }

    fn downloads_widget(&self) -> WidgetData {
        let jobs = self.downloads.snapshot();
        let active: Vec<_> = jobs.iter().filter(|j| j.state.active()).collect();
        if active.is_empty() {
            let paused = jobs.iter().filter(|j| j.state == crate::downloads::State::Paused).count();
            let sub = if paused > 0 { tr!("{n} paused" | "{n} paused" % paused) } else { tr!("Get games in the Store") };
            return widget(Icons::Download, &tr!("Downloads"), &tr!("Nothing downloading"), &sub, -1.0, 0);
        }
        let (done, total) = active.iter().fold((0u64, 0u64), |(d, t), j| (d + j.done, t + j.total));
        let progress = if total > 0 { done as f32 / total as f32 } else { 0.0 };
        let percent = format!("{:.0}", progress * 100.0);
        let value = if active.len() == 1 { format!("{percent}%") } else { tr!("{n} downloading" | "{n} downloading" % active.len()) };
        let sub = if active.len() == 1 { active[0].name.clone() } else { tr!("{}% of all of them", percent) };
        widget(Icons::Download, &tr!("Downloads"), &value, &sub, progress, GOOD)
    }

    fn news_widget(&self) -> WidgetData {
        let mut new: Vec<&GameV> = self.games.iter().filter(|g| g.is_new).collect();
        new.sort_by_key(|g| std::cmp::Reverse(g.rel_day));
        let mut names: Vec<&str> = Vec::new();
        for g in &new {
            if !names.contains(&g.name.as_str()) { names.push(&g.name); }
        }
        if names.is_empty() {
            return widget(Icons::Sparkle, &tr!("New in the catalog"), &tr!("Nothing this fortnight"), &tr!("Browse the whole Store"), -1.0, 0);
        }
        let value = tr!("{n} new game" | "{n} new games" % names.len());
        widget(Icons::Sparkle, &tr!("New in the catalog"), &value, &names.iter().take(2).copied().collect::<Vec<_>>().join(" · "), -1.0, 0)
    }

    fn games_widget(&self) -> WidgetData {
        let games: Vec<usize> = self.row.iter().filter_map(|r| match r { RowItem::Local(l) => Some(*l), _ => None }).collect();
        let played: f64 = games.iter().map(|l| self.playtime(&self.locals[*l].l).total).sum();
        let value = tr!("{n} installed" | "{n} installed" % games.len());
        let sub = if played >= 60.0 { tr!("{} played in all", util::fmt_duration_words(played)) } else { tr!("None played yet") };
        widget(Icons::Grid, &tr!("Your games"), &value, &sub, -1.0, 0)
    }

    /// SwarmPlay's own update first (what needs you), then the emulators.
    fn emulators_widget(&self) -> WidgetData {
        if let Some(v) = &self.upd.installed {
            return widget(Icons::Gear, &tr!("Updates"), &tr!("Restart to update"), &tr!("SwarmPlay {v} is ready", v), -1.0, WARN);
        }
        if self.app_update_available() {
            let v = self.upd.latest.as_ref().map(|r| r.version.clone()).unwrap_or_default();
            return widget(Icons::Gear, &tr!("Updates"), &format!("SwarmPlay {v}"), &tr!("Out now · press to get it"), -1.0, WARN);
        }
        let (kyty, shad) = (crate::kyty::load_state(), crate::shad::load_state());
        let kyty_v = crate::kyty::pretty(&kyty.installed);
        let kyty_v = kyty_v.split(" · ").next().unwrap_or_default();
        let shad_v = crate::shad::pretty(&shad.installed);
        let parts: Vec<String> = [("SwarmPlay", crate::update::current_version()), ("KytyPS5", kyty_v.to_string()), ("shadPS4", shad_v)].iter()
            .filter(|(_, v)| !v.is_empty()).map(|(n, v)| format!("{n} {v}")).collect();
        let (value, kind) = if self.updates_need_you() { (tr!("Update ready"), WARN) } else { (tr!("Up to date"), GOOD) };
        widget(Icons::Gear, &tr!("Updates"), &value, &parts.join(" · "), -1.0, kind)
    }

    pub fn act_welcome(&mut self, a: Act) {
        let i = (self.idx.max(0) as usize).min(WIDGETS.len() - 1);
        if let Some(to) = step(WIDGETS.len(), i, a) {
            return self.move_focus(Z_WELCOME, to as i32);
        }
        match a {
            Act::Up => self.move_focus(Z_ROW, 0),
            Act::Back => self.back(),
            Act::Confirm => match WIDGETS[i] {
                "controller" => self.open_controls(),
                "downloads" => self.open_downloads(None),
                "news" => self.switch_view(1, true),
                // The games are the tiles beside it.
                "games" if self.row.len() > 1 => { audio::play(Sound::Move); self.set_focus(Z_ROW, 0); self.select_tile(1); }
                // Straight to SwarmPlay's update.
                "emulators" => self.open_setting(crate::settings::SId::AppUpdate),
                _ => self.open_settings(),
            },
            _ => {}
        }
    }
}

/// Where focus goes in a grid of `len` widgets, `COLS` a row. None: off its edge.
fn step(len: usize, i: usize, a: Act) -> Option<usize> {
    match a {
        Act::Left if i % COLS > 0 => Some(i - 1),
        Act::Right if i % COLS + 1 < COLS && i + 1 < len => Some(i + 1),
        Act::Down if i + COLS < len => Some(i + COLS),
        Act::Up if i >= COLS => Some(i - COLS),
        _ => None,
    }
}

/// Which icon a widget shows (ui/app.slint picks the image).
#[derive(Clone, Copy)]
enum Icons { Gamepad, Folder, Download, Sparkle, Grid, Gear }

fn widget(icon: Icons, title: &str, value: &str, sub: &str, progress: f32, kind: i32) -> WidgetData {
    WidgetData { icon: icon as i32, title: title.into(), value: value.into(), sub: sub.into(), progress, kind }
}

#[cfg(test)]
mod tests {
    use super::step;
    use crate::app::Act;

    #[test]
    fn focus_moves_across_the_three_by_two_grid() {
        assert_eq!(step(6, 0, Act::Right), Some(1));
        assert_eq!(step(6, 2, Act::Right), None, "the row ends at its third widget");
        assert_eq!(step(6, 3, Act::Left), None);
        assert_eq!(step(6, 1, Act::Down), Some(4));
        assert_eq!(step(6, 4, Act::Down), None);
        assert_eq!(step(6, 5, Act::Up), Some(2));
        assert_eq!(step(6, 0, Act::Up), None, "up from the top row is the tile row's (the caller's)");
    }
}
