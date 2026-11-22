//! The Store (the catalog, once the Library tab), as a console's: Discover, a featured game and
//! short rows the catalog already has the facts for (new, most shared, plays on this PC, by
//! console), and Browse, the grid with its search, sort and filters. L1/R1 go Home ← Discover ↔
//! Browse, as a console's Store tabs do.

use crate::app::*;
use crate::audio::{self, Sound};
use crate::platform::Platform;
use crate::{DiscoverHero, DiscoverRow, DiscoverTile};
use tr::tr;

/// Covers per row on Discover.
const PER_ROW: usize = 12;

impl App {
    /// One game per catalog card (its best release), only cards with a picture.
    fn store_cards(&self) -> Vec<usize> {
        let shown = self.browsable();
        self.groups.members.iter().zip(shown).filter(|(_, s)| *s)
            .filter_map(|(members, _)| members.iter().copied().max_by_key(|i| self.release_rank(*i))).collect()
    }

    /// Discover's rows: what's new, what's most shared (quickest to download), what plays on
    /// this PC, then each console's most shared. A row with nothing in it isn't shown.
    fn discover_rows(&self) -> Vec<(String, Vec<usize>)> {
        let cards = self.store_cards();
        let gs = &self.games;
        let newest = |i: usize| self.groups.releases(i).iter().map(|x| gs[*x].g.id).max().unwrap_or(gs[i].g.id);
        let by_seeders = |mut v: Vec<usize>| { v.sort_by_key(|i| std::cmp::Reverse(gs[*i].g.seeders)); v.truncate(PER_ROW); v };
        let mut new = cards.clone();
        new.sort_by_key(|i| std::cmp::Reverse(newest(*i)));
        new.truncate(PER_ROW);
        let plays = cards.iter().copied().filter(|i| gs[*i].g.platform != Platform::Pc && self.game_compat(&gs[*i]).is_some_and(|e| e.in_game())).collect();
        let console = |p: Platform| cards.iter().copied().filter(|i| gs[*i].g.platform == p).collect();
        let rows = vec![
            (tr!("New in the catalog"), new),
            (tr!("Most shared"), by_seeders(cards.clone())),
            (tr!("Plays on this PC"), by_seeders(plays)),
            (tr!("PS5 games"), by_seeders(console(Platform::Ps5))),
            (tr!("PS4 games"), by_seeders(console(Platform::Ps4))),
            (tr!("PC games"), by_seeders(console(Platform::Pc))),
        ];
        rows.into_iter().filter(|(_, v)| !v.is_empty()).collect()
    }

    /// Discover or Browse (0, 1), as L1/R1 or a click chose.
    pub fn store_switch(&mut self, tab: i32) {
        if self.store_tab != tab { audio::play(Sound::Tab); }
        self.store_tab = tab;
        self.ui().set_store_tab(tab);
        if tab == 0 {
            self.refresh_discover();
            self.set_focus(Z_DISCOVER, 0);
        } else if self.filtered.is_empty() {
            self.set_focus(Z_SEARCH, 0);
        } else {
            let first = self.first_visible_card();
            self.set_focus(Z_GRID, first as i32);
            self.focus_card_changed();
        }
    }

    /// L1/R1 anywhere on Home or the Store: Home ← Discover ↔ Browse.
    pub fn store_tab_key(&mut self, next: bool) {
        match (self.view, self.store_tab, next) {
            (0, _, true) => { self.store_tab = 0; self.ui().set_store_tab(0); self.switch_view(1, true); }
            (1, 0, true) => self.store_switch(1),
            (1, 1, false) => self.store_switch(0),
            (1, 0, false) => self.switch_view(0, true),
            _ => {}
        }
    }

    /// Work Discover's rows out again (going to it: the catalog, downloads may have changed), then
    /// show them. Art coming in only shows them again (push_discover): the rows go through every
    /// card of the catalog, too much to do for each picture.
    pub fn refresh_discover(&mut self) {
        let started = std::time::Instant::now();
        self.discover = self.discover_rows();
        crate::log!("discover: {} rows in {} ms", self.discover.len(), started.elapsed().as_millis());
        self.push_discover();
    }

