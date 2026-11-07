//! The global hotkey that opens and closes the notch from anywhere. Windows delivers a registered hotkey to a window,
//! so it has a hidden message window on its own thread (`msgwin.rs`); the thread sleeps until the key is pressed or the
//! combination is changed. Nothing is recorded: only the one registered combination is ever reported.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicIsize, AtomicU64, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{HOT_KEY_MODIFIERS, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey};
use windows::Win32::UI::WindowsAndMessaging::{DefWindowProcW, PostMessageW, WM_APP, WM_HOTKEY};
use windows::core::w;
use winit::event_loop::EventLoopProxy;
use winit::keyboard::KeyCode;

use crate::{UserEvent, msgwin};

/// Windows' modifier bits.
pub const ALT: u32 = 1;
pub const CTRL: u32 = 2;
pub const SHIFT: u32 = 4;
pub const WIN: u32 = 8;

/// The combination a fresh install has: Ctrl + Alt + N.
pub const DEFAULT_MODS: u32 = CTRL | ALT;
pub const DEFAULT_VK: u32 = 0x4E;

const ID: i32 = 1;
const APPLY: u32 = WM_APP + 1;

static PROXY: OnceLock<EventLoopProxy<UserEvent>> = OnceLock::new();
/// The window the hotkey is registered on.
static WINDOW: AtomicIsize = AtomicIsize::new(0);
/// The combination wanted: modifiers in the high half, the key in the low half; 0 for none.
static WANT: AtomicU64 = AtomicU64::new(0);

fn pack(mods: u32, vk: u32) -> u64 {
    (u64::from(mods) << 32) | u64::from(vk)
}

/// The key's name: "N", "5", "F9" or "Space".
fn key_name(vk: u32) -> Option<String> {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => char::from_u32(vk).map(String::from),
        0x70..=0x7B => Some(format!("F{}", vk - 0x6F)),
        0x20 => Some("Space".to_string()),
        _ => None,
    }
}

/// Whether a combination can be a hotkey: a key we can name, with Ctrl, Alt or Win held (a bare letter, or Shift and a
/// letter, would take over typing everywhere).
pub fn valid(mods: u32, vk: u32) -> bool {
    mods < 16 && mods & (CTRL | ALT | WIN) != 0 && key_name(vk).is_some()
}

/// "Ctrl + Alt + N".
pub fn name(mods: u32, vk: u32) -> String {
    let mut parts: Vec<String> =
        [(CTRL, "Ctrl"), (ALT, "Alt"), (SHIFT, "Shift"), (WIN, "Win")].into_iter().filter(|(bit, _)| mods & bit != 0).map(|(_, n)| n.to_string()).collect();
    parts.push(key_name(vk).unwrap_or_else(|| "?".to_string()));
    parts.join(" + ")
}

/// The Windows key code of a key that can be part of a hotkey: letters, digits, F1 to F12 and Space.
pub fn vk_of(code: KeyCode) -> Option<u32> {
    use KeyCode::*;
    let letters = [KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN, KeyO, KeyP, KeyQ, KeyR, KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ];
    let digits = [Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9];
    let function = [F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12];
    if let Some(i) = letters.iter().position(|k| *k == code) {
        return Some(0x41 + i as u32);
    }
    if let Some(i) = digits.iter().position(|k| *k == code) {
        return Some(0x30 + i as u32);
    }
    // F11 is the settings window's full-screen key.
    if let Some(i) = function.iter().position(|k| *k == code).filter(|i| *i != 10) {
        return Some(0x70 + i as u32);
    }
    (code == Space).then_some(0x20)
}

/// Start the hotkey thread with the combination wanted (`None`: not wanted). Its answers come as
/// `UserEvent::Hotkey` (the key was pressed) and `UserEvent::HotkeyStatus` (the combination could be taken or not).
pub fn start(proxy: EventLoopProxy<UserEvent>, wanted: Option<(u32, u32)>) {
    let _ = PROXY.set(proxy);
    WANT.store(wanted.map_or(0, |(m, v)| pack(m, v)), Ordering::Relaxed);
    // SAFETY: the class name is a `w!` literal, which lives for the whole run.
    unsafe {
        msgwin::spawn(w!("SkyIslandHotkey"), Some(window_proc), |window| {
            WINDOW.store(window.0 as isize, Ordering::Relaxed);
            report(apply(window));
            true
        })
    };
}

