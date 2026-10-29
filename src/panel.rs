//! The Sky Island settings window: an ordinary Windows window holding every setting, including the
//! ones that do not fit in the notch (look, animation, timings, volume, the timer alarm).
//!
//! It is drawn with the same tools as the notch, so it moves the same way: switches slide, the
//! choice highlight glides between options, hovered rows light up. Everything applies at once and is
//! saved as it changes.

use std::rc::Rc;
use std::time::{Duration, Instant};

use softbuffer::{Context, Surface};
use tiny_skia::Pixmap;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, NamedKey};
use winit::window::{Icon, Window, WindowButtons};

use crate::motion::Glows;
use crate::settings::{self, Motion, Range, Settings, ThemeMode};
use crate::{App, View, appicon, gfx, theme};

const WIDTH: f64 = 660.0;
const HEIGHT: f64 = 548.0;
const ROW_H: f32 = 34.0;
const HEAD_H: f32 = 34.0;
const SLIDER_W: f32 = 112.0;
/// How long "Reset to defaults" waits for a second click.
const RESET_WINDOW: Duration = Duration::from_secs(3);
const ACCENT: [u8; 3] = [124, 142, 255];
/// How long the window takes to fade and slide in when it opens.
const ENTER: Duration = Duration::from_millis(420);

type Rect = (f32, f32, f32, f32);

/// The on/off settings.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Toggle {
    Autostart,
    HideFullscreen,
    ShowClock,
    Clock24,
    IdleEyes,
    Sound,
    Clipboard,
    HideIsland,
    Water,
    Timer,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Seg {
    Theme,
    Motion,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Slide {
    OpenDelay,
    CloseDelay,
    Volume,
    AlarmEvery,
    AlarmFor,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Act {
    OpenFolder,
    CheckUpdate,
    Reset,
    Quit,
}

#[derive(Clone, Copy)]
enum Kind {
    Toggle(Toggle),
    Seg(Seg),
    Slider(Slide),
    Button(Act),
    /// A button that shows the current choice, for the timer's alarm sound.
    Pick,
}

struct Item {
    label: &'static str,
    kind: Kind,
    row: Rect,
    /// The part of the row that holds the control.
    ctrl: Rect,
}

struct Section {
    title: &'static str,
    rect: Rect,
    /// The colour of the section's dot and glow.
    tint: [u8; 3],
}

fn tint_of(title: &str) -> [u8; 3] {
    match title {
        "General" => [124, 142, 255],
        "Tabs you can add" => [255, 190, 70],
        "Look and motion" => [190, 120, 255],
        _ => [70, 200, 210],
    }
}

/// Everything in the window, laid out in logical pixels. The layout never changes, so it is cheap to
/// rebuild whenever it is needed.
fn layout() -> (Vec<Section>, Vec<Item>) {
    // Column, top, title and rows of each section.
    type Plan = (usize, f32, &'static str, Vec<(&'static str, Kind)>);
    let columns = [(20.0, 302.0), (338.0, 302.0)];
    let plan: [Plan; 4] = [
        (
            0,
            60.0,
            "General",
            vec![
                ("Start with Windows", Kind::Toggle(Toggle::Autostart)),
                ("Hide in fullscreen apps", Kind::Toggle(Toggle::HideFullscreen)),
                ("Show clock", Kind::Toggle(Toggle::ShowClock)),
                ("24-hour clock", Kind::Toggle(Toggle::Clock24)),
                ("Idle eyes", Kind::Toggle(Toggle::IdleEyes)),
                ("Clipboard history", Kind::Toggle(Toggle::Clipboard)),
                ("Auto-hide island", Kind::Toggle(Toggle::HideIsland)),
            ],
        ),
        (
            0,
            352.0,
            "Tabs you can add",
            vec![("Water tracker", Kind::Toggle(Toggle::Water)), ("Timer", Kind::Toggle(Toggle::Timer))],
        ),
        (
            1,
            60.0,
            "Look and motion",
            vec![
                ("Theme", Kind::Seg(Seg::Theme)),
                ("Animation", Kind::Seg(Seg::Motion)),
                ("Open delay", Kind::Slider(Slide::OpenDelay)),
                ("Close delay", Kind::Slider(Slide::CloseDelay)),
            ],
        ),
        (
            1,
            250.0,
            "Sounds and timer alarm",
            vec![
                ("Sounds", Kind::Toggle(Toggle::Sound)),
                ("Volume", Kind::Slider(Slide::Volume)),
                ("Repeat every", Kind::Slider(Slide::AlarmEvery)),
                ("Ring for", Kind::Slider(Slide::AlarmFor)),
                ("Alarm sound", Kind::Pick),
            ],
        ),
    ];

    let (mut sections, mut items) = (Vec::new(), Vec::new());
    for (col, top, title, rows) in plan {
        let (x, w) = columns[col];
        let height = HEAD_H + rows.len() as f32 * ROW_H + 8.0;
        sections.push(Section { title, rect: (x, top, w, height), tint: tint_of(title) });
        for (i, (label, kind)) in rows.into_iter().enumerate() {
            let row = (x + 8.0, top + HEAD_H + i as f32 * ROW_H, w - 16.0, ROW_H);
            let right = row.0 + row.2 - 8.0;
            let ctrl = match kind {
                Kind::Toggle(_) => (right - 32.0, row.1 + 8.0, 32.0, 18.0),
                Kind::Seg(seg) => (right - seg_width(seg), row.1 + 5.0, seg_width(seg), 24.0),
                Kind::Slider(_) => (right - 170.0, row.1 + 15.0, SLIDER_W, 4.0),
                Kind::Pick => (right - 168.0, row.1 + 5.0, 168.0, 24.0),
                Kind::Button(_) => row,
            };
            items.push(Item { label, kind, row, ctrl });
        }
    }
    // The two buttons under the right-hand column, and Quit under the left-hand one.
    for (i, (label, act)) in [("Open data folder", Act::OpenFolder), ("Reset to defaults", Act::Reset)].into_iter().enumerate() {
        let rect = (338.0 + i as f32 * 154.0, 488.0, 148.0, 26.0);
        items.push(Item { label, kind: Kind::Button(act), row: rect, ctrl: rect });
    }
    let check = (20.0, 488.0, 148.0, 26.0);
    items.push(Item { label: "Check for updates", kind: Kind::Button(Act::CheckUpdate), row: check, ctrl: check });
    let quit = (492.0, 518.0, 148.0, 24.0);
    items.push(Item { label: "Quit Sky Island", kind: Kind::Button(Act::Quit), row: quit, ctrl: quit });
    (sections, items)
}

/// Ask for a sound file with the standard Windows Open dialog.
fn pick_sound(owner: &Window) -> Option<std::path::PathBuf> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::{FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH};
    use windows::core::w;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let RawWindowHandle::Win32(h) = owner.window_handle().ok()?.as_raw() else { return None };
    let hwnd = HWND(h.hwnd.get() as *mut _);
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        dialog.SetTitle(w!("Choose the timer alarm sound")).ok()?;
        let filters = [COMDLG_FILTERSPEC { pszName: w!("Sound files"), pszSpec: w!("*.wav;*.mp3;*.m4a;*.wma;*.aac") }];
        dialog.SetFileTypes(&filters).ok()?;
        // Cancelling the dialog is an error result, which is simply "nothing chosen".
        dialog.Show(Some(hwnd)).ok()?;
        let name = dialog.GetResult().ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = name.to_string().ok().map(std::path::PathBuf::from);
        CoTaskMemFree(Some(name.0 as *const _));
        path
    }
}

