//! Synced lyrics from LRCLIB (https://lrclib.net), cached on disk as raw LRC text.

use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use serde_json::Value;
use winit::event_loop::EventLoopProxy;

use crate::UserEvent;

const API: &str = "https://lrclib.net/api";
/// No song is longer than a day; a stamp beyond that is garbage.
const MAX_STAMP_SECS: f64 = 24.0 * 3600.0;

#[derive(Clone)]
pub struct Line {
    pub at: Duration,
    pub text: String,
}

/// The name a song's lyrics are cached under: the song and its length, because versions of one song
/// (live, remastered) differ in length and in timing. `None` while the length is not known, as then
/// the match is a guess that must not be kept.
fn cache_key(key: &str, duration: Duration) -> Option<String> {
    (!duration.is_zero()).then(|| format!("{key}|{}", duration.as_secs()))
}

/// Look up lyrics on a background thread; the result arrives as `UserEvent::Lyrics`.
pub fn spawn_fetch(key: String, title: String, artist: String, duration: Duration, proxy: EventLoopProxy<UserEvent>) {
    thread::spawn(move || {
        let cached = cache_key(&key, duration);
        let lrc = cached.as_deref().and_then(load_cached).or_else(|| {
            let lrc = fetch(&title, &artist, duration)?;
            if let Some(name) = &cached {
                save_cached(name, &lrc);
            }
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
    let dir = crate::store::path("lyrics")?;
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
        crate::store::write(&p, lrc);
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
    // Lyrics are written by anyone, and "-1", "inf" and "nan" all parse as numbers; Duration panics on them.
    let total = m as f64 * 60.0 + sec;
    if !(0.0..=MAX_STAMP_SECS).contains(&total) {
        return None;
    }
    Duration::try_from_secs_f64(total).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lyrics_are_cached_per_length_and_never_without_one() {
        let (a, b) = (cache_key("Song|Artist", Duration::from_secs(200)), cache_key("Song|Artist", Duration::from_secs(215)));
        assert!(a.is_some() && b.is_some() && a != b, "another version has its own entry");
        assert_eq!(cache_key("Song|Artist", Duration::ZERO), None, "a guess is not kept");
    }

    #[test]
    fn a_normal_stamp_is_read() {
        assert_eq!(parse_stamp("01:02.50"), Some(Duration::from_millis(62_500)));
        assert_eq!(parse_stamp("00:00.00"), Some(Duration::ZERO));
    }

    #[test]
    fn strange_stamps_are_skipped_instead_of_crashing() {
        for bad in ["00:-1.00", "00:inf", "00:nan", "00:1e400", "99999999999999999:00", "00:-0.5", "abc", ""] {
            assert_eq!(parse_stamp(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_line_with_a_bad_stamp_is_dropped_and_the_rest_is_kept() {
        let lines = parse_lrc("[00:-1.00] broken\n[00:05.00] fine\n[00:nan] also broken\n[00:10.00] ok");
        assert_eq!(lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["fine", "ok"]);
    }
}
