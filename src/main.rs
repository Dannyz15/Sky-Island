// A GUI program: without this Windows opens a console window next to it.
#![windows_subsystem = "windows"]
#![allow(clippy::too_many_arguments)]

mod calendar;
mod editor;
mod gfx;
mod lyrics;
mod media;
mod shelf;
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
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use water::Water;
use motion::{Glows, spring};

/// The closed notch, about the size of a MacBook's.
const COLLAPSED: (f64, f64) = (148.0, 28.0);
/// Width of the curved flare on each side of the notch's top edge, where it meets the top of the screen.
/// The window is this much wider than the notch on both sides; the notch is drawn in the middle.
const EAR: f64 = 12.0;
const EXPANDED: (f64, f64) = (420.0, 168.0);
/// Keys for the animated values that are not tied to a rectangle (see `motion::Glows`).
const KEY_PLAYER_PLAY: u64 = 0x4000_0000_0000_0001;
const KEY_TIMER_PLAY: u64 = 0x4000_0000_0000_0002;
const KEY_BAR: u64 = 0x4000_0000_0000_0003;
const KEY_WATER: u64 = 0x4000_0000_0000_0004;
const KEY_LYRIC_SYNC: u64 = 0x4000_0000_0000_0005;
const FRAME: Duration = Duration::from_millis(8);
const PROGRESS_TICK: Duration = Duration::from_millis(500);
/// How often the timer's ring is redrawn while it is on screen and running, so it empties smoothly.
const RING_TICK: Duration = Duration::from_millis(16);
/// How much of a hidden island stays on the top edge of the screen, in logical pixels.
const TUCK_STRIP: f64 = 3.0;

// Expanded layout, in logical pixels.
const CTRL_Y: f32 = 142.0;
const CTRL_R: f32 = 16.0;

pub enum UserEvent {
    Media(MediaState),
    Lyrics { key: String, lines: Option<Vec<Line>> },
    Notice(sys::Notice),
    /// A lock key was pressed; its state is read on the UI thread.
    LockKey { name: &'static str, vk: i32 },
    Presence { presence: Presence, fullscreen: bool },
    Menu(MenuId),
    /// Text or a picture was copied to the clipboard.
    Clip(clipboard::Content),
    /// The answer to "Check for updates".
    Update(update::Status),
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

/// Tab bar entries, left to right.
const TABS: [(&str, View); 7] = [
    ("Music", View::Player),
    ("Timer", View::Timer),
    ("Water", View::Water),
    ("Shelf", View::Shelf),
    ("To-do", View::Todo),
    ("Calendar", View::Calendar),
    ("Clipboard", View::Clipboard),
];
const TAB_W: f32 = 44.0;
const TAB_GAP: f32 = 4.0;
// Ring and controls shared by the timer and water views, in logical pixels.
const RING: (f32, f32, f32) = (104.0, 100.0, 48.0); // center x, center y, radius
const RING_W: f32 = 7.0;
const WATER_CTRL_Y: f32 = 136.0;
const WATER_X: [f32; 3] = [232.0, 293.0, 354.0]; // left, center, right controls (used by the water view)
// Timer view: preset chips on top, plus / minus buttons above and below the hours, minutes and
// seconds, then start / pause and reset side by side.
const TIMER_COLS: [f32; 3] = [237.0, 293.0, 349.0];
const TIMER_PLUS_Y: f32 = 64.0;
const TIMER_VALUE_Y: f32 = 91.0; // baseline
const TIMER_MINUS_Y: f32 = 108.0;
const TIMER_BTN_R: f32 = 10.0;
const TIMER_CTRL_Y: f32 = 140.0;
const TIMER_START_X: f32 = 263.0;
/// The red Stop button shown while the timer's alarm rings.
const STOP_BUTTON: (f32, f32, f32, f32) = (TIMER_COLS[1] - 62.0, TIMER_CTRL_Y - 16.0, 124.0, 32.0);
const TIMER_RESET_X: f32 = 323.0;
const TIMER_CHIP_W: f32 = 34.0;
const ADJUST: [(f32, f32); 2] = [(231.0, 70.0), (355.0, 70.0)]; // minus, plus button centers
const ADJUST_R: f32 = 11.0;
const CHIP_GAP: f32 = 4.0;
const WATER_CHIP_W: f32 = 46.0;
// Shelf tiles, in logical pixels.
const TILE: (f32, f32) = (68.0, 58.0);
const TILE_GAP: (f32, f32) = (8.0, 6.0);
const TILES_ORIGIN: (f32, f32) = (24.0, 36.0);
/// Cursor travel, in physical pixels, before a press on a tile becomes a drag.
const DRAG_THRESHOLD: f64 = 6.0;
/// Compact size of the notch while a system notice (volume, battery...) is showing.
/// The notch while it shows a notice (volume, battery...); the notice is laid out for 300 wide and scaled to fit.
const OVERLAY: (f64, f64) = (240.0, 42.0);
const NOTICE_TIME: Duration = Duration::from_millis(2200);
const GREETING: Duration = Duration::from_millis(1600);
const BLINK: Duration = Duration::from_millis(160);
// To-do view, in logical pixels.
const TODO_FIELD: (f32, f32, f32, f32) = (24.0, 34.0, 372.0, 24.0);
const TODO_ROWS_Y: f32 = 64.0;
const TODO_ROW_H: f32 = 24.0;
const TODO_AMBER: [u8; 3] = [255, 190, 70];
/// How long the cursor must rest on the closed notch before it opens, so fly-bys don't trigger it.
/// How long the cursor can be off the open notch before it closes, so small slips don't close it.
/// Seconds the lyrics take to glide to a newly sung line.
/// The Sync button that appears over the lyrics after they were scrolled away from the music.
const SYNC_RECT: (f32, f32, f32, f32) = (LYRIC_X + LYRIC_W - 62.0, 148.0, 62.0, 20.0);
// Clipboard view, in logical pixels.
/// The shelf's Clear button, in the free corner of the tab row.
const SHELF_CLEAR: (f32, f32, f32, f32) = (378.0, 8.0, 36.0, 16.0);
const CLIP_CLEAR: (f32, f32, f32, f32) = (324.0, 34.0, 72.0, 22.0);
const CLIP_ROWS_Y: f32 = 64.0;
const CLIP_ROW_H: f32 = 32.0;
const CLIP_TEAL: [u8; 3] = [70, 200, 210];
/// How long the "Copied" / "Cleared" message stays in the clipboard header.
const FLASH: Duration = Duration::from_millis(1400);
const ALERT: Duration = Duration::from_secs(6);
/// The banner text when the timer ends; the timer view recognises it by this.
const TIMES_UP: &str = "Time's up!";

enum LyricsState {
    None,
    Loading,
    NotFound,
    Found(Vec<Line>),
}

// Lyrics layout, in logical pixels. The notch grows wider and the lyrics sit to the right of the player.
const LYRICS_W: f64 = 660.0; // notch width while the lyrics are open
const LYRICS_H: f64 = 176.0; // notch height while the lyrics are open
/// After the lyrics open or close, how long the notch waits for the mouse to come back if it now sits outside.
const LYRIC_GRACE: Duration = Duration::from_millis(1500);
const LYRIC_X: f32 = 292.0; // left edge of the lyrics
const LYRIC_AREA: (f32, f32) = (34.0, 172.0); // top, bottom of the scrolling lyrics area
const LYRIC_W: f32 = 344.0; // wrap width
const LYRIC_GAP: f32 = 12.0;

struct LyricItem {
    idx: usize,
    top: f32,
    lh: f32,
    px: f32,
    /// How many lines this one is from the middle of the view (0 when it is the middle line).
    near: f32,
    /// This is the line being sung right now.
    current: bool,
    rows: Vec<String>,
}

/// Text size of a lyric line by how far it is from the middle of the view.
fn lyric_px(near: f32) -> f32 {
    if near < 1.0 {
        18.0 - 4.0 * near
    } else if near < 2.0 {
        14.0 - (near - 1.0)
    } else {
        13.0
    }
}

/// Grey level and strength of a lyric line by how far it is from the middle of the view.
fn lyric_tone(near: f32) -> (u8, f32) {
    const KNOTS: [(f32, f32, f32); 4] = [(0.0, 255.0, 1.0), (1.0, 175.0, 0.85), (2.0, 135.0, 0.6), (3.0, 110.0, 0.45)];
    let near = near.clamp(0.0, 3.0);
    let i = (near.floor() as usize).min(2);
    let t = near - KNOTS[i].0;
    let (a, b) = (KNOTS[i], KNOTS[i + 1]);
    ((a.1 + (b.1 - a.1) * t) as u8, a.2 + (b.2 - a.2) * t)
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
    tray_toggle: Option<MenuItem>,
    tray_toggle_id: Option<MenuId>,
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
    fullscreen: bool,
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
    /// How the last "Check for updates" in the settings window went.
    update: update::Status,
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
            lyric_pos: -1.0,
            lyric_v: 0.0,
            lyric_manual: None,
            timer: Timer::load(),
            timer_field: None,
            timer_input: String::new(),
            water: Water::new(today(), Instant::now()),
            shelf: Shelf::new(),
            icons: HashMap::new(),
            drag_hover: false,
            press: None,
            selected: Vec::new(),
            selecting: false,
            scrub: None,
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
            tray_toggle: None,
            tray_toggle_id: None,
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
            fullscreen: false,
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
            switch_t: 1.0,
            switch_v: 0.0,
            prev_view: None,
            switch_dir: 1.0,
            glows: Glows::default(),
            last_draw: Instant::now(),
            system_light: false,
            panel: None,
            panel_wanted: false,
            update: update::Status::Idle,
            next_chime: None,
            shown_view: View::Player,
            last_tick: Instant::now(),
        }
    }

    fn target(&self) -> f64 {
        if self.hovered || self.drag_hover || self.editing || self.timer_field.is_some() || self.alert_until.is_some() { 1.0 } else { 0.0 }
    }

    /// With "Hide island" on, the island slides away whenever there is nothing to show or touch.
    fn tuck_target(&self) -> f64 {
        let needed = self.hovered
            || self.enter_at.is_some()
            || self.drag_hover
            || self.editing
            || self.timer_field.is_some()
            || self.alert_until.is_some()
            || self.notice_until.is_some()
            || self.progress > 0.0;
        if self.settings.hide_island && !needed { 1.0 } else { 0.0 }
    }

    /// The tab index the highlight is heading for, if the current view has a tab.
    fn tab_target(&self) -> Option<f32> {
        // The lyrics are part of the Music tab.
        let view = if self.view == View::Lyrics { View::Player } else { self.view };
        self.visible_tabs().iter().position(|(_, v)| *v == view).map(|i| i as f32)
    }

    /// How far the tab bar (and the views other than Music) shift right to stay centred while the
    /// notch is wider than usual because the lyrics are open.
    fn tab_dx(&self) -> f32 {
        ((LYRICS_W - EXPANDED.0) * self.view_t.clamp(0.0, 1.0) / 2.0) as f32
    }

    /// Where the parts of the player sit right now.
    fn geo(&self) -> PlayerGeo {
        player_geo(self.view_t.clamp(0.0, 1.0) as f32)
    }

    /// The tabs in the bar: all of them, except Water and Timer while they are switched off.
    fn visible_tabs(&self) -> Vec<(&'static str, View)> {
        TABS.iter()
            .copied()
            .filter(|(_, v)| match v {
                View::Water => self.settings.water,
                View::Timer => self.settings.timer,
                _ => true,
            })
            .collect()
    }

    /// The colour the current view is tinted with.
    fn accent(&self, view: View) -> [u8; 3] {
        match view {
            View::Player | View::Lyrics => self.art_tint,
            View::Timer => [255, 99, 71],
            View::Water => [64, 170, 255],
            View::Shelf => [125, 140, 255],
            View::Todo => TODO_AMBER,
            View::Calendar => [200, 120, 255],
            View::Clipboard => CLIP_TEAL,
        }
    }

    /// The title font.
    fn heading_font(&self) -> Option<&Font> {
        self.bold.as_ref().or(self.font.as_ref())
    }

    /// Whether the mouse is over a rectangle (logical pixels) of the open notch.
    fn hot(&self, r: (f32, f32, f32, f32), s: f32) -> bool {
        if self.progress < 0.9 || !self.hovered {
            return false;
        }
        let (x, y) = (self.cursor.0 as f32 / s, self.cursor.1 as f32 / s);
        x >= r.0 && x <= r.0 + r.2 && y >= r.1 && y <= r.1 + r.3
    }

    /// How strongly the mouse is over a rectangle, 0 to 1, fading in and out instead of switching.
    fn hot_t(&self, r: (f32, f32, f32, f32), s: f32) -> f32 {
        self.glows.fade(motion::rect_key(r), self.hot(r, s))
    }

    /// A toggle's position, 0 (off) to 1 (on), springing across and settling with a small bounce.
    fn toggle_t(&self, id: u64, on: bool) -> f32 {
        self.glows.follow(0x8000_0000_0000_0000 | id, if on { 1.0 } else { 0.0 }, 34.0, 0.62) as f32
    }

    fn animating(&self) -> bool {
        (self.progress - self.target()).abs() > f64::EPSILON
            || self.switch_t < 1.0
            || self.tab_target().is_some_and(|t| (t - self.tab_pos).abs() > f32::EPSILON)
            || (self.tuck_t - self.tuck_target()).abs() > f64::EPSILON
            || (self.view_t - self.view_target()).abs() > f64::EPSILON
            || (self.ov_t - self.ov_target()).abs() > f64::EPSILON
            || self.glows.busy()
            || (self.view == View::Lyrics
                && matches!(self.lyrics, LyricsState::Found(_))
                && (self.lyric_pos - self.lyric_target()).abs() > f64::EPSILON)
    }

    fn ov_target(&self) -> f64 {
        if self.notice_until.is_some() { 1.0 } else { 0.0 }
    }

    fn view_target(&self) -> f64 {
        if self.view == View::Lyrics { 1.0 } else { 0.0 }
    }

    /// Playback position, extrapolated since the last update from the media thread.
    fn position(&self) -> Duration {
        if let Some(at) = self.scrub {
            return at;
        }
        let mut p = self.media.position;
        if self.media.playing {
            p += self.media_at.elapsed();
        }
        if !self.media.duration.is_zero() {
            p = p.min(self.media.duration);
        }
        p
    }

    /// Advance animation, then resize/reposition the window to match.
    fn step(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f64().min(1.0 / 20.0);
        self.last_tick = now;

        // The Animation setting makes the springs slower or quicker, and can take the bounce out.
        let (speed, bounce) = (self.settings.motion.speed(), self.settings.motion.bounce());
        let tune = |omega: f64, zeta: f64| (omega * speed, if bounce { zeta } else { 1.0 });

        // Everything is pulled towards its goal by a spring. Opening swings out a little past the goal and
        // settles; closing and hiding glide in without bouncing.
        let target = self.target();
        let (omega, zeta) = if target > 0.5 { tune(24.0, 0.74) } else { tune(28.0, 1.0) };
        spring(&mut self.progress, &mut self.progress_v, target, dt, omega, zeta);

        let tuck_goal = self.tuck_target();
        spring(&mut self.tuck_t, &mut self.tuck_v, tuck_goal, dt, 22.0 * speed, 1.0);

        spring(&mut self.switch_t, &mut self.switch_v, 1.0, dt, 24.0 * speed, 1.0);
        if self.switch_t >= 1.0 {
            self.prev_view = None;
        }

        if let Some(target) = self.tab_target() {
            let (mut x, mut v) = (self.tab_pos as f64, self.tab_vel as f64);
            let (omega, zeta) = tune(30.0, 0.72);
            spring(&mut x, &mut v, target as f64, dt, omega, zeta);
            self.tab_pos = x as f32;
            self.tab_vel = v as f32;
        }

        let ot = self.ov_target();
        let (omega, zeta) = if ot > 0.5 { tune(26.0, 0.8) } else { tune(28.0, 1.0) };
        spring(&mut self.ov_t, &mut self.ov_v, ot, dt, omega, zeta);

        let vt = self.view_target();
        let (omega, zeta) = if vt > 0.5 { tune(24.0, 0.85) } else { tune(28.0, 1.0) };
        spring(&mut self.view_t, &mut self.view_v, vt, dt, omega, zeta);

        // The lyrics glide to the line being sung, or to wherever they were scrolled.
        if self.view == View::Lyrics && matches!(self.lyrics, LyricsState::Found(_)) {
            let goal = self.lyric_target();
            spring(&mut self.lyric_pos, &mut self.lyric_v, goal, dt, 14.0 * speed, 1.0);
        }

        let Some(window) = self.window.clone() else { return };
        // The size may swing a little past its goal while the notch opens.
        let t = self.progress.max(0.0);
        let wide = self.view_t.clamp(0.0, 1.0);
        let full_h = lerp(EXPANDED.1, LYRICS_H, wide);
        // The closed notch grows to the compact notice size while a notice is showing.
        let ov = self.ov_t.clamp(0.0, 1.0);
        let base = (lerp(COLLAPSED.0, OVERLAY.0, ov), lerp(COLLAPSED.1, OVERLAY.1, ov));
        // Opening the lyrics widens the notch evenly on both sides, so it stays centred on the screen.
        let w = lerp(base.0, lerp(EXPANDED.0, LYRICS_W, wide), t);
        let h = lerp(base.1, full_h, t);
        let _ = window.request_inner_size(LogicalSize::new(w + 2.0 * EAR, h));

        if let Some(monitor) = window.current_monitor().or_else(|| window.primary_monitor()) {
            let scale = window.scale_factor();
            let mpos = monitor.position();
            let msize = monitor.size();
            let pw = (w + 2.0 * EAR) * scale;
            let x = mpos.x as f64 + (msize.width as f64 - pw) / 2.0;
            // Tucked away, only a thin strip stays on screen, so the cursor can still find the island.
            let y = mpos.y as f64 - self.tuck_t.clamp(0.0, 1.0) * (h - TUCK_STRIP) * scale;
            window.set_outer_position(PhysicalPosition::new(x, y));
        }
        window.request_redraw();
    }

    /// The line the lyrics view is heading for: where the wheel took it, otherwise the line being sung
    /// (-1 before the first one starts).
    fn lyric_target(&self) -> f64 {
        self.lyric_manual.unwrap_or_else(|| self.current_line().map_or(-1.0, |c| c as f64))
    }

    /// Put the middle of the view on its target at once, without gliding there.
    fn snap_lyrics(&mut self) {
        self.lyric_pos = self.lyric_target();
        self.lyric_v = 0.0;
    }

    /// Whether the lyrics were scrolled to somewhere other than the line being sung, so Sync is offered.
    fn lyric_off_sync(&self) -> bool {
        let sung = self.current_line().map_or(-1.0, |c| c as f64);
        self.lyric_manual.is_some_and(|m| (m - sung).abs() >= 1.0)
    }

    /// Index of the lyric line being sung at the current position.
    fn current_line(&self) -> Option<usize> {
        let LyricsState::Found(lines) = &self.lyrics else { return None };
        let pos = self.position();
        lines.iter().rposition(|l| l.at <= pos)
    }

