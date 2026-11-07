//! "Every screen": a closed notch on each screen at the same time. The real notch is on one screen (the one the
//! mouse last touched the top of, as with All); on every other screen a copy of its closed face is shown in a small
//! window of its own, drawn at that screen's scale. The copies are never given the focus; when the mouse comes onto
//! one, the real notch moves to that screen (the same move as when the mouse touches the top edge in All mode), where
//! it opens as usual, and the screen it left gets a copy instead.
//!
//! The copies are drawn together with the real notch (`App::draw`), so what they show is always what it shows: the
//! clock, the eyes, the song with its bars, the timer's countdown and notices.

use std::time::{Duration, Instant};

use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::WindowId;

use crate::*;

/// How often the copies are drawn while the real notch is closed (about 30 a second), and while it is open (they do
/// not change then, and a second look now and again is enough).
const CLOSED_EVERY: Duration = Duration::from_millis(30);
const OPEN_EVERY: Duration = Duration::from_millis(500);
/// How often they are put back in front of other always-on-top windows.
const RAISE_EVERY: Duration = Duration::from_secs(1);

/// The copies of the closed notch on the other screens (With Show on = Every), and what the event loop keeps about them.
pub(crate) struct Mirrors {
    pub(crate) list: Vec<Mirror>,
    /// The copies are to be made or taken away the next time the event loop is free to (windows are made from it).
    pub(crate) wanted: bool,
    /// When they were last drawn, and last put in front of other always-on-top windows.
    pub(crate) drawn: Instant,
    pub(crate) raised: Instant,
}

impl Default for Mirrors {
    fn default() -> Self {
        Self { list: Vec::new(), wanted: false, drawn: Instant::now(), raised: Instant::now() }
    }
}

/// The copy of the closed notch on one screen.
pub(crate) struct Mirror {
    window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
    /// Where its screen is, which is how a screen is told from another.
    at: (i32, i32),
    shown: bool,
}

impl Mirror {
    pub(crate) fn id(&self) -> WindowId {
        self.window.id()
    }
}

/// A hidden, borderless, always-on-top window that is never given the focus.
fn make_mirror(event_loop: &ActiveEventLoop, at: (i32, i32)) -> Option<Mirror> {
    let attrs = Window::default_attributes()
        .with_title("Sky Island")
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_visible(false)
        .with_window_level(WindowLevel::AlwaysOnTop)
        .with_skip_taskbar(true)
        .with_inner_size(LogicalSize::new(COLLAPSED.0 + 2.0 * EAR, COLLAPSED.1));
    let window = Rc::new(event_loop.create_window(attrs).ok()?);
    // (Not made click-through: that makes a window "layered", and a layered window does not show what is drawn on it
    // this way.)
    if let Ok(handle) = window.window_handle()
        && let RawWindowHandle::Win32(h) = handle.as_raw()
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_EX_NOACTIVATE};
        let hwnd = HWND(h.hwnd.get() as *mut _);
        // SAFETY: the window was just made by this thread; only a style flag is added.
        unsafe {
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_NOACTIVATE.0 as isize);
        }
    }
    let context = Context::new(window.clone()).ok()?;
    let surface = Surface::new(&context, window.clone()).ok()?;
    Some(Mirror { window, surface, at, shown: false })
}

impl App {
    /// Whether a window is one of the copies (which have nothing to say about the keys or the clicks).
    pub(crate) fn is_mirror(&self, id: WindowId) -> bool {
        self.mirrors.list.iter().any(|m| m.id() == id)
    }

    /// The mouse came onto a copy: the real notch goes to that copy's screen.
    pub(crate) fn mirror_touched(&mut self, id: WindowId) {
        let Some(at) = self.mirrors.list.iter().find(|m| m.id() == id).map(|m| m.at) else { return };
        let Some(window) = self.window.clone() else { return };
        let device = window.available_monitors().find(|m| (m.position().x, m.position().y) == at).and_then(|m| m.name().and_then(|n| display_number(&n)));
        if let Some(device) = device {
            self.follow_to(device);
        }
    }

