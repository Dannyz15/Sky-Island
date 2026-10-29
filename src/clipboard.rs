//! Clipboard history for text and images (screenshots). Kept in memory only and never written to
//! disk, so it is gone when the app closes. Also holds the Windows clipboard calls the app needs.

use std::thread;
use std::time::Duration;

use image::{ImageEncoder, RgbaImage};
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber, IsClipboardFormatAvailable,
    OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::{CF_DIB, CF_UNICODETEXT};
use windows::core::w;
use winit::event_loop::EventLoopProxy;

use crate::UserEvent;

/// Entries kept, newest first.
pub const MAX_ITEMS: usize = 30;
/// Longer text is cut to this many characters.
const MAX_CHARS: usize = 5000;
/// Rows visible at once in the clipboard view.
pub const VISIBLE: usize = 3;
/// Images are stored compressed (PNG). Once they add up to more than this, the oldest are dropped.
const MAX_IMAGE_BYTES: usize = 48 * 1024 * 1024;
/// Pictures with more pixels than this are not kept.
const MAX_PIXELS: usize = 64 * 1024 * 1024;
/// The preview of an image fits in this box, at twice the size it is drawn so it stays sharp.
pub const THUMB_MAX: (u32, u32) = (96, 52);
const POLL: Duration = Duration::from_millis(400);

#[derive(Clone)]
pub struct Image {
    /// The whole picture, PNG-compressed.
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Small preview, straight RGBA.
    pub thumb: Vec<u8>,
    pub thumb_w: u32,
    pub thumb_h: u32,
}

#[derive(Clone)]
pub enum Content {
    Text(String),
    Image(Image),
}

impl Content {
    fn same_as(&self, other: &Content) -> bool {
        match (self, other) {
            (Content::Text(a), Content::Text(b)) => a == b,
            (Content::Image(a), Content::Image(b)) => a.png == b.png,
            _ => false,
        }
    }

    fn image_bytes(&self) -> usize {
        match self {
            Content::Image(i) => i.png.len(),
            Content::Text(_) => 0,
        }
    }
}

pub struct History {
    pub items: Vec<Content>,
    /// First visible row.
    pub scroll: usize,
}

impl History {
    pub fn new() -> Self {
        Self { items: Vec::new(), scroll: 0 }
    }

    /// Add something copied, at the top. Blank text is ignored, and copying the same thing again moves
    /// it to the top instead of adding a second entry.
    pub fn add(&mut self, content: Content) -> bool {
        let content = match content {
            Content::Text(t) if t.trim().is_empty() => return false,
            Content::Text(t) => Content::Text(t.chars().take(MAX_CHARS).collect()),
            image => image,
        };
        self.items.retain(|c| !c.same_as(&content));
        self.items.insert(0, content);
        self.items.truncate(MAX_ITEMS);
        self.evict_old_images();
        self.scroll = 0;
        true
    }