fn inside(r: Rect, x: f32, y: f32) -> bool {
    x >= r.0 && x <= r.0 + r.2 && y >= r.1 && y <= r.1 + r.3
}

const THEME_NAMES: [&str; 3] = ["Dark", "Light", "Follow System"];
const MOTION_NAMES: [&str; 4] = ["Calm", "Normal", "Snappy", "None"];

/// How wide a choice is compared with the others in the same row ("Follow System" is the longest).
fn seg_weights(seg: Seg) -> &'static [f32] {
    match seg {
        Seg::Theme => &[1.0, 1.0, 2.0],
        Seg::Motion => &[1.0, 1.0, 1.0, 1.0],
    }
}

fn seg_width(seg: Seg) -> f32 {
    match seg {
        Seg::Theme => 204.0,
        Seg::Motion => 184.0,
    }
}

/// The area of each choice inside a segmented control.
fn seg_cells(seg: Seg, ctrl: Rect) -> Vec<Rect> {
    let weights = seg_weights(seg);
    let total: f32 = weights.iter().sum();
    let mut x = ctrl.0;
    weights
        .iter()
        .map(|w| {
            let cell = (x, ctrl.1, ctrl.2 * w / total, ctrl.3);
            x += cell.2;
            cell
        })
        .collect()
}

fn seg_names(seg: Seg) -> &'static [&'static str] {
    match seg {
        Seg::Theme => &THEME_NAMES,
        Seg::Motion => &MOTION_NAMES,
    }
}

fn range_of(slide: Slide) -> Range {
    match slide {
        Slide::OpenDelay => settings::OPEN_DELAY,
        Slide::CloseDelay => settings::CLOSE_DELAY,
        Slide::Volume => settings::VOLUME,
        Slide::AlarmEvery => settings::ALARM_EVERY,
        Slide::AlarmFor => settings::ALARM_FOR,
    }
}

fn slider_text(slide: Slide, v: u32) -> String {
    match slide {
        Slide::OpenDelay if v == 0 => "Instant".to_string(),
        Slide::OpenDelay | Slide::CloseDelay => format!("{v} ms"),
        Slide::Volume => format!("{v}%"),
        Slide::AlarmEvery | Slide::AlarmFor => format!("{v} s"),
    }
}

/// The value a slider has when the mouse is at `x`, on a track starting at `track.0`.
fn slider_value(range: Range, track: Rect, x: f32) -> u32 {
    let t = ((x - track.0) / track.2).clamp(0.0, 1.0);
    let steps = ((range.max - range.min) / range.step) as f32;
    range.min + (t * steps).round() as u32 * range.step
}

pub(crate) struct Panel {
    window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
    glows: Glows,
    /// The mouse, in logical pixels.
    cursor: (f32, f32),
    over_window: bool,
    /// The slider being dragged.
    drag: Option<Slide>,
    /// "Reset to defaults" was clicked once and waits for a second click until this time.
    pub(crate) reset_armed: Option<Instant>,
    last_draw: Instant,
    /// When the window opened, for its entrance.
    opened: Instant,
    /// Something is still moving, so more frames are needed.
    pub(crate) busy: bool,
}

impl Panel {
    pub(crate) fn id(&self) -> winit::window::WindowId {
        self.window.id()
    }

