// A GUI program: without this Windows opens a console window next to it.
#![windows_subsystem = "windows"]
#![allow(clippy::too_many_arguments)]

mod appicon;
mod calendar;
mod clipboard;
mod dragdrop;
mod edge;
mod editor;
mod gfx;
mod hotkey;
mod icon;
mod localtime;
mod location;
mod loopback;
mod lyrics;
mod media;
mod motion;
mod msgwin;
mod net;
mod open;
mod panel;
mod prayer;
mod presence;
mod settings;
mod shelf;
mod sound;
mod stats;
mod store;
mod sys;
mod theme;
mod timer;
mod todo;
mod ui;
mod update;
mod water;
mod weather;
mod zones;

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use fontdue::{Font, FontSettings};
use softbuffer::{Context, Surface};
use tiny_skia::{Color, FillRule, FilterQuality, LineCap, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke, Transform};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::platform::windows::WindowAttributesExtWindows;
use winit::window::{Theme, Window, WindowId, WindowLevel};

use clipboard::History;
use lyrics::Line;
use media::{Command, MediaHandle, MediaState};
use motion::{Glows, spring};
use panel::Panel;
use presence::Presence;
use settings::{EyeStyle, Settings, ThemeMode};
use shelf::{COLS, ROWS, Shelf};
use timer::{Timer, Unit};
use todo::Todo;
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use ui::*;
use water::Water;

pub enum UserEvent {
    Media(MediaState),
    /// `online` is whether looking lyrics up online was on when the look began; an answer that began under the other
    /// setting is not shown.
    Lyrics {
        key: String,
        fetched: lyrics::Fetched,
        online: bool,
    },
    Notice(sys::Notice),
    /// A lock key was pressed; its state is read on the UI thread.
    LockKey {
        name: &'static str,
        vk: i32,
    },
    Presence {
        presence: Presence,
        fullscreen: presence::Fullscreen,
    },
    Menu(MenuId),
    /// Text or a picture was copied to the clipboard.
    Clip(clipboard::Content),
    /// How loud the default output device is (percent, muted), without a notice: for the volume slider.
    SysVolume {
        percent: u32,
        muted: bool,
    },
    /// With Show on All screens: the mouse touched the top edge of this screen (its number) where the notch would be.
    FollowTo(u32),
    /// The global hotkey was pressed.
    Hotkey,
    /// Whether the combination chosen for the hotkey could be taken (another program may have it).
    HotkeyStatus(bool),
    /// A screen was plugged in or out, changed its size or position, or the taskbar moved.
    Screens,
    /// Shelf files that were found missing when the app started.
    ShelfMissing(Vec<PathBuf>),
    /// A shelf file's icon, loaded off the UI thread; `None` when the shell has none.
    Icon {
        path: PathBuf,
        icon: Option<Pixmap>,
    },
    /// The answer to "Check for updates".
    Update(update::Status),
    /// The answer to a check the app made by itself (start, every few hours).
    AutoUpdate(update::Status),
    /// The releases and their notes, for the changelog in the settings window.
    Changelog(Option<Vec<update::Release>>),
    /// A forecast for the user's city, or `None` when fetching it did not work.
    Weather(Option<weather::Report>),
    /// The prayer times for the user's city, or `None` when fetching them did not work.
    Prayer(Option<prayer::Table>),
    /// The city a search in the settings window found.
    Place(Result<weather::Place, weather::SearchError>),
    /// The place "Detect my location" found, and how well.
    Located(location::Found),
}

#[derive(PartialEq, Clone, Copy)]
enum View {
    Player,
    Lyrics,
    Timer,
    Water,
    Weather,
    Prayer,
    Stats,
    Shelf,
    Todo,
    Calendar,
    Clipboard,
}

