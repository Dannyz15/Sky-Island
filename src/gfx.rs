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

/// The rim of a piece of glass: bright along its top edge, fading down its sides, with a fainter light along the
/// bottom where light comes back through it. `k` scales how strongly it shows.
pub fn glass_rim(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, width: f32, k: f32) {
    let Some(path) = rounded_path(x + width / 2.0, y + width / 2.0, w - width, h - width, (r - width / 2.0).max(0.0)) else { return };
    let a = |v: f32| (v * k).clamp(0.0, 255.0) as u8;
    let stops = vec![
        GradientStop::new(0.0, color([255, 255, 255, a(150.0)])),
        GradientStop::new(0.35, color([255, 255, 255, a(34.0)])),
        GradientStop::new(0.75, color([255, 255, 255, a(12.0)])),
        GradientStop::new(1.0, color([255, 255, 255, a(60.0)])),
    ];
    let Some(shader) = LinearGradient::new(Point::from_xy(x, y), Point::from_xy(x, y + h), stops, SpreadMode::Pad, Transform::identity()) else { return };
    let paint = Paint { shader, anti_alias: true, ..Default::default() };
    pm.stroke_path(&path, &paint, &Stroke { width, ..Default::default() }, Transform::identity(), None);
}

/// A small piece of liquid glass for a control (with the glass looks of the notch): a faint body, lighter at the
/// top, a soft shine across its upper part and a bright rim. `k` scales how strongly it shows (hover, fade), `tint`
/// colours it (a chosen option, a lit button); `s` is the scale.
pub fn glass_piece(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, k: f32, tint: Option<[u8; 3]>, s: f32) {
    if k <= 0.0 || w <= 0.0 || h <= 0.0 {
        return;
    }
    let a = |v: f32| (v * k).clamp(0.0, 255.0) as u8;
    match tint {
        Some([tr, tg, tb]) => gradient_rounded(pm, x, y, w, h, r, mix([tr, tg, tb, a(96.0)], [255, 255, 255, a(96.0)], 0.18), [tr, tg, tb, a(44.0)], false),
        None => gradient_rounded(pm, x, y, w, h, r, [255, 255, 255, a(24.0)], [255, 255, 255, a(7.0)], false),
    }
    // The shine: a soft light across the upper part, inset from the round ends.
    let inset = (r * 0.45).min(w / 4.0);
    let sh = h * 0.46;
    gradient_rounded(pm, x + inset, y + 1.0 * s, w - 2.0 * inset, sh, (r * 0.8).min(sh / 2.0), [255, 255, 255, a(30.0)], [255, 255, 255, 0], false);
    glass_rim(pm, x, y, w, h, r, 1.0 * s, k);
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

/// How strong `glow` (of this alpha) is at each pixel of a `w` x `h` picture, kept as a mask. The same glow in
/// another colour is then `fill_through` the mask: 1.3 ms instead of 3.4 ms for a window-sized glow, the same picture.
pub fn glow_mask(w: u32, h: u32, cx: f32, cy: f32, radius: f32, alpha: u8) -> Option<Mask> {
    let mut shape = Pixmap::new(w, h)?;
    glow(&mut shape, cx, cy, radius, [255, 255, 255, alpha], None);
    Some(Mask::from_pixmap(shape.as_ref(), tiny_skia::MaskType::Alpha))
}

/// The whole picture in colour `c`, as much of it as `mask` (the same size) lets through.
pub fn fill_through(pm: &mut Pixmap, c: [u8; 3], mask: &Mask) {
    let mut paint = Paint::default();
    paint.set_color(color([c[0], c[1], c[2], 255]));
    paint.anti_alias = true;
    if let Some(rect) = Rect::from_xywh(0.0, 0.0, pm.width() as f32, pm.height() as f32) {
        pm.fill_rect(rect, &paint, Transform::identity(), Some(mask));
    }
}

/// A soft shadow below a rounded rectangle, built from a few growing translucent layers.
pub fn soft_shadow(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, spread: f32, strength: u8) {
    shadow_layers(pm, x, y, w, h, r, spread, strength, tiny_skia::BlendMode::SourceOver, false);
}

/// The same shadow, drawn behind a solid shape that fills the rectangle (so it shows only outside it). The part under the
/// shape is left out: it would be covered anyway, and for a big notch it is most of the cost of the shadow.
pub fn soft_shadow_behind(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, spread: f32, strength: u8) {
    shadow_layers(pm, x, y, w, h, r, spread, strength, tiny_skia::BlendMode::DestinationOver, true);
}

/// How far inside the rectangle the shadow's hole starts when it is left out under the shape, in pixels: the shape's soft
/// edge lies over the shadow a little.
const SHADOW_HOLE_INSET: f32 = 2.0;

#[allow(clippy::too_many_arguments)]
fn shadow_layers(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, spread: f32, strength: u8, blend: tiny_skia::BlendMode, leave_hole: bool) {
    let hole = if leave_hole {
        rounded_path(x + SHADOW_HOLE_INSET, y + SHADOW_HOLE_INSET, w - 2.0 * SHADOW_HOLE_INSET, h - 2.0 * SHADOW_HOLE_INSET, (r - SHADOW_HOLE_INSET).max(0.0))
    } else {
        None
    };
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
            paint.blend_mode = blend;
            // With the hole, the layer is the ring between the two outlines.
            let (path, rule) = match &hole {
                Some(hole) => {
                    let mut pb = PathBuilder::new();
                    pb.push_path(&path);
                    pb.push_path(hole);
                    (pb.finish().unwrap_or(path), FillRule::EvenOdd)
                }
                None => (path, FillRule::Winding),
            };
            pm.fill_path(&path, &paint, rule, Transform::identity(), None);
        }
    }
}

