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

/// What looking a song up came to.
pub enum Fetched {
    Lines(Vec<Line>),
    /// LRCLIB answered, and has no synced lyrics for this song.
    NotFound,
    /// LRCLIB could not be reached or is failing, which says nothing about the song.
    Unreachable,
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
        let looked = match cached.as_deref().and_then(load_cached) {
            Some(lrc) => Looked::Found(lrc),
            None => {
                let looked = fetch(&title, &artist, duration);
                if let (Looked::Found(lrc), Some(name)) = (&looked, &cached) {
                    save_cached(name, lrc);
                }
                looked
            }
        };
        let fetched = match looked {
            Looked::Found(lrc) => match parse_lrc(&lrc) {
                lines if lines.is_empty() => Fetched::NotFound,
                lines => Fetched::Lines(lines),
            },
            Looked::NotFound => Fetched::NotFound,
            Looked::Unreachable => Fetched::Unreachable,
        };
        let _ = proxy.send_event(UserEvent::Lyrics { key, fetched });
    });
}

enum Looked {
    Found(String),
    NotFound,
    Unreachable,
}

/// Whether a failed request says that LRCLIB itself is the problem (no connection, a timeout, a server
/// error, too many requests) and not that the song is simply not there (a 404, an answer that is no JSON).
fn means_down(e: &ureq::Error) -> bool {
    match e {
        ureq::Error::StatusCode(code) => *code == 429 || *code >= 500,
        ureq::Error::Json(_) => false,
        _ => true,
    }
}

fn fetch(title: &str, artist: &str, duration: Duration) -> Looked {
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
    match req.call().and_then(|mut r| r.body_mut().read_json::<Value>()) {
        Ok(v) => {
            if let Some(lrc) = synced(&v) {
                return Looked::Found(lrc);
            }
        }
        // No connection: the search would only wait for its own timeout.
        Err(e) if means_down(&e) => return Looked::Unreachable,
        Err(_) => {}
    }

    // Titles from browsers are often messy; fall back to a looser search.
    let results = agent
        .get(format!("{API}/search"))
        .header("User-Agent", "Sky-Island/0.1")
        .query("q", format!("{title} {artist}"))
        .call()
        .and_then(|mut r| r.body_mut().read_json::<Value>());
    match results {
        Ok(v) => v.as_array().and_then(|list| list.iter().find_map(synced)).map_or(Looked::NotFound, Looked::Found),
        Err(e) if means_down(&e) => Looked::Unreachable,
        Err(_) => Looked::NotFound,
    }
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
    fn a_site_that_is_down_is_told_apart_from_a_song_that_is_missing() {
        use ureq::{Error, Timeout};
        assert!(!means_down(&Error::StatusCode(404)), "no lyrics for the song");
        assert!(!means_down(&Error::StatusCode(400)));
        assert!(means_down(&Error::StatusCode(500)));
        assert!(means_down(&Error::StatusCode(503)));
        assert!(means_down(&Error::StatusCode(429)), "asked too often");
        assert!(means_down(&Error::Timeout(Timeout::Global)));
        assert!(means_down(&Error::HostNotFound));
        assert!(means_down(&Error::ConnectionFailed));
    }

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
