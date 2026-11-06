//! File icons for shelf tiles, from the same shell source Explorer uses.

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread;

use tiny_skia::{Pixmap, PremultipliedColorU8};
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits, GetObjectW,
    HGDIOBJ, ReleaseDC,
};
use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::core::PCWSTR;
use winit::event_loop::EventLoopProxy;

use crate::UserEvent;

/// Requested icon size in pixels; drawn scaled down on screen so it stays sharp on high-DPI displays.
const SIZE_PX: i32 = 64;
/// The largest icon, across or down, that is read. The shell may return a bigger one than asked for
/// (`SIIGBF_BIGGERSIZEOK`), and the picture size comes from it, so it is checked before any memory is taken for it.
const MAX_SIDE: i32 = 1024;

/// The bytes of a 32-bit picture of `w` x `h` pixels: `None` for an empty one, one bigger than any icon, or a size
/// that does not fit (the sum used to be `w * h * 4` in `i32`, which wraps for a huge size reported by the shell).
fn buffer_len(w: i32, h: i32) -> Option<usize> {
    if w <= 0 || h <= 0 || w > MAX_SIDE || h > MAX_SIDE {
        return None;
    }
    (w as usize).checked_mul(h as usize)?.checked_mul(4)
}

/// Start the thread that loads icons one at a time, so a file on a slow or unreachable network drive cannot
/// freeze the notch. Each path sent to it comes back as `UserEvent::Icon`.
pub fn spawn_loader(proxy: EventLoopProxy<UserEvent>) -> Sender<PathBuf> {
    let (tx, rx) = mpsc::channel::<PathBuf>();
    thread::spawn(move || {
        // The shell wants COM on the thread that asks it for an icon.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        while let Ok(path) = rx.recv() {
            let icon = load(&path);
            let _ = proxy.send_event(UserEvent::Icon { path, icon });
        }
    });
    tx
}

/// The shell icon for `path` as a premultiplied RGBA pixmap, or `None` if the shell has none.
pub fn load(path: &Path) -> Option<Pixmap> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let factory: IShellItemImageFactory = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None).ok()?;
        let bitmap = factory
            .GetImage(SIZE { cx: SIZE_PX, cy: SIZE_PX }, SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK)
            .ok()?;

        let mut info = BITMAP::default();
        let got = GetObjectW(
            HGDIOBJ(bitmap.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut info as *mut BITMAP as *mut c_void),
        );
        let (w, h) = (info.bmWidth, info.bmHeight);
        let Some(len) = buffer_len(w, h).filter(|_| got != 0) else {
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            return None;
        };

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // negative height: rows run top to bottom
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bgra = vec![0u8; len];
        let hdc = GetDC(None);
        let rows = GetDIBits(hdc, bitmap, 0, h as u32, Some(bgra.as_mut_ptr() as *mut c_void), &mut bmi, DIB_RGB_COLORS);
        ReleaseDC(None, hdc);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        if rows == 0 {
            return None;
        }

        // Some icons come back with no alpha channel at all; treat those as opaque.
        let opaque = bgra.as_chunks::<4>().0.iter().all(|p| p[3] == 0);
        let mut pixmap = Pixmap::new(w as u32, h as u32)?;
        for (dst, p) in pixmap.pixels_mut().iter_mut().zip(bgra.as_chunks::<4>().0) {
            let a = if opaque { 255 } else { p[3] };
            // The shell returns premultiplied colour; clamp so the value is always valid.
            *dst = PremultipliedColorU8::from_rgba(p[2].min(a), p[1].min(a), p[0].min(a), a)?;
        }
        Some(pixmap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_buffer_for_a_normal_icon_is_four_bytes_a_pixel() {
        assert_eq!(buffer_len(64, 64), Some(64 * 64 * 4));
        assert_eq!(buffer_len(256, 256), Some(256 * 256 * 4));
        assert_eq!(buffer_len(MAX_SIDE, MAX_SIDE), Some(MAX_SIDE as usize * MAX_SIDE as usize * 4));
    }

    #[test]
    fn an_empty_or_enormous_size_from_the_shell_gets_no_buffer() {
        for (w, h) in [(0, 5), (5, 0), (-1, 5), (5, -1), (MAX_SIDE + 1, 1), (1, MAX_SIDE + 1), (40_000, 40_000), (i32::MAX, i32::MAX), (i32::MAX, 1)] {
            assert_eq!(buffer_len(w, h), None, "{w} x {h}");
        }
        // The old sum in i32: 32768 x 32768 x 4 wraps to 0 and (46341 x 46341 x 4) to a negative number.
        assert_eq!((32768i32).wrapping_mul(32768).wrapping_mul(4), 0);
        assert_eq!(buffer_len(32768, 32768), None);
    }
}
