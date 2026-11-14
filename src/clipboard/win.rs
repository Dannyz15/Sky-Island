//! The Windows side: watching the clipboard for changes, reading what is on it, and putting an entry back.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread;
use std::time::Duration;

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW,
    SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::{CF_DIB, CF_UNICODETEXT};
use windows::Win32::UI::WindowsAndMessaging::{DefWindowProcW, KillTimer, SetTimer, WM_CLIPBOARDUPDATE, WM_TIMER};
use windows::core::{PCWSTR, w};
use winit::event_loop::EventLoopProxy;

use super::pictures::{MAX_PICTURE_BYTES, decode_picture, dib_fits, dib_to_rgba, image_from_png, image_from_rgba, png_fits, rgba_to_dib};
use super::{Content, MAX_CHARS};
use crate::{UserEvent, msgwin};

/// How often the clipboard is looked at if Windows cannot tell the app when it changes.
const POLL: Duration = Duration::from_millis(400);

// ---- Windows clipboard ----

/// Where the clipboard watcher sends what it finds, and the clipboard's change counter when it last looked.
static WATCH: OnceLock<EventLoopProxy<UserEvent>> = OnceLock::new();
static LAST_SEEN: AtomicU32 = AtomicU32::new(0);

/// How long after the last change notification the clipboard is read. An app often changes it several times
/// in a row (clear, text, picture), and may still be holding it open for a moment.
const SETTLE: u32 = 120;
const SETTLE_TIMER: usize = 1;

/// Watch for things being copied. Windows tells a hidden window when the clipboard changes, so nothing runs
/// in between; the UI only hears about it when the clipboard holds text or a picture. If that window cannot
/// be made, the clipboard is looked at a few times a second instead.
pub fn spawn_watcher(proxy: EventLoopProxy<UserEvent>) {
    // Start from the current state, so whatever is already on the clipboard is not captured.
    LAST_SEEN.store(unsafe { GetClipboardSequenceNumber() }, Ordering::Relaxed);
    if WATCH.set(proxy).is_err() {
        return;
    }
    // SAFETY: the class name is a `w!` literal, which lives for the whole run.
    let listening = unsafe { msgwin::spawn(w!("SkyIslandClipboard"), Some(watch_proc), |window| AddClipboardFormatListener(window).is_ok()) };
    if !listening {
        thread::spawn(|| {
            loop {
                thread::sleep(POLL);
                look();
            }
        });
    }
}

/// Whether the clipboard history is on. While it is off the clipboard is not opened or read at all (only its change
/// counter is noted, so what was copied meanwhile is not picked up when the history is switched on again).
static ENABLED: AtomicBool = AtomicBool::new(true);

/// The Clipboard setting was changed (or read at start).
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// Read the clipboard if it changed since the last look, and pass on what it holds.
fn look() {
    let now = unsafe { GetClipboardSequenceNumber() };
    if LAST_SEEN.swap(now, Ordering::Relaxed) == now || !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    if let Some(content) = read_content()
        && let Some(proxy) = WATCH.get()
    {
        let _ = proxy.send_event(UserEvent::Clip(content));
    }
}

unsafe extern "system" fn watch_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match message {
            // Every notification restarts the timer, so a burst of changes is read once, after it.
            WM_CLIPBOARDUPDATE => {
                SetTimer(Some(window), SETTLE_TIMER, SETTLE, None);
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == SETTLE_TIMER => {
                let _ = KillTimer(Some(window), SETTLE_TIMER);
                look();
                LRESULT(0)
            }
            _ => DefWindowProcW(window, message, wparam, lparam),
        }
    }
}

/// Open the clipboard, retrying briefly because another app may be using it.
fn open_clipboard() -> bool {
    for _ in 0..8 {
        if unsafe { OpenClipboard(None) }.is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(25));
    }
    false
}

