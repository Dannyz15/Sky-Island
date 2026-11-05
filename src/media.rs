//! Now-playing info and controls via Windows GSMTC. Runs on its own thread and
//! pushes a fresh `MediaState` to the UI whenever something changes.

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
    MediaPropertiesChangedEventArgs, PlaybackInfoChangedEventArgs, TimelinePropertiesChangedEventArgs,
};
use windows::Storage::Streams::{Buffer, DataReader, IRandomAccessStreamReference, InputStreamOptions};
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows::core::Result;
use winit::event_loop::EventLoopProxy;

use crate::UserEvent;

const ART_SIZE: u32 = 96;

/// How often the player is read again even if Windows said nothing, to catch a missed change and
/// to keep the position honest.
const RESYNC: Duration = Duration::from_secs(4);
/// The same while nothing is playing: Windows still announces every change, and no position is running
/// that needs to be kept honest, so the thread can sleep longer.
const RESYNC_IDLE: Duration = Duration::from_secs(15);
/// How long to wait before asking Windows for the media sessions again when that failed.
const START_RETRY: Duration = Duration::from_secs(5);
/// When a track changes, browsers (YouTube above all) fill in its title, length, picture and time one
/// piece at a time. The player is read again after these delays so the settled values win.
const SETTLE: [Duration; 5] = [
    Duration::from_millis(500),
    Duration::from_millis(1500),
    Duration::from_millis(3500),
    Duration::from_millis(6000),
    Duration::from_millis(10000),
];

#[derive(Clone, Default)]
pub struct MediaState {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub playing: bool,
    pub position: Duration,
    pub duration: Duration,
    /// Square RGBA8 thumbnail (ART_SIZE x ART_SIZE).
    pub art: Option<Vec<u8>>,
    pub has_session: bool,
    pub next_enabled: bool,
    /// Whether the player lets other programs move the song (see `seek_allowed`).
    pub can_seek: bool,
}

/// Whether a click on the bar or on a lyric line can move the song. Apple Music says it cannot ("position can
/// be set" is false) and ignores the jump, so the song would carry on where it was and the lyrics would go back
/// to it. A player that has not given a length yet (Firefox, for a moment) is given the benefit of the doubt.
fn seek_allowed(position_enabled: bool, duration: Duration) -> bool {
    position_enabled || duration.is_zero()
}

impl MediaState {
    /// Chrome (YouTube) and Firefox answer with an empty timeline, a length of 0 and a position that only counts up
    /// from a stale moment, for a while after a seek while the song is still the same. Taken as it is, the
    /// position would drop to the start, and the lyrics would glide to the top right after a line was clicked.
    /// So an empty timeline for the song that is already showing keeps the position it has (`shown`) and its
    /// length, when the player gave a real timeline before or Sky Island has just moved the song itself
    /// (`seeked`); everything else is taken from `self`. (A player that never gives a length, and has not
    /// been seeked from here, is believed: its position is all there is.)
    pub fn keep_timeline(self, old: &MediaState, shown: Duration, seeked: bool) -> MediaState {
        let empty = self.has_session && self.duration.is_zero();
        let same_song = (&self.title, &self.artist) == (&old.title, &old.artist);
        if empty && same_song && (!old.duration.is_zero() || seeked) {
            return MediaState { position: shown, duration: old.duration, ..self };
        }
        self
    }

    /// Nothing the UI shows differs between the two.
    fn same_as(&self, o: &MediaState) -> bool {
        (&self.title, &self.artist, &self.album, self.playing, self.position, self.duration)
            == (&o.title, &o.artist, &o.album, o.playing, o.position, o.duration)
            && (self.has_session, self.next_enabled, self.can_seek, &self.art) == (o.has_session, o.next_enabled, o.can_seek, &o.art)
    }
}

pub enum Command {
    TogglePlay,
    Next,
    Previous,
    Seek(Duration),
}

enum Msg {
    Refresh,
    Command(Command),
}

#[derive(Clone)]
pub struct MediaHandle(Sender<Msg>);

impl MediaHandle {
    pub fn send(&self, cmd: Command) {
        let _ = self.0.send(Msg::Command(cmd));
    }
}

