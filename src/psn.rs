//! Official PlayStation artwork and metadata by title ID. (Games Sony has none for get Steam's,
//! which the daily catalog update looks up: see App::art_for_game.)
//!
//! Sony's public PlayStation App catalog API returns the same assets the PS5 shows: square
//! tile icon (MASTER), game hub background, transparent title logo, portrait cover,
//! screenshots, trailer, star rating. Cached in ~/.cache/azrak/psn.json (v1 format).
//!
//! In Portuguese, games from the American region come from the Brazilian store (descriptions and
//! genres in Portuguese where Sony has them, anything missing taken from the English entry),
//! kept apart in psn-pt_BR.json. Until a game's Portuguese entry is fetched, its English one shows.

use crate::util::{atomic_write, cache_dir, clean_multiline, http_json, now_secs};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use crate::i18n::Lang;

const API: &str = "https://m.np.playstation.com/api/catalog/v2/titles/{tid}_00/concepts/?age=99&country={c}&language={l}";
const FRESH: f64 = 14.0 * 86400.0;
const MISS_RETRY: f64 = 3.0 * 86400.0;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Rating {
    pub score: f64,
    pub total: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct AgeRating {
    pub name: String,
    pub icon: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Info {
    pub source: Option<String>,
    pub concept: Option<Value>,
    pub name: String,
    pub publisher: String,
    pub genres: Vec<String>,
    pub release: String,
    pub rating: Option<Rating>,
    pub age: Option<AgeRating>,
    pub players: Option<Value>,
    pub short: String,
    pub long: String,
    pub master: String,
    pub hub: String,
    pub logo: String,
    pub portrait: String,
    pub bg: String,
    pub banner: String,
    pub shots: Vec<String>,
    pub video: String,
    pub poster: String,
    pub store: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Entry {
    pub at: f64,
    pub info: Option<Info>,
}

pub fn region_from_label(v: &str) -> &'static str {
    let v = v.to_uppercase();
    if v.contains("EUR") || v == "GB" || v == "UK" {
        "GB"
    } else if v.contains("JP") || v.contains("JAPAN") {
        "JP"
    } else if v.contains("ASIA") || v.contains("HK") {
        "HK"
    } else {
        "US"
    }
}

pub fn region_from_content_id(cid: &str) -> &'static str {
    match cid.get(..2) {
        Some("EP") => "GB",
        Some("JP") => "JP",
        Some("HP") | Some("KP") => "HK",
        _ => "US",
    }
}

/// The store (country, language) asked for a region's game. In Portuguese the American region's
/// games come from the Brazilian store; other regions have no Portuguese one.
fn store_for(region: &str, lang: Lang) -> (&'static str, &'static str) {
    match (region, lang) {
        ("GB", _) => ("GB", "en-GB"),
        ("JP", _) => ("JP", "ja-JP"),
        ("HK", _) => ("HK", "en-HK"),
        (_, Lang::PtBr) => ("BR", "pt-BR"),
        (_, Lang::En) => ("US", "en-US"),
    }
}

/// What the Portuguese entry lacks (Sony's text for that store, its pictures), from the English one.
fn fill_from(local: &mut Info, en: &Info) {
    let text = |a: &mut String, b: &String| if a.trim().is_empty() { a.clone_from(b) };
    text(&mut local.name, &en.name);
    text(&mut local.publisher, &en.publisher);
    text(&mut local.release, &en.release);
    text(&mut local.short, &en.short);
    text(&mut local.long, &en.long);
    for (a, b) in [(&mut local.master, &en.master), (&mut local.hub, &en.hub), (&mut local.logo, &en.logo),
        (&mut local.portrait, &en.portrait), (&mut local.bg, &en.bg), (&mut local.banner, &en.banner),
        (&mut local.video, &en.video), (&mut local.poster, &en.poster), (&mut local.store, &en.store)] {
        text(a, b);
    }
    if local.genres.is_empty() { local.genres.clone_from(&en.genres); }
    if local.shots.is_empty() { local.shots.clone_from(&en.shots); }
    if local.rating.is_none() { local.rating.clone_from(&en.rating); }
    if local.age.is_none() { local.age.clone_from(&en.age); }
    if local.players.is_none() { local.players.clone_from(&en.players); }
    if local.concept.is_none() { local.concept.clone_from(&en.concept); }
}

static TID_RE: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"^(PPSA|CUSA)\d{5}$").unwrap());

