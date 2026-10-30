//! The app icon, drawn in code so the tray icon and the .ico for the exe always match.
//!
//! A glossy black cloud with two white oval eyes, lit from the top-left, with a blue-to-violet glow
//! along its underside, floating over a soft shadow on a pale sky-blue rounded square.

use tiny_skia::{
    BlendMode, Color, FillRule, GradientStop, LinearGradient, Mask, MaskType, Paint, Path, PathBuilder, Pixmap, PixmapPaint,
    Point, RadialGradient, Rect, Shader, SpreadMode, Transform,
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

fn fill_with(pm: &mut Pixmap, path: &Path, shader: Shader) {
    let paint = Paint { shader, anti_alias: true, ..Paint::default() };
    pm.fill_path(path, &paint, FillRule::Winding, Transform::identity(), None);
}

/// A top-to-bottom gradient between `y0` and `y1`; each stop is (position, colour, alpha).
fn vertical(y0: f32, y1: f32, stops: &[(f32, u32, u8)]) -> Option<Shader<'static>> {
    let stops = stops.iter().map(|&(t, hex, a)| GradientStop::new(t, rgb(hex, a))).collect();
    LinearGradient::new(Point::from_xy(0.0, y0), Point::from_xy(0.0, y1), stops, SpreadMode::Pad, Transform::identity())
}