    /// Take the keyboard while files are highlighted (the notch never takes focus on its own), so that
    /// Ctrl+A and Esc reach it, and give it back to the app the user was in once nothing is highlighted.
    fn sync_selection_focus(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SetForegroundWindow};
        if !self.selected.is_empty() && !self.selecting && !self.editing && self.timer_field.is_none() {
            let fg = unsafe { GetForegroundWindow() };
            self.prev_foreground = (!fg.is_invalid()).then_some(fg);
            self.selecting = true;
            if let Some(w) = &self.window {
                w.focus_window();
            }
        } else if self.selected.is_empty() && self.selecting {
            self.selecting = false;
            if let Some(h) = self.prev_foreground.take() {
                let _ = unsafe { SetForegroundWindow(h) };
            }
        }
    }

    /// Start dragging a shelf file out to another app. Blocks until the drag ends.
    fn drag_out(&mut self, idx: usize) {
        let Some(path) = self.shelf.items.get(idx).cloned() else { return };
        // Dragging a highlighted file takes every highlighted file; any other file goes alone.
        let paths = if self.selected.contains(&path) {
            self.shelf.items.iter().filter(|p| self.selected.contains(p)).cloned().collect()
        } else {
            self.selected.clear();
            self.sync_selection_focus();
            vec![path]
        };
        let dropped = dragdrop::drag_files(&paths).unwrap_or(false);
        // No cursor events arrive during the drag, so work out whether it ended over the notch.
        let back_on_notch = self.cursor_over_window();
        // Files that were dropped somewhere have left the shelf, whether the drop moved or copied them
        // (a copy to another drive or into a chat app leaves the original where it was). Letting go over
        // nothing, cancelling, or dropping them back on the notch keeps them.
        if dropped && !back_on_notch {
            for path in &paths {
                if let Some(i) = self.shelf.items.iter().position(|p| p == path) {
                    self.shelf.remove(i);
                }
            }
            self.selected.clear();
            self.sync_selection_focus();
        }
        // The file may also have been moved by the drop.
        self.shelf.prune();
        self.hovered = back_on_notch;
        self.enter_at = None;
        self.leave_at = None;
        self.last_tick = Instant::now();
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Fetch icons for any shelf files that don't have one cached yet, and forget removed files.
    fn load_icons(&mut self) {
        self.icons.retain(|p, _| self.shelf.items.contains(p));
        for path in &self.shelf.items {
            if !self.icons.contains_key(path) {
                self.icons.insert(path.clone(), icon::load(path));
            }
        }
    }

    fn show_notice(&mut self, notice: sys::Notice) {
        let now = Instant::now();
        self.notice = Some(notice);
        self.notice_until = Some(now + NOTICE_TIME);
        self.last_tick = now;
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn cursor_over_window(&self) -> bool {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let Some(w) = &self.window else { return false };
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt) }.is_err() {
            return false;
        }
        let (pos, size) = (w.outer_position().unwrap_or_default(), w.outer_size());
        pt.x >= pos.x && pt.x < pos.x + size.width as i32 && pt.y >= pos.y && pt.y < pos.y + size.height as i32
    }

    /// The song position under a cursor x (logical pixels), clamped to the ends of the bar.
    fn scrub_at(&self, cx: f32) -> Duration {
        let (bx, _, bw) = self.geo().bar;
        let frac = ((cx - bx) / bw).clamp(0.0, 1.0);
        self.media.duration.mul_f32(frac)
    }

    fn click(&mut self) {
        if self.progress < 0.9 {
            return;
        }
        let Some(window) = self.window.clone() else { return };
        let s = window.scale_factor() as f32;
        let (cx, cy) = (self.cursor.0 as f32 / s, self.cursor.1 as f32 / s);

        // While the timer's alarm rings, only its Stop button stops it.
        if self.alert_until.is_some() && self.alert_msg == TIMES_UP && self.view == View::Timer {
            if cx >= STOP_BUTTON.0 && cx <= STOP_BUTTON.0 + STOP_BUTTON.2 && cy >= STOP_BUTTON.1 && cy <= STOP_BUTTON.1 + STOP_BUTTON.3 {
                self.alert_until = None;
                self.next_chime = None;
                sound::stop_file();
                window.request_redraw();
                return;
            }
            // The hidden controls under the Stop button stay dead; only the tab bar still works.
            if cy > 30.0 {
                return;
            }
        }

        // A click anywhere finishes the number being typed; it is then handled as usual.
        if self.timer_field.is_some() {
            self.end_timer_edit(true);
        }

        {
            let dx = self.tab_dx();
            let tabs = self.visible_tabs();
            for (i, (_, v)) in tabs.iter().enumerate() {
                let r = tab_rect(i, tabs.len());
                let r = (r.0 + dx, r.1, r.2, r.3);
                if cx >= r.0 && cx <= r.0 + r.2 && cy >= r.1 && cy <= r.1 + r.3 {
                    if self.view != *v {
                        self.stop_editing();
                        self.view = *v;
                        if *v == View::Calendar {
                            self.cal = calendar::Calendar::new(today_day());
                        }
                        self.last_tick = Instant::now();
                        window.request_redraw();
                    }
                    return;
                }
            }
        }

        // The editor panel covers the whole notch while a task is being added or changed.
        if self.editing {
            if let Some(action) = editor::hit(cx, cy, self.editor.remind, self.settings.clock_24h) {
                match self.editor.apply(action, clock_now()) {
                    editor::Outcome::Save => self.commit_task(),
                    editor::Outcome::Cancel => self.stop_editing(),
                    editor::Outcome::Continue => {}
                }
            }
            window.request_redraw();
            return;
        }

        match self.view {
            View::Todo => {
                let inside = |r: (f32, f32, f32, f32)| cx >= r.0 && cx <= r.0 + r.2 && cy >= r.1 && cy <= r.1 + r.3;
                if inside(TODO_FIELD) {
                    window.request_redraw();
                    self.start_editing(None);
                    return;
                }
                let row = ((cy - TODO_ROWS_Y) / TODO_ROW_H).floor();
                let Some(idx) = (row >= 0.0).then(|| self.todo.index_at(row as usize)).flatten() else { return };
                if cx <= 46.0 {
                    self.todo.toggle(idx);
                } else if cx >= 372.0 {
                    self.todo.remove(idx);
                } else {
                    // Anywhere else on a task opens it for editing.
                    self.start_editing(Some(idx));
                }
                window.request_redraw();
            }
            View::Calendar => {
                let inside = |r: (f32, f32, f32, f32)| cx >= r.0 && cx <= r.0 + r.2 && cy >= r.1 && cy <= r.1 + r.3;
                if inside(CAL_PREV) {
                    self.cal.shift_month(-1);
                } else if inside(CAL_NEXT) {
                    self.cal.shift_month(1);
                } else if inside(CAL_TITLE) {
                    self.cal = calendar::Calendar::new(today_day());
                } else if inside(CAL_ADD) {
                    window.request_redraw();
                    self.start_editing(None);
                    return;
                } else if let Some(day) = (0..calendar::CELLS).find(|&i| inside(cal_cell_rect(i))).and_then(|i| self.cal.cell(i)) {
                    self.cal.selected = day;
                } else {
                    let row = ((cy - CAL_ROWS_Y) / CAL_ROW_H).floor();
                    if !(CAL_PANEL_X - 4.0..=CAL_PANEL_X + CAL_PANEL_W + 4.0).contains(&cx) || row < 0.0 || row >= CAL_VISIBLE as f32 {
                        return;
                    }
                    let Some(&idx) = self.todo.on_day(self.cal.selected).get(row as usize) else { return };
                    if cx <= CAL_PANEL_X + 18.0 {
                        self.todo.toggle(idx);
                    } else if cx >= CAL_PANEL_X + CAL_PANEL_W - 22.0 {
                        self.todo.remove(idx);
                    } else {
                        self.start_editing(Some(idx));
                    }
                }
                window.request_redraw();
            }
            View::Clipboard => {
                let inside = |r: (f32, f32, f32, f32)| cx >= r.0 && cx <= r.0 + r.2 && cy >= r.1 && cy <= r.1 + r.3;
                if inside(CLIP_CLEAR) {
                    // Clears the history here and the Windows clipboard itself.
                    self.clips.clear();
                    clipboard::empty();
                    self.clip_flash = Some(("Cleared", Instant::now() + FLASH));
                } else {
                    let row = ((cy - CLIP_ROWS_Y) / CLIP_ROW_H).floor();
                    let Some(idx) = (row >= 0.0).then(|| self.clips.index_at(row as usize)).flatten() else { return };
                    if cx >= 372.0 {
                        self.clips.remove(idx);
                    } else if let Some(content) = self.clips.items.get(idx).cloned() {
                        // The watcher will see this change and move the entry to the top.
                        if clipboard::set_content(&content) {
                            self.clip_flash = Some(("Copied", Instant::now() + FLASH));
                        }
                    } else {
                        return;
                    }
                }
                window.request_redraw();
            }
            View::Shelf => {
                let hit = (0..COLS * ROWS).find_map(|slot| {
                    let r = tile_rect(slot);
                    let inside = cx >= r.0 && cx <= r.0 + r.2 && cy >= r.1 && cy <= r.1 + r.3;
                    inside.then(|| self.shelf.index_at(slot).map(|i| (i, r))).flatten()
                });
                let clear = SHELF_CLEAR;
                if !self.shelf.items.is_empty() && cx >= clear.0 && cx <= clear.0 + clear.2 && cy >= clear.1 && cy <= clear.1 + clear.3 {
                    // Takes every file off the shelf; the files themselves are never touched.
                    self.shelf.clear();
                    self.selected.clear();
                    self.sync_selection_focus();
                    window.request_redraw();
                    return;
                }
                let Some((idx, r)) = hit else { return };
                if cx >= r.0 + r.2 - 16.0 && cy <= r.1 + 16.0 {
                    self.shelf.remove(idx);
                    window.request_redraw();
                } else if ctrl_down() {
                    // Ctrl+click highlights or un-highlights one file without opening it.
                    let path = self.shelf.items[idx].clone();
                    match self.selected.iter().position(|p| *p == path) {
                        Some(i) => {
                            self.selected.remove(i);
                        }
                        None => self.selected.push(path),
                    }
                    self.sync_selection_focus();
                    window.request_redraw();
                } else {
                    self.press = Some((idx, self.cursor));
                }
            }
            View::Water => {
                let now = Instant::now();
                let in_label = |x: f32| (cx - x).abs() <= 26.0 && (cy - WATER_CTRL_Y).abs() <= 14.0;
                let chip = (0..water::INTERVALS.len()).find(|&i| {
                    let r = water_chip_rect(i);
                    cx >= r.0 && cx <= r.0 + r.2 && cy >= r.1 && cy <= r.1 + r.3
                });
                let adjust = ADJUST.iter().position(|(x, y)| (cx - x).hypot(cy - y) <= ADJUST_R);
                if let Some(i) = chip {
                    self.water.set_interval(i, now);
                } else if let Some(i) = adjust {
                    // Minus is index 0, plus is index 1.
                    self.water.adjust_goal(if i == 0 { -1 } else { 1 });
                } else if (cx - WATER_X[1]).hypot(cy - WATER_CTRL_Y) <= 18.0 {
                    self.water.add(now);
                } else if in_label(WATER_X[0]) {
                    self.water.undo();
                } else if in_label(WATER_X[2]) {
                    self.water.reset();
                } else {
                    return;
                }
                self.alert_until = None;
                window.request_redraw();
            }
            View::Timer => {
                let now = Instant::now();
                let in_label = |x: f32| (cx - x).abs() <= 26.0 && (cy - TIMER_CTRL_Y).abs() <= 14.0;
                let chip = (0..timer::PRESETS.len()).find(|&i| {
                    let r = timer_chip_rect(i);
                    cx >= r.0 && cx <= r.0 + r.2 && cy >= r.1 && cy <= r.1 + r.3
                });
                // Plus is above a number, minus below it.
                let step = Unit::ALL.iter().enumerate().find_map(|(i, unit)| {
                    let x = TIMER_COLS[i];
                    if (cx - x).hypot(cy - TIMER_PLUS_Y) <= TIMER_BTN_R + 2.0 {
                        Some((*unit, 1))
                    } else if (cx - x).hypot(cy - TIMER_MINUS_Y) <= TIMER_BTN_R + 2.0 {
                        Some((*unit, -1))
                    } else {
                        None
                    }
                });
                let value = (0..Unit::ALL.len()).find(|&i| {
                    let r = timer_value_rect(i);
                    cx >= r.0 && cx <= r.0 + r.2 && cy >= r.1 && cy <= r.1 + r.3
                });
                if let Some(i) = chip {
                    if !self.timer.set(timer::PRESETS[i]) {
                        return;
                    }
                } else if let Some(i) = value.filter(|_| self.timer.is_idle() && step.is_none()) {
                    // Click a number to type it.
                    self.begin_timer_edit(i);
                } else if let Some((unit, dir)) = step {
                    if !self.timer.adjust(unit, dir) {
                        return;
                    }
                } else if (cx - TIMER_START_X).hypot(cy - TIMER_CTRL_Y) <= 18.0 {
                    self.timer.toggle(now);
                } else if in_label(TIMER_RESET_X) {
                    self.timer.reset();
                } else {
                    return;
                }
                self.alert_until = None;
                window.request_redraw();
            }
            View::Player | View::Lyrics => {
                let geo = self.geo();
                // Press on the bar (a taller target than the thin line) and drag to jump in the song.
                let (bx, by, bw) = geo.bar;
                if !self.media.duration.is_zero()
                    && self.media_handle.is_some()
                    && cx >= bx - 6.0
                    && cx <= bx + bw + 6.0
                    && (by - 9.0..=by + 12.0).contains(&cy)
                {
                    self.scrub = Some(self.scrub_at(cx));
                    window.request_redraw();
                    return;
                }
                // The speech-bubble button opens and closes the lyrics.
                if (cx - geo.ctrl_x[0]).hypot(cy - CTRL_Y) <= CTRL_R {
                    self.view = if self.view == View::Lyrics { View::Player } else { View::Lyrics };
                    if self.view == View::Lyrics {
                        // Opening: start on the line being sung, not wherever it was left.
                        self.lyric_manual = None;
                        self.snap_lyrics();
                    }
                    self.lyric_settle = true;
                    self.last_tick = Instant::now();
                    window.request_redraw();
                    return;
                }
                if let Some(handle) = &self.media_handle {
                    for (i, x) in geo.ctrl_x[1..].iter().enumerate() {
                        if (cx - x).hypot(cy - CTRL_Y) <= CTRL_R {
                            handle.send(match i {
                                0 => Command::Previous,
                                1 => Command::TogglePlay,
                                _ => Command::Next,
                            });
                            return;
                        }
                    }
                }
                // Sync takes the lyrics back to the line being sung.
                if self.view == View::Lyrics
                    && self.lyric_off_sync()
                    && cx >= SYNC_RECT.0
                    && cx <= SYNC_RECT.0 + SYNC_RECT.2
                    && cy >= SYNC_RECT.1
                    && cy <= SYNC_RECT.1 + SYNC_RECT.3
                {
                    self.lyric_manual = None;
                    window.request_redraw();
                    return;
                }
                // Click a lyric line to jump to it.
                if self.view != View::Lyrics || cx < LYRIC_X - 8.0 || cy < LYRIC_AREA.0 || cy > LYRIC_AREA.1 {
                    return;
                }
                let LyricsState::Found(lines) = &self.lyrics else { return };
                let hit = self
                    .layout_lyrics()
                    .into_iter()
                    .find(|it| cy >= it.top && cy <= it.top + it.rows.len() as f32 * it.lh);
                if let Some(at) = hit.and_then(|it| lines.get(it.idx)).map(|l| l.at) {
                    if let Some(handle) = &self.media_handle {
                        handle.send(Command::Seek(at));
                    }
                    // Show the jump immediately instead of waiting for the player to report it.
                    self.media.position = at;
                    self.media_at = Instant::now();
                    // The lyrics follow the music again from the line that was picked.
                    self.lyric_manual = None;
                    window.request_redraw();
                }
            }
        }
    }

    /// Lay out the visible lyric lines around the middle of the view, wrapped, in logical pixels.
    fn layout_lyrics(&self) -> Vec<LyricItem> {
        let (Some(font), LyricsState::Found(lines)) = (&self.font, &self.lyrics) else {
            return Vec::new();
        };
        let cur = self.current_line();
        // The middle of the view is a fractional line while it glides. Before the first line starts it
        // is -1, so the upcoming lines start below the middle.
        let pos = self.lyric_pos.clamp(-1.0, lines.len() as f64 - 1.0);
        let base = pos.floor() as isize;
        let frac = (pos - base as f64) as f32;
        let mid = (LYRIC_AREA.0 + LYRIC_AREA.1) / 2.0;

        let make = |d: isize| -> Option<LyricItem> {
            let idx = usize::try_from(base + d).ok().filter(|&i| i < lines.len())?;
            let near = (idx as f64 - pos).abs() as f32;
            let px = lyric_px(near);
            // The middle line is drawn bold, which is wider, so it is wrapped with the bold face too.
            let face = if near < 0.5 { self.heading_font().unwrap_or(font) } else { font };
            Some(LyricItem { idx, top: 0.0, lh: px * 1.3, px, near, current: cur == Some(idx), rows: wrap(face, &lines[idx].text, px, LYRIC_W) })
        };

        let mut items = Vec::new();
        let (mut below, h0) = match make(0) {
            Some(mut it) => {
                let h = it.rows.len() as f32 * it.lh;
                it.top = mid - h / 2.0;
                let bottom = it.top + h;
                let top = it.top;
                items.push(it);
                (bottom, (top, h))
            }
            None => (mid, (mid, 0.0)),
        };
        let mut above = h0.0;
        for d in 1..=5 {
            if let Some(mut it) = make(d) {
                it.top = below + LYRIC_GAP;
                below = it.top + it.rows.len() as f32 * it.lh;
                items.push(it);
            }
            if let Some(mut it) = make(-d) {
                let h = it.rows.len() as f32 * it.lh;
                it.top = above - LYRIC_GAP - h;
                above = it.top;
                items.push(it);
            }
        }

        // Between two lines the whole stack is part of the way from one to the next.
        let center = |it: &LyricItem| it.top + it.rows.len() as f32 * it.lh / 2.0;
        let here = items.iter().find(|i| i.idx as isize == base).map_or(mid, center);
        if let Some(next) = items.iter().find(|i| i.idx as isize == base + 1).map(center) {
            let shift = -(next - here) * frac;
            for it in &mut items {
                it.top += shift;
            }
        }
        items
    }

    /// How long the mouse rests on the notch before it opens, and how long it is away before it closes.
    fn hover_open(&self) -> Duration {
        Duration::from_millis(self.settings.open_delay as u64)
    }

    fn hover_close(&self) -> Duration {
        Duration::from_millis(self.settings.close_delay as u64)
    }

    /// How long the timer's alarm keeps ringing if nobody stops it, and how often it sounds again.
    fn alarm_for(&self) -> Duration {
        Duration::from_secs(self.settings.alarm_for as u64)
    }

    fn alarm_every(&self) -> Duration {
        Duration::from_secs(self.settings.alarm_every as u64)
    }

    /// Whether the light look is on: the setting is on and Windows is using its light theme.
    fn light(&self) -> bool {
        match self.settings.theme {
            ThemeMode::Dark => false,
            ThemeMode::Light => true,
            ThemeMode::System => self.system_light,
        }
    }

    fn draw(&mut self) {
        theme::set_light(self.light());
        let frame_start = Instant::now();
        self.glows.begin(frame_start.duration_since(self.last_draw).as_secs_f64().min(1.0 / 30.0));
        self.last_draw = frame_start;
        if self.view == View::Shelf {
            self.load_icons();
        }
        let Some(window) = self.window.clone() else { return };
        let size = window.inner_size();
        let (Some(full_w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return;
        };
        // The notch itself is drawn in the middle; the flares on its sides are added at the end.
        let s = window.scale_factor() as f32;
        let ear = (EAR as f32 * s).round();
        let Some(w) = NonZeroU32::new(full_w.get().saturating_sub(2 * ear as u32)) else { return };
        let Some(mut pixmap) = Pixmap::new(w.get(), h.get()) else { return };
        pixmap.fill(Color::TRANSPARENT);

        // The notch: square top edge, rounded bottom corners, a dark gradient and a hairline border.
        let (fw, fh) = (w.get() as f32, h.get() as f32);
        let r = (fh / 2.0).min(28.0 * s);
        let mut pb = PathBuilder::new();
        pb.move_to(0.0, 0.0);
        pb.line_to(fw, 0.0);
        pb.line_to(fw, fh - r);
        pb.quad_to(fw, fh, fw - r, fh);
        pb.line_to(r, fh);
        pb.quad_to(0.0, fh, 0.0, fh - r);
        pb.close();
        let open = self.progress.clamp(0.0, 1.0) as f32;
        if let Some(path) = pb.finish() {
            let body = theme::body();
            let stops = vec![
                tiny_skia::GradientStop::new(0.0, Color::from_rgba8(body[0][0], body[0][1], body[0][2], 255)),
                tiny_skia::GradientStop::new(0.5, Color::from_rgba8(body[1][0], body[1][1], body[1][2], 255)),
                tiny_skia::GradientStop::new(1.0, Color::from_rgba8(body[2][0], body[2][1], body[2][2], 255)),
            ];
            let shader = tiny_skia::LinearGradient::new(
                tiny_skia::Point::from_xy(0.0, 0.0),
                tiny_skia::Point::from_xy(0.0, fh),
                stops,
                tiny_skia::SpreadMode::Pad,
                Transform::identity(),
            );
            let mut paint = Paint::default();
            match shader {
                Some(shader) => paint.shader = shader,
                None => paint.set_color_rgba8(body[0][0], body[0][1], body[0][2], 255),
            }
            paint.anti_alias = true;
            pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);

            // A faint wash of the tab's colour rising from the bottom edge.
            if open > 0.0 {
                let mask = gfx::mask_of(w.get(), h.get(), &path);
                let [ar, ag, ab] = self.accent(self.view);
                gfx::glow(&mut pixmap, fw / 2.0, fh * 1.05, fw * 0.62, [ar, ag, ab, (open * 40.0) as u8], mask.as_ref());
            }
        }
        // Border along the sides and bottom; the top edge is flush with the screen.
        let hw = 0.5 * s;
        let mut pb = PathBuilder::new();
        pb.move_to(hw, 0.0);
        pb.line_to(hw, fh - r);
        pb.quad_to(hw, fh - hw, r, fh - hw);
        pb.line_to(fw - r, fh - hw);
        pb.quad_to(fw - hw, fh - hw, fw - hw, fh - r);
        pb.line_to(fw - hw, 0.0);
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            let edge = theme::border();
            paint.set_color_rgba8(edge[0], edge[1], edge[2], edge[3]);
            // The line fades in below the flares, so it does not show beside them.
            let (clear, solid) = (Color::from_rgba8(edge[0], edge[1], edge[2], 0), Color::from_rgba8(edge[0], edge[1], edge[2], edge[3]));
            if let Some(shader) = tiny_skia::LinearGradient::new(
                tiny_skia::Point::from_xy(0.0, ear),
                tiny_skia::Point::from_xy(0.0, ear * 3.0),
                vec![tiny_skia::GradientStop::new(0.0, clear), tiny_skia::GradientStop::new(1.0, solid)],
                tiny_skia::SpreadMode::Pad,
                Transform::identity(),
            ) {
                paint.shader = shader;
            }
            paint.anti_alias = true;
            let stroke = Stroke { width: 1.0 * s, ..Default::default() };
            pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }

        // Fade content in over the second half of the expand animation.
        let alpha = ((self.progress.clamp(0.0, 1.0) - 0.5) * 2.0).clamp(0.0, 1.0) as f32;
        if alpha > 0.0 {
            if self.progress < 0.995 {
                // While opening or closing the content also grows from slightly smaller, so it seems to
                // unfold with the notch.
                if let Some(mut layer) = Pixmap::new(w.get(), h.get()) {
                    self.draw_content(&mut layer, s, alpha, (w.get(), h.get()));
                    let k = (0.93 + 0.07 * self.progress.clamp(0.0, 1.0)) as f32;
                    let at = Transform::from_scale(k, k).post_translate(fw * (1.0 - k) / 2.0, 0.0);
                    let paint = PixmapPaint { quality: FilterQuality::Bilinear, ..Default::default() };
                    pixmap.draw_pixmap(0, 0, layer.as_ref(), &paint, at, None);
                }
            } else {
                self.draw_content(&mut pixmap, s, alpha, (w.get(), h.get()));
            }
        }

        // Collapsed: show the countdown while the timer runs.
        let collapsed_alpha = ((1.0 - self.progress.clamp(0.0, 1.0) * 2.0).clamp(0.0, 1.0) * (1.0 - self.ov_t.clamp(0.0, 1.0))) as f32;
        if collapsed_alpha > 0.0 {
            if let (true, Some(font)) = (self.timer.running(), &self.font) {
                let text = fmt_remaining(self.timer.remaining(Instant::now()));
                let color = [255, 255, 255, (collapsed_alpha * 255.0) as u8];
                draw_text_centered(&mut pixmap, self.heading_font().unwrap_or(font), &text, fw / 2.0, (COLLAPSED.1 as f32 / 2.0 + 5.0) * s, 13.0 * s, color, fw);
            } else {
                self.draw_companion(&mut pixmap, s, collapsed_alpha, fw);
            }
        }

        // System notice (volume, battery...) in the compact notch.
        let notice_alpha = self.ov_t.clamp(0.0, 1.0) as f32 * (1.0 - self.progress.clamp(0.0, 1.0) as f32 * 2.0).clamp(0.0, 1.0);
        if let (true, Some(notice)) = (notice_alpha > 0.02, &self.notice) {
            self.draw_notice(&mut pixmap, notice, s, notice_alpha, fw);
        }

        // Put the notch in the middle of the window and add the flares on both sides of its top edge.
        let Some(mut out) = Pixmap::new(full_w.get(), h.get()) else { return };
        out.draw_pixmap(ear as i32, 0, pixmap.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
        draw_ears(&mut out, ear);

        let Some(surface) = self.surface.as_mut() else { return };
        surface.resize(full_w, h).ok();
        let Ok(mut buffer) = surface.buffer_mut() else { return };
        // tiny-skia is premultiplied RGBA; softbuffer wants 0RGB/ARGB u32.
        for (dst, src) in buffer.iter_mut().zip(out.pixels()) {
            *dst = (src.alpha() as u32) << 24
                | (src.red() as u32) << 16
                | (src.green() as u32) << 8
                | src.blue() as u32;
        }
        buffer.present().ok();
    }

    /// The tab bar and the open view. While the view is changing, the old one fades out and drifts away
    /// as the new one fades in and slides into place, from the side its tab is on.
    fn draw_content(&self, pm: &mut Pixmap, s: f32, alpha: f32, size: (u32, u32)) {
        self.draw_tabs(pm, s, alpha);
        // The other views are drawn for the normal width, so keep them centred while it is still wide.
        let dx = if matches!(self.view, View::Player | View::Lyrics) { 0.0 } else { self.tab_dx() };
        let t = self.switch_t.clamp(0.0, 1.0) as f32;
        let moving = t < 0.999;
        if !moving && dx <= 0.5 {
            self.draw_view(pm, s, alpha, self.view);
            return;
        }
        if let (true, Some(old)) = (moving, self.prev_view)
            && let Some(mut layer) = Pixmap::new(size.0, size.1) {
                self.draw_view(&mut layer, s, alpha, old);
                let paint = PixmapPaint { opacity: (1.0 - t * 1.7).clamp(0.0, 1.0), quality: FilterQuality::Bilinear, ..Default::default() };
                let at = Transform::from_translate(-self.switch_dir * 26.0 * t * s, 0.0);
                pm.draw_pixmap(0, 0, layer.as_ref(), &paint, at, None);
            }
        if let Some(mut layer) = Pixmap::new(size.0, size.1) {
            self.draw_view(&mut layer, s, alpha, self.view);
            let fade_in = if self.prev_view.is_some() { ((t - 0.2) / 0.8).clamp(0.0, 1.0) } else { t };
            let paint = PixmapPaint { opacity: fade_in, quality: FilterQuality::Bilinear, ..Default::default() };
            let slide = if self.prev_view.is_some() { self.switch_dir * 26.0 * (1.0 - t) * s } else { 0.0 };
            let at = Transform::from_translate(dx * s + slide, if self.prev_view.is_some() { 0.0 } else { (1.0 - t) * 8.0 * s });
            pm.draw_pixmap(0, 0, layer.as_ref(), &paint, at, None);
        }
    }

    fn draw_view(&self, pm: &mut Pixmap, s: f32, alpha: f32, view: View) {
        match view {
            View::Player | View::Lyrics => self.draw_player(pm, s, alpha),
            View::Timer => self.draw_timer(pm, s, alpha),
            View::Water => self.draw_water(pm, s, alpha),
            View::Shelf => self.draw_shelf(pm, s, alpha),
            View::Todo => self.draw_todo(pm, s, alpha),
            View::Calendar => self.draw_calendar(pm, s, alpha),
            View::Clipboard => self.draw_clipboard(pm, s, alpha),
        }
    }

    fn draw_art(&self, pm: &mut Pixmap, rect: (f32, f32, f32), s: f32, alpha: f32) {
        let (ax, ay, asz) = rect;
        let radius = (asz * 0.16).min(10.0);
        let [ar, ag, ab] = self.art_tint;
        if asz > 40.0 {
            // The cover's colour spreads softly behind it.
            gfx::glow(pm, (ax + asz / 2.0) * s, (ay + asz / 2.0) * s, asz * 0.95 * s, [ar, ag, ab, (alpha * 52.0) as u8], None);
            gfx::soft_shadow(pm, ax * s, ay * s, asz * s, asz * s, radius * s, 7.0 * s, (alpha * 110.0) as u8);
        }
        let Some(shape) = gfx::rounded_path(ax * s, ay * s, asz * s, asz * s, radius * s) else { return };
        match (&self.media.art, gfx::mask_of(pm.width(), pm.height(), &shape)) {
            (Some(rgba), Some(mask)) => {
                let n = media::art_size();
                if let Some(mut art) = Pixmap::new(n, n) {
                    for (dst, px) in art.pixels_mut().iter_mut().zip(rgba.as_chunks::<4>().0) {
                        *dst = tiny_skia::ColorU8::from_rgba(px[0], px[1], px[2], 255).premultiply();
                    }
                    let scale = asz * s / n as f32;
                    let paint = PixmapPaint { opacity: alpha, quality: FilterQuality::Bilinear, ..Default::default() };
                    let at = Transform::from_scale(scale, scale).post_translate(ax * s, ay * s);
                    pm.draw_pixmap(0, 0, art.as_ref(), &paint, at, Some(&mask));
                }
            }
            _ => {
                // No cover: a soft tile with a note on it.
                let (top, bottom) = ([58, 58, 68, (alpha * 255.0) as u8], [30, 30, 38, (alpha * 255.0) as u8]);
                gfx::gradient_rounded(pm, ax * s, ay * s, asz * s, asz * s, radius * s, top, bottom, false);
                let note = [150, 150, 165, (alpha * 255.0) as u8];
                draw_tab_icon(pm, View::Player, (ax + asz / 2.0) * s, (ay + asz / 2.0) * s, s * (asz / 40.0).clamp(0.6, 1.8), note);
            }
        }
        gfx::stroke_rounded(pm, ax * s, ay * s, asz * s, asz * s, radius * s, 1.0 * s, [255, 255, 255, (alpha * 26.0) as u8]);
    }

    /// The lyrics to the right of the player: the line being sung is large and bright, the rest fade away.
    /// `dx` pushes the panel to the right, which is how it slides in while the notch widens.
    fn draw_lyric_panel(&self, pm: &mut Pixmap, s: f32, alpha: f32, dx: f32) {
        let Some(font) = &self.font else { return };
        let a = (alpha * 255.0) as u8;
        let grey = [160, 160, 165, a];

        if matches!(self.lyrics, LyricsState::Found(_)) {
            for it in self.layout_lyrics() {
                let (rgb, strength) = lyric_tone(it.near);
                for (i, row) in it.rows.iter().enumerate() {
                    let top = it.top + i as f32 * it.lh;
                    let bottom = top + it.lh;
                    // Fade rows out towards the edges of the lyrics area; skip anything outside it.
                    let edge = (top - LYRIC_AREA.0).min(LYRIC_AREA.1 - bottom);
                    if edge <= 0.0 {
                        continue;
                    }
                    let fade = (edge / 28.0).clamp(0.0, 1.0);
                    let mut color = [rgb, rgb, rgb, (a as f32 * strength * fade) as u8];
                    if it.current {
                        // The line being sung is warmed with the cover's colour, and still shows when the
                        // lyrics were scrolled away from it.
                        let [ar, ag, ab] = self.accent(View::Lyrics);
                        let warmth = if it.near < 0.5 { 0.22 } else { 0.55 };
                        color = gfx::mix(color, [ar, ag, ab, (a as f32 * strength.max(0.75) * fade) as u8], warmth);
                    }
                    let face = if it.near < 0.5 { self.heading_font().unwrap_or(font) } else { font };
                    let baseline = (top + it.px * 1.05) * s;
                    draw_text(pm, face, row, (LYRIC_X + dx) * s, baseline, it.px * s, color, (LYRIC_W + 8.0) * s);
                }
            }
            self.draw_lyric_sync(pm, font, s, alpha, dx);
        } else {
            let msg = match &self.lyrics {
                LyricsState::Loading => "Searching for lyrics\u{2026}",
                LyricsState::NotFound => "No synced lyrics found",
                _ => "Play something to see lyrics",
            };
            let y = (LYRIC_AREA.0 + LYRIC_AREA.1) / 2.0 + 5.0;
            draw_text_centered(pm, font, msg, (LYRIC_X + dx + LYRIC_W / 2.0) * s, y * s, 13.0 * s, grey, LYRIC_W * s);
        }
    }

    /// The Sync button: it fades in once the lyrics were scrolled away from the music, with an arrow
    /// that points to where the line being sung is.
    fn draw_lyric_sync(&self, pm: &mut Pixmap, font: &Font, s: f32, alpha: f32, dx: f32) {
        let show = self.glows.fade(KEY_LYRIC_SYNC, self.lyric_off_sync());
        if show < 0.01 {
            return;
        }
        let r = (SYNC_RECT.0 + dx, SYNC_RECT.1, SYNC_RECT.2, SYNC_RECT.3);
        let hot = self.hot_t(r, s);
        let k = alpha * show;
        let a = (k * 255.0) as u8;
        let [ar, ag, ab] = self.accent(View::Lyrics);
        let (x, y, w, h) = (r.0 * s, r.1 * s, r.2 * s, r.3 * s);
        // A soft backing so the button stays readable over the lines behind it.
        fill_rounded(pm, x - 3.0 * s, y - 3.0 * s, w + 6.0 * s, h + 6.0 * s, (h + 6.0 * s) / 2.0, [0, 0, 0, (k * 120.0) as u8]);
        gfx::gradient_rounded(pm, x, y, w, h, h / 2.0, [ar, ag, ab, (k * pick(120.0, 84.0, hot)) as u8], [ar, ag, ab, (k * pick(80.0, 46.0, hot)) as u8], false);
        gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [ar, ag, ab, (k * 150.0) as u8]);
        // The arrow points up when the line being sung is above the middle, down when it is below.
        let sung = self.current_line().map_or(-1.0, |c| c as f64);
        let dir = if sung < self.lyric_manual.unwrap_or(sung) { -1.0 } else { 1.0 };
        let (ax, ay) = (x + 12.0 * s, y + h / 2.0);
        fill_path(pm, &[(ax - 3.5 * s, ay - 1.5 * s * dir), (ax + 3.5 * s, ay - 1.5 * s * dir), (ax, ay + 2.8 * s * dir)], [255, 255, 255, a]);
        draw_text(pm, font, "Sync", x + 22.0 * s, y + h / 2.0 + 4.0 * s, 11.0 * s, [255, 255, 255, a], 36.0 * s);
    }

    /// The tab bar: a tinted highlight that slides to the open tab, and a hover glow.
    fn draw_tabs(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let [ar, ag, ab] = self.accent(self.view);
        let a = (alpha * 255.0) as u8;
        let tabs = self.visible_tabs();
        let (x0, y, w, h) = tab_rect(0, tabs.len());
        let dx = self.tab_dx();
        if self.tab_target().is_some() {
            // The highlight stretches a little in the direction it is moving, like a drop of liquid.
            let stretch = (self.tab_vel.abs() * 2.4).min(18.0);
            let x = (x0 + dx + self.tab_pos * (TAB_W + TAB_GAP) - stretch / 2.0) * s;
            let w = w + stretch;
            let (top, bottom) = ([ar, ag, ab, (alpha * 78.0) as u8], [ar, ag, ab, (alpha * 34.0) as u8]);
            gfx::gradient_rounded(pm, x, y * s, w * s, h * s, h * s / 2.0, top, bottom, false);
            gfx::stroke_rounded(pm, x + 0.5 * s, y * s + 0.5 * s, w * s - s, h * s - s, h * s / 2.0, 1.0 * s, [ar, ag, ab, (alpha * 90.0) as u8]);
        }
        for (i, (_, view)) in tabs.iter().enumerate() {
            let rect = tab_rect(i, tabs.len());
            let rect = (rect.0 + dx, rect.1, rect.2, rect.3);
            let near = 1.0 - (self.tab_pos - i as f32).abs().min(1.0);
            let glow = self.hot_t(rect, s) * (1.0 - near * 2.0).clamp(0.0, 1.0);
            if glow > 0.01 {
                fill_rounded(pm, rect.0 * s, rect.1 * s, rect.2 * s, rect.3 * s, rect.3 * s / 2.0, [255, 255, 255, (alpha * 16.0 * glow) as u8]);
            }
            let color = gfx::mix([150, 150, 158, a], [255, 255, 255, a], near);
            draw_tab_icon(pm, *view, (rect.0 + rect.2 / 2.0) * s, (rect.1 + rect.3 / 2.0) * s, s, color);
        }
    }

    fn draw_timer(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let Some(font) = &self.font else { return };
        let heading = self.heading_font().unwrap_or(font);
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [165, 165, 175, a]);
        let rgb = self.accent(View::Timer);
        let accent = [rgb[0], rgb[1], rgb[2], a];
        let light = gfx::mix(accent, white, 0.35);
        let now = Instant::now();
        let idle = self.timer.is_idle();
        let finished = self.alert_until.is_some() && self.alert_msg == TIMES_UP;

        // Quick choices for the length, or a reminder of the keys while a number is being typed.
        if self.timer_field.is_some() && !finished {
            draw_text_centered(pm, font, "Type digits \u{00B7} Enter to set \u{00B7} Tab for next", TIMER_COLS[1] * s, 47.0 * s, 10.0 * s, grey, 200.0 * s);
        } else if !finished {
            for (i, secs) in timer::PRESETS.iter().enumerate() {
                let r = timer_chip_rect(i);
                let on = idle && self.timer.total().as_secs() == *secs;
                draw_chip(pm, font, r, &preset_label(*secs), on, if idle { self.hot_t(r, s) } else { 0.0 }, rgb, s, alpha);
            }
        }

        // Left: a ring that empties as the time runs down, with the time inside.
        let (rx, ry, rr) = RING;
        gfx::glow(pm, rx * s, ry * s, (rr + 28.0) * s, [rgb[0], rgb[1], rgb[2], (alpha * if self.timer.running() { 44.0 } else { 26.0 }) as u8], None);
        stroke_arc(pm, rx * s, ry * s, rr * s, RING_W * s, 1.0, [255, 255, 255, (alpha * 22.0) as u8]);
        gfx::stroke_arc_gradient(pm, rx * s, ry * s, rr * s, RING_W * s, self.timer.fraction_left(now), light, accent);
        let time = fmt_remaining(self.timer.remaining(now));
        let size = if time.len() > 5 { 22.0 } else { 30.0 };
        draw_text_centered(pm, heading, &time, rx * s, (ry + size / 3.0) * s, size * s, white, (rr * 2.0 - 8.0) * s);
        let caption = if self.timer.running() {
            "Running"
        } else if self.timer.paused() {
            "Paused"
        } else {
            "Ready"
        };
        draw_text_centered(pm, font, caption, rx * s, (ry + 28.0) * s, 10.0 * s, if idle { grey } else { light }, (rr * 2.0 - 8.0) * s);

        // Right: the length as hours, minutes and seconds, each with a plus above and a minus below.
        if finished {
            draw_text_centered(pm, heading, TIMES_UP, TIMER_COLS[1] * s, 92.0 * s, 22.0 * s, accent, 190.0 * s);
            draw_text_centered(pm, font, "Press Stop to silence it", TIMER_COLS[1] * s, 112.0 * s, 11.0 * s, grey, 190.0 * s);
        } else {
            let (h, m, sec) = self.timer.parts();
            for (i, unit) in Unit::ALL.iter().enumerate() {
                let (x, value) = (TIMER_COLS[i], [h, m, sec][i]);
                for (y, plus) in [(TIMER_PLUS_Y, true), (TIMER_MINUS_Y, false)] {
                    let hot = if idle { self.hot_t((x - TIMER_BTN_R, y - TIMER_BTN_R, TIMER_BTN_R * 2.0, TIMER_BTN_R * 2.0), s) } else { 0.0 };
                    let ink = if idle { white } else { [90, 90, 100, a] };
                    draw_round_button(pm, x * s, y * s, TIMER_BTN_R * s, hot, idle, alpha);
                    fill_rect(pm, (x - 3.5) * s, (y - 0.75) * s, 7.0 * s, 1.5 * s, ink);
                    if plus {
                        fill_rect(pm, (x - 0.75) * s, (y - 3.5) * s, 1.5 * s, 7.0 * s, ink);
                    }
                }
                let ink = if idle { white } else { [120, 120, 130, a] };
                let area = timer_value_rect(i);
                let typing = self.timer_field == Some(i);
                if typing {
                    gfx::gradient_rounded(pm, area.0 * s, area.1 * s, area.2 * s, area.3 * s, 8.0 * s, [rgb[0], rgb[1], rgb[2], (alpha * 64.0) as u8], [rgb[0], rgb[1], rgb[2], (alpha * 28.0) as u8], false);
                    gfx::stroke_rounded(pm, area.0 * s + 0.5 * s, area.1 * s + 0.5 * s, area.2 * s - s, area.3 * s - s, 8.0 * s, s, [rgb[0], rgb[1], rgb[2], (alpha * 170.0) as u8]);
                } else if idle {
                    let glow = self.hot_t(area, s);
                    if glow > 0.01 {
                        fill_rounded(pm, area.0 * s, area.1 * s, area.2 * s, area.3 * s, 8.0 * s, [255, 255, 255, (alpha * 16.0 * glow) as u8]);
                    }
                }
                // While typing, the digits so far replace the number (the old one stays until the first digit).
                let shown = if typing && !self.timer_input.is_empty() { self.timer_input.clone() } else { format!("{value:02}") };
                let ink = if typing && self.timer_input.is_empty() { [150, 150, 165, a] } else { ink };
                draw_text_centered(pm, heading, &shown, (x - 5.0) * s, TIMER_VALUE_Y * s, 22.0 * s, ink, 40.0 * s);
                if typing && (self.edit_started.elapsed().as_millis() / 500).is_multiple_of(2) {
                    let cx = (x - 5.0) * s + text_width(heading, &shown, 22.0 * s) / 2.0 + 1.5 * s;
                    fill_rect(pm, cx, (TIMER_VALUE_Y - 16.0) * s, 1.5 * s, 20.0 * s, accent);
                }
                draw_text(pm, font, unit.letter(), (x + 11.0) * s, (TIMER_VALUE_Y - 1.0) * s, 11.0 * s, grey, 12.0 * s);
            }
        }

        // While the alarm rings: one big red Stop button, like a phone's timer (only this button stops it).
        if finished {
            let r = STOP_BUTTON;
            let hot = self.hot_t(r, s);
            let red = [255, 82, 72];
            gfx::glow(pm, (r.0 + r.2 / 2.0) * s, (r.1 + r.3 / 2.0) * s, 70.0 * s, [red[0], red[1], red[2], (alpha * pick(70.0, 44.0, hot)) as u8], None);
            let top = gfx::mix([red[0], red[1], red[2], a], white, pick(0.28, 0.12, hot));
            gfx::gradient_rounded(pm, r.0 * s, r.1 * s, r.2 * s, r.3 * s, r.3 * s / 2.0, top, [red[0], red[1], red[2], a], false);
            let (cx, cy) = (r.0 + 34.0, r.1 + r.3 / 2.0);
            fill_rounded(pm, (cx - 5.0) * s, (cy - 5.0) * s, 10.0 * s, 10.0 * s, 2.0 * s, white);
            draw_text_centered(pm, heading, "Stop", (r.0 + r.2 / 2.0 + 10.0) * s, (cy + 5.0) * s, 15.0 * s, white, 70.0 * s);
            return;
        }

        // Reset, and the big start / pause button.
        let reset = (TIMER_RESET_X - 26.0, TIMER_CTRL_Y - 11.0, 52.0, 22.0);
        let reset_hot = self.hot_t(reset, s);
        if reset_hot > 0.01 {
            fill_rounded(pm, reset.0 * s, reset.1 * s, reset.2 * s, reset.3 * s, 11.0 * s, [255, 255, 255, (alpha * 18.0 * reset_hot) as u8]);
        }
        draw_text_centered(pm, font, "Reset", TIMER_RESET_X * s, (TIMER_CTRL_Y + 4.0) * s, 12.0 * s, pick(white, grey, reset_hot), 60.0 * s);
        let big = self.hot_t((TIMER_START_X - 20.0, TIMER_CTRL_Y - 20.0, 40.0, 40.0), s);
        draw_big_button(pm, TIMER_START_X * s, TIMER_CTRL_Y * s, 18.0 * s, big, rgb, alpha);
        let pause = self.glows.follow(KEY_TIMER_PLAY, if self.timer.running() { 1.0 } else { 0.0 }, 30.0, 0.85) as f32;
        draw_play_pause(pm, TIMER_START_X * s, TIMER_CTRL_Y * s, s * 0.8, [0, 0, 0, a], pause);
    }

    fn draw_water(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let Some(font) = &self.font else { return };
        let heading = self.heading_font().unwrap_or(font);
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [165, 165, 175, a]);
        let rgb = self.accent(View::Water);
        let accent = [rgb[0], rgb[1], rgb[2], a];
        let now = Instant::now();

        // Reminder interval picker.
        for (i, interval) in water::INTERVALS.iter().enumerate() {
            let r = water_chip_rect(i);
            let label = match interval {
                Some(m) if m % 60 == 0 => format!("{}h", m / 60),
                Some(m) => format!("{m}m"),
                None => "Off".to_string(),
            };
            draw_chip(pm, font, r, &label, i == self.water.interval, self.hot_t(r, s), rgb, s, alpha);
        }

        // Left: a ring around a glass that fills with today's count.
        let (rx, ry, rr) = RING;
        gfx::glow(pm, rx * s, ry * s, (rr + 28.0) * s, [rgb[0], rgb[1], rgb[2], (alpha * 34.0) as u8], None);
        stroke_arc(pm, rx * s, ry * s, rr * s, RING_W * s, 1.0, [255, 255, 255, (alpha * 22.0) as u8]);
        // The level springs to its new height, so it sloshes a little when a glass is added.
        let level = (self.glows.follow(KEY_WATER, self.water.progress() as f64, 16.0, 0.55) as f32).clamp(0.0, 1.0);
        gfx::stroke_arc_gradient(pm, rx * s, ry * s, rr * s, RING_W * s, level, gfx::mix(accent, white, 0.35), accent);
        draw_water_fill(pm, rx * s, ry * s, (rr - RING_W / 2.0 - 4.0) * s, level, rgb, s, alpha);
        let count = format!("{}/{}", self.water.count, self.water.goal);
        draw_text_centered(pm, heading, &count, rx * s, (ry + 6.0) * s, 28.0 * s, white, (rr * 2.0 - 8.0) * s);
        draw_text_centered(pm, font, "glasses", rx * s, (ry + 24.0) * s, 10.0 * s, [215, 225, 240, a], (rr * 2.0 - 8.0) * s);

        // Right: title with goal adjusters, and status.
        let cx = WATER_X[1] * s;
        draw_text_centered(pm, heading, "Water", cx, 76.0 * s, 18.0 * s, white, 190.0 * s);
        for (i, (x, y)) in ADJUST.iter().enumerate() {
            let hot = self.hot_t((x - ADJUST_R, y - ADJUST_R, ADJUST_R * 2.0, ADJUST_R * 2.0), s);
            draw_round_button(pm, x * s, y * s, ADJUST_R * s, hot, true, alpha);
            fill_rect(pm, (x - 4.0) * s, (y - 0.75) * s, 8.0 * s, 1.5 * s, white);
            if i == 1 {
                fill_rect(pm, (x - 0.75) * s, (y - 4.0) * s, 1.5 * s, 8.0 * s, white);
            }
        }
        if self.alert_until.is_some() {
            for (i, row) in wrap(font, &self.alert_msg, 12.0, 190.0).iter().take(2).enumerate() {
                draw_text_centered(pm, font, row, cx, (96.0 + i as f32 * 15.0) * s, 12.0 * s, accent, 200.0 * s);
            }
        } else {
            let goal = format!("Daily goal: {} glasses", self.water.goal);
            draw_text_centered(pm, font, &goal, cx, 96.0 * s, 12.0 * s, grey, 190.0 * s);
            let (text, color) = if self.water.reached() {
                ("Goal reached!".to_string(), accent)
            } else {
                match self.water.next_in(now) {
                    // Round up so it never shows "0 min" before the reminder fires.
                    Some(d) => (format!("Next reminder in {} min", d.as_secs().div_ceil(60).max(1)), grey),
                    None => ("Reminders off".to_string(), grey),
                }
            };
            draw_text_centered(pm, font, &text, cx, 114.0 * s, 11.0 * s, color, 190.0 * s);
        }

        // Undo and Reset, and the big add button.
        for (x, label) in [(WATER_X[0], "Undo"), (WATER_X[2], "Reset")] {
            let r = (x - 26.0, WATER_CTRL_Y - 11.0, 52.0, 22.0);
            let hot = self.hot_t(r, s);
            if hot > 0.01 {
                fill_rounded(pm, r.0 * s, r.1 * s, r.2 * s, r.3 * s, 11.0 * s, [255, 255, 255, (alpha * 18.0 * hot) as u8]);
            }
            draw_text_centered(pm, font, label, x * s, (WATER_CTRL_Y + 4.0) * s, 12.0 * s, pick(white, grey, hot), 60.0 * s);
        }
        let big = self.hot_t((WATER_X[1] - 20.0, WATER_CTRL_Y - 20.0, 40.0, 40.0), s);
        draw_big_button(pm, cx, WATER_CTRL_Y * s, 18.0 * s, big, rgb, alpha);
        let black = [0, 0, 0, a];
        fill_rect(pm, cx - 7.0 * s, (WATER_CTRL_Y - 1.25) * s, 14.0 * s, 2.5 * s, black);
        fill_rect(pm, cx - 1.25 * s, (WATER_CTRL_Y - 7.0) * s, 2.5 * s, 14.0 * s, black);
    }

    fn draw_shelf(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let Some(font) = &self.font else { return };
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [165, 165, 175, a]);
        let width = EXPANDED.0 as f32 * s;
        let rgb = self.accent(View::Shelf);
        let heading = self.heading_font().unwrap_or(font);

        let empty = self.shelf.items.is_empty();
        let zone = (24.0, 36.0, 372.0, 124.0);
        if self.drag_hover {
            gfx::glow(pm, width / 2.0, 98.0 * s, 150.0 * s, [rgb[0], rgb[1], rgb[2], (alpha * 40.0) as u8], None);
            gfx::gradient_rounded(pm, zone.0 * s, zone.1 * s, zone.2 * s, zone.3 * s, 16.0 * s, [rgb[0], rgb[1], rgb[2], (alpha * 62.0) as u8], [rgb[0], rgb[1], rgb[2], (alpha * 26.0) as u8], false);
            gfx::stroke_rounded(pm, zone.0 * s, zone.1 * s, zone.2 * s, zone.3 * s, 16.0 * s, 1.5 * s, [rgb[0], rgb[1], rgb[2], (alpha * 170.0) as u8]);
        } else if empty {
            draw_card(pm, zone, 16.0, s, alpha, 0.0, rgb);
        }

        if empty {
            let ink = if self.drag_hover { white } else { [190, 195, 235, a] };
            draw_tab_icon(pm, View::Shelf, width / 2.0, 70.0 * s, s * 2.2, ink);
            let title = if self.drag_hover { "Release to add" } else { "Drop files here" };
            draw_text_centered(pm, heading, title, width / 2.0, 108.0 * s, 15.0 * s, white, width - 80.0 * s);
            let hint = "They stay until you drag them out or remove them";
            draw_text_centered(pm, font, hint, width / 2.0, 128.0 * s, 11.0 * s, grey, width - 80.0 * s);
            return;
        }

        // Clear button: takes every file off the shelf (the files themselves are never touched).
        {
            let (x, y, w, h) = SHELF_CLEAR;
            let hot = self.hot_t(SHELF_CLEAR, s);
            let red = [255, 96, 86];
            gfx::gradient_rounded(pm, x * s, y * s, w * s, h * s, h * s / 2.0, [red[0], red[1], red[2], (alpha * pick(80.0, 48.0, hot)) as u8], [red[0], red[1], red[2], (alpha * pick(46.0, 22.0, hot)) as u8], false);
            gfx::stroke_rounded(pm, x * s + 0.5 * s, y * s + 0.5 * s, w * s - s, h * s - s, h * s / 2.0, s, [red[0], red[1], red[2], (alpha * 90.0) as u8]);
            draw_text_centered(pm, font, "Clear", (x + w / 2.0) * s, (y + 11.5) * s, 10.0 * s, [255, 130, 120, a], w * s);
        }

        const PALETTE: [[u8; 3]; 6] =
            [[255, 99, 71], [80, 200, 120], [90, 150, 255], [255, 190, 70], [190, 120, 255], [70, 200, 210]];
        for slot in 0..COLS * ROWS {
            let Some(i) = self.shelf.index_at(slot) else { continue };
            let path = &self.shelf.items[i];
            let rect = tile_rect(slot);
            let (x, y, w, h) = rect;
            let hot = self.hot_t(rect, s);
            draw_card(pm, rect, 10.0, s, alpha, hot, rgb);
            // A highlighted file glows in the accent colour; it eases in and out.
            let chosen = self.toggle_t(motion::rect_key(rect) >> 1, self.selected.contains(path)).clamp(0.0, 1.0);
            if chosen > 0.01 {
                let tint = |v: f32| [rgb[0], rgb[1], rgb[2], (alpha * v * chosen) as u8];
                gfx::gradient_rounded(pm, x * s, y * s, w * s, h * s, 10.0 * s, tint(90.0), tint(40.0), false);
                gfx::stroke_rounded(pm, x * s + s, y * s + s, w * s - 2.0 * s, h * s - 2.0 * s, 10.0 * s, 1.5 * s, tint(230.0));
            }

            if let Some(Some(icon)) = self.icons.get(path) {
                let size = 32.0 * s;
                let scale = size / icon.width() as f32;
                let paint = PixmapPaint { opacity: alpha, quality: FilterQuality::Bilinear, ..Default::default() };
                let at = Transform::from_scale(scale, scale).post_translate((x + (w - 32.0) / 2.0) * s, (y + 7.0) * s);
                pm.draw_pixmap(0, 0, icon.as_ref(), &paint, at, None);
            } else {
                // No shell icon available: fall back to a colored extension badge.
                let badge = shelf::badge(path);
                let c = PALETTE[badge.bytes().map(usize::from).sum::<usize>() % PALETTE.len()];
                let top = [c[0], c[1], c[2], a];
                gfx::gradient_rounded(pm, (x + (w - 34.0) / 2.0) * s, (y + 8.0) * s, 34.0 * s, 18.0 * s, 5.0 * s, gfx::mix(top, white, 0.25), top, false);
                draw_text_centered(pm, font, &badge, (x + w / 2.0) * s, (y + 21.0) * s, 10.0 * s, [0, 0, 0, a], 32.0 * s);
            }

            draw_text_centered(pm, font, &shelf::file_name(path), (x + w / 2.0) * s, (y + h - 10.0) * s, 10.0 * s, pick(white, [225, 225, 232, a], hot), (w - 8.0) * s);
            // Remove button (takes the file off the shelf, never deletes it).
            let over_x = self.hot_t((x + w - 16.0, y, 16.0, 16.0), s);
            if over_x > 0.01 {
                fill_circle(pm, (x + w - 8.0) * s, (y + 9.0) * s, 7.0 * s, [255, 90, 80, (alpha * 70.0 * over_x) as u8]);
            }
            draw_text_centered(pm, font, "\u{00D7}", (x + w - 8.0) * s, (y + 13.0) * s, 13.0 * s, pick(white, grey, over_x), 12.0 * s);
        }

        // Scroll indicator when there are more rows than fit.
        let total_rows = self.shelf.items.len().div_ceil(COLS);
        if total_rows > ROWS {
            let (top, height) = (TILES_ORIGIN.1, 122.0);
            fill_rounded(pm, 406.0 * s, top * s, 3.0 * s, height * s, 1.5 * s, [255, 255, 255, (alpha * 22.0) as u8]);
            let thumb = height * ROWS as f32 / total_rows as f32;
            let at = top + (height - thumb) * self.shelf.scroll as f32 / (total_rows - ROWS) as f32;
            fill_rounded(pm, 406.0 * s, at * s, 3.0 * s, thumb * s, 1.5 * s, [rgb[0], rgb[1], rgb[2], a]);
        }
    }

    fn draw_notice(&self, pm: &mut Pixmap, notice: &sys::Notice, real_s: f32, alpha: f32, fw: f32) {
        use sys::{BatteryKind, Notice};
        let Some(font) = &self.font else { return };
        // Laid out for a 300 wide notch, shrunk to the size of the notch it is shown in.
        let s = real_s * (OVERLAY.0 / 300.0) as f32;
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [160, 160, 165, a]);
        let (green, red, blue) = ([80, 200, 120, a], [255, 90, 80, a], [64, 170, 255, a]);

        // Content block, centered in the notch.
        let (left, right, cy) = (fw / 2.0 - 120.0 * s, fw / 2.0 + 120.0 * s, OVERLAY.1 as f32 / 2.0 * real_s);
        let icon_x = left + 10.0 * s;
        let text_x = left + 34.0 * s;
        let right_text = |pm: &mut Pixmap, text: &str, color: [u8; 4]| {
            let w = text_width(font, text, 13.0 * s);
            draw_text(pm, font, text, right - w, cy + 5.0 * s, 13.0 * s, color, 120.0 * s);
        };

        match notice {
            Notice::Volume { percent, muted } => {
                draw_speaker(pm, icon_x, cy, s, if *muted { grey } else { white }, *muted);
                draw_bar(pm, text_x, cy, 140.0 * s, if *muted { 0.0 } else { *percent as f32 / 100.0 }, blue, s, a);
                right_text(pm, &if *muted { "Muted".to_string() } else { format!("{percent}%") }, if *muted { grey } else { white });
            }
            Notice::Brightness { percent } => {
                draw_sun(pm, icon_x, cy, s, white);
                draw_bar(pm, text_x, cy, 140.0 * s, *percent as f32 / 100.0, [255, 200, 80, a], s, a);
                right_text(pm, &format!("{percent}%"), white);
            }
            Notice::Lock { name, on } => {
                fill_rounded(pm, icon_x - 10.0 * s, cy - 9.0 * s, 20.0 * s, 18.0 * s, 4.0 * s, [255, 255, 255, (alpha * 45.0) as u8]);
                let glyph = if *name == "Caps Lock" { "A" } else { "1" };
                draw_text_centered(pm, font, glyph, icon_x, cy + 5.0 * s, 13.0 * s, white, 18.0 * s);
                draw_text(pm, font, name, text_x, cy + 5.0 * s, 14.0 * s, white, 120.0 * s);
                right_text(pm, if *on { "On" } else { "Off" }, if *on { green } else { grey });
            }
            Notice::Battery { percent, kind } => {
                let color = match kind {
                    BatteryKind::Low => red,
                    BatteryKind::Plugged => green,
                    BatteryKind::Unplugged => white,
                };
                draw_battery(pm, icon_x, cy, s, *percent as f32 / 100.0, color, a);
                let label = match kind {
                    BatteryKind::Plugged => "Charging",
                    BatteryKind::Unplugged => "On battery",
                    BatteryKind::Low => "Battery low",
                };
                draw_text(pm, font, label, text_x, cy + 5.0 * s, 14.0 * s, if *kind == BatteryKind::Low { red } else { white }, 120.0 * s);
                right_text(pm, &format!("{percent}%"), color);
            }
            Notice::Network { connected, name, wifi } => {
                draw_wifi(pm, icon_x, cy, s, if *connected { blue } else { grey }, !*connected);
                let label = match (*connected, name.is_empty()) {
                    (true, false) => name.as_str(),
                    (true, true) => "Connected",
                    (false, _) => "No internet",
                };
                draw_text(pm, font, label, text_x, cy + 5.0 * s, 14.0 * s, white, 110.0 * s);
                let kind = match (*connected, *wifi) {
                    (false, _) => "Offline",
                    (true, true) => "Wi-Fi",
                    (true, false) => "Online",
                };
                right_text(pm, kind, if *connected { grey } else { red });
            }
            Notice::Audio { name, headphones } => {
                draw_headphones(pm, icon_x, cy, s, white);
                draw_text(pm, font, name, text_x, cy + 5.0 * s, 14.0 * s, white, 120.0 * s);
                right_text(pm, if *headphones { "Headphones" } else { "Output" }, grey);
            }
        }
    }

    /// Tray icon with a small menu, so the app can be closed without opening the notch.
    fn build_tray(&mut self) {
        let menu = Menu::new();
        let toggle = MenuItem::new(island_label(self.settings.hide_island), true, None);
        let open = MenuItem::new("Settings\u{2026}", true, None);
        let quit = MenuItem::new("Quit Sky Island", true, None);
        if menu.append_items(&[&toggle, &open, &quit]).is_err() {
            return;
        }
        let Ok(icon) = Icon::from_rgba(appicon::rgba(32), 32, 32) else { return };
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Sky Island")
            .with_icon(icon)
            .build();

        let proxy = std::sync::Mutex::new(self.proxy.clone());
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Ok(p) = proxy.lock() {
                let _ = p.send_event(UserEvent::Menu(event.id));
            }
        }));
        self.tray_toggle_id = Some(toggle.id().clone());
        self.tray_toggle = Some(toggle);
        self.tray_settings_id = Some(open.id().clone());
        self.tray_quit_id = Some(quit.id().clone());
        self.tray = tray.ok();
    }

    /// Make clicks on the notch not steal focus from the user's current app.
    fn make_non_activating(&self) {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_EX_NOACTIVATE};
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let Some(window) = &self.window else { return };
        if let Ok(handle) = window.window_handle()
            && let RawWindowHandle::Win32(h) = handle.as_raw() {
                let hwnd = HWND(h.hwnd.get() as *mut _);
                unsafe {
                    let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_NOACTIVATE.0 as isize);
                }
            }
    }

    /// Open the editor for a new task, or for the task at `target`. Remember which app had focus,
    /// then take it so key presses reach us.
    fn start_editing(&mut self, target: Option<usize>) {
        use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
        let fg = unsafe { GetForegroundWindow() };
        self.prev_foreground = (!fg.is_invalid()).then_some(fg);
        let clock = clock_now();
        match target.and_then(|i| self.todo.tasks.get(i).map(|task| (i, task))) {
            Some((i, task)) => {
                self.editor = editor::Editor::for_task(i, task, clock);
                self.input = task.text.clone();
            }
            None => {
                // A task added on the calendar starts on the selected day.
                let date = (self.view == View::Calendar).then_some(self.cal.selected);
                self.editor = editor::Editor::new_task(date, clock);
                self.input.clear();
            }
        }
        self.editing = true;
        self.edit_started = Instant::now();
        if let Some(w) = &self.window {
            w.focus_window();
        }
    }

    /// Stop typing and give focus back to the app the user was in.
    fn stop_editing(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;
        if !self.editing {
            return;
        }
        self.editing = false;
        self.last_tick = Instant::now();
        if let Some(h) = self.prev_foreground.take() {
            let _ = unsafe { SetForegroundWindow(h) };
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Start typing one of the timer's numbers: take the keyboard, remembering where it came from.
    fn begin_timer_edit(&mut self, field: usize) {
        use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
        if self.timer_field.is_none() && self.prev_foreground.is_none() {
            let fg = unsafe { GetForegroundWindow() };
            self.prev_foreground = (!fg.is_invalid()).then_some(fg);
        }
        self.timer_field = Some(field);
        self.timer_input.clear();
        self.edit_started = Instant::now();
        if let Some(w) = &self.window {
            w.focus_window();
            w.request_redraw();
        }
    }

    /// Stop typing a timer number, keeping what was typed if `keep`, and give the keyboard back.
    fn end_timer_edit(&mut self, keep: bool) {
        use windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;
        if keep {
            self.apply_timer_input();
        }
        self.timer_field = None;
        self.timer_input.clear();
        self.last_tick = Instant::now();
        if let Some(h) = self.prev_foreground.take() {
            let _ = unsafe { SetForegroundWindow(h) };
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Put the digits typed so far into the timer's length.
    fn apply_timer_input(&mut self) {
        if let (Some(field), Ok(value)) = (self.timer_field, self.timer_input.parse::<u64>()) {
            self.timer.set_part(Unit::ALL[field], value);
        }
        self.timer_input.clear();
    }

    /// Carry on typing in the next (`dir` = 1) or previous (-1) number.
    fn move_timer_field(&mut self, dir: isize) {
        let Some(field) = self.timer_field else { return };
        self.apply_timer_input();
        self.timer_field = Some((field as isize + dir).rem_euclid(Unit::ALL.len() as isize) as usize);
        self.edit_started = Instant::now();
    }

    /// A key pressed while a timer number is being typed.
    fn timer_key(&mut self, event: &winit::event::KeyEvent) {
        let Some(field) = self.timer_field else { return };
        match &event.logical_key {
            Key::Named(NamedKey::Enter) => self.end_timer_edit(true),
            Key::Named(NamedKey::Escape) => self.end_timer_edit(false),
            Key::Named(NamedKey::Backspace) => {
                self.timer_input.pop();
            }
            Key::Named(NamedKey::Tab) | Key::Named(NamedKey::ArrowRight) => self.move_timer_field(1),
            Key::Named(NamedKey::ArrowLeft) => self.move_timer_field(-1),
            Key::Named(key @ (NamedKey::ArrowUp | NamedKey::ArrowDown)) => {
                // Up and down change the number by one, a second at a time.
                self.apply_timer_input();
                self.timer.adjust(Unit::ALL[field], if *key == NamedKey::ArrowUp { 1 } else { -1 });
            }
            _ => {
                if let Some(text) = &event.text {
                    for c in text.chars() {
                        if c.is_ascii_digit() {
                            if self.timer_input.len() < 2 {
                                self.timer_input.push(c);
                            }
                        } else if matches!(c, ':' | '.' | ',' | ' ') {
                            // Typing 1:30:00 moves along as you go.
                            self.move_timer_field(1);
                        }
                    }
                }
            }
        }
    }

    /// Keep the tray entry's text in line with the "Auto-hide island" setting.
    fn sync_tray_toggle(&self) {
        if let Some(item) = &self.tray_toggle {
            item.set_text(island_label(self.settings.hide_island));
        }
    }

    /// The timer's alarm: the sound file chosen in the settings window, or the built-in chime when there
    /// is none (or it can't be played). A long file that is still playing is left to finish.
    fn play_alarm(&self) {
        if !self.settings.sound {
            return;
        }
        if let Some(path) = &self.settings.alarm_sound
            && sound::play_file(path, self.settings.volume) {
                return;
            }
        sound::play(sound::Chime::TimerDone, self.settings.volume);
    }

    fn play_sound(&self, chime: sound::Chime) {
        if self.settings.sound {
            sound::play(chime, self.settings.volume);
        }
    }

    /// Save the task in the editor. Blank text is refused and the editor stays open.
    fn commit_task(&mut self) {
        let (due, date) = (self.editor.due(clock_now()), self.editor.date);
        let saved = match self.editor.index {
            Some(i) => self.todo.update(i, &self.input, due, date),
            None => self.todo.add(&self.input, due, date),
        };
        if saved {
            self.stop_editing();
        }
    }

    /// Hide the window while a full-screen app is in front, if that setting is on.
    fn apply_visibility(&mut self) {
        let hide = self.fullscreen && self.settings.hide_fullscreen;
        if hide != self.hidden {
            self.hidden = hide;
            if hide {
                // A hidden window gets no cursor events, so don't leave it looking hovered.
                self.hovered = false;
                self.enter_at = None;
                self.leave_at = None;
            }
            if let Some(w) = &self.window {
                w.set_visible(!hide);
            }
        }
    }

    /// Schedule and trigger the redraws the closed notch needs: clock minute changes, blinking
    /// eyes, dozing animation and the greeting. Does nothing when something else is showing.
    /// Whether the awake eyes are drawn on the closed notch right now: while you are working they take
    /// the place of the clock (so only when the clock is off), and after a while without use they come
    /// from "Idle eyes".
    fn eyes_open(&self) -> bool {
        match self.presence {
            Presence::Active => self.settings.eyes && !self.settings.show_clock,
            Presence::Idle => self.settings.idle_eyes,
            Presence::Dozing => false,
        }
    }

    fn companion_tick(&mut self, now: Instant, wake: &mut Option<Instant>) {
        if self.hidden || self.tuck_t >= 1.0 || self.progress > 0.0 || self.notice_until.is_some() || self.timer.running() {
            return;
        }
        let mut redraw = false;

        if let Some(g) = self.greeting_until {
            if now >= g {
                self.greeting_until = None;
                redraw = true;
            } else {
                earliest(wake, g);
                return;
            }
        }

        match self.presence {
            Presence::Active if self.settings.show_clock => {
                let (_, minute, sec, ms) = local_time();
                if minute != self.last_minute {
                    self.last_minute = minute;
                    redraw = true;
                }
                // Wake just after the next minute starts.
                let to_next = (59 - sec) * 1000 + (1000 - ms) + 30;
                earliest(wake, now + Duration::from_millis(to_next as u64));
            }
            Presence::Active | Presence::Idle if self.eyes_open() => {
                if let Some(b) = self.blink_until {
                    if now >= b {
                        self.blink_until = None;
                        redraw = true;
                    } else {
                        earliest(wake, b);
                    }
                }
                if self.blink_until.is_none() {
                    if now >= self.next_blink {
                        self.blink_until = Some(now + BLINK);
                        self.blink_count += 1;
                        // Irregular gaps between blinks, 3 to 7 seconds.
                        self.next_blink = now + Duration::from_millis(3000 + (self.blink_count as u64 * 1373) % 4000);
                        redraw = true;
                        earliest(wake, now + BLINK);
                    } else {
                        earliest(wake, self.next_blink);
                    }
                }
            }
            Presence::Dozing if self.settings.idle_eyes => {
                if now >= self.next_zz {
                    self.zz_frame = !self.zz_frame;
                    self.next_zz = now + Duration::from_millis(1100);
                    redraw = true;
                }
                earliest(wake, self.next_zz);
            }
            _ => {}
        }

        if redraw
            && let Some(w) = &self.window {
                w.request_redraw();
            }
    }

    /// What the closed notch shows when nothing else needs it: eyes when idle, otherwise the clock.
    /// One open (or blinking) eye on the closed notch, in the chosen style, centred at `x`, `cy`.
    #[allow(clippy::too_many_arguments)]
    fn draw_eye(&self, pm: &mut Pixmap, x: f32, cy: f32, s: f32, a: u8, blink: bool, glance: f32) {
        let c = eye_colour(self.settings.eye_style);
        let ink = theme::map([c[0], c[1], c[2], a]);
        let mut paint = Paint::default();
        paint.set_color_rgba8(ink[0], ink[1], ink[2], ink[3]);
        paint.anti_alias = true;
        // (width, height) of the eye; a blink squashes it to a thin line.
        let (w, open_h) = match self.settings.eye_style {
            EyeStyle::Classic => (9.0, 12.0),
            EyeStyle::Dots => (8.0, 8.0),
            EyeStyle::Cat => (10.0, 12.0),
            EyeStyle::Glow => (7.0, 13.0),
        };
        let (w, h) = (w * s, if blink { 1.6 } else { open_h } * s);
        if self.settings.eye_style == EyeStyle::Glow && !blink {
            gfx::glow(pm, x, cy, 13.0 * s, [c[0], c[1], c[2], (a as f32 * 0.45) as u8], None);
        }
        if let Some(rect) = Rect::from_xywh(x - w / 2.0, cy - h / 2.0, w, h)
            && let Some(path) = PathBuilder::from_oval(rect) {
                pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
            }
        if blink {
            return;
        }
        match self.settings.eye_style {
            EyeStyle::Classic => fill_circle(pm, x + glance * 1.5 * s, cy + 1.0 * s, 3.0 * s, [0, 0, 0, a]),
            EyeStyle::Cat => {
                // A narrow slit that drifts with the glance.
                if let Some(rect) = Rect::from_xywh(x + glance * 1.2 * s - 1.2 * s, cy - 5.0 * s, 2.4 * s, 10.0 * s)
                    && let Some(path) = PathBuilder::from_oval(rect) {
                        let mut slit = Paint::default();
                        slit.set_color_rgba8(0, 0, 0, a);
                        slit.anti_alias = true;
                        pm.fill_path(&path, &slit, FillRule::Winding, Transform::identity(), None);
                    }
            }
            EyeStyle::Dots | EyeStyle::Glow => {}
        }
    }

    fn draw_companion(&self, pm: &mut Pixmap, s: f32, alpha: f32, fw: f32) {
        let Some(font) = &self.font else { return };
        let a = (alpha * 255.0) as u8;
        let (cx, cy) = (fw / 2.0, COLLAPSED.1 as f32 / 2.0 * s);
        let (left, right) = (cx - 13.0 * s, cx + 13.0 * s);
        let eyes = self.settings.idle_eyes;
        let white = {
            let c = eye_colour(self.settings.eye_style);
            [c[0], c[1], c[2], a]
        };

        if eyes && self.greeting_until.is_some() {
            for x in [left, right] {
                stroke_arc_from(pm, x, cy + 3.0 * s, 5.0 * s, 2.0 * s, -std::f32::consts::PI + 0.35, std::f32::consts::PI - 0.7, white);
            }
            draw_text(pm, font, "hi!", cx + 28.0 * s, cy + 4.0 * s, 11.0 * s, white, 30.0 * s);
            return;
        }

        match self.presence {
            Presence::Active | Presence::Idle if self.eyes_open() => {
                let blink = self.blink_until.is_some();
                let glance = (self.blink_count % 3) as f32 - 1.0;
                for x in [left, right] {
                    self.draw_eye(pm, x, cy, s, a, blink, glance);
                }
            }
            Presence::Dozing if eyes => {
                for x in [left, right] {
                    stroke_arc_from(pm, x, cy - 2.0 * s, 5.0 * s, 2.0 * s, 0.3, std::f32::consts::PI - 0.6, white);
                }
                let grey = [160, 160, 165, a];
                draw_text(pm, font, "z", cx + 26.0 * s, cy + 2.0 * s, 9.0 * s, grey, 12.0 * s);
                if self.zz_frame {
                    draw_text(pm, font, "Z", cx + 34.0 * s, cy - 5.0 * s, 11.0 * s, grey, 14.0 * s);
                }
            }
            _ if self.settings.show_clock => {
                let (hour, minute, _, _) = local_time();
                let text = fmt_clock(hour, minute, self.settings.clock_24h);
                draw_text_centered(pm, self.heading_font().unwrap_or(font), &text, cx, (COLLAPSED.1 as f32 / 2.0 + 5.0) * s, 13.0 * s, [222, 222, 230, a], fw);
            }
            _ => {}
        }
    }

    fn draw_todo(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let Some(font) = &self.font else { return };
        let heading = self.heading_font().unwrap_or(font);
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [165, 165, 175, a]);
        let rgb = TODO_AMBER;
        let amber = [rgb[0], rgb[1], rgb[2], a];
        let width = EXPANDED.0 as f32 * s;
        let (now, today, offset) = (todo::unix_now(), today_day(), tz_offset());

        if self.editing {
            self.draw_editor(pm, s, alpha);
            return;
        }

        // "Add a task" field, which also shows a banner when a reminder fires.
        let field = TODO_FIELD;
        let (fx, fy, fw, fh) = field;
        let hot = self.hot_t(field, s);
        if self.alert_until.is_some() {
            gfx::gradient_rounded(pm, fx * s, fy * s, fw * s, fh * s, 12.0 * s, [rgb[0], rgb[1], rgb[2], (alpha * 70.0) as u8], [rgb[0], rgb[1], rgb[2], (alpha * 36.0) as u8], false);
            gfx::stroke_rounded(pm, fx * s + 0.5 * s, fy * s + 0.5 * s, fw * s - s, fh * s - s, 12.0 * s, s, [rgb[0], rgb[1], rgb[2], (alpha * 120.0) as u8]);
            draw_text(pm, heading, &self.alert_msg, (fx + 12.0) * s, (fy + 16.0) * s, 12.0 * s, white, (fw - 24.0) * s);
        } else {
            draw_card(pm, field, 12.0, s, alpha, hot, rgb);
            gfx::gradient_circle(pm, (fx + 15.0) * s, (fy + fh / 2.0) * s, 8.0 * s, gfx::mix(amber, white, 0.3), amber);
            fill_rect(pm, (fx + 15.0 - 4.0) * s, (fy + fh / 2.0 - 0.75) * s, 8.0 * s, 1.5 * s, [0, 0, 0, a]);
            fill_rect(pm, (fx + 15.0 - 0.75) * s, (fy + fh / 2.0 - 4.0) * s, 1.5 * s, 8.0 * s, [0, 0, 0, a]);
            draw_text(pm, font, "Add a task", (fx + 30.0) * s, (fy + 16.0) * s, 12.0 * s, pick(white, grey, hot), (fw - 44.0) * s);
        }

        if self.todo.tasks.is_empty() {
            gfx::glow(pm, width / 2.0, 98.0 * s, 70.0 * s, [rgb[0], rgb[1], rgb[2], (alpha * 36.0) as u8], None);
            draw_tab_icon(pm, View::Todo, width / 2.0, 90.0 * s, s * 2.4, [rgb[0], rgb[1], rgb[2], a]);
            draw_text_centered(pm, heading, "Nothing to do", width / 2.0, 126.0 * s, 15.0 * s, white, width - 80.0 * s);
            draw_text_centered(pm, font, "Click the box above to add a task", width / 2.0, 146.0 * s, 11.0 * s, grey, width - 80.0 * s);
            return;
        }

        for row in 0..todo::VISIBLE {
            let Some(i) = self.todo.index_at(row) else { continue };
            let task = &self.todo.tasks[i];
            let y = TODO_ROWS_Y + row as f32 * TODO_ROW_H;
            let rect = (24.0, y + 1.0, 372.0, TODO_ROW_H - 2.0);
            let hot = self.hot_t(rect, s);
            if hot > 0.01 {
                gfx::gradient_rounded(pm, rect.0 * s, rect.1 * s, rect.2 * s, rect.3 * s, 8.0 * s, [255, 255, 255, (alpha * 40.0 * hot) as u8], [255, 255, 255, (alpha * 20.0 * hot) as u8], false);
                gfx::stroke_rounded(pm, rect.0 * s + 0.5 * s, rect.1 * s + 0.5 * s, rect.2 * s - s, rect.3 * s - s, 8.0 * s, s, [rgb[0], rgb[1], rgb[2], (alpha * 70.0 * hot) as u8]);
            }

            // Checkbox.
            draw_check(pm, 28.0 * s, (y + 5.0) * s, 14.0 * s, task.done, amber, a);

            // When it is due (or its day) as a small tag, so the text only gets the room that is left.
            let when = task_when(task, today, now, offset, self.settings.clock_24h);
            let tag_w = when.as_ref().map_or(0.0, |(label, _)| text_width(font, label, 10.0 * s) / s + 12.0);
            let text_w = ((346.0 - 52.0 - 8.0) * s - tag_w * s).min(250.0 * s);

            // Text, struck through when done.
            let color = if task.done { [120, 120, 130, a] } else { white };
            draw_text(pm, font, &task.text, 52.0 * s, (y + 16.0) * s, 12.0 * s, color, text_w);
            if task.done {
                let w = text_width(font, &task.text, 12.0 * s).min(text_w);
                fill_rect(pm, 52.0 * s, (y + 12.0) * s, w, 1.2 * s, color);
            }
            if let Some((label, overdue)) = &when {
                let (tint, ink) = if task.done {
                    ([255, 255, 255], [120, 120, 130, a])
                } else if *overdue {
                    ([255, 96, 86], [255, 130, 120, a])
                } else if task.due.is_some() {
                    (rgb, [255, 214, 140, a])
                } else {
                    ([255, 255, 255], [165, 165, 175, a])
                };
                let (tx, tw) = (346.0 - tag_w, tag_w);
                fill_rounded(pm, tx * s, (y + 4.5) * s, tw * s, 15.0 * s, 7.5 * s, [tint[0], tint[1], tint[2], (alpha * if tint == [255, 255, 255] { 16.0 } else { 40.0 }) as u8]);
                draw_text_centered(pm, font, label, (tx + tw / 2.0) * s, (y + 15.5) * s, 10.0 * s, ink, tw * s);
            }

            // Edit and remove buttons.
            let over_edit = self.hot_t((346.0, y, 26.0, TODO_ROW_H), s);
            let over_remove = self.hot_t((372.0, y, 24.0, TODO_ROW_H), s);
            draw_pencil(pm, 358.0 * s, (y + 12.0) * s, s, pick(white, [120, 120, 130, a], over_edit));
            draw_text_centered(pm, font, "\u{00D7}", 384.0 * s, (y + 17.0) * s, 13.0 * s, pick([255, 120, 110, a], grey, over_remove), 14.0 * s);
        }

        // Scroll indicator when there are more tasks than rows.
        let total = self.todo.tasks.len();
        if total > todo::VISIBLE {
            let (top, height) = (TODO_ROWS_Y, TODO_ROW_H * todo::VISIBLE as f32);
            fill_rounded(pm, 410.0 * s, top * s, 3.0 * s, height * s, 1.5 * s, [255, 255, 255, (alpha * 22.0) as u8]);
            let thumb = height * todo::VISIBLE as f32 / total as f32;
            let at = top + (height - thumb) * self.todo.scroll as f32 / (total - todo::VISIBLE) as f32;
            fill_rounded(pm, 410.0 * s, at * s, 3.0 * s, thumb * s, 1.5 * s, amber);
        }
    }

    fn draw_calendar(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let Some(font) = &self.font else { return };
        let heading = self.heading_font().unwrap_or(font);
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [165, 165, 175, a]);
        let amber = [TODO_AMBER[0], TODO_AMBER[1], TODO_AMBER[2], a];
        let rgb = self.accent(View::Calendar);
        let violet = [rgb[0], rgb[1], rgb[2], a];
        let (today, now, offset) = (today_day(), todo::unix_now(), tz_offset());
        let selected = self.cal.selected;

        if self.editing {
            self.draw_editor(pm, s, alpha);
            return;
        }

        // Month heading with previous / next arrows.
        for (rect, dir) in [(CAL_PREV, -1.0), (CAL_NEXT, 1.0)] {
            let (cx, cy) = ((rect.0 + rect.2 / 2.0) * s, (rect.1 + rect.3 / 2.0) * s);
            let glow = self.hot_t(rect, s);
            if glow > 0.01 {
                fill_circle(pm, cx, cy, 9.0 * s, [255, 255, 255, (alpha * 22.0 * glow) as u8]);
            }
            let tip = [(cx - 2.0 * dir * s, cy - 4.0 * s), (cx + 2.0 * dir * s, cy), (cx - 2.0 * dir * s, cy + 4.0 * s)];
            stroke_poly(pm, &tip, 1.7 * s, pick(white, grey, glow), false);
        }
        draw_text_centered(pm, heading, &self.cal.title(), (CAL_TITLE.0 + CAL_TITLE.2 / 2.0) * s, (CAL_TITLE.1 + 13.0) * s, 12.5 * s, white, CAL_TITLE.2 * s);
        for (i, head) in calendar::WEEK_HEADS.iter().enumerate() {
            let x = CAL_GRID.0 + (i as f32 + 0.5) * CAL_CELL.0;
            let ink = if i >= 5 { gfx::mix(grey, violet, 0.45) } else { [130, 130, 142, a] };
            draw_text_centered(pm, font, head, x * s, CAL_HEADS_Y * s, 9.0 * s, ink, CAL_CELL.0 * s);
        }

        // Month grid: the selected day is outlined, today is filled, and a dot marks days with tasks.
        for i in 0..calendar::CELLS {
            let Some(day) = self.cal.cell(i) else { continue };
            let rect = cal_cell_rect(i);
            let (x, y, w, h) = rect;
            let is_today = day == today;
            if day == selected {
                gfx::gradient_rounded(pm, (x + 1.0) * s, y * s, (w - 2.0) * s, h * s, 6.0 * s, [255, 255, 255, (alpha * 44.0) as u8], [255, 255, 255, (alpha * 22.0) as u8], false);
                gfx::stroke_rounded(pm, (x + 1.0) * s + 0.5 * s, y * s + 0.5 * s, (w - 2.0) * s - s, h * s - s, 6.0 * s, s, [255, 255, 255, (alpha * 90.0) as u8]);
            } else {
                let glow = self.hot_t(rect, s);
                if glow > 0.01 {
                    fill_rounded(pm, (x + 1.0) * s, y * s, (w - 2.0) * s, h * s, 6.0 * s, [255, 255, 255, (alpha * 16.0 * glow) as u8]);
                }
            }
            if is_today {
                gfx::glow(pm, (x + w / 2.0) * s, (y + h / 2.0) * s, 16.0 * s, [rgb[0], rgb[1], rgb[2], (alpha * 70.0) as u8], None);
                gfx::gradient_rounded(pm, (x + 3.0) * s, (y + 1.0) * s, (w - 6.0) * s, (h - 2.0) * s, 5.0 * s, gfx::mix(violet, white, 0.25), violet, false);
            }
            let (_, _, dom) = calendar::civil_from_days(day);
            let weekend = i % 7 >= 5;
            let ink = if is_today { white } else if weekend { [190, 175, 225, a] } else { white };
            draw_text_centered(pm, if is_today { heading } else { font }, &dom.to_string(), (x + w / 2.0) * s, (y + 11.5) * s, 10.0 * s, ink, w * s);
            let (open, total) = self.todo.day_counts(day);
            if total > 0 {
                let dot = if open > 0 { amber } else { [110, 110, 120, a] };
                fill_circle(pm, (x + w / 2.0) * s, (y + 14.3) * s, 1.2 * s, if is_today { white } else { dot });
            }
        }

        // The selected day sits in a card: its heading and its tasks.
        draw_card(pm, (CAL_PANEL_X - 8.0, 34.0, CAL_PANEL_W + 16.0, 126.0), 12.0, s, alpha, 0.0, rgb);
        draw_text(pm, heading, &calendar::long_date(selected), CAL_PANEL_X * s, 48.0 * s, 13.0 * s, white, 120.0 * s);
        let ids = self.todo.on_day(selected);
        let note = if ids.len() > CAL_VISIBLE {
            format!("+{} more", ids.len() - CAL_VISIBLE)
        } else if selected == today {
            "Today".to_string()
        } else {
            String::new()
        };
        if !note.is_empty() {
            let nw = text_width(font, &note, 10.0 * s) / s + 12.0;
            let nx = CAL_PANEL_X + CAL_PANEL_W - nw;
            fill_rounded(pm, nx * s, 38.0 * s, nw * s, 15.0 * s, 7.5 * s, [rgb[0], rgb[1], rgb[2], (alpha * 44.0) as u8]);
            draw_text_centered(pm, font, &note, (nx + nw / 2.0) * s, 49.0 * s, 10.0 * s, gfx::mix(violet, white, 0.5), nw * s);
        }
        if ids.is_empty() {
            draw_tab_icon(pm, View::Calendar, (CAL_PANEL_X + CAL_PANEL_W / 2.0) * s, 88.0 * s, s * 1.6, [rgb[0], rgb[1], rgb[2], (alpha * 150.0) as u8]);
            draw_text_centered(pm, font, "No tasks", (CAL_PANEL_X + CAL_PANEL_W / 2.0) * s, 114.0 * s, 12.0 * s, grey, CAL_PANEL_W * s);
        }
        for (row, &i) in ids.iter().take(CAL_VISIBLE).enumerate() {
            let task = &self.todo.tasks[i];
            let y = CAL_ROWS_Y + row as f32 * CAL_ROW_H;
            let rect = (CAL_PANEL_X - 4.0, y + 1.0, CAL_PANEL_W + 8.0, CAL_ROW_H - 2.0);
            let hot = self.hot_t(rect, s);
            if hot > 0.01 {
                fill_rounded(pm, rect.0 * s, rect.1 * s, rect.2 * s, rect.3 * s, 6.0 * s, [255, 255, 255, (alpha * 30.0 * hot) as u8]);
            }
            draw_check(pm, CAL_PANEL_X * s, (y + 4.0) * s, 12.0 * s, task.done, amber, a);
            let color = if task.done { [120, 120, 130, a] } else { white };
            // The time of a reminder, so the text only gets the room that is left.
            let time = task.due.map(|due| split_clock(due, now, offset, self.settings.clock_24h, task.done));
            let label_w = time.as_ref().map_or(0.0, |(label, _)| text_width(font, label, 9.0 * s));
            let text_w = (128.0 * s - label_w - if label_w > 0.0 { 6.0 * s } else { 0.0 }).max(30.0 * s);
            draw_text(pm, font, &task.text, (CAL_PANEL_X + 18.0) * s, (y + 14.0) * s, 11.0 * s, color, text_w);
            if task.done {
                let w = text_width(font, &task.text, 11.0 * s).min(text_w);
                fill_rect(pm, (CAL_PANEL_X + 18.0) * s, (y + 10.5) * s, w, 1.2 * s, color);
            }
            if let Some((label, overdue)) = &time {
                let c = if *overdue { [255, 130, 120, a] } else { [255, 214, 140, a] };
                draw_text(pm, font, label, (CAL_PANEL_X + CAL_PANEL_W - 32.0) * s - label_w, (y + 14.0) * s, 9.0 * s, c, 80.0 * s);
            }
            let over_remove = self.hot_t((CAL_PANEL_X + CAL_PANEL_W - 22.0, y, 30.0, CAL_ROW_H), s);
            let over_edit = (hot - over_remove).max(0.0);
            draw_pencil(pm, (CAL_PANEL_X + CAL_PANEL_W - 24.0) * s, (y + 10.0) * s, s * 0.85, pick(white, [120, 120, 130, a], over_edit));
            draw_text_centered(pm, font, "\u{00D7}", (CAL_PANEL_X + CAL_PANEL_W - 6.0) * s, (y + 15.0) * s, 13.0 * s, pick([255, 120, 110, a], grey, over_remove), 14.0 * s);
        }

        // Add a task for this day.
        let add = CAL_ADD;
        let hot = self.hot_t(add, s);
        let (x, y, w, h) = add;
        gfx::gradient_rounded(pm, x * s, y * s, w * s, h * s, h * s / 2.0, [rgb[0], rgb[1], rgb[2], (alpha * pick(90.0, 56.0, hot)) as u8], [rgb[0], rgb[1], rgb[2], (alpha * pick(56.0, 28.0, hot)) as u8], false);
        gfx::stroke_rounded(pm, x * s + 0.5 * s, y * s + 0.5 * s, w * s - s, h * s - s, h * s / 2.0, s, [rgb[0], rgb[1], rgb[2], (alpha * 100.0) as u8]);
        draw_text_centered(pm, font, "+  Add task", (x + w / 2.0) * s, (y + 13.0) * s, 11.0 * s, white, (w - 16.0) * s);
    }

    /// The task editor: name, day, reminder time and quick choices.
    fn draw_editor(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let Some(font) = &self.font else { return };
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [165, 165, 175, a]);
        let rgb = TODO_AMBER;
        let amber = [rgb[0], rgb[1], rgb[2], a];
        let (clock, ed, clock_24h) = (clock_now(), &self.editor, self.settings.clock_24h);
        // A pill that is lit when chosen, and brighter under the mouse.
        let pill = |pm: &mut Pixmap, action: editor::Action, on: bool| {
            let r = editor::rect(action);
            let (x, y, w, h) = (r.0 * s, r.1 * s, r.2 * s, r.3 * s);
            if on {
                gfx::gradient_rounded(pm, x, y, w, h, h / 2.0, [rgb[0], rgb[1], rgb[2], (alpha * 80.0) as u8], [rgb[0], rgb[1], rgb[2], (alpha * 38.0) as u8], false);
                gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [rgb[0], rgb[1], rgb[2], (alpha * 110.0) as u8]);
            } else {
                let boost = pick(14.0, 0.0, self.hot_t(r, s));
                fill_rounded(pm, x, y, w, h, h / 2.0, [255, 255, 255, (alpha * (16.0 + boost)) as u8]);
            }
        };
        let label = |pm: &mut Pixmap, action: editor::Action, text: &str, color: [u8; 4]| {
            let r = editor::rect(action);
            draw_text_centered(pm, font, text, (r.0 + r.2 / 2.0) * s, (r.1 + 16.0) * s, 11.0 * s, color, r.2 * s);
        };

        // The task's name, in a field that glows while you type.
        let (fx, fy, fw, fh) = editor::FIELD;
        gfx::glow(pm, (fx + fw / 2.0) * s, (fy + fh / 2.0) * s, 150.0 * s, [rgb[0], rgb[1], rgb[2], (alpha * 18.0) as u8], None);
        gfx::gradient_rounded(pm, fx * s, fy * s, fw * s, fh * s, 10.0 * s, [255, 255, 255, (alpha * 36.0) as u8], [255, 255, 255, (alpha * 20.0) as u8], false);
        gfx::stroke_rounded(pm, fx * s + 0.5 * s, fy * s + 0.5 * s, fw * s - s, fh * s - s, 10.0 * s, s, [rgb[0], rgb[1], rgb[2], (alpha * 140.0) as u8]);
        let (tx, ty, max_w) = ((fx + 12.0) * s, (fy + 17.0) * s, (fw - 24.0) * s);
        let shown = tail_fit(font, &self.input, 13.0 * s, max_w - 8.0 * s);
        if shown.is_empty() {
            draw_text(pm, font, "What needs doing?", tx, ty, 13.0 * s, [135, 135, 148, a], max_w);
        } else {
            draw_text(pm, font, &shown, tx, ty, 13.0 * s, white, max_w);
        }
        if (self.edit_started.elapsed().as_millis() / 500).is_multiple_of(2) {
            let x = tx + text_width(font, &shown, 13.0 * s) + 1.0 * s;
            fill_rect(pm, x, (fy + 5.0) * s, 1.5 * s, 16.0 * s, amber);
        }

        // The day: arrows around the date, then quick choices.
        draw_text(pm, font, "Date", 24.0 * s, (editor::DATE_Y + 16.0) * s, 11.0 * s, grey, 50.0 * s);
        for (action, dir) in [(editor::Action::DatePrev, -1.0), (editor::Action::DateNext, 1.0)] {
            let r = editor::rect(action);
            let (cx, cy) = ((r.0 + r.2 / 2.0) * s, (r.1 + r.3 / 2.0) * s);
            fill_circle(pm, cx, cy, 9.0 * s, [255, 255, 255, (alpha * pick(34.0, 16.0, self.hot_t(r, s))) as u8]);
            let tip = [(cx - 2.0 * dir * s, cy - 4.0 * s), (cx + 2.0 * dir * s, cy), (cx - 2.0 * dir * s, cy + 4.0 * s)];
            stroke_poly(pm, &tip, 1.6 * s, white, false);
        }
        let dt = editor::DATE_TEXT;
        let (text, color) = match ed.date {
            Some(day) => (calendar::long_date(day), if day == clock.today { amber } else { white }),
            None => ("No date".to_string(), grey),
        };
        draw_text_centered(pm, self.heading_font().unwrap_or(font), &text, (dt.0 + dt.2 / 2.0) * s, (dt.1 + 16.0) * s, 12.0 * s, color, dt.2 * s);
        for (action, name, on) in [
            (editor::Action::DateNone, "None", ed.date.is_none()),
            (editor::Action::DateToday, "Today", ed.date == Some(clock.today)),
            (editor::Action::DateTomorrow, "Tomorrow", ed.date == Some(clock.today + 1)),
        ] {
            pill(pm, action, on);
            label(pm, action, name, if on { white } else { grey });
        }

        // The reminder: a switch, then hour and minute steppers.
        draw_text(pm, font, "Remind", 24.0 * s, (editor::REMIND_Y + 16.0) * s, 11.0 * s, grey, 54.0 * s);
        draw_switch(pm, 82.0 * s, (editor::REMIND_Y + 3.0) * s, s, self.toggle_t(100, ed.remind), a);
        if ed.remind {
            for (action, plus) in [
                (editor::Action::HourDown, false),
                (editor::Action::HourUp, true),
                (editor::Action::MinuteDown, false),
                (editor::Action::MinuteUp, true),
            ] {
                let r = editor::rect(action);
                draw_step(pm, r, plus, s, alpha, self.hot_t(r, s));
            }
            let (hour, minute) = (ed.minutes / 60, ed.minutes % 60);
            let hour_text = if clock_24h { format!("{hour:02}") } else { format!("{}", if hour % 12 == 0 { 12 } else { hour % 12 }) };
            let clock_font = self.heading_font().unwrap_or(font);
            for (r, text) in [(editor::HOUR_TEXT, hour_text), (editor::MINUTE_TEXT, format!("{minute:02}"))] {
                draw_text_centered(pm, clock_font, &text, (r.0 + r.2 / 2.0) * s, (r.1 + 17.0) * s, 15.0 * s, white, r.2 * s);
            }
            draw_text_centered(pm, font, ":", 194.0 * s, (editor::REMIND_Y + 16.0) * s, 15.0 * s, grey, 8.0 * s);
            if !clock_24h {
                pill(pm, editor::Action::Meridiem, true);
                label(pm, editor::Action::Meridiem, if hour < 12 { "AM" } else { "PM" }, white);
            }
            let (status, past) = ed.status(clock);
            let c = if past { [255, 130, 120, a] } else { [255, 214, 140, a] };
            draw_text(pm, font, &status, 396.0 * s - text_width(font, &status, 10.0 * s), (editor::REMIND_Y + 16.0) * s, 10.0 * s, c, 90.0 * s);
        } else {
            draw_text(pm, font, "No reminder", 126.0 * s, (editor::REMIND_Y + 16.0) * s, 11.0 * s, [135, 135, 148, a], 120.0 * s);
        }

        // Quick choices, then cancel and save.
        for (action, name) in [
            (editor::Action::In(15), "In 15m"),
            (editor::Action::In(60), "In 1h"),
            (editor::Action::In(180), "In 3h"),
            (editor::Action::TomorrowMorning, "Tomorrow 9:00"),
            (editor::Action::Cancel, "Cancel"),
        ] {
            pill(pm, action, false);
            label(pm, action, name, grey);
        }
        let r = editor::rect(editor::Action::Save);
        let hot = self.hot_t(r, s);
        gfx::glow(pm, (r.0 + r.2 / 2.0) * s, (r.1 + r.3 / 2.0) * s, 46.0 * s, [rgb[0], rgb[1], rgb[2], (alpha * pick(80.0, 46.0, hot)) as u8], None);
        gfx::gradient_rounded(pm, r.0 * s, r.1 * s, r.2 * s, r.3 * s, r.3 * s / 2.0, gfx::mix(amber, white, pick(0.4, 0.25, hot)), amber, false);
        label(pm, editor::Action::Save, if ed.index.is_some() { "Save" } else { "Add" }, [20, 14, 0, a]);
    }

    fn draw_clipboard(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let Some(font) = &self.font else { return };
        let heading = self.heading_font().unwrap_or(font);
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [165, 165, 175, a]);
        let rgb = CLIP_TEAL;
        let teal = [rgb[0], rgb[1], rgb[2], a];
        let width = EXPANDED.0 as f32 * s;

        // Header: what the list holds, or a short message after copying or clearing.
        let count = self.clips.items.len();
        let (title, color) = match self.clip_flash {
            Some((msg, _)) => (msg.to_string(), teal),
            None if !self.settings.clipboard => ("Clipboard history is off".to_string(), grey),
            None => (format!("Clipboard \u{00B7} {count} {}", if count == 1 { "item" } else { "items" }), gfx::mix(grey, white, 0.4)),
        };
        draw_text(pm, heading, &title, 24.0 * s, 51.0 * s, 12.5 * s, color, 280.0 * s);

        // Clear button: empties the history and the Windows clipboard.
        let clear = CLIP_CLEAR;
        let (x, y, w, h) = clear;
        let hot = self.hot_t(clear, s);
        let red = [255, 96, 86];
        gfx::gradient_rounded(pm, x * s, y * s, w * s, h * s, h * s / 2.0, [red[0], red[1], red[2], (alpha * pick(80.0, 48.0, hot)) as u8], [red[0], red[1], red[2], (alpha * pick(46.0, 22.0, hot)) as u8], false);
        gfx::stroke_rounded(pm, x * s + 0.5 * s, y * s + 0.5 * s, w * s - s, h * s - s, h * s / 2.0, s, [red[0], red[1], red[2], (alpha * 90.0) as u8]);
        draw_text_centered(pm, font, "Clear", (x + w / 2.0) * s, (y + 15.0) * s, 12.0 * s, [255, 130, 120, a], w * s);

        if !self.settings.clipboard || count == 0 {
            let (title, hint) = if self.settings.clipboard {
                ("Nothing copied yet", "Text and pictures you copy will show up here")
            } else {
                ("Clipboard history is off", "Turn it on in Settings")
            };
            gfx::glow(pm, width / 2.0, 100.0 * s, 70.0 * s, [rgb[0], rgb[1], rgb[2], (alpha * 34.0) as u8], None);
            draw_tab_icon(pm, View::Clipboard, width / 2.0, 92.0 * s, s * 2.4, teal);
            draw_text_centered(pm, heading, title, width / 2.0, 128.0 * s, 15.0 * s, white, width - 80.0 * s);
            draw_text_centered(pm, font, hint, width / 2.0, 147.0 * s, 11.0 * s, grey, width - 80.0 * s);
            return;
        }

        for row in 0..clipboard::VISIBLE {
            let Some(i) = self.clips.index_at(row) else { continue };
            let y = CLIP_ROWS_Y + row as f32 * CLIP_ROW_H;
            let rect = (24.0, y + 1.0, 372.0, CLIP_ROW_H - 2.0);
            let hot = self.hot_t(rect, s);
            draw_card(pm, rect, 8.0, s, alpha, hot, rgb);
            // A teal tab on the left edge marks the row under the mouse.
            if hot > 0.01 {
                let tab = [teal[0], teal[1], teal[2], (teal[3] as f32 * hot) as u8];
                gfx::gradient_rounded(pm, 24.0 * s, (y + 8.0) * s, 3.0 * s, (CLIP_ROW_H - 16.0) * s, 1.5 * s, tab, [rgb[0], rgb[1], rgb[2], 0], false);
            }
            match &self.clips.items[i] {
                clipboard::Content::Text(text) => {
                    let baseline = (y + CLIP_ROW_H / 2.0 + 4.0) * s;
                    draw_text(pm, font, &clipboard::preview(text), 34.0 * s, baseline, 12.0 * s, white, 326.0 * s);
                }
                clipboard::Content::Image(img) => {
                    // The preview is stored at twice its drawn size, so it stays sharp on high-DPI screens.
                    if let Some(mut thumb) = Pixmap::new(img.thumb_w, img.thumb_h) {
                        for (dst, px) in thumb.pixels_mut().iter_mut().zip(img.thumb.as_chunks::<4>().0) {
                            *dst = tiny_skia::ColorU8::from_rgba(px[0], px[1], px[2], px[3]).premultiply();
                        }
                        let (dw, dh) = (img.thumb_w as f32 / 2.0, img.thumb_h as f32 / 2.0);
                        let (tx, ty) = (34.0 + (48.0 - dw) / 2.0, y + (CLIP_ROW_H - dh) / 2.0);
                        let paint = PixmapPaint { opacity: alpha, quality: FilterQuality::Bilinear, ..Default::default() };
                        let at = Transform::from_scale(s / 2.0, s / 2.0).post_translate(tx * s, ty * s);
                        let mask = gfx::rounded_path(tx * s, ty * s, dw * s, dh * s, 3.0 * s).and_then(|shape| gfx::mask_of(pm.width(), pm.height(), &shape));
                        pm.draw_pixmap(0, 0, thumb.as_ref(), &paint, at, mask.as_ref());
                        gfx::stroke_rounded(pm, tx * s, ty * s, dw * s, dh * s, 3.0 * s, s, [255, 255, 255, (alpha * 40.0) as u8]);
                    }
                    draw_text(pm, font, "Image", 94.0 * s, (y + 14.0) * s, 12.0 * s, white, 200.0 * s);
                    let size = format!("{} \u{00D7} {}", img.width, img.height);
                    draw_text(pm, font, &size, 94.0 * s, (y + 27.0) * s, 10.0 * s, grey, 200.0 * s);
                }
            }
            let over_remove = self.hot_t((372.0, y, 24.0, CLIP_ROW_H), s);
            draw_text_centered(pm, font, "\u{00D7}", 384.0 * s, (y + CLIP_ROW_H / 2.0 + 5.0) * s, 13.0 * s, pick([255, 120, 110, a], grey, over_remove), 14.0 * s);
        }

        // Scroll indicator when there are more entries than rows.
        if count > clipboard::VISIBLE {
            let (top, height) = (CLIP_ROWS_Y, CLIP_ROW_H * clipboard::VISIBLE as f32);
            fill_rounded(pm, 410.0 * s, top * s, 3.0 * s, height * s, 1.5 * s, [255, 255, 255, (alpha * 22.0) as u8]);
            let thumb = height * clipboard::VISIBLE as f32 / count as f32;
            let at = top + (height - thumb) * self.clips.scroll as f32 / (count - clipboard::VISIBLE) as f32;
            fill_rounded(pm, 410.0 * s, at * s, 3.0 * s, thumb * s, 1.5 * s, teal);
        }
    }

    fn draw_player(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let a = (alpha * 255.0) as u8;
        let rgb = self.accent(View::Player);
        let accent = [rgb[0], rgb[1], rgb[2], a];
        let white = [255, 255, 255, a];
        let grey = [165, 165, 175, a];
        let open = self.view_t.clamp(0.0, 1.0) as f32;
        let geo = self.geo();

        let (title, sub) = if self.media.has_session && !self.media.title.is_empty() {
            let sub = if self.media.album.is_empty() {
                self.media.artist.clone()
            } else {
                format!("{} \u{b7} {}", self.media.artist, self.media.album)
            };
            (self.media.title.as_str(), sub)
        } else {
            ("Nothing playing", "Play something to see it here".to_string())
        };

        // The lyrics fade in on the right as the notch widens and the player moves into its column.
        // They wait until the player has moved out of the way, then fade in and slide into place.
        let reveal = ((open - 0.45) / 0.55).clamp(0.0, 1.0);
        if reveal > 0.0 {
            self.draw_lyric_panel(pm, s, alpha * reveal, (1.0 - reveal) * 28.0);
        }

        self.draw_art(pm, geo.art, s, alpha);

        let (bx, by, bw) = geo.bar;
        if let Some(font) = &self.font {
            let heading = self.heading_font().unwrap_or(font);
            let max_w = (geo.right - geo.text_x) * s;
            draw_text(pm, heading, title, geo.text_x * s, 56.0 * s, 17.0 * s, white, max_w);
            draw_text(pm, font, &sub, geo.text_x * s, 78.0 * s, 13.0 * s, gfx::mix(grey, accent, 0.4), max_w);

            // Time played on the left of the bar, time left on the right.
            let pos = fmt_time(self.position());
            let left = format!("-{}", fmt_time(self.media.duration.saturating_sub(self.position())));
            let base = (by + 5.5) * s;
            draw_text(pm, font, &pos, geo.left * s, base, 10.0 * s, grey, 42.0 * s);
            let left_w = text_width(font, &left, 10.0 * s);
            draw_text(pm, font, &left, geo.right * s - left_w, base, 10.0 * s, grey, 46.0 * s);
        }

        // Progress bar: a rounded track, a filled part shaded with the cover's colour, and a knob.
        let grab = self.hot((bx - 4.0, by - 9.0, bw + 8.0, 21.0), s) || self.scrub.is_some();
        let grow = self.glows.follow(KEY_BAR, if grab { 1.0 } else { 0.0 }, 30.0, 0.8).clamp(0.0, 1.0) as f32;
        let bh = 5.0 + 2.0 * grow;
        let ty = by + 2.0 - bh / 2.0;
        fill_rounded(pm, bx * s, ty * s, bw * s, bh * s, bh * s / 2.0, [255, 255, 255, (alpha * 30.0) as u8]);
        let frac = if self.media.duration.is_zero() {
            0.0
        } else {
            (self.position().as_secs_f32() / self.media.duration.as_secs_f32()).clamp(0.0, 1.0)
        };
        if frac > 0.0 {
            let fill_w = (bw * frac).max(bh);
            gfx::gradient_rounded(pm, bx * s, ty * s, fill_w * s, bh * s, bh * s / 2.0, gfx::mix(accent, white, 0.15), gfx::mix(accent, white, 0.7), true);
            let knob_x = (bx + fill_w - bh / 2.0) * s;
            gfx::glow(pm, knob_x, (by + 2.0) * s, (12.0 + 5.0 * grow) * s, [rgb[0], rgb[1], rgb[2], (alpha * (90.0 + 50.0 * grow)) as u8], None);
            fill_circle(pm, knob_x, (by + 2.0) * s, (4.0 + 2.0 * grow) * s, white);
        }

        // Transport controls: the lyrics button, then previous, a glowing round play button, and next.
        let py = CTRL_Y * s;
        let [lyrics_x, prev_x, play_x, next_x] = geo.ctrl_x;
        for (i, x) in [lyrics_x, prev_x, next_x].into_iter().enumerate() {
            let enabled = i != 2 || self.media.next_enabled;
            let glow = if enabled { self.hot_t((x - 18.0, CTRL_Y - 18.0, 36.0, 36.0), s) } else { 0.0 };
            if glow > 0.01 {
                fill_circle(pm, x * s, py, 18.0 * s, [255, 255, 255, (alpha * 24.0 * glow) as u8]);
            }
        }
        // The lyrics button is lit while the lyrics are open.
        if open > 0.01 {
            fill_circle(pm, lyrics_x * s, py, 16.0 * s, [rgb[0], rgb[1], rgb[2], (alpha * open * 70.0) as u8]);
        }
        draw_lyrics_icon(pm, lyrics_x * s, py, s, gfx::mix(grey, white, open));
        let dim = [90, 90, 98, a];
        draw_prev(pm, prev_x * s, py, s, white);
        draw_next(pm, next_x * s, py, s, if self.media.next_enabled { white } else { dim });
        let play = self.hot_t((play_x - 22.0, CTRL_Y - 22.0, 44.0, 44.0), s);
        gfx::glow(pm, play_x * s, py, 34.0 * s, [rgb[0], rgb[1], rgb[2], (alpha * 56.0) as u8], None);
        let radius = (pick(21.0, 19.5, play)) * s;
        gfx::gradient_circle(pm, play_x * s, py, radius, white, [214, 214, 226, a]);
        let pause = self.glows.follow(KEY_PLAYER_PLAY, if self.media.playing { 1.0 } else { 0.0 }, 30.0, 0.85) as f32;
        draw_play_pause(pm, play_x * s, py, s * 0.82, [12, 12, 16, a], pause);
    }
}

