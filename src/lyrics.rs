//! Synced lyrics from LRCLIB (https://lrclib.net), cached on disk as raw LRC text.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use serde_json::Value;
use winit::event_loop::EventLoopProxy;

use crate::UserEvent;
use crate::net::BodyExt;

const API: &str = "https://lrclib.net/api";
/// No song is longer than a day; a stamp beyond that is garbage.
const MAX_STAMP_SECS: f64 = 24.0 * 3600.0;

// Lyrics are written by anyone, and a web page chooses which song is looked up, so every size is capped.
// Real synced lyrics are a few KB; these limits are far above that.
/// The largest `syncedLyrics` text that is kept, saved or parsed.
const MAX_LRC_BYTES: usize = 256 * 1024;
/// Lines kept from one song, and timestamps kept from one line.
const MAX_LRC_LINES: usize = 2000;
const MAX_STAMPS_PER_LINE: usize = 16;
/// Characters kept from one line of lyrics (the notch shows a few dozen per row).
const MAX_LINE_CHARS: usize = 300;
/// The saved lyrics folder keeps at most this many files and bytes; the oldest go first.
const MAX_CACHE_FILES: usize = 200;
const MAX_CACHE_BYTES: u64 = 16 * 1024 * 1024;
/// Two lookups on the network start at least this far apart.
const MIN_GAP: Duration = Duration::from_secs(1);
/// A song LRCLIB had no lyrics for is not asked about again for this long (at most `MAX_MISSES` songs are kept).
const MISS_KEEP: Duration = Duration::from_secs(300);
const MAX_MISSES: usize = 32;

/// `s` cut to at most `max_chars` characters, on a character boundary.
pub(crate) fn clip(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((end, _)) => &s[..end],
        None => s,
    }
}

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
    /// Looking lyrics up online is switched off, and none are saved for this song.
    Off,
}

/// The name a song's lyrics are cached under: the song and its length, because versions of one song
/// (live, remastered) differ in length and in timing. `None` while the length is not known, as then
/// the match is a guess that must not be kept.
fn cache_key(key: &str, duration: Duration) -> Option<String> {
    (!duration.is_zero()).then(|| format!("{key}|{}", duration.as_secs()))
}

/// One lookup that is waiting for the worker.
struct Job {
    /// Which request this is; a newer request makes `LATEST` bigger.
    serial: u64,
    key: String,
    title: String,
    artist: String,
    duration: Duration,
    online: bool,
    proxy: EventLoopProxy<UserEvent>,
}

/// The one lookup that waits for the worker. A newer request replaces an older one that has not started, so
/// quickly changing titles never pile up threads or requests.
static SLOT: Mutex<Option<Job>> = Mutex::new(None);
static WAKE: Condvar = Condvar::new();
static LATEST: AtomicU64 = AtomicU64::new(0);
static STARTED: AtomicBool = AtomicBool::new(false);
/// Songs LRCLIB recently had no lyrics for: `(song and length, when)`.
static MISSES: Mutex<VecDeque<(String, Instant)>> = Mutex::new(VecDeque::new());

fn locked<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Ask for the lyrics of a song: the saved ones, else (when `online`) from LRCLIB. The answer arrives as
/// `UserEvent::Lyrics`. One worker thread does all the lookups, one at a time; a request that is still waiting
/// when a newer one comes is dropped, and one that is already running is not followed by its second request.
pub fn spawn_fetch(key: String, title: String, artist: String, duration: Duration, online: bool, proxy: EventLoopProxy<UserEvent>) {
    let serial = LATEST.fetch_add(1, Ordering::SeqCst) + 1;
    *locked(&SLOT) = Some(Job { serial, key, title, artist, duration, online, proxy });
    WAKE.notify_one();
    if !STARTED.swap(true, Ordering::SeqCst) && thread::Builder::new().name("lyrics".into()).spawn(worker).is_err() {
        // The system would not give a thread: say so (the player tries again later) instead of aborting.
        STARTED.store(false, Ordering::SeqCst);
        if let Some(job) = locked(&SLOT).take() {
            let _ = job.proxy.send_event(UserEvent::Lyrics { key: job.key, fetched: Fetched::Unreachable, online: job.online });
        }
    }
}

fn worker() {
    let mut last_online: Option<Instant> = None;
    loop {
        let job = {
            let mut slot = locked(&SLOT);
            loop {
                if let Some(job) = slot.take() {
                    break job;
                }
                slot = WAKE.wait(slot).unwrap_or_else(PoisonError::into_inner);
            }
        };
        run(job, &mut last_online);
    }
}

