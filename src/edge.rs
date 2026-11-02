//! "Show on All screens": the notch follows the mouse. While that setting is on and there is more than one screen, a
//! mouse hook on its own thread watches for the mouse touching the top edge of another screen where the notch would
//! be; when it does, the app is told (`UserEvent::FollowTo`) and moves the notch there. The hook only compares the
//! mouse position with a few rectangles and says nothing while the mouse is anywhere else, and it is taken away
//! again when the setting is switched off or there is no other screen.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, MSG, MSLLHOOKSTRUCT, PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx, WH_MOUSE_LL, WM_MOUSEMOVE, WM_QUIT,
};
use winit::event_loop::EventLoopProxy;

use crate::UserEvent;

/// The part of a screen's top edge where the notch would be: a strip as high as a taskbar docked at the top, if
/// there is one, and as wide as the notch with some room to spare. All in screen pixels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Zone {
    /// The screen's number (the one in Windows' display settings).
    pub screen: u32,
    pub left: i32,
    pub right: i32,
    pub top: i32,
    pub bottom: i32,
}

impl Zone {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y <= self.bottom
    }
}

/// A screen, for working out the zones: its number, where it is and how big, in pixels, how much of its top a bar
/// takes, and its scale.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Screen {
    pub number: u32,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub bar: i32,
    pub scale: f64,
}

/// The zones of every screen but the one the notch is on. `notch_w` is the closed notch's width in logical pixels at
/// the user's chosen size (ears included).
pub fn zones_for(screens: &[Screen], current: u32, notch_w: f64) -> Vec<Zone> {
    screens
        .iter()
        .filter(|s| s.number != current)
        .map(|s| {
            let half = (notch_w * s.scale / 2.0 + 24.0 * s.scale) as i32;
            let middle = s.x + s.width / 2;
            Zone { screen: s.number, left: middle - half, right: middle + half, top: s.y, bottom: s.y + s.bar + 1 }
        })
        .collect()
}

static ZONES: Mutex<Vec<Zone>> = Mutex::new(Vec::new());
static PROXY: OnceLock<EventLoopProxy<UserEvent>> = OnceLock::new();
/// The screen the mouse was last found on, so that resting there says it once; -1 while it is on none.
static LAST: AtomicI64 = AtomicI64::new(-1);
/// The hook thread's id, or 0 until it is running.
static THREAD: AtomicU32 = AtomicU32::new(0);
/// Whether a hook thread has been started and has not ended (so that only one is ever started).
static RUNNING: AtomicBool = AtomicBool::new(false);
/// Whether the hook is wanted. A thread that finds it is not wanted when it is ready ends at once.
static WANTED: AtomicBool = AtomicBool::new(false);

/// Use these zones from now on (and none, to stop the hook saying anything).
pub fn set_zones(zones: Vec<Zone>) {
    if let Ok(mut z) = ZONES.lock() {
        *z = zones;
    }
    LAST.store(-1, Ordering::Relaxed);
}

/// Start watching the mouse, if it is not being watched.
pub fn enable(proxy: EventLoopProxy<UserEvent>) {
    let _ = PROXY.set(proxy);
    WANTED.store(true, Ordering::Relaxed);
    if RUNNING.swap(true, Ordering::Relaxed) {
        return;
    }
    thread::spawn(|| unsafe {
        loop {
            THREAD.store(GetCurrentThreadId(), Ordering::Relaxed);
            let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), None, 0);
            // A hook needs its thread to keep taking messages; it ends when it is told to quit (or, if it was told
            // before it was ready, at once).
            let mut msg = MSG::default();
            while WANTED.load(Ordering::Relaxed) {
                if GetMessageW(&mut msg, None, 0, 0).0 <= 0 {
                    break;
                }
            }
            if let Ok(hook) = hook {
                let _ = UnhookWindowsHookEx(hook);
            }
            THREAD.store(0, Ordering::Relaxed);
            // Told to stop, and not asked again in the meantime: done. (Asked again: watch again.)
            if !WANTED.load(Ordering::Relaxed) {
                RUNNING.store(false, Ordering::Relaxed);
                break;
            }
        }
    });
}

/// Stop watching the mouse.
pub fn disable() {
    WANTED.store(false, Ordering::Relaxed);
    let id = THREAD.load(Ordering::Relaxed);
    if id != 0 {
        // SAFETY: only posts a quit message to the hook's own thread.
        unsafe {
            let _ = PostThreadMessageW(id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
    set_zones(Vec::new());
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && wparam.0 as u32 == WM_MOUSEMOVE {
        // SAFETY: for a WH_MOUSE_LL hook Windows passes a pointer to a MSLLHOOKSTRUCT.
        let pt = unsafe { (*(lparam.0 as *const MSLLHOOKSTRUCT)).pt };
        // Never wait inside a hook (Windows drops slow ones): if the zones are being changed, skip this move.
        if let Ok(zones) = ZONES.try_lock() {
            match zones.iter().find(|z| z.contains(pt.x, pt.y)) {
                Some(z) => {
                    if LAST.swap(i64::from(z.screen), Ordering::Relaxed) != i64::from(z.screen)
                        && let Some(proxy) = PROXY.get()
                    {
                        let _ = proxy.send_event(UserEvent::FollowTo(z.screen));
                    }
                }
                None => LAST.store(-1, Ordering::Relaxed),
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screens() -> Vec<Screen> {
        vec![
            Screen { number: 1, x: 0, y: 0, width: 1920, bar: 0, scale: 1.25 },
            Screen { number: 2, x: 1920, y: 0, width: 2560, bar: 0, scale: 1.0 },
            Screen { number: 3, x: -1280, y: 100, width: 1280, bar: 40, scale: 1.0 },
        ]
    }

    #[test]
    fn every_screen_but_the_one_the_notch_is_on_has_a_zone() {
        let zones = zones_for(&screens(), 1, 172.0);
        assert_eq!(zones.iter().map(|z| z.screen).collect::<Vec<_>>(), [2, 3]);
        assert!(zones_for(&screens()[..1], 1, 172.0).is_empty(), "one screen: nothing to follow");
    }

    #[test]
    fn the_zone_is_the_top_edge_where_the_notch_would_be() {
        let z = zones_for(&screens(), 1, 172.0)[0];
        // Screen 2 is 2560 wide from x = 1920, so its middle is 3200; the notch is 172 wide, and 24 more on each side.
        assert_eq!((z.left, z.right), (3200 - 110, 3200 + 110));
        assert!(z.contains(3200, 0) && z.contains(3200, 1) && z.contains(3095, 0));
        assert!(!z.contains(3200, 2), "below the edge");
        assert!(!z.contains(3000, 0) && !z.contains(3310, 0), "along the edge, away from the notch");
        assert!(!z.contains(100, 0), "another screen");
    }

    #[test]
    fn a_bar_along_the_top_of_a_screen_is_part_of_the_zone() {
        let z = zones_for(&screens(), 1, 172.0)[1];
        assert_eq!((z.top, z.bottom), (100, 141));
        assert!(z.contains(-640, 120) && !z.contains(-640, 142));
    }
}
