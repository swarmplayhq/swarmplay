//! Languages: English (the source text) and Brazilian Portuguese.
//!
//! One translation file per language, `i18n/<lang>/LC_MESSAGES/azrak.po`, serves both sides:
//! the `.slint` files' `@tr(…)` (bundled by build.rs, switched with
//! `slint::select_bundled_translation`) and Rust's `tr!(…)` (the `tr` crate, reading the same file,
//! embedded here). English needs no file: it is the text in the code.
//!
//! What no library does for us lives here too: dates, the clock, decimal commas, "5 minutes ago".

use std::borrow::Cow;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    PtBr,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::En, Lang::PtBr];

    /// The config's and the translation folder's name.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::PtBr => "pt_BR",
        }
    }

    /// The language's name in itself (as a console lists them).
    pub fn name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::PtBr => "Português (Brasil)",
        }
    }

    /// The config's value: a code, or empty for the system's language.
    pub fn from_config(code: &str) -> Lang {
        match code {
            "" => Lang::from_system(),
            c => Lang::from_code(c).unwrap_or(Lang::En),
        }
    }

    fn from_code(code: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|l| l.code() == code)
    }

    /// From the locale (LC_ALL, then LC_MESSAGES, then LANG): "pt_BR.UTF-8" and plain "pt" mean
    /// Portuguese; anything else, English.
    pub fn from_system() -> Lang {
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|v| std::env::var(v).ok())
            .find(|v| !v.is_empty())
            .unwrap_or_default();
        Lang::from_locale(&locale)
    }

    fn from_locale(locale: &str) -> Lang {
        let base = locale.split(['.', '@']).next().unwrap_or_default();
        if base == "pt" || base.starts_with("pt_") { Lang::PtBr } else { Lang::En }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn current() -> Lang {
    if CURRENT.load(Ordering::Relaxed) == 1 { Lang::PtBr } else { Lang::En }
}

const PT_BR: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/pt_BR.mo"));

/// A text with no translation yet comes back as the English one (the `tr` crate's .po reader
/// would give an empty string).
struct Fallback(Box<dyn tr::Translator + Send + Sync>);

impl tr::Translator for Fallback {
    fn translate<'a>(&'a self, text: &'a str, context: Option<&'a str>) -> Cow<'a, str> {
        let t = self.0.translate(text, context);
        if t.is_empty() { Cow::Borrowed(text) } else { t }
    }

    fn ntranslate<'a>(&'a self, n: u64, one: &'a str, many: &'a str, context: Option<&'a str>) -> Cow<'a, str> {
        let t = self.0.ntranslate(n, one, many, context);
        if t.is_empty() { Cow::Borrowed(if n == 1 { one } else { many }) } else { t }
    }
}

fn catalog(lang: Lang) -> Option<Fallback> {
    let bytes = match lang {
        Lang::En => return None,
        Lang::PtBr => PT_BR,
    };
    match tr::MoTranslator::from_vec_u8(bytes.to_vec()) {
        Ok(po) => Some(Fallback(Box::new(po))),
        Err(e) => {
            eprintln!("the {} translation could not be read: {e}", lang.code());
            None
        }
    }
}

/// Rust's texts only (`tr!`); for tests, and before a window exists.
fn set_rust(lang: Lang) {
    match catalog(lang) {
        Some(t) => tr::set_translator!(t),
        None => tr::unset_translator!(),
    }
    CURRENT.store(if lang == Lang::PtBr { 1 } else { 0 }, Ordering::Relaxed);
}

/// The whole app in `lang`: Rust's texts from now on, and the window's own at once (call it
/// after the window is made). Texts already handed to the window are rebuilt by
/// `App::relocalize`.
pub fn set_lang(lang: Lang) {
    set_rust(lang);
    if let Err(e) = slint::select_bundled_translation(lang.code()) {
        eprintln!("could not switch the window to {}: {e}", lang.code());
    }
}

// ------------------------------------------------------------------ formats
//
// Each takes the language (`*_in`) so tests can check both without switching the app's language,
// which is global and would race with every other test reading English.

/// A decimal number with `places` digits after the point: "21.6" in English, "21,6" in Portuguese.
pub fn num(x: f64, places: usize) -> String { num_in(current(), x, places) }

fn num_in(lang: Lang, x: f64, places: usize) -> String {
    let s = format!("{x:.places$}");
    match lang {
        Lang::En => s,
        Lang::PtBr => s.replace('.', ","),
    }
}

/// The clock: "9:05 PM" in English, "21:05" in Portuguese.
pub fn clock(h: u32, m: u32) -> String { clock_in(current(), h, m) }

fn clock_in(lang: Lang, h: u32, m: u32) -> String {
    match lang {
        Lang::En => format!("{}:{m:02} {}", if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "AM" } else { "PM" }),
        Lang::PtBr => format!("{h:02}:{m:02}"),
    }
}

