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
}

impl MediaState {
    /// Nothing the UI shows differs between the two.
    fn same_as(&self, o: &MediaState) -> bool {
        (&self.title, &self.artist, &self.album, self.playing, self.position, self.duration)
            == (&o.title, &o.artist, &o.album, o.playing, o.position, o.duration)
            && (self.has_session, self.next_enabled, &self.art) == (o.has_session, o.next_enabled, &o.art)
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
        let Ok(manager) = Manager::RequestAsync().and_then(|op| op.join()) else {
            return;
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
            let wait = settle.iter().map(|t| t.saturating_duration_since(now)).min().unwrap_or(RESYNC).min(RESYNC);
            // A message from Windows or the UI, or a timer that says to read again.
            let (msg, resubscribe, quiet) = match rx.recv_timeout(wait) {
                Ok(msg) => (msg, true, false),
                Err(RecvTimeoutError::Timeout) => (Msg::Refresh, false, resync_only),
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
                    // Re-subscribe when the active session changes.
                    if resubscribe {
                        subscribed = None;
                        if let Some(s) = &session {
                            subscribed = Subscription::new(s, tx.clone()).ok();
                        }
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

fn read_art(r: &IRandomAccessStreamReference) -> Result<Option<Vec<u8>>> {
    let stream = r.OpenReadAsync()?.join()?;
    let size = stream.Size()? as u32;
    if size == 0 {
        return Ok(None);
    }
    let buffer = Buffer::Create(size)?;
    let buffer = stream
        .ReadAsync(&buffer, size, InputStreamOptions::None)?
        .join()?;
    let reader = DataReader::FromBuffer(&buffer)?;
    let mut bytes = vec![0u8; buffer.Length()? as usize];
    reader.ReadBytes(&mut bytes)?;

    let Ok(img) = image::load_from_memory(&bytes) else {
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

    fn filetime_now() -> i64 {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        (now.as_nanos() / 100) as i64 + 116_444_736_000_000_000
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
