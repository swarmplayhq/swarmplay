//! UI sounds, synthesized and played through ALSA (PipeWire/PulseAudio via their ALSA plugin).
//! libasound is loaded at runtime, so there is no build dependency; without it the
//! launcher is simply silent. The device is opened only while sounds play.

use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{channel, Sender, TryRecvError};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// The interface's sounds, made in the spirit of a console's: short, soft, glassy, with a little
/// room around them. Synthesized here (no recordings).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    Move,
    Select,
    Back,
    Error,
    /// SwarmPlay starting: a warm swell.
    Start,
    /// A notification showing: a two-note chime.
    Notify,
    /// L1 / R1: another tab.
    Tab,
    /// A panel coming up (the Control Center, the overlay over a game, the notifications).
    Open,
    /// A switch turned on / off.
    ToggleOn,
    ToggleOff,
    /// A screenshot taken.
    Shutter,
    /// A game starting.
    Launch,
}

impl Sound {
    /// Every sound, in the enum's order (the cache is indexed by `as usize`).
    pub const ALL: [Sound; 12] = [
        Sound::Move, Sound::Select, Sound::Back, Sound::Error, Sound::Start, Sound::Notify,
        Sound::Tab, Sound::Open, Sound::ToggleOn, Sound::ToggleOff, Sound::Shutter, Sound::Launch,
    ];
}

static TX: OnceLock<Sender<(Sound, f32)>> = OnceLock::new();
static ENABLED: AtomicBool = AtomicBool::new(true);
/// Interface sounds' volume (an f32's bits): Low 0.5, Medium 0.8, High 1.
static LEVEL: AtomicU32 = AtomicU32::new(0x3f4c_cccd); // 0.8
/// The overlay is up over a game: a little quieter, not to fight the game's own sound.
static OVER_GAME: AtomicBool = AtomicBool::new(false);
const OVER_GAME_GAIN: f32 = 0.7;

/// The settings' levels: Off, Low, Medium, High.
pub const LEVELS: [&str; 4] = ["Off", "Low", "Medium", "High"];

/// A level (0 Off … 3 High; LEVELS' name), as Settings shows it.
pub fn level_label(level: usize) -> String {
    match LEVELS[level.min(LEVELS.len() - 1)] {
        "Off" => tr::tr!("Off"),
        "Low" => tr::tr!("Low"),
        "Medium" => tr::tr!("Medium"),
        _ => tr::tr!("High"),
    }
}
const GAINS: [f32; 4] = [0.0, 0.5, 0.8, 1.0];

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// 1 Low, 2 Medium, 3 High (0 or more than 3: Medium).
pub fn set_level(level: u8) {
    let g = GAINS.get(level as usize).copied().filter(|g| *g > 0.0).unwrap_or(0.8);
    LEVEL.store(g.to_bits(), Ordering::Relaxed);
}

pub fn set_over_game(on: bool) {
    OVER_GAME.store(on, Ordering::Relaxed);
}

pub fn play(s: Sound) {
    if !ENABLED.load(Ordering::Relaxed) { return; }
    let mut gain = f32::from_bits(LEVEL.load(Ordering::Relaxed));
    if OVER_GAME.load(Ordering::Relaxed) { gain *= OVER_GAME_GAIN; }
    if let Some(tx) = TX.get() {
        let _ = tx.send((s, gain));
    }
}

const RATE: u32 = 48_000;

type Open = unsafe extern "C" fn(*mut *mut c_void, *const c_char, c_int, c_int) -> c_int;
type SetParams = unsafe extern "C" fn(*mut c_void, c_int, c_int, c_uint, c_uint, c_int, c_uint) -> c_int;
type WriteI = unsafe extern "C" fn(*mut c_void, *const c_void, c_ulong) -> c_long;
type Recover = unsafe extern "C" fn(*mut c_void, c_int, c_int) -> c_int;
type Drain = unsafe extern "C" fn(*mut c_void) -> c_int;
type Close = unsafe extern "C" fn(*mut c_void) -> c_int;

