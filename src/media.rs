//! Now-playing info and controls via Windows GSMTC. Runs on its own thread and
//! pushes a fresh `MediaState` to the UI whenever something changes.

use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

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
        let _ = tx.send(Msg::Refresh);

        while let Ok(msg) = rx.recv() {
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
                }
                Msg::Refresh => {
                    // Re-subscribe when the active session changes.
                    subscribed = None;
                    if let Some(s) = &session {
                        subscribed = Subscription::new(s, tx.clone()).ok();
                    }
                    let state = session
                        .as_ref()
                        .and_then(|s| read_state(s).ok())
                        .unwrap_or_default();
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

fn read_state(session: &Session) -> Result<MediaState> {
    let props = session.TryGetMediaPropertiesAsync()?.join()?;
    let info = session.GetPlaybackInfo()?;
    let playing = info.PlaybackStatus()? == Status::Playing;
    let next_enabled = info.Controls()?.IsNextEnabled()?;
    let timeline = session.GetTimelineProperties()?;

    let art = props
        .Thumbnail()
        .ok()
        .and_then(|r| read_art(&r).ok().flatten());

    Ok(MediaState {
        title: props.Title()?.to_string(),
        artist: props.Artist()?.to_string(),
        album: props.AlbumTitle().map(|a| a.to_string()).unwrap_or_default(),
        playing,
        position: ticks(timeline.Position()?.Duration),
        duration: ticks(timeline.EndTime()?.Duration - timeline.StartTime()?.Duration),
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