/// Keep only what of `pm` lies inside the shape drawn on `shape` (by its alpha), `pm` lying at (`dx`, `dy`) on it.
pub fn clip_to(pm: &mut Pixmap, shape: &Pixmap, dx: i32, dy: i32) {
    let (w, sw, sh) = (pm.width() as i32, shape.width() as i32, shape.height() as i32);
    let mask = shape.pixels();
    for (i, px) in pm.pixels_mut().iter_mut().enumerate() {
        if px.alpha() == 0 {
            continue;
        }
        let (x, y) = (i as i32 % w + dx, i as i32 / w + dy);
        let m = if x >= 0 && y >= 0 && x < sw && y < sh { mask[(y * sw + x) as usize].alpha() as u32 } else { 0 };
        if m < 255 {
            let k = |v: u8| ((v as u32 * m + 127) / 255) as u8;
            *px = tiny_skia::PremultipliedColorU8::from_rgba(k(px.red()), k(px.green()), k(px.blue()), k(px.alpha())).unwrap_or(tiny_skia::PremultipliedColorU8::TRANSPARENT);
        }
    }
}

/// Blur a picture in place by about `r` pixels (two box passes each way, close to a gaussian). Works on the
/// premultiplied pixels directly, so clear areas stay clear. Used for the open notch's content coming in from a blur.
pub fn blur(pm: &mut Pixmap, r: usize) {
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    if r == 0 || w == 0 || h == 0 {
        return;
    }
    // Two passes of a box half as wide come close to one soft blur of `r`.
    let rad = r.div_ceil(2);
    let data = pm.data_mut();
    let mut line: Vec<[u32; 4]> = Vec::with_capacity(w.max(h));
    for _ in 0..2 {
        for y in 0..h {
            box_pass(data, &mut line, y * w * 4, 4, w, rad);
        }
        for x in 0..w {
            box_pass(data, &mut line, x * 4, w * 4, h, rad);
        }
    }
}

/// A copy of a picture `f` times smaller each way (`f` = 1 copies it). Blurring a smaller copy and drawing it back
/// `f` times larger looks the same as blurring the picture, and is many times quicker.
pub fn shrink(pm: &Pixmap, f: u32) -> Option<Pixmap> {
    let f = f.max(1);
    let mut small = Pixmap::new(pm.width().div_ceil(f), pm.height().div_ceil(f))?;
    let k = 1.0 / f as f32;
    let paint = tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
    small.draw_pixmap(0, 0, pm.as_ref(), &paint, Transform::from_scale(k, k), None);
    Some(small)
}

