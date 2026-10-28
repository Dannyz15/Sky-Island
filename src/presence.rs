//! Watches whether the user is at the computer and whether a full-screen app is in front.
//! Runs on its own thread and only wakes the UI when one of them changes.

use std::thread;
use std::time::Duration;

use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::Shell::{
    QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState,
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
        let mut last = (Presence::Active, false);
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

fn fullscreen() -> bool {
    match unsafe { SHQueryUserNotificationState() } {
        Ok(state) => state == QUNS_BUSY || state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_thresholds() {
        assert_eq!(classify(Duration::from_secs(5)), Presence::Active);
        assert_eq!(classify(IDLE_AFTER), Presence::Idle);
        assert_eq!(classify(DOZE_AFTER - Duration::from_secs(1)), Presence::Idle);
        assert_eq!(classify(DOZE_AFTER), Presence::Dozing);
    }
}
