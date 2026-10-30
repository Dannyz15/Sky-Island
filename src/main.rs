// A GUI program: without this Windows opens a console window next to it.
#![windows_subsystem = "windows"]
#![allow(clippy::too_many_arguments)]

mod calendar;
mod editor;
mod gfx;
mod lyrics;
mod media;
mod shelf;
mod store;
mod sound;
mod presence;
mod settings;
mod sys;
mod todo;
mod water;
mod appicon;
mod clipboard;
mod dragdrop;
mod theme;
mod timer;
mod icon;
mod motion;
mod panel;
mod ui;
mod update;

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
    Lyrics { key: String, fetched: lyrics::Fetched },
    Notice(sys::Notice),
    /// A lock key was pressed; its state is read on the UI thread.
    LockKey { name: &'static str, vk: i32 },
    Presence { presence: Presence, fullscreen: presence::Fullscreen },
    Menu(MenuId),
    /// Text or a picture was copied to the clipboard.
    Clip(clipboard::Content),
    /// The answer to "Check for updates".
    Update(update::Status),
    /// The releases and their notes, for the changelog in the settings window.
    Changelog(Option<Vec<update::Release>>),
}

#[derive(PartialEq, Clone, Copy)]
enum View {
    Player,
    Lyrics,
    Timer,
    Water,
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
    next_blink: Instant,
    blink_until: Option<Instant>,
    blink_count: u32,
    next_zz: Instant,
    zz_frame: bool,
    last_minute: u32,
    quit: bool,
    /// While set, the notch stays open to show a timer notification.
    alert_until: Option<Instant>,
    alert_msg: String,
    last_ui_tick: Instant,
    proxy: EventLoopProxy<UserEvent>,
    cursor: (f64, f64),
    hovered: bool,
    /// 0.0 = collapsed, 1.0 = expanded. A spring, so it can swing a little past 1.0 when it opens.
    progress: f64,
    progress_v: f64,
    /// 0.0 = in place, 1.0 = slid up out of view (the "Hide island" setting).
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
    /// Windows is using its light theme (apps), as last reported by the system.
    system_light: bool,
    /// The settings window, while it is open.
    panel: Option<Panel>,
    /// Open the settings window the next time the event loop is free to (a click cannot create a window).
    panel_wanted: bool,
    /// Whether the settings window should show the About page when it opens.
    panel_about: bool,
    /// How the last "Check for updates" in the settings window went.
    update: update::Status,
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
            notice: None,
            notice_until: None,
            ov_t: 0.0,
            ov_v: 0.0,
            clips: History::new(),
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
            autostart: settings::autostart_enabled(),
            presence: Presence::Active,
            fullscreen: presence::Fullscreen::No,
            monitor_area: None,
            hidden: false,
            greeting_until: None,
            next_blink: Instant::now(),
            blink_until: None,
            blink_count: 0,
            next_zz: Instant::now(),
            zz_frame: false,
            last_minute: u32::MAX,
            quit: false,
            alert_until: None,
            alert_msg: String::new(),
            last_ui_tick: Instant::now(),
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
            system_light: false,
            panel: None,
            panel_wanted: false,
            panel_about: false,
            update: update::Status::Idle,
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

/// With no console, a panic would vanish silently. Write it to a file the user can find and send.
fn install_crash_log() {
    std::panic::set_hook(Box::new(|info| {
        let Some(appdata) = std::env::var_os("APPDATA") else { return };
        let dir = PathBuf::from(appdata).join("Sky-Island");
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("crash.log"), format!("{info}\n"));
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
    fn each_data_folder_has_its_own_instance_lock() {
        let a = instance_name(r"C:\Users\A\AppData\Roaming");
        assert_eq!(a, instance_name(r"c:\users\a\appdata\roaming"), "case does not matter on Windows");
        assert_ne!(a, instance_name(r"C:\Users\B\AppData\Roaming"));
        assert!(a.starts_with("Local\\SkyIsland-"));
    }

}
