//! Players' words for technical things: what's on the main screens says what it means for the
//! player ("Getting PS4 support · 9%"), and the numbers and names behind it stay in Settings and
//! the Game Hub's technical details.

use tr::tr;

/// How easy a game is to download, from how many people were sharing it (seeders) when the
/// catalog counted them. The counts aren't live, so this is a hint, not a promise.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Availability {
    /// Many people share it (`GOOD_SEEDERS` or more): it should download steadily.
    Good,
    /// A few: it may download slowly (most of them can't be reached from behind a router), or
    /// stall when they're away.
    Low,
    /// Nobody was sharing it.
    None,
    /// The catalog has no count for it.
    Unknown,
}

/// From how many seeders a release counts as easy to download.
pub const GOOD_SEEDERS: u64 = 10;

impl Availability {
    pub fn of(seeders: Option<u64>) -> Availability {
        match seeders {
            None => Availability::Unknown,
            Some(0) => Availability::None,
            Some(n) if n < GOOD_SEEDERS => Availability::Low,
            Some(_) => Availability::Good,
        }
    }

    /// The label with how many were sharing it ("Good · 69 sharing"), where there's room for it.
    pub fn with_count(seeders: Option<u64>) -> String {
        let a = Availability::of(seeders);
        match seeders {
            Some(n) if n > 0 => format!("{} · {}", a.label(), tr!("{n} sharing" | "{n} sharing" % n)),
            _ => a.label(),
        }
    }

    pub fn label(self) -> String {
        match self {
            Availability::Good => tr!("Availability" => "Good"),
            Availability::Low => tr!("Availability" => "Low"),
            Availability::None => tr!("Availability" => "None"),
            Availability::Unknown => tr!("Availability" => "Unknown"),
        }
    }

    /// 1 good (green), 2 low (amber), 4 none or unknown (grey): the Fact colours.
    pub fn kind(self) -> i32 {
        match self {
            Availability::Good => 1,
            Availability::Low => 2,
            Availability::None | Availability::Unknown => 4,
        }
    }
}

/// The status line under the Home screen, in players' words. Progress messages name emulators
/// and versions ("Downloading shadPS4 0.20.0 · 9%"), which Settings → Updates still shows; on
/// the Home screen it's what they're for that matters. What's matched is the tools' English; what
/// comes out is in the player's language.
pub fn status(text: &str) -> String {
    let percent = |done: &str| -> Option<u64> {
        let (a, b) = done.split_once('/')?;
        let (a, b): (f64, f64) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
        (b > 0.0).then(|| (a / b * 100.0).clamp(0.0, 100.0).floor() as u64)
    };
    // "… · 9%" at the end of a progress message.
    let tail = text.rsplit_once(" · ").map(|(_, p)| p).filter(|p| p.ends_with('%')).map(|p| format!(" · {p}")).unwrap_or_default();
    // The emulators' and the launcher's own progress come in the interface's language (kyty.rs,
    // shad.rs, update.rs): matched by their English start or by their translation's, up to its
    // first {}.
    let begins = |english: &str, template: String| {
        let head = template.split('{').next().unwrap_or_default().trim_end_matches('…').to_string();
        text.starts_with(english) || (!head.trim().is_empty() && text.starts_with(head.as_str()))
    };
    let installing = begins("Installing shadPS4", tr!("Installing shadPS4…")) || begins("Installing KytyPS5", tr!("Installing KytyPS5…"));
    let support = |console: &str| {
        if installing { tr!("Setting up {console} support…", console) } else { tr!("Getting {console} support", console) + &tail }
    };
    if begins("Downloading shadPS4", tr!("Downloading shadPS4 {} · {}%")) || begins("Installing shadPS4", tr!("Installing shadPS4…")) {
        return support("PS4");
    }
    if begins("Downloading KytyPS5", tr!("Downloading KytyPS5 {} · {}%")) || begins("Installing KytyPS5", tr!("Installing KytyPS5…")) {
        return support("PS5");
    }
    if begins("Downloading SwarmPlay", tr!("Downloading SwarmPlay {} · {}%")) {
        return tr!("Getting the launcher's update") + &tail;
    }
    if let Some(done) = text.strip_prefix("Downloading artwork ").or_else(|| text.strip_prefix("Downloading game art ")) {
        return match percent(done) {
            Some(p) => tr!("Getting game pictures · {p}%", p),
            None => tr!("Getting game pictures…"),
        };
    }
    if text.starts_with("Loading RuTracker") || (text.starts_with("Loaded ") && text.ends_with("RuTracker releases")) {
        return tr!("Loading the game list…");
    }
    if let Some(console) = text.strip_prefix("Downloading the ").and_then(|t| t.strip_suffix(" catalog")) {
        return tr!("Updating the {console} game list…", console);
    }
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_from_seeders() {
        assert_eq!(Availability::of(Some(46)), Availability::Good);
        assert_eq!(Availability::of(Some(10)), Availability::Good);
        assert_eq!(Availability::of(Some(9)), Availability::Low, "a few seeders download slowly");
        assert_eq!(Availability::of(Some(2)), Availability::Low);
        assert_eq!(Availability::of(Some(1)), Availability::Low);
        assert_eq!(Availability::of(Some(0)), Availability::None);
        assert_eq!(Availability::of(None), Availability::Unknown, "a missing count is unknown, not zero");
        assert_eq!((Availability::Good.kind(), Availability::Low.kind(), Availability::None.kind()), (1, 2, 4));
        assert_eq!(Availability::with_count(Some(69)), "Good · 69 sharing");
        assert_eq!(Availability::with_count(Some(0)), "None");
        assert_eq!(Availability::with_count(None), "Unknown");
    }

    #[test]
    fn status_line_in_players_words() {
        assert_eq!(status("Downloading shadPS4 0.20.0 · 9%"), "Getting PS4 support · 9%");
        assert_eq!(status("Installing shadPS4…"), "Setting up PS4 support…");
        assert_eq!(status("Downloading KytyPS5 2026-10-19 · 40%"), "Getting PS5 support · 40%");
        assert_eq!(status("Downloading artwork 1382/3602"), "Getting game pictures · 38%");
        assert_eq!(status("Downloading game art 10/20"), "Getting game pictures · 50%");
        assert_eq!(status("Downloading the PS4 catalog"), "Updating the PS4 game list…");
        assert_eq!(status("Downloading SwarmPlay 1.18.0 · 5%"), "Getting the launcher's update · 5%");
        assert_eq!(status("Checking for new releases…"), "Checking for new releases…");
        assert_eq!(status(""), "");
    }
}
