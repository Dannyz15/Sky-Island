//! Drawing primitives and small widgets.

use crate::*;

/// A checkbox of `size` physical pixels, ticked when the task is done.
/// A pill-shaped choice: tinted with `accent` when chosen, a soft highlight while the mouse is over it.
pub(crate) fn draw_chip(pm: &mut Pixmap, font: &Font, r: (f32, f32, f32, f32), label: &str, on: bool, hot: f32, accent: [u8; 3], s: f32, alpha: f32) {
    let (x, y, w, h) = (r.0 * s, r.1 * s, r.2 * s, r.3 * s);
    let a = (alpha * 255.0) as u8;
    if on {
        gfx::gradient_rounded(pm, x, y, w, h, h / 2.0, [accent[0], accent[1], accent[2], (alpha * 70.0) as u8], [accent[0], accent[1], accent[2], (alpha * 34.0) as u8], false);
        gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [accent[0], accent[1], accent[2], (alpha * 90.0) as u8]);
    } else if hot > 0.01 {
        fill_rounded(pm, x, y, w, h, h / 2.0, [255, 255, 255, (alpha * 16.0 * hot) as u8]);
    }
    let color = if on { [255, 255, 255, a] } else { pick([225, 225, 232, a], [150, 150, 160, a], hot) };
    draw_text_centered(pm, font, label, (r.0 + r.2 / 2.0) * s, (r.1 + 13.0) * s, 10.0 * s, color, r.2 * s);
}

/// A small round button, brighter under the mouse and dimmed when it cannot be used.
pub(crate) fn draw_round_button(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, hot: f32, enabled: bool, alpha: f32) {
    let base = if !enabled { 12.0 } else { pick(44.0, 26.0, hot) };
    fill_circle(pm, cx, cy, r, [255, 255, 255, (alpha * base) as u8]);
}

/// The big round start / add button: a gradient in the tab's colour with a soft glow.
pub(crate) fn draw_big_button(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, hot: f32, rgb: [u8; 3], alpha: f32) {
    let a = (alpha * 255.0) as u8;
    let accent = [rgb[0], rgb[1], rgb[2], a];
    gfx::glow(pm, cx, cy, r * 2.0, [rgb[0], rgb[1], rgb[2], (alpha * pick(90.0, 56.0, hot)) as u8], None);
    let r = pick(r + 1.2 * (r / 18.0), r, hot);
    gfx::gradient_circle(pm, cx, cy, r, gfx::mix(accent, [255, 255, 255, a], 0.3), accent);
}

/// Water inside a circle, filled to `frac` with two soft waves on top.
pub(crate) fn draw_water_fill(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, frac: f32, rgb: [u8; 3], s: f32, alpha: f32) {
    if frac <= 0.0 {
        return;
    }
    let Some(circle) = PathBuilder::from_circle(cx, cy, r) else { return };
    let Some(mask) = gfx::mask_of(pm.width(), pm.height(), &circle) else { return };
    let level = cy + r - 2.0 * r * frac.clamp(0.0, 1.0);
    for (phase, amp, strong, faint) in [(0.0f32, 2.2f32, 64.0f32, 24.0f32), (2.1, 1.6, 96.0, 44.0)] {
        let mut pb = PathBuilder::new();
        let steps = 28;
        for i in 0..=steps {
            let x = cx - r + 2.0 * r * i as f32 / steps as f32;
            let y = level + (x / s * 0.16 + phase + frac * 40.0).sin() * amp * s;
            if i == 0 {
                pb.move_to(x, y);
            } else {
                pb.line_to(x, y);
            }
        }
        pb.line_to(cx + r, cy + r + 4.0 * s);
        pb.line_to(cx - r, cy + r + 4.0 * s);
        pb.close();
        let Some(path) = pb.finish() else { continue };
        let stops = vec![
            tiny_skia::GradientStop::new(0.0, Color::from_rgba8(rgb[0], rgb[1], rgb[2], (alpha * strong) as u8)),
            tiny_skia::GradientStop::new(1.0, Color::from_rgba8(rgb[0], rgb[1], rgb[2], (alpha * faint) as u8)),
        ];
        let from = tiny_skia::Point::from_xy(cx, level);
        let to = tiny_skia::Point::from_xy(cx, cy + r);
        let Some(shader) = tiny_skia::LinearGradient::new(from, to, stops, tiny_skia::SpreadMode::Pad, Transform::identity()) else {
            continue;
        };
        let paint = Paint { shader, anti_alias: true, ..Default::default() };
        pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), Some(&mask));
    }
}

pub(crate) fn draw_check(pm: &mut Pixmap, x: f32, y: f32, size: f32, done: bool, amber: [u8; 4], a: u8) {
    let k = size / 14.0;
    if done {
        fill_rounded(pm, x, y, size, size, 4.0 * k, amber);
        stroke_line(pm, x + 3.5 * k, y + 7.5 * k, x + 6.0 * k, y + 10.0 * k, 1.8 * k, [0, 0, 0, a]);
        stroke_line(pm, x + 6.0 * k, y + 10.0 * k, x + 10.5 * k, y + 4.5 * k, 1.8 * k, [0, 0, 0, a]);
    } else {
        fill_rounded(pm, x, y, size, size, 4.0 * k, [110, 110, 116, a]);
        fill_rounded(pm, x + 1.5 * k, y + 1.5 * k, size - 3.0 * k, size - 3.0 * k, 3.0 * k, [0, 0, 0, 255]);
    }
}