pub fn spawn(proxy: EventLoopProxy<UserEvent>) -> MediaHandle {
    let (tx, rx) = mpsc::channel::<Msg>();
    let handle = MediaHandle(tx.clone());
    thread::spawn(move || {
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }
        // Windows may not be ready for this right after signing in: try again until it is.
        let manager = loop {
            match Manager::RequestAsync().and_then(|op| op.join()) {
                Ok(manager) => break manager,
                // A command meant for a player that is not there yet is dropped.
                Err(_) => match rx.recv_timeout(START_RETRY) {
                    Ok(_) | Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                },
            }
        };

        let t = tx.clone();
        let _ = manager.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
            let _ = t.send(Msg::Refresh);
            Ok(())
        }));

        let mut subscribed: Option<Subscription> = None;
        // What was sent last, and the times at which to read again.
        let mut last: Option<(String, String, Duration)> = None;
        // The last state sent to the UI, so a quiet resync can reuse its cover and need not repeat it.
        let mut sent: Option<MediaState> = None;
        let mut settle: Vec<Instant> = Vec::new();
        let _ = tx.send(Msg::Refresh);

        loop {
            let now = Instant::now();
            // Nothing is settling: a timeout is then only the regular resync, not a track change.
            let resync_only = settle.is_empty();
            let resync = if sent.as_ref().is_some_and(|state| state.playing) { RESYNC } else { RESYNC_IDLE };
            let wait = settle.iter().map(|t| t.saturating_duration_since(now)).min().unwrap_or(resync).min(resync);
            // A message from Windows or the UI, or a timer that says to read again.
            let (msg, quiet) = match rx.recv_timeout(wait) {
                Ok(msg) => (msg, false),
                Err(RecvTimeoutError::Timeout) => (Msg::Refresh, resync_only),
                Err(RecvTimeoutError::Disconnected) => break,
            };
            let session = manager.GetCurrentSession().ok();
            match msg {
                Msg::Command(cmd) => {
                    if let Some(s) = &session {
                        let _ = match cmd {
                            Command::TogglePlay => s.TryTogglePlayPauseAsync().map(|_| ()),
                            Command::Next => s.TrySkipNextAsync().map(|_| ()),
                            Command::Previous => skip_previous(s),
                            Command::Seek(d) => s
                                .TryChangePlaybackPositionAsync((d.as_nanos() / 100) as i64)
                                .map(|_| ()),
                        };
                    }
                    // The answer to a command comes a moment later; read it again then.
                    settle = SETTLE.iter().map(|d| Instant::now() + *d).collect();
                }
                Msg::Refresh => {
                    let now = Instant::now();
                    settle.retain(|t| *t > now);
                    // Subscribe again only when the active session is not the one already subscribed to: a player
                    // that reports every second would otherwise have its three registrations dropped and made again
                    // each time, and what it said in between would be missed.
                    if subscribed.as_ref().map(|s| &s.session) != session.as_ref() {
                        subscribed = session.as_ref().and_then(|s| Subscription::new(s, tx.clone()).ok());
                    }
                    let state = match &session {
                        None => MediaState::default(),
                        Some(s) => match read_state(s, quiet.then_some(sent.as_ref()).flatten()) {
                            Ok(state) => state,
                            Err(_) => {
                                // Not ready yet (a track is changing): keep what is shown and try again soon.
                                if settle.is_empty() {
                                    settle.push(now + SETTLE[0]);
                                }
                                continue;
                            }
                        },
                    };
                    let seen = (state.title.clone(), state.artist.clone(), state.duration);
                    if last.as_ref() != Some(&seen) {
                        // A new track (or the first look): read again as its details settle.
                        settle = SETTLE.iter().map(|d| now + *d).collect();
                        last = Some(seen);
                    }
                    // A paused player that has not changed tells the UI nothing new.
                    if !state.playing && sent.as_ref().is_some_and(|old| old.same_as(&state)) {
                        continue;
                    }
                    sent = Some(state.clone());
                    let _ = proxy.send_event(UserEvent::Media(state));
                }
            }
            let _ = &subscribed;
        }
    });
    handle
}

/// Holds the per-session event registrations; unregisters on drop.
struct Subscription {
    session: Session,
    props: i64,
    playback: i64,
    timeline: i64,
}

impl Subscription {
    fn new(session: &Session, tx: Sender<Msg>) -> Result<Self> {
        let t = tx.clone();
        let props = session.MediaPropertiesChanged(&TypedEventHandler::<
            Session,
            MediaPropertiesChangedEventArgs,
        >::new(move |_, _| {
            let _ = t.send(Msg::Refresh);
            Ok(())
        }))?;
        let t = tx.clone();
        let playback = session.PlaybackInfoChanged(&TypedEventHandler::<
            Session,
            PlaybackInfoChangedEventArgs,
        >::new(move |_, _| {
            let _ = t.send(Msg::Refresh);
            Ok(())
        }))?;
        let t = tx;
        let timeline = session.TimelinePropertiesChanged(&TypedEventHandler::<
            Session,
            TimelinePropertiesChangedEventArgs,
        >::new(move |_, _| {
            let _ = t.send(Msg::Refresh);
            Ok(())
        }))?;
        Ok(Self { session: session.clone(), props, playback, timeline })
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let _ = self.session.RemoveMediaPropertiesChanged(self.props);
        let _ = self.session.RemovePlaybackInfoChanged(self.playback);
        let _ = self.session.RemoveTimelinePropertiesChanged(self.timeline);
    }
}