    /// Drop the oldest images while the stored pictures take too much memory. The newest entry stays.
    fn evict_old_images(&mut self) {
        let mut total: usize = self.items.iter().map(Content::image_bytes).sum();
        let mut i = self.items.len();
        while total > MAX_IMAGE_BYTES && i > 1 {
            i -= 1;
            let size = self.items[i].image_bytes();
            if size > 0 {
                total -= size;
                self.items.remove(i);
            }
        }
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.items.len() {
            self.items.remove(index);
            self.clamp_scroll();
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.scroll = 0;
    }

    pub fn scroll_by(&mut self, rows: isize) {
        self.scroll = (self.scroll as isize + rows).max(0) as usize;
        self.clamp_scroll();
    }

    fn clamp_scroll(&mut self) {
        self.scroll = self.scroll.min(self.items.len().saturating_sub(VISIBLE));
    }

    /// Entry index shown in a visible row, if there is one.
    pub fn index_at(&self, row: usize) -> Option<usize> {
        let i = self.scroll + row;
        (row < VISIBLE && i < self.items.len()).then_some(i)
    }
}

/// One line for the list: line breaks and runs of spaces become a single space.
pub fn preview(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---- pictures ----

/// A kept picture made from straight RGBA pixels.
fn image_from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Option<Image> {
    if width == 0 || height == 0 || (width as usize).checked_mul(height as usize)? > MAX_PIXELS {
        return None;
    }
    let img = RgbaImage::from_raw(width, height, rgba)?;

    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(
        &mut png,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Sub,
    )
    .write_image(img.as_raw(), width, height, image::ExtendedColorType::Rgba8)
    .ok()?;

    Some(Image { png, width, height, ..thumbnail_of(&img) })
}

/// The small preview of a picture, as the thumbnail fields of an `Image`.
fn thumbnail_of(img: &RgbaImage) -> Image {
    let (tw, th) = fit(img.width(), img.height(), THUMB_MAX.0, THUMB_MAX.1);
    let thumb = image::imageops::resize(img, tw, th, image::imageops::FilterType::Triangle);
    Image { png: Vec::new(), width: 0, height: 0, thumb_w: tw, thumb_h: th, thumb: thumb.into_raw() }
}

/// A kept picture made from PNG data copied by another app.
fn image_from_png(png: Vec<u8>) -> Option<Image> {
    let decoded = image::load_from_memory(&png).ok()?.to_rgba8();
    let (width, height) = decoded.dimensions();
    if width == 0 || height == 0 || (width as usize).checked_mul(height as usize)? > MAX_PIXELS {
        return None;
    }
    Some(Image { png, width, height, ..thumbnail_of(&decoded) })
}

/// The largest size with the same shape as `w` x `h` that fits in `max_w` x `max_h`, never zero.
fn fit(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    let scale = (max_w as f64 / w as f64).min(max_h as f64 / h as f64).min(1.0);
    (((w as f64 * scale).round() as u32).max(1), ((h as f64 * scale).round() as u32).max(1))
}

/// Pixels of a Windows device-independent bitmap (what `CF_DIB` holds), as straight RGBA with full
/// opacity. Handles the common 24 and 32 bit uncompressed forms.
fn dib_to_rgba(dib: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let u32_at = |i: usize| dib.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let header = u32_at(0)? as usize;
    let width = u32_at(4)? as i32;
    let height = u32_at(8)? as i32;
    let bits = dib.get(14..16).map(|b| u16::from_le_bytes([b[0], b[1]]))? as usize;
    let compression = u32_at(16)?;
    let colors_used = u32_at(32)? as usize;

    if header < 40 || header > dib.len() || width <= 0 || height == 0 || !(bits == 24 || bits == 32) {
        return None;
    }
    if compression != 0 && compression != 3 {
        return None; // compressed bitmaps are not handled
    }
    let (w, h) = (width as usize, height.unsigned_abs() as usize);
    if w.checked_mul(h)? > MAX_PIXELS {
        return None;
    }
    // Pixel data follows the header, the three colour masks (only with BI_BITFIELDS and a plain
    // 40 byte header) and any colour table.
    let masks = if compression == 3 && header == 40 { 12 } else { 0 };
    let offset = header + masks + colors_used * 4;
    let stride = (w * bits).div_ceil(32) * 4;
    if offset.checked_add(stride.checked_mul(h)?)? > dib.len() {
        return None;
    }

    let bytes = bits / 8;
    let top_down = height < 0;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let src = if top_down { y } else { h - 1 - y };
        let row = &dib[offset + src * stride..offset + src * stride + w * bytes];
        for px in row.chunks_exact(bytes) {
            rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
        }
    }
    Some((w as u32, h as u32, rgba))
}

/// The reverse: a 32 bit bottom-up `CF_DIB` for straight RGBA pixels.
fn rgba_to_dib(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut dib = Vec::with_capacity(40 + w * h * 4);
    dib.extend_from_slice(&40u32.to_le_bytes()); // header size
    dib.extend_from_slice(&(width as i32).to_le_bytes());
    dib.extend_from_slice(&(height as i32).to_le_bytes()); // positive: rows run bottom to top
    dib.extend_from_slice(&1u16.to_le_bytes()); // planes
    dib.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    dib.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    dib.extend_from_slice(&((w * h * 4) as u32).to_le_bytes());
    dib.extend_from_slice(&[0u8; 16]); // resolution and colour counts
    for y in (0..h).rev() {
        for px in rgba[y * w * 4..(y + 1) * w * 4].as_chunks::<4>().0 {
            dib.extend_from_slice(&[px[2], px[1], px[0], 255]);
        }
    }
    dib
}

// ---- Windows clipboard ----

/// Watch for things being copied. Runs on its own thread and only wakes the UI when the clipboard
/// changes and holds text or a picture.
pub fn spawn_watcher(proxy: EventLoopProxy<UserEvent>) {
    thread::spawn(move || {
        // Start from the current state, so whatever is already on the clipboard is not captured.
        let mut last = unsafe { GetClipboardSequenceNumber() };
        loop {
            thread::sleep(POLL);
            let now = unsafe { GetClipboardSequenceNumber() };
            if now == last {
                continue;
            }
            last = now;
            if let Some(content) = read_content() {
                let _ = proxy.send_event(UserEvent::Clip(content));
            }
        }
    });
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

/// What is on the clipboard, unless the app that copied it asked not to be monitored (password
/// managers do this for the things they copy). Text wins over a picture when both are there, as
/// with cells copied from a spreadsheet.
fn read_content() -> Option<Content> {
    if !open_clipboard() {
        return None;
    }
    let bytes = unsafe {
        let excluded = RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"));
        if excluded != 0 && IsClipboardFormatAvailable(excluded).is_ok() {
            (None, None, None)
        } else {
            let png_format = RegisterClipboardFormatW(w!("PNG"));
            (read_unicode_text(), read_format(png_format), read_format(CF_DIB.0 as u32))
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
    let (w, h, rgba) = dib.as_deref().and_then(dib_to_rgba)?;
    image_from_rgba(w, h, rgba).map(Content::Image)
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
        // The text ends at a zero. Stop early if it is absurdly long.
        let mut len = 0;
        while len < MAX_CHARS * 2 && *ptr.add(len) != 0 {
            len += 1;
        }
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
        let _ = GlobalUnlock(hglobal);
        Some(text)
    }
}

/// The raw bytes stored under a clipboard format. Must be called with the clipboard open.
unsafe fn read_format(format: u32) -> Option<Vec<u8>> {
    if format == 0 {
        return None;
    }
    unsafe {
        let handle = GetClipboardData(format).ok()?;
        let hglobal = HGLOBAL(handle.0);
        let size = GlobalSize(hglobal);
        // Pictures can be large, but a gigabyte is not a screenshot.
        if size == 0 || size > 1 << 30 {
            return None;
        }
        let ptr = GlobalLock(hglobal) as *const u8;
        if ptr.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(ptr, size).to_vec();
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
            let Ok(decoded) = image::load_from_memory(&img.png) else { return false };
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

    fn text(s: &str) -> Content {
        Content::Text(s.to_string())
    }

    /// An entry that only matters for its size and identity.
    fn fake_image(bytes: usize, tag: u8) -> Content {
        Content::Image(Image { png: vec![tag; bytes], width: 2, height: 2, thumb: vec![0; 16], thumb_w: 2, thumb_h: 2 })
    }

    fn texts(h: &History) -> Vec<String> {
        h.items
            .iter()
            .map(|c| match c {
                Content::Text(t) => t.clone(),
                Content::Image(_) => "<image>".to_string(),
            })
            .collect()
    }

    #[test]
    fn newest_first_and_copying_again_moves_to_top() {
        let mut h = History::new();
        h.add(text("one"));
        h.add(text("two"));
        h.add(text("one"));
        assert_eq!(texts(&h), vec!["one", "two"]);
    }

    #[test]
    fn blank_text_is_ignored_and_long_text_is_cut() {
        let mut h = History::new();
        assert!(!h.add(text("   \n\t ")));
        assert!(h.items.is_empty());
        assert!(h.add(text(&"x".repeat(MAX_CHARS + 100))));
        assert_eq!(texts(&h)[0].chars().count(), MAX_CHARS);
    }

    #[test]
    fn keeps_only_the_newest_entries() {
        let mut h = History::new();
        for i in 0..MAX_ITEMS + 5 {
            h.add(text(&format!("item {i}")));
        }
        assert_eq!(h.items.len(), MAX_ITEMS);
        assert_eq!(texts(&h)[0], format!("item {}", MAX_ITEMS + 4));
    }

    #[test]
    fn clear_removes_text_and_images() {
        let mut h = History::new();
        h.add(text("a"));
        h.add(fake_image(10, 1));
        h.scroll_by(1);
        h.clear();
        assert!(h.items.is_empty());
        assert_eq!(h.scroll, 0);
        assert!(h.index_at(0).is_none());
    }

    #[test]
    fn scroll_and_remove_stay_in_range() {
        let mut h = History::new();
        for i in 0..7 {
            h.add(text(&format!("t{i}")));
        }
        h.scroll_by(10);
        assert_eq!(h.scroll, 4, "7 entries, 3 visible: at most 4 rows of scroll");
        assert_eq!(h.index_at(0), Some(4));
        assert_eq!(h.index_at(VISIBLE), None);
        h.remove(6);
        h.remove(5);
        h.remove(4);
        h.remove(3);
        assert_eq!(h.scroll, 0);
    }

    #[test]
    fn preview_collapses_whitespace() {
        assert_eq!(preview("  hello \n\n   world\t!  "), "hello world !");
        assert_eq!(preview(""), "");
    }

    #[test]
    fn the_same_picture_moves_to_the_top_instead_of_repeating() {
        let mut h = History::new();
        h.add(fake_image(100, 1));
        h.add(text("between"));
        h.add(fake_image(100, 1));
        assert_eq!(texts(&h), vec!["<image>", "between"]);
    }

    #[test]
    fn old_pictures_are_dropped_when_they_use_too_much_memory() {
        let mut h = History::new();
        let big = MAX_IMAGE_BYTES / 2 + 1;
        h.add(fake_image(big, 1));
        h.add(text("keep me"));
        h.add(fake_image(big, 2)); // now over the limit: the oldest picture goes
        assert_eq!(texts(&h), vec!["<image>", "keep me"]);
        assert!(matches!(&h.items[0], Content::Image(i) if i.png[0] == 2));
    }

    #[test]
    fn one_huge_picture_is_still_kept() {
        let mut h = History::new();
        h.add(fake_image(MAX_IMAGE_BYTES + 10, 1));
        assert_eq!(h.items.len(), 1, "the newest entry is never evicted");
    }

    #[test]
    fn rgba_survives_a_trip_through_a_dib() {
        // 3 x 2 pixels, each different.
        let rgba: Vec<u8> = (0..6u8).flat_map(|i| [i * 10 + 1, i * 10 + 2, i * 10 + 3, 255]).collect();
        let dib = rgba_to_dib(3, 2, &rgba);
        assert_eq!(dib.len(), 40 + 3 * 2 * 4);
        let (w, h, back) = dib_to_rgba(&dib).expect("valid dib");
        assert_eq!((w, h), (3, 2));
        assert_eq!(back, rgba);
    }

    #[test]
    fn dib_with_24_bit_pixels_and_row_padding() {
        // 2 x 2, 24 bit: each row is 6 bytes of pixels padded to 8. Rows are stored bottom first.
        let mut dib = Vec::new();
        dib.extend_from_slice(&40u32.to_le_bytes());
        dib.extend_from_slice(&2i32.to_le_bytes());
        dib.extend_from_slice(&2i32.to_le_bytes());
        dib.extend_from_slice(&1u16.to_le_bytes());
        dib.extend_from_slice(&24u16.to_le_bytes());
        dib.extend_from_slice(&[0u8; 24]);
        // bottom row (BGR): blue, green + 2 bytes of padding
        dib.extend_from_slice(&[255, 0, 0, 0, 255, 0, 0, 0]);
        // top row: red, white + padding
        dib.extend_from_slice(&[0, 0, 255, 255, 255, 255, 0, 0]);
        let (w, h, rgba) = dib_to_rgba(&dib).expect("valid dib");
        assert_eq!((w, h), (2, 2));
        assert_eq!(&rgba[0..4], &[255, 0, 0, 255], "top left is red");
        assert_eq!(&rgba[4..8], &[255, 255, 255, 255], "top right is white");
        assert_eq!(&rgba[8..12], &[0, 0, 255, 255], "bottom left is blue");
        assert_eq!(&rgba[12..16], &[0, 255, 0, 255], "bottom right is green");
    }

    #[test]
    fn damaged_or_unsupported_bitmaps_are_rejected() {
        assert!(dib_to_rgba(&[]).is_none());
        assert!(dib_to_rgba(&[0u8; 39]).is_none());
        let mut dib = rgba_to_dib(4, 4, &[255u8; 4 * 4 * 4]);
        dib.truncate(dib.len() - 10);
        assert!(dib_to_rgba(&dib).is_none(), "pixel data is cut short");
    }

    #[test]
    fn pictures_get_a_small_preview_with_the_same_shape() {
        let rgba = vec![200u8; 400 * 100 * 4];
        let img = image_from_rgba(400, 100, rgba).expect("valid picture");
        assert_eq!((img.width, img.height), (400, 100));
        assert!(img.thumb_w <= THUMB_MAX.0 && img.thumb_h <= THUMB_MAX.1);
        assert_eq!(img.thumb.len(), (img.thumb_w * img.thumb_h * 4) as usize);
        let aspect = img.thumb_w as f32 / img.thumb_h as f32;
        assert!((aspect - 4.0).abs() < 0.3, "preview keeps the 4:1 shape, got {aspect}");
        // The stored PNG decodes back to the same size.
        let again = image_from_png(img.png.clone()).expect("png decodes");
        assert_eq!((again.width, again.height), (400, 100));
    }

    #[test]
    fn tiny_and_empty_pictures_are_handled() {
        assert!(image_from_rgba(0, 10, vec![]).is_none());
        let one = image_from_rgba(1, 1, vec![1, 2, 3, 255]).expect("1x1 picture");
        assert_eq!((one.thumb_w, one.thumb_h), (1, 1));
        assert!(image_from_png(vec![1, 2, 3]).is_none(), "not a PNG");
    }
}