struct Alsa {
    _lib: libloading::Library,
    open: Open,
    set_params: SetParams,
    writei: WriteI,
    recover: Recover,
    drain: Drain,
    close: Close,
}

impl Alsa {
    fn load() -> Option<Alsa> {
        unsafe {
            let lib = libloading::Library::new("libasound.so.2").ok()?;
            Some(Alsa {
                open: *lib.get(b"snd_pcm_open\0").ok()?,
                set_params: *lib.get(b"snd_pcm_set_params\0").ok()?,
                writei: *lib.get(b"snd_pcm_writei\0").ok()?,
                recover: *lib.get(b"snd_pcm_recover\0").ok()?,
                drain: *lib.get(b"snd_pcm_drain\0").ok()?,
                close: *lib.get(b"snd_pcm_close\0").ok()?,
                _lib: lib,
            })
        }
    }

    fn open_pcm(&self) -> Option<*mut c_void> {
        let mut pcm: *mut c_void = std::ptr::null_mut();
        unsafe {
            if (self.open)(&mut pcm, c"default".as_ptr(), 0, 0) < 0 {
                return None;
            }
            // S16_LE = 2, RW_INTERLEAVED = 3, mono, 48 kHz, 25 ms latency.
            if (self.set_params)(pcm, 2, 3, 1, RATE, 1, 25_000) < 0 {
                (self.close)(pcm);
                return None;
            }
        }
        Some(pcm)
    }

    fn write(&self, pcm: *mut c_void, samples: &[i16]) {
        let mut off = 0usize;
        while off < samples.len() {
            let n = unsafe { (self.writei)(pcm, samples[off..].as_ptr() as *const c_void, (samples.len() - off) as c_ulong) };
            if n < 0 {
                if unsafe { (self.recover)(pcm, n as c_int, 1) } < 0 {
                    return;
                }
            } else {
                off += n as usize;
            }
        }
    }
}

// ---------------------------------------------------------------- synthesis

const SR: f32 = RATE as f32;

/// A sine gliding from f1 to f2 (exponentially, over `glide` s), with a linear attack and an
/// exponential fade of time constant `tau`, starting at `t0` s.
#[allow(clippy::too_many_arguments)]
fn partial(out: &mut Vec<f32>, t0: f32, f1: f32, f2: f32, glide: f32, attack: f32, tau: f32, amp: f32) {
    let start = (t0 * SR) as usize;
    let len = ((attack + tau * 9.0) * SR) as usize;
    if out.len() < start + len { out.resize(start + len, 0.0); }
    let glide = glide.max(0.002);
    let mut phase = 0.0f32;
    for i in 0..len {
        let t = i as f32 / SR;
        let f = if t < glide { f1 * (f2 / f1).powf(t / glide) } else { f2 };
        phase = (phase + f / SR).fract();
        let env = if t < attack { t / attack } else { (-(t - attack) / tau).exp() };
        out[start + i] += (phase * std::f32::consts::TAU).sin() * env * amp;
    }
}

#[derive(Clone, Copy)]
enum Filter { High, Band, Low }

