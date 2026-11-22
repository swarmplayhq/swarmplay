//! Sound, in Settings → Sound: the system's volume, mute and where sound goes (PipeWire or
//! PulseAudio, through pactl), with the launcher's own interface sounds. Read in the background
//! while Settings is open (it can change from elsewhere); changed at once.

use crate::app::*;
use crate::os::system::{self, Output};
use crate::settings::SId;
use crate::SettingData;
use std::process::Command;
use tr::tr;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    /// The default output's volume, 0–150 (None: no pactl, or no output).
    pub volume: Option<u8>,
    pub muted: bool,
    pub outputs: Vec<Output>,
    pub output: Option<usize>,
}

fn pactl(args: &[&str]) -> Option<String> {
    let out = Command::new("pactl").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// "Volume: front-left: 64001 /  98% / -0.62 dB, ..." → 98.
fn parse_volume(text: &str) -> Option<u8> {
    let pct = text.split('%').next()?.rsplit(|c: char| !c.is_ascii_digit()).next()?;
    pct.parse::<u16>().ok().map(|v| v.min(150) as u8)
}

pub fn read() -> State {
    let volume = pactl(&["get-sink-volume", "@DEFAULT_SINK@"]).and_then(|t| parse_volume(&t));
    let muted = pactl(&["get-sink-mute", "@DEFAULT_SINK@"]).is_some_and(|t| t.contains("yes"));
    let (outputs, output) = system::outputs();
    State { volume, muted, outputs, output }
}

fn set_volume(percent: u8) -> Result<(), String> {
    pactl(&["set-sink-volume", "@DEFAULT_SINK@", &format!("{percent}%")]).map(|_| ()).ok_or_else(|| "pactl failed".into())
}

fn set_mute(on: bool) -> Result<(), String> {
    pactl(&["set-sink-mute", "@DEFAULT_SINK@", if on { "1" } else { "0" }]).map(|_| ()).ok_or_else(|| "pactl failed".into())
}

impl App {
    /// Read the sound state in the background (every 3 s at most); Settings follows.
    pub fn sound_poll(&mut self) {
        let now = crate::util::now_secs();
        if self.sound_polling || now - self.sound_polled < 3.0 { return; }
        self.sound_polling = true;
        std::thread::spawn(|| {
            let state = read();
            let _ = slint::invoke_from_event_loop(move || with_app(move |app| {
                app.sound_polling = false;
                app.sound_polled = crate::util::now_secs();
                if app.sound != state {
                    app.sound = state;
                    if app.overlay == Overlay::Settings && app.edit_index < 0 { app.refresh_settings(); }
                }
            }));
        });
    }

    pub fn sound_rows(&self, sounds_on: bool) -> Vec<(SId, SettingData)> {
        let row = |kind: i32, label: &str| SettingData { kind, label: label.into(), ..Default::default() };
        let s = &self.sound;
        let mut rows = Vec::new();
        if let Some(v) = s.volume {
            let mut r = row(3, &tr!("Volume"));
            r.value = if s.muted { tr!("Muted").into() } else { format!("{v}%").into() };
            r.has_bar = true;
            r.bar = if s.muted { 0.0 } else { (v as f32 / 100.0).min(1.0) };
            r.hint = tr!("The whole system's · ◂ ▸ change it").into();
            rows.push((SId::SoundVolume, r));
            let mut r = row(2, &tr!("Mute"));
            r.on = s.muted;
            rows.push((SId::SoundMute, r));
        }
        if !s.outputs.is_empty() {
            let mut r = row(3, &tr!("Sound output"));
            r.value = s.output.and_then(|i| s.outputs.get(i)).map(|o| o.label.clone()).unwrap_or_else(|| tr!("Choose")).into();
            rows.push((SId::SoundOut, r));
        }
        let level = self.cfg.lock().unwrap().sounds_level.clamp(1, 3);
        let mut r = row(3, &tr!("Interface sounds"));
        r.value = crate::audio::level_label(if sounds_on { level as usize } else { 0 }).into();
        r.hint = tr!("The launcher's clicks and chimes · a little quieter over a game").into();
        rows.push((SId::Sounds, r));
        rows
    }

    /// Left/right (or a press) on a Sound row. False: not one.
    pub fn sound_change(&mut self, id: SId, dir: i32) -> bool {
        let result = match id {
            SId::SoundVolume => {
                let Some(v) = self.sound.volume else { return true };
                let next = (v as i32 + dir.signum() * 5).clamp(0, 100) as u8;
                // Turning it up brings the sound back.
                if self.sound.muted && dir > 0 { let _ = set_mute(false); self.sound.muted = false; }
                set_volume(next).map(|_| self.sound.volume = Some(next))
            }
            SId::SoundMute => {
                let on = dir > 0;
                set_mute(on).map(|_| self.sound.muted = on)
            }
            SId::Sounds => {
                // Off, Low, Medium, High: heard at the new level.
                let (on, level) = { let c = self.cfg.lock().unwrap(); (c.sounds, c.sounds_level.clamp(1, 3)) };
                let now = if on { level as i32 } else { 0 };
                // ◂ ▸ stop at the ends; a press (2) goes round.
                let next = if dir == 2 { (now + 1) % 4 } else { (now + dir.signum()).clamp(0, 3) } as u8;
                {
                    let mut c = self.cfg.lock().unwrap();
                    c.sounds = next > 0;
                    if next > 0 { c.sounds_level = next; }
                    c.save();
                }
                crate::audio::set_enabled(next > 0);
                if next > 0 { crate::audio::set_level(next); }
                crate::audio::play(crate::audio::Sound::Select);
                self.refresh_settings();
                return true;
            }
            SId::SoundOut if !self.sound.outputs.is_empty() => {
                let n = self.sound.outputs.len() as i32;
                let next = self.sound.output.map_or(0, |i| i as i32 + dir).rem_euclid(n) as usize;
                system::set_output(&self.sound.outputs[next].name).map(|_| self.sound.output = Some(next))
            }
            _ => return false,
        };
        if let Err(e) = result {
            self.note_setting(id, &tr!("Didn't change: {error}", error = e), 2);
        }
        // Heard at the new volume (Mute: a switch's click).
        crate::audio::play(match id {
            SId::SoundMute if dir > 0 => crate::audio::Sound::ToggleOn,
            SId::SoundMute => crate::audio::Sound::ToggleOff,
            _ => crate::audio::Sound::Move,
        });
        self.refresh_settings();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::parse_volume;

    #[test]
    fn reads_the_volume_pactl_prints() {
        assert_eq!(parse_volume("Volume: front-left: 64001 /  98% / -0.62 dB,   front-right: 64001 /  98% / -0.62 dB"), Some(98));
        assert_eq!(parse_volume("Volume: mono: 0 /   0% / -inf dB"), Some(0));
        assert_eq!(parse_volume("nothing"), None);
    }
}