fn load_font() -> Option<Font> {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    for name in ["segoeui.ttf", "arial.ttf"] {
        if let Ok(bytes) = std::fs::read(format!("{windir}\\Fonts\\{name}"))
            && let Ok(font) = Font::from_bytes(bytes, FontSettings::default()) {
                return Some(font);
            }
    }
    None
}

fn load_bold_font() -> Option<Font> {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    for name in ["seguisb.ttf", "segoeuib.ttf"] {
        if let Ok(bytes) = std::fs::read(format!("{windir}\\Fonts\\{name}"))
            && let Ok(font) = Font::from_bytes(bytes, FontSettings::default()) {
                return Some(font);
            }
    }
    None
}

/// A bright, readable colour taken from cover art (RGBA pixels): the average of its colourful
/// pixels, or of all of them when it has none.
fn art_tint(rgba: &[u8]) -> [u8; 3] {
    let (mut vivid, mut vivid_n) = ([0u64; 3], 0u64);
    let (mut all, mut all_n) = ([0u64; 3], 0u64);
    for px in rgba.as_chunks::<4>().0.iter().step_by(7) {
        let c = [px[0] as u64, px[1] as u64, px[2] as u64];
        let (hi, lo) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
        for i in 0..3 {
            all[i] += c[i];
        }
        all_n += 1;
        if hi > 70 && (hi - lo) * 100 / hi > 30 {
            for i in 0..3 {
                vivid[i] += c[i];
            }
            vivid_n += 1;
        }
    }
    let (sum, n) = if vivid_n > 8 { (vivid, vivid_n) } else { (all, all_n.max(1)) };
    let avg = [sum[0] as f32 / n as f32, sum[1] as f32 / n as f32, sum[2] as f32 / n as f32];
    // Lift it so that even a dark cover gives a visible accent.
    let peak = avg[0].max(avg[1]).max(avg[2]).max(1.0);
    let k = (235.0 / peak).min(2.4);
    let lifted = [(avg[0] * k).min(255.0) as u8, (avg[1] * k).min(255.0) as u8, (avg[2] * k).min(255.0) as u8];
    // A dark or grey cover has no colour to borrow, so lean towards a soft silver.
    let brightest = lifted[0].max(lifted[1]).max(lifted[2]);
    if brightest >= 170 {
        return lifted;
    }
    let t = (170 - brightest) as f32 / 170.0;
    let mix = |c: u8, silver: u8| (c as f32 + (silver as f32 - c as f32) * t) as u8;
    [mix(lifted[0], 200), mix(lifted[1], 200), mix(lifted[2], 212)]
}

