//! The global hotkey that opens and closes the notch from anywhere. On Windows it registers a native hotkey on a
//! hidden message window; on Linux it listens on a per-config Unix control socket (`sky-island --toggle`, for
//! Wayland compositors like Hyprland/Sway) and optionally grabs keys on X11 when running outside Wayland.

#[cfg(windows)]
use std::sync::OnceLock;
#[cfg(windows)]
use std::sync::atomic::{AtomicIsize, AtomicU64, Ordering};

#[cfg(windows)]
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{HOT_KEY_MODIFIERS, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{DefWindowProcW, PostMessageW, WM_APP, WM_HOTKEY};
#[cfg(windows)]
use windows::core::w;
use winit::event_loop::EventLoopProxy;
use winit::keyboard::KeyCode;

use crate::UserEvent;
#[cfg(windows)]
use crate::msgwin;

/// Windows' modifier bits.
pub const ALT: u32 = 1;
pub const CTRL: u32 = 2;
pub const SHIFT: u32 = 4;
pub const WIN: u32 = 8;

/// The combination a fresh install has: Ctrl + Alt + N.
pub const DEFAULT_MODS: u32 = CTRL | ALT;
pub const DEFAULT_VK: u32 = 0x4E;

#[cfg(any(windows, test))]
const ID: i32 = 1;
#[cfg(windows)]
const APPLY: u32 = WM_APP + 1;

#[cfg(windows)]
static PROXY: OnceLock<EventLoopProxy<UserEvent>> = OnceLock::new();
/// The window the hotkey is registered on.
#[cfg(windows)]
static WINDOW: AtomicIsize = AtomicIsize::new(0);
/// The combination wanted: modifiers in the high half, the key in the low half; 0 for none.
#[cfg(windows)]
static WANT: AtomicU64 = AtomicU64::new(0);

#[cfg(any(windows, target_os = "linux", test))]
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
    #[cfg(windows)]
    {
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
    #[cfg(target_os = "linux")]
    linux::start(proxy, wanted);
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = wanted;
        let _ = proxy.send_event(UserEvent::HotkeyStatus(true));
    }
}

/// Use another combination, or none.
pub fn set(wanted: Option<(u32, u32)>) {
    #[cfg(windows)]
    {
        WANT.store(wanted.map_or(0, |(m, v)| pack(m, v)), Ordering::Relaxed);
        let window = WINDOW.load(Ordering::Relaxed);
        if window != 0 {
            // SAFETY: only posts a message to the hotkey thread's own window.
            unsafe {
                let _ = PostMessageW(Some(HWND(window as *mut _)), APPLY, WPARAM(0), LPARAM(0));
            }
        }
    }
    #[cfg(target_os = "linux")]
    linux::set(wanted);
    #[cfg(not(any(windows, target_os = "linux")))]
    let _ = wanted;
}

#[cfg(target_os = "linux")]
pub(crate) use linux::send_toggle;