fn run(job: Job, last_online: &mut Option<Instant>) {
    let Job { serial, key, title, artist, duration, online, proxy } = job;
    let superseded = || LATEST.load(Ordering::SeqCst) != serial;
    let cached = cache_key(&key, duration);
    let miss = format!("{key}|{}", duration.as_secs());
    let looked = match cached.as_deref().and_then(load_cached) {
        Some(lrc) => Looked::Found(lrc),
        None if !online => Looked::Off,
        None if recently_missed(&miss) => Looked::NotFound,
        None => {
            if let Some(started) = *last_online {
                thread::sleep(MIN_GAP.saturating_sub(started.elapsed()));
            }
            if superseded() {
                return;
            }
            *last_online = Some(Instant::now());
            let looked = fetch(&title, &artist, duration, &superseded);
            match (&looked, &cached) {
                (Looked::Found(lrc), Some(name)) => save_cached(name, lrc),
                (Looked::NotFound, _) => remember_miss(miss),
                _ => {}
            }
            looked
        }
    };
    // A newer request has come: nobody waits for this answer.
    if superseded() || matches!(looked, Looked::Superseded) {
        return;
    }
    let fetched = match looked {
        Looked::Found(lrc) => match parse_lrc(&lrc) {
            lines if lines.is_empty() => Fetched::NotFound,
            lines => Fetched::Lines(lines),
        },
        Looked::NotFound => Fetched::NotFound,
        Looked::Unreachable => Fetched::Unreachable,
        Looked::Off => Fetched::Off,
        Looked::Superseded => return,
    };
    let _ = proxy.send_event(UserEvent::Lyrics { key, fetched, online });
}

fn recently_missed(song: &str) -> bool {
    let mut misses = locked(&MISSES);
    misses.retain(|(_, when)| when.elapsed() < MISS_KEEP);
    misses.iter().any(|(s, _)| s == song)
}

fn remember_miss(song: String) {
    let mut misses = locked(&MISSES);
    misses.retain(|(_, when)| when.elapsed() < MISS_KEEP);
    if misses.len() >= MAX_MISSES {
        misses.pop_front();
    }
    misses.push_back((song, Instant::now()));
}

enum Looked {
    Found(String),
    NotFound,
    Unreachable,
    Off,
    /// A newer request came while this one was running.
    Superseded,
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

fn fetch(title: &str, artist: &str, duration: Duration, superseded: &dyn Fn() -> bool) -> Looked {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .timeout_global(Some(Duration::from_secs(8)))
        .build()
        .into();
    let artist = artist.trim_end_matches(" - Topic").trim();

    let mut req = agent
        .get(format!("{API}/get"))
        .header("User-Agent", concat!("Sky-Island/", env!("CARGO_PKG_VERSION")))
        .query("track_name", title)
        .query("artist_name", artist);
    if !duration.is_zero() {
        req = req.query("duration", duration.as_secs().to_string());
    }
    match req.call().and_then(|mut r| r.body_mut().read_limited_json()) {
        Ok(v) => {
            if let Some(lrc) = synced(&v) {
                return Looked::Found(lrc);
            }
        }
        // No connection: the search would only wait for its own timeout.
        Err(e) if means_down(&e) => return Looked::Unreachable,
        Err(_) => {}
    }

    // The song has changed again while the first answer was awaited: skip the second request.
    if superseded() {
        return Looked::Superseded;
    }
    // Titles from browsers are often messy; fall back to a looser search.
    let results = agent
        .get(format!("{API}/search"))
        .header("User-Agent", concat!("Sky-Island/", env!("CARGO_PKG_VERSION")))
        .query("q", format!("{title} {artist}"))
        .call()
        .and_then(|mut r| r.body_mut().read_limited_json());
    match results {
        Ok(v) => v.as_array().and_then(|list| list.iter().find_map(synced)).map_or(Looked::NotFound, Looked::Found),
        Err(e) if means_down(&e) => Looked::Unreachable,
        Err(_) => Looked::NotFound,
    }
}

fn synced(v: &Value) -> Option<String> {
    let s = v.get("syncedLyrics")?.as_str()?;
    // A record bigger than any real song is skipped (the search then tries the next one).
    (!s.trim().is_empty() && s.len() <= MAX_LRC_BYTES).then(|| s.to_string())
}

/// Delete every saved lyrics file. Returns how many songs that was.
pub fn clear_cache() -> usize {
    locked(&MISSES).clear();
    crate::store::path("lyrics").map_or(0, |dir| clear_dir(&dir))
}

fn clear_dir(dir: &std::path::Path) -> usize {
    let Ok(files) = fs::read_dir(dir) else { return 0 };
    files
        .flatten()
        .map(|f| f.path())
        .filter(|p| p.extension().is_some_and(|e| e == "lrc") && fs::remove_file(p).is_ok())
        .count()
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
    let path = cache_path(key)?;
    // A saved file bigger than any real song (an older version had no limit) is ignored and later replaced.
    if fs::metadata(&path).ok()?.len() > MAX_LRC_BYTES as u64 {
        return None;
    }
    fs::read_to_string(path).ok().filter(|s| !s.trim().is_empty())
}

fn save_cached(key: &str, lrc: &str) {
    if let Some(p) = cache_path(key) {
        crate::store::write(&p, lrc);
        if let Some(dir) = p.parent() {
            trim_cache(dir);
        }
    }
}

/// Keep the lyrics folder small: remove the oldest files while there are too many or they are too big.
fn trim_cache(dir: &Path) {
    let Ok(files) = fs::read_dir(dir) else { return };
    let entries: Vec<(PathBuf, SystemTime, u64)> = files
        .flatten()
        .filter_map(|f| {
            let path = f.path();
            if path.extension().is_none_or(|e| e != "lrc") {
                return None;
            }
            let meta = f.metadata().ok()?;
            Some((path, meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), meta.len()))
        })
        .collect();
    for path in oldest_beyond(entries, MAX_CACHE_FILES, MAX_CACHE_BYTES) {
        let _ = fs::remove_file(path);
    }
}

