//! This PC in the launcher: the Game Hub's line about it, the Settings row, the download
//! dialog's warning and a one-time notice when a console's games can't run here.

use crate::app::*;
use crate::pc::{Specs, Verdict};
use crate::platform::Platform;
use tr::tr;

/// The reason behind a verdict, as a sentence.
pub fn explain(v: &Verdict, platform: Platform) -> String {
    let ps5 = platform == Platform::Ps5;
    let console = platform.label();
    match v {
        Verdict::Good => tr!("{console} games should run well", console),
        Verdict::No(why) if why == "no AVX2" => tr!("PS4 games need a processor from 2013 or later (with AVX2)"),
        Verdict::No(why) if why == "no Vulkan 1.3" => tr!("Games need a graphics card with Vulkan 1.3: update the graphics driver, or the card is too old"),
        Verdict::No(why) if why == "missing features" => tr!(
            "Your graphics card lacks Vulkan features {} needs: update its driver, or the card is too old", platform.emulator()),
        Verdict::No(_) => tr!("No graphics driver found: install the driver for your graphics card"),
        Verdict::Slow(why) if why == "GPU at low clock" => tr!("On Linux this NVIDIA card runs on the open-source NVK driver, which keeps GTX 600–1000 series cards at a low clock speed, so games run slowly"),
        Verdict::Slow(why) if why == "integrated GPU" => tr!("{console} games run best on a graphics card; built-in graphics may be slow", console),
        Verdict::Slow(why) if why.ends_with("GPU") => tr!("{console} games run best with {gb} GB of graphics memory", console, gb = if ps5 { 8 } else { 4 }),
        Verdict::Slow(why) if why.ends_with("CPU") => tr!("{console} games run best on {cores} or more processor cores", console, cores = if ps5 { 6 } else { 4 }),
        Verdict::Slow(_) => tr!("{console} games run best with {gb} GB of RAM", console, gb = if ps5 { 16 } else { 8 }),
    }
}

impl App {
    /// Read this PC in the background (at start, and from Settings).
    pub fn pc_check(&mut self) {
        if self.pc_checking {
            return;
        }
        self.pc_checking = true;
        std::thread::spawn(|| {
            let specs = crate::pc::detect();
            let _ = slint::invoke_from_event_loop(move || with_app(move |app| app.pc_detected(specs)));
        });
    }

    fn pc_detected(&mut self, specs: Specs) {
        self.pc_checking = false;
        self.pc = Some(crate::pc::Reading::new(specs));
        // Which version each Library card shows depends on what plays here: the focused card
        // stays the one focused (keep_grid_focus).
        let anchor = self.catalog_anchor();
        self.apply_filter();
        if !self.boot.active {
            if self.view == 1 {
                self.push_grid();
            }
            // Also a card saved under an open card while another view shows.
            self.keep_grid_focus(&anchor);
        }
        match self.overlay {
            Overlay::Hub => self.push_hub(),
            Overlay::Settings => self.refresh_settings(),
            _ => {}
        }
    }

    /// This PC's verdict for a platform's games (worked out once per reading of the PC).
    pub fn pc_verdict(&self, platform: Platform) -> Option<&Verdict> {
        self.pc.as_ref()?.verdict(platform)
    }

    /// The Hub's line about this PC, under its verdict: "This PC: PS5 games should run well".
    pub fn pc_line(&self, t: Target) -> Option<String> {
        let platform = self.target_platform(t);
        self.pc_verdict(platform).map(|v| tr!("This PC: {}", explain(v, platform)))
    }

    /// Under the Hub's Download button: what pressing it means. The download's size, the folder
    /// it goes to and the next step, or a warning when that folder's disk is short of space.
    /// `None` once it's downloading or installed (the button says so itself).
    pub fn download_note(&self, t: Target) -> Option<(String, bool)> {
        let g = &self.games[t.game?];
        if t.local.is_some() || g.g.magnet.is_empty() || g.g.is_pkg() || self.download_for_target(t).is_some() {
            return None;
        }
        let dir = self.cfg.lock().unwrap().download_dir.clone();
        let shown = crate::util::display_path(&dir);
        let size = crate::present::size_label(&g.g);
        let free = crate::pc::free_gb(&crate::util::expand_home(&dir));
        let short = matches!((g.g.size_gb, free), (Some(need), Some(free)) if need > free);
        // An older version than the catalog has: said before anything is downloaded.
        let older_note = g.newer.and_then(|n| Some((g.version_shown.as_deref()?, self.games[n].version_shown.as_deref()?)))
            .map(|(this, newest)| if g.outdated {
                tr!("Older version {this}: {newest} is under Choose another release", this, newest)
            } else {
                tr!("Version {this}: {newest} is out, but nobody is sharing it", this, newest)
            });
        let mut parts: Vec<String> = size.into_iter().collect();
        parts.push(tr!("goes to {shown}", shown));
        parts.push(match free {
            Some(free) if short => tr!("only {free} GB free there", free = format!("{free:.0}")),
            _ => tr!("then it installs by itself"),
        });
        // An older version said first, unless the disk is short of space: that comes first (the
        // line is cut at the end when it's long).
        if let Some(note) = older_note {
            if short { parts.push(note) } else { parts.insert(0, note) }
        }
        // Amber when there's something to do about it: a newer version someone shares.
        Some((parts.join(" · "), short || g.outdated))
    }

    /// For the download dialog: a warning when this game won't start or may be slow here.
    pub fn pc_download_warning(&self, platform: Platform) -> String {
        match self.pc_verdict(platform) {
            Some(v @ Verdict::No(_)) => tr!("This PC can't run {} games yet. {}.", platform.label(), explain(v, platform)),
            Some(v @ Verdict::Slow(_)) => tr!("It may run slowly on this PC. {}.", explain(v, platform)),
            _ => String::new(),
        }
    }
}
