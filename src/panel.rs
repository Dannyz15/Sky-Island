//! The Sky Island settings window: an ordinary Windows window with a sidebar of pages (General, Notch,
//! Appearance, Tabs, Sounds, About) and the settings of the chosen page on the right, grouped in cards.
//! It holds every setting, including the ones that do not fit in the notch.
//!
//! It is drawn with the same tools as the notch, so it moves the same way: switches slide, the choice
//! highlight glides between options, the sidebar highlight glides between pages, hovered rows light up.
//! Everything applies at once and is saved as it changes.

use std::rc::Rc;
use std::time::{Duration, Instant};

use softbuffer::{Context, Surface};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, NamedKey};
use winit::window::{Icon, Window, WindowButtons};

use crate::motion::Glows;
use crate::settings::{self, EyeStyle, Motion, NotchSize, Range, Settings, ThemeMode};
use crate::{App, View, appicon, gfx, theme};

const WIDTH: f64 = 840.0;
const HEIGHT: f64 = 700.0;
const SIDEBAR_W: f32 = 212.0;
const SIDE_TOP: f32 = 62.0;
const SIDE_ITEM_H: f32 = 42.0;
const CONTENT_X: f32 = SIDEBAR_W + 30.0;
const CONTENT_W: f32 = WIDTH as f32 - CONTENT_X - 30.0;
const CONTENT_TOP: f32 = 100.0;
const ROW_H: f32 = 56.0;
const GROUP_LABEL_H: f32 = 24.0;
const GROUP_GAP: f32 = 16.0;
const SLIDER_W: f32 = 150.0;
const SLIDER_TEXT_W: f32 = 62.0;
/// How long "Reset to defaults" waits for a second click.
const RESET_WINDOW: Duration = Duration::from_secs(3);
/// How long the window takes to fade and slide in when it opens.
const ENTER: Duration = Duration::from_millis(420);
/// How long a page takes to fade in when it is chosen.
const PAGE_FADE: Duration = Duration::from_millis(220);
const KEY_SIDE: u64 = 0x3000_0000;

type Rect = (f32, f32, f32, f32);