    pub(crate) fn redraw(&self) {
        self.window.request_redraw();
    }
}

impl App {
    /// Whether an on/off setting is on.
    fn toggle_value(&self, t: Toggle) -> bool {
        match t {
            Toggle::Autostart => self.autostart,
            Toggle::HideFullscreen => self.settings.hide_fullscreen,
            Toggle::ShowClock => self.settings.show_clock,
            Toggle::Clock24 => self.settings.clock_24h,
            Toggle::IdleEyes => self.settings.idle_eyes,
            Toggle::Sound => self.settings.sound,
            Toggle::Clipboard => self.settings.clipboard,
            Toggle::HideIsland => self.settings.hide_island,
            Toggle::Water => self.settings.water,
            Toggle::Timer => self.settings.timer,
        }
    }

    /// Flip an on/off setting, with everything that has to follow from it. Used by the notch's Settings
    /// tab and by this window.
    pub(crate) fn toggle(&mut self, t: Toggle) {
        match t {
            Toggle::Autostart => {
                let on = !self.autostart;
                settings::set_autostart(on);
                self.autostart = settings::autostart_enabled();
            }
            Toggle::HideFullscreen => self.settings.hide_fullscreen = !self.settings.hide_fullscreen,
            Toggle::ShowClock => self.settings.show_clock = !self.settings.show_clock,
            Toggle::Clock24 => self.settings.clock_24h = !self.settings.clock_24h,
            Toggle::IdleEyes => self.settings.idle_eyes = !self.settings.idle_eyes,
            Toggle::Sound => self.settings.sound = !self.settings.sound,
            Toggle::Clipboard => self.settings.clipboard = !self.settings.clipboard,
            Toggle::HideIsland => self.settings.hide_island = !self.settings.hide_island,
            Toggle::Water => {
                self.settings.water = !self.settings.water;
                if self.settings.water {
                    self.water.restart(Instant::now());
                } else if self.view == View::Water {
                    self.view = View::Player;
                }
                self.tabs_changed();
            }
            Toggle::Timer => {
                self.settings.timer = !self.settings.timer;
                if !self.settings.timer {
                    // Switching it off stops a running timer and puts the tab away.
                    self.end_timer_edit(false);
                    self.timer.reset();
                    if self.view == View::Timer {
                        self.view = View::Player;
                    }
                }
                self.tabs_changed();
            }
        }
        self.settings_changed();
    }

    /// The tab bar has one tab more or fewer, so its highlight jumps instead of sliding.
    fn tabs_changed(&mut self) {
        if let Some(target) = self.tab_target() {
            self.tab_pos = target;
            self.tab_vel = 0.0;
        }
    }

    /// Save the settings and let the notch (and this window) pick them up.
    pub(crate) fn settings_changed(&mut self) {
        self.settings.save();
        self.sync_tray_toggle();
        self.apply_visibility();
        self.last_tick = Instant::now();
        if let Some(w) = &self.window {
            w.request_redraw();
        }
        if let Some(p) = &self.panel {
            p.redraw();
        }
    }

    fn seg_value(&self, seg: Seg) -> usize {
        match seg {
            Seg::Theme => ThemeMode::ALL.iter().position(|t| *t == self.settings.theme).unwrap_or(0),
            Seg::Motion => Motion::ALL.iter().position(|m| *m == self.settings.motion).unwrap_or(1),
        }
    }

    fn set_seg(&mut self, seg: Seg, i: usize) {
        match seg {
            Seg::Theme => self.settings.theme = ThemeMode::ALL[i.min(2)],
            Seg::Motion => self.settings.motion = Motion::ALL[i.min(3)],
        }
        self.settings_changed();
    }

    fn slider_get(&self, slide: Slide) -> u32 {
        match slide {
            Slide::OpenDelay => self.settings.open_delay,
            Slide::CloseDelay => self.settings.close_delay,
            Slide::Volume => self.settings.volume,
            Slide::AlarmEvery => self.settings.alarm_every,
            Slide::AlarmFor => self.settings.alarm_for,
        }
    }

    /// Change a slider's value. It takes effect at once; it is saved when the mouse is let go.
    fn slider_set(&mut self, slide: Slide, v: u32) {
        let v = range_of(slide).clamp(v);
        match slide {
            Slide::OpenDelay => self.settings.open_delay = v,
            Slide::CloseDelay => self.settings.close_delay = v,
            Slide::Volume => self.settings.volume = v,
            Slide::AlarmEvery => self.settings.alarm_every = v,
            Slide::AlarmFor => self.settings.alarm_for = v,
        }
    }

    /// Put every setting back to what a fresh install has.
    fn reset_settings(&mut self) {
        settings::set_autostart(false);
        self.autostart = false;
        self.settings = Settings::default();
        self.end_timer_edit(false);
        self.timer.reset();
        if matches!(self.view, View::Water | View::Timer) {
            self.view = View::Player;
        }
        self.tabs_changed();
        self.settings_changed();
    }