fn text_width(font: &Font, text: &str, px: f32) -> f32 {
    text.chars().map(|c| font.metrics(c, px).advance_width).sum()
}

/// Draw `text` with its baseline at `y`, truncating with an ellipsis past `max_w`.
fn draw_text(pm: &mut Pixmap, font: &Font, text: &str, x: f32, y: f32, px: f32, rgba: [u8; 4], max_w: f32) {
    let rgba = theme::map(rgba);
    let mut s = text.to_string();
    if text_width(font, &s, px) > max_w {
        while !s.is_empty() && text_width(font, &format!("{s}…"), px) > max_w {
            s.pop();
        }
        s.push('…');
    }

    let (pw, ph) = (pm.width() as i32, pm.height() as i32);
    let data = pm.pixels_mut();
    let mut pen = x;
    for c in s.chars() {
        let (m, bitmap) = font.rasterize(c, px);
        let gx = (pen + m.xmin as f32).round() as i32;
        let gy = (y - m.height as f32 - m.ymin as f32).round() as i32;
        for row in 0..m.height {
            for col in 0..m.width {
                let cov = bitmap[row * m.width + col] as u32;
                if cov == 0 {
                    continue;
                }
                let (dx, dy) = (gx + col as i32, gy + row as i32);
                if dx < 0 || dy < 0 || dx >= pw || dy >= ph {
                    continue;
                }
                let sa = cov * rgba[3] as u32 / 255;
                let dst = &mut data[(dy * pw + dx) as usize];
                let inv = 255 - sa;
                let blend = |src: u8, d: u8| ((src as u32 * sa + d as u32 * inv) / 255) as u8;
                *dst = tiny_skia::PremultipliedColorU8::from_rgba(
                    blend(rgba[0], dst.red()),
                    blend(rgba[1], dst.green()),
                    blend(rgba[2], dst.blue()),
                    (sa + dst.alpha() as u32 * inv / 255).min(255) as u8,
                )
                .unwrap_or(*dst);
            }
        }
        pen += m.advance_width;
    }
}

