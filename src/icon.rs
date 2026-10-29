//! File icons for shelf tiles, from the same shell source Explorer uses.

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use tiny_skia::{Pixmap, PremultipliedColorU8};
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits, GetObjectW,
    HGDIOBJ, ReleaseDC,
};
use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY};
use windows::core::PCWSTR;

/// Requested icon size in pixels; drawn scaled down on screen so it stays sharp on high-DPI displays.
const SIZE_PX: i32 = 64;

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
        if got == 0 || info.bmWidth <= 0 || info.bmHeight <= 0 {
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            return None;
        }
        let (w, h) = (info.bmWidth, info.bmHeight);

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
        let mut bgra = vec![0u8; (w * h * 4) as usize];
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