/// A breath of filtered noise (the tick in a click, the air in a swell): the filter's cutoff
/// glides f1 → f2 over `len` s; a linear attack, then an exponential fade to silence.
#[allow(clippy::too_many_arguments)]
fn noise(out: &mut Vec<f32>, t0: f32, len: f32, amp: f32, kind: Filter, f1: f32, f2: f32, attack: f32, seed: u32) {
    let start = (t0 * SR) as usize;
    let n = (len * SR) as usize;
    if out.len() < start + n { out.resize(start + n, 0.0); }
    let mut x = seed | 1;
    let (mut x1, mut x2, mut y1, mut y2) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let mut c = [0.0f32; 5];
    for i in 0..n {
        let t = i as f32 / SR;
        if i % 32 == 0 {
            // RBJ biquad, recomputed as the cutoff moves.
            let f = f1 * (f2 / f1).powf(t / len);
            let q = if matches!(kind, Filter::Band) { 1.4 } else { 0.707 };
            let w = std::f32::consts::TAU * f.min(SR * 0.45) / SR;
            let (sn, cs) = w.sin_cos();
            let alpha = sn / (2.0 * q);
            let (b0, b1, b2) = match kind {
                Filter::High => ((1.0 + cs) / 2.0, -(1.0 + cs), (1.0 + cs) / 2.0),
                Filter::Low => ((1.0 - cs) / 2.0, 1.0 - cs, (1.0 - cs) / 2.0),
                Filter::Band => (alpha, 0.0, -alpha),
            };
            let a0 = 1.0 + alpha;
            c = [b0 / a0, b1 / a0, b2 / a0, -2.0 * cs / a0, (1.0 - alpha) / a0];
        }
        x ^= x << 13; x ^= x >> 17; x ^= x << 5;
        let w = (x as f32 / u32::MAX as f32) * 2.0 - 1.0;
        let y = c[0] * w + c[1] * x1 + c[2] * x2 - c[3] * y1 - c[4] * y2;
        (x2, x1, y2, y1) = (x1, w, y1, y);
        let env = if t < attack { t / attack } else { amp * (0.0001f32 / amp).powf((t - attack) / (len - attack).max(1e-4)) / amp };
        out[start + i] += y * env * amp;
    }
}

/// A small bell: its note and two overtones fading faster.
fn bell(out: &mut Vec<f32>, t: f32, f: f32, amp: f32, tau: f32) {
    partial(out, t, f, f, 0.0, 0.0015, tau, amp);
    partial(out, t, f * 2.0, f * 2.0, 0.0, 0.0015, tau * 0.45, amp * 0.28);
    partial(out, t, f * 3.01, f * 3.01, 0.0, 0.0015, tau * 0.22, amp * 0.1);
}

/// The room around a sound: four damped combs and two all-passes (as Freeverb), mixed in at
/// `wet`, ringing out over about `secs`.
fn room(dry: Vec<f32>, wet: f32, secs: f32) -> Vec<f32> {
    let tail = (secs * SR) as usize;
    let n = dry.len() + tail;
    let scale = SR / 44_100.0;
    let mut acc = vec![0.0f32; n];
    for d in [1116usize, 1188, 1277, 1356] {
        let d = (d as f32 * scale) as usize;
        // Down 60 dB over `secs`.
        let g = 10f32.powf(-3.0 * d as f32 / SR / secs.max(0.05));
        let mut line = vec![0.0f32; d];
        let (mut k, mut lp) = (0usize, 0.0f32);
        for i in 0..n {
            let x = dry.get(i).copied().unwrap_or(0.0);
            let y = line[k];
            lp = y * 0.7 + lp * 0.3;
            line[k] = x + lp * g;
            acc[i] += y;
            k = (k + 1) % d;
        }
    }
    for d in [556usize, 441] {
        let d = (d as f32 * scale) as usize;
        let mut line = vec![0.0f32; d];
        let mut k = 0usize;
        for v in acc.iter_mut() {
            let b = line[k];
            line[k] = *v + b * 0.5;
            *v = b - *v * 0.5;
            k = (k + 1) % d;
        }
    }
    let mut out = dry;
    out.resize(n, 0.0);
    for (o, r) in out.iter_mut().zip(&acc) { *o += r * wet * 0.35; }
    out
}

/// What's left once it's inaudible goes; the very end fades, so nothing clicks.
fn finish(mut v: Vec<f32>) -> Vec<f32> {
    // Inaudible: 50 dB under its loudest.
    let floor = v.iter().fold(0.0f32, |m, x| m.max(x.abs())) * 0.003;
    let end = v.iter().rposition(|x| x.abs() > floor).map_or(0, |i| i + 1);
    v.truncate(end);
    let fade = (0.006 * SR) as usize;
    let n = v.len();
    for i in 0..fade.min(n) {
        v[n - 1 - i] *= i as f32 / fade as f32;
    }
    v
}

