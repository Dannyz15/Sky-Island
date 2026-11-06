//! Drawing helpers beyond flat fills: gradients, glows, borders and soft shadows. Colours are
//! `[r, g, b, a]` with straight (not premultiplied) alpha, like the rest of the drawing code.

use tiny_skia::{Color, FillRule, GradientStop, LinearGradient, Mask, Paint, Path, PathBuilder, Pixmap, Point, RadialGradient, Rect, SpreadMode, Stroke, Transform};

fn color(c: [u8; 4]) -> Color {
    let c = crate::theme::map(c);
    Color::from_rgba8(c[0], c[1], c[2], c[3])
}

/// Mix two colours; `t` = 0 gives `a`, 1 gives `b`.
pub fn mix(a: [u8; 4], b: [u8; 4], t: f32) -> [u8; 4] {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    [f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2]), f(a[3], b[3])]
}

/// The same colour with its alpha scaled by `k`.
pub fn fade(c: [u8; 4], k: f32) -> [u8; 4] {
    [c[0], c[1], c[2], (c[3] as f32 * k.clamp(0.0, 1.0)).round() as u8]
}

/// A rounded rectangle outline path.
pub fn rounded_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    pb.finish()
}

/// Fill a rounded rectangle with a two-colour gradient, top to bottom or left to right.
pub fn gradient_rounded(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, from: [u8; 4], to: [u8; 4], horizontal: bool) {
    let Some(path) = rounded_path(x, y, w, h, r) else { return };
    let (a, b) = if horizontal { (Point::from_xy(x, y), Point::from_xy(x + w, y)) } else { (Point::from_xy(x, y), Point::from_xy(x, y + h)) };
    let stops = vec![GradientStop::new(0.0, color(from)), GradientStop::new(1.0, color(to))];
    let Some(shader) = LinearGradient::new(a, b, stops, SpreadMode::Pad, Transform::identity()) else { return };
    let paint = Paint { shader, anti_alias: true, ..Default::default() };
    pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
}

/// Fill a circle with a top-to-bottom gradient.
pub fn gradient_circle(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, from: [u8; 4], to: [u8; 4]) {
    gradient_rounded(pm, cx - r, cy - r, r * 2.0, r * 2.0, r, from, to, false);
}

/// Outline a rounded rectangle.
pub fn stroke_rounded(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, width: f32, c: [u8; 4]) {
    let Some(path) = rounded_path(x, y, w, h, r) else { return };
    let mut paint = Paint::default();
    let c = crate::theme::map(c);
    paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
    paint.anti_alias = true;
    let stroke = Stroke { width, ..Default::default() };
    pm.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
}

/// Stroke a clockwise arc from 12 o'clock covering `frac` of a circle, shaded from `top` at the top
/// of the circle to `bottom` at its foot.
pub fn stroke_arc_gradient(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, width: f32, frac: f32, top: [u8; 4], bottom: [u8; 4]) {
    if frac < 0.002 {
        return;
    }
    let steps = ((frac * 120.0).ceil() as usize).max(2);
    let start = -std::f32::consts::FRAC_PI_2;
    let mut pb = PathBuilder::new();
    for i in 0..=steps {
        let ang = start + std::f32::consts::TAU * frac * i as f32 / steps as f32;
        let (x, y) = (cx + r * ang.cos(), cy + r * ang.sin());
        if i == 0 {
            pb.move_to(x, y);
        } else {
            pb.line_to(x, y);
        }
    }
    let Some(path) = pb.finish() else { return };
    let stops = vec![GradientStop::new(0.0, color(top)), GradientStop::new(1.0, color(bottom))];
    let Some(shader) = LinearGradient::new(Point::from_xy(cx, cy - r), Point::from_xy(cx, cy + r), stops, SpreadMode::Pad, Transform::identity()) else {
        return;
    };
    let paint = Paint { shader, anti_alias: true, ..Default::default() };
    let cap = if frac >= 0.999 { tiny_skia::LineCap::Butt } else { tiny_skia::LineCap::Round };
    let stroke = Stroke { width, line_cap: cap, ..Default::default() };
    pm.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
}

/// A soft round glow that fades from `c` at the centre to nothing at `radius`. With a mask it is
/// clipped to the mask's shape.
pub fn glow(pm: &mut Pixmap, cx: f32, cy: f32, radius: f32, c: [u8; 4], mask: Option<&Mask>) {
    let centre = Point::from_xy(cx, cy);
    let stops = vec![GradientStop::new(0.0, color(c)), GradientStop::new(0.5, color(fade(c, 0.35))), GradientStop::new(1.0, color(fade(c, 0.0)))];
    let Some(shader) = RadialGradient::new(centre, 0.0, centre, radius, stops, SpreadMode::Pad, Transform::identity()) else {
        return;
    };
    let paint = Paint { shader, anti_alias: true, ..Default::default() };
    if let Some(rect) = Rect::from_xywh(cx - radius, cy - radius, radius * 2.0, radius * 2.0) {
        pm.fill_rect(rect, &paint, Transform::identity(), mask);
    }
}