/// The on/off settings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Toggle {
    Autostart,
    HideFullscreen,
    ShowClock,
    Clock24,
    Eyes,
    IdleEyes,
    Minimal,
    EmptyShelf,
    HudVolume,
    HudBrightness,
    HudLock,
    HudBattery,
    HudNetwork,
    HudAudio,
    Sound,
    Clipboard,
    HideIsland,
    Water,
    Timer,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Seg {
    Theme,
    Motion,
    Eyes,
    Size,
    Open,
    WeekStart,
    CalView,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Slide {
    OpenDelay,
    CloseDelay,
    Volume,
    AlarmEvery,
    AlarmFor,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Act {
    OpenFolder,
    CheckUpdate,
    Changelog,
    Reset,
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Toggle(Toggle),
    Seg(Seg),
    Slider(Slide),
    Button(Act),
    /// A button that shows the current choice, for the timer's alarm sound.
    Pick,
    /// A line with a value on the right; with an address it opens that page when clicked.
    Link(&'static str),
}

/// The pages of the window, in the order of the sidebar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Page {
    General,
    Notch,
    Appearance,
    Tabs,
    Sounds,
    Calendar,
    Shelf,
    Notices,
    About,
}

impl Page {
    const ALL: [Page; 9] =
        [Page::General, Page::Notch, Page::Appearance, Page::Tabs, Page::Sounds, Page::Calendar, Page::Shelf, Page::Notices, Page::About];

    fn index(self) -> usize {
        Self::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }

    fn title(self) -> &'static str {
        match self {
            Page::General => "General",
            Page::Notch => "Notch",
            Page::Appearance => "Appearance",
            Page::Tabs => "Tabs",
            Page::Sounds => "Sounds",
            Page::Calendar => "Calendar",
            Page::Shelf => "Shelf",
            Page::Notices => "Notices",
            Page::About => "About",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Page::General => "Startup, and how the island behaves and opens.",
            Page::Notch => "How big the island is, and what the closed notch shows.",
            Page::Appearance => "The colours and the motion.",
            Page::Tabs => "Choose which tabs the notch has.",
            Page::Sounds => "Chimes and the timer alarm.",
            Page::Calendar => "How the calendar counts its weeks and what it shows.",
            Page::Shelf => "What happens to the files on the shelf.",
            Page::Notices => "Choose which changes pop up on the notch.",
            Page::About => "This version, updates and your data.",
        }
    }

    /// The colour of the page's icon, and of its sliders and choices.
    fn tint(self) -> [u8; 3] {
        match self {
            Page::General => [142, 148, 168],
            Page::Notch => [96, 110, 255],
            Page::Appearance => [170, 112, 255],
            Page::Tabs => [255, 176, 64],
            Page::Sounds => [255, 92, 120],
            Page::Calendar => [255, 96, 86],
            Page::Shelf => [255, 140, 56],
            Page::Notices => [56, 176, 240],
            Page::About => [70, 190, 210],
        }
    }
}

/// One row of a page: its label, a line under it, what it controls, and the text of a button.
type RowSpec = (&'static str, &'static str, Kind, &'static str);

/// The groups (title and rows) on a page.
fn page_spec(page: Page) -> Vec<(&'static str, Vec<RowSpec>)> {
    use Kind::{Button, Link, Pick, Seg as S, Slider, Toggle as T};
    match page {
        Page::General => vec![
            ("Startup", vec![("Start with Windows", "Open Sky Island when you sign in", T(Toggle::Autostart), "")]),
            (
                "Behaviour",
                vec![
                    ("Hide in full screen", "Games, videos and slides that fill the screen", T(Toggle::HideFullscreen), ""),
                    ("Auto-hide island", "Slide out of view when you are not using it", T(Toggle::HideIsland), ""),
                ],
            ),
            (
                "Opening",
                vec![
                    ("Open the notch", "Hover over it, or click it", S(Seg::Open), ""),
                    ("Open delay", "How long the mouse rests on the notch before it opens", Slider(Slide::OpenDelay), ""),
                    ("Close delay", "How long the mouse is away before it closes", Slider(Slide::CloseDelay), ""),
                ],
            ),
        ],
        Page::Notch => vec![
            (
                "Size",
                vec![
                    ("Notch size", "Make the whole island bigger or smaller", S(Seg::Size), ""),
                    ("Minimal notch", "A smaller pill when nothing is playing", T(Toggle::Minimal), ""),
                ],
            ),
            (
                "Closed notch",
                vec![
                    ("Show clock", "The time on the closed notch", T(Toggle::ShowClock), ""),
                    ("24-hour clock", "13:00 instead of 1:00 PM", T(Toggle::Clock24), ""),
                ],
            ),
            (
                "Eyes",
                vec![
                    ("Enable eyes", "Blinking eyes on the closed notch while you work", T(Toggle::Eyes), ""),
                    ("Idle eyes", "They appear, and doze off, when you stop using the PC", T(Toggle::IdleEyes), ""),
                    ("Eye style", "How the eyes look", S(Seg::Eyes), ""),
                ],
            ),
        ],
        Page::Appearance => vec![
            (
                "Look",
                vec![
                    ("Theme", "Dark, light, or the same as Windows", S(Seg::Theme), ""),
                    ("Animation", "How lively the movement is", S(Seg::Motion), ""),
                ],
            ),
        ],
        Page::Tabs => vec![
            (
                "Tabs you can add",
                vec![
                    ("Water tracker", "A daily glass counter with reminders", T(Toggle::Water), ""),
                    ("Timer", "A countdown timer with an alarm", T(Toggle::Timer), ""),
                ],
            ),
            ("Clipboard", vec![("Clipboard history", "Keep what you copy, text and pictures", T(Toggle::Clipboard), "")]),
        ],
        Page::Sounds => vec![
            (
                "Chimes",
                vec![
                    ("Sounds", "Chimes for the timer, to-do and water reminders", T(Toggle::Sound), ""),
                    ("Volume", "How loud they are", Slider(Slide::Volume), ""),
                ],
            ),
            (
                "Timer alarm",
                vec![
                    ("Repeat every", "How often the alarm sounds again", Slider(Slide::AlarmEvery), ""),
                    ("Ring for", "How long it rings if nobody stops it", Slider(Slide::AlarmFor), ""),
                    ("Alarm sound", "Use a sound file of your own", Pick, ""),
                ],
            ),
        ],
        Page::Calendar => vec![(
            "Options",
            vec![
                ("Week starts on", "The first column of the month", S(Seg::WeekStart), ""),
                ("Default view", "A month, or just one week", S(Seg::CalView), ""),
            ],
        )],
        Page::Shelf => vec![(
            "Shelf",
            vec![("Start with an empty shelf", "Files from your last session are cleared when Sky Island starts", T(Toggle::EmptyShelf), "")],
        )],
        Page::Notices => vec![
            (
                "Indicators",
                vec![
                    ("Volume", "When you change the volume or mute", T(Toggle::HudVolume), ""),
                    ("Brightness", "On laptop screens", T(Toggle::HudBrightness), ""),
                    ("Caps Lock and Num Lock", "When you press the lock keys", T(Toggle::HudLock), ""),
                ],
            ),
            (
                "Popups",
                vec![
                    ("Battery", "Charging, unplugged and low battery", T(Toggle::HudBattery), ""),
                    ("Wi-Fi and internet", "When the connection changes", T(Toggle::HudNetwork), ""),
                    ("Headphones and speakers", "When an audio device is connected", T(Toggle::HudAudio), ""),
                ],
            ),
        ],
        Page::About => vec![
            (
                "Updates and data",
                vec![
                    ("Check for updates", "", Button(Act::CheckUpdate), "Check for updates"),
                    ("Changelog", "What changed in each version", Button(Act::Changelog), "View"),
                    ("Open data folder", "Your settings, tasks and shelf live here", Button(Act::OpenFolder), "Open"),
                    ("Reset to defaults", "Put every setting back to what a fresh install has", Button(Act::Reset), "Reset"),
                ],
            ),
            (
                "Info",
                vec![
                    ("Website", "", Link("https://github.com/Dannyz15/Sky-Island"), "github.com/Dannyz15/Sky-Island"),
                    ("Made by", "", Link(""), "Dannyz15"),
                ],
            ),
            (
                "Thanks",
                vec![
                    ("Synced lyrics", "", Link("https://lrclib.net"), "LRCLIB"),
                    ("Built with", "", Link("https://github.com/Dannyz15/Sky-Island/blob/main/Cargo.toml"), "Rust and open-source crates"),
                ],
            ),
        ],
    }
}

struct Item {
    label: &'static str,
    desc: &'static str,
    kind: Kind,
    /// The text of a button.
    btn: &'static str,
    row: Rect,
    /// The part of the row that holds the control.
    ctrl: Rect,
    /// Whether another row of the same card is above it, for the line between them.
    after_row: bool,
}

struct Group {
    title: &'static str,
    rect: Rect,
}

struct Layout {
    groups: Vec<Group>,
    items: Vec<Item>,
    /// The card at the top of the About page.
    hero: Option<Rect>,
}

/// Everything on a page, laid out in logical pixels.
fn layout(page: Page) -> Layout {
    let (mut groups, mut items) = (Vec::new(), Vec::new());
    let mut y = CONTENT_TOP;
    let mut hero = None;
    if page == Page::About {
        hero = Some((CONTENT_X, y, CONTENT_W, 80.0));
        y += 80.0 + GROUP_GAP;
    }
    if page == Page::Calendar {
        // A preview of the month (or week) with a label above it.
        y += GROUP_LABEL_H;
        hero = Some((CONTENT_X, y, CONTENT_W, CAL_PREVIEW_H));
        y += CAL_PREVIEW_H + GROUP_GAP;
    }
    for (title, rows) in page_spec(page) {
        y += GROUP_LABEL_H;
        let height: f32 = rows.iter().map(|(_, desc, kind, _)| row_height(desc, *kind)).sum();
        groups.push(Group { title, rect: (CONTENT_X, y, CONTENT_W, height) });
        let mut row_y = y;
        for (i, (label, desc, kind, btn)) in rows.into_iter().enumerate() {
            let row_h = row_height(desc, kind);
            let row = (CONTENT_X, row_y, CONTENT_W, row_h);
            row_y += row_h;
            let right = row.0 + row.2 - 18.0;
            let mid = row.1 + row_h / 2.0;
            let ctrl = match kind {
                Kind::Toggle(_) => (right - 40.0, mid - 11.25, 40.0, 22.5),
                Kind::Seg(seg) => (right - seg_width(seg), mid - 14.0, seg_width(seg), 28.0),
                Kind::Slider(_) => (right - SLIDER_TEXT_W - 12.0 - SLIDER_W, mid - 2.0, SLIDER_W, 4.0),
                Kind::Pick => (right - 200.0, mid - 14.0, 200.0, 28.0),
                Kind::Button(_) => {
                    let w = if btn.len() > 12 { 168.0 } else { 104.0 };
                    (right - w, mid - 14.0, w, 28.0)
                }
                Kind::Link(_) => (right - 260.0, mid - 14.0, 260.0, 28.0),
            };
            items.push(Item { label, desc, kind, btn, row, ctrl, after_row: i > 0 });
        }
        y += height + GROUP_GAP;
    }
    Layout { groups, items, hero }
}

/// The height of a row: taller when it has a line of text under its label.
fn row_height(desc: &str, kind: Kind) -> f32 {
    if desc.is_empty() && !matches!(kind, Kind::Button(Act::CheckUpdate)) { 40.0 } else { ROW_H }
}

/// The height of the calendar preview card.
const CAL_PREVIEW_H: f32 = 232.0;

/// The sidebar row of a page.
fn side_rect(page: Page) -> Rect {
    (12.0, SIDE_TOP + page.index() as f32 * SIDE_ITEM_H, SIDEBAR_W - 24.0, SIDE_ITEM_H - 4.0)
}

/// The Back button of the changelog, and where its card starts.
const CHANGELOG_BACK: Rect = (CONTENT_X + CONTENT_W - 96.0, 34.0, 96.0, 30.0);
const CHANGELOG_TOP: f32 = 100.0;

/// What kind of line a changelog row is.
#[derive(Clone, PartialEq, Debug)]
enum ChangeKind {
    /// The name of a release, with its date.
    Version(String),
    Heading,
    Bullet,
    Plain,
}

struct ChangeLine {
    kind: ChangeKind,
    text: String,
    /// Where the line sits, from the top of the list.
    y: f32,
}

/// Where the version number sits at the bottom of the sidebar (the baseline).
const VERSION_Y: f32 = HEIGHT as f32 - 24.0;

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
const EYE_NAMES: [&str; 4] = ["Classic", "Dots", "Cat", "Glow"];
const SIZE_NAMES: [&str; 5] = ["Auto", "100%", "115%", "130%", "150%"];
const OPEN_NAMES: [&str; 2] = ["Hover", "Click"];
const WEEK_NAMES: [&str; 2] = ["Sunday", "Monday"];
const VIEW_NAMES: [&str; 2] = ["Month", "Week"];

/// How wide a choice is compared with the others in the same row ("Follow System" is the longest).
fn seg_weights(seg: Seg) -> &'static [f32] {
    match seg {
        Seg::Theme => &[1.0, 1.0, 2.0],
        Seg::Motion | Seg::Eyes => &[1.0, 1.0, 1.0, 1.0],
        Seg::Size => &[1.0, 1.0, 1.0, 1.0, 1.0],
        Seg::Open | Seg::WeekStart | Seg::CalView => &[1.0, 1.0],
    }
}

fn seg_width(seg: Seg) -> f32 {
    match seg {
        Seg::Theme => 260.0,
        Seg::Motion | Seg::Eyes => 240.0,
        Seg::Size => 300.0,
        Seg::Open | Seg::CalView => 170.0,
        Seg::WeekStart => 190.0,
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
        Seg::Eyes => &EYE_NAMES,
        Seg::Size => &SIZE_NAMES,
        Seg::Open => &OPEN_NAMES,
        Seg::WeekStart => &WEEK_NAMES,
        Seg::CalView => &VIEW_NAMES,
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
    /// The changelog is shown in place of the page, scrolled this many logical pixels, and no further than `changelog_max`.
    show_changelog: bool,
    changelog_scroll: f32,
    changelog_max: f32,
    /// The page being shown, and since when (for its fade).
    page: Page,
    page_since: Instant,
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

    fn show(&mut self, page: Page) {
        // Choosing a page also leaves the changelog.
        if self.show_changelog {
            self.show_changelog = false;
            self.page_since = Instant::now();
            self.redraw();
        }
        if self.page != page {
            self.page = page;
            self.page_since = Instant::now();
            self.drag = None;
            self.redraw();
        }
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
            Toggle::Eyes => self.settings.eyes,
            Toggle::IdleEyes => self.settings.idle_eyes,
            Toggle::Minimal => self.settings.minimal,
            Toggle::EmptyShelf => self.settings.empty_shelf,
            Toggle::HudVolume => self.settings.hud_volume,
            Toggle::HudBrightness => self.settings.hud_brightness,
            Toggle::HudLock => self.settings.hud_lock,
            Toggle::HudBattery => self.settings.hud_battery,
            Toggle::HudNetwork => self.settings.hud_network,
            Toggle::HudAudio => self.settings.hud_audio,
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
            Toggle::Eyes => self.settings.eyes = !self.settings.eyes,
            Toggle::IdleEyes => self.settings.idle_eyes = !self.settings.idle_eyes,
            Toggle::Minimal => self.settings.minimal = !self.settings.minimal,
            Toggle::EmptyShelf => self.settings.empty_shelf = !self.settings.empty_shelf,
            Toggle::HudVolume => self.settings.hud_volume = !self.settings.hud_volume,
            Toggle::HudBrightness => self.settings.hud_brightness = !self.settings.hud_brightness,
            Toggle::HudLock => self.settings.hud_lock = !self.settings.hud_lock,
            Toggle::HudBattery => self.settings.hud_battery = !self.settings.hud_battery,
            Toggle::HudNetwork => self.settings.hud_network = !self.settings.hud_network,
            Toggle::HudAudio => self.settings.hud_audio = !self.settings.hud_audio,
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
            Seg::Eyes => EyeStyle::ALL.iter().position(|e| *e == self.settings.eye_style).unwrap_or(0),
            Seg::Size => NotchSize::ALL.iter().position(|z| *z == self.settings.size).unwrap_or(0),
            Seg::Open => usize::from(self.settings.open_on_click),
            Seg::WeekStart => usize::from(self.settings.week_monday),
            Seg::CalView => usize::from(self.settings.calendar_week),
        }
    }

    fn set_seg(&mut self, seg: Seg, i: usize) {
        match seg {
            Seg::Theme => self.settings.theme = ThemeMode::ALL[i.min(2)],
            Seg::Motion => self.settings.motion = Motion::ALL[i.min(3)],
            Seg::Eyes => self.settings.eye_style = EyeStyle::ALL[i.min(3)],
            Seg::Size => self.settings.size = NotchSize::ALL[i.min(4)],
            Seg::Open => self.settings.open_on_click = i == 1,
            Seg::WeekStart => self.settings.week_monday = i == 1,
            Seg::CalView => self.settings.calendar_week = i == 1,
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
            show_changelog: false,
            changelog_scroll: 0.0,
            changelog_max: 0.0,
            page: Page::General,
            page_since: Instant::now() - PAGE_FADE,
            reset_armed: None,
            last_draw: Instant::now(),
            opened: Instant::now(),
            busy: false,
        });
    }

    /// Switch the open settings window to its About page.
    pub(crate) fn show_about_page(&mut self) {
        if let Some(p) = &mut self.panel {
            p.show(Page::About);
        }
    }

    /// The page the settings window shows.
    fn page(&self) -> Page {
        self.panel.as_ref().map_or(Page::General, |p| p.page)
    }

    /// An event for the settings window.
    pub(crate) fn panel_event(&mut self, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.panel = None,
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let page = self.page();
                let step = |delta: isize| Page::ALL[(page.index() as isize + delta).clamp(0, Page::ALL.len() as isize - 1) as usize];
                match event.logical_key {
                    Key::Named(NamedKey::Escape) => match &mut self.panel {
                        Some(p) if p.show_changelog => {
                            p.show_changelog = false;
                            p.page_since = Instant::now();
                            p.redraw();
                        }
                        _ => self.panel = None,
                    },
                    Key::Named(NamedKey::ArrowDown) => {
                        if let Some(p) = &mut self.panel {
                            p.show(step(1));
                        }
                    }
                    Key::Named(NamedKey::ArrowUp) => {
                        if let Some(p) = &mut self.panel {
                            p.show(step(-1));
                        }
                    }
                    _ => {}
                }
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
                let (cx, drag, page) = (p.cursor.0, p.drag, p.page);
                p.redraw();
                if let Some(slide) = drag {
                    let lay = layout(page);
                    if let Some(item) = lay.items.iter().find(|i| matches!(i.kind, Kind::Slider(k) if k == slide)) {
                        let v = slider_value(range_of(slide), item.ctrl, cx);
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
                // The sidebar: a page.
                if cx < SIDEBAR_W {
                    if let Some(page) = Page::ALL.into_iter().find(|pg| inside(side_rect(*pg), cx, cy))
                        && let Some(p) = &mut self.panel {
                            p.show(page);
                        }
                    return;
                }
                if p.show_changelog {
                    // Only the Back button does anything here.
                    if inside(CHANGELOG_BACK, cx, cy)
                        && let Some(p) = &mut self.panel {
                            p.show_changelog = false;
                            p.page_since = Instant::now();
                            p.redraw();
                        }
                    return;
                }
                let page = p.page;
                let lay = layout(page);
                let Some(item) = lay.items.iter().find(|i| match i.kind {
                    Kind::Slider(_) => inside((i.ctrl.0 - 8.0, i.row.1, SLIDER_W + 16.0, i.row.3), cx, cy),
                    Kind::Seg(_) | Kind::Pick | Kind::Button(_) => inside(i.ctrl, cx, cy),
                    Kind::Link(url) => !url.is_empty() && inside(i.row, cx, cy),
                    Kind::Toggle(_) => inside(i.row, cx, cy),
                }) else {
                    return;
                };
                match item.kind {
                    Kind::Link(url) => {
                        let _ = std::process::Command::new("explorer").arg(url).spawn();
                    }
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
                        if self.settings.alarm_sound.is_some() && cx >= item.ctrl.0 + item.ctrl.2 - 30.0 {
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
                    Kind::Button(Act::Changelog) => {
                        if let Some(p) = &mut self.panel {
                            p.show_changelog = true;
                            p.changelog_scroll = 0.0;
                            p.page_since = Instant::now();
                            p.redraw();
                        }
                        if matches!(self.changelog, crate::update::Changelog::Idle | crate::update::Changelog::Failed) {
                            self.changelog = crate::update::Changelog::Loading;
                            crate::update::spawn_changelog(self.proxy.clone());
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
                                } else {
                                    // The file was removed or cannot be run: offer the download again.
                                    self.update = crate::update::Status::DownloadFailed;
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
            WindowEvent::MouseWheel { delta, .. } => {
                let Some(p) = &mut self.panel else { return };
                if p.show_changelog {
                    let dy = match delta {
                        winit::event::MouseScrollDelta::LineDelta(_, y) => y * 54.0,
                        winit::event::MouseScrollDelta::PixelDelta(d) => d.y as f32,
                    };
                    p.changelog_scroll = (p.changelog_scroll - dy).clamp(0.0, p.changelog_max);
                    p.redraw();
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

    /// What the last update check found, with the colour to show it in.
    fn update_note(&self) -> (String, [u8; 4]) {
        let (grey, green, amber, red) = ([165, 165, 175, 255], [90, 210, 130, 255], [255, 190, 70, 255], [255, 130, 120, 255]);
        match &self.update {
            crate::update::Status::Idle => (format!("You have version {}", env!("CARGO_PKG_VERSION")), grey),
            crate::update::Status::Checking => ("Checking...".to_string(), grey),
            crate::update::Status::Latest => ("You have the latest version".to_string(), green),
            crate::update::Status::Available { version, .. } => (format!("Version {version} is available"), amber),
            crate::update::Status::Downloading { percent, .. } => (format!("Downloading... {percent}%"), grey),
            crate::update::Status::Ready { .. } => ("Downloaded. Ready to install".to_string(), green),
            crate::update::Status::Failed => ("Could not check. Are you online?".to_string(), red),
            crate::update::Status::DownloadFailed => ("Download failed. Try again".to_string(), red),
        }
    }

    fn paint_panel(&self, panel: &mut Panel) {
        theme::set_light(self.light());
        let now = Instant::now();
        let speed = self.settings.motion.speed() as f32;
        panel.glows.set_motion(speed as f64, self.settings.motion.bounce());
        panel.glows.begin(now.duration_since(panel.last_draw).as_secs_f64().min(1.0 / 30.0));
        panel.last_draw = now;

        let size = panel.window.inner_size();
        let (Some(w), Some(h)) = (std::num::NonZeroU32::new(size.width), std::num::NonZeroU32::new(size.height)) else {
            return;
        };
        let Some(mut pm) = Pixmap::new(w.get(), h.get()) else { return };
        let s = panel.window.scale_factor() as f32;
        let (fw, fh) = (w.get() as f32, h.get() as f32);
        let page = panel.page;
        let tint = page.tint();

        // The background: a dark gradient with a soft glow of the page's colour in the top right corner.
        gfx::gradient_rounded(&mut pm, 0.0, 0.0, fw, fh, 0.0, [13, 13, 18, 255], [24, 24, 32, 255], false);
        gfx::glow(&mut pm, fw * 0.92, fh * 0.04, fw * 0.55, [tint[0], tint[1], tint[2], 20], None);

        // Everything else is drawn on a layer, which glides up and fades in when the window opens.
        let mut layer = Pixmap::new(w.get(), h.get()).unwrap_or_else(|| Pixmap::new(1, 1).unwrap());
        if let Some(font) = &self.font {
            let heading = self.heading_font().unwrap_or(font);
            self.paint_sidebar(&mut layer, panel, font, heading, s, fh);

            // The page: its title, and its groups on a layer of its own that fades in when it is chosen.
            let mut content = Pixmap::new(w.get(), h.get()).unwrap_or_else(|| Pixmap::new(1, 1).unwrap());
            if panel.show_changelog {
                self.paint_changelog(&mut content, panel, font, heading, tint, s);
            } else {
                crate::draw_text(&mut content, heading, page.title(), CONTENT_X * s, 52.0 * s, 24.0 * s, [255, 255, 255, 255], 400.0 * s);
                crate::draw_text(&mut content, font, page.subtitle(), CONTENT_X * s, 74.0 * s, 11.5 * s, [150, 150, 164, 255], CONTENT_W * s);
                let lay = layout(page);
                self.paint_page(&mut content, panel, font, heading, &lay, tint, s);
            }
            let x = (panel.page_since.elapsed().as_secs_f32() * speed / PAGE_FADE.as_secs_f32()).clamp(0.0, 1.0);
            let ease = 1.0 - (1.0 - x).powi(3);
            let paint = tiny_skia::PixmapPaint { opacity: ease, ..Default::default() };
            layer.draw_pixmap(0, 0, content.as_ref(), &paint, Transform::from_translate((1.0 - ease) * 14.0 * s, 0.0), None);
        }

        // The entrance: the layer rises into place and fades in.
        let x = (panel.opened.elapsed().as_secs_f32() * speed / ENTER.as_secs_f32()).clamp(0.0, 1.0);
        let ease = 1.0 - (1.0 - x).powi(3);
        let paint = tiny_skia::PixmapPaint { opacity: ease, ..Default::default() };
        pm.draw_pixmap(0, 0, layer.as_ref(), &paint, Transform::from_translate(0.0, (1.0 - ease) * 16.0 * s), None);

        // How far the changelog can scroll depends on its length, which is known once it has been laid out.
        if panel.show_changelog
            && let Some(font) = &self.font {
                let (_, total) = self.changelog_lines(font, self.heading_font().unwrap_or(font), CONTENT_W - 44.0);
                panel.changelog_max = (total - (HEIGHT as f32 - CHANGELOG_TOP - 28.0)).max(0.0);
                panel.changelog_scroll = panel.changelog_scroll.clamp(0.0, panel.changelog_max);
            }

        let Ok(()) = panel.surface.resize(w, h) else { return };
        let Ok(mut buffer) = panel.surface.buffer_mut() else { return };
        for (dst, src) in buffer.iter_mut().zip(pm.pixels()) {
            *dst = (src.alpha() as u32) << 24 | (src.red() as u32) << 16 | (src.green() as u32) << 8 | src.blue() as u32;
        }
        buffer.present().ok();
        panel.busy = panel.glows.busy() || panel.opened.elapsed().mul_f32(speed) < ENTER || panel.page_since.elapsed().mul_f32(speed) < PAGE_FADE;
    }

    /// The sidebar: the app's name, the pages with a highlight that glides between them, and Quit.
    fn paint_sidebar(&self, pm: &mut Pixmap, panel: &Panel, font: &fontdue::Font, heading: &fontdue::Font, s: f32, fh: f32) {
        let white = [255, 255, 255, 255];
        let (cx, cy) = panel.cursor;
        // The sidebar is a slightly darker panel with a line on its right edge.
        crate::fill_rect(pm, 0.0, 0.0, SIDEBAR_W * s, fh, [0, 0, 0, 70]);
        crate::fill_rect(pm, (SIDEBAR_W - 1.0) * s, 0.0, s, fh, [255, 255, 255, 14]);

        // The name at the top.
        let logo = appicon::pixmap(64);
        let scale = 26.0 * s / logo.width() as f32;
        let paint = tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
        pm.draw_pixmap(0, 0, logo.as_ref(), &paint, Transform::from_scale(scale, scale).post_translate(18.0 * s, 16.0 * s), None);
        crate::draw_text(pm, heading, "Sky Island", 52.0 * s, 35.0 * s, 15.0 * s, white, 150.0 * s);

        // The highlight glides to the chosen page, tinted with that page's colour.
        let pos = panel.glows.follow(KEY_SIDE, panel.page.index() as f64, 30.0, 0.8) as f32;
        let tint = panel.page.tint();
        let (hx, hy, hw, hh) = (12.0, SIDE_TOP + pos * SIDE_ITEM_H, SIDEBAR_W - 24.0, SIDE_ITEM_H - 4.0);
        gfx::gradient_rounded(pm, hx * s, hy * s, hw * s, hh * s, 10.0 * s, [255, 255, 255, 30], [255, 255, 255, 16], false);
        gfx::stroke_rounded(pm, hx * s + 0.5 * s, hy * s + 0.5 * s, hw * s - s, hh * s - s, 10.0 * s, s, [tint[0], tint[1], tint[2], 90]);

        for (i, page) in Page::ALL.into_iter().enumerate() {
            let r = side_rect(page);
            let hot = panel.glows.fade(KEY_SIDE + 1 + i as u64, inside(r, cx, cy));
            if hot > 0.01 && page != panel.page {
                crate::fill_rounded(pm, r.0 * s, r.1 * s, r.2 * s, r.3 * s, 10.0 * s, [255, 255, 255, (12.0 * hot) as u8]);
            }
            let near = 1.0 - (pos - i as f32).abs().min(1.0);
            draw_page_icon(pm, page, r.0 + 8.0, r.1 + (r.3 - 26.0) / 2.0, s);
            let ink = crate::pick(white, [200, 200, 212, 255], near.max(hot * 0.6));
            crate::draw_text(pm, font, page.title(), (r.0 + 44.0) * s, (r.1 + r.3 / 2.0 + 4.5) * s, 13.0 * s, ink, (r.2 - 52.0) * s);
        }

        // The version, at the bottom. (Quitting is in the tray menu.)
        let version = format!("Version {}", env!("CARGO_PKG_VERSION"));
        crate::draw_text_centered(pm, font, &version, SIDEBAR_W / 2.0 * s, VERSION_Y * s, 10.5 * s, [130, 130, 144, 255], (SIDEBAR_W - 24.0) * s);
    }

    /// The groups of a page as cards with their rows.
    #[allow(clippy::too_many_arguments)]
    fn paint_page(&self, pm: &mut Pixmap, panel: &Panel, font: &fontdue::Font, heading: &fontdue::Font, lay: &Layout, tint: [u8; 3], s: f32) {
        let white = [255, 255, 255, 255];
        let grey = [160, 160, 172, 255];

        if let Some(hero) = lay.hero {
            if panel.page == Page::Calendar {
                crate::draw_text(pm, font, "Preview", (hero.0 + 4.0) * s, (hero.1 - 8.0) * s, 11.5 * s, grey, 300.0 * s);
                self.paint_calendar_preview(pm, font, heading, hero, tint, s);
            } else {
                self.paint_hero(pm, font, heading, hero, tint, s);
            }
        }
        for g in &lay.groups {
            crate::draw_text(pm, font, g.title, (g.rect.0 + 4.0) * s, (g.rect.1 - 8.0) * s, 11.5 * s, grey, 300.0 * s);
            crate::draw_card(pm, g.rect, 14.0, s, 1.0, 0.0, tint);
        }
        for (idx, item) in lay.items.iter().enumerate() {
            let key = 0x2000_0000 + idx as u64 * 16;
            let (cx, cy) = panel.cursor;
            let row_hot = !matches!(item.kind, Kind::Link("")) && inside(item.row, cx, cy);
            let hot = panel.glows.fade(key, row_hot);
            if hot > 0.01 {
                crate::fill_rounded(pm, (item.row.0 + 4.0) * s, (item.row.1 + 3.0) * s, (item.row.2 - 8.0) * s, (item.row.3 - 6.0) * s, 10.0 * s, [255, 255, 255, (11.0 * hot) as u8]);
            }
            if item.after_row {
                crate::fill_rect(pm, (item.row.0 + 18.0) * s, item.row.1 * s, (item.row.2 - 36.0) * s, s * 0.7, [255, 255, 255, 13]);
            }
            // The label and the line under it. The About page's update row shows what the check found.
            let (desc, desc_ink) = match item.kind {
                Kind::Button(Act::CheckUpdate) => self.update_note(),
                Kind::Button(Act::Reset) if panel.reset_armed.is_some_and(|t| Instant::now() < t) => ("Click Reset again within 3 seconds to confirm".to_string(), [255, 150, 140, 255]),
                _ => (item.desc.to_string(), [146, 146, 160, 255]),
            };
            let text_w = item.ctrl.0 - item.row.0 - 36.0;
            // A row with no line under its label has the label in the middle.
            let label_y = if desc.is_empty() { item.row.3 / 2.0 + 5.0 } else { 24.0 };
            crate::draw_text(pm, font, item.label, (item.row.0 + 18.0) * s, (item.row.1 + label_y) * s, 13.5 * s, crate::pick(white, [232, 232, 240, 255], hot), text_w * s);
            crate::draw_text(pm, font, &desc, (item.row.0 + 18.0) * s, (item.row.1 + 42.0) * s, 11.0 * s, desc_ink, text_w * s);
            self.paint_control(pm, panel, font, heading, item, key, tint, s);
        }
    }

    /// The changelog as lines at their place (relative to the top of the list), and the height of all of it.
    /// `width` is how wide the text may be, in logical pixels.
    fn changelog_lines(&self, font: &fontdue::Font, heading: &fontdue::Font, width: f32) -> (Vec<ChangeLine>, f32) {
        use crate::update::{Changelog, Note};
        // Only letters the font has are drawn, so symbols and emoji in the notes do not show as boxes.
        let usable = |text: &str| -> String { text.chars().filter(|c| c.is_whitespace() || font.lookup_glyph_index(*c) != 0).collect::<String>().trim().to_string() };
        let mut lines = Vec::new();
        let mut y = 0.0;
        let push = |lines: &mut Vec<ChangeLine>, y: &mut f32, kind: ChangeKind, text: String, step: f32| {
            lines.push(ChangeLine { kind, text, y: *y });
            *y += step;
        };
        match &self.changelog {
            Changelog::Idle | Changelog::Loading => push(&mut lines, &mut y, ChangeKind::Plain, "Loading the changelog...".to_string(), 20.0),
            Changelog::Failed => push(&mut lines, &mut y, ChangeKind::Plain, "Could not load the changelog. Are you online?".to_string(), 20.0),
            Changelog::Ready(releases) => {
                for (i, release) in releases.iter().enumerate() {
                    if i > 0 {
                        y += 14.0;
                    }
                    push(&mut lines, &mut y, ChangeKind::Version(release.date.clone()), format!("Version {}", release.version), 30.0);
                    let notes = crate::update::notes_of(&release.body);
                    if notes.is_empty() {
                        push(&mut lines, &mut y, ChangeKind::Plain, "No notes for this version.".to_string(), 20.0);
                    }
                    for note in notes {
                        match note {
                            Note::Heading(h) => {
                                y += 4.0;
                                push(&mut lines, &mut y, ChangeKind::Heading, usable(&h), 22.0);
                            }
                            Note::Bullet(b) => {
                                for (k, row) in crate::wrap(font, &usable(&b), 12.0, width - 18.0).into_iter().enumerate() {
                                    push(&mut lines, &mut y, if k == 0 { ChangeKind::Bullet } else { ChangeKind::Plain }, row, 17.0);
                                }
                                y += 3.0;
                            }
                            Note::Text(t) => {
                                for row in crate::wrap(font, &usable(&t), 12.0, width - 18.0) {
                                    push(&mut lines, &mut y, ChangeKind::Plain, row, 17.0);
                                }
                                y += 3.0;
                            }
                        }
                    }
                }
            }
        }
        let _ = heading;
        lines.retain(|l| !l.text.is_empty());
        (lines, y)
    }

    /// The changelog in place of a page: its title, a Back button, and the releases in a card that scrolls.
    fn paint_changelog(&self, pm: &mut Pixmap, panel: &Panel, font: &fontdue::Font, heading: &fontdue::Font, tint: [u8; 3], s: f32) {
        let (white, grey) = ([255, 255, 255, 255], [160, 160, 172, 255]);
        crate::draw_text(pm, heading, "Changelog", CONTENT_X * s, 52.0 * s, 24.0 * s, white, 400.0 * s);
        crate::draw_text(pm, font, "What changed in each version, from the project's releases on GitHub.", CONTENT_X * s, 74.0 * s, 11.5 * s, [150, 150, 164, 255], CONTENT_W * s);

        // The Back button.
        let (cx, cy) = panel.cursor;
        let hot = panel.glows.fade(KEY_SIDE + 30, inside(CHANGELOG_BACK, cx, cy));
        let (x, y, w, h) = (CHANGELOG_BACK.0 * s, CHANGELOG_BACK.1 * s, CHANGELOG_BACK.2 * s, CHANGELOG_BACK.3 * s);
        gfx::gradient_rounded(pm, x, y, w, h, h / 2.0, [tint[0], tint[1], tint[2], crate::pick(110.0, 62.0, hot) as u8], [tint[0], tint[1], tint[2], crate::pick(66.0, 30.0, hot) as u8], false);
        gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [tint[0], tint[1], tint[2], 130]);
        let tip = [(x + 17.0 * s, y + h / 2.0 - 4.0 * s), (x + 13.0 * s, y + h / 2.0), (x + 17.0 * s, y + h / 2.0 + 4.0 * s)];
        stroke_points(pm, &tip, false, 1.6 * s, white);
        crate::draw_text_centered(pm, heading, "Back", x + w / 2.0 + 6.0 * s, y + 19.0 * s, 12.0 * s, white, w - 30.0 * s);

        // The card with the releases.
        let card = (CONTENT_X, CHANGELOG_TOP, CONTENT_W, HEIGHT as f32 - CHANGELOG_TOP - 18.0);
        crate::draw_card(pm, card, 14.0, s, 1.0, 0.0, tint);
        let (lines, _) = self.changelog_lines(font, heading, CONTENT_W - 44.0);
        let (top, bottom) = (card.1 + 8.0, card.1 + card.3 - 8.0);
        let accent = [tint[0], tint[1], tint[2], 255];
        for line in &lines {
            let ly = top + 6.0 + line.y - panel.changelog_scroll;
            // Only lines that fit completely inside the card are drawn, so none is cut in half.
            if ly < top || ly + 18.0 > bottom {
                continue;
            }
            let x = card.0 + 22.0;
            match &line.kind {
                ChangeKind::Version(date) => {
                    crate::draw_text(pm, heading, &line.text, x * s, (ly + 15.0) * s, 15.0 * s, white, (CONTENT_W - 150.0) * s);
                    if !date.is_empty() {
                        let tw = crate::text_width(font, date, 11.0 * s);
                        crate::draw_text(pm, font, date, (card.0 + card.2 - 22.0) * s - tw, (ly + 14.0) * s, 11.0 * s, grey, 100.0 * s);
                    }
                    crate::fill_rect(pm, x * s, (ly + 22.0) * s, (card.2 - 44.0) * s, s, [255, 255, 255, 22]);
                }
                ChangeKind::Heading => crate::draw_text(pm, heading, &line.text, x * s, (ly + 13.0) * s, 12.5 * s, accent, (CONTENT_W - 44.0) * s),
                ChangeKind::Bullet => {
                    crate::fill_circle(pm, (x + 4.0) * s, (ly + 8.0) * s, 2.0 * s, accent);
                    crate::draw_text(pm, font, &line.text, (x + 18.0) * s, (ly + 12.0) * s, 12.0 * s, [220, 220, 230, 255], (CONTENT_W - 62.0) * s);
                }
                ChangeKind::Plain => crate::draw_text(pm, font, &line.text, (x + 18.0) * s, (ly + 12.0) * s, 12.0 * s, [200, 200, 212, 255], (CONTENT_W - 62.0) * s),
            }
        }
        // A scroll bar when there is more than fits.
        if panel.changelog_max > 0.0 {
            let (track_top, track_h) = (top + 6.0, bottom - top - 12.0);
            let thumb = (track_h * track_h / (track_h + panel.changelog_max)).max(26.0);
            let at = track_top + (track_h - thumb) * panel.changelog_scroll / panel.changelog_max;
            crate::fill_rounded(pm, (card.0 + card.2 - 9.0) * s, track_top * s, 3.0 * s, track_h * s, 1.5 * s, [255, 255, 255, 18]);
            crate::fill_rounded(pm, (card.0 + card.2 - 9.0) * s, at * s, 3.0 * s, thumb * s, 1.5 * s, accent);
        }
    }

    /// A small calendar for today's month (or week) as the settings below make it look.
    fn paint_calendar_preview(&self, pm: &mut Pixmap, font: &fontdue::Font, heading: &fontdue::Font, hero: Rect, tint: [u8; 3], s: f32) {
        let (x, y, w, _) = hero;
        crate::draw_card(pm, hero, 16.0, s, 1.0, 0.0, tint);
        let (white, grey, muted) = ([255, 255, 255, 255], [160, 160, 172, 255], [96, 96, 108, 255]);
        let (monday, week) = (self.settings.week_monday, self.settings.calendar_week);
        let today = crate::today_day();
        let (year, month, _) = crate::calendar::civil_from_days(today);
        let first = crate::calendar::days_from_civil(year, month, 1);
        let heads = crate::calendar::week_heads(monday);
        let (cell_w, cell_h) = (42.0, if week { 62.0 } else { 27.0 });
        let left = x + (w - 7.0 * cell_w) / 2.0;
        let title = if week {
            crate::calendar::week_title(crate::calendar::week_start(today, monday))
        } else {
            format!("{} {year}", ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"][month as usize - 1])
        };
        crate::draw_text_centered(pm, heading, &title, (x + w / 2.0) * s, (y + 28.0) * s, 14.0 * s, [210, 210, 220, 255], (w - 40.0) * s);
        for (c, head) in heads.iter().enumerate() {
            let ink = if crate::calendar::is_weekend(c, monday) { gfx::mix(grey, [tint[0], tint[1], tint[2], 255], 0.5) } else { grey };
            crate::draw_text_centered(pm, font, head, (left + (c as f32 + 0.5) * cell_w) * s, (y + 52.0) * s, 10.5 * s, ink, cell_w * s);
        }
        let top = y + 62.0;
        let start = if week { crate::calendar::week_start(today, monday) } else { first - crate::calendar::column(first, monday) as i64 };
        let rows = if week { 1 } else { 6 };
        for r in 0..rows {
            for c in 0..7 {
                let day = start + (r * 7 + c) as i64;
                let (_, m, dom) = crate::calendar::civil_from_days(day);
                let (cx, cy) = (left + (c as f32 + 0.5) * cell_w, top + r as f32 * cell_h + cell_h / 2.0);
                let is_today = day == today;
                if is_today {
                    let r = if week { 20.0 } else { 11.0 };
                    gfx::glow(pm, cx * s, cy * s, (r + 8.0) * s, [tint[0], tint[1], tint[2], 60], None);
                    crate::fill_circle(pm, cx * s, cy * s, r * s, [tint[0], tint[1], tint[2], 255]);
                }
                let ink = if is_today { white } else if m != month && !week { muted } else { [214, 214, 224, 255] };
                crate::draw_text_centered(pm, if is_today { heading } else { font }, &dom.to_string(), cx * s, (cy + if week { 6.0 } else { 4.5 }) * s, if week { 18.0 } else { 12.0 } * s, ink, cell_w * s);
                let (open, total) = self.todo.day_counts(day);
                if total > 0 {
                    let dot = if is_today { white } else if open > 0 { [255, 190, 70, 255] } else { [110, 110, 120, 255] };
                    crate::fill_circle(pm, cx * s, (cy + if week { 20.0 } else { 11.0 }) * s, 1.6 * s, dot);
                }
            }
        }
    }

    /// The card at the top of the About page.
    fn paint_hero(&self, pm: &mut Pixmap, font: &fontdue::Font, heading: &fontdue::Font, hero: Rect, tint: [u8; 3], s: f32) {
        let (x, y, w, h) = hero;
        crate::draw_card(pm, hero, 16.0, s, 1.0, 0.0, tint);
        if let Some(path) = gfx::rounded_path(x * s, y * s, w * s, h * s, 16.0 * s)
            && let Some(mask) = gfx::mask_of(pm.width(), pm.height(), &path) {
                gfx::glow(pm, (x + 60.0) * s, (y + 50.0) * s, 170.0 * s, [tint[0], tint[1], tint[2], 36], Some(&mask));
            }
        let logo = appicon::pixmap(128);
        let scale = 64.0 * s / logo.width() as f32;
        let paint = tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
        pm.draw_pixmap(0, 0, logo.as_ref(), &paint, Transform::from_scale(scale, scale).post_translate((x + 22.0) * s, (y + 12.0) * s), None);
        crate::draw_text(pm, heading, "Sky Island", (x + 102.0) * s, (y + 36.0) * s, 21.0 * s, [255, 255, 255, 255], 300.0 * s);
        crate::draw_text(pm, font, "A notch for the top of your Windows screen.", (x + 102.0) * s, (y + 55.0) * s, 11.5 * s, [170, 170, 182, 255], (w - 120.0) * s);
        let version = format!("Version {}", env!("CARGO_PKG_VERSION"));
        crate::draw_text(pm, font, &version, (x + 102.0) * s, (y + 73.0) * s, 11.0 * s, [tint[0], tint[1], tint[2], 255], 200.0 * s);
    }

    /// The control on the right of a row.
    #[allow(clippy::too_many_arguments)]
    fn paint_control(&self, pm: &mut Pixmap, panel: &Panel, font: &fontdue::Font, heading: &fontdue::Font, item: &Item, key: u64, tint: [u8; 3], s: f32) {
        let white = [255, 255, 255, 255];
        let grey = [165, 165, 175, 255];
        let (cx, cy) = panel.cursor;
        let accent = tint;

        match item.kind {
            Kind::Link(url) => {
                // The value on the right, and an arrow when the line opens something.
                let right = (item.row.0 + item.row.2 - 18.0) * s;
                let (tw, y) = (crate::text_width(font, item.btn, 12.0 * s), (item.row.1 + item.row.3 / 2.0 + 4.5) * s);
                if url.is_empty() {
                    crate::draw_text(pm, font, item.btn, right - tw, y, 12.0 * s, grey, 240.0 * s);
                } else {
                    let tip = [(right - 5.0 * s, y - 9.0 * s), (right, y - 5.0 * s), (right - 5.0 * s, y - 1.0 * s)];
                    stroke_points(pm, &tip, false, 1.5 * s, [165, 165, 175, 255]);
                    crate::draw_text(pm, font, item.btn, right - 16.0 * s - tw, y, 12.0 * s, [200, 200, 212, 255], 240.0 * s);
                }
            }
            Kind::Toggle(t) => {
                let on = panel.glows.follow(key + 1, if self.toggle_value(t) { 1.0 } else { 0.0 }, 34.0, 0.62) as f32;
                // The shared switch is 32 x 18; it is drawn 1.25 times as big here.
                crate::draw_switch(pm, item.ctrl.0 * s, item.ctrl.1 * s, s * 1.25, on, 255);
            }
            Kind::Seg(seg) => {
                let names = seg_names(seg);
                let cells = seg_cells(seg, item.ctrl);
                let (x, y, w, h) = (item.ctrl.0 * s, item.ctrl.1 * s, item.ctrl.2 * s, item.ctrl.3 * s);
                crate::fill_rounded(pm, x, y, w, h, h / 2.0, [255, 255, 255, 18]);
                // The highlight slides (and changes width) to the chosen option, the way the tab bar's circle
                // does: it stretches in the direction it is moving, squashes a little, and has a soft halo.
                let pos = panel.glows.follow(key + 2, self.seg_value(seg) as f64, 28.0, 0.7) as f32;
                let stretch = (panel.glows.velocity(key + 2).abs() as f32 * 7.0).min(30.0);
                let (i, f) = ((pos.floor().max(0.0) as usize).min(cells.len() - 1), pos - pos.floor().max(0.0));
                let (a, b) = (cells[i], cells[(i + 1).min(cells.len() - 1)]);
                let (px, pw) = (a.0 + (b.0 - a.0) * f, a.2 + (b.2 - a.2) * f);
                let (px, pw) = ((px - stretch / 2.0) * s + 2.0 * s, (pw + stretch) * s - 4.0 * s);
                let squash = stretch * 0.1 * s;
                let (hy, hh) = (y + 2.0 * s + squash / 2.0, h - 4.0 * s - squash);
                gfx::gradient_rounded(pm, px - 2.0 * s, hy - 2.0 * s, pw + 4.0 * s, hh + 4.0 * s, (hh + 4.0 * s) / 2.0, [accent[0], accent[1], accent[2], 26], [accent[0], accent[1], accent[2], 10], false);
                gfx::gradient_rounded(pm, px, hy, pw, hh, hh / 2.0, [accent[0], accent[1], accent[2], 140], [accent[0], accent[1], accent[2], 86], false);
                gfx::stroke_rounded(pm, px + 0.5 * s, hy + 0.5 * s, pw - s, hh - s, hh / 2.0, s, [accent[0], accent[1], accent[2], 160]);
                for (i, name) in names.iter().enumerate() {
                    let cell = cells[i];
                    let near = 1.0 - (pos - i as f32).abs().min(1.0);
                    let over = panel.glows.fade(key + 3 + i as u64, inside(cell, cx, cy));
                    let ink = crate::pick(white, [150, 150, 160, 255], near.max(over * 0.7));
                    crate::draw_text_centered(pm, font, name, (cell.0 + cell.2 / 2.0) * s, (cell.1 + 18.5) * s, 12.0 * s, ink, (cell.2 - 6.0) * s);
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
                gfx::gradient_rounded(pm, x, y, fill_w, h, h / 2.0, [accent[0], accent[1], accent[2], 210], [255, 255, 255, 210], true);
                let kx = x + w * frac;
                gfx::glow(pm, kx, y + h / 2.0, (12.0 + 4.0 * grab) * s, [accent[0], accent[1], accent[2], (60.0 + 60.0 * grab) as u8], None);
                // White in both looks (the light look turns white into dark, so ask for black there).
                let knob = if theme::is_light() { [0, 0, 0, 255] } else { white };
                crate::fill_circle(pm, kx, y + h / 2.0, (6.5 + 1.5 * grab) * s, knob);
                let text = slider_text(slide, v);
                let tw = crate::text_width(font, &text, 12.0 * s);
                crate::draw_text(pm, font, &text, (item.row.0 + item.row.2 - 18.0) * s - tw, (item.row.1 + item.row.3 / 2.0 + 4.5) * s, 12.0 * s, grey, SLIDER_TEXT_W * s);
            }
            Kind::Pick => {
                let (x, y, w, h) = (item.ctrl.0 * s, item.ctrl.1 * s, item.ctrl.2 * s, item.ctrl.3 * s);
                let over = panel.glows.fade(key + 9, inside(item.ctrl, cx, cy));
                gfx::gradient_rounded(pm, x, y, w, h, h / 2.0, [accent[0], accent[1], accent[2], crate::pick(110.0, 64.0, over) as u8], [accent[0], accent[1], accent[2], crate::pick(66.0, 32.0, over) as u8], false);
                gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [accent[0], accent[1], accent[2], 130]);
                let custom = self.settings.alarm_sound.as_ref();
                let name = custom.and_then(|p| p.file_name()).map_or_else(|| "Built-in chime".to_string(), |n| n.to_string_lossy().into_owned());
                let text_w = if custom.is_some() { item.ctrl.2 - 44.0 } else { item.ctrl.2 - 20.0 };
                crate::draw_text_centered(pm, heading, &name, (item.ctrl.0 + 10.0 + text_w / 2.0) * s, (item.ctrl.1 + 18.5) * s, 12.0 * s, white, text_w * s);
                if custom.is_some() {
                    let (bx, by) = ((item.ctrl.0 + item.ctrl.2 - 16.0) * s, (item.ctrl.1 + item.ctrl.3 / 2.0) * s);
                    let hot_x = panel.glows.fade(key + 10, cx >= item.ctrl.0 + item.ctrl.2 - 30.0 && inside(item.ctrl, cx, cy));
                    crate::fill_circle(pm, bx, by, 9.0 * s, [255, 96, 86, (70.0 * hot_x) as u8]);
                    crate::draw_text_centered(pm, font, "\u{00D7}", bx, by + 5.0 * s, 15.0 * s, crate::pick(white, grey, hot_x), 14.0 * s);
                }
            }
            Kind::Button(act) => {
                let armed = act == Act::Reset && panel.reset_armed.is_some_and(|t| Instant::now() < t);
                let over = panel.glows.fade(key + 9, inside(item.ctrl, cx, cy));
                let (x, y, w, h) = (item.ctrl.0 * s, item.ctrl.1 * s, item.ctrl.2 * s, item.ctrl.3 * s);
                let colour = if armed || act == Act::Reset { [255, 110, 96] } else { accent };
                gfx::gradient_rounded(pm, x, y, w, h, h / 2.0, [colour[0], colour[1], colour[2], crate::pick(110.0, 62.0, over) as u8], [colour[0], colour[1], colour[2], crate::pick(66.0, 30.0, over) as u8], false);
                gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [colour[0], colour[1], colour[2], 130]);
                let download = match &self.update {
                    _ if act != Act::CheckUpdate => None,
                    crate::update::Status::Available { version, .. } => Some(format!("Download {version}")),
                    crate::update::Status::Downloading { .. } => Some("Downloading...".to_string()),
                    crate::update::Status::Ready { version, .. } => Some(format!("Install {version}")),
                    _ => None,
                };
                let label = if armed { "Click again" } else { download.as_deref().unwrap_or(item.btn) };
                crate::draw_text_centered(pm, heading, label, (item.ctrl.0 + item.ctrl.2 / 2.0) * s, (item.ctrl.1 + 18.5) * s, 12.0 * s, white, (item.ctrl.2 - 14.0) * s);
            }
        }
    }
}

/// A white shape on the tile, drawn the same in the dark and the light look.
fn fill_poly(pm: &mut Pixmap, points: &[(f32, f32)], c: [u8; 4]) {
    let mut pb = PathBuilder::new();
    for (i, (x, y)) in points.iter().enumerate() {
        if i == 0 {
            pb.move_to(*x, *y);
        } else {
            pb.line_to(*x, *y);
        }
    }
    pb.close();
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        paint.anti_alias = true;
        pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }
}

/// A line (or ring) of a given width.
fn stroke_points(pm: &mut Pixmap, points: &[(f32, f32)], close: bool, width: f32, c: [u8; 4]) {
    let mut pb = PathBuilder::new();
    for (i, (x, y)) in points.iter().enumerate() {
        if i == 0 {
            pb.move_to(*x, *y);
        } else {
            pb.line_to(*x, *y);
        }
    }
    if close {
        pb.close();
    }
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        paint.anti_alias = true;
        let stroke = Stroke { width, line_cap: tiny_skia::LineCap::Round, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
        pm.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

/// Points along an arc of a circle, in degrees (0 is to the right, 90 is down).
fn arc_points(cx: f32, cy: f32, r: f32, from: f32, to: f32, steps: usize) -> Vec<(f32, f32)> {
    (0..=steps)
        .map(|i| {
            let a = (from + (to - from) * i as f32 / steps as f32).to_radians();
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

/// The coloured tile with a white picture on it, for a page in the sidebar. `x`, `y` is its top left.
fn draw_page_icon(pm: &mut Pixmap, page: Page, x: f32, y: f32, s: f32) {
    let t = page.tint();
    let from = gfx::mix([t[0], t[1], t[2], 255], [255, 255, 255, 255], 0.22);
    let to = [t[0], t[1], t[2], 255];
    let size = 26.0;
    gfx::gradient_rounded(pm, x * s, y * s, size * s, size * s, 7.0 * s, from, to, false);
    let (cx, cy) = ((x + size / 2.0) * s, (y + size / 2.0) * s);
    let w = [255, 255, 255, 255];
    let u = s; // one logical pixel
    match page {
        Page::General => {
            // A gear: a ring of teeth with a hole in the middle.
            let mut pts = Vec::new();
            for i in 0..16 {
                let a = (i as f32 * 22.5 - 11.25).to_radians();
                let r = if (i / 2) % 2 == 0 { 8.2 } else { 6.2 } * u;
                pts.push((cx + r * a.cos(), cy + r * a.sin()));
                let a2 = (i as f32 * 22.5 + 11.25).to_radians();
                pts.push((cx + r * a2.cos(), cy + r * a2.sin()));
            }
            fill_poly(pm, &pts, w);
            crate::fill_circle(pm, cx, cy, 2.8 * u, gfx::mix(from, to, 0.5));
        }
        Page::Notch => {
            // A screen with the notch at the top.
            gfx::stroke_rounded(pm, cx - 8.0 * u, cy - 6.0 * u, 16.0 * u, 12.0 * u, 2.5 * u, 1.6 * u, w);
            crate::fill_rounded(pm, cx - 3.4 * u, cy - 6.4 * u, 6.8 * u, 3.4 * u, 1.7 * u, w);
        }
        Page::Appearance => {
            // A circle, half of it filled.
            stroke_points(pm, &arc_points(cx, cy, 6.6 * u, 0.0, 360.0, 36), true, 1.6 * u, w);
            let mut half = arc_points(cx, cy, 6.6 * u, -90.0, 90.0, 18);
            half.push((cx, cy + 6.6 * u));
            fill_poly(pm, &half, w);
        }
        Page::Tabs => {
            // Four tiles.
            for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                crate::fill_rounded(pm, cx + dx * 3.5 * u - 3.0 * u, cy + dy * 3.5 * u - 3.0 * u, 6.0 * u, 6.0 * u, 1.6 * u, w);
            }
        }
        Page::Sounds => {
            // A speaker with two waves.
            fill_poly(pm, &[(cx - 7.5 * u, cy - 2.4 * u), (cx - 4.0 * u, cy - 2.4 * u), (cx + 0.6 * u, cy - 6.4 * u), (cx + 0.6 * u, cy + 6.4 * u), (cx - 4.0 * u, cy + 2.4 * u), (cx - 7.5 * u, cy + 2.4 * u)], w);
            stroke_points(pm, &arc_points(cx + 1.0 * u, cy, 4.2 * u, -48.0, 48.0, 8), false, 1.4 * u, w);
            stroke_points(pm, &arc_points(cx + 1.0 * u, cy, 7.4 * u, -48.0, 48.0, 8), false, 1.4 * u, w);
        }
        Page::Calendar => {
            // A page with a header strip and two rings, and a few dots for days.
            gfx::stroke_rounded(pm, cx - 7.5 * u, cy - 6.0 * u, 15.0 * u, 13.5 * u, 2.6 * u, 1.6 * u, w);
            crate::fill_rect(pm, cx - 7.5 * u, cy - 3.0 * u, 15.0 * u, 1.6 * u, w);
            crate::fill_rounded(pm, cx - 3.8 * u, cy - 8.0 * u, 1.8 * u, 3.4 * u, 0.9 * u, w);
            crate::fill_rounded(pm, cx + 2.0 * u, cy - 8.0 * u, 1.8 * u, 3.4 * u, 0.9 * u, w);
            for (dx, dy) in [(-3.4, 1.0), (0.0, 1.0), (3.4, 1.0), (-3.4, 4.2), (0.0, 4.2)] {
                crate::fill_circle(pm, cx + dx * u, cy + dy * u, 0.9 * u, w);
            }
        }
        Page::Shelf => {
            // An open tray with an arrow going in.
            stroke_points(pm, &[(cx - 7.5 * u, cy + 0.5 * u), (cx - 7.5 * u, cy + 6.5 * u), (cx + 7.5 * u, cy + 6.5 * u), (cx + 7.5 * u, cy + 0.5 * u)], false, 1.7 * u, w);
            stroke_points(pm, &[(cx - 7.5 * u, cy + 0.5 * u), (cx - 3.2 * u, cy + 0.5 * u), (cx - 2.0 * u, cy + 3.0 * u), (cx + 2.0 * u, cy + 3.0 * u), (cx + 3.2 * u, cy + 0.5 * u), (cx + 7.5 * u, cy + 0.5 * u)], false, 1.7 * u, w);
            stroke_points(pm, &[(cx, cy - 7.0 * u), (cx, cy - 0.5 * u)], false, 1.7 * u, w);
            fill_poly(pm, &[(cx - 3.2 * u, cy - 2.4 * u), (cx + 3.2 * u, cy - 2.4 * u), (cx, cy + 1.2 * u)], w);
        }
        Page::Notices => {
            // A bell.
            let mut bell = arc_points(cx, cy - 1.0 * u, 5.4 * u, 180.0, 360.0, 14);
            bell.extend([(cx + 5.4 * u, cy + 3.8 * u), (cx + 7.4 * u, cy + 5.2 * u), (cx - 7.4 * u, cy + 5.2 * u), (cx - 5.4 * u, cy + 3.8 * u)]);
            fill_poly(pm, &bell, w);
            crate::fill_circle(pm, cx, cy + 7.2 * u, 1.5 * u, w);
            crate::fill_rounded(pm, cx - 0.9 * u, cy - 7.6 * u, 1.8 * u, 2.6 * u, 0.9 * u, w);
        }
        Page::About => {
            // An "i" in a circle.
            stroke_points(pm, &arc_points(cx, cy, 7.4 * u, 0.0, 360.0, 36), true, 1.6 * u, w);
            crate::fill_circle(pm, cx, cy - 3.2 * u, 1.1 * u, w);
            crate::fill_rounded(pm, cx - 0.9 * u, cy - 0.8 * u, 1.8 * u, 5.4 * u, 0.9 * u, w);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_fits_inside_the_window() {
        for page in Page::ALL {
            let lay = layout(page);
            for g in &lay.groups {
                assert!(g.rect.0 >= SIDEBAR_W && g.rect.0 + g.rect.2 <= WIDTH as f32, "{} is outside on {}", g.title, page.title());
                assert!(g.rect.1 + g.rect.3 <= HEIGHT as f32 - 14.0, "{} runs past the bottom on {}", g.title, page.title());
            }
            for item in &lay.items {
                assert!(item.row.0 + item.row.2 <= WIDTH as f32, "{} is too wide", item.label);
                assert!(item.ctrl.0 >= item.row.0 && item.ctrl.0 + item.ctrl.2 <= item.row.0 + item.row.2, "{} control is outside its row", item.label);
            }
            if let Some(hero) = lay.hero {
                assert!(hero.1 + hero.3 <= HEIGHT as f32);
            }
        }
    }

    #[test]
    fn groups_on_a_page_do_not_overlap() {
        for page in Page::ALL {
            let lay = layout(page);
            for pair in lay.groups.windows(2) {
                assert!(pair[0].rect.1 + pair[0].rect.3 < pair[1].rect.1, "{} overlaps {} on {}", pair[0].title, pair[1].title, page.title());
            }
        }
    }

    #[test]
    fn labels_have_room_next_to_their_controls() {
        for page in Page::ALL {
            for item in layout(page).items {
                if item.ctrl.2 > 0.0 {
                    assert!(item.ctrl.0 - item.row.0 > 230.0, "{} leaves too little room for its text", item.label);
                }
            }
        }
    }

    #[test]
    fn every_setting_is_on_exactly_one_page() {
        let mut toggles = Vec::new();
        let (mut segs, mut slides, mut picks) = (Vec::new(), Vec::new(), 0);
        for page in Page::ALL {
            for item in layout(page).items {
                match item.kind {
                    Kind::Toggle(t) => toggles.push(t),
                    Kind::Seg(g) => segs.push(g),
                    Kind::Slider(l) => slides.push(l),
                    Kind::Pick => picks += 1,
                    _ => {}
                }
            }
        }
        assert_eq!(toggles.len(), 19, "all nineteen switches: {toggles:?}");
        for (i, a) in toggles.iter().enumerate() {
            assert!(!toggles[i + 1..].contains(a), "{a:?} is on two pages");
        }
        assert_eq!(segs.len(), 7);
        assert_eq!(slides.len(), 5);
        assert_eq!(picks, 1);
    }

    #[test]
    fn the_sidebar_rows_fit_above_the_version() {
        let last = side_rect(Page::About);
        assert!(last.1 + last.3 < VERSION_Y - 12.0, "the pages must not run into the version");
        for (i, page) in Page::ALL.into_iter().enumerate() {
            assert_eq!(page.index(), i);
            let r = side_rect(page);
            assert!(r.0 + r.2 <= SIDEBAR_W);
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
        assert_eq!(seg_weights(Seg::Eyes).len(), EYE_NAMES.len());
        assert_eq!(EYE_NAMES.len(), EyeStyle::ALL.len());
        assert_eq!(seg_weights(Seg::Size).len(), SIZE_NAMES.len());
        assert_eq!(SIZE_NAMES.len(), NotchSize::ALL.len());
        for seg in [Seg::Open, Seg::WeekStart, Seg::CalView] {
            assert_eq!(seg_weights(seg).len(), seg_names(seg).len());
        }
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