    /// Discover to the screen: the featured game (the newest card with big art) and the rows.
    pub fn push_discover(&mut self) {
        let mut keys = Vec::new();
        let mut rows = Vec::new();
        for (title, games) in self.discover.clone() {
            let mut tiles = Vec::new();
            for g in games {
                let r = self.card_req(g);
                keys.push(r.as_ref().map(|r| r.key.clone()).unwrap_or_default());
                let img = self.get_img(&r, crate::images::prio::TILE);
                let t = Target { game: Some(g), local: self.games[g].local };
                let (badge, kind) = if t.local.is_some() { (tr!("Installed"), 1) } else {
                    match self.download_for_target(t) {
                        Some(j) if j.state == crate::downloads::State::Complete => (tr!("Install"), 1),
                        Some(j) if j.total > 0 => (format!("{}%", (j.progress() * 100.0).floor() as u32), 2),
                        _ => (String::new(), 0),
                    }
                };
                tiles.push(DiscoverTile { image: img.clone().unwrap_or_default(), loaded: img.is_some(), label: self.games[g].name.clone().into(), badge: badge.into(), kind });
            }
            rows.push(DiscoverRow { title: title.into(), tiles: model(tiles) });
        }
        let featured = self.discover.first().and_then(|(_, v)| v.iter().copied().find(|g| self.games[*g].info.as_ref().is_some_and(|i| !i.hub.is_empty())).or(v.first().copied()));
        let mut hero = DiscoverHero::default();
        if let Some(g) = featured {
            let t = Target { game: Some(g), local: self.games[g].local };
            let r = self.target_bg_url(t);
            keys.push(r.as_ref().map(|r| r.key.clone()).unwrap_or_default());
            let img = self.get_img(&r, crate::images::prio::HERO);
            let gv = &self.games[g];
            let facts = [gv.g.platform.label().to_string(), gv.g.size.clone(), self.game_compat(gv).map(|e| e.chip_text()).unwrap_or_default()];
            hero = DiscoverHero { image: img.clone().unwrap_or_default(), loaded: img.is_some(), kicker: tr!("NEW IN THE CATALOG").into(), title: gv.name.clone().into(),
                sub: facts.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ").into() };
        }
        self.discover_hero = featured;
        self.discover_keys = keys;
        let ui = self.ui();
        ui.set_discover_hero(hero);
        ui.set_discover_rows(model(rows));
    }

    /// Art for Discover came in.
    pub fn discover_image(&mut self, key: &str) {
        // Only art that came (a failed one would ask again at once, and round it would go).
        if self.view == 1 && self.store_tab == 0 && self.images.get(key).is_some() && self.discover_keys.iter().any(|k| k == key) { self.push_discover(); }
    }

    /// Discover's focus: idx = row × 100 + column; row 0 is the featured game's button.
    pub fn act_discover(&mut self, a: Act) {
        let (row, col) = ((self.idx / 100) as usize, (self.idx % 100) as usize);
        let len = |r: usize| if r == 0 { 1 } else { self.discover.get(r - 1).map_or(0, |(_, v)| v.len()) };
        let rows = self.discover.len() + 1;
        let to = |r: usize, c: usize| (r * 100 + c) as i32;
        match a {
            Act::Left if col > 0 => self.move_focus(Z_DISCOVER, to(row, col - 1)),
            Act::Right if col + 1 < len(row) => self.move_focus(Z_DISCOVER, to(row, col + 1)),
            Act::Up if row == 0 => self.move_focus(Z_TABS, 1),
            Act::Up => self.move_focus(Z_DISCOVER, to(row - 1, col.min(len(row - 1).saturating_sub(1)))),
            Act::Down if row + 1 < rows => self.move_focus(Z_DISCOVER, to(row + 1, col.min(len(row + 1).saturating_sub(1)))),
            Act::Confirm => self.discover_open(row, col),
            Act::Search => self.open_search(),
            _ => {}
        }
    }

    pub fn click_discover(&mut self, idx: i32) {
        if self.zone == Z_DISCOVER && self.idx == idx { self.discover_open((idx / 100) as usize, (idx % 100) as usize); } else { self.move_focus(Z_DISCOVER, idx); }
    }

    /// The featured game or a cover: its Game Hub.
    fn discover_open(&mut self, row: usize, col: usize) {
        let g = if row == 0 { self.discover_hero } else { self.discover.get(row - 1).and_then(|(_, v)| v.get(col).copied()) };
        if let Some(g) = g {
            audio::play(Sound::Select);
            self.open_hub(Target { game: Some(g), local: self.games[g].local });
        }
    }
}
