// A GUI program: without this Windows opens a console window next to it.
#![windows_subsystem = "windows"]
#![allow(clippy::too_many_arguments)]

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
mod focus;
mod icon;

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
use winit::window::{Window, WindowId, WindowLevel};

use lyrics::Line;
use media::{Command, MediaHandle, MediaState};
use clipboard::History;
use focus::{CYCLE, FocusTimer};
use presence::Presence;
use settings::Settings;
use shelf::{COLS, ROWS, Shelf};
use todo::Todo;
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use water::Water;

const COLLAPSED: (f64, f64) = (180.0, 32.0);
const EXPANDED: (f64, f64) = (420.0, 168.0);
const ANIM: Duration = Duration::from_millis(220);
const FRAME: Duration = Duration::from_millis(8);
const PROGRESS_TICK: Duration = Duration::from_millis(500);

// Expanded layout, in logical pixels.
const ART_RECT: (f32, f32, f32) = (24.0, 30.0, 64.0); // x, y, size
const TEXT_X: f32 = 104.0;
const BAR: (f32, f32, f32) = (24.0, 106.0, 372.0); // x, y, width
const CTRL_Y: f32 = 142.0;
const CTRL_X: [f32; 3] = [150.0, 210.0, 270.0]; // prev, play/pause, next
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
}

#[derive(PartialEq, Clone, Copy)]
enum View {
    Player,
    Lyrics,
    Focus,
    Water,
    Shelf,
    Todo,
    Clipboard,
    Settings,
}

/// Tab bar entries, left to right.
const TABS: [(&str, View); 7] = [
    ("Music", View::Player),
    ("Focus", View::Focus),
    ("Water", View::Water),
    ("Shelf", View::Shelf),
    ("To-do", View::Todo),
    ("Clipboard", View::Clipboard),
    ("Settings", View::Settings),
];
const TAB_W: f32 = 44.0;
const TAB_GAP: f32 = 4.0;
// Focus view layout, in logical pixels.
const RING: (f32, f32, f32) = (104.0, 100.0, 48.0); // center x, center y, radius
const RING_W: f32 = 7.0;
const FOCUS_CTRL_Y: f32 = 136.0;
const FOCUS_X: [f32; 3] = [232.0, 293.0, 354.0]; // left, center, right controls (used by the water view)
// Focus view controls: start/pause and reset, centered as a pair under the title.
const FOCUS_START_X: f32 = 263.0;
const FOCUS_RESET_X: f32 = 323.0;
const ADJUST: [(f32, f32); 2] = [(231.0, 70.0), (355.0, 70.0)]; // minus, plus button centers
const ADJUST_R: f32 = 11.0;
const CHIP_W: f32 = 54.0;
const CHIP_GAP: f32 = 4.0;
const WATER_CHIP_W: f32 = 46.0;
// Shelf tiles, in logical pixels.
const TILE: (f32, f32) = (68.0, 58.0);
const TILE_GAP: (f32, f32) = (8.0, 6.0);
const TILES_ORIGIN: (f32, f32) = (24.0, 36.0);
/// Cursor travel, in physical pixels, before a press on a tile becomes a drag.
const DRAG_THRESHOLD: f64 = 6.0;
/// Compact size of the notch while a system notice (volume, battery...) is showing.
const OVERLAY: (f64, f64) = (300.0, 52.0);
const NOTICE_TIME: Duration = Duration::from_millis(2200);
const GREETING: Duration = Duration::from_millis(1600);
const BLINK: Duration = Duration::from_millis(160);
const SETTINGS_ITEMS: usize = 8;
// To-do view, in logical pixels.
const TODO_FIELD: (f32, f32, f32, f32) = (24.0, 34.0, 372.0, 24.0);
const TODO_ROWS_Y: f32 = 64.0;
const TODO_ROW_H: f32 = 24.0;
const TODO_AMBER: [u8; 3] = [255, 190, 70];
/// How long the cursor must rest on the closed notch before it opens, so fly-bys don't trigger it.
const HOVER_OPEN: Duration = Duration::from_millis(90);
/// How long the cursor can be off the open notch before it closes, so small slips don't close it.
const HOVER_CLOSE: Duration = Duration::from_millis(250);
/// Seconds the lyrics take to glide to a newly sung line.
const LYRIC_GLIDE: f32 = 0.28;
// Clipboard view, in logical pixels.
const CLIP_CLEAR: (f32, f32, f32, f32) = (324.0, 34.0, 72.0, 22.0);
const CLIP_ROWS_Y: f32 = 64.0;
const CLIP_ROW_H: f32 = 32.0;
const CLIP_TEAL: [u8; 3] = [70, 200, 210];
/// How long the "Copied" / "Cleared" message stays in the clipboard header.
const FLASH: Duration = Duration::from_millis(1400);
const ALERT: Duration = Duration::from_secs(6);

enum LyricsState {
    None,
    Loading,
    NotFound,
    Found(Vec<Line>),
}

// Lyrics view layout, in logical pixels.
const BACK_RECT: (f32, f32, f32) = (16.0, 10.0, 28.0); // x, y, size
const LYRICS_H: f64 = 236.0; // notch height while the lyrics view is open
const LYRIC_AREA: (f32, f32) = (48.0, 224.0); // top, bottom of the scrolling lyrics area
const LYRIC_W: f32 = 372.0; // wrap width
const LYRIC_GAP: f32 = 12.0;

struct LyricItem {
    idx: usize,
    top: f32,
    lh: f32,
    px: f32,
    dist: u32,
    rows: Vec<String>,
}

struct App {
    window: Option<Rc<Window>>,
    surface: Option<Surface<Rc<Window>, Rc<Window>>>,
    font: Option<Font>,
    media: MediaState,
    media_at: Instant,
    media_handle: Option<MediaHandle>,
    view: View,
    /// 0.0 = player height, 1.0 = lyrics height
    view_t: f64,
    lyrics: LyricsState,
    lyrics_key: String,
    focus: FocusTimer,
    water: Water,
    shelf: Shelf,
    /// Shell icons for shelf files; `None` when the shell has no icon for a file.
    icons: HashMap<PathBuf, Option<Pixmap>>,
    /// A file is being dragged over the notch from outside.
    drag_hover: bool,
    /// Shelf item index and cursor position where the mouse went down.
    press: Option<(usize, (f64, f64))>,
    /// Latest system notice, kept after it expires so it can fade out.
    notice: Option<sys::Notice>,
    notice_until: Option<Instant>,
    /// 0.0 = no notice, 1.0 = compact notice size
    ov_t: f64,
    clips: History,
    /// A short message for the clipboard header, with when to remove it.
    clip_flash: Option<(&'static str, Instant)>,
    /// When the cursor entering/leaving takes effect (see HOVER_OPEN, HOVER_CLOSE).
    enter_at: Option<Instant>,
    leave_at: Option<Instant>,
    tray: Option<TrayIcon>,
    tray_settings_id: Option<MenuId>,
    tray_quit_id: Option<MenuId>,
    todo: Todo,
    /// Typing a new task. While true the notch stays open and keys go to `input`.
    editing: bool,
    input: String,
    edit_started: Instant,
    /// Index into `todo::REMINDERS` for the task being typed.
    remind: usize,
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
    /// 0.0 = collapsed, 1.0 = expanded
    progress: f64,
    last_tick: Instant,
}

impl App {
    fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        Self {
            window: None,
            surface: None,
            font: load_font(),
            media: MediaState::default(),
            media_at: Instant::now(),
            media_handle: None,
            view: View::Player,
            view_t: 0.0,
            lyrics: LyricsState::None,
            lyrics_key: String::new(),
            focus: FocusTimer::new(),
            water: Water::new(today(), Instant::now()),
            shelf: Shelf::new(),
            icons: HashMap::new(),
            drag_hover: false,
            press: None,
            notice: None,
            notice_until: None,
            ov_t: 0.0,
            clips: History::new(),
            clip_flash: None,
            enter_at: None,
            leave_at: None,
            tray: None,
            tray_settings_id: None,
            tray_quit_id: None,
            todo: Todo::new(),
            editing: false,
            input: String::new(),
            edit_started: Instant::now(),
            remind: 0,
            prev_foreground: None,
            settings: Settings::load(),
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
            last_tick: Instant::now(),
        }
    }

