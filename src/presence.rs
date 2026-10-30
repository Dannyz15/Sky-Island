//! Watches whether the user is at the computer and whether a full-screen app is in front.
//! Runs on its own thread and only wakes the UI when one of them changes.

use std::thread;
use std::time::Duration;

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromWindow};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::Shell::{
    QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GW_HWNDPREV, GWL_EXSTYLE, GetForegroundWindow, GetShellWindow, GetWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowThreadProcessId, HWND_TOPMOST, IsIconic, IsWindowVisible, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SetWindowPos, WS_EX_TOPMOST,
};
use winit::event_loop::EventLoopProxy;

use crate::UserEvent;

/// No keyboard or mouse input for this long counts as idle, then as dozing.
const IDLE_AFTER: Duration = Duration::from_secs(30);
const DOZE_AFTER: Duration = Duration::from_secs(120);
const POLL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Presence {
    Active,
    Idle,
    Dozing,
}

/// A monitor's area on the screen, in pixels: left, top, right, bottom.
pub type Area = (i32, i32, i32, i32);

/// Whether a full-screen app is in front, and on which monitor.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Fullscreen {
    No,
    /// Windows says so, but not on which monitor: it counts for every monitor.
    Somewhere,
    On(Area),
}

impl Fullscreen {
    /// Whether this should hide a notch that sits on `monitor`: only a full-screen app on that same
    /// monitor does, so a video on a second screen leaves the notch alone.
    pub fn hides(self, monitor: Option<Area>) -> bool {
        match (self, monitor) {
            (Fullscreen::No, _) => false,
            (Fullscreen::Somewhere, _) | (_, None) => true,
            (Fullscreen::On(area), Some(m)) => {
                let (cx, cy) = ((m.0 + m.2) / 2, (m.1 + m.3) / 2);
                (area.0..area.2).contains(&cx) && (area.1..area.3).contains(&cy)
            }
        }
    }
}

pub fn classify(idle: Duration) -> Presence {
    if idle >= DOZE_AFTER {
        Presence::Dozing
    } else if idle >= IDLE_AFTER {
        Presence::Idle
    } else {
        Presence::Active
    }
}

/// `island` is the notch window's handle, which this thread also keeps in front (see `keep_in_front`).
pub fn spawn(proxy: EventLoopProxy<UserEvent>, island: isize) {
    thread::spawn(move || {
        let mut last = (Presence::Active, Fullscreen::No);
        loop {
            let now = (classify(idle_time()), fullscreen());
            if now != last {
                last = now;
                let _ = proxy.send_event(UserEvent::Presence { presence: now.0, fullscreen: now.1 });
            }
            keep_in_front(HWND(island as *mut _));
            thread::sleep(POLL);
        }
    });
}

/// Whether two areas share any pixel.
fn overlaps(a: Area, b: Area) -> bool {
    a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
}