struct App {
    window: Option<Rc<Window>>,
    surface: Option<Surface<Rc<Window>, Rc<Window>>>,
    font: Option<Font>,
    /// A semibold face for titles; falls back to `font`.
    bold: Option<Font>,
    media: MediaState,
    /// Playing a song: when its state was last read, the handle to control the player, where the bar is being dragged to, and whether Sky Island moved the song itself.
    play: ui::Playback,
    view: View,
    /// The notch's springs: how far each view is from the player's height, the extra height for what is in the view, the closed notch's size, how open the notch is, and how far it is tucked away.
    spring: ui::Springs,
    /// x
    lyric: ui::LyricsView,
    timer: Timer,
    /// Typing a number into the timer.
    timer_entry: ui::TimerEntry,
    water: Water,
    shelf: Shelf,
    /// x
    shelf_ui: ui::ShelfUi,
    /// The scale the Size setting gives the whole island (1.0 is normal), and the width of the screen in
    /// logical pixels, which Auto goes by.
    ui_k: f64,
    screen_w: f64,
    /// The mouse: where it is, whether it is over the notch, when entering or leaving takes effect, and when the wheel last changed the tab.
    pointer: ui::Pointer,
    /// Notices: the system notice on show and its fade, and the timer alert that keeps the notch open with its chime.
    notices: ui::Notices,
    clips: History,
    /// A short message for the clipboard header, with when to remove it.
    clip_flash: Option<(&'static str, Instant)>,
    /// The tray icon and its menu.
    tray: ui::Tray,
    todo: Todo,
    cal: calendar::Calendar,
    /// Typing a task: whether the notch is taking keys for it, the text so far, and the window to give the keyboard back to.
    edit: ui::Editing,
    settings: Settings,
    /// Cached "start with Windows" state from the registry.
    autostart: bool,
    /// Whether the notch is shown: awake or dozing, hidden for a full-screen app, the monitor it is on, and the screen it last followed the mouse to.
    vis: ui::Visibility,
    /// x
    eyes: ui::Eyes,
    last_minute: u32,
    quit: bool,
    last_ui_tick: Instant,
    /// The closed notch's width for the album cover and bars while a song plays, and the bars' clock.
    music: ui::ClosedMusic,
    proxy: EventLoopProxy<UserEvent>,
    /// The song's cover colours: the tint taken from the cover and its glide, and the fade-in of a new track.
    art: ui::Art,
    /// The highlight under the tabs.
    tab_bar: ui::TabHighlight,
    /// Lists and the calendar easing in after they changed: how far along they are, the way they come from, and what the list was.
    slides: ui::Slides,
    /// The change from one view to another: how far the new view has taken over, the view fading out, and the way the new one slides from.
    switch: ui::ViewSwitch,
    /// Hover glows, switch knobs and other small values that glide towards their goal.
    glows: Glows,
    /// Frame pacing: when the last frame was drawn, how long one frame is, when the screen's refresh rate was last read, and when the next frame is due.
    pacing: ui::Pacing,
    /// The open notch's background (body, glow and border), kept while nothing about it changes.
    body: Option<(ui::BodyKey, Pixmap)>,
    /// Windows is using its light theme (apps), as last reported by the system.
    system_light: bool,
    /// The settings window, while it is open.
    panel: Option<Panel>,
    /// With Show on = Every: the copies of the closed notch on the other screens (`ui/mirrors.rs`).
    mirrors: ui::Mirrors,
    /// Open the settings window the next time the event loop is free to (a click cannot create a window).
    panel_wanted: bool,
    /// The page the settings window should show when it opens.
    panel_page: Option<panel::Page>,
    /// Updates: how the last check went, the version the notch has already told about, and the changelog.
    updates: ui::Updates,
    /// The forecast the Weather tab shows, when it was fetched, whether the last fetch is still on its way or failed,
    /// and the thread that fetches it (told which place to watch; `None` before the window exists).
    weather: ui::WeatherTab,
    /// The prayer times the Prayer tab shows, the same bookkeeping as the weather, the prayer a notice was last
    /// shown for (day and index), and the minute the Prayer view was last drawn.
    prayer: ui::PrayerTab,
    /// The Music view's volume slider.
    volume: ui::Volume,
    /// Whether the hotkey's combination is registered.
    hotkey_ok: bool,
    /// What the Stats tab keeps.
    stats: ui::StatsTab,
    last_tick: Instant,
}

impl App {
    fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        let settings = Settings::load();
        Self {
            window: None,
            surface: None,
            font: load_font(),
            bold: load_bold_font(),
            media: MediaState::default(),
            play: ui::Playback::default(),
            view: View::Player,
            spring: ui::Springs::new(settings.hide_island),
            lyric: ui::LyricsView::default(),
            timer: Timer::load(),
            timer_entry: ui::TimerEntry::default(),
            water: Water::new(today(), Instant::now()),
            shelf: {
                let mut shelf = Shelf::new();
                if settings.empty_shelf {
                    shelf.clear();
                }
                shelf
            },
            shelf_ui: ui::ShelfUi::default(),
            ui_k: 1.0,
            screen_w: 1536.0,
            pointer: ui::Pointer::default(),
            notices: ui::Notices::default(),
            clips: {
                let mut clips = History::new();
                clips.load_pinned(clipboard::read_pinned());
                clips
            },
            clip_flash: None,
            tray: ui::Tray::default(),
            todo: Todo::new(),
            cal: calendar::Calendar::new(today_day()),
            edit: ui::Editing::default(),
            settings: settings.clone(),
            autostart: settings::repair_autostart(),
            vis: ui::Visibility::default(),
            eyes: ui::Eyes::default(),
            last_minute: u32::MAX,
            quit: false,
            last_ui_tick: Instant::now(),
            music: ui::ClosedMusic::default(),
            proxy,
            art: ui::Art::default(),
            tab_bar: ui::TabHighlight::default(),
            slides: ui::Slides::default(),
            switch: ui::ViewSwitch::default(),
            glows: Glows::default(),
            pacing: ui::Pacing::default(),
            body: None,
            system_light: false,
            panel: None,
            mirrors: ui::Mirrors::default(),
            panel_wanted: false,
            panel_page: None,
            updates: ui::Updates::default(),
            hotkey_ok: true,
            stats: ui::StatsTab::default(),
            volume: ui::Volume::default(),
            weather: ui::WeatherTab::default(),
            prayer: ui::PrayerTab::default(),
            last_tick: Instant::now(),
        }
    }
}

