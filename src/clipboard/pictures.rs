//! Pictures from the clipboard: Windows bitmaps and PNG data, kept as PNG with a small preview.

use image::{ImageEncoder, RgbaImage};

use super::Image;

/// Pictures with more pixels than this are not kept (an 8K screenshot has 33 million).
const MAX_PIXELS: usize = 40 * 1024 * 1024;
/// The most bytes of a picture read from the clipboard: that many pixels as an uncompressed bitmap.
pub(super) const MAX_PICTURE_BYTES: usize = MAX_PIXELS * 4 + 4096;
/// The most memory the decoding of a picture from another app may take: that many pixels at four bytes, and some room.
const MAX_DECODE_BYTES: u64 = MAX_PIXELS as u64 * 4 + 32 * 1024 * 1024;

/// A picture from another app, decoded within the limits above (and not within the image crate's defaults).
pub(super) fn decode_picture(bytes: &[u8]) -> Option<image::DynamicImage> {
    crate::gfx::decode_limited(bytes, MAX_PIXELS as u32, MAX_DECODE_BYTES)
}
/// The preview of an image fits in this box, at twice the size it is drawn so it stays sharp.
pub const THUMB_MAX: (u32, u32) = (96, 52);

/// A kept picture made from straight RGBA pixels.
pub(super) fn image_from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Option<Image> {
    if width == 0 || height == 0 || (width as usize).checked_mul(height as usize)? > MAX_PIXELS {
        return None;
    }
    let img = RgbaImage::from_raw(width, height, rgba)?;

    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(&mut png, image::codecs::png::CompressionType::Fast, image::codecs::png::FilterType::Sub)
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
pub(super) fn image_from_png(png: Vec<u8>) -> Option<Image> {
    let decoded = decode_picture(&png)?.to_rgba8();
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
pub(super) fn dib_to_rgba(dib: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
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
pub(super) fn rgba_to_dib(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
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

/// Whether the start of PNG data says it is a PNG with a size worth keeping. The size comes from the header,
/// so a huge picture is turned away before anything is copied or decoded.
pub(super) fn png_fits(head: &[u8]) -> bool {
    if head.get(..8) != Some(b"\x89PNG\r\n\x1a\n".as_slice()) {
        return false;
    }
    let side = |at: usize| head.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize);
    matches!((side(16), side(20)), (Some(w), Some(h)) if w > 0 && h > 0 && w.checked_mul(h).is_some_and(|px| px <= MAX_PIXELS))
}

/// The same for the start of a Windows bitmap.
pub(super) fn dib_fits(head: &[u8]) -> bool {
    let value = |at: usize| head.get(at..at + 4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    match (value(0), value(4), value(8)) {
        (Some(header), Some(w), Some(h)) if header >= 40 && w > 0 && h != 0 => (w as usize).checked_mul(h.unsigned_abs() as usize).is_some_and(|px| px <= MAX_PIXELS),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// A PNG that is nothing but a header saying the picture is `w` x `h` pixels (the checksum is made here).
    fn png_header_only(w: u32, h: u32) -> Vec<u8> {
        fn crc32(data: &[u8]) -> u32 {
            let mut crc = 0xFFFF_FFFFu32;
            for &b in data {
                crc ^= b as u32;
                for _ in 0..8 {
                    crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
                }
            }
            !crc
        }
        let mut ihdr = b"IHDR".to_vec();
        ihdr.extend(w.to_be_bytes());
        ihdr.extend(h.to_be_bytes());
        ihdr.extend([8, 6, 0, 0, 0]); // 8 bits, RGBA, no interlace
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        png.extend(13u32.to_be_bytes());
        png.extend(&ihdr);
        png.extend(crc32(&ihdr).to_be_bytes());
        png
    }

    #[test]
    fn a_png_that_claims_a_huge_picture_is_refused_before_anything_is_made() {
        let started = Instant::now();
        // 70,000 x 70,000 pixels would be 19.6 GB as RGBA, and the data is not even there.
        assert!(image_from_png(png_header_only(70_000, 70_000)).is_none());
        assert!(decode_picture(&png_header_only(70_000, 70_000)).is_none());
        assert!(started.elapsed() < Duration::from_secs(2), "took {:?}", started.elapsed());
    }

    #[test]
    fn an_ordinary_png_from_another_app_is_still_kept() {
        let mut png = Vec::new();
        RgbaImage::from_pixel(40, 30, image::Rgba([1, 2, 3, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let kept = image_from_png(png).expect("a small picture is kept");
        assert_eq!((kept.width, kept.height), (40, 30));
    }

    #[test]
    fn a_picture_is_judged_by_its_header_before_it_is_copied() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&1920u32.to_be_bytes());
        png.extend_from_slice(&1080u32.to_be_bytes());
        png.resize(64, 0);
        assert!(png_fits(&png));
        png[16..20].copy_from_slice(&60_000u32.to_be_bytes());
        png[20..24].copy_from_slice(&60_000u32.to_be_bytes());
        assert!(!png_fits(&png), "3.6 billion pixels");
        assert!(!png_fits(&[0; 64]), "not a PNG");

        let dib = |w: i32, h: i32| {
            let mut d = vec![0u8; 64];
            d[..4].copy_from_slice(&40i32.to_le_bytes());
            d[4..8].copy_from_slice(&w.to_le_bytes());
            d[8..12].copy_from_slice(&h.to_le_bytes());
            d
        };
        assert!(dib_fits(&dib(3840, 2160)));
        assert!(dib_fits(&dib(3840, -2160)), "a top-down bitmap has a negative height");
        assert!(!dib_fits(&dib(40_000, 40_000)));
        assert!(!dib_fits(&dib(0, 10)));
        assert!(!dib_fits(&[]));
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
