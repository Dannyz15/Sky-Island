//! Where the settings window opens: the size and place it had when it was closed, kept in `settings-window.json`, and the fit to a small screen.

use super::*;

/// Where the settings window was when it was closed: its size in the units of the default window (so it
/// means the same on a screen with another scale) and where its top left corner was on the desktop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Spot {
    pub(super) width: f64,
    pub(super) height: f64,
    pub(super) x: i32,
    pub(super) y: i32,
}

impl Spot {
    pub(super) fn load() -> Option<Spot> {
        Self::from_json(&store::read_json(&store::path("settings-window.json")?)?)
    }

    /// The spot a file says, or `None` when it is missing a number or has a silly one. A size a little under the
    /// smallest window (the window at its smallest, rounded) is fine: `size_on_screen` brings it up to it.
    pub(super) fn from_json(v: &Value) -> Option<Spot> {
        let float = |k: &str| v.get(k).and_then(Value::as_f64).filter(|n| n.is_finite());
        let int = |k: &str| v.get(k).and_then(Value::as_i64).and_then(|n| i32::try_from(n).ok());
        let (width, height) = (float("width")?, float("height")?);
        if !(100.0..=10_000.0).contains(&width) || !(100.0..=10_000.0).contains(&height) {
            return None;
        }
        Some(Spot { width, height, x: int("x")?, y: int("y")? })
    }

    pub(super) fn to_json(self) -> Value {
        json!({ "width": self.width, "height": self.height, "x": self.x, "y": self.y })
    }

    pub(super) fn save(self) {
        if cfg!(test) {
            return;
        }
        if let Some(path) = store::path("settings-window.json") {
            store::write(&path, &self.to_json().to_string());
        }
    }
}

/// The size to open at, in the units of the default window: the one remembered, but no smaller than the window
/// can be and no bigger than a screen of `screen_w` x `screen_h` physical pixels at scale `sf` has room for.
pub(super) fn size_on_screen(width: f64, height: f64, screen_w: f64, screen_h: f64, sf: f64, fit: f64) -> (f64, f64) {
    let max_w = (screen_w * 0.96 / sf / fit).max(WIDTH);
    let max_h = ((screen_h / sf - 130.0) / fit).max(MIN_HEIGHT);
    (width.clamp(WIDTH, max_w), height.clamp(MIN_HEIGHT, max_h))
}

/// Whether a window whose top left corner is at (`x`, `y`) has its title bar on one of the screens
/// (each `(left, top, width, height)`), so it can still be taken hold of.
pub(super) fn on_a_screen(x: i32, y: i32, screens: &[(i32, i32, u32, u32)]) -> bool {
    let (px, py) = (x + 120, y + 16);
    screens.iter().any(|&(l, t, w, h)| px >= l && px < l + w as i32 && py >= t && py < t + h as i32)
}

/// How much to shrink the window to fit a screen of `width` x `height` physical pixels at scale `sf`: 1 when it
/// fits with room for the title bar and the taskbar, otherwise less, but never below a little over half.
pub(super) fn fit_to_screen(width: f64, height: f64, sf: f64) -> f64 {
    let room_h = height - 130.0 * sf;
    let room_w = width * 0.96;
    (room_h / (HEIGHT * sf)).min(room_w / (WIDTH * sf)).clamp(0.55, 1.0)
}

/// The Windows handle of a window.
pub(super) fn hwnd_of(window: &Window) -> Option<windows::Win32::Foundation::HWND> {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(windows::Win32::Foundation::HWND(h.hwnd.get() as *mut _)),
        _ => None,
    }
}