/// `reuse`: the state sent last, on a plain resync. Its cover is kept for the same song instead of being
/// read and decoded again.
fn read_state(session: &Session, reuse: Option<&MediaState>) -> Result<MediaState> {
    let props = session.TryGetMediaPropertiesAsync()?.join()?;
    let info = session.GetPlaybackInfo()?;
    let playing = info.PlaybackStatus()? == Status::Playing;
    let next_enabled = info.Controls()?.IsNextEnabled()?;
    let timeline = session.GetTimelineProperties()?;
    let title = props.Title()?.to_string();
    let artist = props.Artist()?.to_string();

    // The picture is read on every look except a plain resync of the same song: for a moment after a skip
    // a browser offers its own logo, and the real cover only arrives later (the settle reads catch it).
    let art = match reuse {
        Some(old) if old.title == title && old.artist == artist => old.art.clone(),
        _ => props.Thumbnail().ok().and_then(|r| read_art(&r).ok().flatten()),
    };

    // Windows gives the position as it was at `LastUpdatedTime`, not now. Add the time since then
    // while the track plays, or a restarted app starts from a stale minute.
    let mut position = ticks(timeline.Position()?.Duration);
    let duration = ticks(timeline.EndTime()?.Duration - timeline.StartTime()?.Duration);
    if playing && let Ok(updated) = timeline.LastUpdatedTime() {
        position += since(updated.UniversalTime);
        if !duration.is_zero() {
            position = position.min(duration);
        }
    }

    Ok(MediaState {
        title,
        artist,
        album: props.AlbumTitle().map(|a| a.to_string()).unwrap_or_default(),
        playing,
        position,
        duration,
        art,
        has_session: true,
        next_enabled,
        can_seek: seek_allowed(info.Controls()?.IsPlaybackPositionEnabled()?, duration),
    })
}

/// Previous track if the source supports it (Spotify), otherwise restart the current one
/// (single YouTube videos in Chrome don't expose a previous track).
fn skip_previous(s: &Session) -> Result<()> {
    let can_skip = s.GetPlaybackInfo()?.Controls()?.IsPreviousEnabled()?;
    if can_skip {
        s.TrySkipPreviousAsync()?;
    } else {
        s.TryChangePlaybackPositionAsync(0)?;
    }
    Ok(())
}

/// How long ago a WinRT DateTime (100 ns ticks since 1601) was. Zero for a time in the future, and for
/// one so old that it is not a real update time.
fn since(universal: i64) -> Duration {
    const EPOCH_TICKS: i64 = 116_444_736_000_000_000;
    let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) else { return Duration::ZERO };
    let now_ticks = (now.as_nanos() / 100) as i64 + EPOCH_TICKS;
    let age = ticks(now_ticks - universal);
    if age > Duration::from_secs(12 * 3600) { Duration::ZERO } else { age }
}

/// WinRT TimeSpan is in 100 ns ticks.
fn ticks(t: i64) -> Duration {
    Duration::from_nanos(t.max(0) as u64 * 100)
}

/// The most bytes of album art that are read. Real covers are well under 1 MB; a player (or a web page,
/// through the browser's media session) can hand over anything, and a bigger one is left out.
const ART_MAX_BYTES: u64 = 8 * 1024 * 1024;
/// The largest cover that is decoded, across and down, and the most memory the decoding may take.
const ART_MAX_SIDE: u32 = 4096;
const ART_MAX_ALLOC: u64 = 96 * 1024 * 1024;

/// Decode a cover within the limits above (`None` for one that is too big or not a picture).
fn decode_art(bytes: &[u8]) -> Option<image::DynamicImage> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(ART_MAX_SIDE);
    limits.max_image_height = Some(ART_MAX_SIDE);
    limits.max_alloc = Some(ART_MAX_ALLOC);
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    reader.limits(limits);
    reader.decode().ok()
}

fn read_art(r: &IRandomAccessStreamReference) -> Result<Option<Vec<u8>>> {
    let stream = r.OpenReadAsync()?.join()?;
    let size = stream.Size()?;
    if size == 0 || size > ART_MAX_BYTES {
        return Ok(None);
    }
    let size = size as u32;
    let buffer = Buffer::Create(size)?;
    let buffer = stream
        .ReadAsync(&buffer, size, InputStreamOptions::None)?
        .join()?;
    let reader = DataReader::FromBuffer(&buffer)?;
    let mut bytes = vec![0u8; buffer.Length()? as usize];
    reader.ReadBytes(&mut bytes)?;

    let Some(img) = decode_art(&bytes) else {
        return Ok(None);
    };
    let img = img.resize_to_fill(ART_SIZE, ART_SIZE, image::imageops::FilterType::Triangle);
    Ok(Some(img.to_rgba8().into_raw()))
}