/// Greedy word wrap. An empty line (instrumental gap) becomes three dots (the system font has no music note).
fn wrap(font: &Font, text: &str, px: f32, max_w: f32) -> Vec<String> {
    let mut rows = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let candidate = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
        if cur.is_empty() || text_width(font, &candidate, px) <= max_w {
            cur = candidate;
        } else {
            rows.push(std::mem::take(&mut cur));
            cur = word.to_string();
        }
    }
    if !cur.is_empty() {
        rows.push(cur);
    }
    if rows.is_empty() {
        rows.push("\u{00B7} \u{00B7} \u{00B7}".to_string());
    }
    rows
}

fn draw_text_centered(pm: &mut Pixmap, font: &Font, text: &str, cx: f32, y: f32, px: f32, rgba: [u8; 4], max_w: f32) {
    let w = text_width(font, text, px).min(max_w);
    draw_text(pm, font, text, cx - w / 2.0, y, px, rgba, max_w);
}

/// The area of one timer number that can be clicked to type a value.
fn timer_value_rect(i: usize) -> (f32, f32, f32, f32) {
    (TIMER_COLS[i] - 21.0, TIMER_VALUE_Y - 17.0, 40.0, 24.0)
}

/// Preset chip, centered over the right-hand column of the timer view.
fn timer_chip_rect(i: usize) -> (f32, f32, f32, f32) {
    let n = timer::PRESETS.len() as f32;
    let total = n * TIMER_CHIP_W + (n - 1.0) * CHIP_GAP;
    (TIMER_COLS[1] - total / 2.0 + i as f32 * (TIMER_CHIP_W + CHIP_GAP), 34.0, TIMER_CHIP_W, 18.0)
}