/// One sound, at full level (the mixer turns it down to Interface sounds' level).
pub fn render(s: Sound) -> Vec<f32> {
    let mut b = Vec::new();
    let (wet, secs) = match s {
        Sound::Move => {
            partial(&mut b, 0.0, 2350.0, 2050.0, 0.01, 0.0006, 0.010, 0.13);
            partial(&mut b, 0.0, 4700.0, 4500.0, 0.01, 0.0006, 0.004, 0.03);
            noise(&mut b, 0.0, 0.006, 0.025, Filter::High, 3500.0, 3500.0, 0.0005, 7);
            (0.16, 0.5)
        }
        Sound::Tab => {
            partial(&mut b, 0.0, 1650.0, 1480.0, 0.012, 0.0008, 0.016, 0.14);
            partial(&mut b, 0.035, 2200.0, 1980.0, 0.012, 0.0008, 0.012, 0.07);
            noise(&mut b, 0.0, 0.008, 0.02, Filter::Band, 2600.0, 2600.0, 0.0005, 11);
            (0.18, 0.5)
        }
        Sound::Select => {
            partial(&mut b, 0.0, 1046.0, 1568.0, 0.028, 0.002, 0.065, 0.16);
            partial(&mut b, 0.0, 2093.0, 3136.0, 0.028, 0.002, 0.03, 0.045);
            partial(&mut b, 0.0, 523.0, 784.0, 0.028, 0.002, 0.035, 0.05);
            (0.24, 0.8)
        }
        Sound::Back => {
            partial(&mut b, 0.0, 1318.0, 880.0, 0.04, 0.002, 0.05, 0.14);
            partial(&mut b, 0.0, 2637.0, 1760.0, 0.04, 0.002, 0.022, 0.03);
            (0.2, 0.7)
        }
        Sound::Open => {
            noise(&mut b, 0.0, 0.26, 0.05, Filter::Band, 500.0, 2600.0, 0.12, 23);
            partial(&mut b, 0.04, 784.0, 1175.0, 0.18, 0.06, 0.12, 0.09);
            partial(&mut b, 0.1, 1568.0, 2349.0, 0.16, 0.05, 0.08, 0.03);
            (0.32, 0.9)
        }
        Sound::ToggleOn | Sound::ToggleOff => {
            let (a, z) = if s == Sound::ToggleOn { (1760.0, 2349.0) } else { (2349.0, 1760.0) };
            partial(&mut b, 0.0, a, a, 0.0, 0.0008, 0.016, 0.11);
            partial(&mut b, 0.05, z, z, 0.0, 0.0008, 0.02, 0.11);
            (0.16, 0.5)
        }
        Sound::Error => {
            partial(&mut b, 0.0, 392.0, 330.0, 0.05, 0.002, 0.045, 0.2);
            partial(&mut b, 0.0, 784.0, 660.0, 0.05, 0.002, 0.02, 0.05);
            partial(&mut b, 0.11, 349.0, 294.0, 0.05, 0.002, 0.05, 0.18);
            partial(&mut b, 0.11, 698.0, 588.0, 0.05, 0.002, 0.02, 0.045);
            (0.18, 0.6)
        }
        Sound::Notify => {
            bell(&mut b, 0.0, 1318.5, 0.1, 0.32);
            bell(&mut b, 0.11, 1975.5, 0.09, 0.42);
            (0.3, 1.2)
        }
        Sound::Shutter => {
            noise(&mut b, 0.0, 0.03, 0.16, Filter::Band, 3200.0, 3200.0, 0.0005, 31);
            noise(&mut b, 0.07, 0.045, 0.12, Filter::Band, 2400.0, 2400.0, 0.0005, 37);
            partial(&mut b, 0.07, 1200.0, 900.0, 0.03, 0.001, 0.012, 0.05);
            (0.14, 0.4)
        }
        Sound::Launch => {
            noise(&mut b, 0.0, 0.5, 0.05, Filter::Low, 400.0, 5000.0, 0.35, 41);
            for (i, f) in [440.0f32, 554.4, 659.3].into_iter().enumerate() {
                partial(&mut b, 0.25 + i as f32 * 0.05, f, f * 2.0, 0.3, 0.12, 0.35, 0.06);
            }
            bell(&mut b, 0.55, 1760.0, 0.05, 0.5);
            (0.36, 1.6)
        }
        Sound::Start => {
            for (i, f) in [220.0f32, 329.6, 440.0, 554.4, 659.3].into_iter().enumerate() {
                let t = i as f32 * 0.06;
                partial(&mut b, t, f * 0.9985, f * 0.9985, 0.0, 0.5, 0.5, 0.045);
                partial(&mut b, t, f * 1.0015, f * 1.0015, 0.0, 0.5, 0.5, 0.045);
            }
            bell(&mut b, 0.35, 1760.0, 0.03, 0.6);
            bell(&mut b, 0.5, 2637.0, 0.022, 0.6);
            (0.4, 2.2)
        }
    };
    // Under the old sounds' loudness: a console's menus stay quiet next to its games.
    finish(room(b, wet, secs)).into_iter().map(|x| x * HEADROOM).collect()
}