/// One box blur of radius `r` along a line of `n` pixels starting at byte `start`, `step` bytes apart; the ends are
/// repeated past the edge.
fn box_pass(data: &mut [u8], line: &mut Vec<[u32; 4]>, start: usize, step: usize, n: usize, r: usize) {
    line.clear();
    line.extend((0..n).map(|i| {
        let o = start + i * step;
        [data[o] as u32, data[o + 1] as u32, data[o + 2] as u32, data[o + 3] as u32]
    }));
    let width = (2 * r + 1) as u32;
    let last = n as isize - 1;
    let at = |i: isize| line[i.clamp(0, last) as usize];
    let mut sum = [0u32; 4];
    for i in -(r as isize)..=(r as isize) {
        let p = at(i);
        (0..4).for_each(|c| sum[c] += p[c]);
    }
    for i in 0..n as isize {
        let o = start + i as usize * step;
        (0..4).for_each(|c| data[o + c] = (sum[c] / width) as u8);
        let (add, sub) = (at(i + r as isize + 1), at(i - r as isize));
        (0..4).for_each(|c| sum[c] = sum[c] + add[c] - sub[c]);
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

/// `layer` laid over `dst` (both the same size, premultiplied pixels) at `opacity`, 0 to 1, moved `dx` pixels to
/// the right, and only from column `from_x` on. The same picture as `draw_pixmap` with a pure move, much faster,
/// because clear pixels are skipped and nothing is sampled: the settings window fades a page in with this, one
/// frame in 7 ms on a 144 Hz screen.
pub fn blend_layer(dst: &mut Pixmap, layer: &Pixmap, dx: i32, opacity: f32, from_x: usize) {
    let (w, h) = (dst.width() as usize, dst.height() as usize);
    if (layer.width() as usize, layer.height() as usize) != (w, h) {
        return;
    }
    let op = (opacity.clamp(0.0, 1.0) * 256.0).round() as u32;
    // The columns of `dst` that have something of the layer under them.
    let start = from_x.max(dx.max(0) as usize);
    let end = (w as i64 + dx.min(0) as i64).clamp(0, w as i64) as usize;
    if op == 0 || start >= end {
        return;
    }
    let (src_data, dst_data) = (layer.data(), dst.data_mut());
    for y in 0..h {
        let row = y * w * 4;
        let (from, to) = (row + start * 4, row + end * 4);
        let src_from = (row as i64 + (start as i64 - dx as i64) * 4) as usize;
        let src = &src_data[src_from..src_from + (to - from)];
        let ((dst_px, _), (src_px, _)) = (dst_data[from..to].as_chunks_mut::<4>(), src.as_chunks::<4>());
        for (d, s) in dst_px.iter_mut().zip(src_px) {
            let sa = s[3] as u32;
            if sa == 0 {
                continue;
            }
            let keep = 256 - ((sa * op + 128) >> 8);
            for c in 0..4 {
                d[c] = ((s[c] as u32 * op + d[c] as u32 * keep + 128) >> 8).min(255) as u8;
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
    fn a_shadow_behind_a_solid_shape_looks_the_same_without_the_part_under_it() {
        let (w, h) = (300u32, 220u32);
        let body = |pm: &mut Pixmap, c: [u8; 4]| {
            if let Some(path) = rounded_path(20.0, -40.0, 260.0, 220.0, 24.0) {
                let mut paint = Paint::default();
                paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
                paint.anti_alias = true;
                pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
            }
        };
        let mut ring = Pixmap::new(w, h).unwrap();
        body(&mut ring, [200, 40, 40, 255]);
        soft_shadow_behind(&mut ring, 20.0, -40.0, 260.0, 220.0, 24.0, 10.0, 150);
        // The same with the whole shadow laid under it.
        let mut full = Pixmap::new(w, h).unwrap();
        shadow_layers(&mut full, 20.0, -40.0, 260.0, 220.0, 24.0, 10.0, 150, tiny_skia::BlendMode::SourceOver, false);
        let mut under = Pixmap::new(w, h).unwrap();
        body(&mut under, [200, 40, 40, 255]);
        full.draw_pixmap(0, 0, under.as_ref(), &tiny_skia::PixmapPaint::default(), Transform::identity(), None);
        let differ = ring.pixels().iter().zip(full.pixels()).filter(|(a, b)| a.red().abs_diff(b.red()) > 2 || a.alpha().abs_diff(b.alpha()) > 5).count();
        assert_eq!(differ, 0, "{differ} pixels differ");
        assert!(ring.pixel(150, 80).unwrap().alpha() == 255, "the shape itself is untouched");
        assert!(ring.pixel(150, 188).unwrap().alpha() > 0, "the shadow shows below it");
    }

    #[test]
    fn blur_keeps_clear_clear_and_solid_solid_and_softens_an_edge() {
        let mut clear = Pixmap::new(30, 20).unwrap();
        blur(&mut clear, 6);
        assert!(clear.data().iter().all(|&b| b == 0));
        let mut solid = Pixmap::new(30, 20).unwrap();
        solid.fill(Color::from_rgba8(200, 100, 50, 255));
        let before = solid.data().to_vec();
        blur(&mut solid, 6);
        assert_eq!(solid.data(), &before[..]);
        // Left half white, right half clear: after the blur the edge is a ramp, and every pixel stays premultiplied.
        let mut edge = Pixmap::new(40, 4).unwrap();
        for y in 0..4 {
            for x in 0..20 {
                edge.pixels_mut()[y * 40 + x] = tiny_skia::PremultipliedColorU8::from_rgba(255, 255, 255, 255).unwrap();
            }
        }
        blur(&mut edge, 6);
        let a: Vec<u8> = (0..40).map(|x| edge.pixels()[x].alpha()).collect();
        assert_eq!(a[0], 255);
        assert_eq!(a[39], 0);
        assert!(a[19] > 100 && a[19] < 255 && a[20] > 0 && a[20] < 160, "{a:?}");
        assert!(a.windows(2).all(|w| w[0] >= w[1]), "{a:?}");
        assert!(edge.pixels().iter().all(|p| p.red() <= p.alpha()));
    }

    #[test]
    #[ignore = "a timing, not a check: cargo test --release -- --ignored time_the_blur --nocapture"]
    fn time_the_blur() {
        // The open notch's content at 150 %: 420 x 176 logical pixels.
        let mut pm = patterned(630, 264, 5);
        let start = std::time::Instant::now();
        for _ in 0..20 {
            blur(&mut pm, 15);
        }
        println!("blur: {:.2} ms", start.elapsed().as_secs_f64() * 1000.0 / 20.0);
        let start = std::time::Instant::now();
        for _ in 0..20 {
            let mut small = shrink(&pm, 4).unwrap();
            blur(&mut small, 4);
        }
        println!("shrunk 4x and blurred: {:.2} ms", start.elapsed().as_secs_f64() * 1000.0 / 20.0);
        let start = std::time::Instant::now();
        for _ in 0..20 {
            let mut small = shrink(&pm, 2).unwrap();
            blur(&mut small, 4);
        }
        println!("shrunk 2x and blurred: {:.2} ms", start.elapsed().as_secs_f64() * 1000.0 / 20.0);
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

    #[test]
    fn a_layer_blended_by_hand_matches_the_library_blend() {
        // A layer with clear parts, soft edges and solid parts, over a background that is not flat.
        let (w, h) = (90, 40);
        let mut layer = Pixmap::new(w, h).unwrap();
        gradient_rounded(&mut layer, 20.0, 5.0, 60.0, 30.0, 8.0, [255, 120, 40, 200], [40, 90, 255, 90], false);
        fill_circle_for_test(&mut layer);
        let mut back = Pixmap::new(w, h).unwrap();
        gradient_rounded(&mut back, 0.0, 0.0, w as f32, h as f32, 0.0, [13, 13, 18, 255], [90, 90, 110, 255], false);
        for (dx, opacity) in [(0, 1.0), (7, 0.5), (17, 0.23), (3, 0.97), (0, 0.0)] {
            let (mut a, mut b) = (back.clone(), back.clone());
            let paint = tiny_skia::PixmapPaint { opacity, ..Default::default() };
            a.draw_pixmap(0, 0, layer.as_ref(), &paint, Transform::from_translate(dx as f32, 0.0), None);
            blend_layer(&mut b, &layer, dx, opacity, 0);
            let worst = a.data().iter().zip(b.data()).map(|(x, y)| (*x as i32 - *y as i32).abs()).max().unwrap();
            assert!(worst <= 2, "dx {dx} opacity {opacity}: differs by {worst}");
        }
        // Nothing left of `from_x` is touched.
        let mut c = back.clone();
        blend_layer(&mut c, &layer, 0, 1.0, 50);
        assert_eq!(&c.data()[..50 * 4], &back.data()[..50 * 4]);
    }

    fn fill_circle_for_test(pm: &mut Pixmap) {
        let mut paint = tiny_skia::Paint::default();
        paint.set_color_rgba8(255, 255, 255, 255);
        paint.anti_alias = true;
        let path = PathBuilder::from_circle(30.0, 20.0, 9.0).unwrap();
        pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }

    #[test]
    fn a_glow_through_its_mask_is_the_same_picture_as_the_glow() {
        let (w, h) = (300u32, 240u32);
        let mask = glow_mask(w, h, 276.0, 10.0, 165.0, 20).unwrap();
        for tint in [[96u8, 110, 255], [255, 176, 64], [142, 148, 168]] {
            let (mut a, mut b) = (Pixmap::new(w, h).unwrap(), Pixmap::new(w, h).unwrap());
            glow(&mut a, 276.0, 10.0, 165.0, [tint[0], tint[1], tint[2], 20], None);
            fill_through(&mut b, tint, &mask);
            let worst = a.data().iter().zip(b.data()).map(|(x, y)| (*x as i32 - *y as i32).abs()).max().unwrap();
            assert!(worst <= 1, "{tint:?} differs by {worst}");
        }
    }
}
