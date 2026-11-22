//! Themes: a wallpaper and an accent colour, chosen in Settings → Theme and kept in the
//! config. The wallpaper shows on Home's Welcome and behind Settings (a game's tile always
//! shows the game's own art, as on a console); the accent colours what's highlighted.

use crate::app::*;
use crate::settings::SId;
use crate::{util, SettingData};
use slint::{Color, ComponentHandle};
use tr::tr;

/// The wallpapers, in the order Settings steps through them; ui/app.slint draws each by its
/// position ("Your image" is the user's picture).
pub const WALLPAPERS: [&str; 6] = ["Game art", "Deep blue", "Midnight", "Aurora", "Ember", "Your image"];
const YOUR_IMAGE: usize = 5;

/// The accent colours and their focus glow.
pub const ACCENTS: [(&str, u32); 6] = [
    ("Blue", 0x0a6cff), ("Violet", 0x7c5cff), ("Teal", 0x14b8a6), ("Orange", 0xff7a1a), ("Pink", 0xff4fa3), ("Green", 0x22c55e),
];

/// A wallpaper (WALLPAPERS' name, which the config keeps), as shown.
pub fn wallpaper_label(key: &str) -> String {
    wallpaper_text(key).unwrap_or_else(|| key.to_string())
}

fn wallpaper_text(key: &str) -> Option<String> {
    Some(match key {
        "Game art" => tr!("Game art"),
        "Deep blue" => tr!("Deep blue"),
        "Midnight" => tr!("Midnight"),
        "Aurora" => tr!("Aurora"),
        "Ember" => tr!("Ember"),
        "Your image" => tr!("Your image"),
        _ => return None,
    })
}

/// An accent colour (ACCENTS' name, which the config keeps), as shown.
pub fn accent_label(key: &str) -> String {
    accent_text(key).unwrap_or_else(|| key.to_string())
}

fn accent_text(key: &str) -> Option<String> {
    Some(match key {
        "Blue" => tr!("Blue"),
        "Violet" => tr!("Violet"),
        "Teal" => tr!("Teal"),
        "Orange" => tr!("Orange"),
        "Pink" => tr!("Pink"),
        "Green" => tr!("Green"),
        _ => return None,
    })
}

fn rgb(hex: u32) -> Color {
    Color::from_rgb_u8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// One step through `list` from `cur` (by name; an unknown name starts at the first).
fn step<'a>(list: &[&'a str], cur: &str, dir: i32) -> &'a str {
    let pos = list.iter().position(|n| *n == cur).unwrap_or(0) as i32;
    list[(pos + dir).rem_euclid(list.len() as i32) as usize]
}

impl App {
    /// Show the chosen theme: the accent (and its glow) and the wallpaper.
    pub fn apply_theme(&self) {
        let cfg = self.cfg.lock().unwrap().clone();
        let ui = self.ui();
        let accent = ACCENTS.iter().find(|(n, _)| *n == cfg.theme_accent).map_or(ACCENTS[0].1, |a| a.1);
        let theme = ui.global::<crate::Theme>();
        theme.set_accent(rgb(accent));
        // Lighter than the accent, so the halo shows over dark art too.
        theme.set_glow(rgb(accent).brighter(0.45).with_alpha(0.78));
        self.ov_style();
        let kind = WALLPAPERS.iter().position(|w| *w == cfg.theme_wallpaper).unwrap_or(0);
        let image = (kind == YOUR_IMAGE)
            .then(|| slint::Image::load_from_path(&util::expand_home(&cfg.theme_image)).ok())
            .flatten();
        // The picture can't be read: the default wallpaper, not an empty one.
        ui.set_wallpaper_kind(if kind == YOUR_IMAGE && image.is_none() { 1 } else { kind as i32 });
        ui.set_wallpaper_image(image.unwrap_or_default());
    }

    pub fn theme_rows(&self) -> Vec<(SId, SettingData)> {
        let cfg = self.cfg.lock().unwrap().clone();
        let row = |kind: i32, label: &str| SettingData { kind, label: label.into(), ..Default::default() };
        let mut rows = Vec::new();
        let mut r = row(3, &tr!("Wallpaper"));
        r.value = wallpaper_label(&cfg.theme_wallpaper).into();
        r.hint = tr!("On Welcome and behind Settings · a game always shows its own art").into();
        rows.push((SId::ThemeWallpaper, r));
        if cfg.theme_wallpaper == WALLPAPERS[YOUR_IMAGE] {
            let mut r = row(1, &tr!("Your image"));
            r.value = util::display_path(&cfg.theme_image).into();
            let readable = slint::Image::load_from_path(&util::expand_home(&cfg.theme_image)).is_ok();
            (r.hint, r.hint_kind) = if cfg.theme_image.is_empty() { (tr!("A PNG or JPEG file, like ~/Pictures/wallpaper.png").into(), 0) }
                else if readable { (tr!("Shown as your wallpaper").into(), 1) }
                else { (tr!("Can't open this picture · showing {}", wallpaper_label("Deep blue")).into(), 2) };
            rows.push((SId::ThemeImage, r));
        }
        let mut r = row(3, &tr!("Accent colour"));
        r.value = accent_label(&cfg.theme_accent).into();
        r.hint = tr!("Buttons, highlights and the focus glow").into();
        rows.push((SId::ThemeAccent, r));
        rows
    }

    /// Left or Right on a Theme row. False: not a Theme row.
    pub fn theme_change(&mut self, id: SId, dir: i32) -> bool {
        let names: Vec<&str> = ACCENTS.iter().map(|(n, _)| *n).collect();
        match id {
            SId::ThemeWallpaper => {
                let cur = self.cfg.lock().unwrap().theme_wallpaper.clone();
                let next = step(&WALLPAPERS, &cur, dir).to_string();
                self.save_theme(|c| c.theme_wallpaper = next);
            }
            SId::ThemeAccent => {
                let cur = self.cfg.lock().unwrap().theme_accent.clone();
                let next = step(&names, &cur, dir).to_string();
                self.save_theme(|c| c.theme_accent = next);
            }
            _ => return false,
        }
        true
    }

    /// The picture typed for "Your image".
    pub fn theme_image_typed(&mut self, text: &str) {
        let t = text.trim().to_string();
        self.save_theme(|c| c.theme_image = t);
    }

    fn save_theme(&mut self, f: impl FnOnce(&mut crate::config::Config)) {
        {
            let mut c = self.cfg.lock().unwrap();
            f(&mut c);
            c.save();
        }
        self.apply_theme();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_step_round_and_start_over_when_unknown() {
        assert_eq!(step(&WALLPAPERS, "Game art", 1), "Deep blue");
        assert_eq!(step(&WALLPAPERS, "Game art", -1), "Your image");
        assert_eq!(step(&WALLPAPERS, "Your image", 1), "Game art");
        assert_eq!(step(&WALLPAPERS, "Gone", 1), "Deep blue", "an unknown name counts as the first");
    }

    #[test]
    fn every_choice_has_a_label() {
        for w in WALLPAPERS { assert_eq!(wallpaper_text(w).as_deref(), Some(w)); }
        for (a, _) in ACCENTS { assert_eq!(accent_text(a).as_deref(), Some(a)); }
    }
}
