//! Avatars: the picture on someone's profile, chosen as on a console from a grid: their initial
//! on a colour, the system account's picture, a picture of their own (the folder picker, files
//! shown), or the art of the games here. What's chosen is cut to a square, made small and kept
//! in ~/.config/azrak/avatars/, so it never depends on a cache or a file that moves.

use crate::app::*;
use crate::audio::{self, Sound};
use crate::AvatarChoice;
use slint::{Image, Model, VecModel};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use tr::tr;

/// Avatars are kept this big (they show at most ~200 px).
const SIDE: u32 = 256;
/// Columns of the grid (the window lays them out this many to a row).
pub const COLS: usize = 7;
/// Games offered, at most.
const GAMES: usize = 40;

/// What a square of the grid is.
#[derive(Clone, Debug, PartialEq)]
enum Pick {
    Initial,
    Account(PathBuf),
    OwnPicture,
    Game(String),
}

pub struct AvatarPick {
    picks: Vec<Pick>,
    model: Rc<VecModel<AvatarChoice>>,
}

fn dir() -> PathBuf {
    crate::util::config_dir().join("avatars")
}

/// The system account's picture (GNOME's, KDE's), when there's one.
fn account_picture() -> Option<PathBuf> {
    let user = std::env::var("USER").unwrap_or_default();
    [crate::util::expand_home("~/.face"), crate::util::expand_home("~/.face.icon"), PathBuf::from(format!("/var/lib/AccountsService/icons/{user}"))]
        .into_iter()
        .find(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() > 0))
}

/// The middle square of `img`, `SIDE` wide.
fn square(img: image::RgbaImage) -> image::RgbaImage {
    let (w, h) = img.dimensions();
    let s = w.min(h).max(1);
    let cut = image::imageops::crop_imm(&img, (w - s) / 2, (h - s) / 2, s, s).to_image();
    image::imageops::resize(&cut, SIDE, SIDE, image::imageops::FilterType::Lanczos3)
}

/// Keep `img` as an avatar: its file's path.
fn keep(img: image::RgbaImage) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    let name = format!("{}.png", (crate::util::now_secs() * 1000.0) as u64);
    let path = dir().join(name);
    square(img).save(&path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// An avatar from a picture on disk (PNG, JPEG, WebP).
pub fn from_file(path: &Path) -> Result<PathBuf, String> {
    let img = image::open(path).map_err(|e| e.to_string())?.to_rgba8();
    keep(img)
}

/// An avatar from a picture already on screen (a game's art).
fn from_image(img: &Image) -> Result<PathBuf, String> {
    let buf = img.to_rgba8().ok_or_else(|| tr!("the picture isn't ready yet"))?;
    let rgba = image::RgbaImage::from_raw(buf.width(), buf.height(), buf.as_bytes().to_vec()).ok_or_else(|| tr!("the picture couldn't be read"))?;
    keep(rgba)
}

/// A picture file the avatar picker can use.
pub fn is_picture(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "webp"))
}

/// An avatar's picture, to show.
pub fn load(path: &str) -> Option<Image> {
    if path.is_empty() {
        return None;
    }
    Image::load_from_path(Path::new(path)).ok()
}