/// Keep the earliest of the wake-up times asked for.
fn earliest(wake: &mut Option<Instant>, t: Instant) {
    *wake = Some(wake.map_or(t, |w| w.min(t)));
}

/// The most a crash log may hold. When a new report would make it longer, the oldest ones are cut off.
const CRASH_LOG_MAX: usize = 64 * 1024;

/// "2026-11-05 10:34:07 UTC" for seconds since 1970.
fn utc_text(unix: u64) -> String {
    let (year, month, day) = calendar::civil_from_days((unix / 86_400) as i64);
    let secs = unix % 86_400;
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC", secs / 3600, secs % 3600 / 60, secs % 60)
}

/// One report for the crash log: which version, when, on which thread, what went wrong and where.
fn crash_entry(version: &str, unix: u64, thread: &str, what: &str, backtrace: &str) -> String {
    format!("---- {} | Sky Island {version} | thread {thread} ----\n{what}\n{}\n\n", utc_text(unix), backtrace.trim_end())
}

/// The log with a new entry added at the end, and the oldest entries cut off to stay within `max` bytes (the newest
/// entry is always kept whole, and the cut falls between entries when it can).
fn crash_log_with(old: &str, entry: &str, max: usize) -> String {
    let mut log = String::from(old);
    log.push_str(entry);
    if log.len() <= max {
        return log;
    }
    let mut start = log.len().saturating_sub(max).min(log.len() - entry.len().min(log.len()));
    while !log.is_char_boundary(start) {
        start += 1;
    }
    if let Some(next) = log[start..].find("---- ") {
        start += next;
    }
    log[start..].to_string()
}

/// Add a report to `crash.log` in `dir`, keeping the earlier ones (within the size limit).
fn record_crash(dir: &std::path::Path, entry: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("crash.log");
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    std::fs::write(&path, crash_log_with(&old, entry, CRASH_LOG_MAX))
}

/// With no console, a panic would vanish silently. Write it to a file the user can find and send: what went wrong and
/// where (the file and line come with the panic itself), the version, the time and the thread. A trace of the calls is
/// added too, but in the shipped exe its frames are not named (Windows keeps names in a separate .pdb file that users
/// do not have), so the panic's own file and line are what to go by. Reports are added one after the other, up to a size
/// limit, so a second crash does not wipe out the first.
fn install_crash_log() {
    std::panic::set_hook(Box::new(|info| {
        let Some(appdata) = std::env::var_os("APPDATA") else { return };
        let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let thread = std::thread::current();
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        let entry = crash_entry(env!("CARGO_PKG_VERSION"), unix, thread.name().unwrap_or("unnamed"), &info.to_string(), &backtrace);
        let _ = record_crash(&PathBuf::from(appdata).join("Sky-Island"), &entry);
    }));
}