    /// Make or take away the copies so there is one on every screen but the one the real notch is on while the Show
    /// on setting is Every, and none otherwise. Run when the loop is free (windows are made from it).
    pub(crate) fn sync_mirrors(&mut self, event_loop: &ActiveEventLoop) {
        let Some(window) = self.window.clone() else { return };
        let at_of = |m: &winit::monitor::MonitorHandle| (m.position().x, m.position().y);
        let here = self.island_monitor(&window).map(|m| at_of(&m));
        let wanted: Vec<(i32, i32)> =
            if self.settings.screen == settings::Screen::Every { window.available_monitors().map(|m| at_of(&m)).filter(|at| Some(*at) != here).collect() } else { Vec::new() };
        self.mirrors.list.retain(|m| wanted.contains(&m.at));
        for at in wanted {
            if !self.mirrors.list.iter().any(|m| m.at == at)
                && let Some(mirror) = make_mirror(event_loop, at)
            {
                self.mirrors.list.push(mirror);
            }
        }
        self.draw_mirrors(true);
    }

    /// Draw the copies. Not more often than they need: about 30 times a second while the notch is closed and now and
    /// again while it is open, unless `force` says it must be now.
    pub(crate) fn draw_mirrors(&mut self, force: bool) {
        if self.mirrors.list.is_empty() {
            return;
        }
        let every = if self.spring.progress <= 0.001 { CLOSED_EVERY } else { OPEN_EVERY };
        if !force && self.mirrors.drawn.elapsed() < every {
            return;
        }
        self.mirrors.drawn = Instant::now();
        let Some(window) = self.window.clone() else { return };
        let monitors: Vec<_> = window.available_monitors().collect();
        let tucked = self.spring.tuck_t > 0.5;
        let raise = self.mirrors.raised.elapsed() >= RAISE_EVERY;
        if raise {
            self.mirrors.raised = Instant::now();
        }
        let mut mirrors = std::mem::take(&mut self.mirrors.list);
        // Screens drawn at the same scale (most sets of monitors) share one picture of the notch.
        let mut pictures: Vec<(u32, Option<Pixmap>)> = Vec::new();
        for mirror in &mut mirrors {
            let Some(monitor) = monitors.iter().find(|m| (m.position().x, m.position().y) == mirror.at) else {
                // Its screen is gone: the next sync takes the copy away.
                self.mirrors.wanted = true;
                continue;
            };
            let (pos, size) = (monitor.position(), monitor.size());
            let area = (pos.x, pos.y, pos.x + size.width as i32, pos.y + size.height as i32);
            // Like the real notch: out of the way for a full-screen app on this screen and while auto-hide has it away.
            if tucked || (self.settings.hide_fullscreen && self.vis.fullscreen.hides(Some(area))) {
                if mirror.shown {
                    mirror.window.set_visible(false);
                    mirror.shown = false;
                }
                continue;
            }
            let scale = monitor.scale_factor();
            let s = (scale * self.k_for(size.width as f64 / scale)) as f32;
            if !pictures.iter().any(|(key, _)| *key == s.to_bits()) {
                pictures.push((s.to_bits(), self.closed_picture(s)));
            }
            let Some(picture) = pictures.iter().find(|(key, _)| *key == s.to_bits()).and_then(|(_, picture)| picture.as_ref()) else { continue };
            let (w, h) = (picture.width(), picture.height());
            let (Some(nw), Some(nh)) = (NonZeroU32::new(w), NonZeroU32::new(h)) else { continue };
            let _ = mirror.window.request_inner_size(PhysicalSize::new(w, h));
            let x = pos.x as f64 + (size.width as f64 - w as f64) / 2.0;
            mirror.window.set_outer_position(PhysicalPosition::new(x, (pos.y + top_inset(monitor)) as f64));
            // Shown before it is drawn on: a picture given to a window that is still hidden is not kept.
            if !mirror.shown {
                mirror.window.set_visible(true);
                mirror.shown = true;
            }
            mirror.surface.resize(nw, nh).ok();
            let Ok(mut buffer) = mirror.surface.buffer_mut() else { continue };
            // tiny-skia is premultiplied RGBA; softbuffer wants 0RGB/ARGB u32.
            for (dst, src) in buffer.iter_mut().zip(picture.pixels()) {
                *dst = (src.alpha() as u32) << 24 | (src.red() as u32) << 16 | (src.green() as u32) << 8 | src.blue() as u32;
            }
            buffer.present().ok();
            if raise
                && let Ok(handle) = mirror.window.window_handle()
                && let RawWindowHandle::Win32(h) = handle.as_raw()
            {
                use windows::Win32::Foundation::HWND;
                use windows::Win32::UI::WindowsAndMessaging::{HWND_TOPMOST, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos};
                // SAFETY: puts one of this app's own windows in front without taking the focus.
                unsafe {
                    let _ = SetWindowPos(HWND(h.hwnd.get() as *mut _), Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS);
                }
            }
        }
        self.mirrors.list = mirrors;
    }
}