/// The `CanIncludeInClipboardHistory` format holds a DWORD that is 0 when the app that copied wants the
/// content kept out of the clipboard history (password managers set it for what they copy). `flag` is what could be
/// read of it. A format that is there but cannot be read, or is shorter than a number, says nothing: it counts as
/// a wish to stay out, because keeping a password in the history is worse than missing one copy.
fn is_barred_from_history(flag: Option<&[u8]>) -> bool {
    match flag {
        Some(flag) if flag.len() >= 4 => flag[..4] == [0, 0, 0, 0],
        _ => true,
    }
}

/// What is on the clipboard, unless the app that copied it asked not to be monitored (password
/// managers do this for the things they copy). Text wins over a picture when both are there, as
/// with cells copied from a spreadsheet.
fn read_content() -> Option<Content> {
    if !open_clipboard() {
        return None;
    }
    let bytes = unsafe {
        let marked = |name: PCWSTR| {
            let format = RegisterClipboardFormatW(name);
            format != 0 && IsClipboardFormatAvailable(format).is_ok()
        };
        let can_include = RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory"));
        let kept_out_of_history = can_include != 0 && IsClipboardFormatAvailable(can_include).is_ok() && is_barred_from_history(read_flag(can_include).as_deref());
        if marked(w!("ExcludeClipboardContentFromMonitorProcessing")) || marked(w!("Clipboard Viewer Ignore")) || kept_out_of_history {
            (None, None, None)
        } else if let Some(text) = read_unicode_text().filter(|t| !t.trim().is_empty()) {
            // Text wins, so a picture that comes with it (cells copied from a spreadsheet) is not even copied.
            (Some(text), None, None)
        } else {
            let png = read_picture(RegisterClipboardFormatW(w!("PNG")), png_fits);
            // The bitmap form is the same picture again, so it is only read when there is no PNG.
            let dib = if png.is_none() { read_picture(CF_DIB.0 as u32, dib_fits) } else { None };
            (None, png, dib)
        }
    };
    unsafe {
        let _ = CloseClipboard();
    }
    let (text, png, dib) = bytes;

    // The slow work (decoding, compressing) happens after the clipboard is closed again.
    if let Some(t) = text.filter(|t| !t.trim().is_empty()) {
        return Some(Content::Text(t));
    }
    if let Some(image) = png.and_then(image_from_png) {
        return Some(Content::Image(image));
    }
    let converted = dib.as_deref().and_then(dib_to_rgba);
    // The bitmap is as big as the converted pixels: let go of it before the picture is compressed.
    drop(dib);
    let (w, h, rgba) = converted?;
    image_from_rgba(w, h, rgba).map(Content::Image)
}

/// The bytes of a picture format, copied only if its first bytes say it is a picture that fits (`fits`) and
/// the whole is not larger than such a picture can be. Must be called with the clipboard open.
unsafe fn read_picture(format: u32, fits: fn(&[u8]) -> bool) -> Option<Vec<u8>> {
    if format == 0 {
        return None;
    }
    unsafe {
        let handle = GetClipboardData(format).ok()?;
        let hglobal = HGLOBAL(handle.0);
        let size = GlobalSize(hglobal);
        if !(64..=MAX_PICTURE_BYTES).contains(&size) {
            return None;
        }
        let ptr = GlobalLock(hglobal) as *const u8;
        if ptr.is_null() {
            return None;
        }
        let all = std::slice::from_raw_parts(ptr, size);
        let bytes = fits(&all[..64]).then(|| all.to_vec());
        let _ = GlobalUnlock(hglobal);
        bytes
    }
}

/// Must be called with the clipboard open.
unsafe fn read_unicode_text() -> Option<String> {
    unsafe {
        let handle = GetClipboardData(CF_UNICODETEXT.0 as u32).ok()?;
        let hglobal = HGLOBAL(handle.0);
        let ptr = GlobalLock(hglobal) as *const u16;
        if ptr.is_null() {
            return None;
        }
        // The text ends at a zero. Stop early if it is absurdly long, and never read past the end of the
        // block: another app may have put text there with no zero at the end.
        let room = GlobalSize(hglobal) / 2;
        let mut len = 0;
        while len < (MAX_CHARS * 2).min(room) && *ptr.add(len) != 0 {
            len += 1;
        }
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
        let _ = GlobalUnlock(hglobal);
        Some(text)
    }
}