/// Use another combination, or none.
pub fn set(wanted: Option<(u32, u32)>) {
    WANT.store(wanted.map_or(0, |(m, v)| pack(m, v)), Ordering::Relaxed);
    let window = WINDOW.load(Ordering::Relaxed);
    if window != 0 {
        // SAFETY: only posts a message to the hotkey thread's own window.
        unsafe {
            let _ = PostMessageW(Some(HWND(window as *mut _)), APPLY, WPARAM(0), LPARAM(0));
        }
    }
}

/// Register the combination wanted (taking the old one away first). True when it is registered, or none is wanted.
fn apply(window: HWND) -> bool {
    // SAFETY: plain calls on this thread's own window.
    unsafe {
        let _ = UnregisterHotKey(Some(window), ID);
        let want = WANT.load(Ordering::Relaxed);
        if want == 0 {
            return true;
        }
        RegisterHotKey(Some(window), ID, HOT_KEY_MODIFIERS((want >> 32) as u32) | MOD_NOREPEAT, want as u32).is_ok()
    }
}

fn report(ok: bool) {
    if let Some(proxy) = PROXY.get() {
        let _ = proxy.send_event(UserEvent::HotkeyStatus(ok));
    }
}

unsafe extern "system" fn window_proc(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match message {
        WM_HOTKEY => {
            if let Some(proxy) = PROXY.get() {
                let _ = proxy.send_event(UserEvent::Hotkey);
            }
            LRESULT(0)
        }
        APPLY => {
            report(apply(window));
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(window, message, w, l) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_combination_is_written_out() {
        assert_eq!(name(CTRL | ALT, 0x4E), "Ctrl + Alt + N");
        assert_eq!(name(WIN | SHIFT, 0x70), "Shift + Win + F1");
        assert_eq!(name(CTRL, 0x20), "Ctrl + Space");
        assert_eq!(name(ALT, 0x35), "Alt + 5");
        assert_eq!(name(DEFAULT_MODS, DEFAULT_VK), "Ctrl + Alt + N");
    }

    #[test]
    fn a_hotkey_needs_ctrl_alt_or_win_and_a_known_key() {
        assert!(valid(CTRL | ALT, 0x4E) && valid(WIN, 0x20) && valid(ALT, 0x7B));
        assert!(!valid(0, 0x4E), "a bare letter");
        assert!(!valid(SHIFT, 0x4E), "shift and a letter");
        assert!(!valid(CTRL, 0x1B), "Escape is not a key we name");
        assert!(!valid(16, 0x4E), "not a modifier");
    }

    #[test]
    fn keys_map_to_windows_codes() {
        assert_eq!(vk_of(KeyCode::KeyA), Some(0x41));
        assert_eq!(vk_of(KeyCode::KeyZ), Some(0x5A));
        assert_eq!(vk_of(KeyCode::Digit0), Some(0x30));
        assert_eq!(vk_of(KeyCode::Digit9), Some(0x39));
        assert_eq!(vk_of(KeyCode::F1), Some(0x70));
        assert_eq!(vk_of(KeyCode::F12), Some(0x7B));
        assert_eq!(vk_of(KeyCode::F11), None, "F11 is full screen in the settings window");
        assert_eq!(vk_of(KeyCode::Space), Some(0x20));
        assert_eq!(vk_of(KeyCode::ControlLeft), None);
        assert_eq!(vk_of(KeyCode::Escape), None);
    }

    #[test]
    fn the_wanted_combination_packs_both_halves() {
        assert_eq!(pack(CTRL | ALT, 0x4E), (3u64 << 32) | 0x4E);
    }
}