/// All sounds turned down together (Medium then plays them a little below the old ones).
const HEADROOM: f32 = 0.5;

// ---------------------------------------------------------------- mixing

/// A sound playing: which, how far in, how loud.
#[derive(Clone, Copy, Debug)]
struct Voice {
    s: usize,
    pos: usize,
    gain: f32,
}

const MAX_VOICES: usize = 8;
/// 5 ms at a time: a new sound starts within a block (plus the device's 25 ms).
const BLOCK: usize = 240;

/// Loud moments rounded off instead of clipped: untouched up to 0.6, then a soft knee to 1.
fn limit(x: f32) -> f32 {
    let a = x.abs();
    if a <= 0.6 { x } else { x.signum() * (0.6 + 0.4 * ((a - 0.6) / 0.4).tanh()) }
}

/// The next block of every voice playing, summed; finished voices go.
fn mix(voices: &mut Vec<Voice>, cache: &[Vec<f32>], out: &mut [f32]) {
    out.iter_mut().for_each(|x| *x = 0.0);
    for v in voices.iter_mut() {
        let src = &cache[v.s];
        let n = out.len().min(src.len().saturating_sub(v.pos));
        for (o, x) in out[..n].iter_mut().zip(&src[v.pos..v.pos + n]) { *o += x * v.gain; }
        v.pos += n;
    }
    voices.retain(|v| v.pos < cache[v.s].len());
    out.iter_mut().for_each(|x| *x = limit(*x));
}

/// A sound asked for: a voice, unless it's a Move right on another's heels (a held d-pad).
fn start(voices: &mut Vec<Voice>, s: Sound, gain: f32, last_move: &mut Instant) {
    if s == Sound::Move {
        if last_move.elapsed() < Duration::from_millis(30) { return; }
        *last_move = Instant::now();
    }
    if voices.len() >= MAX_VOICES { voices.remove(0); }
    voices.push(Voice { s: s as usize, pos: 0, gain });
}

/// `swarmplay --sounds`: every sound in turn, to hear them on this machine.
pub fn demo() {
    init();
    for s in Sound::ALL {
        println!("{s:?}");
        play(s);
        std::thread::sleep(Duration::from_secs_f32(render(s).len() as f32 / SR + 0.5));
    }
    std::thread::sleep(Duration::from_millis(300));
}

