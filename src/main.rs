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
mod location;
mod localtime;
mod loopback;
mod lyrics;
mod media;
mod motion;
mod msgwin;
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
use tiny_skia::{
    Color, FillRule, FilterQuality, LineCap, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke,
    Transform,
};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::keyboard::{Key, NamedKey};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::platform::windows::WindowAttributesExtWindows;
use winit::window::{Theme, Window, WindowId, WindowLevel};

use lyrics::Line;
use media::{Command, MediaHandle, MediaState};
use clipboard::History;
use timer::{Timer, Unit};
use presence::Presence;
use panel::Panel;
use settings::{EyeStyle, Settings, ThemeMode};
use shelf::{COLS, ROWS, Shelf};
use todo::Todo;
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use water::Water;
use motion::{Glows, spring};
use ui::*;

pub enum UserEvent {
    Media(MediaState),
    /// `online` is whether looking lyrics up online was on when the look began; an answer that began under the other
    /// setting is not shown.
    Lyrics { key: String, fetched: lyrics::Fetched, online: bool },
    Notice(sys::Notice),
    /// A lock key was pressed; its state is read on the UI thread.
    LockKey { name: &'static str, vk: i32 },
    Presence { presence: Presence, fullscreen: presence::Fullscreen },
    Menu(MenuId),
    /// Text or a picture was copied to the clipboard.
    Clip(clipboard::Content),
    /// How loud the default output device is (percent, muted), without a notice: for the volume slider.
    SysVolume { percent: u32, muted: bool },
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
    Icon { path: PathBuf, icon: Option<Pixmap> },
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
    media_at: Instant,
    media_handle: Option<MediaHandle>,
    view: View,
    /// 0.0 = player height, 1.0 = lyrics height
    view_t: f64,
    view_v: f64,
    /// How much taller than normal the open notch is for what is in its view (the Stats view with many drives), in
    /// logical pixels, and how fast that is changing.
    grow_t: f64,
    grow_v: f64,
    lyrics: LyricsState,
    lyrics_key: String,
    /// The song length the lyrics were looked up with, and whether they were looked up again once a
    /// different length arrived (the browser often reports the length a moment after the title).
    lyrics_duration: Duration,
    lyrics_refetched: bool,
    /// When to look the lyrics up again after the site could not be reached.
    lyrics_retry_at: Option<Instant>,
    /// The line (fractional) in the middle of the lyrics view. A spring follows the line being sung, or
    /// wherever the wheel took it.
    lyric_pos: f64,
    lyric_v: f64,
    /// Set while the lyrics were scrolled by hand: the line the wheel moved to. `None` follows the music.
    lyric_manual: Option<f64>,
    timer: Timer,
    /// The timer number being typed (index into `Unit::ALL`), and the digits typed so far.
    timer_field: Option<usize>,
    timer_input: String,
    water: Water,
    shelf: Shelf,
    /// Shell icons for shelf files; `None` when the shell has no icon for a file.
    icons: HashMap<PathBuf, Option<Pixmap>>,
    /// Where to send a file whose icon is wanted; the thread that loads them starts with the first one.
    icon_loader: Option<std::sync::mpsc::Sender<PathBuf>>,
    /// A file is being dragged over the notch from outside.
    drag_hover: bool,
    /// Shelf item index and cursor position where the mouse went down.
    press: Option<(usize, (f64, f64))>,
    /// Shelf files highlighted with Ctrl+click; dragging one of them takes all of them.
    selected: Vec<PathBuf>,
    /// The notch took the keyboard for the shelf's Ctrl+A and Esc, so it hands it back when the highlight ends.
    selecting: bool,
    /// Where the progress bar is being dragged to; the song jumps there when the button is let go.
    scrub: Option<Duration>,
    /// The scale the Size setting gives the whole island (1.0 is normal), and the width of the screen in
    /// logical pixels, which Auto goes by.
    ui_k: f64,
    screen_w: f64,
    /// How much of its normal size the closed notch has (smaller for the minimal notch), with its speed.
    closed_k: f64,
    closed_v: f64,
    /// When the mouse wheel last changed the tab, so one flick moves one tab.
    last_wheel_tab: Instant,
    /// The lyrics just opened or closed: the notch changes width under the mouse, so the system's
    /// "mouse left" is ignored until it has settled.
    lyric_settle: bool,
    /// Sky Island moved the song itself (a click on a lyric line or on the bar) and the song has not changed since.
    seeked: bool,
    /// Latest system notice, kept after it expires so it can fade out.
    notice: Option<sys::Notice>,
    notice_until: Option<Instant>,
    /// 0.0 = no notice, 1.0 = compact notice size
    ov_t: f64,
    ov_v: f64,
    clips: History,
    /// A short message for the clipboard header, with when to remove it.
    clip_flash: Option<(&'static str, Instant)>,
    /// When the cursor entering/leaving takes effect (see `hover_open`, `hover_close`).
    enter_at: Option<Instant>,
    leave_at: Option<Instant>,
    tray: Option<TrayIcon>,
    tray_settings_id: Option<MenuId>,
    tray_quit_id: Option<MenuId>,
    /// The tray's Show / Hide island entry, whose text follows the setting.
    tray_about_id: Option<MenuId>,
    todo: Todo,
    cal: calendar::Calendar,
    /// Typing a new task. While true the notch stays open and keys go to `input`.
    editing: bool,
    input: String,
    edit_started: Instant,
    /// The day and reminder time of the task being added or edited.
    editor: editor::Editor,
    /// The window that had focus before we took it for typing, so it can be given back.
    prev_foreground: Option<windows::Win32::Foundation::HWND>,
    settings: Settings,
    /// Cached "start with Windows" state from the registry.
    autostart: bool,
    presence: Presence,
    fullscreen: presence::Fullscreen,
    /// Where the monitor the notch is on lies, so only a full-screen app on that monitor hides it.
    monitor_area: Option<presence::Area>,
    /// The window is hidden because a full-screen app is in front.
    hidden: bool,
    greeting_until: Option<Instant>,
    /// The closed notch shows the eyes until then, after an Eye style is picked in the settings window.
    eye_preview_until: Option<Instant>,
    next_blink: Instant,
    /// When the blink in progress began (it lasts `eyes::blink_len`), if the eyes are blinking.
    blink_since: Option<Instant>,
    /// Where the eyes look now, how fast that is changing, and where they are turning to (across, down; -1 to 1).
    look: (f64, f64),
    look_v: (f64, f64),
    look_goal: (f64, f64),
    /// When the eyes next look somewhere without a blink, and how many times they have.
    next_glance: Instant,
    glances: u32,
    blink_count: u32,
    next_zz: Instant,
    zz_frame: bool,
    last_minute: u32,
    quit: bool,
    /// While set, the notch stays open to show a timer notification.
    alert_until: Option<Instant>,
    alert_msg: String,
    last_ui_tick: Instant,
    /// How wide the closed notch is for the album cover and bars (0 = plain, 1 = with them), and how fast that changes;
    /// and when the moving bars were last drawn.
    music_k: f64,
    music_v: f64,
    /// How much wider than usual the closed notch is for them, in logical pixels, and how fast that changes.
    music_w: f64,
    music_wv: f64,
    bars_at: Instant,
    proxy: EventLoopProxy<UserEvent>,
    cursor: (f64, f64),
    hovered: bool,
    /// 0.0 = collapsed, 1.0 = expanded. A spring, so it can swing a little past 1.0 when it opens.
    progress: f64,
    progress_v: f64,
    /// 0.0 = in place, 1.0 = slid up out of view (the "Auto-hide notch" setting).
    tuck_t: f64,
    tuck_v: f64,
    /// The `tuck_target` the animation is heading for, to restart its clock when that changes.
    tuck_goal: f64,
    /// Colour of the current song's cover art, used to tint the player.
    art_tint: [u8; 3],
    /// Which song and cover `art_tint` was worked out for.
    art_key: String,
    /// The tab highlight's position, as a (fractional) tab index, so it can slide between tabs.
    tab_pos: f32,
    /// How fast it is moving, in tabs per second; the highlight stretches while it moves.
    tab_vel: f32,
    /// Rows of a list ease in after the list changed (it scrolled, an entry came or went): 0 right after,
    /// 1 once they have settled. `list_dir` is the way they come from, and `list_sig` what the list was.
    list_t: f64,
    list_v: f64,
    list_dir: f32,
    list_sig: (View, usize, usize, i64),
    /// The calendar's month (or week) slides in the same way when the arrows are used.
    cal_t: f64,
    cal_v: f64,
    cal_dir: f32,
    /// A new track fades its cover and title in.
    track_t: f64,
    track_v: f64,
    /// The accent taken from the cover glides to a new colour instead of jumping.
    art_shown: [f32; 3],
    /// 0 right after the view changed, 1 once the new view has taken over.
    switch_t: f64,
    switch_v: f64,
    /// The view that is fading out while the new one comes in, and which way the new one slides from.
    prev_view: Option<View>,
    switch_dir: f32,
    shown_view: View,
    /// Hover glows, switch knobs and other small values that glide towards their goal.
    glows: Glows,
    last_draw: Instant,
    /// The open notch's background (body, glow and border), kept while nothing about it changes.
    body: Option<(ui::BodyKey, Pixmap)>,
    /// How long one frame of an animation is: one refresh of the screen the notch is on.
    frame: Duration,
    /// When the screen's refresh rate was last read. Windows can change it at any time (the user picks another
    /// one, the notch moves to another screen), so it is read again every few seconds.
    rate_read: Option<Instant>,
    /// When a frame of the animation was last due; the next ones follow at steady steps of `frame`.
    beat: Instant,
    /// Windows is using its light theme (apps), as last reported by the system.
    system_light: bool,
    /// The settings window, while it is open.
    panel: Option<Panel>,
    /// With Show on = Every: the copies of the closed notch on the other screens (`ui/mirrors.rs`).
    mirrors: Vec<Mirror>,
    /// The copies are to be made or taken away the next time the event loop is free to (windows are made from it).
    mirrors_wanted: bool,
    /// When they were last drawn, and last put in front of other always-on-top windows.
    mirror_drawn: Instant,
    mirror_raised: Instant,
    /// Open the settings window the next time the event loop is free to (a click cannot create a window).
    panel_wanted: bool,
    /// The page the settings window should show when it opens.
    panel_page: Option<panel::Page>,
    /// How the last "Check for updates" in the settings window went.
    update: update::Status,
    /// The forecast the Weather tab shows, when it was fetched, whether the last fetch is still on its way or failed,
    /// and the thread that fetches it (told which place to watch; `None` before the window exists).
    weather: Option<weather::Report>,
    weather_at: Option<Instant>,
    weather_loading: bool,
    weather_failed: bool,
    weather_tx: Option<std::sync::mpsc::Sender<Option<weather::Place>>>,
    /// The prayer times the Prayer tab shows, the same bookkeeping as the weather, the prayer a notice was last
    /// shown for (day and index), and the minute the Prayer view was last drawn.
    prayer: Option<prayer::Table>,
    prayer_loading: bool,
    prayer_failed: bool,
    prayer_tx: Option<std::sync::mpsc::Sender<Option<prayer::Request>>>,
    prayer_told: Option<(i64, usize)>,
    prayer_minute: u32,
    /// The volume of the default output device and whether it is muted, as last heard, for the Music view's slider;
    /// and whether that slider is being dragged.
    sys_volume: Option<(u32, bool)>,
    volume_drag: bool,
    /// Whether the hotkey's combination is registered.
    hotkey_ok: bool,
    /// The Stats view's numbers and when they were last looked at.
    stats: stats::Stats,
    stats_at: Option<Instant>,
    /// When the Stats view was last drawn for its gliding graphs; the next is a frame later.
    stats_frame: Option<Instant>,
    /// The parts of the Stats view that do not change from one frame to the next, drawn once and copied in (see
    /// `blit_backdrop` in `ui/stats.rs`).
    stats_backdrops: std::cell::RefCell<ui::StatsBackdrops>,
    /// With Show on All screens: the number of the screen the notch last followed the mouse to (none until it has).
    follow: Option<u32>,
    /// The version the notch has already told the user about, so the notice comes once per version.
    update_told: Option<String>,
    /// The changelog of the settings window, loaded from GitHub when it is first opened.
    changelog: update::Changelog,
    /// When the timer's alarm sounds again while it is ringing.
    next_chime: Option<Instant>,
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
            media_at: Instant::now(),
            media_handle: None,
            view: View::Player,
            view_t: 0.0,
            view_v: 0.0,
            grow_t: 0.0,
            grow_v: 0.0,
            lyrics: LyricsState::None,
            lyrics_key: String::new(),
            lyrics_duration: Duration::ZERO,
            lyrics_refetched: false,
            lyrics_retry_at: None,
            lyric_pos: -1.0,
            lyric_v: 0.0,
            lyric_manual: None,
            timer: Timer::load(),
            timer_field: None,
            timer_input: String::new(),
            water: Water::new(today(), Instant::now()),
            shelf: {
                let mut shelf = Shelf::new();
                if settings.empty_shelf {
                    shelf.clear();
                }
                shelf
            },
            icons: HashMap::new(),
            icon_loader: None,
            drag_hover: false,
            press: None,
            selected: Vec::new(),
            selecting: false,
            scrub: None,
            ui_k: 1.0,
            screen_w: 1536.0,
            closed_k: 1.0,
            closed_v: 0.0,
            last_wheel_tab: Instant::now(),
            lyric_settle: false,
            seeked: false,
            notice: None,
            notice_until: None,
            ov_t: 0.0,
            ov_v: 0.0,
            clips: {
                let mut clips = History::new();
                clips.load_pinned(clipboard::read_pinned());
                clips
            },
            clip_flash: None,
            enter_at: None,
            leave_at: None,
            tray: None,
            tray_settings_id: None,
            tray_quit_id: None,
            tray_about_id: None,
            todo: Todo::new(),
            cal: calendar::Calendar::new(today_day()),
            editing: false,
            input: String::new(),
            edit_started: Instant::now(),
            editor: editor::Editor::new_task(None, clock_now()),
            prev_foreground: None,
            settings: settings.clone(),
            autostart: settings::repair_autostart(),
            presence: Presence::Active,
            fullscreen: presence::Fullscreen::No,
            monitor_area: None,
            hidden: false,
            greeting_until: None,
            eye_preview_until: None,
            next_blink: Instant::now(),
            blink_since: None,
            look: (0.0, 0.0),
            look_v: (0.0, 0.0),
            look_goal: (0.0, 0.0),
            next_glance: Instant::now(),
            glances: 0,
            blink_count: 0,
            next_zz: Instant::now(),
            zz_frame: false,
            last_minute: u32::MAX,
            quit: false,
            alert_until: None,
            alert_msg: String::new(),
            last_ui_tick: Instant::now(),
            music_k: 0.0,
            music_v: 0.0,
            music_w: 0.0,
            music_wv: 0.0,
            bars_at: Instant::now(),
            proxy,
            cursor: (0.0, 0.0),
            hovered: false,
            progress: 0.0,
            progress_v: 0.0,
            tuck_t: if settings.hide_island { 1.0 } else { 0.0 },
            tuck_v: 0.0,
            tuck_goal: if settings.hide_island { 1.0 } else { 0.0 },
            art_tint: [255, 110, 140],
            art_key: String::new(),
            tab_pos: 0.0,
            tab_vel: 0.0,
            list_t: 1.0,
            list_v: 0.0,
            list_dir: 1.0,
            list_sig: (View::Player, 0, 0, 0),
            cal_t: 1.0,
            cal_v: 0.0,
            cal_dir: 1.0,
            track_t: 1.0,
            track_v: 0.0,
            art_shown: [255.0, 110.0, 140.0],
            switch_t: 1.0,
            switch_v: 0.0,
            prev_view: None,
            switch_dir: 1.0,
            glows: Glows::default(),
            last_draw: Instant::now(),
            body: None,
            frame: Duration::from_micros(16_667),
            rate_read: None,
            beat: Instant::now(),
            system_light: false,
            panel: None,
            mirrors: Vec::new(),
            mirrors_wanted: false,
            mirror_drawn: Instant::now(),
            mirror_raised: Instant::now(),
            panel_wanted: false,
            panel_page: None,
            update: update::Status::Idle,
            update_told: None,
            follow: None,
            hotkey_ok: true,
            stats: stats::Stats::default(),
            stats_at: None,
            stats_frame: None,
            stats_backdrops: Default::default(),
            sys_volume: None,
            volume_drag: false,
            weather: None,
            weather_at: None,
            weather_loading: false,
            weather_failed: false,
            weather_tx: None,
            prayer: None,
            prayer_loading: false,
            prayer_failed: false,
            prayer_tx: None,
            prayer_told: None,
            prayer_minute: 0,
            changelog: update::Changelog::Idle,
            next_chime: None,
            shown_view: View::Player,
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
    let hash = data_dir
        .to_lowercase()
        .bytes()
        .fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
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
