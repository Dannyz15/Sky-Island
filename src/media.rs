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
use windows::core::{HRESULT, Result};
use winit::event_loop::EventLoopProxy;

use crate::UserEvent;

/// The longest a call to a player or to Windows is waited for. A player that never answers must not freeze the
/// media thread for good, and with it every now-playing update.
const CALL_LIMIT: Duration = Duration::from_secs(10);

/// ERROR_TIMEOUT as an HRESULT.
const TIMED_OUT: HRESULT = HRESULT(0x8007_05B4_u32 as i32);

/// The result of a finished Windows call on its way from the thread that finished it to the one that waits.
/// Some of these objects are not marked as sendable by the `windows` crate, but they are free-threaded here: the media
/// thread is in the multithreaded apartment, and `join()` hands them over from the finishing thread the same way.
struct Carried<T>(Result<T>);
// SAFETY: see above; the object is used only by the waiting thread after it is handed over.
unsafe impl<T> Send for Carried<T> {}

/// The result of a Windows call (`when` hands over a function that is called with it), waited for at most `limit`.
/// After that `cancel` is called and an error comes back.
fn within<T: 'static>(
    when: impl FnOnce(Box<dyn FnOnce(Result<T>) + Send>) -> Result<()>,
    cancel: impl FnOnce(),
    limit: Duration,
) -> Result<T> {
    let (tx, rx) = mpsc::channel();
    when(Box::new(move |result| {
        let _ = tx.send(Carried(result));
    }))?;
    match rx.recv_timeout(limit) {
        Ok(Carried(result)) => result,
        Err(_) => {
            cancel();
            Err(TIMED_OUT.into())
        }
    }
}

