//! Synced lyrics from LRCLIB (https://lrclib.net), cached on disk as raw LRC text.

use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use serde_json::Value;
use winit::event_loop::EventLoopProxy;

use crate::UserEvent;

const API: &str = "https://lrclib.net/api";

#[derive(Clone)]
pub struct Line {
    pub at: Duration,
    pub text: String,
}

/// Look up lyrics on a background thread; the result arrives as `UserEvent::Lyrics`.
pub fn spawn_fetch(key: String, title: String, artist: String, duration: Duration, proxy: EventLoopProxy<UserEvent>) {
    thread::spawn(move || {
        let lrc = load_cached(&key).or_else(|| {
            let lrc = fetch(&title, &artist, duration)?;
            save_cached(&key, &lrc);
            Some(lrc)
        });
        let lines = lrc.map(|l| parse_lrc(&l)).filter(|l| !l.is_empty());
        let _ = proxy.send_event(UserEvent::Lyrics { key, lines });
    });
}

fn fetch(title: &str, artist: &str, duration: Duration) -> Option<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(8)))
        .build()
        .into();
    let artist = artist.trim_end_matches(" - Topic").trim();

    let mut req = agent
        .get(format!("{API}/get"))
        .header("User-Agent", "Sky-Island/0.1")
        .query("track_name", title)
        .query("artist_name", artist);
    if !duration.is_zero() {
        req = req.query("duration", duration.as_secs().to_string());
    }
    if let Some(lrc) = req
        .call()
        .ok()
        .and_then(|mut r| r.body_mut().read_json::<Value>().ok())
        .and_then(|v| synced(&v))
    {
        return Some(lrc);
    }

    // Titles from browsers are often messy; fall back to a looser search.
    let results = agent
        .get(format!("{API}/search"))
        .header("User-Agent", "Sky-Island/0.1")
        .query("q", format!("{title} {artist}"))
        .call()
        .ok()?
        .body_mut()
        .read_json::<Value>()
        .ok()?;
    results.as_array()?.iter().find_map(synced)
}

fn synced(v: &Value) -> Option<String> {
    let s = v.get("syncedLyrics")?.as_str()?;
    (!s.trim().is_empty()).then(|| s.to_string())
}

fn cache_path(key: &str) -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("APPDATA")?).join("Sky-Island").join("lyrics");
    fs::create_dir_all(&dir).ok()?;
    // FNV-1a: stable across runs and Rust versions, unlike DefaultHasher.
    let hash = key
        .bytes()
        .fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    Some(dir.join(format!("{hash:016x}.lrc")))
}

fn load_cached(key: &str) -> Option<String> {
    fs::read_to_string(cache_path(key)?).ok().filter(|s| !s.trim().is_empty())
}

fn save_cached(key: &str, lrc: &str) {
    if let Some(p) = cache_path(key) {
        let _ = fs::write(p, lrc);
    }
}

/// Parse `[mm:ss.xx] text` lines (a line may carry several timestamps), sorted by time.
fn parse_lrc(lrc: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    for raw in lrc.lines() {
        let mut rest = raw.trim();
        let mut stamps = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(end) = stripped.find(']') else { break };
            if let Some(t) = parse_stamp(&stripped[..end]) {
                stamps.push(t);
            }
            rest = stripped[end + 1..].trim_start();
        }
        for at in stamps {
            lines.push(Line { at, text: rest.to_string() });
        }
    }
    lines.sort_by_key(|l| l.at);
    lines
}

fn parse_stamp(s: &str) -> Option<Duration> {
    let (m, sec) = s.split_once(':')?;
    let m: u64 = m.parse().ok()?;
    let sec: f64 = sec.parse().ok()?;
    Some(Duration::from_secs_f64(m as f64 * 60.0 + sec))
}
