//! Watches whether the user is at the computer and whether a full-screen app is in front.
//! Runs on its own thread and only wakes the UI when one of them changes.

#[cfg(windows)]
use std::sync::OnceLock;
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(any(windows, target_os = "linux"))]
use std::thread;
#[cfg(windows)]
use std::thread::Thread;
#[cfg(any(windows, target_os = "linux", test))]
use std::time::Duration;

#[cfg(windows)]
use windows::Win32::Foundation::{HWND, RECT};
#[cfg(windows)]
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromWindow};
#[cfg(windows)]
use windows::Win32::System::SystemInformation::GetTickCount;
#[cfg(windows)]
use windows::Win32::System::Threading::GetCurrentProcessId;
#[cfg(windows)]
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
#[cfg(windows)]
use windows::Win32::UI::Shell::{QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EVENT_SYSTEM_FOREGROUND, GW_HWNDPREV, GWL_EXSTYLE, GWL_STYLE, GetForegroundWindow, GetMessageW, GetShellWindow, GetWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowThreadProcessId, HWND_TOPMOST, IsIconic, IsWindowVisible, IsZoomed, MSG, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos, TranslateMessage,
    WINEVENT_OUTOFCONTEXT, WS_CAPTION, WS_EX_TOPMOST,
};
use winit::event_loop::EventLoopProxy;

use crate::UserEvent;

/// No keyboard or mouse input for this long counts as idle, then as dozing.
#[cfg(any(windows, test))]
const IDLE_AFTER: Duration = Duration::from_secs(30);
#[cfg(any(windows, test))]
const DOZE_AFTER: Duration = Duration::from_secs(120);
#[cfg(windows)]
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

#[cfg(any(windows, test))]
pub fn classify(idle: Duration) -> Presence {
    if idle >= DOZE_AFTER {
        Presence::Dozing
    } else if idle >= IDLE_AFTER {
        Presence::Idle
    } else {
        Presence::Active
    }
}

/// Set when another window came to the front, so the look at full screen is not left to the next tick.
#[cfg(windows)]
static FRONT_CHANGED: AtomicBool = AtomicBool::new(false);
#[cfg(windows)]
static WATCHER: OnceLock<Thread> = OnceLock::new();

/// `island` is the notch window's handle, which this thread also keeps in front (see `keep_in_front`).
pub fn spawn(proxy: EventLoopProxy<UserEvent>, island: isize) {
    #[cfg(windows)]
    thread::spawn(move || {
        let _ = WATCHER.set(thread::current());
        watch_foreground();
        let mut last = (Presence::Active, Fullscreen::No);
        let mut seen = screens();
        loop {
            let now = (classify(idle_time()), fullscreen());
            if now != last {
                last = now;
                let _ = proxy.send_event(UserEvent::Presence { presence: now.0, fullscreen: now.1 });
            }
            keep_in_front(HWND(island as *mut _));
            let now = screens();
            if now != seen {
                seen = now;
                let _ = proxy.send_event(UserEvent::Screens);
            }
            // A window just came to the front: look again once it has had a moment to settle (a game may
            // still be changing its size), instead of waiting for the next tick.
            if FRONT_CHANGED.swap(false, Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(300));
                continue;
            }
            thread::park_timeout(POLL);
        }
    });
    #[cfg(target_os = "linux")]
    {
        let _ = island;
        spawn_hyprland(proxy);
    }
    #[cfg(all(not(windows), not(target_os = "linux")))]
    let _ = (proxy, island);
}

/// Watches Hyprland IPC events for fullscreen apps and monitor changes (silent no-op outside Hyprland).
#[cfg(target_os = "linux")]
fn spawn_hyprland(proxy: EventLoopProxy<UserEvent>) {
    let Some((req_sock, evt_sock)) = hyprland_sockets() else { return };
    thread::spawn(move || {
        use std::io::{BufRead, BufReader};
        use std::os::unix::net::UnixStream;

        let mut last = query_hyprland_fullscreen(&req_sock);
        if last != Fullscreen::No {
            let _ = proxy.send_event(UserEvent::Presence { presence: Presence::Active, fullscreen: last });
        }

        let Ok(stream) = UnixStream::connect(&evt_sock) else { return };
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        while reader.read_line(&mut line).is_ok_and(|n| n > 0) {
            let ev = line.trim_end();
            let check_fs = ev.starts_with("fullscreen>>")
                || ev.starts_with("workspace>>")
                || ev.starts_with("workspacev2>>")
                || ev.starts_with("focusedmon>>")
                || ev.starts_with("focusedmonv2>>")
                || ev.starts_with("activewindow>>")
                || ev.starts_with("activewindowv2>>")
                || ev.starts_with("openwindow>>")
                || ev.starts_with("closewindow>>")
                || ev.starts_with("movewindow>>")
                || ev.starts_with("movewindowv2>>")
                || ev.starts_with("monitoradded>>")
                || ev.starts_with("monitorremoved>>");
            let screens_changed = ev.starts_with("monitoradded>>") || ev.starts_with("monitorremoved>>");
            line.clear();

            if screens_changed {
                let _ = proxy.send_event(UserEvent::Screens);
            }
            if check_fs {
                let now = query_hyprland_fullscreen(&req_sock);
                if now != last {
                    last = now;
                    let _ = proxy.send_event(UserEvent::Presence { presence: Presence::Active, fullscreen: now });
                }
            }
        }
    });
}