fn compact(c: &Value, lang: &str) -> Info {
    let mut imgs: HashMap<String, String> = HashMap::new();
    let mut shots = Vec::new();
    for i in c["media"]["images"].as_array().into_iter().flatten() {
        let (t, u) = (i["type"].as_str().unwrap_or(""), i["url"].as_str().unwrap_or(""));
        if u.is_empty() {
            continue;
        }
        if t == "SCREENSHOT" {
            shots.push(u.to_string());
        } else {
            imgs.entry(t.to_string()).or_insert_with(|| u.to_string());
        }
    }
    let video = c["media"]["videos"].as_array().into_iter().flatten().find(|v| v["url"].as_str().is_some());
    let desc = |kind: &str| {
        c["descriptions"].as_array().into_iter().flatten().find(|d| d["type"] == kind).and_then(|d| d["desc"].as_str()).map(clean_multiline).unwrap_or_default()
    };
    let rating = {
        let sr = &c["starRating"];
        let score = sr["score"].as_str().and_then(|s| s.parse::<f64>().ok()).or(sr["score"].as_f64());
        let total = sr["total"].as_str().and_then(|s| s.parse::<i64>().ok()).or(sr["total"].as_i64()).unwrap_or(0);
        score.filter(|_| total > 0).map(|s| Rating { score: (s * 100.0).round() / 100.0, total })
    };
    let cr = &c["contentRating"];
    let img = |k: &str| imgs.get(k).cloned().unwrap_or_default();
    Info {
        source: None,
        concept: c.get("id").cloned(),
        name: c["nameEn"].as_str().or(c["name"].as_str()).unwrap_or("").to_string(),
        publisher: c["publisherName"].as_str().unwrap_or("").to_string(),
        genres: c["localizedGenres"].as_array().into_iter().flatten().filter_map(|g| g["value"].as_str().map(String::from)).collect(),
        release: c["releaseDate"]["date"].as_str().unwrap_or("").chars().take(10).collect(),
        rating,
        age: cr.as_object().map(|_| AgeRating {
            name: cr["description"].as_str().unwrap_or("").to_string(),
            icon: cr["url"].as_str().unwrap_or("").to_string(),
        }),
        players: c["compatibilityNotices"].as_array().into_iter().flatten().find(|n| n["type"] == "NO_OF_PLAYERS").map(|n| n["value"].clone()),
        short: desc("SHORT"),
        long: desc("LONG"),
        master: img("MASTER"),
        hub: img("GAMEHUB_COVER_ART"),
        logo: img("LOGO"),
        portrait: img("PORTRAIT_BANNER"),
        bg: img("BACKGROUND_LAYER_ART"),
        banner: img("FOUR_BY_THREE_BANNER"),
        shots: shots.into_iter().take(12).collect(),
        video: video.and_then(|v| v["url"].as_str()).unwrap_or("").to_string(),
        poster: video.and_then(|v| v["posters"][0].as_str()).unwrap_or("").to_string(),
        store: c.get("id").map(|id| format!("https://store.playstation.com/{}/concept/{}", lang.to_lowercase(), id)).unwrap_or_default(),
    }
}

fn fetch_sony(tid: &str, region: &str) -> Option<Info> {
    let mut order = vec![region];
    order.extend(["US", "GB", "HK", "JP"].into_iter().filter(|r| *r != region));
    for r in order {
        let (country, lang) = store_for(r, Lang::En);
        let url = API.replace("{tid}", tid).replace("{c}", country).replace("{l}", lang);
        let Ok(items) = http_json(&url) else { continue };
        let Some(arr) = items.as_array() else { continue };
        let game = arr.iter().find(|c| c["type"] == "GAME").or(arr.first());
        if let Some(c) = game {
            return Some(compact(c, lang));
        }
    }
    None
}

/// A game's entry from the Brazilian store, completed from its English one. None when that store
/// doesn't have it (a game from another region, say): the English entry shows.
fn fetch_local(tid: &str, region: &str, lang: Lang, en: Option<&Info>) -> Option<Info> {
    let (country, code) = store_for(region, lang);
    if code.starts_with("en") || code == "ja-JP" { return None; }
    let url = API.replace("{tid}", tid).replace("{c}", country).replace("{l}", code);
    let items = http_json(&url).ok()?;
    let arr = items.as_array()?;
    let mut info = compact(arr.iter().find(|c| c["type"] == "GAME").or(arr.first())?, code);
    if let Some(en) = en { fill_from(&mut info, en); }
    Some(info)
}

// ------------------------------------------------------------------ cache

#[derive(Default)]
pub struct Store {
    /// English entries (psn.json): every game's pictures and details.
    pub data: HashMap<String, Entry>,
    /// Portuguese entries (psn-pt_BR.json), shown instead while the app is in Portuguese.
    local: HashMap<String, Entry>,
    dirty: usize,
}

pub type Shared = Arc<Mutex<Store>>;

impl Store {
    fn path() -> std::path::PathBuf {
        cache_dir().join("psn.json")
    }

    fn local_path() -> std::path::PathBuf {
        cache_dir().join("psn-pt_BR.json")
    }

    pub fn load() -> Shared {
        let read = |path: std::path::PathBuf| -> HashMap<String, Entry> {
            std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
        };
        let mut data = read(Self::path());
        // RAWG results from before it was dropped: nothing reads them any more.
        data.retain(|key, _| !key.starts_with("rawg:"));
        Arc::new(Mutex::new(Store { data, local: read(Self::local_path()), dirty: 0 }))
    }