pub fn init() {
    let (tx, rx) = channel::<(Sound, f32)>();
    if TX.set(tx).is_err() {
        return;
    }
    std::thread::Builder::new()
        .name("audio".into())
        .spawn(move || {
            let Some(alsa) = Alsa::load() else {
                crate::log!("audio: libasound not available, sounds disabled");
                return;
            };
            let cache: Vec<Vec<f32>> = Sound::ALL.into_iter().map(render).collect();
            let mut pcm: Option<*mut c_void> = None;
            let mut voices: Vec<Voice> = Vec::new();
            let mut last_move = Instant::now() - Duration::from_secs(1);
            let mut quiet_since = Instant::now();
            let mut block = vec![0.0f32; BLOCK];
            let mut pcm16 = vec![0i16; BLOCK];
            loop {
                // Nothing playing and the device let go: wait for the next sound.
                if pcm.is_none() {
                    match rx.recv() {
                        Ok((s, g)) => start(&mut voices, s, g, &mut last_move),
                        Err(_) => return,
                    }
                }
                loop {
                    match rx.try_recv() {
                        Ok((s, g)) => start(&mut voices, s, g, &mut last_move),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => return,
                    }
                }
                if voices.is_empty() {
                    // Kept open a little (no start-up delay for the next one), then let go.
                    if quiet_since.elapsed() > Duration::from_secs(3) {
                        if let Some(p) = pcm.take() {
                            unsafe { (alsa.drain)(p); (alsa.close)(p); }
                        }
                        continue;
                    }
                } else {
                    quiet_since = Instant::now();
                }
                if pcm.is_none() {
                    pcm = alsa.open_pcm();
                    if pcm.is_none() { voices.clear(); continue; }
                }
                mix(&mut voices, &cache, &mut block);
                for (o, x) in pcm16.iter_mut().zip(&block) { *o = (x * i16::MAX as f32) as i16; }
                // Blocks until the device has room: this paces the loop.
                if let Some(p) = pcm { alsa.write(p, &pcm16); }
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sound_is_clean() {
        for s in Sound::ALL {
            let v = render(s);
            assert!(!v.is_empty(), "{s:?} is empty");
            assert!(v.iter().all(|x| x.is_finite()), "{s:?} has NaN");
            let peak = v.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            assert!(peak > 0.02 && peak < 0.3, "{s:?} peaks at {peak}");
            // Ends in silence: no click when it stops.
            assert!(v.last().unwrap().abs() < 1e-3, "{s:?} ends at {}", v.last().unwrap());
            // Clicks are short, the start and a game's launch long.
            let secs = v.len() as f32 / SR;
            assert!(secs < 4.0, "{s:?} lasts {secs}");
        }
        assert!(render(Sound::Move).len() < render(Sound::Select).len());
        assert!(render(Sound::Select).len() < render(Sound::Start).len());
    }

    #[test]
    fn every_level_has_a_label() {
        for (i, l) in LEVELS.iter().enumerate() { assert_eq!(level_label(i), *l); }
    }

    #[test]
    fn the_cache_has_every_sound_in_order() {
        for (i, s) in Sound::ALL.iter().enumerate() { assert_eq!(*s as usize, i); }
    }

    #[test]
    fn voices_overlap_and_end() {
        let cache = vec![vec![0.1f32; 300], vec![0.2f32; 100]];
        let mut voices = vec![Voice { s: 0, pos: 0, gain: 1.0 }, Voice { s: 1, pos: 0, gain: 0.5 }];
        let mut out = vec![0.0f32; BLOCK];
        mix(&mut voices, &cache, &mut out);
        assert!((out[0] - 0.2).abs() < 1e-6);
        assert!((out[150] - 0.1).abs() < 1e-6);
        // The short one is done; the long one carries on.
        assert_eq!(voices.len(), 1);
        mix(&mut voices, &cache, &mut out);
        assert!((out[0] - 0.1).abs() < 1e-6 && out[100] == 0.0);
        assert!(voices.is_empty());
    }

    #[test]
    fn loud_moments_are_rounded_off() {
        assert_eq!(limit(0.3), 0.3);
        assert!(limit(3.0) < 1.0 && limit(3.0) > 0.95);
        assert!(limit(-3.0) > -1.0);
        assert!(limit(0.8) < 0.8 && limit(0.8) > 0.6);
    }

    #[test]
    fn a_held_dpad_doesnt_rattle_and_voices_are_capped() {
        let mut v = Vec::new();
        let mut last = Instant::now() - Duration::from_secs(1);
        start(&mut v, Sound::Move, 1.0, &mut last);
        start(&mut v, Sound::Move, 1.0, &mut last);
        assert_eq!(v.len(), 1);
        for _ in 0..20 { start(&mut v, Sound::Select, 1.0, &mut last); }
        assert_eq!(v.len(), MAX_VOICES);
    }
}