#[cfg(target_os = "linux")]
fn hyprland_sockets() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let sig = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let dir = std::path::PathBuf::from(runtime).join("hypr").join(sig);
    let req = dir.join(".socket.sock");
    let evt = dir.join(".socket2.sock");
    (req.exists() && evt.exists()).then_some((req, evt))
}

#[cfg(target_os = "linux")]
fn query_hyprland(sock: &std::path::Path, cmd: &str) -> Option<String> {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    let mut s = UnixStream::connect(sock).ok()?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
    s.write_all(cmd.as_bytes()).ok()?;
    let mut out = String::new();
    s.read_to_string(&mut out).ok()?;
    Some(out)
}

#[cfg(target_os = "linux")]
fn query_hyprland_fullscreen(sock: &std::path::Path) -> Fullscreen {
    let (Some(monitors), Some(clients)) = (query_hyprland(sock, "j/monitors"), query_hyprland(sock, "j/clients")) else {
        return Fullscreen::No;
    };
    hyprland_fullscreen(&monitors, &clients)
}

/// Extract `(monitor_id, active_workspace_id, logical_area)` from a `hyprctl monitors -j` element.
#[cfg(any(target_os = "linux", test))]
fn hyprland_monitor_area(m: &serde_json::Value) -> Option<(i64, i64, Area)> {
    if m.get("disabled").and_then(serde_json::Value::as_bool).unwrap_or(false) {
        return None;
    }
    let id = m.get("id")?.as_i64()?;
    let ws_id = m.pointer("/activeWorkspace/id").and_then(serde_json::Value::as_i64)?;
    let x = m.get("x")?.as_i64()? as i32;
    let y = m.get("y")?.as_i64()? as i32;
    let mut w = m.get("width")?.as_f64()?;
    let mut h = m.get("height")?.as_f64()?;
    let scale = m.get("scale").and_then(serde_json::Value::as_f64).unwrap_or(1.0).max(0.1);
    let transform = m.get("transform").and_then(serde_json::Value::as_i64).unwrap_or(0);
    if transform % 2 != 0 {
        std::mem::swap(&mut w, &mut h);
    }
    let lw = (w / scale).round() as i32;
    let lh = (h / scale).round() as i32;
    Some((id, ws_id, (x, y, x + lw, y + lh)))
}

/// Compute notch fullscreen state from `hyprctl monitors -j` and `hyprctl clients -j` JSON.
/// Hyprland `fullscreen` bitmask: bit 0 (1) = maximized, bit 1 (2) = true fullscreen.
#[cfg(any(target_os = "linux", test))]
fn hyprland_fullscreen(monitors_json: &str, clients_json: &str) -> Fullscreen {
    let Ok(serde_json::Value::Array(monitors)) = serde_json::from_str::<serde_json::Value>(monitors_json) else {
        return Fullscreen::No;
    };
    let Ok(serde_json::Value::Array(clients)) = serde_json::from_str::<serde_json::Value>(clients_json) else {
        return Fullscreen::No;
    };
    let active_monitors: Vec<_> = monitors.iter().filter_map(hyprland_monitor_area).collect();
    let mut matched: Vec<Area> = Vec::new();
    for c in &clients {
        let mapped = c.get("mapped").and_then(serde_json::Value::as_bool).unwrap_or(false);
        let hidden = c.get("hidden").and_then(serde_json::Value::as_bool).unwrap_or(false);
        let fs_bits = c.get("fullscreen").and_then(serde_json::Value::as_u64).unwrap_or(0);
        if !mapped || hidden || (fs_bits & 2) == 0 {
            continue;
        }
        let ws_id = c.pointer("/workspace/id").and_then(serde_json::Value::as_i64);
        let mon_id = c.get("monitor").and_then(serde_json::Value::as_i64);
        if let Some((_, _, area)) = active_monitors.iter().find(|&&(id, active_ws, _)| Some(id) == mon_id && Some(active_ws) == ws_id)
            && !matched.contains(area)
        {
            matched.push(*area);
        }
    }
    match matched.len() {
        0 => Fullscreen::No,
        1 => Fullscreen::On(matched[0]),
        _ => Fullscreen::Somewhere,
    }
}

