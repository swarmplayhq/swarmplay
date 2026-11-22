//! Game Library, as a console's: your games, apart from the catalog. Two tabs (L1/R1):
//! Installed (played most recently first) and Your collection (also what's downloading or ready
//! to install). Each cover says where the game stands; ✕ plays it (or opens its download), the
//! Options button opens its menu (Information, Move, Delete…). Opened from the last tile of
//! Home's row and from the Options menu.

use crate::app::*;
use crate::audio::{self, Sound};
use crate::downloads::State;
use crate::{util, LibTile};
use tr::tr;

/// Covers per row (the grid's width is shared out between them).
pub const COLS: i32 = 6;

#[derive(Clone, Debug, PartialEq)]
pub enum LibItem {
    Local(usize),
    /// A download not installed yet (its key).
    Download(String),
}

#[derive(Default)]
pub struct LibView {
    /// 0 Installed, 1 Your collection.
    pub tab: i32,
    pub items: Vec<LibItem>,
    /// The tiles' image keys, to put art in as it arrives.
    pub keys: Vec<String>,
}

impl App {
    pub fn open_game_library(&mut self) {
        audio::play(Sound::Select);
        self.lib_view.tab = 0;
        self.lib_items();
        self.push_overlay(Overlay::GameLibrary, Z_LIBRARY, 0);
        self.lib_push();
    }

    /// What each tab holds: installed games, played most recently first (never played: by name);
    /// the collection adds downloads not installed yet, newest first, before them.
    fn lib_items(&mut self) {
        let mut locals: Vec<usize> = (0..self.locals.len()).collect();
        let last = |app: &App, l: usize| app.playtime(&app.locals[l].l).last;
        locals.sort_by(|&a, &b| last(self, b).total_cmp(&last(self, a)).then_with(|| self.locals[a].name.to_lowercase().cmp(&self.locals[b].name.to_lowercase())));
        let mut items: Vec<LibItem> = Vec::new();
        if self.lib_view.tab == 1 {
            let installs = self.installer.snapshot();
            let mut jobs = self.downloads.snapshot();
            jobs.sort_by_key(|j| std::cmp::Reverse(j.added));
            for j in jobs {
                let installed = installs.iter().any(|r| r.key == j.key && r.state == crate::installer::State::Installed);
                if !installed && !matches!(j.state, State::Cancelled) && j.total > 0 { items.push(LibItem::Download(j.key)); }
            }
        }
        items.extend(locals.into_iter().map(LibItem::Local));
        self.lib_view.items = items;
    }

    /// The grid to the screen (again as art and downloads come in).
    pub fn lib_push(&mut self) {
        if !self.ui().get_lib_open() { return; }
        let installs = self.installer.snapshot();
        let mut tiles = Vec::new();
        let mut keys = Vec::new();
        for item in self.lib_view.items.clone() {
            let (req, label, sub, badge, kind, bar) = match &item {
                LibItem::Local(l) => {
                    let lv = &self.locals[*l];
                    let pt = self.playtime(&lv.l);
                    let moving = self.games_view.moving.as_ref().filter(|m| m.from == lv.l.path)
                        .map(|m| if m.total > 0 { m.done.load(std::sync::atomic::Ordering::Relaxed) as f32 / m.total as f32 } else { 0.0 });
                    let sub = if self.session_for_local(*l).is_some() { tr!("Playing now") }
                        else if let Some(f) = moving { tr!("Moving · {}%", (f * 100.0).floor() as u32) }
                        else if pt.last > 0.0 { tr!("Last played {}", util::fmt_last_played(pt.last).to_lowercase()) }
                        else { tr!("Not played yet") };
                    (self.tile_req(RowItem::Local(*l)), lv.name.clone(), sub, lv.l.platform.label().to_string(), 0, moving.unwrap_or(-1.0))
                }
                LibItem::Download(key) => {
                    let Some(j) = self.downloads.job(key) else { continue };
                    let g = self.games.iter().position(|g| g.g.id == j.topic);
                    let req = g.and_then(|g| self.card_req(g)).or_else(|| g.and_then(|g| self.cover_req(Target { game: Some(g), local: None })));
                    let name = g.map(|g| self.games[g].name.clone()).unwrap_or(j.name.clone());
                    let installing = installs.iter().find(|r| r.key == *key && r.state.active());
                    let pct = |f: f32| (f * 100.0).floor() as u32;
                    let (sub, badge, kind, bar) = match (installing, j.state) {
                        (Some(r), _) => (tr!("Installing · {}%", pct(r.progress())), tr!("Installing"), 2, r.progress()),
                        (None, State::Complete) => (tr!("Ready to install"), tr!("Install"), 1, -1.0),
                        (None, State::Downloading | State::Checking) => (tr!("Downloading · {}%", pct(j.progress())), format!("{}%", pct(j.progress())), 2, j.progress()),
                        (None, State::Paused) => (tr!("Paused · {}%", pct(j.progress())), tr!("Paused"), 3, j.progress()),
                        (None, State::Failed) => (tr!("Stopped"), tr!("Stopped"), 3, -1.0),
                        _ => (tr!("Getting ready"), String::new(), 3, -1.0),
                    };
                    (req, name, sub, badge, kind, bar)
                }
            };
            keys.push(req.as_ref().map(|r| r.key.clone()).unwrap_or_default());
            let img = self.get_img(&req, crate::images::prio::TILE);
            tiles.push(LibTile { image: img.clone().unwrap_or_default(), loaded: img.is_some(), label: label.into(), sub: sub.into(), badge: badge.into(), kind, bar });
        }
        self.lib_view.keys = keys;
        let installed = self.locals.len();
        let collection = self.lib_view.items.len().max(installed);
        let ui = self.ui();
        ui.set_lib_tab(self.lib_view.tab);
        ui.set_lib_tabs(model(vec![tr!("Installed · {}", installed).into(), tr!("Your collection · {}", if self.lib_view.tab == 1 { collection } else { self.lib_collection_count() }).into()]));
        ui.set_lib_tiles(model(tiles));
    }