    /// Open the settings window, or bring it forward if it is already open.
    pub(crate) fn open_panel(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(p) = &self.panel {
            p.window.set_minimized(false);
            p.window.focus_window();
            return;
        }
        let mut attrs = Window::default_attributes()
            .with_title("Sky Island Settings")
            .with_inner_size(LogicalSize::new(WIDTH, HEIGHT))
            .with_resizable(false)
            .with_enabled_buttons(WindowButtons::CLOSE | WindowButtons::MINIMIZE);
        if let Ok(icon) = Icon::from_rgba(appicon::rgba(32), 32, 32) {
            attrs = attrs.with_window_icon(Some(icon));
        }
        let Ok(window) = event_loop.create_window(attrs) else { return };
        let window = Rc::new(window);
        // In the middle of the screen the notch is on.
        if let Some(monitor) = window.current_monitor().or_else(|| window.primary_monitor()) {
            let (pos, size) = (monitor.position(), monitor.size());
            let outer = window.outer_size();
            let x = pos.x + (size.width as i32 - outer.width as i32) / 2;
            let y = pos.y + (size.height as i32 - outer.height as i32) / 2;
            window.set_outer_position(PhysicalPosition::new(x, y));
        }
        let Ok(context) = Context::new(window.clone()) else { return };
        let Ok(surface) = Surface::new(&context, window.clone()) else { return };
        window.request_redraw();
        self.panel = Some(Panel {
            window,
            surface,
            glows: Glows::default(),
            cursor: (-1.0, -1.0),
            over_window: false,
            drag: None,
            reset_armed: None,
            last_draw: Instant::now(),
            opened: Instant::now(),
            busy: false,
        });
    }

