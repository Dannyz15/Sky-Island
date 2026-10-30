//! Watches whether the user is at the computer and whether a full-screen app is in front.
//! Runs on its own thread and only wakes the UI when one of them changes.

use std::thread;
use std::time::Duration;

use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromWindow};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::Shell::{
    QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState,
};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
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

pub fn spawn(proxy: EventLoopProxy<UserEvent>) {
    thread::spawn(move || {
        let mut last = (Presence::Active, Fullscreen::No);
        loop {
            let now = (classify(idle_time()), fullscreen());
            if now != last {
                last = now;
                let _ = proxy.send_event(UserEvent::Presence { presence: now.0, fullscreen: now.1 });
            }
            thread::sleep(POLL);
        }
    });
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
    fn idle_thresholds() {
        assert_eq!(classify(Duration::from_secs(5)), Presence::Active);
        assert_eq!(classify(IDLE_AFTER), Presence::Idle);
        assert_eq!(classify(DOZE_AFTER - Duration::from_secs(1)), Presence::Idle);
        assert_eq!(classify(DOZE_AFTER), Presence::Dozing);
    }
}