/// A toggle switch, 32 by 18 logical pixels with its top-left corner at `x`, `y`.
/// `on` goes from 0 (off) to 1 (on) and may swing a little past 1: the knob slides, and the green
/// fades in as it goes.
pub(crate) fn draw_switch(pm: &mut Pixmap, x: f32, y: f32, s: f32, on: f32, a: u8) {
    let (w, h) = (32.0 * s, 18.0 * s);
    let alpha = a as f32 / 255.0;
    let t = on.clamp(0.0, 1.0);
    if t < 0.99 {
        fill_rounded(pm, x, y, w, h, h / 2.0, [255, 255, 255, (alpha * 34.0 * (1.0 - t)) as u8]);
        gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [255, 255, 255, (alpha * 30.0 * (1.0 - t)) as u8]);
    }
    if t > 0.01 {
        gfx::glow(pm, x + w / 2.0, y + h / 2.0, 24.0 * s, [70, 210, 120, (alpha * 46.0 * t) as u8], None);
        let fade = |c: u8| (c as f32 * t) as u8;
        gfx::gradient_rounded(pm, x, y, w, h, h / 2.0, [104, 228, 148, fade(a)], [50, 188, 102, fade(a)], false);
    }
    let knob_x = x + h / 2.0 + (w - h) * on;
    fill_circle(pm, knob_x, y + h / 2.0 + 0.8 * s, 7.0 * s, [0, 0, 0, (alpha * 60.0) as u8]);
    // White in both looks (the light look turns white into dark, so ask for black there).
    let knob = if theme::is_light() { [0, 0, 0, a] } else { [255, 255, 255, a] };
    fill_circle(pm, knob_x, y + h / 2.0, 7.0 * s, knob);
}

/// A rounded card: a soft gradient with a hairline border, outlined in the accent colour when `hot`.
pub(crate) fn draw_card(pm: &mut Pixmap, r: (f32, f32, f32, f32), radius: f32, s: f32, alpha: f32, hot: f32, accent: [u8; 3]) {
    let (x, y, w, h) = (r.0 * s, r.1 * s, r.2 * s, r.3 * s);
    let (top, bottom) = (pick(30.0, 20.0, hot), pick(16.0, 9.0, hot));
    gfx::gradient_rounded(pm, x, y, w, h, radius * s, [255, 255, 255, (alpha * top) as u8], [255, 255, 255, (alpha * bottom) as u8], false);
    let edge = pick([accent[0], accent[1], accent[2], (alpha * 110.0) as u8], [255, 255, 255, (alpha * 16.0) as u8], hot);
    gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, radius * s, s, edge);
}

