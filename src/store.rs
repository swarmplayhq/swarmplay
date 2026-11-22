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

/// What each line of Sort and filter changes.
#[derive(Clone, Copy, PartialEq)]
enum FilterLine { Sort, Show, Console, Genre, Covers }

impl App {
    /// The lines of Sort and filter (Console only when the catalog has more than one).
    fn filter_lines(&self) -> Vec<FilterLine> {
        let mut v = vec![FilterLine::Sort, FilterLine::Show];
        if self.mixed_consoles { v.push(FilterLine::Console); }
        v.extend([FilterLine::Genre, FilterLine::Covers]);
        v
    }

    /// The genres to step through ("All" first), as the chips listed them.
    fn filter_genres(&self) -> Vec<String> {
        std::iter::once("All".to_string()).chain(self.genre_list.iter().skip(self.status_count + 1).map(|(g, _)| g.clone())).collect()
    }

    /// What's chosen, in a few words: under Browse's search, and on the panel.
    pub fn filter_summary(&self) -> String {
        let mut parts = vec![crate::app::sort_label(self.sort)];
        if self.status_filter != "All" { parts.push(crate::app::filter_label(&self.status_filter)); }
        if let Some(p) = self.platform_filter { parts.push(p.label().to_string()); }
        if self.genre != "All" { parts.push(crate::app::filter_label(&self.genre)); }
        parts.join(" · ")
    }

    pub fn open_filters(&mut self) {
        audio::play(Sound::Select);
        self.push_overlay(Overlay::Filters, Z_FILTERS, 0);
        self.push_filters();
    }

    fn push_filters(&mut self) {
        let rows: Vec<crate::FilterRow> = self.filter_lines().into_iter().map(|l| {
            let (label, value) = match l {
                FilterLine::Sort => (tr!("Sort"), crate::app::sort_label(self.sort)),
                FilterLine::Show => (tr!("Show"), if self.status_filter == "All" { tr!("All games") } else { crate::app::filter_label(&self.status_filter) }),
                FilterLine::Console => (tr!("Console"), self.platform_filter.map_or(tr!("Every console"), |p| p.label().to_string())),
                FilterLine::Genre => (tr!("Genre"), if self.genre == "All" { tr!("All genres") } else { crate::app::filter_label(&self.genre) }),
                FilterLine::Covers => (tr!("Covers"), if self.cfg.lock().unwrap().library_compact { tr!("Small") } else { tr!("Large") }),
            };
            crate::FilterRow { label: label.into(), value: value.into() }
        }).collect();
        let ui = self.ui();
        ui.set_filter_rows(model(rows));
        ui.set_filter_count(self.ui().get_count_label());
        ui.set_filter_summary(self.filter_summary().into());
    }

    /// ◂ ▸ (or ✕, forwards) on a line: the next choice, shown at once behind the panel.
    fn filter_step(&mut self, line: usize, d: i32) {
        let Some(l) = self.filter_lines().get(line).copied() else { return };
        let step = |len: usize, cur: usize| (cur as i32 + d).rem_euclid(len.max(1) as i32) as usize;
        match l {
            FilterLine::Sort => { self.cycle_sort(d); return self.push_filters(); }
            FilterLine::Covers => {
                let compact = self.cfg.lock().unwrap().library_compact;
                self.set_library_density(!compact);
                return self.push_filters();
            }
            FilterLine::Show => {
                const SHOW: [&str; 3] = ["All", "Installed", "Playable"];
                let cur = SHOW.iter().position(|s| *s == self.status_filter).unwrap_or(0);
                self.status_filter = SHOW[step(SHOW.len(), cur)].to_string();
            }
            FilterLine::Console => {
                let all: Vec<Option<Platform>> = std::iter::once(None).chain(self.consoles.iter().copied().map(Some)).collect();
                let cur = all.iter().position(|p| *p == self.platform_filter).unwrap_or(0);
                self.platform_filter = all[step(all.len(), cur)];
            }
            FilterLine::Genre => {
                let genres = self.filter_genres();
                let cur = genres.iter().position(|g| *g == self.genre).unwrap_or(0);
                self.genre = genres[step(genres.len(), cur)].clone();
            }
        }
        audio::play(Sound::Move);
        self.remember_library_view();
        self.apply_filter();
        self.set_grid_scroll(0.0, 0);
        self.push_library();
        self.push_filters();
    }

    pub fn act_filters(&mut self, a: Act) {
        let n = self.filter_lines().len() as i32;
        let i = self.idx.max(0);
        match a {
            Act::Up if i > 0 => self.move_focus(Z_FILTERS, i - 1),
            Act::Down if i + 1 < n => self.move_focus(Z_FILTERS, i + 1),
            Act::Left => self.filter_step(i as usize, -1),
            Act::Right | Act::Confirm => self.filter_step(i as usize, 1),
            Act::Back | Act::Options => self.back_soft(),
            _ => {}
        }
    }

    /// A line clicked: chosen, then (chosen already) its next choice; outside: closed.
    pub fn click_filters(&mut self, idx: i32) {
        if idx < 0 { return self.back_soft(); }
        if self.idx == idx { self.filter_step(idx as usize, 1); } else { self.move_focus(Z_FILTERS, idx); }
    }
}