/// Windows says when another window comes to the front; that wakes the thread above at once, so a full-screen
/// app hides the notch (and leaving it brings the notch back) without waiting for the next look.
#[cfg(windows)]
fn watch_foreground() {
    unsafe extern "system" fn front_changed(_: HWINEVENTHOOK, _: u32, _: HWND, _: i32, _: i32, _: u32, _: u32) {
        FRONT_CHANGED.store(true, Ordering::Relaxed);
        if let Some(thread) = WATCHER.get() {
            thread.unpark();
        }
    }
    // The hook belongs to the thread that sets it up, which has to keep taking messages.
    thread::spawn(|| unsafe {
        let hook = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(front_changed), 0, 0, WINEVENT_OUTOFCONTEXT);
        if hook.is_invalid() {
            return;
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}

/// A number that changes when a screen is plugged in or out, moved or resized, or when the usable part of one
/// changes (a taskbar docked somewhere else).
#[cfg(windows)]
fn screens() -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    use windows::Win32::Foundation::{LPARAM, TRUE};
    use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR};

    unsafe extern "system" fn each(monitor: HMONITOR, _: HDC, _: *mut RECT, hasher: LPARAM) -> windows::core::BOOL {
        let hasher = unsafe { &mut *(hasher.0 as *mut DefaultHasher) };
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            let (m, w) = (info.rcMonitor, info.rcWork);
            (m.left, m.top, m.right, m.bottom, w.left, w.top, w.right, w.bottom, info.dwFlags).hash(hasher);
        }
        TRUE
    }

    let mut hasher = DefaultHasher::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut hasher as *mut DefaultHasher as isize));
    }
    hasher.finish()
}

/// Whether two areas share any pixel.
#[cfg(any(windows, test))]
fn overlaps(a: Area, b: Area) -> bool {
    a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
}

/// The notch is made always-on-top once, but Windows puts a window that becomes topmost later above the
/// older ones (a task manager, a picture-in-picture video, a screen recorder), and a window can also lose
/// the flag. So once a second: if the notch is no longer topmost, or another app's window sits over it,
/// put it back on top without taking the focus. Nothing is done while it is hidden on purpose.
#[cfg(windows)]
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
#[cfg(windows)]
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
#[cfg(windows)]
fn idle_time() -> Duration {
    let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    unsafe {
        if !GetLastInputInfo(&mut info).as_bool() {
            return Duration::ZERO;
        }
        Duration::from_millis(GetTickCount().wrapping_sub(info.dwTime) as u64)
    }
}

/// Whether a window with no title bar covers the whole of a screen (`window` and `screen` are areas). A window
/// that has been maximised is not one: that is an ordinary app (one that draws its own title bar has no
/// title-bar style, but it is still maximised, not full screen).
#[cfg(windows)]
fn covers_whole_screen(window: Area, screen: Area, style: u32, maximised: bool) -> bool {
    !maximised && style & WS_CAPTION.0 != WS_CAPTION.0 && window.0 <= screen.0 && window.1 <= screen.1 && window.2 >= screen.2 && window.3 >= screen.3
}

/// A game or player in "borderless windowed" mode is a window with no title bar the size of the screen, which
/// Windows does not always count as full screen. Its screen, if the window in front is one. The desktop, the
/// shell's own windows and this app's are not an app in full screen.
#[cfg(windows)]
fn borderless() -> Option<Area> {
    unsafe {
        let front = GetForegroundWindow();
        if front.0.is_null() || front == GetShellWindow() || !IsWindowVisible(front).as_bool() || IsIconic(front).as_bool() {
            return None;
        }
        let (mut owner, mut shell) = (0u32, 0u32);
        GetWindowThreadProcessId(front, Some(&mut owner));
        GetWindowThreadProcessId(GetShellWindow(), Some(&mut shell));
        if owner == shell || owner == GetCurrentProcessId() {
            return None;
        }
        let mut rect = RECT::default();
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let monitor = MonitorFromWindow(front, MONITOR_DEFAULTTONULL);
        if GetWindowRect(front, &mut rect).is_err() || monitor.is_invalid() || !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return None;
        }
        let (window, screen) = ((rect.left, rect.top, rect.right, rect.bottom), info.rcMonitor);
        let screen = (screen.left, screen.top, screen.right, screen.bottom);
        covers_whole_screen(window, screen, GetWindowLongPtrW(front, GWL_STYLE) as u32, IsZoomed(front).as_bool()).then_some(screen)
    }
}