/// Stroke a clockwise arc starting at 12 o'clock covering `frac` of the circle.
pub(crate) fn stroke_arc(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, width: f32, frac: f32, c: [u8; 4]) {
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
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        {
            let c = theme::map(c);
            paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        }
        paint.anti_alias = true;
        let stroke = Stroke { width, line_cap: if frac >= 0.999 { LineCap::Butt } else { LineCap::Round }, ..Default::default() };
        pm.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

/// Stroke an arc from `start` radians, sweeping clockwise (screen coordinates, 0 = 3 o'clock).
pub(crate) fn stroke_arc_from(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, width: f32, start: f32, sweep: f32, c: [u8; 4]) {
    let steps = ((sweep.abs() * r / 2.0).ceil() as usize).max(6);
    let mut pb = PathBuilder::new();
    for i in 0..=steps {
        let ang = start + sweep * i as f32 / steps as f32;
        let (x, y) = (cx + r * ang.cos(), cy + r * ang.sin());
        if i == 0 {
            pb.move_to(x, y);
        } else {
            pb.line_to(x, y);
        }
    }
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        {
            let c = theme::map(c);
            paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        }
        paint.anti_alias = true;
        let stroke = Stroke { width, line_cap: LineCap::Round, ..Default::default() };
        pm.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

pub(crate) fn stroke_line(pm: &mut Pixmap, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, c: [u8; 4]) {
    let mut pb = PathBuilder::new();
    pb.move_to(x1, y1);
    pb.line_to(x2, y2);
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        {
            let c = theme::map(c);
            paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        }
        paint.anti_alias = true;
        let stroke = Stroke { width, line_cap: LineCap::Round, ..Default::default() };
        pm.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

/// Horizontal level bar centered on `cy`.
pub(crate) fn draw_bar(pm: &mut Pixmap, x: f32, cy: f32, w: f32, frac: f32, fill: [u8; 4], s: f32, a: u8) {
    let h = 6.0 * s;
    let alpha = a as f32 / 255.0;
    fill_rounded(pm, x, cy - h / 2.0, w, h, h / 2.0, [255, 255, 255, (alpha * 30.0) as u8]);
    if frac > 0.0 {
        let fill_w = (w * frac.clamp(0.0, 1.0)).max(h);
        gfx::glow(pm, x + fill_w, cy, 14.0 * s, [fill[0], fill[1], fill[2], (alpha * 70.0) as u8], None);
        gfx::gradient_rounded(pm, x, cy - h / 2.0, fill_w, h, h / 2.0, gfx::mix(fill, [255, 255, 255, a], 0.1), gfx::mix(fill, [255, 255, 255, a], 0.55), true);
    }
}

/// Stroke a polyline with rounded ends and corners; `closed` joins the last point to the first.
pub(crate) fn stroke_poly(pm: &mut Pixmap, pts: &[(f32, f32)], width: f32, c: [u8; 4], closed: bool) {
    let mut pb = PathBuilder::new();
    pb.move_to(pts[0].0, pts[0].1);
    for p in &pts[1..] {
        pb.line_to(p.0, p.1);
    }
    if closed {
        pb.close();
    }
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        {
            let c = theme::map(c);
            paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        }
        paint.anti_alias = true;
        let stroke = Stroke { width, line_cap: LineCap::Round, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
        pm.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

/// Fill a path and/or draw its outline, with the theme's colours.
pub(crate) fn paint_path(pm: &mut Pixmap, path: &tiny_skia::Path, fill: Option<[u8; 4]>, outline: Option<(f32, [u8; 4])>) {
    if let Some(c) = fill {
        let c = theme::map(c);
        let mut paint = Paint::default();
        paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        paint.anti_alias = true;
        pm.fill_path(path, &paint, FillRule::Winding, Transform::identity(), None);
    }
    if let Some((width, c)) = outline {
        let c = theme::map(c);
        let mut paint = Paint::default();
        paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        paint.anti_alias = true;
        let stroke = Stroke { width, line_cap: LineCap::Round, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
        pm.stroke_path(path, &paint, &stroke, Transform::identity(), None);
    }
}

/// A closed shape through the points, filled softly and outlined.
pub(crate) fn shape(pm: &mut Pixmap, pts: &[(f32, f32)], soft: [u8; 4], width: f32, c: [u8; 4]) {
    let mut pb = PathBuilder::new();
    pb.move_to(pts[0].0, pts[0].1);
    for p in &pts[1..] {
        pb.line_to(p.0, p.1);
    }
    pb.close();
    if let Some(path) = pb.finish() {
        paint_path(pm, &path, Some(soft), Some((width, c)));
    }
}

/// A rounded box, filled softly and outlined.
pub(crate) fn soft_box(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, soft: [u8; 4], width: f32, c: [u8; 4]) {
    fill_rounded(pm, x, y, w, h, r, soft);
    gfx::stroke_rounded(pm, x + width / 2.0, y + width / 2.0, w - width, h - width, r, width, c);
}

pub(crate) fn fill_circle(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, c: [u8; 4]) {
    if let Some(path) = PathBuilder::from_circle(cx, cy, r) {
        let mut paint = Paint::default();
        {
            let c = theme::map(c);
            paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        }
        paint.anti_alias = true;
        pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }
}

pub(crate) fn fill_rounded(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, c: [u8; 4]) {
    let r = r.min(w / 2.0).min(h / 2.0);
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
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        {
            let c = theme::map(c);
            paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        }
        paint.anti_alias = true;
        pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }
}

pub(crate) fn fill_rect(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, c: [u8; 4]) {
    let Some(rect) = Rect::from_xywh(x, y, w.max(0.01), h) else { return };
    let mut paint = Paint::default();
    {
            let c = theme::map(c);
            paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        }
    pm.fill_rect(rect, &paint, Transform::identity(), None);
}

pub(crate) fn fill_path(pm: &mut Pixmap, pts: &[(f32, f32)], c: [u8; 4]) {
    let mut pb = PathBuilder::new();
    pb.move_to(pts[0].0, pts[0].1);
    for p in &pts[1..] {
        pb.line_to(p.0, p.1);
    }
    pb.close();
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        {
            let c = theme::map(c);
            paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        }
        paint.anti_alias = true;
        pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }
}

/// Things that can be blended between two states with a 0 to 1 amount.
pub(crate) trait Blend: Copy {
    fn blend(off: Self, on: Self, t: f32) -> Self;
}

impl Blend for f32 {
    fn blend(off: f32, on: f32, t: f32) -> f32 {
        off + (on - off) * t
    }
}

impl Blend for [u8; 4] {
    fn blend(off: [u8; 4], on: [u8; 4], t: f32) -> [u8; 4] {
        gfx::mix(off, on, t.clamp(0.0, 1.0))
    }
}

/// `on` when `t` is 1, `off` when it is 0, and a blend in between: how hover glows fade.
pub(crate) fn pick<T: Blend>(on: T, off: T, t: f32) -> T {
    T::blend(off, on, t)
}

pub(crate) fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}