/// Avatar files no profile uses any more go (a new pick replaces the old file).
fn tidy(cfg: &crate::config::Config) {
    let used: Vec<String> = cfg.profiles.iter().map(|p| p.avatar_image.clone()).chain(std::iter::once(cfg.avatar_image.clone())).collect();
    for e in std::fs::read_dir(dir()).into_iter().flatten().flatten() {
        let p = e.path().to_string_lossy().into_owned();
        if !used.contains(&p) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

impl App {
    /// The grid of avatars to choose from.
    pub fn open_avatar_pick(&mut self) {
        audio::play(Sound::Select);
        let mut picks = vec![Pick::Initial];
        if let Some(p) = account_picture() {
            picks.push(Pick::Account(p));
        }
        picks.push(Pick::OwnPicture);
        // The games here: installed ones first, then the Library's.
        let mut reqs: Vec<(String, crate::images::Src, u32, f32)> = Vec::new();
        let items = (0..self.locals.len()).map(RowItem::Local).chain(self.filtered.iter().copied().map(RowItem::Cat));
        for item in items {
            if reqs.len() >= GAMES { break; }
            if let Some(r) = self.tile_req(item) {
                if !reqs.iter().any(|(k, ..)| *k == r.key) {
                    reqs.push((r.key, r.src, r.w, r.crop));
                }
            }
        }
        for (key, src, w, crop) in &reqs {
            self.images.want(key, src.clone(), *w, *crop, crate::images::prio::TILE);
            picks.push(Pick::Game(key.clone()));
        }
        self.avatar_pick = Some(AvatarPick { picks, model: Rc::new(VecModel::default()) });
        self.push_overlay(Overlay::Avatar, Z_AVATAR, 0);
        self.push_avatar_pick();
        // On the one in use, when it's the initial.
        let cur = self.cfg.lock().unwrap().avatar_image.is_empty();
        self.set_focus(Z_AVATAR, if cur { 0 } else { -1 }.max(0));
    }

    fn push_avatar_pick(&mut self) {
        let Some(p) = self.avatar_pick.as_ref() else { return };
        let name = self.player_name();
        let initial = crate::app::user_initial(&name);
        let rows: Vec<AvatarChoice> = p.picks.iter().map(|k| {
            let (label, image, kind) = match k {
                Pick::Initial => (tr!("Your initial"), None, 0),
                Pick::Account(path) => (tr!("Account picture"), Image::load_from_path(path).ok(), 1),
                Pick::OwnPicture => (tr!("A picture of yours"), None, 2),
                Pick::Game(key) => (String::new(), self.images.get(key), 3),
            };
            AvatarChoice { kind, label: label.into(), initial: initial.clone().into(), has_image: image.is_some(), image: image.unwrap_or_default() }
        }).collect();
        let model = p.model.clone();
        if model.row_count() != rows.len() {
            model.set_vec(rows);
        } else {
            for (i, r) in rows.into_iter().enumerate() {
                if model.row_data(i).as_ref() != Some(&r) { model.set_row_data(i, r); }
            }
        }
        self.ui().set_avatar_choices(model.into());
    }

    /// A game's art arrived: its square fills in.
    pub fn avatar_image_loaded(&mut self, key: &str) {
        if self.avatar_pick.as_ref().is_some_and(|p| p.picks.iter().any(|k| *k == Pick::Game(key.to_string()))) {
            self.push_avatar_pick();
        }
    }

    pub fn act_avatar_pick(&mut self, a: Act) {
        let n = self.avatar_pick.as_ref().map_or(0, |p| p.picks.len()) as i32;
        let i = self.idx.max(0);
        let to = |j: i32| j.clamp(0, (n - 1).max(0));
        match a {
            Act::Left if i % COLS as i32 > 0 => self.move_focus(Z_AVATAR, i - 1),
            Act::Right if i + 1 < n && (i + 1) % COLS as i32 != 0 => self.move_focus(Z_AVATAR, i + 1),
            Act::Up if i >= COLS as i32 => self.move_focus(Z_AVATAR, i - COLS as i32),
            Act::Down if i + (COLS as i32) < n => self.move_focus(Z_AVATAR, to(i + COLS as i32)),
            Act::Confirm => self.avatar_choose(i as usize),
            Act::Back => { self.avatar_pick = None; self.back_soft(); }
            _ => {}
        }
    }

    pub fn click_avatar_pick(&mut self, idx: i32) {
        if idx < 0 {
            self.avatar_pick = None;
            return self.back_soft();
        }
        self.set_focus(Z_AVATAR, idx);
        self.avatar_choose(idx as usize);
    }

    fn avatar_choose(&mut self, i: usize) {
        let Some(pick) = self.avatar_pick.as_ref().and_then(|p| p.picks.get(i).cloned()) else { return };
        let made = match pick {
            Pick::Initial => Ok(String::new()),
            Pick::Account(path) => from_file(&path).map(|p| p.to_string_lossy().into_owned()),
            Pick::OwnPicture => {
                // Pictures in a folder of theirs; back here once one's chosen.
                return self.open_folder_pick(crate::folder_pick::PickFor::Picture);
            }
            Pick::Game(key) => match self.images.get(&key) {
                Some(img) => from_image(&img).map(|p| p.to_string_lossy().into_owned()),
                None => Err(tr!("the picture isn't ready yet")),
            },
        };
        match made {
            Ok(path) => {
                self.avatar_set(path);
                self.avatar_pick = None;
                self.back_soft();
            }
            Err(e) => {
                audio::play(Sound::Error);
                crate::log!("avatar: {e}");
            }
        }
    }

    /// The picture chosen in the folder picker (PickFor::Picture).
    pub fn avatar_from_file(&mut self, path: &Path) {
        match from_file(path) {
            Ok(p) => {
                self.avatar_set(p.to_string_lossy().into_owned());
                audio::play(Sound::Select);
                // The grid under the picker goes too.
                if self.overlay == Overlay::Avatar {
                    self.avatar_pick = None;
                    self.back();
                }
            }
            Err(e) => {
                audio::play(Sound::Error);
                self.toast(&tr!("Couldn't use that picture"), &e, "alert", crate::toast::FAILED);
            }
        }
    }

    fn avatar_set(&mut self, path: String) {
        {
            let mut c = self.cfg.lock().unwrap();
            c.avatar_image = path;
            c.save();
            c.keep_profile();
            tidy(&c);
        }
        self.push_user();
        self.refresh_settings();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avatars_are_the_middle_square_made_small() {
        let wide = image::RgbaImage::from_fn(600, 300, |x, _| if x < 150 || x >= 450 { image::Rgba([255, 0, 0, 255]) } else { image::Rgba([0, 0, 255, 255]) });
        let a = square(wide);
        assert_eq!(a.dimensions(), (SIDE, SIDE));
        // The red edges are cut off: only the blue middle is left.
        assert_eq!(a.get_pixel(5, 128).0, [0, 0, 255, 255]);
    }

    #[test]
    fn pictures_by_their_name() {
        assert!(is_picture(Path::new("/x/me.JPG")));
        assert!(is_picture(Path::new("a.webp")));
        assert!(!is_picture(Path::new("a.txt")));
    }
}