/// The files to remove so that at most `max_files` files and `max_bytes` bytes are left: the oldest first.
fn oldest_beyond(mut entries: Vec<(PathBuf, SystemTime, u64)>, max_files: usize, max_bytes: u64) -> Vec<PathBuf> {
    entries.sort_by_key(|e| e.1);
    let mut count = entries.len();
    let mut bytes: u64 = entries.iter().map(|e| e.2).sum();
    let mut remove = Vec::new();
    for (path, _, len) in entries {
        if count <= max_files && bytes <= max_bytes {
            break;
        }
        remove.push(path);
        count -= 1;
        bytes = bytes.saturating_sub(len);
    }
    remove
}

/// Parse `[mm:ss.xx] text` lines (a line may carry several timestamps), sorted by time. The result is bounded
/// whatever the input: at most `MAX_LRC_LINES` lines of `MAX_LINE_CHARS` characters, and `MAX_STAMPS_PER_LINE`
/// timestamps for one line (a line of `k` stamps and `T` bytes of text used to take `k * T` bytes).
fn parse_lrc(lrc: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    for raw in lrc.lines().take(MAX_LRC_LINES * 2) {
        if lines.len() >= MAX_LRC_LINES {
            break;
        }
        let mut rest = raw.trim();
        let mut stamps = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(end) = stripped.find(']') else { break };
            if stamps.len() < MAX_STAMPS_PER_LINE
                && let Some(t) = parse_stamp(&stripped[..end])
            {
                stamps.push(t);
            }
            rest = stripped[end + 1..].trim_start();
        }
        let text = clip(rest, MAX_LINE_CHARS);
        for at in stamps {
            if lines.len() >= MAX_LRC_LINES {
                break;
            }
            lines.push(Line { at, text: text.to_string() });
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
    fn clearing_the_cache_removes_only_lyrics_files() {
        let dir = std::env::temp_dir().join(format!("sky-island-lyrics-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for name in ["a.lrc", "b.lrc", "c.lrc", "keep.txt"] {
            fs::write(dir.join(name), "x").unwrap();
        }
        assert_eq!(clear_dir(&dir), 3);
        assert!(dir.join("keep.txt").exists() && !dir.join("a.lrc").exists());
        assert_eq!(clear_dir(&dir), 0);
        assert_eq!(clear_dir(&dir.join("missing")), 0);
        let _ = fs::remove_dir_all(&dir);
    }

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

    /// The audit's worst case: one line of thousands of stamps in front of a long text used to copy the text once
    /// per stamp (about 4 MB from 12 KB of input, and growing with the square of the input).
    #[test]
    fn a_line_of_many_stamps_and_long_text_stays_small() {
        let lrc = format!("{}{}", "[0:0]".repeat(2000), "x".repeat(2000));
        let lines = parse_lrc(&lrc);
        assert_eq!(lines.len(), MAX_STAMPS_PER_LINE);
        assert!(lines.iter().all(|l| l.text.chars().count() == MAX_LINE_CHARS));
        let bytes: usize = lines.iter().map(|l| l.text.len()).sum();
        assert!(bytes <= MAX_STAMPS_PER_LINE * MAX_LINE_CHARS);
        // Twice the input does not make the result bigger.
        let double = parse_lrc(&format!("{}{}", "[0:0]".repeat(4000), "x".repeat(4000)));
        assert_eq!(double.iter().map(|l| l.text.len()).sum::<usize>(), bytes);
    }

    #[test]
    fn a_normal_line_with_a_few_stamps_is_kept_whole() {
        let lines = parse_lrc("[00:10.00][01:10.00][02:10.00] la la la");
        assert_eq!(lines.len(), 3);
        assert!(lines.iter().all(|l| l.text == "la la la"));
    }

    #[test]
    fn a_very_long_line_or_song_is_cut() {
        let long = parse_lrc(&format!("[00:01.00] {}", "word ".repeat(1000)));
        assert_eq!(long.len(), 1);
        assert_eq!(long[0].text.chars().count(), MAX_LINE_CHARS);
        let many: String = (0..10_000).map(|i| format!("[{:02}:{:02}.00] line {i}\n", i / 60 % 60, i % 60)).collect();
        assert_eq!(parse_lrc(&many).len(), MAX_LRC_LINES);
        // A cut never splits a character.
        let wide = parse_lrc(&format!("[00:01.00] {}", "\u{00E9}\u{4E2D}".repeat(500)));
        assert_eq!(wide[0].text.chars().count(), MAX_LINE_CHARS);
    }

    #[test]
    fn clip_cuts_on_a_character_boundary() {
        assert_eq!(clip("hello", 10), "hello");
        assert_eq!(clip("hello", 3), "hel");
        assert_eq!(clip("\u{00E9}\u{4E2D}\u{1F600}x", 3), "\u{00E9}\u{4E2D}\u{1F600}");
        assert_eq!(clip("anything", 0), "");
    }

    #[test]
    fn a_record_bigger_than_any_song_is_not_used() {
        let ok = serde_json::json!({ "syncedLyrics": "[00:01.00] hi" });
        assert_eq!(synced(&ok).as_deref(), Some("[00:01.00] hi"));
        let huge = serde_json::json!({ "syncedLyrics": "x".repeat(MAX_LRC_BYTES + 1) });
        assert_eq!(synced(&huge), None);
        let edge = serde_json::json!({ "syncedLyrics": "x".repeat(MAX_LRC_BYTES) });
        assert!(synced(&edge).is_some());
        assert_eq!(synced(&serde_json::json!({ "syncedLyrics": "   " })), None);
        assert_eq!(synced(&serde_json::json!({})), None);
    }

    fn file(name: &str, age_secs: u64, len: u64) -> (PathBuf, SystemTime, u64) {
        (PathBuf::from(name), SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 - age_secs), len)
    }

    #[test]
    fn the_saved_lyrics_folder_drops_its_oldest_files_first() {
        let files = vec![file("new", 10, 5), file("old", 300, 5), file("mid", 100, 5), file("oldest", 900, 5)];
        let names = |v: Vec<PathBuf>| v.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert_eq!(names(oldest_beyond(files.clone(), 4, 1000)), Vec::<String>::new(), "nothing to remove");
        assert_eq!(names(oldest_beyond(files.clone(), 2, 1000)), ["oldest", "old"], "too many files");
        assert_eq!(names(oldest_beyond(files.clone(), 10, 12)), ["oldest", "old"], "too many bytes: 20 -> 10");
        assert_eq!(names(oldest_beyond(files, 10, 0)), ["oldest", "old", "mid", "new"], "even the newest goes at a limit of 0");
    }

    #[test]
    fn songs_without_lyrics_are_remembered_for_a_while_and_only_a_few() {
        let song = format!("a song nobody wrote lyrics for {}", std::process::id());
        assert!(!recently_missed(&song));
        remember_miss(song.clone());
        assert!(recently_missed(&song));
        for i in 0..(MAX_MISSES * 2) {
            remember_miss(format!("filler {i} {}", std::process::id()));
        }
        assert!(locked(&MISSES).len() <= MAX_MISSES, "the list stays small");
    }
}
