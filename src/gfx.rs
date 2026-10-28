//! Drawing helpers beyond flat fills: gradients, glows, borders and soft shadows. Colours are
//! `[r, g, b, a]` with straight (not premultiplied) alpha, like the rest of the drawing code.

use tiny_skia::{
    Color, FillRule, GradientStop, LinearGradient, Mask, Paint, Path, PathBuilder, Pixmap, Point, RadialGradient, Rect,
    SpreadMode, Stroke, Transform,
};

fn color(c: [u8; 4]) -> Color {
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
    let (a, b) = if horizontal {
        (Point::from_xy(x, y), Point::from_xy(x + w, y))
    } else {
        (Point::from_xy(x, y), Point::from_xy(x, y + h))
    };
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
    let Some(shader) = LinearGradient::new(Point::from_xy(cx, cy - r), Point::from_xy(cx, cy + r), stops, SpreadMode::Pad, Transform::identity())
    else {
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
    let stops = vec![
        GradientStop::new(0.0, color(c)),
        GradientStop::new(0.5, color(fade(c, 0.35))),
        GradientStop::new(1.0, color(fade(c, 0.0))),
    ];
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

/// A mask in the shape of `path`, to keep glows inside the notch.
pub fn mask_of(w: u32, h: u32, path: &Path) -> Option<Mask> {
    let mut mask = Mask::new(w, h)?;
    mask.fill_path(path, FillRule::Winding, true, Transform::identity());
    Some(mask)
}

#[cfg(test)]
mod tests {
    use super::*;

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