    fn target(&self) -> f64 {
        if self.hovered || self.drag_hover || self.editing || self.alert_until.is_some() { 1.0 } else { 0.0 }
    }

    fn animating(&self) -> bool {
        (self.progress - self.target()).abs() > f64::EPSILON
            || (self.view_t - self.view_target()).abs() > f64::EPSILON
            || (self.ov_t - self.ov_target()).abs() > f64::EPSILON
    }

    fn ov_target(&self) -> f64 {
        if self.notice_until.is_some() { 1.0 } else { 0.0 }
    }

    fn view_target(&self) -> f64 {
        if self.view == View::Lyrics { 1.0 } else { 0.0 }
    }

    /// Playback position, extrapolated since the last update from the media thread.
    fn position(&self) -> Duration {
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
        let dt = now.duration_since(self.last_tick).as_secs_f64();
        self.last_tick = now;

        let delta = dt / ANIM.as_secs_f64();
        let target = self.target();
        self.progress = if self.progress < target {
            (self.progress + delta).min(target)
        } else {
            (self.progress - delta).max(target)
        };

        let ot = self.ov_target();
        self.ov_t = if self.ov_t < ot {
            (self.ov_t + delta).min(ot)
        } else {
            (self.ov_t - delta).max(ot)
        };

        let vt = self.view_target();
        self.view_t = if self.view_t < vt {
            (self.view_t + delta).min(vt)
        } else {
            (self.view_t - delta).max(vt)
        };

        let Some(window) = self.window.clone() else { return };
        let t = ease(self.progress);
        let full_h = lerp(EXPANDED.1, LYRICS_H, ease(self.view_t));
        // The closed notch grows to the compact notice size while a notice is showing.
        let ov = ease(self.ov_t);
        let base = (lerp(COLLAPSED.0, OVERLAY.0, ov), lerp(COLLAPSED.1, OVERLAY.1, ov));
        let w = lerp(base.0, EXPANDED.0, t);
        let h = lerp(base.1, full_h, t);
        let _ = window.request_inner_size(LogicalSize::new(w, h));

        if let Some(monitor) = window.current_monitor().or_else(|| window.primary_monitor()) {
            let scale = window.scale_factor();
            let mpos = monitor.position();
            let msize = monitor.size();
            let pw = w * scale;
            let x = mpos.x as f64 + (msize.width as f64 - pw) / 2.0;
            window.set_outer_position(PhysicalPosition::new(x, mpos.y as f64));
        }
        window.request_redraw();
    }

    /// Index of the lyric line being sung at the current position.
    fn current_line(&self) -> Option<usize> {
        let LyricsState::Found(lines) = &self.lyrics else { return None };
        let pos = self.position();
        lines.iter().rposition(|l| l.at <= pos)
    }