pub const fn art_size() -> u32 {
    ART_SIZE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        use image::ImageEncoder;
        let mut out = Vec::new();
        let pixels = vec![200u8; (width * height * 4) as usize];
        image::codecs::png::PngEncoder::new(&mut out).write_image(&pixels, width, height, image::ExtendedColorType::Rgba8).unwrap();
        out
    }

    #[test]
    fn a_cover_is_decoded_only_within_the_limits() {
        assert!(decode_art(&png(64, 64)).is_some());
        assert!(decode_art(&png(ART_MAX_SIDE + 1, 1)).is_none(), "too wide");
        assert!(decode_art(b"not a picture").is_none());
    }

    fn filetime_now() -> i64 {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        (now.as_nanos() / 100) as i64 + 116_444_736_000_000_000
    }

    fn song(position: u64, duration: u64) -> MediaState {
        MediaState {
            title: "Song".into(),
            artist: "Artist".into(),
            has_session: true,
            playing: true,
            position: Duration::from_secs(position),
            duration: Duration::from_secs(duration),
            ..Default::default()
        }
    }

    #[test]
    fn a_player_that_cannot_be_moved_is_not_offered_a_jump() {
        let length = Duration::from_secs(261);
        assert!(seek_allowed(true, length), "Spotify, Chrome and Firefox");
        assert!(!seek_allowed(false, length), "Apple Music says no, with a real length");
        assert!(seek_allowed(false, Duration::ZERO), "no length yet: not judged");
    }

    #[test]
    fn an_empty_timeline_for_the_same_song_keeps_the_position_that_is_shown() {
        let shown = Duration::from_secs(133);
        let kept = song(0, 0).keep_timeline(&song(133, 230), shown, false);
        assert_eq!((kept.position, kept.duration), (shown, Duration::from_secs(230)));
        assert!(kept.playing && kept.has_session && kept.title == "Song", "the rest is what the player said");
        // Firefox's empty timeline has a small position that counts up from a stale moment.
        let counting = MediaState { position: Duration::from_millis(529), ..song(0, 0) };
        assert_eq!(counting.keep_timeline(&song(133, 230), shown, false).position, shown);
    }

    #[test]
    fn a_song_moved_from_here_does_not_believe_an_empty_timeline_even_before_a_real_one_came() {
        let shown = Duration::from_secs(133);
        // Firefox never gave a length, then a click on a lyric line moved the song: its next empty answer is stale.
        let first = MediaState { position: Duration::from_millis(22), ..song(0, 0) };
        let kept = first.keep_timeline(&song(133, 0), shown, true);
        assert_eq!((kept.position, kept.duration), (shown, Duration::ZERO));
    }

    #[test]
    fn a_real_timeline_is_taken_as_it_is() {
        let shown = Duration::from_secs(133);
        // A song that restarts has a length; a seek to the start is a real position.
        let restarted = song(0, 230).keep_timeline(&song(133, 230), shown, true);
        assert_eq!((restarted.position, restarted.duration), (Duration::ZERO, Duration::from_secs(230)));
        let moved = song(90, 230).keep_timeline(&song(133, 230), shown, true);
        assert_eq!(moved.position, Duration::from_secs(90));
    }

    #[test]
    fn an_empty_timeline_for_another_song_or_no_song_is_taken_as_it_is() {
        let shown = Duration::from_secs(133);
        let mut other = song(0, 0);
        other.title = "Another song".into();
        assert_eq!(other.keep_timeline(&song(133, 230), shown, true).duration, Duration::ZERO, "a new song");
        let gone = MediaState::default().keep_timeline(&song(133, 230), shown, true);
        assert!(!gone.has_session && gone.duration.is_zero(), "no player at all");
        // A player that never gave a length, and was not moved from here, is believed: its position is all there is.
        assert_eq!(song(7, 0).keep_timeline(&song(5, 0), shown, false).position, Duration::from_secs(7));
    }

    #[test]
    fn the_age_of_an_update_is_the_time_since_it() {
        let age = since(filetime_now() - 30 * 10_000_000);
        assert!((29..=31).contains(&age.as_secs()), "{age:?}");
    }

    #[test]
    fn a_future_or_ancient_update_time_adds_nothing() {
        assert_eq!(since(filetime_now() + 50_000_000), Duration::ZERO);
        assert_eq!(since(0), Duration::ZERO);
    }
}
