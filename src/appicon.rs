//! The app icon, drawn in code so the tray icon and the .ico for the exe always match.
//!
//! A small night-to-day sky inside a rounded square, a soft cloud at the bottom, and the black
//! notch hanging from the top edge with two eyes looking out of it.

use tiny_skia::{
    Color, FillRule, GradientStop, LinearGradient, Paint, Path, PathBuilder, Pixmap, Point, Rect, SpreadMode, Transform,
};

fn rgb(hex: u32, alpha: u8) -> Color {
    Color::from_rgba8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8, alpha)
}

fn fill(pm: &mut Pixmap, path: &Path, color: Color) {
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    pm.fill_path(path, &paint, FillRule::Winding, Transform::identity(), None);
}

/// Fill `path` with a top-to-bottom gradient between `y0` and `y1`.
fn fill_vertical_gradient(pm: &mut Pixmap, path: &Path, y0: f32, y1: f32, stops: Vec<GradientStop>) {
    let Some(shader) =
        LinearGradient::new(Point::from_xy(0.0, y0), Point::from_xy(0.0, y1), stops, SpreadMode::Pad, Transform::identity())
    else {
        return;
    };
    let mut paint = Paint::default();
    paint.shader = shader;
    paint.anti_alias = true;
    pm.fill_path(path, &paint, FillRule::Winding, Transform::identity(), None);
}

fn circle(cx: f32, cy: f32, r: f32) -> Option<Path> {
    PathBuilder::from_circle(cx, cy, r)
}

fn oval(cx: f32, cy: f32, rx: f32, ry: f32) -> Option<Path> {
    PathBuilder::from_oval(Rect::from_xywh(cx - rx, cy - ry, rx * 2.0, ry * 2.0)?)
}

/// A rounded rectangle with every corner radius `r`.
fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
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

fn sky_stops() -> Vec<GradientStop> {
    vec![
        GradientStop::new(0.0, rgb(0x1A1F5C, 255)),
        GradientStop::new(0.55, rgb(0x3D5FE0, 255)),
        GradientStop::new(1.0, rgb(0x9BDCFF, 255)),
    ]
}

/// A cloud with a flat base: `left`/`base_top` place the base, `w` is its width.
fn draw_cloud(pm: &mut Pixmap, left: f32, base_top: f32, w: f32) {
    let color = rgb(0xF4F8FF, 255);
    if let Some(base) = rounded_rect(left, base_top, w, w * 0.1765, w * 0.088) {
        fill(pm, &base, color);
    }
    // Three puffs on the base: (centre x, centre y, radius) as fractions of the width.
    for (x, y, r) in [(0.265, -0.059, 0.162), (0.529, -0.132, 0.221), (0.779, -0.044, 0.147)] {
        if let Some(puff) = circle(left + w * x, base_top + w * y, w * r) {
            fill(pm, &puff, color);
        }
    }
}

/// The black notch hanging from the top edge of a canvas `s` pixels wide, with two eyes in it.
/// At tiny sizes the eye details would only smear, so the eyes get bigger and plain white.
fn draw_notch(pm: &mut Pixmap, s: f32, tiny: bool) {
    let (nw, nh) = (s * 0.64, s * 0.38);
    let (nx, nr) = ((s - nw) / 2.0, nh * 0.48);
    let mut pb = PathBuilder::new();
    pb.move_to(nx, 0.0);
    pb.line_to(nx + nw, 0.0);
    pb.line_to(nx + nw, nh - nr);
    pb.quad_to(nx + nw, nh, nx + nw - nr, nh);
    pb.line_to(nx + nr, nh);
    pb.quad_to(nx, nh, nx, nh - nr);
    pb.close();
    if let Some(notch) = pb.finish() {
        fill(pm, &notch, rgb(0x05060D, 255));
    }

    let (rx, ry) = if tiny { (s * 0.075, s * 0.105) } else { (s * 0.056, s * 0.085) };
    for dx in [-0.135, 0.135] {
        let cx = s * (0.5 + dx);
        let cy = nh * 0.50;
        if let Some(eye) = oval(cx, cy, rx, ry) {
            fill(pm, &eye, rgb(0xFFFFFF, 255));
        }
        if tiny {
            continue;
        }
        if let Some(pupil) = oval(cx + s * 0.010, cy + s * 0.012, s * 0.030, s * 0.045) {
            fill(pm, &pupil, rgb(0x0B1030, 255));
        }
        if let Some(glint) = circle(cx + s * 0.020, cy - s * 0.014, s * 0.011) {
            fill(pm, &glint, rgb(0xFFFFFF, 255));
        }
    }
}

/// The icon, `size` x `size`, with premultiplied pixels as tiny-skia stores them.
pub fn pixmap(size: u32) -> Pixmap {
    let s = size as f32;
    let mut pm = Pixmap::new(size, size).expect("icon size is non-zero");

    // Sky: deep indigo at the top, through blue, to a pale horizon at the bottom.
    if let Some(bg) = rounded_rect(0.0, 0.0, s, s, s * 0.22) {
        fill_vertical_gradient(&mut pm, &bg, 0.0, s, sky_stops());
        // Soft sheen over the upper half, fading out.
        fill_vertical_gradient(
            &mut pm,
            &bg,
            0.0,
            s * 0.55,
            vec![GradientStop::new(0.0, rgb(0xFFFFFF, 46)), GradientStop::new(1.0, rgb(0xFFFFFF, 0))],
        );
    }

    // A few stars in the dark part of the sky.
    for (x, y, r) in [(0.16, 0.30, 0.016), (0.84, 0.26, 0.020), (0.74, 0.46, 0.012), (0.26, 0.50, 0.012)] {
        if let Some(star) = circle(s * x, s * y, s * r) {
            fill(&mut pm, &star, rgb(0xFFFFFF, 230));
        }
    }

    // Cloud near the bottom.
    draw_cloud(&mut pm, s * 0.16, s * 0.76, s * 0.68);

    draw_notch(&mut pm, s, size <= 24);

    pm
}

/// Straight RGBA, `size` x `size`, as icon formats want it.
pub fn rgba(size: u32) -> Vec<u8> {
    pixmap(size)
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect()
}

/// The tall picture on the left of the installer wizard: the sky, a few stars, the notch with its
/// eyes hanging from the top, and clouds.
#[allow(dead_code)] // only the icon-generating example uses this
pub fn wizard_banner(w: u32, h: u32) -> Pixmap {
    let (wf, hf) = (w as f32, h as f32);
    let mut pm = Pixmap::new(w, h).expect("banner size is non-zero");
    if let Some(rect) = Rect::from_xywh(0.0, 0.0, wf, hf) {
        fill_vertical_gradient(&mut pm, &PathBuilder::from_rect(rect), 0.0, hf, sky_stops());
    }

    // Stars scattered in the upper part, from a fixed sequence so the picture never changes.
    let mut seed = 0x2F6E2B1u32;
    let mut next = || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (seed >> 8) as f32 / (1u32 << 24) as f32
    };
    for _ in 0..16 {
        let (x, y, r) = (next() * wf, next() * hf * 0.55, wf * (0.006 + next() * 0.008));
        if let Some(star) = circle(x, y, r) {
            fill(&mut pm, &star, rgb(0xFFFFFF, 200));
        }
    }

    draw_notch(&mut pm, wf, false);

    draw_cloud(&mut pm, wf * 0.10, hf * 0.86, wf * 0.55);
    draw_cloud(&mut pm, wf * 0.52, hf * 0.93, wf * 0.38);
    pm
}