    /// Start dragging a shelf file out to another app. Blocks until the drag ends.
    fn drag_out(&mut self, idx: usize) {
        let Some(path) = self.shelf.items.get(idx).cloned() else { return };
        let _ = dragdrop::drag_file(&path);
        // The file may have been moved by the drop.
        self.shelf.prune();
        // No cursor events arrive during the drag, so work out whether it ended over the notch.
        self.hovered = self.cursor_over_window();
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

    fn click(&mut self) {
        if self.progress < 0.9 {
            return;
        }
        let Some(window) = self.window.clone() else { return };
        let s = window.scale_factor() as f32;
        let (cx, cy) = (self.cursor.0 as f32 / s, self.cursor.1 as f32 / s);
        let in_rect = |r: (f32, f32, f32)| cx >= r.0 && cx <= r.0 + r.2 && cy >= r.1 && cy <= r.1 + r.2;

        if self.view != View::Lyrics {
            for (i, (_, v)) in TABS.iter().enumerate() {
                let r = tab_rect(i);
                if cx >= r.0 && cx <= r.0 + r.2 && cy >= r.1 && cy <= r.1 + r.3 {
                    if self.view != *v {
                        self.stop_editing();
                        self.view = *v;
                        self.last_tick = Instant::now();
                        window.request_redraw();
                    }
                    return;
                }
            }
        }

        match self.view {
            View::Todo => {
                let inside = |r: (f32, f32, f32, f32)| cx >= r.0 && cx <= r.0 + r.2 && cy >= r.1 && cy <= r.1 + r.3;
                if self.editing {
                    if let Some(i) = (0..todo::REMINDERS.len()).find(|&i| inside(todo_chip_rect(i))) {
                        self.remind = i;
                    } else if inside(TODO_ADD) {
                        window.request_redraw();
                        self.commit_task();
                        return;
                    } else {
                        return;
                    }
                } else if inside(TODO_FIELD) {
                    window.request_redraw();
                    self.start_editing();
                    return;
                } else {
                    let row = ((cy - TODO_ROWS_Y) / TODO_ROW_H).floor();
                    let Some(idx) = (row >= 0.0).then(|| self.todo.index_at(row as usize)).flatten() else { return };
                    if cx <= 46.0 {
                        self.todo.toggle(idx);
                    } else if cx >= 372.0 {
                        self.todo.remove(idx);
                    } else {
                        return;
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
            View::Settings => {
                let Some(i) = (0..SETTINGS_ITEMS).find(|&i| {
                    let r = settings_cell(i);
                    cx >= r.0 && cx <= r.0 + r.2 && cy >= r.1 && cy <= r.1 + r.3
                }) else {
                    return;
                };
                match i {
                    0 => {
                        let on = !self.autostart;
                        settings::set_autostart(on);
                        self.autostart = settings::autostart_enabled();
                    }
                    1 => self.settings.hide_fullscreen = !self.settings.hide_fullscreen,
                    2 => self.settings.show_clock = !self.settings.show_clock,
                    3 => self.settings.clock_24h = !self.settings.clock_24h,
                    4 => self.settings.idle_eyes = !self.settings.idle_eyes,
                    5 => self.settings.sound = !self.settings.sound,
                    6 => self.settings.clipboard = !self.settings.clipboard,
                    _ => self.quit = true,
                }
                self.settings.save();
                window.request_redraw();
                self.apply_visibility();
            }
            View::Shelf => {
                let hit = (0..COLS * ROWS).find_map(|slot| {
                    let r = tile_rect(slot);
                    let inside = cx >= r.0 && cx <= r.0 + r.2 && cy >= r.1 && cy <= r.1 + r.3;
                    inside.then(|| self.shelf.index_at(slot).map(|i| (i, r))).flatten()
                });
                let Some((idx, r)) = hit else { return };
                if cx >= r.0 + r.2 - 16.0 && cy <= r.1 + 16.0 {
                    self.shelf.remove(idx);
                    window.request_redraw();
                } else {
                    self.press = Some((idx, self.cursor));
                }
            }
            View::Water => {
                let now = Instant::now();
                let in_label = |x: f32| (cx - x).abs() <= 26.0 && (cy - FOCUS_CTRL_Y).abs() <= 14.0;
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
                } else if (cx - FOCUS_X[1]).hypot(cy - FOCUS_CTRL_Y) <= 18.0 {
                    self.water.add(now);
                } else if in_label(FOCUS_X[0]) {
                    self.water.undo();
                } else if in_label(FOCUS_X[2]) {
                    self.water.reset();
                } else {
                    return;
                }
                self.alert_until = None;
                window.request_redraw();
            }
            View::Focus => {
                let now = Instant::now();
                let in_label = |x: f32| (cx - x).abs() <= 26.0 && (cy - FOCUS_CTRL_Y).abs() <= 14.0;
                let adjust = ADJUST.iter().position(|(x, y)| (cx - x).hypot(cy - y) <= ADJUST_R);
                let chip = (0..focus::Phase::ALL.len()).find(|&i| {
                    let r = chip_rect(i);
                    cx >= r.0 && cx <= r.0 + r.2 && cy >= r.1 && cy <= r.1 + r.3
                });
                if let Some(i) = chip {
                    self.focus.select(focus::Phase::ALL[i]);
                } else if let Some(i) = adjust {
                    // Minus is index 0, plus is index 1.
                    if !self.focus.adjust(if i == 0 { -1 } else { 1 }) {
                        return;
                    }
                } else if (cx - FOCUS_START_X).hypot(cy - FOCUS_CTRL_Y) <= 18.0 {
                    self.focus.toggle(now);
                } else if in_label(FOCUS_RESET_X) {
                    self.focus.reset();
                } else {
                    return;
                }
                self.alert_until = None;
                window.request_redraw();
            }
            View::Player => {
                if in_rect(ART_RECT) {
                    self.view = View::Lyrics;
                    self.last_tick = Instant::now();
                    window.request_redraw();
                    return;
                }
                let Some(handle) = &self.media_handle else { return };
                for (i, x) in CTRL_X.iter().enumerate() {
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
            View::Lyrics => {
                if in_rect(BACK_RECT) {
                    self.view = View::Player;
                    self.last_tick = Instant::now();
                    window.request_redraw();
                    return;
                }
                let LyricsState::Found(lines) = &self.lyrics else { return };
                if cy < LYRIC_AREA.0 || cy > LYRIC_AREA.1 {
                    return;
                }
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
                    window.request_redraw();
                }
            }
        }
    }

    /// Lay out the visible lyric lines around the current one, wrapped, in logical pixels.
    fn layout_lyrics(&self) -> Vec<LyricItem> {
        let (Some(font), LyricsState::Found(lines)) = (&self.font, &self.lyrics) else {
            return Vec::new();
        };
        let cur = self.current_line();
        // Before the first line starts nothing is current; the upcoming lines start below the middle.
        let focus = cur.map_or(-1, |c| c as isize);
        let mid = (LYRIC_AREA.0 + LYRIC_AREA.1) / 2.0;

        let make = |d: isize| -> Option<LyricItem> {
            let idx = usize::try_from(focus + d).ok().filter(|&i| i < lines.len())?;
            let dist = d.unsigned_abs() as u32;
            let px = match dist {
                0 => 18.0,
                1 => 14.0,
                _ => 13.0,
            };
            Some(LyricItem { idx, top: 0.0, lh: px * 1.3, px, dist, rows: wrap(font, &lines[idx].text, px, LYRIC_W) })
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
        for d in 1..=4 {
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

        // Glide to the new line instead of jumping: start where the previous line was centered.
        if let Some(c) = cur.filter(|&c| c > 0) {
            let t = self.position().saturating_sub(lines[c].at).as_secs_f32() / LYRIC_GLIDE;
            if t < 1.0 {
                let center = |it: &LyricItem| it.top + it.rows.len() as f32 * it.lh / 2.0;
                let now_item = items.iter().find(|i| i.idx == c);
                let prev_item = items.iter().find(|i| i.idx == c - 1);
                if let (Some(now_item), Some(prev_item)) = (now_item, prev_item) {
                    let shift = (center(now_item) - center(prev_item)) * (1.0 - ease(t as f64) as f32);
                    for it in &mut items {
                        it.top += shift;
                    }
                }
            }
        }
        items
    }

    /// True while the lyrics are still gliding to a newly sung line, so frames are needed.
    fn lyric_gliding(&self) -> bool {
        let LyricsState::Found(lines) = &self.lyrics else { return false };
        self.view == View::Lyrics
            && self
                .current_line()
                .is_some_and(|c| c > 0 && self.position().saturating_sub(lines[c].at).as_secs_f32() < LYRIC_GLIDE)
    }

    fn draw(&mut self) {
        if self.view == View::Shelf {
            self.load_icons();
        }
        let Some(window) = self.window.clone() else { return };
        let size = window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return;
        };
        let Some(mut pixmap) = Pixmap::new(w.get(), h.get()) else { return };
        pixmap.fill(Color::TRANSPARENT);

        // Black notch: square top edge, rounded bottom corners.
        let s = window.scale_factor() as f32;
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
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color_rgba8(0, 0, 0, 255);
            paint.anti_alias = true;
            pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }

        // Fade content in over the second half of the expand animation.
        let alpha = ((ease(self.progress) - 0.5) * 2.0).clamp(0.0, 1.0) as f32;
        if alpha > 0.0 {
            match self.view {
                View::Player => self.draw_player(&mut pixmap, s, alpha),
                View::Lyrics => self.draw_lyrics(&mut pixmap, s, alpha),
                View::Focus => self.draw_focus(&mut pixmap, s, alpha),
                View::Water => self.draw_water(&mut pixmap, s, alpha),
                View::Shelf => self.draw_shelf(&mut pixmap, s, alpha),
                View::Settings => self.draw_settings(&mut pixmap, s, alpha),
                View::Todo => self.draw_todo(&mut pixmap, s, alpha),
                View::Clipboard => self.draw_clipboard(&mut pixmap, s, alpha),
            }
        }

        // Collapsed: show the countdown while the timer runs.
        let collapsed_alpha = ((1.0 - ease(self.progress) * 2.0).clamp(0.0, 1.0) * (1.0 - ease(self.ov_t))) as f32;
        if collapsed_alpha > 0.0 {
            if let (true, Some(font)) = (self.focus.running(), &self.font) {
                let text = fmt_remaining(self.focus.remaining(Instant::now()));
                let color = [255, 255, 255, (collapsed_alpha * 255.0) as u8];
                draw_text_centered(&mut pixmap, font, &text, fw / 2.0, 21.0 * s, 13.0 * s, color, fw);
            } else {
                self.draw_companion(&mut pixmap, s, collapsed_alpha, fw);
            }
        }

        // System notice (volume, battery...) in the compact notch.
        let notice_alpha = ease(self.ov_t) as f32 * (1.0 - ease(self.progress) as f32 * 2.0).clamp(0.0, 1.0);
        if let (true, Some(notice)) = (notice_alpha > 0.02, &self.notice) {
            self.draw_notice(&mut pixmap, notice, s, notice_alpha, fw);
        }

        let Some(surface) = self.surface.as_mut() else { return };
        surface.resize(w, h).ok();
        let Ok(mut buffer) = surface.buffer_mut() else { return };
        // tiny-skia is premultiplied RGBA; softbuffer wants 0RGB/ARGB u32.
        for (dst, src) in buffer.iter_mut().zip(pixmap.pixels()) {
            *dst = (src.alpha() as u32) << 24
                | (src.red() as u32) << 16
                | (src.green() as u32) << 8
                | src.blue() as u32;
        }
        buffer.present().ok();
    }

    fn draw_art(&self, pm: &mut Pixmap, rect: (f32, f32, f32), s: f32, alpha: f32) {
        let (ax, ay, asz) = rect;
        match &self.media.art {
            Some(rgba) => {
                let n = media::art_size();
                if let Some(mut art) = Pixmap::new(n, n) {
                    for (dst, px) in art.pixels_mut().iter_mut().zip(rgba.chunks_exact(4)) {
                        *dst = tiny_skia::ColorU8::from_rgba(px[0], px[1], px[2], 255).premultiply();
                    }
                    let scale = asz * s / n as f32;
                    let paint = PixmapPaint {
                        opacity: alpha,
                        quality: FilterQuality::Bilinear,
                        ..Default::default()
                    };
                    pm.draw_pixmap(
                        0,
                        0,
                        art.as_ref(),
                        &paint,
                        Transform::from_scale(scale, scale).post_translate(ax * s, ay * s),
                        None,
                    );
                }
            }
            None => fill_rect(pm, ax * s, ay * s, asz * s, asz * s, [40, 40, 44, (alpha * 255.0) as u8]),
        }
    }

    fn draw_lyrics(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let Some(font) = &self.font else { return };
        let a = (alpha * 255.0) as u8;
        let width = EXPANDED.0 as f32 * s;
        let grey = [160, 160, 165, a];

        self.draw_art(pm, BACK_RECT, s, alpha);

        // Header: title and artist, centered.
        let head = if self.media.has_session && !self.media.title.is_empty() {
            format!("{} \u{2014} {}", self.media.title, self.media.artist)
        } else {
            "Nothing playing".to_string()
        };
        draw_text_centered(pm, font, &head, width / 2.0, 30.0 * s, 11.0 * s, grey, width - 120.0 * s);

        if matches!(self.lyrics, LyricsState::Found(_)) {
            for it in self.layout_lyrics() {
                let (rgb, strength) = match it.dist {
                    0 => (255u8, 1.0f32),
                    1 => (175, 0.85),
                    2 => (135, 0.6),
                    _ => (110, 0.45),
                };
                for (i, row) in it.rows.iter().enumerate() {
                    let top = it.top + i as f32 * it.lh;
                    let bottom = top + it.lh;
                    // Fade rows out towards the edges of the lyrics area; skip anything outside it.
                    let edge = (top - LYRIC_AREA.0).min(LYRIC_AREA.1 - bottom);
                    if edge <= 0.0 {
                        continue;
                    }
                    let fade = (edge / 28.0).clamp(0.0, 1.0);
                    let color = [rgb, rgb, rgb, (a as f32 * strength * fade) as u8];
                    let baseline = (top + it.px * 1.05) * s;
                    draw_text_centered(pm, font, row, width / 2.0, baseline, it.px * s, color, (LYRIC_W + 20.0) * s);
                }
            }
        } else {
            let msg = match &self.lyrics {
                LyricsState::Loading => "Searching for lyrics\u{2026}",
                LyricsState::NotFound => "No synced lyrics found",
                _ => "Play something to see lyrics",
            };
            let y = (LYRIC_AREA.0 + LYRIC_AREA.1) / 2.0 + 5.0;
            draw_text_centered(pm, font, msg, width / 2.0, y * s, 13.0 * s, grey, width - 48.0 * s);
        }
    }

    fn draw_tabs(&self, pm: &mut Pixmap, s: f32, alpha: f32, active: View) {
        let a = (alpha * 255.0) as u8;
        for (i, (_, view)) in TABS.iter().enumerate() {
            let (x, y, w, h) = tab_rect(i);
            let on = *view == active;
            if on {
                fill_rounded(pm, x * s, y * s, w * s, h * s, h * s / 2.0, [255, 255, 255, (alpha * 40.0) as u8]);
            }
            let color = if on { [255, 255, 255, a] } else { [150, 150, 155, a] };
            draw_tab_icon(pm, *view, (x + w / 2.0) * s, (y + h / 2.0) * s, s, color);
        }
    }

    fn draw_focus(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let Some(font) = &self.font else { return };
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [160, 160, 165, a]);
        let now = Instant::now();

        self.draw_tabs(pm, s, alpha, View::Focus);

        // Phase picker: jump straight to Focus, Short break or Long break.
        for (i, phase) in focus::Phase::ALL.iter().enumerate() {
            let (x, y, w, h) = chip_rect(i);
            let on = *phase == self.focus.phase;
            if on {
                fill_rounded(pm, x * s, y * s, w * s, h * s, h * s / 2.0, [255, 255, 255, (alpha * 40.0) as u8]);
            }
            let color = if on { white } else { [150, 150, 155, a] };
            draw_text_centered(pm, font, phase.chip_label(), (x + w / 2.0) * s, (y + 13.0) * s, 10.0 * s, color, w * s);
        }

        // Left: progress ring with the countdown inside.
        let accent = match self.focus.phase {
            focus::Phase::Focus => [255, 99, 71],
            focus::Phase::ShortBreak => [80, 200, 120],
            focus::Phase::LongBreak => [90, 150, 255],
        };
        let accent = [accent[0], accent[1], accent[2], a];
        let (rx, ry, rr) = RING;
        stroke_arc(pm, rx * s, ry * s, rr * s, RING_W * s, 1.0, [50, 50, 56, a]);
        stroke_arc(pm, rx * s, ry * s, rr * s, RING_W * s, self.focus.progress(now).clamp(0.0, 1.0), accent);
        let time = fmt_remaining(self.focus.remaining(now));
        draw_text_centered(pm, font, &time, rx * s, (ry + 10.0) * s, 30.0 * s, white, (rr * 2.0 - 8.0) * s);

        // Right: phase, status and controls, centered in the remaining space.
        let cx = FOCUS_X[1] * s;
        draw_text_centered(pm, font, self.focus.phase.label(), cx, 76.0 * s, 18.0 * s, white, 190.0 * s);
        if self.alert_until.is_some() {
            for (i, row) in wrap(font, &self.alert_msg, 12.0, 190.0).iter().take(2).enumerate() {
                draw_text_centered(pm, font, row, cx, (96.0 + i as f32 * 15.0) * s, 12.0 * s, accent, 200.0 * s);
            }
        } else {
            let status = match self.focus.phase {
                focus::Phase::Focus => format!("Session {} of {}", self.focus.completed % CYCLE + 1, CYCLE),
                _ => "Take a breather".to_string(),
            };
            draw_text_centered(pm, font, &status, cx, 96.0 * s, 12.0 * s, grey, 190.0 * s);
            let stats = self.focus.stats_for(today());
            let summary = match stats.sessions {
                0 => "No sessions yet today".to_string(),
                1 => format!("Today: 1 session \u{00B7} {}", focus::fmt_focused(stats.focused)),
                n => format!("Today: {n} sessions \u{00B7} {}", focus::fmt_focused(stats.focused)),
            };
            draw_text_centered(pm, font, &summary, cx, 114.0 * s, 11.0 * s, [130, 130, 136, a], 190.0 * s);
        }

        // Length adjusters, only active while the timer is stopped.
        let idle = !self.focus.running();
        let (bg, fg) = if idle { ([50, 50, 56, a], white) } else { ([32, 32, 36, a], [90, 90, 95, a]) };
        for (i, (x, y)) in ADJUST.iter().enumerate() {
            fill_circle(pm, x * s, y * s, ADJUST_R * s, bg);
            fill_rect(pm, (x - 4.0) * s, (y - 0.75) * s, 8.0 * s, 1.5 * s, fg);
            if i == 1 {
                fill_rect(pm, (x - 0.75) * s, (y - 4.0) * s, 1.5 * s, 8.0 * s, fg);
            }
        }

        let label_y = (FOCUS_CTRL_Y + 4.0) * s;
        draw_text_centered(pm, font, "Reset", FOCUS_RESET_X * s, label_y, 12.0 * s, grey, 60.0 * s);
        fill_circle(pm, FOCUS_START_X * s, FOCUS_CTRL_Y * s, 18.0 * s, accent);
        draw_play_pause(pm, FOCUS_START_X * s, FOCUS_CTRL_Y * s, s * 0.8, [0, 0, 0, a], self.focus.running());
    }

    fn draw_water(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let Some(font) = &self.font else { return };
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [160, 160, 165, a]);
        let accent = [64, 170, 255, a];
        let now = Instant::now();

        self.draw_tabs(pm, s, alpha, View::Water);

        // Reminder interval picker.
        for (i, interval) in water::INTERVALS.iter().enumerate() {
            let (x, y, w, h) = water_chip_rect(i);
            let on = i == self.water.interval;
            if on {
                fill_rounded(pm, x * s, y * s, w * s, h * s, h * s / 2.0, [255, 255, 255, (alpha * 40.0) as u8]);
            }
            let label = match interval {
                Some(m) if m % 60 == 0 => format!("{}h", m / 60),
                Some(m) => format!("{m}m"),
                None => "Off".to_string(),
            };
            let color = if on { white } else { [150, 150, 155, a] };
            draw_text_centered(pm, font, &label, (x + w / 2.0) * s, (y + 13.0) * s, 10.0 * s, color, w * s);
        }

        // Left: progress ring with today's count.
        let (rx, ry, rr) = RING;
        stroke_arc(pm, rx * s, ry * s, rr * s, RING_W * s, 1.0, [50, 50, 56, a]);
        stroke_arc(pm, rx * s, ry * s, rr * s, RING_W * s, self.water.progress(), accent);
        let count = format!("{}/{}", self.water.count, self.water.goal);
        draw_text_centered(pm, font, &count, rx * s, (ry + 6.0) * s, 28.0 * s, white, (rr * 2.0 - 8.0) * s);
        draw_text_centered(pm, font, "glasses", rx * s, (ry + 24.0) * s, 10.0 * s, grey, (rr * 2.0 - 8.0) * s);

        // Right: title with goal adjusters, and status.
        let cx = FOCUS_X[1] * s;
        draw_text_centered(pm, font, "Water", cx, 76.0 * s, 18.0 * s, white, 190.0 * s);
        for (i, (x, y)) in ADJUST.iter().enumerate() {
            fill_circle(pm, x * s, y * s, ADJUST_R * s, [50, 50, 56, a]);
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

        let label_y = (FOCUS_CTRL_Y + 4.0) * s;
        draw_text_centered(pm, font, "Undo", FOCUS_X[0] * s, label_y, 12.0 * s, grey, 60.0 * s);
        draw_text_centered(pm, font, "Reset", FOCUS_X[2] * s, label_y, 12.0 * s, grey, 60.0 * s);
        fill_circle(pm, cx, FOCUS_CTRL_Y * s, 18.0 * s, accent);
        let black = [0, 0, 0, a];
        fill_rect(pm, cx - 7.0 * s, (FOCUS_CTRL_Y - 1.25) * s, 14.0 * s, 2.5 * s, black);
        fill_rect(pm, cx - 1.25 * s, (FOCUS_CTRL_Y - 7.0) * s, 2.5 * s, 14.0 * s, black);
    }

    fn draw_shelf(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let Some(font) = &self.font else { return };
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [160, 160, 165, a]);
        let width = EXPANDED.0 as f32 * s;

        self.draw_tabs(pm, s, alpha, View::Shelf);

        let empty = self.shelf.items.is_empty();
        if self.drag_hover {
            fill_rounded(pm, 24.0 * s, 34.0 * s, 372.0 * s, 128.0 * s, 14.0 * s, [64, 170, 255, (alpha * 45.0) as u8]);
        } else if empty {
            fill_rounded(pm, 24.0 * s, 34.0 * s, 372.0 * s, 128.0 * s, 14.0 * s, [255, 255, 255, (alpha * 14.0) as u8]);
        }

        if empty {
            let title = if self.drag_hover { "Release to add" } else { "Drop files here" };
            draw_text_centered(pm, font, title, width / 2.0, 94.0 * s, 15.0 * s, white, width - 80.0 * s);
            let hint = "They stay until you drag them out or remove them";
            draw_text_centered(pm, font, hint, width / 2.0, 114.0 * s, 11.0 * s, grey, width - 80.0 * s);
            return;
        }

        const PALETTE: [[u8; 3]; 6] =
            [[255, 99, 71], [80, 200, 120], [90, 150, 255], [255, 190, 70], [190, 120, 255], [70, 200, 210]];
        for slot in 0..COLS * ROWS {
            let Some(i) = self.shelf.index_at(slot) else { continue };
            let path = &self.shelf.items[i];
            let (x, y, w, h) = tile_rect(slot);
            fill_rounded(pm, x * s, y * s, w * s, h * s, 8.0 * s, [38, 38, 44, a]);

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
                fill_rounded(pm, (x + (w - 34.0) / 2.0) * s, (y + 8.0) * s, 34.0 * s, 18.0 * s, 5.0 * s, [c[0], c[1], c[2], a]);
                draw_text_centered(pm, font, &badge, (x + w / 2.0) * s, (y + 21.0) * s, 10.0 * s, [0, 0, 0, a], 32.0 * s);
            }

            draw_text_centered(pm, font, &shelf::file_name(path), (x + w / 2.0) * s, (y + h - 10.0) * s, 10.0 * s, white, (w - 8.0) * s);
            // Remove button (takes the file off the shelf, never deletes it).
            draw_text_centered(pm, font, "\u{00D7}", (x + w - 8.0) * s, (y + 13.0) * s, 13.0 * s, grey, 12.0 * s);
        }

        // Scroll indicator when there are more rows than fit.
        let total_rows = self.shelf.items.len().div_ceil(COLS);
        if total_rows > ROWS {
            let (top, height) = (TILES_ORIGIN.1, 122.0);
            fill_rounded(pm, 406.0 * s, top * s, 3.0 * s, height * s, 1.5 * s, [50, 50, 56, a]);
            let thumb = height * ROWS as f32 / total_rows as f32;
            let at = top + (height - thumb) * self.shelf.scroll as f32 / (total_rows - ROWS) as f32;
            fill_rounded(pm, 406.0 * s, at * s, 3.0 * s, thumb * s, 1.5 * s, grey);
        }
    }

    fn draw_notice(&self, pm: &mut Pixmap, notice: &sys::Notice, s: f32, alpha: f32, fw: f32) {
        use sys::{BatteryKind, Notice};
        let Some(font) = &self.font else { return };
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [160, 160, 165, a]);
        let (green, red, blue) = ([80, 200, 120, a], [255, 90, 80, a], [64, 170, 255, a]);

        // Content block, centered in the notch.
        let (left, right, cy) = (fw / 2.0 - 120.0 * s, fw / 2.0 + 120.0 * s, 27.0 * s);
        let icon_x = left + 10.0 * s;
        let text_x = left + 34.0 * s;
        let right_text = |pm: &mut Pixmap, text: &str, color: [u8; 4]| {
            let w = text_width(font, text, 13.0 * s);
            draw_text(pm, font, text, right - w, cy + 5.0 * s, 13.0 * s, color, 120.0 * s);
        };

        match notice {
            Notice::Volume { percent, muted } => {
                draw_speaker(pm, icon_x, cy, s, if *muted { grey } else { white }, *muted);
                draw_bar(pm, text_x, cy, 140.0 * s, if *muted { 0.0 } else { *percent as f32 / 100.0 }, white, s, a);
                right_text(pm, &if *muted { "Muted".to_string() } else { format!("{percent}%") }, if *muted { grey } else { white });
            }
            Notice::Brightness { percent } => {
                draw_sun(pm, icon_x, cy, s, white);
                draw_bar(pm, text_x, cy, 140.0 * s, *percent as f32 / 100.0, white, s, a);
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
        let open = MenuItem::new("Open Settings", true, None);
        let quit = MenuItem::new("Quit Sky Island", true, None);
        if menu.append_items(&[&open, &quit]).is_err() {
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

    /// Begin typing a task: remember which app had focus, then take it so key presses reach us.
    fn start_editing(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
        let fg = unsafe { GetForegroundWindow() };
        self.prev_foreground = (!fg.is_invalid()).then_some(fg);
        self.editing = true;
        self.input.clear();
        self.remind = 0;
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

    fn play_sound(&self, chime: sound::Chime) {
        if self.settings.sound {
            sound::play(chime);
        }
    }

    fn commit_task(&mut self) {
        self.todo.add(&self.input, self.remind, todo::unix_now());
        self.stop_editing();
    }

    /// Hide the window while a full-screen app is in front, if that setting is on.
    fn apply_visibility(&mut self) {
        let hide = self.fullscreen && self.settings.hide_fullscreen;
        if hide != self.hidden {
            self.hidden = hide;
            if let Some(w) = &self.window {
                w.set_visible(!hide);
            }
        }
    }

    /// Schedule and trigger the redraws the closed notch needs: clock minute changes, blinking
    /// eyes, dozing animation and the greeting. Does nothing when something else is showing.
    fn companion_tick(&mut self, now: Instant, wake: &mut Option<Instant>) {
        if self.hidden || self.progress > 0.0 || self.notice_until.is_some() || self.focus.running() {
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
            Presence::Idle if self.settings.idle_eyes => {
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
    fn draw_companion(&self, pm: &mut Pixmap, s: f32, alpha: f32, fw: f32) {
        let Some(font) = &self.font else { return };
        let a = (alpha * 255.0) as u8;
        let white = [255, 255, 255, a];
        let (cx, cy) = (fw / 2.0, 16.0 * s);
        let (left, right) = (cx - 13.0 * s, cx + 13.0 * s);
        let eyes = self.settings.idle_eyes;

        if eyes && self.greeting_until.is_some() {
            for x in [left, right] {
                stroke_arc_from(pm, x, cy + 3.0 * s, 5.0 * s, 2.0 * s, -std::f32::consts::PI + 0.35, std::f32::consts::PI - 0.7, white);
            }
            draw_text(pm, font, "hi!", cx + 28.0 * s, cy + 4.0 * s, 11.0 * s, white, 30.0 * s);
            return;
        }

        match self.presence {
            Presence::Idle if eyes => {
                let blink = self.blink_until.is_some();
                let glance = (self.blink_count % 3) as f32 - 1.0;
                for x in [left, right] {
                    let h = if blink { 1.6 } else { 12.0 } * s;
                    if let Some(rect) = Rect::from_xywh(x - 4.5 * s, cy - h / 2.0, 9.0 * s, h)
                        && let Some(path) = PathBuilder::from_oval(rect) {
                            let mut paint = Paint::default();
                            paint.set_color_rgba8(white[0], white[1], white[2], a);
                            paint.anti_alias = true;
                            pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
                        }
                    if !blink {
                        fill_circle(pm, x + glance * 1.5 * s, cy + 1.0 * s, 3.0 * s, [0, 0, 0, a]);
                    }
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
                draw_text_centered(pm, font, &text, cx, 21.0 * s, 13.0 * s, [210, 210, 215, a], fw);
            }
            _ => {}
        }
    }

    fn draw_settings(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let Some(font) = &self.font else { return };
        let a = (alpha * 255.0) as u8;
        let white = [255, 255, 255, a];

        self.draw_tabs(pm, s, alpha, View::Settings);

        let toggles = [
            ("Start with Windows", self.autostart),
            ("Hide in fullscreen", self.settings.hide_fullscreen),
            ("Show clock", self.settings.show_clock),
            ("24-hour clock", self.settings.clock_24h),
            ("Idle eyes", self.settings.idle_eyes),
            ("Sounds", self.settings.sound),
            ("Clipboard history", self.settings.clipboard),
        ];
        for (i, (label, on)) in toggles.iter().enumerate() {
            let (x, y, w, h) = settings_cell(i);
            draw_text(pm, font, label, (x + 4.0) * s, (y + h / 2.0 + 4.0) * s, 12.0 * s, white, (w - 48.0) * s);
            draw_switch(pm, (x + w - 34.0) * s, (y + h / 2.0 - 8.0) * s, s, *on, a);
        }

        let (x, y, w, h) = settings_cell(7);
        fill_rounded(pm, x * s, (y + 2.0) * s, w * s, (h - 4.0) * s, 8.0 * s, [255, 90, 80, (alpha * 36.0) as u8]);
        draw_text_centered(pm, font, "Quit Sky Island", (x + w / 2.0) * s, (y + h / 2.0 + 4.0) * s, 12.0 * s, [255, 120, 110, a], w * s);
    }

    fn draw_todo(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let Some(font) = &self.font else { return };
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [160, 160, 165, a]);
        let amber = [TODO_AMBER[0], TODO_AMBER[1], TODO_AMBER[2], a];
        let width = EXPANDED.0 as f32 * s;
        let now = todo::unix_now();

        self.draw_tabs(pm, s, alpha, View::Todo);

        // Input field (also shows a reminder banner when one fires).
        let (fx, fy, fw, fh) = TODO_FIELD;
        let fill = if self.editing { 30.0 } else { 16.0 };
        fill_rounded(pm, fx * s, fy * s, fw * s, fh * s, 8.0 * s, [255, 255, 255, (alpha * fill) as u8]);
        let (tx, ty, max_w) = ((fx + 10.0) * s, (fy + 16.0) * s, (fw - 20.0) * s);
        if self.alert_until.is_some() {
            draw_text(pm, font, &self.alert_msg, tx, ty, 12.0 * s, amber, max_w);
        } else if self.editing {
            let shown = tail_fit(font, &self.input, 12.0 * s, max_w - 8.0 * s);
            if shown.is_empty() {
                draw_text(pm, font, "Type a task, then press Enter", tx, ty, 12.0 * s, grey, max_w);
            } else {
                draw_text(pm, font, &shown, tx, ty, 12.0 * s, white, max_w);
            }
            // Blinking caret after the text.
            if (self.edit_started.elapsed().as_millis() / 500).is_multiple_of(2) {
                let x = tx + text_width(font, &shown, 12.0 * s) + 1.0 * s;
                fill_rect(pm, x, (fy + 5.0) * s, 1.5 * s, 14.0 * s, white);
            }
        } else {
            draw_text(pm, font, "+  Add a task", tx, ty, 12.0 * s, grey, max_w);
        }

        if self.editing {
            draw_text(pm, font, "Remind", 24.0 * s, 80.0 * s, 11.0 * s, grey, 50.0 * s);
            for (i, (label, _)) in todo::REMINDERS.iter().enumerate() {
                let (x, y, w, h) = todo_chip_rect(i);
                let on = i == self.remind;
                if on {
                    fill_rounded(pm, x * s, y * s, w * s, h * s, h * s / 2.0, [255, 255, 255, (alpha * 40.0) as u8]);
                }
                let color = if on { white } else { [150, 150, 155, a] };
                draw_text_centered(pm, font, label, (x + w / 2.0) * s, (y + 13.0) * s, 10.0 * s, color, w * s);
            }
            let (x, y, w, h) = TODO_ADD;
            fill_rounded(pm, x * s, y * s, w * s, h * s, h * s / 2.0, amber);
            draw_text_centered(pm, font, "Add", (x + w / 2.0) * s, (y + 13.0) * s, 11.0 * s, [0, 0, 0, a], w * s);
            draw_text_centered(pm, font, "Enter to add  \u{00B7}  Esc to cancel", width / 2.0, 122.0 * s, 10.0 * s, grey, width - 48.0 * s);
            return;
        }

        if self.todo.tasks.is_empty() {
            draw_text_centered(pm, font, "Nothing to do", width / 2.0, 104.0 * s, 15.0 * s, white, width - 80.0 * s);
            draw_text_centered(pm, font, "Click the box above to add a task", width / 2.0, 124.0 * s, 11.0 * s, grey, width - 80.0 * s);
            return;
        }

        for row in 0..todo::VISIBLE {
            let Some(i) = self.todo.index_at(row) else { continue };
            let task = &self.todo.tasks[i];
            let y = TODO_ROWS_Y + row as f32 * TODO_ROW_H;

            // Checkbox.
            let (bx, by) = (28.0 * s, (y + 5.0) * s);
            if task.done {
                fill_rounded(pm, bx, by, 14.0 * s, 14.0 * s, 4.0 * s, amber);
                stroke_line(pm, bx + 3.5 * s, by + 7.5 * s, bx + 6.0 * s, by + 10.0 * s, 1.8 * s, [0, 0, 0, a]);
                stroke_line(pm, bx + 6.0 * s, by + 10.0 * s, bx + 10.5 * s, by + 4.5 * s, 1.8 * s, [0, 0, 0, a]);
            } else {
                fill_rounded(pm, bx, by, 14.0 * s, 14.0 * s, 4.0 * s, [110, 110, 116, a]);
                fill_rounded(pm, bx + 1.5 * s, by + 1.5 * s, 11.0 * s, 11.0 * s, 3.0 * s, [0, 0, 0, 255]);
            }

            // Text, struck through when done.
            let color = if task.done { [120, 120, 125, a] } else { white };
            let text_w = 250.0 * s;
            draw_text(pm, font, &task.text, 52.0 * s, (y + 16.0) * s, 12.0 * s, color, text_w);
            if task.done {
                let w = text_width(font, &task.text, 12.0 * s).min(text_w);
                fill_rect(pm, 52.0 * s, (y + 12.0) * s, w, 1.2 * s, color);
            }

            // Reminder time, then the remove button.
            if let (Some(due), false) = (task.due, task.done) {
                let label = todo::due_label(due, now);
                let c = if label == "overdue" { [255, 110, 100, a] } else { grey };
                let w = text_width(font, &label, 10.0 * s);
                draw_text(pm, font, &label, 366.0 * s - w, (y + 16.0) * s, 10.0 * s, c, 80.0 * s);
            }
            draw_text_centered(pm, font, "\u{00D7}", 384.0 * s, (y + 17.0) * s, 13.0 * s, grey, 14.0 * s);
        }

        // Scroll indicator when there are more tasks than rows.
        let total = self.todo.tasks.len();
        if total > todo::VISIBLE {
            let (top, height) = (TODO_ROWS_Y, TODO_ROW_H * todo::VISIBLE as f32);
            fill_rounded(pm, 410.0 * s, top * s, 3.0 * s, height * s, 1.5 * s, [50, 50, 56, a]);
            let thumb = height * todo::VISIBLE as f32 / total as f32;
            let at = top + (height - thumb) * self.todo.scroll as f32 / (total - todo::VISIBLE) as f32;
            fill_rounded(pm, 410.0 * s, at * s, 3.0 * s, thumb * s, 1.5 * s, grey);
        }
    }

    fn draw_clipboard(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let Some(font) = &self.font else { return };
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [160, 160, 165, a]);
        let teal = [CLIP_TEAL[0], CLIP_TEAL[1], CLIP_TEAL[2], a];
        let width = EXPANDED.0 as f32 * s;

        self.draw_tabs(pm, s, alpha, View::Clipboard);

        // Header: what the list holds, or a short message after copying or clearing.
        let count = self.clips.items.len();
        let (title, color) = match self.clip_flash {
            Some((msg, _)) => (msg.to_string(), teal),
            None if !self.settings.clipboard => ("Clipboard history is off".to_string(), grey),
            None => (format!("Clipboard \u{00B7} {count} {}", if count == 1 { "item" } else { "items" }), grey),
        };
        draw_text(pm, font, &title, 24.0 * s, 51.0 * s, 12.0 * s, color, 280.0 * s);

        // Clear button: empties the history and the Windows clipboard.
        let (x, y, w, h) = CLIP_CLEAR;
        fill_rounded(pm, x * s, y * s, w * s, h * s, h * s / 2.0, [255, 90, 80, (alpha * 36.0) as u8]);
        draw_text_centered(pm, font, "Clear", (x + w / 2.0) * s, (y + 15.0) * s, 12.0 * s, [255, 120, 110, a], w * s);

        if !self.settings.clipboard || count == 0 {
            let (title, hint) = if self.settings.clipboard {
                ("Nothing copied yet", "Text and pictures you copy will show up here")
            } else {
                ("Clipboard history is off", "Turn it on in Settings")
            };
            draw_text_centered(pm, font, title, width / 2.0, 106.0 * s, 15.0 * s, white, width - 80.0 * s);
            draw_text_centered(pm, font, hint, width / 2.0, 126.0 * s, 11.0 * s, grey, width - 80.0 * s);
            return;
        }

        for row in 0..clipboard::VISIBLE {
            let Some(i) = self.clips.index_at(row) else { continue };
            let y = CLIP_ROWS_Y + row as f32 * CLIP_ROW_H;
            fill_rounded(pm, 24.0 * s, (y + 1.0) * s, 372.0 * s, (CLIP_ROW_H - 2.0) * s, 6.0 * s, [255, 255, 255, (alpha * 14.0) as u8]);
            match &self.clips.items[i] {
                clipboard::Content::Text(text) => {
                    let baseline = (y + CLIP_ROW_H / 2.0 + 4.0) * s;
                    draw_text(pm, font, &clipboard::preview(text), 32.0 * s, baseline, 12.0 * s, white, 328.0 * s);
                }
                clipboard::Content::Image(img) => {
                    // The preview is stored at twice its drawn size, so it stays sharp on high-DPI screens.
                    if let Some(mut thumb) = Pixmap::new(img.thumb_w, img.thumb_h) {
                        for (dst, px) in thumb.pixels_mut().iter_mut().zip(img.thumb.chunks_exact(4)) {
                            *dst = tiny_skia::ColorU8::from_rgba(px[0], px[1], px[2], px[3]).premultiply();
                        }
                        let (dw, dh) = (img.thumb_w as f32 / 2.0, img.thumb_h as f32 / 2.0);
                        let paint = PixmapPaint { opacity: alpha, quality: FilterQuality::Bilinear, ..Default::default() };
                        let at = Transform::from_scale(s / 2.0, s / 2.0)
                            .post_translate((32.0 + (48.0 - dw) / 2.0) * s, (y + (CLIP_ROW_H - dh) / 2.0) * s);
                        pm.draw_pixmap(0, 0, thumb.as_ref(), &paint, at, None);
                    }
                    draw_text(pm, font, "Image", 92.0 * s, (y + 14.0) * s, 12.0 * s, white, 200.0 * s);
                    let size = format!("{} \u{00D7} {}", img.width, img.height);
                    draw_text(pm, font, &size, 92.0 * s, (y + 27.0) * s, 10.0 * s, grey, 200.0 * s);
                }
            }
            draw_text_centered(pm, font, "\u{00D7}", 384.0 * s, (y + CLIP_ROW_H / 2.0 + 5.0) * s, 13.0 * s, grey, 14.0 * s);
        }

        // Scroll indicator when there are more entries than rows.
        if count > clipboard::VISIBLE {
            let (top, height) = (CLIP_ROWS_Y, CLIP_ROW_H * clipboard::VISIBLE as f32);
            fill_rounded(pm, 410.0 * s, top * s, 3.0 * s, height * s, 1.5 * s, [50, 50, 56, a]);
            let thumb = height * clipboard::VISIBLE as f32 / count as f32;
            let at = top + (height - thumb) * self.clips.scroll as f32 / (count - clipboard::VISIBLE) as f32;
            fill_rounded(pm, 410.0 * s, at * s, 3.0 * s, thumb * s, 1.5 * s, grey);
        }
    }

    fn draw_player(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let a = (alpha * 255.0) as u8;
        self.draw_tabs(pm, s, alpha, View::Player);
        let white = [255, 255, 255, a];
        let grey = [160, 160, 165, a];

        let (title, artist) = if self.media.has_session && !self.media.title.is_empty() {
            (self.media.title.as_str(), self.media.artist.as_str())
        } else {
            ("Nothing playing", "Play something to see it here")
        };

        self.draw_art(pm, ART_RECT, s, alpha);

        if let Some(font) = &self.font {
            let max_w = (EXPANDED.0 as f32 - 24.0 - TEXT_X) * s;
            draw_text(pm, font, title, TEXT_X * s, 56.0 * s, 16.0 * s, white, max_w);
            draw_text(pm, font, artist, TEXT_X * s, 78.0 * s, 13.0 * s, grey, max_w);

            let pos = fmt_time(self.position());
            let dur = fmt_time(self.media.duration);
            draw_text(pm, font, &pos, BAR.0 * s, 124.0 * s, 10.0 * s, grey, 60.0 * s);
            let dur_w = text_width(font, &dur, 10.0 * s);
            draw_text(pm, font, &dur, (BAR.0 + BAR.2) * s - dur_w, 124.0 * s, 10.0 * s, grey, 60.0 * s);
        }

        // Progress bar.
        fill_rect(pm, BAR.0 * s, BAR.1 * s, BAR.2 * s, 4.0 * s, [60, 60, 66, a]);
        let frac = if self.media.duration.is_zero() {
            0.0
        } else {
            (self.position().as_secs_f32() / self.media.duration.as_secs_f32()).clamp(0.0, 1.0)
        };
        fill_rect(pm, BAR.0 * s, BAR.1 * s, BAR.2 * s * frac, 4.0 * s, white);

        // Transport controls.
        let (px, py) = (CTRL_X[0] * s, CTRL_Y * s);
        let dim = [90, 90, 95, a];
        let next_color = if self.media.next_enabled { white } else { dim };
        draw_prev(pm, px, py, s, white);
        draw_play_pause(pm, CTRL_X[1] * s, py, s, white, self.media.playing);
        draw_next(pm, CTRL_X[2] * s, py, s, next_color);
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

fn text_width(font: &Font, text: &str, px: f32) -> f32 {
    text.chars().map(|c| font.metrics(c, px).advance_width).sum()
}

/// Draw `text` with its baseline at `y`, truncating with an ellipsis past `max_w`.
fn draw_text(pm: &mut Pixmap, font: &Font, text: &str, x: f32, y: f32, px: f32, rgba: [u8; 4], max_w: f32) {
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

/// Phase picker chip, centered over the right-hand column of the focus view.
fn chip_rect(i: usize) -> (f32, f32, f32, f32) {
    let n = focus::Phase::ALL.len() as f32;
    let total = n * CHIP_W + (n - 1.0) * CHIP_GAP;
    (FOCUS_X[1] - total / 2.0 + i as f32 * (CHIP_W + CHIP_GAP), 34.0, CHIP_W, 18.0)
}

/// Reminder interval chip, centered over the right-hand column of the water view.
fn water_chip_rect(i: usize) -> (f32, f32, f32, f32) {
    let n = water::INTERVALS.len() as f32;
    let total = n * WATER_CHIP_W + (n - 1.0) * CHIP_GAP;
    (FOCUS_X[1] - total / 2.0 + i as f32 * (WATER_CHIP_W + CHIP_GAP), 34.0, WATER_CHIP_W, 18.0)
}

/// Today's local date as yyyymmdd.
fn today() -> u32 {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe { GetLocalTime() };
    t.wYear as u32 * 10000 + t.wMonth as u32 * 100 + t.wDay as u32
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

/// Settings cell: 0-3 left column, 4-6 right column, 7 is the Quit button (right column, last row).
fn settings_cell(i: usize) -> (f32, f32, f32, f32) {
    let (col, row) = match i {
        0..=3 => (0, i),
        4..=6 => (1, i - 4),
        _ => (1, 3), // the Quit button
    };
    (24.0 + col as f32 * 190.0, 36.0 + row as f32 * 32.0, 182.0, 28.0)
}

fn draw_switch(pm: &mut Pixmap, x: f32, y: f32, s: f32, on: bool, a: u8) {
    let track = if on { [80, 200, 120, a] } else { [60, 60, 66, a] };
    fill_rounded(pm, x, y, 30.0 * s, 16.0 * s, 8.0 * s, track);
    let knob_x = if on { x + 22.0 * s } else { x + 8.0 * s };
    fill_circle(pm, knob_x, y + 8.0 * s, 6.0 * s, [255, 255, 255, a]);
}

const TODO_ADD: (f32, f32, f32, f32) = (332.0, 68.0, 64.0, 18.0);

/// Reminder choice chip shown while typing a task.
fn todo_chip_rect(i: usize) -> (f32, f32, f32, f32) {
    (76.0 + i as f32 * 48.0, 68.0, 44.0, 18.0)
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

fn tab_rect(i: usize) -> (f32, f32, f32, f32) {
    let total = TABS.len() as f32 * TAB_W + (TABS.len() as f32 - 1.0) * TAB_GAP;
    let x0 = (EXPANDED.0 as f32 - total) / 2.0;
    (x0 + i as f32 * (TAB_W + TAB_GAP), 6.0, TAB_W, 20.0)
}

/// mm:ss, rounded up so the display hits 0:00 exactly when the phase ends.
fn fmt_remaining(d: Duration) -> String {
    let secs = d.as_secs() + (d.subsec_nanos() > 0) as u64;
    format!("{:02}:{:02}", secs / 60, secs % 60)
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
        paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
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
        paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
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
        paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        paint.anti_alias = true;
        let stroke = Stroke { width, line_cap: LineCap::Round, ..Default::default() };
        pm.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

/// Horizontal level bar centered on `cy`.
fn draw_bar(pm: &mut Pixmap, x: f32, cy: f32, w: f32, frac: f32, fill: [u8; 4], s: f32, a: u8) {
    let h = 6.0 * s;
    fill_rounded(pm, x, cy - h / 2.0, w, h, h / 2.0, [60, 60, 66, a]);
    if frac > 0.0 {
        fill_rounded(pm, x, cy - h / 2.0, (w * frac.clamp(0.0, 1.0)).max(h), h, h / 2.0, fill);
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
        paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
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
        View::Focus => {
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
                paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
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
        View::Lyrics => {}
        View::Settings => {
            // Three sliders.
            for (dy, knob) in [(-4.5, -2.5), (0.0, 2.5), (4.5, -1.0)] {
                stroke_line(pm, cx - 6.0 * s, cy + dy * s, cx + 6.0 * s, cy + dy * s, 1.3 * s, c);
                fill_circle(pm, cx + knob * s, cy + dy * s, 2.1 * s, c);
            }
        }
    }
}

fn fill_circle(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, c: [u8; 4]) {
    if let Some(path) = PathBuilder::from_circle(cx, cy, r) {
        let mut paint = Paint::default();
        paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
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
        paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        paint.anti_alias = true;
        pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }
}

fn fill_rect(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, c: [u8; 4]) {
    let Some(rect) = Rect::from_xywh(x, y, w.max(0.01), h) else { return };
    let mut paint = Paint::default();
    paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
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
        paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        paint.anti_alias = true;
        pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }
}

fn draw_play_pause(pm: &mut Pixmap, cx: f32, cy: f32, s: f32, c: [u8; 4], playing: bool) {
    if playing {
        fill_rect(pm, cx - 7.0 * s, cy - 9.0 * s, 5.0 * s, 18.0 * s, c);
        fill_rect(pm, cx + 2.0 * s, cy - 9.0 * s, 5.0 * s, 18.0 * s, c);
    } else {
        fill_path(pm, &[(cx - 6.0 * s, cy - 10.0 * s), (cx + 9.0 * s, cy), (cx - 6.0 * s, cy + 10.0 * s)], c);
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
    format!("{}:{:02}", t / 60, t % 60)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn ease(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Sky Island")
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_skip_taskbar(true)
            .with_inner_size(LogicalSize::new(COLLAPSED.0, COLLAPSED.1));
        let window = Rc::new(event_loop.create_window(attrs).expect("create window"));
        let context = Context::new(window.clone()).expect("softbuffer context");
        self.surface = Some(Surface::new(&context, window.clone()).expect("softbuffer surface"));
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
                if self.settings.clipboard && self.clips.add(content) && self.view == View::Clipboard && self.progress > 0.0 {
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
            }
            UserEvent::Menu(id) => {
                let now = Instant::now();
                if Some(&id) == self.tray_quit_id.as_ref() {
                    self.quit = true;
                } else if Some(&id) == self.tray_settings_id.as_ref() {
                    // Open the notch on the settings tab for a few seconds.
                    self.view = View::Settings;
                    self.alert_until = Some(now + ALERT * 2);
                    self.last_tick = now;
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
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
                    lyrics::spawn_fetch(key, state.title.clone(), state.artist.clone(), state.duration, self.proxy.clone());
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

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::CursorEntered { .. } => {
                let now = Instant::now();
                self.leave_at = None;
                if !self.hovered {
                    self.enter_at = Some(now + HOVER_OPEN);
                    event_loop.set_control_flow(ControlFlow::WaitUntil(now + HOVER_OPEN));
                }
            }
            WindowEvent::CursorLeft { .. } => {
                let now = Instant::now();
                self.enter_at = None; // a fly-by never opened it
                if self.hovered {
                    self.leave_at = Some(now + HOVER_CLOSE);
                    event_loop.set_control_flow(ControlFlow::WaitUntil(now + HOVER_CLOSE));
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x, position.y);
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
                // A press that never turned into a drag is a click: open the file.
                if let Some((idx, _)) = self.press.take()
                    && let Some(path) = self.shelf.items.get(idx) {
                        let _ = std::process::Command::new("explorer").arg(path).spawn();
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

        if let Some(finished) = self.focus.tick(now) {
            self.alert_msg = match finished {
                focus::Phase::Focus => "Focus complete. Time for a break!".into(),
                _ => "Break over. Ready to focus?".into(),
            };
            self.alert_until = Some(now + ALERT);
            self.view = View::Focus;
            self.last_tick = now;
            self.play_sound(match finished {
                focus::Phase::Focus => sound::Chime::FocusDone,
                _ => sound::Chime::BreakOver,
            });
        }
        if let Some(text) = self.todo.take_due(todo::unix_now()) {
            let short: String = text.chars().take(40).collect();
            self.alert_msg = format!("Reminder: {short}");
            self.alert_until = Some(now + ALERT);
            self.view = View::Todo;
            self.last_tick = now;
            self.play_sound(sound::Chime::Reminder);
        }
        self.focus.roll(today());
        self.water.roll(today());
        if self.water.due(now) {
            self.alert_msg = "Time to drink some water!".into();
            self.alert_until = Some(now + ALERT);
            self.view = View::Water;
            self.last_tick = now;
            self.play_sound(sound::Chime::Reminder);
        }
        if self.alert_until.is_some_and(|t| now >= t) {
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
        if (expanded && (self.media.playing || self.editing || matches!(self.view, View::Water | View::Todo)))
            || self.focus.running()
        {
            let tick = if expanded && self.media.playing && self.lyric_gliding() { FRAME * 2 } else { PROGRESS_TICK };
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
        if let Some(d) = self.focus.deadline(now) {
            at(d);
        }
        if let Some(d) = self.water.deadline() {
            at(d);
        }
        if let Some(due) = self.todo.next_due() {
            at(now + Duration::from_secs(due.saturating_sub(todo::unix_now())));
        }
        if let Some(t) = self.alert_until {
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

    /// The whole tab bar has to fit in the notch.
    #[test]
    fn tab_bar_fits_the_notch() {
        let total = TABS.len() as f32 * TAB_W + (TABS.len() as f32 - 1.0) * TAB_GAP;
        assert!(total <= EXPANDED.0 as f32 - 8.0, "tab bar is {total}px wide");
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