    /// An event for the settings window.
    pub(crate) fn panel_event(&mut self, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.panel = None,
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed && event.logical_key == Key::Named(NamedKey::Escape) => {
                self.panel = None;
            }
            WindowEvent::ThemeChanged(theme) => {
                self.system_light = theme == winit::window::Theme::Light;
                self.settings_changed();
            }
            WindowEvent::Resized(_) | WindowEvent::Focused(_) => {
                if let Some(p) = &self.panel {
                    p.redraw();
                }
            }
            WindowEvent::CursorEntered { .. } => {
                if let Some(p) = &mut self.panel {
                    p.over_window = true;
                }
            }
            WindowEvent::CursorLeft { .. } => {
                if let Some(p) = &mut self.panel {
                    p.over_window = false;
                    p.cursor = (-1.0, -1.0);
                    p.redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let Some(p) = &mut self.panel else { return };
                let s = p.window.scale_factor() as f32;
                p.cursor = (position.x as f32 / s, position.y as f32 / s);
                p.over_window = true;
                let (cx, cy, drag) = (p.cursor.0, p.cursor.1, p.drag);
                p.redraw();
                if let Some(slide) = drag {
                    let (_, items) = layout();
                    if let Some(item) = items.iter().find(|i| matches!(i.kind, Kind::Slider(k) if k == slide)) {
                        let v = slider_value(range_of(slide), item.ctrl, cx);
                        let _ = cy;
                        self.slider_set(slide, v);
                    }
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                let Some(p) = &self.panel else { return };
                let (cx, cy) = p.cursor;
                if state == ElementState::Released {
                    if let Some(slide) = self.panel.as_mut().and_then(|p| p.drag.take()) {
                        self.settings_changed();
                        if slide == Slide::Volume {
                            // Let the new loudness be heard.
                            self.play_sound(crate::sound::Chime::Reminder);
                        }
                    }
                    return;
                }
                let (_, items) = layout();
                let Some(item) = items.iter().find(|i| match i.kind {
                    Kind::Slider(_) => inside((i.ctrl.0 - 8.0, i.row.1, SLIDER_W + 16.0, i.row.3), cx, cy),
                    Kind::Seg(_) | Kind::Pick => inside(i.ctrl, cx, cy),
                    _ => inside(i.row, cx, cy),
                }) else {
                    return;
                };
                match item.kind {
                    Kind::Toggle(t) => self.toggle(t),
                    Kind::Seg(seg) => {
                        let cells = seg_cells(seg, item.ctrl);
                        let i = cells.iter().position(|c| cx < c.0 + c.2).unwrap_or(cells.len() - 1);
                        self.set_seg(seg, i);
                    }
                    Kind::Slider(slide) => {
                        let v = slider_value(range_of(slide), item.ctrl, cx);
                        self.slider_set(slide, v);
                        if let Some(p) = &mut self.panel {
                            p.drag = Some(slide);
                            p.redraw();
                        }
                    }
                    Kind::Pick => {
                        if self.settings.alarm_sound.is_some() && cx >= item.ctrl.0 + item.ctrl.2 - 26.0 {
                            // The cross on the right goes back to the built-in chime.
                            crate::sound::stop_file();
                            self.settings.alarm_sound = None;
                            self.settings_changed();
                        } else if let Some(path) = self.panel.as_ref().and_then(|p| pick_sound(&p.window)) {
                            self.settings.alarm_sound = Some(path);
                            self.settings_changed();
                            // Let the new sound be heard (a file still playing is stopped first).
                            crate::sound::stop_file();
                            self.play_alarm();
                        }
                        if let Some(p) = &self.panel {
                            p.redraw();
                        }
                    }
                    Kind::Button(Act::OpenFolder) => {
                        if let Some(dir) = std::env::var_os("APPDATA").map(|d| std::path::PathBuf::from(d).join("Sky-Island")) {
                            let _ = std::fs::create_dir_all(&dir);
                            let _ = std::process::Command::new("explorer").arg(dir).spawn();
                        }
                    }
                    Kind::Button(Act::CheckUpdate) => {
                        match self.update.clone() {
                            crate::update::Status::Available { version, asset: Some(asset), .. } => {
                                self.update = crate::update::Status::Downloading { version: version.clone(), percent: 0 };
                                crate::update::spawn_download(version, asset, self.proxy.clone());
                            }
                            crate::update::Status::Available { page, .. } => {
                                // A release without an installer: show its page instead.
                                let _ = std::process::Command::new("explorer").arg(page).spawn();
                            }
                            crate::update::Status::Ready { path, .. } => {
                                // The installer closes this app, installs over it and starts it again.
                                if std::process::Command::new(path).spawn().is_ok() {
                                    self.quit = true;
                                }
                            }
                            crate::update::Status::Checking | crate::update::Status::Downloading { .. } => {}
                            _ => {
                                self.update = crate::update::Status::Checking;
                                crate::update::spawn_check(self.proxy.clone());
                            }
                        }
                        if let Some(p) = &self.panel {
                            p.redraw();
                        }
                    }
                    Kind::Button(Act::Quit) => self.quit = true,
                    Kind::Button(Act::Reset) => {
                        let armed = self.panel.as_ref().is_some_and(|p| p.reset_armed.is_some_and(|t| Instant::now() < t));
                        if armed {
                            if let Some(p) = &mut self.panel {
                                p.reset_armed = None;
                            }
                            self.reset_settings();
                        } else if let Some(p) = &mut self.panel {
                            p.reset_armed = Some(Instant::now() + RESET_WINDOW);
                            p.redraw();
                        }
                    }
                }
            }
            WindowEvent::RedrawRequested => self.draw_panel(),
            _ => {}
        }
    }

    fn draw_panel(&mut self) {
        let Some(mut panel) = self.panel.take() else { return };
        self.paint_panel(&mut panel);
        self.panel = Some(panel);
    }

    fn paint_panel(&self, panel: &mut Panel) {
        theme::set_light(self.light());
        let now = Instant::now();
        panel.glows.begin(now.duration_since(panel.last_draw).as_secs_f64().min(1.0 / 30.0));
        panel.last_draw = now;

        let size = panel.window.inner_size();
        let (Some(w), Some(h)) = (std::num::NonZeroU32::new(size.width), std::num::NonZeroU32::new(size.height)) else {
            return;
        };
        let Some(mut pm) = Pixmap::new(w.get(), h.get()) else { return };
        let s = panel.window.scale_factor() as f32;
        let (fw, fh) = (w.get() as f32, h.get() as f32);

        // A soft background that gets a little lighter towards the bottom (the other way in the light look).
        gfx::gradient_rounded(&mut pm, 0.0, 0.0, fw, fh, 0.0, [11, 11, 15, 255], [24, 24, 31, 255], false);
        gfx::glow(&mut pm, fw * 0.5, fh * 1.1, fw * 0.7, [ACCENT[0], ACCENT[1], ACCENT[2], 26], None);

        // Everything else is drawn on a layer, which glides up and fades in when the window opens.
        let mut layer = Pixmap::new(w.get(), h.get()).unwrap_or_else(|| Pixmap::new(1, 1).unwrap());
        if let Some(font) = &self.font {
            let heading = self.heading_font().unwrap_or(font);
            let white = [255, 255, 255, 255];
            let grey = [165, 165, 175, 255];

            // Header: the app's icon, the title, and the version.
            let logo = appicon::pixmap(64);
            let scale = 30.0 * s / logo.width() as f32;
            let paint = tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
            layer.draw_pixmap(0, 0, logo.as_ref(), &paint, tiny_skia::Transform::from_scale(scale, scale).post_translate(22.0 * s, 12.0 * s), None);
            crate::draw_text(&mut layer, heading, "Settings", 60.0 * s, 28.0 * s, 17.0 * s, white, 200.0 * s);
            crate::draw_text(&mut layer, font, "Changes apply right away and are saved as you make them.", 60.0 * s, 44.0 * s, 10.5 * s, grey, 420.0 * s);
            let version = format!("Version {}", env!("CARGO_PKG_VERSION"));
            let chip = (548.0, 17.0, 92.0, 22.0);
            gfx::gradient_rounded(&mut layer, chip.0 * s, chip.1 * s, chip.2 * s, chip.3 * s, chip.3 * s / 2.0, [ACCENT[0], ACCENT[1], ACCENT[2], 56], [ACCENT[0], ACCENT[1], ACCENT[2], 24], false);
            gfx::stroke_rounded(&mut layer, chip.0 * s + 0.5 * s, chip.1 * s + 0.5 * s, chip.2 * s - s, chip.3 * s - s, chip.3 * s / 2.0, s, [ACCENT[0], ACCENT[1], ACCENT[2], 110]);
            crate::draw_text_centered(&mut layer, font, &version, (chip.0 + chip.2 / 2.0) * s, (chip.1 + 15.0) * s, 10.5 * s, white, chip.2 * s);

            let (sections, items) = layout();
            for sec in &sections {
                let (x, y, cw, ch) = sec.rect;
                let tint = [sec.tint[0], sec.tint[1], sec.tint[2]];
                crate::draw_card(&mut layer, sec.rect, 14.0, s, 1.0, 0.0, tint);
                // A soft glow of the section's colour in its top corner, kept inside the card.
                if let Some(path) = gfx::rounded_path(x * s, y * s, cw * s, ch * s, 14.0 * s)
                    && let Some(mask) = gfx::mask_of(layer.width(), layer.height(), &path) {
                        gfx::glow(&mut layer, (x + 22.0) * s, (y + 20.0) * s, 120.0 * s, [tint[0], tint[1], tint[2], 34], Some(&mask));
                    }
                gfx::glow(&mut layer, (x + 20.0) * s, (y + 20.0) * s, 11.0 * s, [tint[0], tint[1], tint[2], 90], None);
                gfx::gradient_circle(&mut layer, (x + 20.0) * s, (y + 20.0) * s, 4.0 * s, [tint[0], tint[1], tint[2], 255], gfx::mix([tint[0], tint[1], tint[2], 255], [255, 255, 255, 255], 0.5));
                crate::draw_text(&mut layer, heading, sec.title, (x + 33.0) * s, (y + 24.0) * s, 12.0 * s, white, (cw - 48.0) * s);
                crate::fill_rect(&mut layer, (x + 14.0) * s, (y + HEAD_H - 2.0) * s, (cw - 28.0) * s, s, [255, 255, 255, 18]);
            }
            for (idx, item) in items.iter().enumerate() {
                // A faint line between two rows of the same section.
                if idx > 0 && !matches!(item.kind, Kind::Button(_)) {
                    let prev = &items[idx - 1];
                    if !matches!(prev.kind, Kind::Button(_)) && prev.row.0 == item.row.0 && (item.row.1 - prev.row.1 - ROW_H).abs() < 0.5 {
                        crate::fill_rect(&mut layer, (item.row.0 + 10.0) * s, item.row.1 * s, (item.row.2 - 20.0) * s, s * 0.7, [255, 255, 255, 9]);
                    }
                }
                self.paint_item(&mut layer, panel, font, heading, idx, item, s);
            }

            // The bottom bar: a divider, what the last update check found, and a reassurance.
            crate::fill_rect(&mut layer, 20.0 * s, 479.0 * s, 620.0 * s, s, [255, 255, 255, 20]);
            let (note, note_ink) = match &self.update {
                crate::update::Status::Idle => (String::new(), grey),
                crate::update::Status::Checking => ("Checking...".to_string(), grey),
                crate::update::Status::Latest => ("You have the latest version".to_string(), [90, 210, 130, 255]),
                crate::update::Status::Available { version, .. } => (format!("Version {version} is available"), [255, 190, 70, 255]),
                crate::update::Status::Downloading { percent, .. } => (format!("Downloading... {percent}%"), grey),
                crate::update::Status::Ready { .. } => ("Downloaded. Ready to install".to_string(), [90, 210, 130, 255]),
                crate::update::Status::Failed => ("Could not check. Are you online?".to_string(), [255, 130, 120, 255]),
                crate::update::Status::DownloadFailed => ("Download failed. Try again".to_string(), [255, 130, 120, 255]),
            };
            crate::draw_text(&mut layer, font, &note, 176.0 * s, 505.0 * s, 10.5 * s, note_ink, 156.0 * s);
            crate::draw_text(&mut layer, font, "Your settings stay on this computer.", 24.0 * s, 535.0 * s, 10.0 * s, [120, 120, 134, 255], 280.0 * s);
        }

        // The entrance: the layer rises into place and fades in.
        let x = (panel.opened.elapsed().as_secs_f32() / ENTER.as_secs_f32()).clamp(0.0, 1.0);
        let ease = 1.0 - (1.0 - x).powi(3);
        let paint = tiny_skia::PixmapPaint { opacity: ease, ..Default::default() };
        pm.draw_pixmap(0, 0, layer.as_ref(), &paint, tiny_skia::Transform::from_translate(0.0, (1.0 - ease) * 16.0 * s), None);

        let Ok(()) = panel.surface.resize(w, h) else { return };
        let Ok(mut buffer) = panel.surface.buffer_mut() else { return };
        for (dst, src) in buffer.iter_mut().zip(pm.pixels()) {
            *dst = (src.alpha() as u32) << 24 | (src.red() as u32) << 16 | (src.green() as u32) << 8 | src.blue() as u32;
        }
        buffer.present().ok();
        panel.busy = panel.glows.busy() || panel.opened.elapsed() < ENTER;
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_item(&self, pm: &mut Pixmap, panel: &Panel, font: &fontdue::Font, heading: &fontdue::Font, idx: usize, item: &Item, s: f32) {
        let white = [255, 255, 255, 255];
        let grey = [165, 165, 175, 255];
        let (cx, cy) = panel.cursor;
        let key = 0x2000_0000 + idx as u64 * 16;
        let row_hot = !matches!(item.kind, Kind::Button(_)) && inside(item.row, cx, cy);
        let hot = panel.glows.fade(key, row_hot);
        let accent = [ACCENT[0], ACCENT[1], ACCENT[2]];

        if !matches!(item.kind, Kind::Button(_)) {
            if hot > 0.01 {
                crate::fill_rounded(pm, item.row.0 * s, item.row.1 * s, item.row.2 * s, item.row.3 * s, 8.0 * s, [255, 255, 255, (14.0 * hot) as u8]);
            }
            let label_w = match item.kind {
                Kind::Toggle(_) => item.row.2 - 56.0,
                Kind::Seg(_) | Kind::Slider(_) | Kind::Pick => item.ctrl.0 - item.row.0 - 18.0,
                Kind::Button(_) => item.row.2,
            };
            crate::draw_text(pm, font, item.label, (item.row.0 + 8.0) * s, (item.row.1 + 21.5) * s, 12.5 * s, crate::pick(white, [226, 226, 234, 255], hot), label_w * s);
        }

        match item.kind {
            Kind::Toggle(t) => {
                let on = panel.glows.follow(key + 1, if self.toggle_value(t) { 1.0 } else { 0.0 }, 34.0, 0.62) as f32;
                crate::draw_switch(pm, item.ctrl.0 * s, item.ctrl.1 * s, s, on, 255);
            }
            Kind::Seg(seg) => {
                let names = seg_names(seg);
                let cells = seg_cells(seg, item.ctrl);
                let (x, y, w, h) = (item.ctrl.0 * s, item.ctrl.1 * s, item.ctrl.2 * s, item.ctrl.3 * s);
                crate::fill_rounded(pm, x, y, w, h, h / 2.0, [255, 255, 255, 18]);
                // The highlight glides (and changes width) to the chosen option.
                let pos = panel.glows.follow(key + 2, self.seg_value(seg) as f64, 30.0, 0.78) as f32;
                let (i, f) = ((pos.floor().max(0.0) as usize).min(cells.len() - 1), pos - pos.floor().max(0.0));
                let (a, b) = (cells[i], cells[(i + 1).min(cells.len() - 1)]);
                let (px, pw) = (a.0 + (b.0 - a.0) * f, a.2 + (b.2 - a.2) * f);
                let (px, pw) = (px * s + 2.0 * s, pw * s - 4.0 * s);
                gfx::gradient_rounded(pm, px, y + 2.0 * s, pw, h - 4.0 * s, (h - 4.0 * s) / 2.0, [accent[0], accent[1], accent[2], 130], [accent[0], accent[1], accent[2], 80], false);
                gfx::stroke_rounded(pm, px + 0.5 * s, y + 2.5 * s, pw - s, h - 5.0 * s, (h - 5.0 * s) / 2.0, s, [accent[0], accent[1], accent[2], 150]);
                for (i, name) in names.iter().enumerate() {
                    let cell = cells[i];
                    let near = 1.0 - (pos - i as f32).abs().min(1.0);
                    let over = panel.glows.fade(key + 3 + i as u64, inside(cell, cx, cy));
                    let ink = crate::pick(white, [150, 150, 160, 255], near.max(over * 0.7));
                    crate::draw_text_centered(pm, font, name, (cell.0 + cell.2 / 2.0) * s, (cell.1 + 16.0) * s, 11.5 * s, ink, (cell.2 - 6.0) * s);
                }
            }
            Kind::Slider(slide) => {
                let range = range_of(slide);
                let v = self.slider_get(slide);
                let frac = (v - range.min) as f32 / (range.max - range.min) as f32;
                let (x, y, w, h) = (item.ctrl.0 * s, item.ctrl.1 * s, item.ctrl.2 * s, item.ctrl.3 * s);
                let grab = panel.glows.fade(key + 8, panel.drag == Some(slide) || inside((item.ctrl.0 - 8.0, item.row.1, SLIDER_W + 16.0, item.row.3), cx, cy));
                crate::fill_rounded(pm, x, y, w, h, h / 2.0, [255, 255, 255, 34]);
                let fill_w = (w * frac).max(h);
                gfx::gradient_rounded(pm, x, y, fill_w, h, h / 2.0, [accent[0], accent[1], accent[2], 200], [255, 255, 255, 200], true);
                let kx = x + w * frac;
                gfx::glow(pm, kx, y + h / 2.0, (11.0 + 4.0 * grab) * s, [accent[0], accent[1], accent[2], (60.0 + 60.0 * grab) as u8], None);
                // White in both looks (the light look turns white into dark, so ask for black there).
                let knob = if theme::is_light() { [0, 0, 0, 255] } else { white };
                crate::fill_circle(pm, kx, y + h / 2.0, (5.5 + 1.5 * grab) * s, knob);
                let text = slider_text(slide, v);
                let tw = crate::text_width(font, &text, 11.5 * s);
                crate::draw_text(pm, font, &text, (item.row.0 + item.row.2 - 8.0) * s - tw, (item.row.1 + 21.0) * s, 11.5 * s, grey, 60.0 * s);
            }
            Kind::Pick => {
                let (x, y, w, h) = (item.ctrl.0 * s, item.ctrl.1 * s, item.ctrl.2 * s, item.ctrl.3 * s);
                let over = panel.glows.fade(key + 9, inside(item.ctrl, cx, cy));
                gfx::gradient_rounded(pm, x, y, w, h, h / 2.0, [accent[0], accent[1], accent[2], crate::pick(100.0, 56.0, over) as u8], [accent[0], accent[1], accent[2], crate::pick(60.0, 28.0, over) as u8], false);
                gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [accent[0], accent[1], accent[2], 120]);
                let custom = self.settings.alarm_sound.as_ref();
                let name = custom.and_then(|p| p.file_name()).map_or_else(|| "Built-in chime".to_string(), |n| n.to_string_lossy().into_owned());
                let text_w = if custom.is_some() { item.ctrl.2 - 38.0 } else { item.ctrl.2 - 16.0 };
                crate::draw_text_centered(pm, heading, &name, (item.ctrl.0 + 8.0 + text_w / 2.0) * s, (item.ctrl.1 + 16.5) * s, 11.5 * s, white, text_w * s);
                if custom.is_some() {
                    let (bx, by) = ((item.ctrl.0 + item.ctrl.2 - 14.0) * s, (item.ctrl.1 + item.ctrl.3 / 2.0) * s);
                    let hot_x = panel.glows.fade(key + 10, cx >= item.ctrl.0 + item.ctrl.2 - 26.0 && inside(item.ctrl, cx, cy));
                    crate::fill_circle(pm, bx, by, 8.0 * s, [255, 96, 86, (70.0 * hot_x) as u8]);
                    crate::draw_text_centered(pm, font, "\u{00D7}", bx, by + 4.5 * s, 14.0 * s, crate::pick(white, grey, hot_x), 14.0 * s);
                }
            }
            Kind::Button(act) => {
                let armed = act == Act::Reset && panel.reset_armed.is_some_and(|t| Instant::now() < t);
                let over = panel.glows.fade(key + 9, inside(item.row, cx, cy));
                let (x, y, w, h) = (item.row.0 * s, item.row.1 * s, item.row.2 * s, item.row.3 * s);
                let tint = if armed || act == Act::Quit { [255, 96, 86] } else { accent };
                gfx::gradient_rounded(pm, x, y, w, h, h / 2.0, [tint[0], tint[1], tint[2], crate::pick(100.0, 56.0, over) as u8], [tint[0], tint[1], tint[2], crate::pick(60.0, 28.0, over) as u8], false);
                gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [tint[0], tint[1], tint[2], 120]);
                let download = match &self.update {
                    _ if act != Act::CheckUpdate => None,
                    crate::update::Status::Available { version, .. } => Some(format!("Download {version}")),
                    crate::update::Status::Downloading { .. } => Some("Downloading...".to_string()),
                    crate::update::Status::Ready { version, .. } => Some(format!("Install {version}")),
                    _ => None,
                };
                let label = if armed { "Click again to reset" } else { download.as_deref().unwrap_or(item.label) };
                crate::draw_text_centered(pm, heading, label, (item.row.0 + item.row.2 / 2.0) * s, (item.row.1 + 17.0) * s, 11.5 * s, white, (item.row.2 - 10.0) * s);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_control_fits_inside_the_window() {
        let (sections, items) = layout();
        for sec in &sections {
            assert!(sec.rect.0 >= 0.0 && sec.rect.0 + sec.rect.2 <= WIDTH as f32, "{} is too wide", sec.title);
            assert!(sec.rect.1 + sec.rect.3 <= HEIGHT as f32 - 20.0, "{} runs past the bottom", sec.title);
        }
        for item in &items {
            assert!(item.row.0 >= 0.0 && item.row.0 + item.row.2 <= WIDTH as f32, "{} is outside", item.label);
            assert!(item.row.1 + item.row.3 <= HEIGHT as f32, "{} is outside", item.label);
        }
    }

    #[test]
    fn sections_in_one_column_do_not_overlap() {
        let (sections, _) = layout();
        for (i, a) in sections.iter().enumerate() {
            for b in sections.iter().skip(i + 1) {
                let apart_x = a.rect.0 + a.rect.2 <= b.rect.0 || b.rect.0 + b.rect.2 <= a.rect.0;
                let apart_y = a.rect.1 + a.rect.3 <= b.rect.1 || b.rect.1 + b.rect.3 <= a.rect.1;
                assert!(apart_x || apart_y, "{} overlaps {}", a.title, b.title);
            }
        }
    }

    #[test]
    fn sliders_land_on_their_steps_and_stay_in_range() {
        let range = settings::OPEN_DELAY;
        let track = (100.0, 0.0, 100.0, 4.0);
        assert_eq!(slider_value(range, track, 0.0), 0, "left of the track is the minimum");
        assert_eq!(slider_value(range, track, 100.0), 0);
        assert_eq!(slider_value(range, track, 200.0), 500);
        assert_eq!(slider_value(range, track, 900.0), 500, "right of the track is the maximum");
        assert_eq!(slider_value(range, track, 150.0), 250);
        assert_eq!(slider_value(range, track, 153.0) % 10, 0, "always a whole step");
        let alarm = settings::ALARM_EVERY;
        assert_eq!(slider_value(alarm, track, 100.0), 1);
        assert_eq!(slider_value(alarm, track, 200.0), 10);
    }

    #[test]
    fn slider_text_is_readable() {
        assert_eq!(slider_text(Slide::OpenDelay, 0), "Instant");
        assert_eq!(slider_text(Slide::OpenDelay, 90), "90 ms");
        assert_eq!(slider_text(Slide::Volume, 60), "60%");
        assert_eq!(slider_text(Slide::AlarmEvery, 3), "3 s");
    }

    #[test]
    fn the_choice_lists_match_the_settings() {
        assert_eq!(THEME_NAMES.len(), ThemeMode::ALL.len());
        assert_eq!(MOTION_NAMES.len(), Motion::ALL.len());
        assert_eq!(seg_weights(Seg::Theme).len(), THEME_NAMES.len());
        assert_eq!(seg_weights(Seg::Motion).len(), MOTION_NAMES.len());
    }

    #[test]
    fn segmented_choices_fill_the_control_without_gaps() {
        let ctrl = (100.0, 10.0, 204.0, 24.0);
        let cells = seg_cells(Seg::Theme, ctrl);
        assert_eq!(cells.len(), 3);
        assert_eq!(cells[0].0, 100.0);
        assert!((cells[2].0 + cells[2].2 - 304.0).abs() < 0.01, "the last choice ends where the control ends");
        for pair in cells.windows(2) {
            assert!((pair[0].0 + pair[0].2 - pair[1].0).abs() < 0.01, "no gap between choices");
        }
        assert!(cells[2].2 > cells[0].2 * 1.9, "Follow System gets the most room");
    }
}