/// A soft shadow below a rounded rectangle, built from a few growing translucent layers.
pub fn soft_shadow(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, spread: f32, strength: u8) {
    const LAYERS: u32 = 6;
    for i in 0..LAYERS {
        let t = (i + 1) as f32 / LAYERS as f32;
        let grow = spread * t;
        let alpha = (strength as f32 * (1.0 - t) * 0.5) as u8;
        if alpha == 0 {
            continue;
        }
        let path = rounded_path(x - grow, y - grow * 0.4 + spread * 0.35, w + grow * 2.0, h + grow * 2.0, r + grow);
        if let Some(path) = path {
            let mut paint = Paint::default();
            paint.set_color_rgba8(0, 0, 0, alpha);
            paint.anti_alias = true;
            pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }
}

/// Lay `src` over `dst` with its top left corner at (`dx`, `dy`), cut off at the edges of `dst`: what drawing it with
/// a plain `draw_pixmap` (no scaling, no fading, no mask) does, but several times quicker, because it only has to
/// copy or blend the pixels. A frame of the open notch does this for every layer it puts together.
pub fn blit_over(dst: &mut Pixmap, src: &Pixmap, dx: i32, dy: i32) {
    let (dw, dh, sw, sh) = (dst.width() as i32, dst.height() as i32, src.width() as i32, src.height() as i32);
    let (x0, x1) = (dx.max(0), (dx + sw).min(dw));
    let (y0, y1) = (dy.max(0), (dy + sh).min(dh));
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let n = (x1 - x0) as usize;
    let (from, to) = (src.data(), dst.data_mut());
    for y in y0..y1 {
        let s = &from[(((y - dy) * sw + (x0 - dx)) as usize) * 4..][..n * 4];
        let d = &mut to[((y * dw + x0) as usize) * 4..][..n * 4];
        let ((s, _), (d, _)) = (s.as_chunks::<4>(), d.as_chunks_mut::<4>());
        for (sp, dp) in s.iter().zip(d.iter_mut()) {
            let alpha = u32::from(sp[3]);
            if alpha == 0 {
                continue;
            }
            if alpha == 255 {
                *dp = *sp;
                continue;
            }
            // Premultiplied "over": the source, and what the source lets through of what is there.
            let through = 255 - alpha;
            for (d, s) in dp.iter_mut().zip(sp) {
                let kept = ((u32::from(*d) * through + 128) * 257) >> 16;
                *d = (u32::from(*s) + kept).min(255) as u8;
            }
        }
    }
}

/// A mask in the shape of `path`, to keep glows inside the notch.
pub fn mask_of(w: u32, h: u32, path: &Path) -> Option<Mask> {
    let mut mask = Mask::new(w, h)?;
    mask.fill_path(path, FillRule::Winding, true, Transform::identity());
    Some(mask)
}

/// A picture from outside (another app's clipboard, a player's cover) decoded within limits that are set here and do
/// not depend on the defaults of the image crate: no side longer than `max_side` pixels and at most `max_alloc` bytes
/// of memory for the decoding. `None` for a picture that is too big, damaged, or not a picture. The limits are checked
/// from the header on, before the pixels are made.
pub fn decode_limited(bytes: &[u8], max_side: u32, max_alloc: u64) -> Option<image::DynamicImage> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(max_side);
    limits.max_image_height = Some(max_side);
    limits.max_alloc = Some(max_alloc);
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    reader.limits(limits);
    reader.decode().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_bytes(w: u32, h: u32) -> Vec<u8> {
        use image::ImageEncoder;
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out).write_image(&vec![200u8; (w * h * 4) as usize], w, h, image::ExtendedColorType::Rgba8).unwrap();
        out
    }

    #[test]
    fn a_picture_is_decoded_only_within_the_limits_given() {
        let png = png_bytes(64, 32);
        let ok = decode_limited(&png, 64, 1 << 20).expect("within the limits");
        assert_eq!((ok.width(), ok.height()), (64, 32));
        assert!(decode_limited(&png, 63, 1 << 20).is_none(), "one side too long");
        assert!(decode_limited(&png, 4096, 100).is_none(), "more memory than allowed");
        assert!(decode_limited(b"not a picture at all", 4096, 1 << 20).is_none());
        assert!(decode_limited(&[], 4096, 1 << 20).is_none());
    }

    /// A picture of every kind of pixel (clear, solid, half clear), made the same way each time.
    fn patterned(w: u32, h: u32, seed: u32) -> Pixmap {
        let mut pm = Pixmap::new(w, h).unwrap();
        let mut state = seed;
        for px in pm.pixels_mut() {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let a = match (state >> 24) % 4 {
                0 => 0,
                1 => 255,
                _ => ((state >> 8) & 0xFF) as u8,
            };
            // Premultiplied: no colour is above the alpha.
            let c = |shift: u32| (((state >> shift) & 0xFF) * u32::from(a) / 255) as u8;
            *px = tiny_skia::PremultipliedColorU8::from_rgba(c(0), c(5), c(10), a).unwrap();
        }
        pm
    }

    #[test]
    fn blit_over_does_what_draw_pixmap_does_at_a_whole_pixel_offset() {
        let src = patterned(37, 23, 7);
        for (dx, dy) in [(0, 0), (5, 3), (-4, -9), (20, 10), (-30, 2), (40, 40), (-50, -50)] {
            let (mut quick, mut slow) = (patterned(50, 40, 99), patterned(50, 40, 99));
            blit_over(&mut quick, &src, dx, dy);
            slow.draw_pixmap(dx, dy, src.as_ref(), &tiny_skia::PixmapPaint::default(), Transform::identity(), None);
            let worst = quick.data().iter().zip(slow.data()).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
            assert!(worst <= 2, "offset ({dx}, {dy}): the pictures differ by {worst}");
        }
    }

    #[test]
    fn blit_over_a_clear_picture_changes_nothing_and_a_solid_one_replaces() {
        let mut dst = patterned(20, 20, 3);
        let before = dst.data().to_vec();
        blit_over(&mut dst, &Pixmap::new(20, 20).unwrap(), 0, 0);
        assert_eq!(dst.data(), &before[..], "nothing to lay over");
        let mut solid = Pixmap::new(6, 6).unwrap();
        solid.fill(Color::from_rgba8(10, 20, 30, 255));
        blit_over(&mut dst, &solid, 4, 4);
        assert_eq!(dst.pixel(4, 4).unwrap().red(), 10);
        assert_eq!(dst.pixel(9, 9).unwrap().blue(), 30);
        assert_ne!(dst.pixel(10, 10).unwrap().blue(), 30, "only the six by six square");
    }

    #[test]
    fn mixing_and_fading_colours() {
        assert_eq!(mix([0, 0, 0, 255], [200, 100, 50, 255], 0.5), [100, 50, 25, 255]);
        assert_eq!(mix([10, 10, 10, 0], [20, 20, 20, 100], 2.0), [20, 20, 20, 100], "t is clamped");
        assert_eq!(fade([1, 2, 3, 200], 0.5), [1, 2, 3, 100]);
        assert_eq!(fade([1, 2, 3, 200], 3.0), [1, 2, 3, 200]);
    }

    #[test]
    fn gradients_and_glows_paint_something() {
        let mut pm = Pixmap::new(40, 40).unwrap();
        gradient_rounded(&mut pm, 4.0, 4.0, 32.0, 32.0, 8.0, [255, 0, 0, 255], [0, 0, 255, 255], false);
        let top = pm.pixel(20, 6).unwrap();
        let bottom = pm.pixel(20, 34).unwrap();
        assert!(top.red() > top.blue() && bottom.blue() > bottom.red(), "red fades to blue downwards");
        assert_eq!(pm.pixel(0, 0).unwrap().alpha(), 0, "outside the shape stays clear");

        let mut pm = Pixmap::new(40, 40).unwrap();
        glow(&mut pm, 20.0, 20.0, 18.0, [0, 200, 255, 200], None);
        assert!(pm.pixel(20, 20).unwrap().alpha() > pm.pixel(20, 34).unwrap().alpha(), "glow is strongest in the middle");
    }

    #[test]
    fn mask_keeps_a_glow_inside_its_shape() {
        let shape = rounded_path(10.0, 10.0, 20.0, 20.0, 4.0).unwrap();
        let mask = mask_of(40, 40, &shape).unwrap();
        let mut pm = Pixmap::new(40, 40).unwrap();
        glow(&mut pm, 20.0, 20.0, 30.0, [255, 255, 255, 255], Some(&mask));
        assert!(pm.pixel(20, 20).unwrap().alpha() > 0);
        assert_eq!(pm.pixel(5, 5).unwrap().alpha(), 0);
    }
}