/// The name of the lock that says "a copy is running": one per data folder, so two copies never write the
/// same files, while a copy pointed at another folder (for testing) is its own.
fn instance_name(data_dir: &str) -> String {
    let hash = data_dir.to_lowercase().bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    format!("Local\\SkyIsland-{hash:016x}")
}

/// Takes the "a copy is running" lock; false when another copy of Sky Island already holds it.
fn first_instance() -> bool {
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::core::HSTRING;

    let data_dir = std::env::var("APPDATA").unwrap_or_default();
    unsafe {
        // The handle is kept until the process ends, which is what holds the lock.
        match CreateMutexW(None, false, &HSTRING::from(instance_name(&data_dir))) {
            Ok(_) => GetLastError() != ERROR_ALREADY_EXISTS,
            Err(_) => true, // cannot tell: better two copies than none
        }
    }
}

fn main() {
    if !first_instance() {
        return;
    }
    install_crash_log();
    let event_loop = EventLoop::<UserEvent>::with_user_event().build().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new(event_loop.create_proxy());
    event_loop.run_app(&mut app).expect("run app");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crash_report_says_which_version_when_and_where() {
        assert_eq!(utc_text(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(utc_text(1_793_871_247), "2026-11-05 09:34:07 UTC");
        let entry = crash_entry("1.5.8", 1_793_871_247, "main", "panicked at src/x.rs:1:1:\nboom", "  0: a\n  1: b\n");
        assert!(entry.starts_with("---- 2026-11-05 09:34:07 UTC | Sky Island 1.5.8 | thread main ----\n"), "{entry}");
        assert!(entry.contains("boom") && entry.contains("1: b") && entry.ends_with("\n\n"));
    }

    #[test]
    fn a_second_crash_is_added_and_the_oldest_are_cut_when_the_log_is_too_long() {
        let entry = |n: usize| crash_entry("1.5.8", 0, "main", &format!("report {n} {}", "x".repeat(200)), "");
        let log = crash_log_with(&crash_log_with("", &entry(1), 10_000), &entry(2), 10_000);
        assert!(log.contains("report 1") && log.contains("report 2"), "both are kept while there is room");
        assert!(log.find("report 1") < log.find("report 2"), "oldest first");

        let mut long = String::new();
        for n in 0..50 {
            long = crash_log_with(&long, &entry(n), 2_000);
        }
        assert!(long.len() <= 2_000, "{}", long.len());
        assert!(long.starts_with("---- "), "the cut falls between reports");
        assert!(long.contains("report 49") && !long.contains("report 0 "), "the newest stays, the oldest went");

        let huge = crash_entry("1.5.8", 0, "main", &"y".repeat(5_000), "");
        let kept = crash_log_with("old stuff\n", &huge, 1_000);
        assert!(kept.contains("yyyy") && !kept.contains("old stuff"), "a report bigger than the limit is still written");
    }

    #[test]
    fn crashes_are_written_to_the_log_one_after_the_other() {
        let dir = std::env::temp_dir().join(format!("sky-island-crash-test-{}", std::process::id())).join("Sky-Island");
        let _ = std::fs::remove_dir_all(&dir);
        record_crash(&dir, &crash_entry("1.5.8", 1, "main", "first", "")).unwrap();
        record_crash(&dir, &crash_entry("1.5.8", 2, "lyrics", "second", "")).unwrap();
        let text = std::fs::read_to_string(dir.join("crash.log")).unwrap();
        assert!(text.find("first") < text.find("second") && text.contains("thread lyrics"), "{text}");
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn each_data_folder_has_its_own_instance_lock() {
        let a = instance_name(r"C:\Users\A\AppData\Roaming");
        assert_eq!(a, instance_name(r"c:\users\a\appdata\roaming"), "case does not matter on Windows");
        assert_ne!(a, instance_name(r"C:\Users\B\AppData\Roaming"));
        assert!(a.starts_with("Local\\SkyIsland-"));
    }
}
