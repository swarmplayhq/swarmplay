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

/// A decimal number with `places` digits after the point: "21.6" in English, "21,6" in Portuguese.
pub fn num(x: f64, places: usize) -> String {
    let s = format!("{x:.places$}");
    match current() {
        Lang::En => s,
        Lang::PtBr => s.replace('.', ","),
    }
}

/// The clock: "9:05 PM" in English, "21:05" in Portuguese.
pub fn clock(h: u32, m: u32) -> String {
    match current() {
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
pub fn date(y: i32, m: u32, d: u32) -> String {
    match current() {
        Lang::En => format!("{} {d}, {y}", month(m)),
        Lang::PtBr => format!("{d} de {} de {y}", month(m)),
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
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The language is global: tests that set it take turns.
    pub static LANG_LOCK: Mutex<()> = Mutex::new(());

    /// Runs `f` with Rust's texts in `lang`, then puts English back.
    pub fn in_lang<T>(lang: Lang, f: impl FnOnce() -> T) -> T {
        let _g = LANG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_rust(lang);
        let out = f();
        set_rust(Lang::En);
        out
    }

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
    fn the_portuguese_file_reads() {
        assert!(tr::MoTranslator::from_vec_u8(PT_BR.to_vec()).is_ok());
    }

    #[test]
    fn texts_switch_and_fall_back() {
        assert_eq!(tr::tr!("Yesterday"), "Yesterday");
        in_lang(Lang::PtBr, || {
            assert_eq!(tr::tr!("Yesterday"), "Ontem");
            // Not in the file: the English text, never an empty one. (Looked up by hand: a tr!
            // here would be extracted into the file.)
            use tr::Translator as _;
            let unknown = String::from("A text nobody translated");
            let shown = tr::internal::with_translator(module_path!(), |t| t.translate(&unknown, None).into_owned());
            assert_eq!(shown, "A text nobody translated");
        });
        assert_eq!(tr::tr!("Yesterday"), "Yesterday");
    }

    #[test]
    fn plurals_follow_each_language() {
        let ago = |m: u64| tr::tr!("{n} minute ago" | "{n} minutes ago" % m);
        assert_eq!(ago(1), "1 minute ago");
        assert_eq!(ago(2), "2 minutes ago");
        assert_eq!(ago(0), "0 minutes ago");
        in_lang(Lang::PtBr, || {
            // Portuguese: 0 and 1 are singular.
            assert_eq!(ago(0), "há 0 minuto");
            assert_eq!(ago(1), "há 1 minuto");
            assert_eq!(ago(2), "há 2 minutos");
        });
    }

    #[test]
    fn formats_follow_each_language() {
        let _g = LANG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_rust(Lang::En);
        assert_eq!(num(21.64, 1), "21.6");
        assert_eq!(clock(21, 5), "9:05 PM");
        assert_eq!(clock(0, 0), "12:00 AM");
        assert_eq!(date(2026, 10, 30), "Oct 30, 2026");
        assert_eq!(ago(30.0, None), "Just now");
        assert_eq!(ago(3.0 * 86_400.0, Some((2026, 10, 30))), "Oct 30, 2026");
        set_rust(Lang::PtBr);
        assert_eq!(num(21.64, 1), "21,6");
        assert_eq!(clock(21, 5), "21:05");
        assert_eq!(clock(7, 30), "07:30");
        assert_eq!(date(2026, 10, 30), "30 de out. de 2026");
        assert_eq!(ago(7200.0, None), "há 2 horas");
        assert_eq!(ago(90_000.0, None), "Ontem");
        set_rust(Lang::En);
    }
}