/// `op.join()` for a Windows async operation, but it gives up after `limit` and cancels the call.
macro_rules! join_within {
    ($op:expr, $limit:expr) => {{
        let op = $op;
        within(|done| op.when(done), || drop(op.Cancel()), $limit)
    }};
}


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
            match Manager::RequestAsync().and_then(|op| join_within!(op, CALL_LIMIT)) {
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

/// The most characters of a title, artist or album that are kept.
const MAX_META_CHARS: usize = 256;

fn clip_meta(s: &str) -> String {
    crate::lyrics::clip(s, MAX_META_CHARS).to_string()
}

/// `reuse`: the state sent last, on a plain resync. Its cover is kept for the same song instead of being
/// read and decoded again.
fn read_state(session: &Session, reuse: Option<&MediaState>) -> Result<MediaState> {
    let props = join_within!(session.TryGetMediaPropertiesAsync()?, CALL_LIMIT)?;
    let info = session.GetPlaybackInfo()?;
    let playing = info.PlaybackStatus()? == Status::Playing;
    let next_enabled = info.Controls()?.IsNextEnabled()?;
    let timeline = session.GetTimelineProperties()?;
    // A player or a web page decides these texts, so they are cut at once; nothing shows more than a few dozen.
    let title = clip_meta(&props.Title()?.to_string());
    let artist = clip_meta(&props.Artist()?.to_string());

    // The picture is read on every look except a plain resync of the same song: for a moment after a skip
    // a browser offers its own logo, and the real cover only arrives later (the settle reads catch it).
    let art = match reuse {
        Some(old) if old.title == title && old.artist == artist => old.art.clone(),
        _ => props.Thumbnail().ok().and_then(|r| read_art(&r).ok().flatten()),
    };

    // Windows gives the position as it was at `LastUpdatedTime`, not now. Add the time since then
    // while the track plays, or a restarted app starts from a stale minute.
    let mut position = ticks(timeline.Position()?.Duration);
    let duration = ticks(timeline.EndTime()?.Duration.saturating_sub(timeline.StartTime()?.Duration));
    if playing && let Ok(updated) = timeline.LastUpdatedTime() {
        position = position.saturating_add(since(updated.UniversalTime));
        if !duration.is_zero() {
            position = position.min(duration);
        }
    }

    Ok(MediaState {
        title,
        artist,
        album: props.AlbumTitle().map(|a| clip_meta(&a.to_string())).unwrap_or_default(),
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
    let age = ticks(now_ticks.saturating_sub(universal));
    if age > Duration::from_secs(12 * 3600) { Duration::ZERO } else { age }
}

/// WinRT TimeSpan is in 100 ns ticks.
fn ticks(t: i64) -> Duration {
    // A player can send any number here; a huge one must not overflow.
    Duration::from_nanos((t.max(0) as u64).saturating_mul(100))
}

/// The most bytes of album art that are read. Real covers are well under 1 MB; a player (or a web page,
/// through the browser's media session) can hand over anything, and a bigger one is left out.
const ART_MAX_BYTES: u64 = 8 * 1024 * 1024;
/// The largest cover that is decoded, across and down, and the most memory the decoding may take.
const ART_MAX_SIDE: u32 = 4096;
const ART_MAX_ALLOC: u64 = 96 * 1024 * 1024;

/// Decode a cover within the limits above (`None` for one that is too big or not a picture).
fn decode_art(bytes: &[u8]) -> Option<image::DynamicImage> {
    crate::gfx::decode_limited(bytes, ART_MAX_SIDE, ART_MAX_ALLOC)
}

/// The cover as a square picture of `ART_SIZE`: the middle square is cut out FIRST, and only that is scaled.
/// (`resize_to_fill` scales the whole picture to fill the square and cuts afterwards, so a cover of 4096 x 1 pixels
/// was first made 393216 x 96 pixels.)
fn cover_square(img: image::DynamicImage) -> image::DynamicImage {
    let (w, h) = (img.width(), img.height());
    let side = w.min(h);
    if side == 0 {
        return image::DynamicImage::new_rgba8(ART_SIZE, ART_SIZE);
    }
    img.crop_imm((w - side) / 2, (h - side) / 2, side, side).resize_exact(ART_SIZE, ART_SIZE, image::imageops::FilterType::Triangle)
}

fn read_art(r: &IRandomAccessStreamReference) -> Result<Option<Vec<u8>>> {
    let stream = join_within!(r.OpenReadAsync()?, CALL_LIMIT)?;
    let size = stream.Size()?;
    if size == 0 || size > ART_MAX_BYTES {
        return Ok(None);
    }
    let size = size as u32;
    let buffer = Buffer::Create(size)?;
    let buffer = join_within!(stream.ReadAsync(&buffer, size, InputStreamOptions::None)?, CALL_LIMIT)?;
    let reader = DataReader::FromBuffer(&buffer)?;
    let mut bytes = vec![0u8; buffer.Length()? as usize];
    reader.ReadBytes(&mut bytes)?;

    let Some(img) = decode_art(&bytes) else {
        return Ok(None);
    };
    Ok(Some(cover_square(img).to_rgba8().into_raw()))
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
    fn strange_times_from_a_player_do_not_overflow() {
        assert_eq!(ticks(10_000_000), Duration::from_secs(1));
        assert_eq!(ticks(-5), Duration::ZERO, "a negative time is nothing");
        assert_eq!(ticks(i64::MAX), Duration::from_nanos(u64::MAX), "a huge one is held to the longest time there is");
        assert_eq!(since(i64::MIN), Duration::ZERO, "a time far in the past is not a real update time");
        assert_eq!(since(i64::MAX), Duration::ZERO);
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

    /// What `within` hands to a call so that it can report back.
    type Done = Box<dyn FnOnce(Result<i32>) + Send>;

    fn answered_later(value: i32, after_ms: u64) -> impl FnOnce(Done) -> Result<()> {
        move |done| {
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(after_ms));
                done(Ok(value));
            });
            Ok(())
        }
    }

    #[test]
    fn a_call_that_answers_in_time_gives_its_answer() {
        let cancelled = std::cell::Cell::new(false);
        let got = within(answered_later(7, 0), || cancelled.set(true), Duration::from_secs(5));
        assert_eq!(got.unwrap(), 7);
        assert!(!cancelled.get(), "nothing to cancel");
    }

    #[test]
    fn a_call_that_never_answers_is_given_up_on_and_cancelled() {
        let cancelled = std::cell::Cell::new(false);
        let started = Instant::now();
        let got = within(answered_later(7, 3000), || cancelled.set(true), Duration::from_millis(150));
        assert_eq!(got.unwrap_err().code(), TIMED_OUT);
        assert!(cancelled.get(), "the call is cancelled");
        assert!(started.elapsed() < Duration::from_secs(2), "it did not wait for the slow answer");
    }

    #[test]
    fn an_error_from_the_call_is_passed_on() {
        let failing = |done: Done| {
            done(Err(windows::core::Error::from_hresult(HRESULT(0x8000_4005_u32 as i32))));
            Ok(())
        };
        assert_eq!(within(failing, || {}, Duration::from_secs(5)).unwrap_err().code(), HRESULT(0x8000_4005_u32 as i32));
        // A call that cannot even be started fails at once.
        let refused = |_done: Done| -> Result<()> { Err(TIMED_OUT.into()) };
        assert!(within(refused, || {}, Duration::from_secs(5)).is_err());
    }

    fn picture(width: u32, height: u32, colour: impl Fn(u32, u32) -> [u8; 4]) -> image::DynamicImage {
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(width, height, |x, y| image::Rgba(colour(x, y))))
    }

    /// The audit's worst case: a strip of 4096 x 1 pixels used to be scaled to 393216 x 96 before it was cut.
    #[test]
    fn a_cover_is_cut_to_a_square_before_it_is_scaled() {
        let started = Instant::now();
        for (w, h) in [(4096, 1), (1, 4096), (200, 100), (100, 200), (96, 96), (1, 1), (5000, 3)] {
            let out = cover_square(picture(w, h, |_, _| [10, 20, 30, 255]));
            assert_eq!((out.width(), out.height()), (ART_SIZE, ART_SIZE), "{w} x {h}");
        }
        assert!(started.elapsed() < Duration::from_secs(5), "took {:?}", started.elapsed());
    }

    #[test]
    fn the_middle_of_a_wide_cover_is_kept() {
        // Red on the left half, blue on the right: the middle square holds a quarter of each side of the middle line.
        let out = cover_square(picture(200, 100, |x, _| if x < 100 { [255, 0, 0, 255] } else { [0, 0, 255, 255] })).to_rgba8();
        assert!(out.get_pixel(5, 48)[0] > 200 && out.get_pixel(5, 48)[2] < 50, "left is red");
        assert!(out.get_pixel(90, 48)[2] > 200 && out.get_pixel(90, 48)[0] < 50, "right is blue");
    }

    #[test]
    fn long_titles_from_a_player_are_cut_to_a_small_size() {
        assert_eq!(clip_meta("Short"), "Short");
        let cut = clip_meta(&"\u{4E2D}".repeat(100_000));
        assert_eq!(cut.chars().count(), MAX_META_CHARS);
    }
}