#[cfg(target_os = "linux")]
mod linux {
    use super::{ALT, CTRL, SHIFT, WIN, pack};
    use crate::UserEvent;
    use std::ffi::{c_char, c_int, c_long, c_short, c_uint, c_ulong, c_void};
    use std::io::{Read, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
    use std::thread;
    use std::time::Duration;
    use winit::event_loop::EventLoopProxy;

    const RTLD_LAZY: c_int = 0x1;
    const POLLIN: c_short = 0x1;
    const KEY_PRESS: c_int = 2;
    const GRAB_MODE_ASYNC: c_int = 1;
    const BAD_ACCESS: u8 = 10;

    static WANT: AtomicU64 = AtomicU64::new(0);
    static WAKE_WRITE_FD: AtomicI32 = AtomicI32::new(-1);
    static STARTED: AtomicBool = AtomicBool::new(false);
    static GRAB_ERROR: AtomicBool = AtomicBool::new(false);
    static X11_PROXY: OnceLock<EventLoopProxy<UserEvent>> = OnceLock::new();

    unsafe extern "C" {
        fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn pipe(fds: *mut c_int) -> c_int;
        fn poll(fds: *mut PollFd, nfds: c_ulong, timeout: c_int) -> c_int;
        fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
        fn write(fd: c_int, buf: *const c_void, count: usize) -> isize;
        fn close(fd: c_int) -> c_int;
    }

    #[repr(C)]
    #[derive(Copy, Clone)]
    struct PollFd {
        fd: c_int,
        events: c_short,
        revents: c_short,
    }

    #[repr(C)]
    struct XErrorEvent {
        kind: c_int,
        display: *mut c_void,
        resourceid: c_ulong,
        serial: c_ulong,
        error_code: u8,
        request_code: u8,
        minor_code: u8,
    }

    #[repr(C)]
    union XEvent {
        kind: c_int,
        _pad: [c_long; 24],
    }

    type ErrorHandler = unsafe extern "C" fn(*mut c_void, *mut XErrorEvent) -> c_int;
    type FnOpenDisplay = unsafe extern "C" fn(*const c_char) -> *mut c_void;
    type FnCloseDisplay = unsafe extern "C" fn(*mut c_void) -> c_int;
    type FnDefaultRootWindow = unsafe extern "C" fn(*mut c_void) -> c_ulong;
    type FnKeysymToKeycode = unsafe extern "C" fn(*mut c_void, c_ulong) -> c_uint;
    type FnGrabKey = unsafe extern "C" fn(*mut c_void, c_int, c_uint, c_ulong, c_int, c_int, c_int) -> c_int;
    type FnUngrabKey = unsafe extern "C" fn(*mut c_void, c_int, c_uint, c_ulong) -> c_int;
    type FnSync = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
    type FnSetErrorHandler = unsafe extern "C" fn(Option<ErrorHandler>) -> Option<ErrorHandler>;
    type FnConnectionNumber = unsafe extern "C" fn(*mut c_void) -> c_int;
    type FnPending = unsafe extern "C" fn(*mut c_void) -> c_int;
    type FnNextEvent = unsafe extern "C" fn(*mut c_void, *mut XEvent) -> c_int;

    struct X11Api {
        _lib: *mut c_void,
        open_display: FnOpenDisplay,
        close_display: FnCloseDisplay,
        default_root_window: FnDefaultRootWindow,
        keysym_to_keycode: FnKeysymToKeycode,
        grab_key: FnGrabKey,
        ungrab_key: FnUngrabKey,
        sync: FnSync,
        set_error_handler: FnSetErrorHandler,
        connection_number: FnConnectionNumber,
        pending: FnPending,
        next_event: FnNextEvent,
    }

    unsafe fn sym<T>(handle: *mut c_void, name: &std::ffi::CStr) -> Option<T> {
        let ptr = unsafe { dlsym(handle, name.as_ptr()) };
        if ptr.is_null() { None } else { Some(unsafe { std::mem::transmute_copy::<*mut c_void, T>(&ptr) }) }
    }

    fn load_x11() -> Option<X11Api> {
        unsafe {
            let lib = dlopen(c"libX11.so.6".as_ptr(), RTLD_LAZY);
            if lib.is_null() {
                return None;
            }
            Some(X11Api {
                _lib: lib,
                open_display: sym(lib, c"XOpenDisplay")?,
                close_display: sym(lib, c"XCloseDisplay")?,
                default_root_window: sym(lib, c"XDefaultRootWindow")?,
                keysym_to_keycode: sym(lib, c"XKeysymToKeycode")?,
                grab_key: sym(lib, c"XGrabKey")?,
                ungrab_key: sym(lib, c"XUngrabKey")?,
                sync: sym(lib, c"XSync")?,
                set_error_handler: sym(lib, c"XSetErrorHandler")?,
                connection_number: sym(lib, c"XConnectionNumber")?,
                pending: sym(lib, c"XPending")?,
                next_event: sym(lib, c"XNextEvent")?,
            })
        }
    }

    unsafe extern "C" fn x11_error_handler(_display: *mut c_void, err: *mut XErrorEvent) -> c_int {
        if !err.is_null() && unsafe { (*err).error_code } == BAD_ACCESS {
            GRAB_ERROR.store(true, Ordering::Relaxed);
        }
        0
    }

    pub(super) fn vk_to_keysym(vk: u32) -> Option<u64> {
        match vk {
            0x30..=0x39 => Some(u64::from(vk)),
            0x41..=0x5A => Some(u64::from(vk + 0x20)),
            0x70..=0x7B => Some(0xFFBE + u64::from(vk - 0x70)),
            0x20 => Some(0x0020),
            _ => None,
        }
    }

    pub(super) fn mods_to_x11(mods: u32) -> u32 {
        let mut mask = 0u32;
        if mods & SHIFT != 0 {
            mask |= 1 << 0;
        }
        if mods & CTRL != 0 {
            mask |= 1 << 2;
        }
        if mods & ALT != 0 {
            mask |= 1 << 3;
        }
        if mods & WIN != 0 {
            mask |= 1 << 6;
        }
        mask
    }

    pub(crate) fn socket_path() -> Option<PathBuf> {
        let data = crate::store::data_dir()?;
        let hash = data.to_string_lossy().bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3));
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()).map(PathBuf::from).filter(|p| p.is_dir()).unwrap_or_else(std::env::temp_dir);
        Some(runtime.join(format!("sky-island-{hash:016x}.sock")))
    }

    pub(crate) fn send_toggle() -> bool {
        let Some(path) = socket_path() else {
            return false;
        };
        let Ok(mut stream) = UnixStream::connect(path) else {
            return false;
        };
        let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
        stream.write_all(b"toggle\n").is_ok()
    }

    pub(super) fn run_socket_listener(path: &Path, mut on_toggle: impl FnMut() -> bool + Send + 'static) -> Option<thread::JoinHandle<()>> {
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path).ok()?;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        let handle = thread::Builder::new()
            .name("hotkey-sock".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { break };
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
                    let mut buf = [0u8; 64];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    let cmd = String::from_utf8_lossy(&buf[..n]);
                    let cmd = cmd.trim();
                    if (cmd.is_empty() || cmd == "toggle") && !on_toggle() {
                        break;
                    }
                }
            })
            .ok()?;
        Some(handle)
    }

    fn apply_x11_grab(api: &X11Api, dpy: *mut c_void, root: c_ulong, current: &mut Option<(c_int, u32)>, want: u64) -> bool {
        const LOCK_MASKS: [u32; 4] = [0, 1 << 1, 1 << 4, (1 << 1) | (1 << 4)];
        if let Some((keycode, mask)) = current.take() {
            for extra in LOCK_MASKS {
                unsafe {
                    (api.ungrab_key)(dpy, keycode, mask | extra, root);
                }
            }
        }
        if want == 0 {
            unsafe {
                (api.sync)(dpy, 0);
            }
            return true;
        }
        let mods = (want >> 32) as u32;
        let vk = want as u32;
        let Some(keysym) = vk_to_keysym(vk) else {
            return false;
        };
        let keycode = unsafe { (api.keysym_to_keycode)(dpy, keysym as c_ulong) } as c_int;
        if keycode == 0 {
            return false;
        }
        let base_mask = mods_to_x11(mods);
        GRAB_ERROR.store(false, Ordering::Relaxed);
        unsafe {
            let prev = (api.set_error_handler)(Some(x11_error_handler));
            for extra in LOCK_MASKS {
                (api.grab_key)(dpy, keycode, base_mask | extra, root, 0, GRAB_MODE_ASYNC, GRAB_MODE_ASYNC);
            }
            (api.sync)(dpy, 0);
            (api.set_error_handler)(prev);
        }
        if GRAB_ERROR.load(Ordering::Relaxed) {
            for extra in LOCK_MASKS {
                unsafe {
                    (api.ungrab_key)(dpy, keycode, base_mask | extra, root);
                }
            }
            unsafe {
                (api.sync)(dpy, 0);
            }
            return false;
        }
        *current = Some((keycode, base_mask));
        true
    }

    fn start_x11_grab(proxy: EventLoopProxy<UserEvent>) {
        let mut fds = [0 as c_int; 2];
        if unsafe { pipe(fds.as_mut_ptr()) } != 0 {
            let _ = proxy.send_event(UserEvent::HotkeyStatus(true));
            return;
        }
        let (wake_rd, wake_wr) = (fds[0], fds[1]);
        WAKE_WRITE_FD.store(wake_wr, Ordering::Release);

        thread::Builder::new()
            .name("hotkey-x11".into())
            .spawn(move || {
                let Some(api) = load_x11() else {
                    let _ = proxy.send_event(UserEvent::HotkeyStatus(true));
                    unsafe {
                        close(wake_rd);
                    }
                    return;
                };
                let dpy = unsafe { (api.open_display)(std::ptr::null()) };
                if dpy.is_null() {
                    let _ = proxy.send_event(UserEvent::HotkeyStatus(true));
                    unsafe {
                        close(wake_rd);
                    }
                    return;
                }
                let root = unsafe { (api.default_root_window)(dpy) };
                let x_fd = unsafe { (api.connection_number)(dpy) };
                let mut current: Option<(c_int, u32)> = None;
                let ok = apply_x11_grab(&api, dpy, root, &mut current, WANT.load(Ordering::Relaxed));
                let _ = proxy.send_event(UserEvent::HotkeyStatus(ok));

                let mut poll_fds = [PollFd { fd: x_fd, events: POLLIN, revents: 0 }, PollFd { fd: wake_rd, events: POLLIN, revents: 0 }];
                loop {
                    poll_fds[0].revents = 0;
                    poll_fds[1].revents = 0;
                    let rc = unsafe { poll(poll_fds.as_mut_ptr(), 2, -1) };
                    if rc < 0 {
                        if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                            continue;
                        }
                        break;
                    }
                    if poll_fds[1].revents & POLLIN != 0 {
                        let mut drain = [0u8; 64];
                        let _ = unsafe { read(wake_rd, drain.as_mut_ptr().cast(), drain.len()) };
                        let ok = apply_x11_grab(&api, dpy, root, &mut current, WANT.load(Ordering::Relaxed));
                        if proxy.send_event(UserEvent::HotkeyStatus(ok)).is_err() {
                            break;
                        }
                    }
                    while unsafe { (api.pending)(dpy) } > 0 {
                        let mut ev = XEvent { _pad: [0; 24] };
                        unsafe {
                            (api.next_event)(dpy, &mut ev);
                        }
                        if unsafe { ev.kind } == KEY_PRESS && WANT.load(Ordering::Relaxed) != 0 && proxy.send_event(UserEvent::Hotkey).is_err() {
                            unsafe {
                                (api.close_display)(dpy);
                                close(wake_rd);
                            }
                            return;
                        }
                    }
                }
                unsafe {
                    (api.close_display)(dpy);
                    close(wake_rd);
                }
            })
            .ok();
    }

    pub(super) fn start(proxy: EventLoopProxy<UserEvent>, wanted: Option<(u32, u32)>) {
        WANT.store(wanted.map_or(0, |(m, v)| pack(m, v)), Ordering::Relaxed);
        let _ = X11_PROXY.set(proxy.clone());
        if STARTED.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Some(path) = socket_path() {
            let sock_proxy = proxy.clone();
            let _ = run_socket_listener(&path, move || sock_proxy.send_event(UserEvent::Hotkey).is_ok());
        }
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
        let x11 = !wayland && std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty());
        if x11 {
            start_x11_grab(proxy);
        } else {
            let _ = proxy.send_event(UserEvent::HotkeyStatus(true));
        }
    }

    pub(super) fn set(wanted: Option<(u32, u32)>) {
        WANT.store(wanted.map_or(0, |(m, v)| pack(m, v)), Ordering::Relaxed);
        let fd = WAKE_WRITE_FD.load(Ordering::Acquire);
        if fd >= 0 {
            let byte: u8 = 1;
            let _ = unsafe { write(fd, (&byte as *const u8).cast(), 1) };
        } else if let Some(proxy) = X11_PROXY.get() {
            let _ = proxy.send_event(UserEvent::HotkeyStatus(true));
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::mpsc;

        #[test]
        fn windows_keys_map_to_x11_keysyms_and_modifier_masks() {
            assert_eq!(vk_to_keysym(0x4E), Some(u64::from(b'n')));
            assert_eq!(vk_to_keysym(0x35), Some(u64::from(b'5')));
            assert_eq!(vk_to_keysym(0x70), Some(0xFFBE));
            assert_eq!(vk_to_keysym(0x7B), Some(0xFFC9));
            assert_eq!(vk_to_keysym(0x20), Some(0x0020));
            assert_eq!(vk_to_keysym(0x1B), None);
            assert_eq!(mods_to_x11(CTRL | ALT), (1 << 2) | (1 << 3));
            assert_eq!(mods_to_x11(SHIFT | WIN), (1 << 0) | (1 << 6));
        }

        #[test]
        fn control_socket_receives_toggle_command() {
            let path = std::env::temp_dir().join(format!("sky-island-hotkey-test-{}.sock", std::process::id()));
            let _ = std::fs::remove_file(&path);
            let (tx, rx) = mpsc::channel();
            let _guard = run_socket_listener(&path, move || {
                let _ = tx.send(());
                false
            })
            .expect("bind control socket");

            let mut stream = UnixStream::connect(&path).expect("connect to control socket");
            stream.write_all(b"toggle\n").expect("write toggle");
            assert!(rx.recv_timeout(Duration::from_secs(1)).is_ok(), "listener did not trigger");
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Register the combination wanted (taking the old one away first). True when it is registered, or none is wanted.
#[cfg(windows)]
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

#[cfg(windows)]
fn report(ok: bool) {
    if let Some(proxy) = PROXY.get() {
        let _ = proxy.send_event(UserEvent::HotkeyStatus(ok));
    }
}

/// Whether a `WM_HOTKEY` is the one this app asked Windows for: its id is ours and a combination is wanted. Any
/// program of the user can post the message to the window by hand, and it must not toggle the notch after the user
/// switched the hotkey off.
#[cfg(any(windows, test))]
fn is_ours(id: usize, want: u64) -> bool {
    id == ID as usize && want != 0
}

#[cfg(windows)]
unsafe extern "system" fn window_proc(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match message {
        WM_HOTKEY if is_ours(w.0, WANT.load(Ordering::Relaxed)) => {
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
    fn only_the_registered_hotkey_message_counts() {
        let on = pack(CTRL | ALT, 0x4E);
        assert!(is_ours(ID as usize, on));
        assert!(!is_ours(ID as usize + 1, on), "another id");
        assert!(!is_ours(ID as usize, 0), "the hotkey is switched off");
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