/// "5m", "30m" or "1h" for a preset length.
fn preset_label(secs: u64) -> String {
    if secs.is_multiple_of(3600) { format!("{}h", secs / 3600) } else { format!("{}m", secs / 60) }
}

/// Reminder interval chip, centered over the right-hand column of the water view.
fn water_chip_rect(i: usize) -> (f32, f32, f32, f32) {
    let n = water::INTERVALS.len() as f32;
    let total = n * WATER_CHIP_W + (n - 1.0) * CHIP_GAP;
    (WATER_X[1] - total / 2.0 + i as f32 * (WATER_CHIP_W + CHIP_GAP), 34.0, WATER_CHIP_W, 18.0)
}

/// Today's local date as yyyymmdd.
fn today() -> u32 {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe { GetLocalTime() };
    t.wYear as u32 * 10000 + t.wMonth as u32 * 100 + t.wDay as u32
}

/// Today's day number, for the calendar.
fn today_day() -> i64 {
    let t = today();
    calendar::days_from_civil((t / 10_000) as i32, t / 100 % 100, t % 100)
}

fn island_label(auto_hide: bool) -> &'static str {
    if auto_hide { "Always Show Island" } else { "Auto-hide Island" }
}

// Calendar tab layout, in logical pixels: month grid on the left, the chosen day's tasks on the right.
// The month heading stays at the top; the weekday letters and the dates sit lower, with a gap between.
const CAL_GRID: (f32, f32) = (24.0, 65.0);
const CAL_CELL: (f32, f32) = (24.0, 16.0);
const CAL_HEADS_Y: f32 = 62.0; // baseline of the weekday letters
const CAL_PREV: (f32, f32, f32, f32) = (24.0, 30.0, 24.0, 16.0);
const CAL_NEXT: (f32, f32, f32, f32) = (168.0, 30.0, 24.0, 16.0);
const CAL_TITLE: (f32, f32, f32, f32) = (52.0, 30.0, 112.0, 16.0);
/// The day panel on the right, from this x to `CAL_PANEL_X + CAL_PANEL_W`.
const CAL_PANEL_X: f32 = 216.0;
const CAL_PANEL_W: f32 = 180.0;
const CAL_ROWS_Y: f32 = 60.0;
const CAL_ROW_H: f32 = 20.0;
const CAL_VISIBLE: usize = 4;
const CAL_ADD: (f32, f32, f32, f32) = (CAL_PANEL_X, 142.0, CAL_PANEL_W, 18.0);

fn cal_cell_rect(i: usize) -> (f32, f32, f32, f32) {
    let (col, row) = ((i % 7) as f32, (i / 7) as f32);
    (CAL_GRID.0 + col * CAL_CELL.0, CAL_GRID.1 + row * CAL_CELL.1, CAL_CELL.0, CAL_CELL.1)
}

/// A checkbox of `size` physical pixels, ticked when the task is done.
/// A pill-shaped choice: tinted with `accent` when chosen, a soft highlight while the mouse is over it.
fn draw_chip(pm: &mut Pixmap, font: &Font, r: (f32, f32, f32, f32), label: &str, on: bool, hot: f32, accent: [u8; 3], s: f32, alpha: f32) {
    let (x, y, w, h) = (r.0 * s, r.1 * s, r.2 * s, r.3 * s);
    let a = (alpha * 255.0) as u8;
    if on {
        gfx::gradient_rounded(pm, x, y, w, h, h / 2.0, [accent[0], accent[1], accent[2], (alpha * 70.0) as u8], [accent[0], accent[1], accent[2], (alpha * 34.0) as u8], false);
        gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [accent[0], accent[1], accent[2], (alpha * 90.0) as u8]);
    } else if hot > 0.01 {
        fill_rounded(pm, x, y, w, h, h / 2.0, [255, 255, 255, (alpha * 16.0 * hot) as u8]);
    }
    let color = if on { [255, 255, 255, a] } else { pick([225, 225, 232, a], [150, 150, 160, a], hot) };
    draw_text_centered(pm, font, label, (r.0 + r.2 / 2.0) * s, (r.1 + 13.0) * s, 10.0 * s, color, r.2 * s);
}

/// A small round button, brighter under the mouse and dimmed when it cannot be used.
fn draw_round_button(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, hot: f32, enabled: bool, alpha: f32) {
    let base = if !enabled { 12.0 } else { pick(44.0, 26.0, hot) };
    fill_circle(pm, cx, cy, r, [255, 255, 255, (alpha * base) as u8]);
}

/// The big round start / add button: a gradient in the tab's colour with a soft glow.
fn draw_big_button(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, hot: f32, rgb: [u8; 3], alpha: f32) {
    let a = (alpha * 255.0) as u8;
    let accent = [rgb[0], rgb[1], rgb[2], a];
    gfx::glow(pm, cx, cy, r * 2.0, [rgb[0], rgb[1], rgb[2], (alpha * pick(90.0, 56.0, hot)) as u8], None);
    let r = pick(r + 1.2 * (r / 18.0), r, hot);
    gfx::gradient_circle(pm, cx, cy, r, gfx::mix(accent, [255, 255, 255, a], 0.3), accent);
}

/// Water inside a circle, filled to `frac` with two soft waves on top.
fn draw_water_fill(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, frac: f32, rgb: [u8; 3], s: f32, alpha: f32) {
    if frac <= 0.0 {
        return;
    }
    let Some(circle) = PathBuilder::from_circle(cx, cy, r) else { return };
    let Some(mask) = gfx::mask_of(pm.width(), pm.height(), &circle) else { return };
    let level = cy + r - 2.0 * r * frac.clamp(0.0, 1.0);
    for (phase, amp, strong, faint) in [(0.0f32, 2.2f32, 64.0f32, 24.0f32), (2.1, 1.6, 96.0, 44.0)] {
        let mut pb = PathBuilder::new();
        let steps = 28;
        for i in 0..=steps {
            let x = cx - r + 2.0 * r * i as f32 / steps as f32;
            let y = level + (x / s * 0.16 + phase + frac * 40.0).sin() * amp * s;
            if i == 0 {
                pb.move_to(x, y);
            } else {
                pb.line_to(x, y);
            }
        }
        pb.line_to(cx + r, cy + r + 4.0 * s);
        pb.line_to(cx - r, cy + r + 4.0 * s);
        pb.close();
        let Some(path) = pb.finish() else { continue };
        let stops = vec![
            tiny_skia::GradientStop::new(0.0, Color::from_rgba8(rgb[0], rgb[1], rgb[2], (alpha * strong) as u8)),
            tiny_skia::GradientStop::new(1.0, Color::from_rgba8(rgb[0], rgb[1], rgb[2], (alpha * faint) as u8)),
        ];
        let from = tiny_skia::Point::from_xy(cx, level);
        let to = tiny_skia::Point::from_xy(cx, cy + r);
        let Some(shader) = tiny_skia::LinearGradient::new(from, to, stops, tiny_skia::SpreadMode::Pad, Transform::identity()) else {
            continue;
        };
        let paint = Paint { shader, anti_alias: true, ..Default::default() };
        pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), Some(&mask));
    }
}

