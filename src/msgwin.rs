//! A hidden window on a thread of its own, for the notifications that Windows only delivers to a window
//! (the clipboard changed, the power state changed). The thread sleeps inside `GetMessageW`, so nothing
//! runs between two notifications.

use std::sync::mpsc;
use std::thread;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DispatchMessageW, GetMessageW, HWND_MESSAGE, MSG, RegisterClassW, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSW, WNDPROC,
};
use windows::core::PCWSTR;

/// Start the thread with a window of the class `class`, whose messages go to `proc`. `setup` runs on the
/// thread once the window exists and says whether the notifications could be switched on. Returns whether
/// all of that worked, so the caller can fall back to looking at regular intervals instead.
pub fn spawn(class: PCWSTR, proc: WNDPROC, setup: fn(HWND) -> bool) -> bool {
    let (tx, rx) = mpsc::channel();
    // PCWSTR is a bare pointer; the class names passed in are string literals that live for the whole run.
    let class = class.0 as usize;
    thread::spawn(move || unsafe {
        let class = PCWSTR(class as *const u16);
        let instance = GetModuleHandleW(None).ok().map(|module| module.into());
        let definition = WNDCLASSW { lpfnWndProc: proc, hInstance: instance.unwrap_or_default(), lpszClassName: class, ..Default::default() };
        RegisterClassW(&definition);
        let window = CreateWindowExW(WINDOW_EX_STYLE(0), class, class, WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, instance, None);
        let ready = window.is_ok_and(setup);
        let _ = tx.send(ready);
        if !ready {
            return;
        }
        let mut msg = MSG::default();
        // GetMessageW is 0 for WM_QUIT and -1 for an error: both end the loop.
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
    rx.recv().unwrap_or(false)
}
