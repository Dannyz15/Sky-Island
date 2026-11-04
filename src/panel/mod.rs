//! The Sky Island settings window: an ordinary Windows window with a sidebar of pages (General, Notch,
//! Appearance, Tabs, Location, Sounds, Calendar, Shelf, Notices, About) and the settings of the chosen page on the right,
//! grouped in cards.
//! It holds every setting, including the ones that do not fit in the notch.
//!
//! It is drawn with the same tools as the notch, so it moves the same way: switches slide, the choice
//! highlight glides between options, the sidebar highlight glides between pages, hovered rows light up.
//! Everything applies at once and is saved as it changes.
//!
//! The code is in this folder: `layout` (what is on each page), `values` (the settings behind the controls),
//! `window` (opening it, keys and mouse), `city` and `keys` (the two boxes that take typing and key presses),
//! `paint`, `controls` and `changelog` (drawing), `scroll` (scroll bars and keys) and `spot` (where it opens).
//! This file holds the window itself, `Panel`.

use std::rc::Rc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use softbuffer::{Context, Surface};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, NamedKey};
use winit::window::{Fullscreen, Icon, Window, WindowButtons};

use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsZoomed};

use crate::motion::Glows;
use crate::settings::{self, ClipKeep, EyeStyle, Motion, NotchSize, Range, Screen, Settings, ThemeMode};
use crate::{App, View, appicon, gfx, store, theme};

mod changelog;
mod city;
mod controls;
mod keys;
mod layout;
mod paint;
mod scroll;
mod spot;
#[cfg(test)]
mod tests;
mod values;
mod window;

pub(crate) use layout::Page;

use changelog::*;
use city::*;
use controls::*;
use layout::*;
use scroll::*;
use spot::*;
use values::*;

const WIDTH: f64 = 840.0;
const HEIGHT: f64 = 700.0;
const SIDEBAR_W: f32 = 212.0;
const SIDE_TOP: f32 = 62.0;
const SIDE_ITEM_H: f32 = 42.0;
const CONTENT_TOP: f32 = 100.0;
/// The page's content is at least this wide (the width it has in the default window) and at most MAX_CONTENT_W.
const MIN_CONTENT_W: f32 = WIDTH as f32 - SIDEBAR_W - 60.0;
const MAX_CONTENT_W: f32 = 720.0;
/// The smallest the window can be made, in logical pixels: as wide as the default, and tall enough for the sidebar.
const MIN_HEIGHT: f64 = 520.0;
/// Where the scrolling part of a page starts, below the title and its line.
const SCROLL_TOP: f32 = 88.0;
/// Room left under the last card when a page is scrolled to the end.
const BOTTOM_PAD: f32 = 24.0;
const ROW_H: f32 = 56.0;
/// How far apart the lines of the text under a row's label are, and the size of that text.
const DESC_LINE_H: f32 = 14.0;
const DESC_PX: f32 = 11.0;
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
const KEY_SCROLL: u64 = 0x3000_0100;

type Rect = (f32, f32, f32, f32);

pub(crate) struct Panel {
    window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
    glows: Glows,
    /// The mouse, in logical pixels.
    cursor: (f32, f32),
    over_window: bool,
    /// The slider being dragged.
    drag: Option<Slide>,
    /// The city being typed, and how the last search went.
    city: CityField,
    /// The hotkey's combination is being taken from the keys pressed, and the modifier keys held right now.
    capture: bool,
    mods: winit::keyboard::ModifiersState,
    /// The changelog is shown in place of the page, scrolled this many logical pixels, and no further than `changelog_max`.
    show_changelog: bool,
    changelog_scroll: f32,
    changelog_max: f32,
    /// How far the page is scrolled (where it is going), how far it can go, and how far it is drawn scrolled right now.
    scroll: f32,
    scroll_max: f32,
    scroll_shown: f32,
    /// The scroll bar being dragged.
    bar_drag: Option<BarDrag>,
    /// Where the window last was in its ordinary state (not maximised or full screen), as it moves and is resized.
    spot: Option<Spot>,
    /// The page being shown, and since when (for its fade).
    page: Page,
    page_since: Instant,
    /// "Reset to defaults" was clicked once and waits for a second click until this time.
    pub(crate) reset_armed: Option<Instant>,
    /// How many saved songs the Clear button just removed, and until when to say so.
    pub(crate) cleared: Option<(usize, Instant)>,
    last_draw: Instant,
    /// When the window opened, for its entrance.
    opened: Instant,
    /// How much the window is shrunk to fit a small screen (1 on a roomy one), as a share of its size.
    fit: f64,
    /// Something is still moving, so more frames are needed.
    pub(crate) busy: bool,
    /// The picture each frame is drawn on, kept between frames so a big window does not allocate one every time.
    canvas: Option<Pixmap>,
    /// The window's background for one size, page colour and look (light or dark): it only changes when those do,
    /// so it is drawn once.
    backdrop: Option<(u32, u32, [u8; 3], bool, Pixmap)>,
}