/// A month's short name (1–12), as dates show it.
pub fn month(m: u32) -> String {
    match m {
        1 => tr::tr!("Jan"),
        2 => tr::tr!("Feb"),
        3 => tr::tr!("Mar"),
        4 => tr::tr!("Apr"),
        5 => tr::tr!("May"),
        6 => tr::tr!("Jun"),
        7 => tr::tr!("Jul"),
        8 => tr::tr!("Aug"),
        9 => tr::tr!("Sep"),
        10 => tr::tr!("Oct"),
        11 => tr::tr!("Nov"),
        12 => tr::tr!("Dec"),
        _ => String::new(),
    }
}

/// A day: "Oct 30, 2026" in English, "30 de out. de 2026" in Portuguese.
pub fn date(y: i32, m: u32, d: u32) -> String { date_in(current(), y, &month(m), d) }

fn date_in(lang: Lang, y: i32, month: &str, d: u32) -> String {
    match lang {
        Lang::En => format!("{month} {d}, {y}"),
        Lang::PtBr => format!("{d} de {month} de {y}"),
    }
}

/// How long ago, for lists of things that happened: "Just now", "5 minutes ago", "2 hours ago",
/// "Yesterday", then the date.
pub fn ago(secs: f64, then: Option<(i32, u32, u32)>) -> String {
    let s = secs.max(0.0) as u64;
    if s < 60 {
        tr::tr!("Just now")
    } else if s < 3600 {
        let m = s / 60;
        tr::tr!("{n} minute ago" | "{n} minutes ago" % m)
    } else if s < 86_400 {
        let h = s / 3600;
        tr::tr!("{n} hour ago" | "{n} hours ago" % h)
    } else if s < 2 * 86_400 {
        tr::tr!("Yesterday")
    } else {
        match then {
            Some((y, m, d)) => date(y, m, d),
            None => {
                let days = s / 86_400;
                tr::tr!("{n} day ago" | "{n} days ago" % days)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! These never switch the app's language (it's global, and other tests read English at the
    //! same time): they ask the Portuguese catalog itself.
    use super::*;
    use tr::Translator as _;

    fn pt() -> Fallback { catalog(Lang::PtBr).expect("the Portuguese file reads") }

    #[test]
    fn languages_from_the_locale() {
        assert_eq!(Lang::from_locale("pt_BR.UTF-8"), Lang::PtBr);
        assert_eq!(Lang::from_locale("pt"), Lang::PtBr);
        assert_eq!(Lang::from_locale("pt_PT.UTF-8@euro"), Lang::PtBr);
        assert_eq!(Lang::from_locale("en_US.UTF-8"), Lang::En);
        assert_eq!(Lang::from_locale("C"), Lang::En);
        assert_eq!(Lang::from_locale(""), Lang::En);
        assert_eq!(Lang::from_config("pt_BR"), Lang::PtBr);
        assert_eq!(Lang::from_config("en"), Lang::En);
        assert_eq!(Lang::from_config("xx"), Lang::En);
    }

    #[test]
    fn texts_translate_and_fall_back() {
        let pt = pt();
        assert_eq!(pt.translate("Yesterday", None), "Ontem");
        assert_eq!(pt.translate("Oct", None), "out.");
        // Not in the file: the English text, never an empty one.
        assert_eq!(pt.translate("A text nobody translated", None), "A text nobody translated");
        assert_eq!(pt.ntranslate(3, "One thing", "{n} things", None), "{n} things");
    }

    #[test]
    fn plurals_follow_each_language() {
        let pt = pt();
        let ago = |n| pt.ntranslate(n, "{n} minute ago", "{n} minutes ago", None).into_owned();
        // Portuguese: 0 and 1 are singular.
        assert_eq!(ago(0), "há {n} minuto");
        assert_eq!(ago(1), "há {n} minuto");
        assert_eq!(ago(2), "há {n} minutos");
        // And the {n} is filled as tr! does it.
        assert_eq!(tr::runtime_format!(ago(2), n = 2), "há 2 minutos");
    }

    #[test]
    fn formats_follow_each_language() {
        assert_eq!(num_in(Lang::En, 21.64, 1), "21.6");
        assert_eq!(num_in(Lang::PtBr, 21.64, 1), "21,6");
        assert_eq!(clock_in(Lang::En, 21, 5), "9:05 PM");
        assert_eq!(clock_in(Lang::En, 0, 0), "12:00 AM");
        assert_eq!(clock_in(Lang::PtBr, 21, 5), "21:05");
        assert_eq!(clock_in(Lang::PtBr, 7, 30), "07:30");
        assert_eq!(date_in(Lang::En, 2026, "Oct", 30), "Oct 30, 2026");
        assert_eq!(date_in(Lang::PtBr, 2026, &pt().translate("Oct", None), 30), "30 de out. de 2026");
    }
}