/// The first bytes stored under a clipboard format (a small flag is all this is used for, so another app cannot make a
/// big block be copied here). Must be called with the clipboard open.
unsafe fn read_flag(format: u32) -> Option<Vec<u8>> {
    /// The most that is copied. The flag is a 4 byte number.
    const MAX_FLAG: usize = 8;
    if format == 0 {
        return None;
    }
    unsafe {
        let handle = GetClipboardData(format).ok()?;
        let hglobal = HGLOBAL(handle.0);
        let size = GlobalSize(hglobal);
        if size == 0 {
            return None;
        }
        let ptr = GlobalLock(hglobal) as *const u8;
        if ptr.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(ptr, size.min(MAX_FLAG)).to_vec();
        let _ = GlobalUnlock(hglobal);
        Some(bytes)
    }
}

/// Put `bytes` on the open clipboard under `format`. On success Windows owns the memory.
unsafe fn set_format(format: u32, bytes: &[u8]) -> bool {
    unsafe {
        let Ok(mem) = GlobalAlloc(GMEM_MOVEABLE, bytes.len()) else { return false };
        let ptr = GlobalLock(mem) as *mut u8;
        if ptr.is_null() {
            let _ = GlobalFree(Some(mem));
            return false;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        let _ = GlobalUnlock(mem);
        if SetClipboardData(format, Some(HANDLE(mem.0))).is_ok() {
            true
        } else {
            let _ = GlobalFree(Some(mem));
            false
        }
    }
}

/// Put an entry back on the clipboard. Returns whether it worked.
pub fn set_content(content: &Content) -> bool {
    // The text bytes, the bitmap and the PNG of what is put back; each entry has only what it needs.
    type Prepared<'a> = (Option<Vec<u8>>, Option<Vec<u8>>, Option<&'a [u8]>);
    // Prepare everything before opening the clipboard, so it is held for as short a time as possible.
    let (text_bytes, dib, png): Prepared = match content {
        Content::Text(t) => {
            let wide: Vec<u16> = t.encode_utf16().chain(Some(0)).collect();
            (Some(wide.iter().flat_map(|u| u.to_le_bytes()).collect()), None, None)
        }
        Content::Image(img) => {
            let Some(decoded) = decode_picture(&img.png) else { return false };
            let rgba = decoded.to_rgba8();
            (None, Some(rgba_to_dib(rgba.width(), rgba.height(), rgba.as_raw())), Some(img.png.as_slice()))
        }
    };

    if !open_clipboard() {
        return false;
    }
    let ok = unsafe {
        let _ = EmptyClipboard();
        let mut ok = false;
        if let Some(t) = &text_bytes {
            ok = set_format(CF_UNICODETEXT.0 as u32, t);
        }
        if let Some(d) = &dib {
            ok = set_format(CF_DIB.0 as u32, d);
        }
        if let Some(p) = png {
            // Many apps (browsers, chat apps) prefer the PNG form when it is there.
            let png_format = RegisterClipboardFormatW(w!("PNG"));
            if png_format != 0 {
                set_format(png_format, p);
            }
        }
        ok
    };
    unsafe {
        let _ = CloseClipboard();
    }
    ok
}

/// Empty the Windows clipboard itself.
pub fn empty() {
    if open_clipboard() {
        unsafe {
            let _ = EmptyClipboard();
            let _ = CloseClipboard();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_flag_or_one_that_cannot_be_read_keeps_a_copy_out_of_the_history() {
        assert!(is_barred_from_history(Some(&[0, 0, 0, 0])));
        assert!(!is_barred_from_history(Some(&[1, 0, 0, 0])), "1 means it may be included");
        assert!(!is_barred_from_history(Some(&[1, 0, 0, 0, 0, 0, 0, 0])), "only the first four bytes are the number");
        assert!(is_barred_from_history(Some(&[0, 0])), "too short to be a flag: stays out");
        assert!(is_barred_from_history(Some(&[])), "empty: stays out");
        assert!(is_barred_from_history(None), "there, but could not be read: stays out");
    }
}