impl Panel {
    /// Physical pixels per logical pixel of the window: the screen's scale, and the shrinking to fit it.
    fn scale(&self) -> f32 {
        (self.window.scale_factor() * self.fit) as f32
    }

    /// The size of the window's drawing area in logical pixels.
    fn logical_size(&self) -> (f32, f32) {
        let (size, s) = (self.window.inner_size(), self.scale());
        (size.width as f32 / s, size.height as f32 / s)
    }

    pub(crate) fn id(&self) -> winit::window::WindowId {
        self.window.id()
    }

    pub(crate) fn redraw(&self) {
        self.window.request_redraw();
    }

    /// The scroll bar of what is shown, with how far it is scrolled and how far it can go.
    fn bar(&self) -> (Bar, f32, f32) {
        let (width, height) = self.logical_size();
        if self.show_changelog {
            (changelog_bar(frame_for(width), height), self.changelog_scroll, self.changelog_max)
        } else {
            (page_bar(width, height), self.scroll_shown, self.scroll_max)
        }
    }

    /// Take hold of the scroll bar if the mouse is on it: the thumb is gripped where it is, and a click on the
    /// track above or below it brings the thumb under the mouse. Returns whether the mouse was on the bar.
    fn grab_bar(&mut self) -> bool {
        let (bar, scroll, max) = self.bar();
        let (x, y) = self.cursor;
        if max <= 0.0 || !bar.grabs(x, y) {
            return false;
        }
        let (top, len) = (bar.thumb_top(scroll, max), bar.thumb(max));
        let grab = if y >= top && y <= top + len { y - top } else { len / 2.0 };
        self.bar_drag = Some(BarDrag { changelog: self.show_changelog, grab });
        self.drag_bar_to(y);
        true
    }

    /// Move what is scrolled so the thumb follows the mouse at height `y`.
    fn drag_bar_to(&mut self, y: f32) {
        let Some(drag) = self.bar_drag else { return };
        let (bar, _, max) = self.bar();
        let to = bar.scroll_at(y - drag.grab, max);
        if drag.changelog {
            self.changelog_scroll = to;
        } else {
            self.scroll = to;
        }
        self.redraw();
    }

    /// PageUp, PageDown, Home or End: scroll the page (or the changelog) shown.
    fn scroll_key(&mut self, key: ScrollKey) {
        let height = self.logical_size().1;
        if self.show_changelog {
            let view = height - CHANGELOG_TOP - 28.0;
            self.changelog_scroll = scrolled_by_key(self.changelog_scroll, self.changelog_max, view, key);
        } else {
            self.scroll = scrolled_by_key(self.scroll, self.scroll_max, height - SCROLL_TOP, key);
        }
        self.redraw();
    }

    /// Note where the window is, so it can open there next time. Not while it is full screen, maximised or minimised:
    /// then what was noted last, the window as it was in its ordinary state, stays.
    fn note_spot(&mut self) {
        let w = &self.window;
        // Windows is asked itself: the window's own record of being maximised comes a moment after the first
        // "moved" event of the change, which would note the maximised rectangle as the ordinary one.
        let (zoomed, iconic) = hwnd_of(w).map_or((false, false), |h| unsafe { (IsZoomed(h).as_bool(), IsIconic(h).as_bool()) });
        if w.fullscreen().is_some() || zoomed || iconic || w.is_maximized() || w.is_minimized().unwrap_or(false) {
            return;
        }
        let Ok(pos) = w.outer_position() else { return };
        let (size, s) = (w.inner_size(), self.scale() as f64);
        self.spot = Some(Spot { width: size.width as f64 / s, height: size.height as f64 / s, x: pos.x, y: pos.y });
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
            self.city.typing = None;
            self.capture = false;
            self.scroll = 0.0;
            self.redraw();
        }
    }
}

impl Drop for Panel {
    /// However the window goes (closed, Escape, the app quitting), it opens in the same place next time.
    fn drop(&mut self) {
        self.note_spot();
        if let Some(spot) = self.spot {
            spot.save();
        }
    }
}