fn draw_check(pm: &mut Pixmap, x: f32, y: f32, size: f32, done: bool, amber: [u8; 4], a: u8) {
    let k = size / 14.0;
    if done {
        fill_rounded(pm, x, y, size, size, 4.0 * k, amber);
        stroke_line(pm, x + 3.5 * k, y + 7.5 * k, x + 6.0 * k, y + 10.0 * k, 1.8 * k, [0, 0, 0, a]);
        stroke_line(pm, x + 6.0 * k, y + 10.0 * k, x + 10.5 * k, y + 4.5 * k, 1.8 * k, [0, 0, 0, a]);
    } else {
        fill_rounded(pm, x, y, size, size, 4.0 * k, [110, 110, 116, a]);
        fill_rounded(pm, x + 1.5 * k, y + 1.5 * k, size - 3.0 * k, size - 3.0 * k, 3.0 * k, [0, 0, 0, 255]);
    }
}

fn tile_rect(slot: usize) -> (f32, f32, f32, f32) {
    let (col, row) = ((slot % COLS) as f32, (slot / COLS) as f32);
    (
        TILES_ORIGIN.0 + col * (TILE.0 + TILE_GAP.0),
        TILES_ORIGIN.1 + row * (TILE.1 + TILE_GAP.1),
        TILE.0,
        TILE.1,
    )
}

/// Keep the earliest of the wake-up times asked for.
fn earliest(wake: &mut Option<Instant>, t: Instant) {
    *wake = Some(wake.map_or(t, |w| w.min(t)));
}

/// Local time of day as (hour, minute, second, millisecond).
fn local_time() -> (u32, u32, u32, u32) {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe { GetLocalTime() };
    (t.wHour as u32, t.wMinute as u32, t.wSecond as u32, t.wMilliseconds as u32)
}

fn fmt_clock(hour: u32, minute: u32, clock_24h: bool) -> String {
    if clock_24h {
        format!("{hour:02}:{minute:02}")
    } else {
        format!("{}:{minute:02} {}", if hour.is_multiple_of(12) { 12 } else { hour % 12 }, if hour < 12 { "AM" } else { "PM" })
    }
}

/// A toggle switch, 32 by 18 logical pixels with its top-left corner at `x`, `y`.
/// `on` goes from 0 (off) to 1 (on) and may swing a little past 1: the knob slides, and the green
/// fades in as it goes.
fn draw_switch(pm: &mut Pixmap, x: f32, y: f32, s: f32, on: f32, a: u8) {
    let (w, h) = (32.0 * s, 18.0 * s);
    let alpha = a as f32 / 255.0;
    let t = on.clamp(0.0, 1.0);
    if t < 0.99 {
        fill_rounded(pm, x, y, w, h, h / 2.0, [255, 255, 255, (alpha * 34.0 * (1.0 - t)) as u8]);
        gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [255, 255, 255, (alpha * 30.0 * (1.0 - t)) as u8]);
    }
    if t > 0.01 {
        gfx::glow(pm, x + w / 2.0, y + h / 2.0, 24.0 * s, [70, 210, 120, (alpha * 46.0 * t) as u8], None);
        let fade = |c: u8| (c as f32 * t) as u8;
        gfx::gradient_rounded(pm, x, y, w, h, h / 2.0, [104, 228, 148, fade(a)], [50, 188, 102, fade(a)], false);
    }
    let knob_x = x + h / 2.0 + (w - h) * on;
    fill_circle(pm, knob_x, y + h / 2.0 + 0.8 * s, 7.0 * s, [0, 0, 0, (alpha * 60.0) as u8]);
    // White in both looks (the light look turns white into dark, so ask for black there).
    let knob = if theme::is_light() { [0, 0, 0, a] } else { [255, 255, 255, a] };
    fill_circle(pm, knob_x, y + h / 2.0, 7.0 * s, knob);
}

/// A rounded card: a soft gradient with a hairline border, outlined in the accent colour when `hot`.
fn draw_card(pm: &mut Pixmap, r: (f32, f32, f32, f32), radius: f32, s: f32, alpha: f32, hot: f32, accent: [u8; 3]) {
    let (x, y, w, h) = (r.0 * s, r.1 * s, r.2 * s, r.3 * s);
    let (top, bottom) = (pick(30.0, 20.0, hot), pick(16.0, 9.0, hot));
    gfx::gradient_rounded(pm, x, y, w, h, radius * s, [255, 255, 255, (alpha * top) as u8], [255, 255, 255, (alpha * bottom) as u8], false);
    let edge = pick([accent[0], accent[1], accent[2], (alpha * 110.0) as u8], [255, 255, 255, (alpha * 16.0) as u8], hot);
    gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, radius * s, s, edge);
}

/// Local time minus UTC in seconds, read from the system clock (rounded to a quarter hour).
fn tz_offset() -> i64 {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe { GetLocalTime() };
    let local = calendar::days_from_civil(t.wYear as i32, t.wMonth as u32, t.wDay as u32) * 86_400
        + i64::from(t.wHour) * 3600
        + i64::from(t.wMinute) * 60
        + i64::from(t.wSecond);
    ((local - todo::unix_now() as i64) as f64 / 900.0).round() as i64 * 900
}

/// Everything the task editor needs to know about the present moment.
fn clock_now() -> editor::Clock {
    let (hour, minute, _, _) = local_time();
    editor::Clock { today: today_day(), minutes: hour * 60 + minute, now: todo::unix_now(), offset: tz_offset() }
}

/// The time of a reminder as text, and whether it is overdue. Finished tasks are never overdue.
fn split_clock(due: u64, now: u64, offset: i64, clock_24h: bool, done: bool) -> (String, bool) {
    if !done && due <= now {
        return ("overdue".to_string(), true);
    }
    let (_, minutes) = todo::split_due(due, offset);
    (fmt_clock(minutes / 60, minutes % 60, clock_24h), false)
}

/// What to show at the right of a task: when it is due, or else its day.
fn task_when(task: &todo::Task, today: i64, now: u64, offset: i64, clock_24h: bool) -> Option<(String, bool)> {
    if let Some(due) = task.due {
        let (time, overdue) = split_clock(due, now, offset, clock_24h, task.done);
        if overdue {
            return Some((time, true));
        }
        let (day, _) = todo::split_due(due, offset);
        let label = match day - today {
            0 => time,
            1 => format!("Tomorrow {time}"),
            _ => format!("{} {time}", calendar::short_date(day)),
        };
        return Some((label, false));
    }
    task.date.map(|day| (calendar::short_date(day), false))
}

/// A round "minus" or "plus" button for the time steppers.
fn draw_step(pm: &mut Pixmap, r: editor::Rect, plus: bool, s: f32, alpha: f32, hot: f32) {
    let (cx, cy) = ((r.0 + r.2 / 2.0) * s, (r.1 + r.3 / 2.0) * s);
    fill_circle(pm, cx, cy, 8.5 * s, [255, 255, 255, (alpha * pick(40.0, 18.0, hot)) as u8]);
    let ink = [210, 210, 215, (alpha * 255.0) as u8];
    stroke_line(pm, cx - 3.0 * s, cy, cx + 3.0 * s, cy, 1.5 * s, ink);
    if plus {
        stroke_line(pm, cx, cy - 3.0 * s, cx, cy + 3.0 * s, 1.5 * s, ink);
    }
}

/// A small pencil, the edit button on a task.
fn draw_pencil(pm: &mut Pixmap, cx: f32, cy: f32, s: f32, c: [u8; 4]) {
    stroke_line(pm, cx - 2.2 * s, cy + 2.2 * s, cx + 3.0 * s, cy - 3.0 * s, 2.2 * s, c);
    fill_path(pm, &[(cx - 4.2 * s, cy + 4.2 * s), (cx - 4.2 * s, cy + 1.4 * s), (cx - 1.4 * s, cy + 4.2 * s)], c);
}

/// The end of `text` that fits in `max_w`, so the caret stays visible while typing a long task.
fn tail_fit(font: &Font, text: &str, px: f32, max_w: f32) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut start = 0;
    while start < chars.len() && text_width(font, &chars[start..].iter().collect::<String>(), px) > max_w {
        start += 1;
    }
    chars[start..].iter().collect()
}

/// Where the parts of the player sit. `t` is how far the lyrics have opened (0 closed, 1 open): the
/// player slides into a narrower column on the left to make room for them.
struct PlayerGeo {
    art: (f32, f32, f32), // x, y, size
    text_x: f32,
    left: f32,
    right: f32,
    bar: (f32, f32, f32), // x, y, width
    ctrl_x: [f32; 4],     // lyrics, previous, play/pause, next
}

fn player_geo(t: f32) -> PlayerGeo {
    let right = 396.0 + (264.0 - 396.0) * t;
    let mid = (24.0 + right) / 2.0;
    PlayerGeo {
        art: (24.0, 30.0, 64.0),
        text_x: 104.0,
        left: 24.0,
        right,
        bar: (64.0, 106.0, right - 104.0),
        ctrl_x: [mid - 100.0, mid - 50.0, mid, mid + 50.0],
    }
}

/// A speech bubble with two lines of text: the lyrics button.
fn draw_lyrics_icon(pm: &mut Pixmap, cx: f32, cy: f32, s: f32, c: [u8; 4]) {
    let p = |dx: f32, dy: f32| (cx + dx * s, cy + dy * s);
    stroke_poly(pm, &[p(-7.0, -6.0), p(7.0, -6.0), p(7.0, 3.5), p(1.5, 3.5), p(-2.5, 7.0), p(-2.5, 3.5), p(-7.0, 3.5)], 1.5 * s, c, true);
    stroke_line(pm, cx - 3.5 * s, cy - 2.5 * s, cx + 3.5 * s, cy - 2.5 * s, 1.3 * s, c);
    stroke_line(pm, cx - 3.5 * s, cy + 0.2 * s, cx + 1.5 * s, cy + 0.2 * s, 1.3 * s, c);
}

/// Tab `i` of `count`, centred in the notch.
fn tab_rect(i: usize, count: usize) -> (f32, f32, f32, f32) {
    let total = count as f32 * TAB_W + (count as f32 - 1.0) * TAB_GAP;
    let x0 = (EXPANDED.0 as f32 - total) / 2.0;
    (x0 + i as f32 * (TAB_W + TAB_GAP), 6.0, TAB_W, 20.0)
}

/// mm:ss, rounded up so the display hits 0:00 exactly when the phase ends.
fn fmt_remaining(d: Duration) -> String {
    let secs = d.as_secs() + (d.subsec_nanos() > 0) as u64;
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, secs % 3600 / 60, secs % 60)
    } else {
        format!("{:02}:{:02}", secs / 60, secs % 60)
    }
}

