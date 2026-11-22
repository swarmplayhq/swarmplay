//! Who's playing: the start screen asks, as a console's ("Who's using this controller?"), with
//! everyone who plays here and "Add user". Each profile has its own name, avatar, theme and
//! language (config::Profile); games, downloads and the other settings are everyone's.
//! Settings → Launcher has Switch user (back to that screen) and Remove this user.

use crate::app::*;
use crate::audio::{self, Sound};
use crate::ProfileData;
use tr::tr;

impl App {
    /// The start screen's row: "Add user", then everyone. `boot.pick` 0 is Add user.
    pub fn push_boot_profiles(&mut self) {
        let cfg = self.cfg.lock().unwrap().clone();
        let rows: Vec<ProfileData> = cfg.profiles.iter().map(|p| {
            let name = if p.user_name.trim().is_empty() { crate::app::user_name() } else { p.user_name.clone() };
            let picture = crate::avatar::load(&p.avatar_image);
            let tint = crate::theme::accent_color(&p.avatar_color)
                .or_else(|| crate::theme::accent_color(&p.theme_accent))
                .unwrap_or_else(|| slint::Color::from_rgb_u8(0x0a, 0x6c, 0xff));
            ProfileData { initial: crate::app::user_initial(&name).into(), name: name.into(), tint, has_picture: picture.is_some(), picture: picture.unwrap_or_default() }
        }).collect();
        let ui = self.ui();
        ui.set_boot_profiles(model(rows));
        ui.set_boot_pick(self.boot.pick);
        ui.set_boot_chosen(self.boot.chosen);
    }

    /// Left and Right along the row, ✕ on someone (or on Add user).
    pub fn act_boot_profiles(&mut self, a: Act) {
        if self.boot.chosen {
            return;
        }
        let n = self.cfg.lock().unwrap().profiles.len() as i32;
        match a {
            Act::Left if self.boot.pick > 0 => self.boot.pick -= 1,
            Act::Right if self.boot.pick < n => self.boot.pick += 1,
            Act::Confirm => return self.boot_choose(self.boot.pick),
            _ => return,
        }
        audio::play(Sound::Move);
        self.ui().set_boot_pick(self.boot.pick);
    }

    pub fn click_boot_profile(&mut self, i: i32) {
        if i < 0 || self.boot.chosen {
            return;
        }
        if i != self.boot.pick {
            self.boot.pick = i;
            self.ui().set_boot_pick(i);
        }
        self.boot_choose(i);
    }

    /// Someone chose themself (or Add user).
    fn boot_choose(&mut self, pick: i32) {
        if pick == 0 {
            return self.add_user();
        }
        let i = (pick - 1) as usize;
        let changed = {
            let mut c = self.cfg.lock().unwrap();
            let changed = c.profile != i;
            if changed {
                c.use_profile(i);
                c.save();
            }
            changed
        };
        if changed {
            self.profile_applied();
        }
        audio::play(Sound::Start);
        self.boot.chosen = true;
        self.ui().set_boot_chosen(true);
        // Ready (it usually is by the time someone chooses): straight in; else as soon as it is.
        if self.boot.ready {
            self.boot_finish();
        } else {
            self.boot_render();
        }
    }

    /// Their name, avatar, theme and language, everywhere.
    pub fn profile_applied(&mut self) {
        let lang = self.cfg.lock().unwrap().language.clone();
        crate::i18n::set_lang(crate::i18n::Lang::from_config(&lang));
        self.apply_theme();
        self.relocalize();
        if self.boot.active {
            self.push_boot_profiles();
            self.boot_render();
        }
    }

    /// Add user: a new profile, set up with the short setup (name and avatar, language, look).
    fn add_user(&mut self) {
        let n = self.cfg.lock().unwrap().profiles.len() + 1;
        self.boot.came_from = Some(self.cfg.lock().unwrap().profile);
        {
            let mut c = self.cfg.lock().unwrap();
            c.add_profile(&tr!("Player {}", n));
            c.save();
        }
        self.profile_applied();
        self.onboard_open_new_user();
    }

    /// The short setup for a new user ended: back to who's playing, on them (`kept`), or on whoever
    /// was there before (left with Back: the new profile goes).
    pub fn new_user_done(&mut self, kept: bool) {
        let before = self.boot.came_from.take().unwrap_or(0);
        {
            let mut c = self.cfg.lock().unwrap();
            if !kept {
                let i = c.profile;
                c.remove_profile(i);
                let back = before.min(c.profiles.len() - 1);
                c.use_profile(back);
            }
            c.save();
        }
        self.profile_applied();
        let i = self.cfg.lock().unwrap().profile as i32;
        self.boot.pick = i + 1;
        if self.boot.active {
            self.ui().set_overlay(crate::boot::OVERLAY_BOOT);
            self.push_boot_profiles();
        }
    }

    /// Settings → Switch user: back to the start screen's question.
    pub fn switch_user(&mut self) {
        for _ in 0..12 {
            if self.overlay == Overlay::None { break; }
            self.back();
        }
        self.boot_start(false);
    }

    /// Settings → Remove this user: pressed twice (the first says so), then whoever is first plays
    /// and the start screen asks who's playing.
    pub fn remove_user_pressed(&mut self) {
        let now = crate::util::now_secs();
        if self.remove_armed.is_some_and(|t| now - t < 6.0) {
            self.remove_armed = None;
            {
                let mut c = self.cfg.lock().unwrap();
                let i = c.profile;
                c.remove_profile(i);
                c.save();
            }
            audio::play(Sound::Back);
            self.profile_applied();
            self.switch_user();
        } else {
            self.remove_armed = Some(now);
            audio::play(Sound::Error);
            let name = self.player_name();
            self.note_setting(crate::settings::SId::RemoveUser, &tr!("Press again to remove {} · their games stay", name), 2);
        }
    }
}