fn whole(pm: &Pixmap) -> Option<Path> {
    Some(PathBuilder::from_rect(Rect::from_xywh(0.0, 0.0, pm.width() as f32, pm.height() as f32)?))
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

/// A four-pointed sparkle of radius `r` centred on (`cx`, `cy`).
fn sparkle(cx: f32, cy: f32, r: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(cx, cy - r);
    pb.quad_to(cx, cy, cx + r, cy);
    pb.quad_to(cx, cy, cx, cy + r);
    pb.quad_to(cx, cy, cx - r, cy);
    pb.quad_to(cx, cy, cx, cy - r);
    pb.close();
    pb.finish()
}

/// A soft glow: a radial gradient from `hex` at `alpha` in the middle to nothing at the edge of an
/// ellipse with radii (`rx`, `ry`), painted over the whole pixmap.
fn glow(pm: &mut Pixmap, cx: f32, cy: f32, rx: f32, ry: f32, hex: u32, alpha: u8) {
    let stops = vec![GradientStop::new(0.0, rgb(hex, alpha)), GradientStop::new(1.0, rgb(hex, 0))];
    let origin = Point::from_xy(0.0, 0.0);
    let transform = Transform::from_scale(rx, ry).post_translate(cx, cy);
    let Some(shader) = RadialGradient::new(origin, 0.0, origin, 1.0, stops, SpreadMode::Pad, transform) else { return };
    if let Some(all) = whole(pm) {
        fill_with(pm, &all, shader);
    }
}

fn round_glow(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, hex: u32, alpha: u8) {
    glow(pm, cx, cy, r, r, hex, alpha);
}

/// The cloud's outline as four overlapping solid shapes, shifted by (`dx`, `dy`) canvas widths.
fn cloud_shape(pm: &mut Pixmap, s: f32, color: Color, dx: f32, dy: f32) {
    let (dx, dy) = (dx * s, dy * s);
    if let Some(base) = rounded_rect(0.12 * s + dx, 0.43 * s + dy, 0.76 * s, 0.40 * s, 0.20 * s) {
        fill(pm, &base, color);
    }
    for (x, y, r) in [(0.50, 0.39, 0.27), (0.28, 0.50, 0.19), (0.73, 0.50, 0.19)] {
        if let Some(puff) = circle(x * s + dx, y * s + dy, r * s) {
            fill(pm, &puff, color);
        }
    }
}

/// The cloud mascot on a canvas `s` pixels wide, drawn over whatever is already there: a soft shadow
/// and blue light beneath it, the body lit from the top-left with a glowing underside, shine
/// highlights, a light edge and the eyes. At tiny sizes the details would only smear, so the eyes
/// get bigger and plain.
fn draw_cloud(pm: &mut Pixmap, s: f32, tiny: bool) {
    let q = |v: f32| v * s;
    let (width, height) = (pm.width(), pm.height());
    glow(pm, q(0.5), q(0.90), q(0.40), q(0.10), 0x1D3FA8, 150);
    glow(pm, q(0.5), q(0.86), q(0.46), q(0.16), 0x4B8CFF, 110);

    // The body is painted on its own layer, then cut to the cloud's outline.
    let mut shape = Pixmap::new(width, height).expect("icon size is non-zero");
    cloud_shape(&mut shape, s, rgb(0x000000, 255), 0.0, 0.0);
    let mask = Mask::from_pixmap(shape.as_ref(), MaskType::Alpha);
    let mut body = Pixmap::new(width, height).expect("icon size is non-zero");
    if let Some(all) = whole(&body) {
        let centre = Point::from_xy(q(0.38), q(0.28));
        let stops = vec![
            GradientStop::new(0.0, rgb(0x5A5D78, 255)),
            GradientStop::new(0.35, rgb(0x232432, 255)),
            GradientStop::new(0.75, rgb(0x08080E, 255)),
            GradientStop::new(1.0, rgb(0x030305, 255)),
        ];
        if let Some(lit) = RadialGradient::new(centre, 0.0, centre, q(0.62), stops, SpreadMode::Pad, Transform::identity()) {
            fill_with(&mut body, &all, lit);
        }
        // Glowing underside: blue on the left, violet on the right, brightest at the bottom edge.
        round_glow(&mut body, q(0.28), q(0.92), q(0.36), 0x2D8CFF, 255);
        round_glow(&mut body, q(0.50), q(0.94), q(0.32), 0x35D6FF, 235);
        round_glow(&mut body, q(0.74), q(0.92), q(0.36), 0x9B5CFF, 255);
        if let Some(rim) = vertical(q(0.70), q(0.84), &[(0.0, 0x7FE3FF, 0), (1.0, 0xA9F0FF, 170)]) {
            fill_with(&mut body, &all, rim);
        }
    }
    // Shine: a soft glow and a crisp slanted highlight on the big puff, smaller ones on the sides.
    glow(&mut body, q(0.42), q(0.24), q(0.20), q(0.12), 0xFFFFFF, 80);
    for (x, y, rx, ry, angle, alpha) in
        [(0.405, 0.205, 0.075, 0.022, -24.0, 215), (0.215, 0.42, 0.040, 0.014, -32.0, 150), (0.66, 0.30, 0.030, 0.011, -30.0, 120)]
    {
        let Some(shine) = oval(q(x), q(y), q(rx), q(ry)) else { continue };
        let paint = Paint { shader: Shader::SolidColor(rgb(0xFFFFFF, alpha)), anti_alias: true, ..Paint::default() };
        body.fill_path(&shine, &paint, FillRule::Winding, Transform::from_rotate_at(angle, q(x), q(y)), None);
    }
    pm.draw_pixmap(0, 0, body.as_ref(), &PixmapPaint::default(), Transform::identity(), Some(&mask));

    // A thin light edge on the top-left of every puff: the outline minus itself shifted down-right.
    let mut rim = Pixmap::new(width, height).expect("icon size is non-zero");
    cloud_shape(&mut rim, s, rgb(0xFFFFFF, 255), 0.0, 0.0);
    let mut shifted = Pixmap::new(width, height).expect("icon size is non-zero");
    cloud_shape(&mut shifted, s, rgb(0xFFFFFF, 255), 0.008, 0.012);
    let cut = PixmapPaint { blend_mode: BlendMode::DestinationOut, ..PixmapPaint::default() };
    rim.draw_pixmap(0, 0, shifted.as_ref(), &cut, Transform::identity(), None);
    pm.draw_pixmap(0, 0, rim.as_ref(), &PixmapPaint { opacity: 0.42, ..PixmapPaint::default() }, Transform::identity(), None);

    // Eyes: glossy white ovals, slightly blue at the bottom, with a small glint.
    let cy = q(0.56);
    for dx in [-0.095, 0.095] {
        let cx = q(0.5 + dx);
        round_glow(pm, cx, cy, q(0.13), 0xBBD4FF, 80);
        let (rx, ry) = if tiny { (q(0.06), q(0.085)) } else { (q(0.043), q(0.068)) };
        if let (Some(eye), Some(shader)) = (oval(cx, cy, rx, ry), vertical(cy - ry, cy + ry, &[(0.0, 0xFFFFFF, 255), (1.0, 0xC9DDFF, 255)])) {
            fill_with(pm, &eye, shader);
        }
        if !tiny && let Some(glint) = oval(cx - rx * 0.25, cy - ry * 0.45, rx * 0.30, ry * 0.22) {
            fill(pm, &glint, rgb(0xFFFFFF, 255));
        }
    }
}

/// The pale sky-blue backdrop with a soft white haze behind the cloud.
fn fill_sky(pm: &mut Pixmap) {
    let Some(all) = whole(pm) else { return };
    let (w, h) = (pm.width() as f32, pm.height() as f32);
    if let Some(sky) = vertical(0.0, h, &[(0.0, 0xD7E8FF, 255), (1.0, 0xF4F8FF, 255)]) {
        fill_with(pm, &all, sky);
    }
    round_glow(pm, w * 0.5, h * 0.45, w * 0.60, 0xFFFFFF, 255);
}

/// The icon, `size` x `size`, with premultiplied pixels as tiny-skia stores them.
pub fn pixmap(size: u32) -> Pixmap {
    let s = size as f32;
    let mut pm = Pixmap::new(size, size).expect("icon size is non-zero");
    let mut layer = Pixmap::new(size, size).expect("icon size is non-zero");
    fill_sky(&mut layer);
    // A sheen across the top of the tile.
    if let (Some(all), Some(sheen)) = (whole(&layer), vertical(0.0, s * 0.30, &[(0.0, 0xFFFFFF, 90), (1.0, 0xFFFFFF, 0)])) {
        fill_with(&mut layer, &all, sheen);
    }
    draw_cloud(&mut layer, s, size <= 32);
    if size > 32 {
        for (x, y, r) in [(0.16, 0.18, 0.030), (0.86, 0.16, 0.038), (0.82, 0.78, 0.026)] {
            if let Some(star) = sparkle(s * x, s * y, s * r) {
                fill(&mut layer, &star, rgb(0x7FA6FF, 240));
            }
        }
    }

    // Clip to the rounded square.
    if let (Some(square), Some(mut mask)) = (rounded_rect(0.0, 0.0, s, s, s * 0.22), Mask::new(size, size)) {
        mask.fill_path(&square, FillRule::Winding, true, Transform::identity());
        pm.draw_pixmap(0, 0, layer.as_ref(), &PixmapPaint::default(), Transform::identity(), Some(&mask));
    }
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

/// The tall picture on the left of the installer wizard: the same pale sky, the cloud near the top,
/// and sparkles scattered below it.
#[allow(dead_code)] // only the icon-generating example uses this
pub fn wizard_banner(w: u32, h: u32) -> Pixmap {
    let (wf, hf) = (w as f32, h as f32);
    let mut pm = Pixmap::new(w, h).expect("banner size is non-zero");
    fill_sky(&mut pm);
    round_glow(&mut pm, wf * 0.5, hf * 1.02, wf * 0.9, 0x7FA6FF, 120);

    // Sparkles from a fixed sequence so the picture never changes.
    let mut seed = 0x2F6E2B1u32;
    let mut next = || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (seed >> 8) as f32 / (1u32 << 24) as f32
    };
    for _ in 0..14 {
        let (x, y) = (next() * wf, wf * 0.95 + next() * (hf - wf * 0.95));
        let (r, alpha) = (wf * (0.012 + next() * 0.022), 120.0 + next() * 120.0);
        if let Some(star) = sparkle(x, y, r) {
            fill(&mut pm, &star, rgb(0x7FA6FF, alpha as u8));
        }
    }

    // The cloud is drawn on a square canvas as wide as the banner, then placed near the top.
    if let Some(mut mascot) = Pixmap::new(w, w) {
        draw_cloud(&mut mascot, wf, false);
        pm.draw_pixmap(0, (hf * 0.07) as i32, mascot.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    }
    pm
}