/// Windows decides whether a full-screen app is in front; the monitor of the window in front says which
/// screen it is on.
#[cfg(windows)]
fn fullscreen() -> Fullscreen {
    let busy = match unsafe { SHQueryUserNotificationState() } {
        Ok(state) => state == QUNS_BUSY || state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE,
        Err(_) => false,
    };
    if !busy {
        return borderless().map_or(Fullscreen::No, Fullscreen::On);
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

    #[cfg(windows)]
    #[test]
    fn only_a_window_without_a_title_bar_that_fills_the_screen_counts_as_full_screen() {
        let (screen, bar) = ((0, 0, 1920, 1080), WS_CAPTION.0);
        assert!(covers_whole_screen((0, 0, 1920, 1080), screen, 0, false), "a borderless window the size of the screen");
        assert!(covers_whole_screen((-8, -8, 1928, 1088), screen, 0, false), "a little larger is still covering it");
        assert!(!covers_whole_screen((0, 0, 1920, 1080), screen, bar, false), "a title bar: an ordinary maximised window");
        assert!(!covers_whole_screen((0, 0, 1920, 1040), screen, 0, false), "the taskbar still shows");
        assert!(!covers_whole_screen((100, 100, 900, 700), screen, 0, false), "a small window");
        assert!(covers_whole_screen((1920, 0, 3840, 1080), (1920, 0, 3840, 1080), 0, false), "on a second screen");
        assert!(!covers_whole_screen((0, 0, 1920, 1080), (1920, 0, 3840, 1080), 0, false), "that is on another screen");
        assert!(!covers_whole_screen((-8, -8, 1928, 1088), screen, 0, true), "maximised, with a title bar of its own drawn inside: not full screen");
    }

    #[test]
    fn idle_thresholds() {
        assert_eq!(classify(Duration::from_secs(5)), Presence::Active);
        assert_eq!(classify(IDLE_AFTER), Presence::Idle);
        assert_eq!(classify(DOZE_AFTER - Duration::from_secs(1)), Presence::Idle);
        assert_eq!(classify(DOZE_AFTER), Presence::Dozing);
    }

    #[test]
    fn hyprland_fullscreen_ignores_maximized_and_inactive_workspaces() {
        let monitors = r#"[
            {"id": 0, "x": 320, "y": 1080, "width": 1920, "height": 1080, "scale": 1.5, "transform": 0, "activeWorkspace": {"id": 1}},
            {"id": 1, "x": 0, "y": 0, "width": 1080, "height": 1920, "scale": 1.0, "transform": 1, "activeWorkspace": {"id": 3}}
        ]"#;
        // Maximized window (fullscreen: 1) + true fullscreen on inactive workspace 2 -> No
        let clients_non_fs = r#"[
            {"mapped": true, "hidden": false, "fullscreen": 1, "monitor": 0, "workspace": {"id": 1}},
            {"mapped": true, "hidden": false, "fullscreen": 2, "monitor": 0, "workspace": {"id": 2}}
        ]"#;
        assert_eq!(hyprland_fullscreen(monitors, clients_non_fs), Fullscreen::No);

        // True fullscreen on fractional-scale monitor 0 -> On((320, 1080, 1600, 1800))
        let clients_fs0 = r#"[{"mapped": true, "hidden": false, "fullscreen": 2, "monitor": 0, "workspace": {"id": 1}}]"#;
        assert_eq!(hyprland_fullscreen(monitors, clients_fs0), Fullscreen::On((320, 1080, 1600, 1800)));

        // Rotated monitor 1 swaps 1080x1920 to 1920x1080
        let clients_fs1 = r#"[{"mapped": true, "hidden": false, "fullscreen": 2, "monitor": 1, "workspace": {"id": 3}}]"#;
        assert_eq!(hyprland_fullscreen(monitors, clients_fs1), Fullscreen::On((0, 0, 1920, 1080)));
    }
}