    /// How many the collection tab would hold (shown on its tab while the other is open).
    fn lib_collection_count(&self) -> usize {
        let installs = self.installer.snapshot();
        let downloads = self.downloads.snapshot().into_iter().filter(|j| j.total > 0 && !matches!(j.state, State::Cancelled)
            && !installs.iter().any(|r| r.key == j.key && r.state == crate::installer::State::Installed)).count();
        downloads + self.locals.len()
    }

    /// Art for a tile came in.
    pub fn lib_image(&mut self, key: &str) {
        if self.images.get(key).is_some() && self.lib_view.keys.iter().any(|k| k == key) { self.lib_push(); }
    }

    /// Every second: downloads, installs and moves as they go.
    pub fn lib_tick(&mut self) {
        if self.overlay == Overlay::GameLibrary && (self.lib_view.tab == 1 || self.games_view.moving.is_some()) { self.lib_push(); }
    }

    fn lib_switch(&mut self, tab: i32) {
        if tab == self.lib_view.tab { return; }
        audio::play(Sound::Tab);
        self.lib_view.tab = tab;
        self.lib_items();
        self.set_focus(Z_LIBRARY, 0);
        self.lib_push();
    }

    pub fn act_game_library(&mut self, a: Act) {
        let n = self.lib_view.items.len() as i32;
        let i = self.idx.max(0);
        match a {
            Act::Left if i % COLS > 0 => self.move_focus(Z_LIBRARY, i - 1),
            Act::Right if i % COLS < COLS - 1 && i + 1 < n => self.move_focus(Z_LIBRARY, i + 1),
            Act::Up if i >= COLS => self.move_focus(Z_LIBRARY, i - COLS),
            Act::Down if i + COLS < n => self.move_focus(Z_LIBRARY, i + COLS),
            // The last row may be short: Down lands on its last cover.
            Act::Down if i / COLS < (n - 1) / COLS => self.move_focus(Z_LIBRARY, n - 1),
            Act::TabPrev => self.lib_switch(0),
            Act::TabNext => self.lib_switch(1),
            Act::Confirm => self.lib_open(i as usize),
            Act::Options => self.lib_options(i as usize),
            Act::Back => self.back_soft(),
            _ => {}
        }
    }

    /// A cover clicked: selected, or (already selected) opened. A tab clicked: that tab.
    pub fn click_game_library(&mut self, idx: i32) {
        match idx {
            -1 => self.back_soft(),
            -3 | -2 => self.lib_switch(-idx - 2),
            _ if self.idx == idx => self.lib_open(idx as usize),
            _ => self.move_focus(Z_LIBRARY, idx),
        }
    }

    /// ✕: play it (Resume if it's running), or open its download.
    fn lib_open(&mut self, i: usize) {
        match self.lib_view.items.get(i).cloned() {
            Some(LibItem::Local(l)) => {
                let t = Target { game: self.locals[l].cat, local: Some(l) };
                if self.session_for_local(l).is_some() { self.run_action("resume", t); } else { self.run_action("play", t); }
            }
            Some(LibItem::Download(key)) => self.open_downloads(Some(&key)),
            None => {}
        }
    }

    /// The Options button: the game's menu (Game Hub, Information, Move, Delete…).
    fn lib_options(&mut self, i: usize) {
        match self.lib_view.items.get(i).cloned() {
            Some(LibItem::Local(l)) => self.open_menu(Target { game: self.locals[l].cat, local: Some(l) }),
            Some(LibItem::Download(key)) => self.open_downloads(Some(&key)),
            None => {}
        }
    }
}