    pub fn save(&mut self) {
        self.dirty = 0;
        if let Ok(json) = serde_json::to_vec(&self.data) {
            let _ = atomic_write(&Self::path(), &json);
        }
        if !self.local.is_empty() {
            if let Ok(json) = serde_json::to_vec(&self.local) {
                let _ = atomic_write(&Self::local_path(), &json);
            }
        }
    }

    /// A game's entry in the app's language (its English one until the other is fetched).
    pub fn get(&self, key: &str) -> Option<&Info> {
        let local = (crate::i18n::current() != Lang::En).then(|| self.local.get(key).and_then(|e| e.info.as_ref())).flatten();
        local.or_else(|| self.data.get(key).and_then(|e| e.info.as_ref()))
    }

    fn fresh_in(map: &HashMap<String, Entry>, key: &str) -> bool {
        map.get(key).is_some_and(|e| now_secs() - e.at < if e.info.is_some() { FRESH } else { MISS_RETRY })
    }

    /// Fetched recently enough, in English and (in Portuguese) in the app's language.
    pub fn fresh(&self, key: &str) -> bool {
        Self::fresh_in(&self.data, key) && (crate::i18n::current() == Lang::En || Self::fresh_in(&self.local, key))
    }

    fn put(&mut self, key: String, info: Option<Info>) {
        let prev = self.data.get(&key).and_then(|e| e.info.clone());
        // Keep old data if the service is temporarily unreachable.
        self.data.insert(key, Entry { at: now_secs(), info: info.or(prev) });
        self.bump();
    }

    fn put_local(&mut self, key: String, info: Option<Info>) {
        let prev = self.local.get(&key).and_then(|e| e.info.clone());
        self.local.insert(key, Entry { at: now_secs(), info: info.or(prev) });
        self.bump();
    }

    fn bump(&mut self) {
        self.dirty += 1;
        if self.dirty >= 40 {
            self.save();
        }
    }
}

pub fn valid_title_id(tid: &str) -> bool {
    TID_RE.is_match(tid)
}

/// Fetch missing/stale Sony entries in parallel.
/// `progress` receives status text; returns true if anything changed.
pub fn enrich(store: &Shared, titles: Vec<(String, &'static str)>, progress: &(dyn Fn(String) + Sync)) -> bool {
    let todo: Vec<(String, &str)> = {
        let s = store.lock().unwrap();
        let mut seen = std::collections::HashSet::new();
        titles.into_iter().filter(|(t, _)| valid_title_id(t) && !s.fresh(t) && seen.insert(t.clone())).collect()
    };
    let lang = crate::i18n::current();
    let mut changed = false;
    if !todo.is_empty() {
        let next = std::sync::atomic::AtomicUsize::new(0);
        let done = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|sc| {
            for _ in 0..6 {
                sc.spawn(|| loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((tid, region)) = todo.get(i) else { break };
                    let (en_fresh, local_fresh) = {
                        let s = store.lock().unwrap();
                        (Store::fresh_in(&s.data, tid), lang == Lang::En || Store::fresh_in(&s.local, tid))
                    };
                    if !en_fresh {
                        let info = fetch_sony(tid, region);
                        store.lock().unwrap().put(tid.clone(), info);
                    }
                    if !local_fresh {
                        let en = store.lock().unwrap().data.get(tid).and_then(|e| e.info.clone());
                        let info = fetch_local(tid, region, lang, en.as_ref());
                        store.lock().unwrap().put_local(tid.clone(), info);
                    }
                    let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    progress(format!("Downloading artwork {n}/{}", todo.len()));
                });
            }
        });
        changed = true;
    }
    if changed {
        store.lock().unwrap().save();
    }
    changed
}

/// Sony image CDN resizes on request; other hosts ignore the parameter, so only add it there.
pub fn sized(url: &str, w: u32) -> String {
    if url.contains("image.api.playstation.com") && !url.contains('?') {
        format!("{url}?w={w}")
    } else {
        url.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_store_asked_follows_the_language() {
        assert_eq!(store_for("US", Lang::En), ("US", "en-US"));
        assert_eq!(store_for("US", Lang::PtBr), ("BR", "pt-BR"));
        // Other regions have no Portuguese store: their own, as in English.
        assert_eq!(store_for("GB", Lang::PtBr), ("GB", "en-GB"));
        assert_eq!(store_for("JP", Lang::PtBr), ("JP", "ja-JP"));
        assert_eq!(store_for("HK", Lang::En), ("HK", "en-HK"));
    }

    #[test]
    fn a_portuguese_entry_is_completed_from_the_english_one() {
        let en = Info { name: "Game".into(), short: "Short".into(), long: "Long".into(), genres: vec!["Action".into()],
            master: "m.png".into(), shots: vec!["s.png".into()], ..Default::default() };
        let mut pt = Info { long: "Longo".into(), ..Default::default() };
        fill_from(&mut pt, &en);
        assert_eq!((pt.name.as_str(), pt.short.as_str(), pt.long.as_str()), ("Game", "Short", "Longo"));
        assert_eq!((pt.genres, pt.master, pt.shots), (vec!["Action".to_string()], "m.png".to_string(), vec!["s.png".to_string()]));
    }
}