/// Stroke a clockwise arc starting at 12 o'clock covering `frac` of the circle.
fn stroke_arc(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, width: f32, frac: f32, c: [u8; 4]) {
    if frac < 0.002 {
        return;
    }
    let steps = ((frac * 120.0).ceil() as usize).max(2);
    let start = -std::f32::consts::FRAC_PI_2;
    let mut pb = PathBuilder::new();
    for i in 0..=steps {
        let ang = start + std::f32::consts::TAU * frac * i as f32 / steps as f32;
        let (x, y) = (cx + r * ang.cos(), cy + r * ang.sin());
        if i == 0 {
            pb.move_to(x, y);
        } else {
            pb.line_to(x, y);
        }
    }
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        {
            let c = theme::map(c);
            paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        }
        paint.anti_alias = true;
        let stroke = Stroke { width, line_cap: if frac >= 0.999 { LineCap::Butt } else { LineCap::Round }, ..Default::default() };
        pm.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

/// Stroke an arc from `start` radians, sweeping clockwise (screen coordinates, 0 = 3 o'clock).
fn stroke_arc_from(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, width: f32, start: f32, sweep: f32, c: [u8; 4]) {
    let steps = ((sweep.abs() * r / 2.0).ceil() as usize).max(6);
    let mut pb = PathBuilder::new();
    for i in 0..=steps {
        let ang = start + sweep * i as f32 / steps as f32;
        let (x, y) = (cx + r * ang.cos(), cy + r * ang.sin());
        if i == 0 {
            pb.move_to(x, y);
        } else {
            pb.line_to(x, y);
        }
    }
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        {
            let c = theme::map(c);
            paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        }
        paint.anti_alias = true;
        let stroke = Stroke { width, line_cap: LineCap::Round, ..Default::default() };
        pm.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

fn stroke_line(pm: &mut Pixmap, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, c: [u8; 4]) {
    let mut pb = PathBuilder::new();
    pb.move_to(x1, y1);
    pb.line_to(x2, y2);
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        {
            let c = theme::map(c);
            paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        }
        paint.anti_alias = true;
        let stroke = Stroke { width, line_cap: LineCap::Round, ..Default::default() };
        pm.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

/// Horizontal level bar centered on `cy`.
fn draw_bar(pm: &mut Pixmap, x: f32, cy: f32, w: f32, frac: f32, fill: [u8; 4], s: f32, a: u8) {
    let h = 6.0 * s;
    let alpha = a as f32 / 255.0;
    fill_rounded(pm, x, cy - h / 2.0, w, h, h / 2.0, [255, 255, 255, (alpha * 30.0) as u8]);
    if frac > 0.0 {
        let fill_w = (w * frac.clamp(0.0, 1.0)).max(h);
        gfx::glow(pm, x + fill_w, cy, 14.0 * s, [fill[0], fill[1], fill[2], (alpha * 70.0) as u8], None);
        gfx::gradient_rounded(pm, x, cy - h / 2.0, fill_w, h, h / 2.0, gfx::mix(fill, [255, 255, 255, a], 0.1), gfx::mix(fill, [255, 255, 255, a], 0.55), true);
    }
}

fn draw_speaker(pm: &mut Pixmap, cx: f32, cy: f32, s: f32, c: [u8; 4], muted: bool) {
    fill_rect(pm, cx - 9.0 * s, cy - 3.0 * s, 4.0 * s, 6.0 * s, c);
    fill_path(pm, &[(cx - 5.0 * s, cy - 3.0 * s), (cx + 1.0 * s, cy - 8.0 * s), (cx + 1.0 * s, cy + 8.0 * s), (cx - 5.0 * s, cy + 3.0 * s)], c);
    if muted {
        stroke_line(pm, cx + 4.0 * s, cy - 4.0 * s, cx + 10.0 * s, cy + 4.0 * s, 1.6 * s, c);
        stroke_line(pm, cx + 10.0 * s, cy - 4.0 * s, cx + 4.0 * s, cy + 4.0 * s, 1.6 * s, c);
    } else {
        for r in [5.0, 9.0] {
            stroke_arc_from(pm, cx + 1.0 * s, cy, r * s, 1.6 * s, -0.7, 1.4, c);
        }
    }
}

fn draw_sun(pm: &mut Pixmap, cx: f32, cy: f32, s: f32, c: [u8; 4]) {
    fill_circle(pm, cx, cy, 4.0 * s, c);
    for i in 0..8 {
        let ang = i as f32 * std::f32::consts::FRAC_PI_4;
        let (dx, dy) = (ang.cos(), ang.sin());
        stroke_line(pm, cx + dx * 7.0 * s, cy + dy * 7.0 * s, cx + dx * 10.0 * s, cy + dy * 10.0 * s, 1.6 * s, c);
    }
}

fn draw_battery(pm: &mut Pixmap, cx: f32, cy: f32, s: f32, frac: f32, c: [u8; 4], a: u8) {
    let (x, y, w, h) = (cx - 11.0 * s, cy - 6.0 * s, 20.0 * s, 12.0 * s);
    fill_rounded(pm, x, y, w, h, 3.0 * s, [150, 150, 155, a]);
    fill_rounded(pm, x + 1.5 * s, y + 1.5 * s, w - 3.0 * s, h - 3.0 * s, 2.0 * s, [0, 0, 0, 255]);
    fill_rounded(pm, x + 2.5 * s, y + 2.5 * s, ((w - 5.0 * s) * frac.clamp(0.0, 1.0)).max(1.0), h - 5.0 * s, 1.0 * s, c);
    fill_rect(pm, x + w, cy - 2.5 * s, 2.5 * s, 5.0 * s, [150, 150, 155, a]);
}

fn draw_wifi(pm: &mut Pixmap, cx: f32, cy: f32, s: f32, c: [u8; 4], slashed: bool) {
    let base_y = cy + 7.0 * s;
    fill_circle(pm, cx, base_y, 1.8 * s, c);
    for r in [5.0, 9.0, 13.0] {
        stroke_arc_from(pm, cx, base_y, r * s, 1.8 * s, -2.36, 1.57, c);
    }
    if slashed {
        stroke_line(pm, cx - 9.0 * s, cy - 9.0 * s, cx + 9.0 * s, cy + 9.0 * s, 1.8 * s, [255, 90, 80, c[3]]);
    }
}

fn draw_headphones(pm: &mut Pixmap, cx: f32, cy: f32, s: f32, c: [u8; 4]) {
    stroke_arc_from(pm, cx, cy + 1.0 * s, 8.0 * s, 2.0 * s, -std::f32::consts::PI, std::f32::consts::PI, c);
    fill_rounded(pm, cx - 11.0 * s, cy + 1.0 * s, 5.0 * s, 9.0 * s, 2.0 * s, c);
    fill_rounded(pm, cx + 6.0 * s, cy + 1.0 * s, 5.0 * s, 9.0 * s, 2.0 * s, c);
}

/// Stroke a polyline with rounded ends and corners; `closed` joins the last point to the first.
fn stroke_poly(pm: &mut Pixmap, pts: &[(f32, f32)], width: f32, c: [u8; 4], closed: bool) {
    let mut pb = PathBuilder::new();
    pb.move_to(pts[0].0, pts[0].1);
    for p in &pts[1..] {
        pb.line_to(p.0, p.1);
    }
    if closed {
        pb.close();
    }
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        {
            let c = theme::map(c);
            paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        }
        paint.anti_alias = true;
        let stroke = Stroke { width, line_cap: LineCap::Round, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
        pm.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

/// The small icon that stands in for a tab's name, about 13 px across.
fn draw_tab_icon(pm: &mut Pixmap, view: View, cx: f32, cy: f32, s: f32, c: [u8; 4]) {
    let p = |dx: f32, dy: f32| (cx + dx * s, cy + dy * s);
    let w = 1.5 * s;
    match view {
        View::Player => {
            // Music note: head, stem and flag.
            fill_circle(pm, cx - 2.5 * s, cy + 4.0 * s, 2.5 * s, c);
            stroke_poly(pm, &[p(-0.2, 3.5), p(-0.2, -5.5), p(4.0, -3.0)], w, c, false);
        }
        View::Timer => {
            // Timer: ring with two hands.
            stroke_arc(pm, cx, cy + 0.5 * s, 5.5 * s, w, 1.0, c);
            stroke_poly(pm, &[p(0.0, 3.0), p(0.0, -0.5), p(2.5, 1.5)], 1.3 * s, c, false);
            stroke_line(pm, cx - 1.8 * s, cy - 7.0 * s, cx + 1.8 * s, cy - 7.0 * s, w, c);
        }
        View::Water => {
            // Drop.
            let mut pb = PathBuilder::new();
            pb.move_to(cx, cy - 6.5 * s);
            pb.cubic_to(cx + 1.0 * s, cy - 3.0 * s, cx + 5.0 * s, cy - 0.5 * s, cx + 5.0 * s, cy + 2.0 * s);
            pb.cubic_to(cx + 5.0 * s, cy + 5.0 * s, cx + 2.8 * s, cy + 6.5 * s, cx, cy + 6.5 * s);
            pb.cubic_to(cx - 2.8 * s, cy + 6.5 * s, cx - 5.0 * s, cy + 5.0 * s, cx - 5.0 * s, cy + 2.0 * s);
            pb.cubic_to(cx - 5.0 * s, cy - 0.5 * s, cx - 1.0 * s, cy - 3.0 * s, cx, cy - 6.5 * s);
            pb.close();
            if let Some(path) = pb.finish() {
                let mut paint = Paint::default();
                {
            let c = theme::map(c);
            paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        }
                paint.anti_alias = true;
                pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
            }
        }
        View::Shelf => {
            // Tray with an arrow dropping into it.
            stroke_poly(pm, &[p(-6.0, 0.5), p(-6.0, 5.5), p(6.0, 5.5), p(6.0, 0.5)], w, c, false);
            stroke_line(pm, cx, cy - 6.0 * s, cx, cy + 2.0 * s, w, c);
            stroke_poly(pm, &[p(-3.0, -1.0), p(0.0, 2.0), p(3.0, -1.0)], w, c, false);
        }
        View::Todo => {
            // Check mark in a circle.
            stroke_arc(pm, cx, cy, 6.0 * s, w, 1.0, c);
            stroke_poly(pm, &[p(-2.6, 0.2), p(-0.7, 2.3), p(2.8, -2.0)], w, c, false);
        }
        View::Clipboard => {
            // Board, clip and two lines of text.
            stroke_poly(pm, &[p(-5.0, -5.0), p(5.0, -5.0), p(5.0, 6.5), p(-5.0, 6.5)], w, c, true);
            fill_rounded(pm, cx - 2.6 * s, cy - 7.2 * s, 5.2 * s, 3.4 * s, 1.2 * s, c);
            stroke_line(pm, cx - 2.4 * s, cy, cx + 2.4 * s, cy, 1.2 * s, c);
            stroke_line(pm, cx - 2.4 * s, cy + 3.0 * s, cx + 1.0 * s, cy + 3.0 * s, 1.2 * s, c);
        }
        View::Calendar => {
            // Page with two rings, a heading line and a few dates.
            stroke_poly(pm, &[p(-6.0, -4.5), p(6.0, -4.5), p(6.0, 6.5), p(-6.0, 6.5)], w, c, true);
            stroke_line(pm, cx - 6.0 * s, cy - 1.0 * s, cx + 6.0 * s, cy - 1.0 * s, 1.2 * s, c);
            stroke_line(pm, cx - 3.0 * s, cy - 7.0 * s, cx - 3.0 * s, cy - 3.5 * s, w, c);
            stroke_line(pm, cx + 3.0 * s, cy - 7.0 * s, cx + 3.0 * s, cy - 3.5 * s, w, c);
            for (dx, dy) in [(-3.0, 2.0), (0.0, 2.0), (3.0, 2.0), (-3.0, 4.8), (0.0, 4.8)] {
                fill_circle(pm, cx + dx * s, cy + dy * s, 0.8 * s, c);
            }
        }
        View::Lyrics => {}
    }
}

fn fill_circle(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, c: [u8; 4]) {
    if let Some(path) = PathBuilder::from_circle(cx, cy, r) {
        let mut paint = Paint::default();
        {
            let c = theme::map(c);
            paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        }
        paint.anti_alias = true;
        pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }
}

fn fill_rounded(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, c: [u8; 4]) {
    let r = r.min(w / 2.0).min(h / 2.0);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        {
            let c = theme::map(c);
            paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        }
        paint.anti_alias = true;
        pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }
}

fn fill_rect(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, c: [u8; 4]) {
    let Some(rect) = Rect::from_xywh(x, y, w.max(0.01), h) else { return };
    let mut paint = Paint::default();
    {
            let c = theme::map(c);
            paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        }
    pm.fill_rect(rect, &paint, Transform::identity(), None);
}

fn fill_path(pm: &mut Pixmap, pts: &[(f32, f32)], c: [u8; 4]) {
    let mut pb = PathBuilder::new();
    pb.move_to(pts[0].0, pts[0].1);
    for p in &pts[1..] {
        pb.line_to(p.0, p.1);
    }
    pb.close();
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        {
            let c = theme::map(c);
            paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        }
        paint.anti_alias = true;
        pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }
}

/// The play triangle and the pause bars. `pause` goes from 0 (play) to 1 (pause); in between, one icon
/// shrinks away while the other grows in, so pressing the button looks like a change instead of a swap.
fn draw_play_pause(pm: &mut Pixmap, cx: f32, cy: f32, s: f32, c: [u8; 4], pause: f32) {
    let pause = pause.clamp(0.0, 1.0);
    let tint = |t: f32| [c[0], c[1], c[2], (c[3] as f32 * t) as u8];
    if pause > 0.01 {
        let k = (0.55 + 0.45 * pause) * s;
        fill_rect(pm, cx - 7.0 * k, cy - 9.0 * k, 5.0 * k, 18.0 * k, tint(pause));
        fill_rect(pm, cx + 2.0 * k, cy - 9.0 * k, 5.0 * k, 18.0 * k, tint(pause));
    }
    if pause < 0.99 {
        let k = (0.55 + 0.45 * (1.0 - pause)) * s;
        fill_path(pm, &[(cx - 6.0 * k, cy - 10.0 * k), (cx + 9.0 * k, cy), (cx - 6.0 * k, cy + 10.0 * k)], tint(1.0 - pause));
    }
}

fn draw_next(pm: &mut Pixmap, cx: f32, cy: f32, s: f32, c: [u8; 4]) {
    fill_path(pm, &[(cx - 8.0 * s, cy - 8.0 * s), (cx + 4.0 * s, cy), (cx - 8.0 * s, cy + 8.0 * s)], c);
    fill_rect(pm, cx + 5.0 * s, cy - 8.0 * s, 3.0 * s, 16.0 * s, c);
}

fn draw_prev(pm: &mut Pixmap, cx: f32, cy: f32, s: f32, c: [u8; 4]) {
    fill_path(pm, &[(cx + 8.0 * s, cy - 8.0 * s), (cx - 4.0 * s, cy), (cx + 8.0 * s, cy + 8.0 * s)], c);
    fill_rect(pm, cx - 8.0 * s, cy - 8.0 * s, 3.0 * s, 16.0 * s, c);
}

fn fmt_time(d: Duration) -> String {
    let t = d.as_secs();
    format!("{:02}:{:02}", t / 60, t % 60)
}

/// Things that can be blended between two states with a 0 to 1 amount.
trait Blend: Copy {
    fn blend(off: Self, on: Self, t: f32) -> Self;
}

impl Blend for f32 {
    fn blend(off: f32, on: f32, t: f32) -> f32 {
        off + (on - off) * t
    }
}

impl Blend for [u8; 4] {
    fn blend(off: [u8; 4], on: [u8; 4], t: f32) -> [u8; 4] {
        gfx::mix(off, on, t.clamp(0.0, 1.0))
    }
}

/// `on` when `t` is 1, `off` when it is 0, and a blend in between: how hover glows fade.
fn pick<T: Blend>(on: T, off: T, t: f32) -> T {
    T::blend(off, on, t)
}

/// Whether Ctrl is held right now. The notch never has the keyboard focus, so its own key events
/// cannot be trusted to say.
fn ctrl_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL};
    unsafe { GetAsyncKeyState(VK_CONTROL.0 as i32) < 0 }
}

/// The curved flares on both sides of the notch's top edge, where it meets the top of the screen: the
/// outer corners curve inwards, like a notch cut into the screen. `ear` is their width and height.
fn draw_ears(pm: &mut Pixmap, ear: f32) {
    if ear < 1.0 {
        return;
    }
    let width = pm.width() as f32;
    let body = theme::body()[0];
    let mut paint = Paint::default();
    paint.set_color_rgba8(body[0], body[1], body[2], 255);
    paint.anti_alias = true;
    for left in [true, false] {
        // Drawn for the left side, then mirrored for the right.
        let x = |v: f32| if left { v } else { width - v };
        let mut pb = PathBuilder::new();
        pb.move_to(x(0.0), 0.0);
        pb.line_to(x(ear), 0.0);
        pb.line_to(x(ear), ear);
        pb.quad_to(x(ear), 0.0, x(0.0), 0.0);
        pb.close();
        if let Some(path) = pb.finish() {
            pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }
}

/// The colour of the eyes (and of their sleepy and happy faces) in a style.
fn eye_colour(style: EyeStyle) -> [u8; 3] {
    match style {
        EyeStyle::Classic | EyeStyle::Dots => [255, 255, 255],
        EyeStyle::Cat => [255, 196, 80],
        EyeStyle::Glow => [110, 175, 255],
    }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        // A setup downloaded by "Check for updates" has done its job once this version is running.
        update::clean_up_downloads();
        let attrs = Window::default_attributes()
            .with_title("Sky Island")
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_skip_taskbar(true)
            .with_inner_size(LogicalSize::new(COLLAPSED.0 + 2.0 * EAR, COLLAPSED.1));
        let window = Rc::new(event_loop.create_window(attrs).expect("create window"));
        let context = Context::new(window.clone()).expect("softbuffer context");
        self.surface = Some(Surface::new(&context, window.clone()).expect("softbuffer surface"));
        self.system_light = window.theme() == Some(Theme::Light);
        self.window = Some(window);
        self.media_handle = Some(media::spawn(self.proxy.clone()));
        sys::spawn_all(self.proxy.clone());
        self.make_non_activating();
        self.build_tray();
        sys::install_lock_hook(self.proxy.clone());
        presence::spawn(self.proxy.clone());
        clipboard::spawn_watcher(self.proxy.clone());
        self.step();
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Notice(notice) => self.show_notice(notice),
            UserEvent::Clip(content) => {
                if self.settings.clipboard
                    && self.clips.add(content)
                    && self.view == View::Clipboard
                    && self.progress > 0.0
                    && let Some(w) = &self.window {
                        w.request_redraw();
                    }
            }
            UserEvent::Menu(id) => {
                let now = Instant::now();
                if Some(&id) == self.tray_quit_id.as_ref() {
                    self.quit = true;
                } else if Some(&id) == self.tray_toggle_id.as_ref() {
                    self.settings.hide_island = !self.settings.hide_island;
                    self.settings.save();
                    self.sync_tray_toggle();
                    self.last_tick = now;
                } else if Some(&id) == self.tray_settings_id.as_ref() {
                    self.panel_wanted = true;
                    self.last_tick = now;
                }
            }
            UserEvent::Presence { presence, fullscreen } => {
                let now = Instant::now();
                // Coming back from idle: greet first, then back to the clock.
                if self.presence != Presence::Active && presence == Presence::Active {
                    self.greeting_until = Some(now + GREETING);
                }
                if presence == Presence::Idle {
                    self.next_blink = now + Duration::from_millis(1500);
                }
                self.presence = presence;
                self.fullscreen = fullscreen;
                self.apply_visibility();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            UserEvent::Update(status) => {
                self.update = status;
                if let Some(p) = &self.panel {
                    p.redraw();
                }
            }
            UserEvent::LockKey { name, vk } => {
                self.show_notice(sys::Notice::Lock { name, on: sys::key_toggled(vk) });
            }
            UserEvent::Lyrics { key, lines } => {
                // Ignore answers for a song that is no longer playing.
                if key == self.lyrics_key {
                    self.lyrics = match lines {
                        Some(l) => LyricsState::Found(l),
                        None => LyricsState::NotFound,
                    };
                    self.lyric_manual = None;
                    self.snap_lyrics();
                    if self.progress > 0.0
                        && let Some(w) = &self.window {
                            w.request_redraw();
                        }
                }
            }
            UserEvent::Media(state) => {
                let key = format!("{}|{}", state.title, state.artist);
                if !state.has_session || state.title.is_empty() {
                    self.lyrics_key.clear();
                    self.lyrics = LyricsState::None;
                } else if key != self.lyrics_key {
                    self.lyrics_key = key.clone();
                    self.lyrics = LyricsState::Loading;
                    self.lyric_manual = None;
                    self.lyric_pos = -1.0;
                    self.lyric_v = 0.0;
                    lyrics::spawn_fetch(key, state.title.clone(), state.artist.clone(), state.duration, self.proxy.clone());
                }
                if let Some(art) = &state.art {
                    let art_key = format!("{}|{}|{}", state.title, state.artist, art.len());
                    if art_key != self.art_key {
                        self.art_tint = art_tint(art);
                        self.art_key = art_key;
                    }
                }
                self.media = state;
                self.media_at = Instant::now();
                // Only worth repainting if the player is visible.
                if self.progress > 0.0
                    && let Some(w) = &self.window {
                        w.request_redraw();
                    }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.panel.as_ref().is_some_and(|p| p.id() == id) {
            self.panel_event(event);
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::CursorEntered { .. } => {
                let now = Instant::now();
                self.leave_at = None;
                if !self.hovered {
                    self.enter_at = Some(now + self.hover_open());
                    event_loop.set_control_flow(ControlFlow::WaitUntil(now + self.hover_open()));
                }
            }
            WindowEvent::CursorLeft { .. } => {
                let now = Instant::now();
                self.enter_at = None; // a fly-by never opened it
                if self.hovered && self.scrub.is_none() && !self.lyric_settle {
                    self.leave_at = Some(now + self.hover_close());
                    event_loop.set_control_flow(ControlFlow::WaitUntil(now + self.hover_close()));
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                // The window has a flare on each side of the notch; positions count from the notch itself.
                let ear = EAR * self.window.as_ref().map_or(1.0, |w| w.scale_factor());
                let position = PhysicalPosition::new(position.x - ear, position.y);
                self.cursor = (position.x, position.y);
                if self.scrub.is_some() {
                    let s = self.window.as_ref().map_or(1.0, |w| w.scale_factor());
                    self.scrub = Some(self.scrub_at((position.x / s) as f32));
                }
                // Hover highlights follow the mouse.
                if self.progress >= 0.9
                    && let Some(w) = &self.window {
                        w.request_redraw();
                    }
                if let Some((idx, start)) = self.press
                    && (position.x - start.0).hypot(position.y - start.1) > DRAG_THRESHOLD {
                        self.press = None;
                        self.drag_out(idx);
                    }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                self.click();
            }
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. } => {
                if let Some(at) = self.scrub.take() {
                    if let Some(handle) = &self.media_handle {
                        handle.send(Command::Seek(at));
                    }
                    // Show the jump immediately instead of waiting for the player to report it.
                    self.media.position = at;
                    self.media_at = Instant::now();
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
                // A press that never turned into a drag is a click: open the file.
                if let Some((idx, _)) = self.press.take()
                    && let Some(path) = self.shelf.items.get(idx).cloned() {
                        let _ = std::process::Command::new("explorer").arg(&path).spawn();
                        self.selected.clear();
                        self.sync_selection_focus();
                    }
            }
            WindowEvent::Focused(false) if self.selecting => {
                // The user went to another app: the highlight and the keyboard are no longer ours.
                self.selecting = false;
                self.prev_foreground = None;
                self.selected.clear();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } if self.view == View::Shelf && event.state == ElementState::Pressed => {
                match &event.logical_key {
                    Key::Named(NamedKey::Escape) => self.selected.clear(),
                    _ => return,
                }
                self.sync_selection_focus();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } if self.view == View::Timer && !self.editing && self.progress >= 0.9 => {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y,
                };
                let s = self.window.as_ref().map_or(1.0, |w| w.scale_factor()) as f32;
                let (cx, cy) = (self.cursor.0 as f32 / s, self.cursor.1 as f32 / s);
                let column = TIMER_COLS.iter().position(|x| (cx - x).abs() <= 28.0);
                if let Some(i) = column.filter(|_| dy != 0.0 && (TIMER_PLUS_Y - 12.0..=TIMER_MINUS_Y + 12.0).contains(&cy))
                    && self.timer.adjust(Unit::ALL[i], if dy > 0.0 { 1 } else { -1 })
                    && let Some(w) = &self.window {
                        w.request_redraw();
                    }
            }
            WindowEvent::MouseWheel { delta, .. } if self.view == View::Lyrics && !self.editing && self.progress >= 0.9 => {
                let s = self.window.as_ref().map_or(1.0, |w| w.scale_factor()) as f32;
                let (cx, cy) = (self.cursor.0 as f32 / s, self.cursor.1 as f32 / s);
                let lines = match &self.lyrics {
                    LyricsState::Found(l) => l.len(),
                    _ => 0,
                };
                // Scroll the lyrics by hand: wheel up goes to earlier lines.
                let step = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y / 24.0,
                };
                if lines > 0 && step != 0.0 && cx >= LYRIC_X - 8.0 && (LYRIC_AREA.0..=LYRIC_AREA.1).contains(&cy) {
                    let from = self.lyric_manual.unwrap_or(self.lyric_pos.max(0.0));
                    let to = (from - step).clamp(0.0, lines as f64 - 1.0);
                    let sung = self.current_line();
                    // Back on the line being sung: carry on following the music.
                    self.lyric_manual = if sung.is_some_and(|c| c == to.round() as usize) { None } else { Some(to) };
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } if self.editing && self.progress >= 0.9 => {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y,
                };
                let s = self.window.as_ref().map_or(1.0, |w| w.scale_factor()) as f32;
                let (cx, cy) = (self.cursor.0 as f32 / s, self.cursor.1 as f32 / s);
                if dy != 0.0
                    && self.editor.wheel(cx, cy, if dy > 0.0 { 1 } else { -1 }, clock_now())
                    && let Some(w) = &self.window {
                        w.request_redraw();
                    }
            }
            WindowEvent::MouseWheel { delta, .. }
                if matches!(self.view, View::Shelf | View::Todo | View::Clipboard) && self.progress >= 0.9 =>
            {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y,
                };
                if dy != 0.0 {
                    // Wheel up shows earlier rows.
                    let rows = if dy > 0.0 { -1 } else { 1 };
                    match self.view {
                        View::Todo => self.todo.scroll_by(rows),
                        View::Clipboard => self.clips.scroll_by(rows),
                        _ => self.shelf.scroll_by(rows),
                    }
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } if self.timer_field.is_some() && event.state == ElementState::Pressed => {
                self.timer_key(&event);
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } if self.editing && event.state == ElementState::Pressed => {
                match &event.logical_key {
                    Key::Named(NamedKey::Enter) => self.commit_task(),
                    Key::Named(NamedKey::Escape) => self.stop_editing(),
                    Key::Named(NamedKey::Backspace) => {
                        self.input.pop();
                    }
                    _ => {
                        if let Some(text) = &event.text {
                            for c in text.chars().filter(|c| !c.is_control()) {
                                if self.input.chars().count() < 80 {
                                    self.input.push(c);
                                }
                            }
                        }
                    }
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::Focused(false) if self.timer_field.is_some() => {
                // The user clicked into another app: keep what was typed and don't steal focus back.
                self.prev_foreground = None;
                self.end_timer_edit(true);
            }
            WindowEvent::Focused(false) if self.editing => {
                // The user clicked into another app: stop typing without stealing focus back.
                self.prev_foreground = None;
                self.stop_editing();
            }
            WindowEvent::HoveredFile(_) => {
                self.drag_hover = true;
                self.view = View::Shelf;
                self.last_tick = Instant::now();
                event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + FRAME));
            }
            WindowEvent::HoveredFileCancelled => {
                self.drag_hover = false;
                self.last_tick = Instant::now();
            }
            WindowEvent::DroppedFile(path) => {
                self.shelf.add(path);
                self.drag_hover = false;
                // The drop happened over the notch, so keep it open until the cursor leaves.
                self.hovered = true;
                self.leave_at = None;
                self.view = View::Shelf;
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::ThemeChanged(theme) => {
                self.system_light = theme == Theme::Light;
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => self.draw(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.quit {
            event_loop.exit();
            return;
        }
        let now = Instant::now();

        // Highlighted shelf files hold the keyboard; let go of both once the notch closes or the shelf is left.
        if self.selecting && (self.view != View::Shelf || !self.hovered) {
            self.selected.clear();
            self.sync_selection_focus();
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }

        if self.panel_wanted {
            self.panel_wanted = false;
            self.open_panel(event_loop);
        }
        // The settings window asks for frames while its controls are moving.
        let mut panel_wake = None;
        if let Some(p) = &mut self.panel {
            if p.reset_armed.is_some_and(|t| now >= t) {
                p.reset_armed = None;
                p.redraw();
            }
            if p.busy {
                p.redraw();
                panel_wake = Some(now + FRAME);
            } else if let Some(t) = p.reset_armed {
                panel_wake = Some(t);
            }
        }

        if self.enter_at.is_some_and(|t| now >= t) {
            self.enter_at = None;
            self.hovered = true;
            self.last_tick = now;
        }
        if self.leave_at.is_some_and(|t| now >= t) {
            self.leave_at = None;
            self.hovered = false;
            self.last_tick = now;
        }

        // The notch has finished changing width. If the mouse is now outside it (closing the lyrics pulls
        // the sides in), give it a moment to come back before the notch closes.
        if self.lyric_settle && !self.animating() {
            self.lyric_settle = false;
            if self.hovered && !self.cursor_over_window() {
                self.leave_at = Some(now + LYRIC_GRACE);
            }
        }

        if self.timer.tick(now) {
            self.alert_msg = TIMES_UP.into();
            self.alert_until = Some(now + self.alarm_for());
            self.next_chime = Some(now + self.alarm_every());
            self.view = View::Timer;
            self.last_tick = now;
            self.play_alarm();
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
        if let Some(text) = self.todo.take_due(todo::unix_now()) {
            let short: String = text.chars().take(40).collect();
            self.alert_msg = format!("Reminder: {short}");
            self.alert_until = Some(now + ALERT);
            self.view = View::Todo;
            self.last_tick = now;
            self.play_sound(sound::Chime::Reminder);
        }
        self.water.roll(today());
        if self.settings.water && self.water.due(now) {
            self.alert_msg = "Time to drink some water!".into();
            self.alert_until = Some(now + ALERT);
            self.view = View::Water;
            self.last_tick = now;
            self.play_sound(sound::Chime::Reminder);
        }
        if self.next_chime.is_some_and(|t| now >= t) {
            if self.alert_until.is_some() && self.alert_msg == TIMES_UP {
                self.play_alarm();
                self.next_chime = Some(now + self.alarm_every());
            } else {
                self.next_chime = None;
            }
        }
        if self.alert_until.is_some_and(|t| now >= t) {
            if self.alert_msg == TIMES_UP {
                sound::stop_file();
            }
            self.alert_until = None;
            self.last_tick = now;
        }
        if self.notice_until.is_some_and(|t| now >= t) {
            self.notice_until = None;
            self.last_tick = now;
        }
        if self.clip_flash.as_ref().is_some_and(|(_, t)| now >= *t) {
            self.clip_flash = None;
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }

        if self.view != self.shown_view {
            // Moving between Music and its lyrics is the same page; any other change is a transition in
            // which the old view fades out as the new one slides in from the side of its tab.
            let tab = |v: View| self.visible_tabs().iter().position(|(_, t)| *t == if v == View::Lyrics { View::Player } else { v });
            let (from, to) = (tab(self.shown_view), tab(self.view));
            if from != to || from.is_none() {
                self.prev_view = Some(self.shown_view);
                self.switch_dir = match (from, to) {
                    (Some(a), Some(b)) if b < a => -1.0,
                    _ => 1.0,
                };
                self.switch_t = 0.0;
                self.switch_v = 0.0;
            }
            self.shown_view = self.view;
            self.last_tick = now;
        }
        let goal = self.tuck_target();
        if goal != self.tuck_goal {
            self.tuck_goal = goal;
            self.last_tick = now;
        }

        // Only tick while something is changing; otherwise sleep until the next event (0% idle CPU).
        if self.animating() {
            self.step();
            event_loop.set_control_flow(ControlFlow::WaitUntil(now + FRAME));
            return;
        }

        let mut wake: Option<Instant> = None;
        self.companion_tick(now, &mut wake);
        let mut at = |t: Instant| earliest(&mut wake, t);

        // Periodic refresh for the progress bar / countdown. Only redraw when due, since a redraw
        // itself triggers another about_to_wait.
        let expanded = self.progress >= 1.0;
        if (expanded && (self.media.playing || self.editing || self.timer_field.is_some() || matches!(self.view, View::Water | View::Todo)))
            || self.timer.running()
        {
            let tick = if expanded && self.view == View::Timer && self.timer.running() { RING_TICK } else { PROGRESS_TICK };
            let due = self.last_ui_tick + tick;
            if now >= due {
                self.last_ui_tick = now;
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
                at(now + tick);
            } else {
                at(due);
            }
        }
        // Wake exactly when the next lyric line starts, so the glide begins on time.
        if expanded && self.view == View::Lyrics && self.media.playing
            && let LyricsState::Found(lines) = &self.lyrics {
                let next = self.current_line().map_or(0, |c| c + 1);
                if let Some(line) = lines.get(next) {
                    at(now + line.at.saturating_sub(self.position()));
                }
            }
        if let Some(d) = self.timer.deadline() {
            at(d);
        }
        if let Some(d) = self.water.deadline().filter(|_| self.settings.water) {
            at(d);
        }
        if let Some(due) = self.todo.next_due() {
            at(now + Duration::from_secs(due.saturating_sub(todo::unix_now())));
        }
        if let Some(t) = self.alert_until {
            at(t);
        }
        if let Some(t) = self.next_chime {
            at(t);
        }
        if let Some(t) = panel_wake {
            at(t);
        }
        for t in [self.enter_at, self.leave_at].into_iter().flatten() {
            at(t);
        }
        if let Some(t) = self.notice_until {
            at(t);
        }
        if let Some((_, t)) = self.clip_flash {
            at(t);
        }

        event_loop.set_control_flow(match wake {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        });
    }
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

fn main() {
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
    fn lyric_lines_shrink_and_fade_with_distance_from_the_middle() {
        assert_eq!(lyric_px(0.0), 18.0);
        assert_eq!(lyric_px(1.0), 14.0);
        assert_eq!(lyric_px(2.0), 13.0);
        assert_eq!(lyric_px(5.0), 13.0);
        assert!(lyric_px(0.5) > 14.0 && lyric_px(0.5) < 18.0, "smooth in between");
        assert_eq!(lyric_tone(0.0), (255, 1.0));
        assert_eq!(lyric_tone(9.0), (110, 0.45));
        let (grey, strength) = lyric_tone(0.5);
        assert!(grey > 175 && grey < 255 && strength > 0.85 && strength < 1.0);
    }

    /// The whole tab bar has to fit in the notch.
    #[test]
    fn tab_bar_fits_the_notch() {
        let total = TABS.len() as f32 * TAB_W + (TABS.len() as f32 - 1.0) * TAB_GAP;
        assert!(total <= EXPANDED.0 as f32 - 8.0, "tab bar is {total}px wide");
    }

    #[test]
    fn art_tint_follows_the_cover_colour() {
        let solid = |c: [u8; 4]| c.iter().copied().cycle().take(4 * 400).collect::<Vec<u8>>();
        let [r, g, b] = art_tint(&solid([200, 30, 30, 255]));
        assert!(r > 200 && g < 80 && b < 80, "red cover gives a red accent: {r},{g},{b}");
        let [r, g, b] = art_tint(&solid([10, 10, 10, 255]));
        assert!(r > 150 && (b as i32 - r as i32).abs() < 20 && g > 150, "a dark grey cover still gives a visible, neutral accent: {r},{g},{b}");
        assert!(art_tint(&[]).iter().all(|&c| c > 150), "no pixels: no panic, a neutral accent");
    }

    /// Every non-ASCII character the UI draws must exist in the loaded font, or it shows as a box.
    #[test]
    fn ui_glyphs_exist_in_font() {
        let font = load_font().expect("a system font");
        let missing: Vec<char> = "\u{00D7}\u{00B7}\u{2026}\u{2014}"
            .chars()
            .filter(|&c| font.lookup_glyph_index(c) == 0)
            .collect();
        assert!(missing.is_empty(), "missing glyphs: {missing:?}");
    }
}