/// The notch is made always-on-top once, but Windows puts a window that becomes topmost later above the
/// older ones (a task manager, a picture-in-picture video, a screen recorder), and a window can also lose
/// the flag. So once a second: if the notch is no longer topmost, or another app's window sits over it,
/// put it back on top without taking the focus. Nothing is done while it is hidden on purpose.
fn keep_in_front(island: HWND) {
    unsafe {
        if !IsWindowVisible(island).as_bool() {
            return;
        }
        let mut rect = RECT::default();
        if GetWindowRect(island, &mut rect).is_err() {
            return;
        }
        let topmost = GetWindowLongPtrW(island, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0;
        if topmost && !covered(island, (rect.left, rect.top, rect.right, rect.bottom)) {
            return;
        }
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS;
        let _ = SetWindowPos(island, Some(HWND_TOPMOST), 0, 0, 0, 0, flags);
    }
}

/// Whether a window of another app, above the notch in the stacking order, overlaps it. Windows of the
/// shell (the taskbar, Task View, Alt+Tab) are left alone: they are meant to be in front of everything.
unsafe fn covered(island: HWND, area: Area) -> bool {
    unsafe {
        let (mut shell, me) = (0u32, GetCurrentProcessId());
        GetWindowThreadProcessId(GetShellWindow(), Some(&mut shell));
        let mut above = GetWindow(island, GW_HWNDPREV);
        // The walk is bounded, in case the list changes under it.
        for _ in 0..2000 {
            let Ok(w) = above else { return false };
            if IsWindowVisible(w).as_bool() && !IsIconic(w).as_bool() {
                let mut r = RECT::default();
                let cloaked = {
                    let mut c = 0u32;
                    DwmGetWindowAttribute(w, DWMWA_CLOAKED, &mut c as *mut u32 as *mut _, 4).is_ok() && c != 0
                };
                if GetWindowRect(w, &mut r).is_ok() && overlaps(area, (r.left, r.top, r.right, r.bottom)) && !cloaked {
                    let mut pid = 0u32;
                    GetWindowThreadProcessId(w, Some(&mut pid));
                    if pid != shell && pid != me {
                        return true;
                    }
                }
            }
            above = GetWindow(w, GW_HWNDPREV);
        }
        false
    }
}

/// Time since the last keyboard or mouse input anywhere in the system.
fn idle_time() -> Duration {
    let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    unsafe {
        if !GetLastInputInfo(&mut info).as_bool() {
            return Duration::ZERO;
        }
        Duration::from_millis(GetTickCount().wrapping_sub(info.dwTime) as u64)
    }
}

/// Windows decides whether a full-screen app is in front; the monitor of the window in front says which
/// screen it is on.
fn fullscreen() -> Fullscreen {
    let busy = match unsafe { SHQueryUserNotificationState() } {
        Ok(state) => state == QUNS_BUSY || state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE,
        Err(_) => false,
    };
    if !busy {
        return Fullscreen::No;
    }
    unsafe {
        let front = GetForegroundWindow();
        if front.0.is_null() {
            return Fullscreen::Somewhere;
        }
        let monitor = MonitorFromWindow(front, MONITOR_DEFAULTTONULL);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if monitor.is_invalid() || !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return Fullscreen::Somewhere;
        }
        let r = info.rcMonitor;
        Fullscreen::On((r.left, r.top, r.right, r.bottom))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_screen_app_only_hides_the_notch_on_its_own_monitor() {
        let main = (0, 0, 1920, 1080);
        let second = (1920, 0, 3840, 1080);
        assert!(!Fullscreen::No.hides(Some(main)));
        assert!(Fullscreen::On(main).hides(Some(main)), "same monitor");
        assert!(!Fullscreen::On(second).hides(Some(main)), "a video on monitor 2 leaves monitor 1 alone");
        assert!(Fullscreen::On(main).hides(Some(main)) && !Fullscreen::On(main).hides(Some(second)));
        assert!(Fullscreen::Somewhere.hides(Some(main)), "unknown monitor: hide, as before");
        assert!(Fullscreen::On(second).hides(None), "own monitor unknown: hide, as before");
        // A monitor to the left of the main one has negative coordinates.
        let left = (-1920, 0, 0, 1080);
        assert!(Fullscreen::On(left).hides(Some(left)) && !Fullscreen::On(left).hides(Some(main)));
    }

    #[test]
    fn only_windows_that_share_pixels_with_the_notch_cover_it() {
        let notch = (853, 0, 1068, 35);
        assert!(overlaps(notch, (800, 0, 1100, 120)), "a window across the top");
        assert!(overlaps(notch, (1000, 20, 1500, 400)), "a corner of it");
        assert!(!overlaps(notch, (0, 0, 853, 1080)), "touching edges share no pixel");
        assert!(!overlaps(notch, (853, 35, 1068, 400)), "just below it");
        assert!(!overlaps(notch, (1068, 0, 1920, 1080)), "to the right of it");
    }

    #[test]
    fn idle_thresholds() {
        assert_eq!(classify(Duration::from_secs(5)), Presence::Active);
        assert_eq!(classify(IDLE_AFTER), Presence::Idle);
        assert_eq!(classify(DOZE_AFTER - Duration::from_secs(1)), Presence::Idle);
        assert_eq!(classify(DOZE_AFTER), Presence::Dozing);
    }
}
