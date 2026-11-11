//! The tile with a white picture on it that stands for a page, and the little shapes the pictures are drawn with.

use super::*;

pub(super) fn fill_poly(pm: &mut Pixmap, points: &[(f32, f32)], c: [u8; 4]) {
    let mut pb = PathBuilder::new();
    for (i, (x, y)) in points.iter().enumerate() {
        if i == 0 {
            pb.move_to(*x, *y);
        } else {
            pb.line_to(*x, *y);
        }
    }
    pb.close();
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        paint.anti_alias = true;
        pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }
}

/// A line (or ring) of a given width.
pub(super) fn stroke_points(pm: &mut Pixmap, points: &[(f32, f32)], close: bool, width: f32, c: [u8; 4]) {
    let mut pb = PathBuilder::new();
    for (i, (x, y)) in points.iter().enumerate() {
        if i == 0 {
            pb.move_to(*x, *y);
        } else {
            pb.line_to(*x, *y);
        }
    }
    if close {
        pb.close();
    }
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        paint.anti_alias = true;
        let stroke = Stroke { width, line_cap: tiny_skia::LineCap::Round, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
        pm.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

/// Points along an arc of a circle, in degrees (0 is to the right, 90 is down).
pub(super) fn arc_points(cx: f32, cy: f32, r: f32, from: f32, to: f32, steps: usize) -> Vec<(f32, f32)> {
    (0..=steps)
        .map(|i| {
            let a = (from + (to - from) * i as f32 / steps as f32).to_radians();
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

/// Where a page's picture goes and what it is drawn with: the centre, one logical pixel of the 26 pixel tile, white, and the two ends of the tile's colour.
#[derive(Clone, Copy)]
struct Glyph {
    cx: f32,
    cy: f32,
    u: f32,
    w: [u8; 4],
    from: [u8; 4],
    to: [u8; 4],
}

/// The coloured tile with a white picture on it, for a page: `size` logical pixels square (26 in the sidebar, bigger
/// beside the page's title), with `x`, `y` its top left.
pub(super) fn draw_page_icon(pm: &mut Pixmap, page: Page, x: f32, y: f32, size: f32, s: f32) {
    let t = page.tint();
    let from = gfx::mix([t[0], t[1], t[2], 255], [255, 255, 255, 255], 0.22);
    let to = [t[0], t[1], t[2], 255];
    gfx::gradient_rounded(pm, x * s, y * s, size * s, size * s, size * 0.27 * s, from, to, false);
    let (cx, cy) = ((x + size / 2.0) * s, (y + size / 2.0) * s);
    let w = [255, 255, 255, 255];
    // One logical pixel of the 26 pixel tile the pictures are drawn for.
    let u = s * size / 26.0;
    let g = Glyph { cx, cy, u, w, from, to };
    match page {
        Page::General => icon_gear(pm, &g),
        Page::Notch => icon_screen(pm, &g),
        Page::ClosedNotch => icon_closed_notch(pm, &g),
        Page::Music => icon_music(pm, &g),
        Page::Appearance => icon_half_circle(pm, &g),
        Page::Tabs => icon_tiles(pm, &g),
        Page::Location => icon_pin(pm, &g),
        Page::Sounds => icon_speaker(pm, &g),
        Page::Calendar => icon_calendar(pm, &g),
        Page::Shelf => icon_tray(pm, &g),
        Page::Notices => icon_bell(pm, &g),
        Page::About => icon_info(pm, &g),
    }
}

/// A gear: a ring of teeth with a hole in the middle.
fn icon_gear(pm: &mut Pixmap, g: &Glyph) {
    let Glyph { cx, cy, u, w, from, to } = *g;
    // A gear: a ring of teeth with a hole in the middle.
    let mut pts = Vec::new();
    for i in 0..16 {
        let a = (i as f32 * 22.5 - 11.25).to_radians();
        let r = if (i / 2) % 2 == 0 { 8.2 } else { 6.2 } * u;
        pts.push((cx + r * a.cos(), cy + r * a.sin()));
        let a2 = (i as f32 * 22.5 + 11.25).to_radians();
        pts.push((cx + r * a2.cos(), cy + r * a2.sin()));
    }
    fill_poly(pm, &pts, w);
    crate::fill_circle(pm, cx, cy, 2.8 * u, gfx::mix(from, to, 0.5));
}

/// A screen with the notch at the top.
fn icon_screen(pm: &mut Pixmap, g: &Glyph) {
    let Glyph { cx, cy, u, w, .. } = *g;
    // A screen with the notch at the top.
    gfx::stroke_rounded(pm, cx - 8.0 * u, cy - 6.0 * u, 16.0 * u, 12.0 * u, 2.5 * u, 1.6 * u, w);
    crate::fill_rounded(pm, cx - 3.4 * u, cy - 6.4 * u, 6.8 * u, 3.4 * u, 1.7 * u, w);
}

/// The closed notch: a pill with two eyes in it.
fn icon_closed_notch(pm: &mut Pixmap, g: &Glyph) {
    let Glyph { cx, cy, u, w, from, to } = *g;
    // The closed notch: a pill with two eyes in it.
    crate::fill_rounded(pm, cx - 8.5 * u, cy - 3.6 * u, 17.0 * u, 7.2 * u, 3.6 * u, w);
    let eye = gfx::mix(from, to, 0.6);
    crate::fill_rounded(pm, cx - 3.6 * u, cy - 1.8 * u, 2.0 * u, 3.6 * u, 1.0 * u, eye);
    crate::fill_rounded(pm, cx + 1.6 * u, cy - 1.8 * u, 2.0 * u, 3.6 * u, 1.0 * u, eye);
}

/// Two notes under one beam, as on the notch's Music tab.
fn icon_music(pm: &mut Pixmap, g: &Glyph) {
    let Glyph { cx, cy, u, w, .. } = *g;
    // Two notes under one beam, as on the notch's Music tab.
    crate::draw_tab_icon(pm, View::Player, cx - 1.0 * u, cy - 0.6 * u, u, w);
}

/// A circle, half of it filled.
fn icon_half_circle(pm: &mut Pixmap, g: &Glyph) {
    let Glyph { cx, cy, u, w, .. } = *g;
    // A circle, half of it filled.
    stroke_points(pm, &arc_points(cx, cy, 6.6 * u, 0.0, 360.0, 36), true, 1.6 * u, w);
    let mut half = arc_points(cx, cy, 6.6 * u, -90.0, 90.0, 18);
    half.push((cx, cy + 6.6 * u));
    fill_poly(pm, &half, w);
}

/// Four tiles.
fn icon_tiles(pm: &mut Pixmap, g: &Glyph) {
    let Glyph { cx, cy, u, w, .. } = *g;
    // Four tiles.
    for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        crate::fill_rounded(pm, cx + dx * 3.5 * u - 3.0 * u, cy + dy * 3.5 * u - 3.0 * u, 6.0 * u, 6.0 * u, 1.6 * u, w);
    }
}

/// A map pin: a drop with a hole.
fn icon_pin(pm: &mut Pixmap, g: &Glyph) {
    let Glyph { cx, cy, u, w, from, to } = *g;
    // A map pin: a drop with a hole.
    let top = cy - 2.6 * u;
    let mut pts = arc_points(cx, top, 5.6 * u, 150.0, 390.0, 20);
    pts.push((cx, cy + 8.0 * u));
    fill_poly(pm, &pts, w);
    crate::fill_circle(pm, cx, top, 2.2 * u, gfx::mix(from, to, 0.5));
}

/// A speaker with two waves.
fn icon_speaker(pm: &mut Pixmap, g: &Glyph) {
    let Glyph { cx, cy, u, w, .. } = *g;
    // A speaker with two waves.
    fill_poly(
        pm,
        &[
            (cx - 7.5 * u, cy - 2.4 * u),
            (cx - 4.0 * u, cy - 2.4 * u),
            (cx + 0.6 * u, cy - 6.4 * u),
            (cx + 0.6 * u, cy + 6.4 * u),
            (cx - 4.0 * u, cy + 2.4 * u),
            (cx - 7.5 * u, cy + 2.4 * u),
        ],
        w,
    );
    stroke_points(pm, &arc_points(cx + 1.0 * u, cy, 4.2 * u, -48.0, 48.0, 8), false, 1.4 * u, w);
    stroke_points(pm, &arc_points(cx + 1.0 * u, cy, 7.4 * u, -48.0, 48.0, 8), false, 1.4 * u, w);
}

/// A page with a header strip and two rings, and a few dots for days.
fn icon_calendar(pm: &mut Pixmap, g: &Glyph) {
    let Glyph { cx, cy, u, w, .. } = *g;
    // A page with a header strip and two rings, and a few dots for days.
    gfx::stroke_rounded(pm, cx - 7.5 * u, cy - 6.0 * u, 15.0 * u, 13.5 * u, 2.6 * u, 1.6 * u, w);
    crate::fill_rect(pm, cx - 7.5 * u, cy - 3.0 * u, 15.0 * u, 1.6 * u, w);
    crate::fill_rounded(pm, cx - 3.8 * u, cy - 8.0 * u, 1.8 * u, 3.4 * u, 0.9 * u, w);
    crate::fill_rounded(pm, cx + 2.0 * u, cy - 8.0 * u, 1.8 * u, 3.4 * u, 0.9 * u, w);
    for (dx, dy) in [(-3.4, 1.0), (0.0, 1.0), (3.4, 1.0), (-3.4, 4.2), (0.0, 4.2)] {
        crate::fill_circle(pm, cx + dx * u, cy + dy * u, 0.9 * u, w);
    }
}

/// An open tray with an arrow going in.
fn icon_tray(pm: &mut Pixmap, g: &Glyph) {
    let Glyph { cx, cy, u, w, .. } = *g;
    // An open tray with an arrow going in.
    stroke_points(pm, &[(cx - 7.5 * u, cy + 0.5 * u), (cx - 7.5 * u, cy + 6.5 * u), (cx + 7.5 * u, cy + 6.5 * u), (cx + 7.5 * u, cy + 0.5 * u)], false, 1.7 * u, w);
    stroke_points(
        pm,
        &[
            (cx - 7.5 * u, cy + 0.5 * u),
            (cx - 3.2 * u, cy + 0.5 * u),
            (cx - 2.0 * u, cy + 3.0 * u),
            (cx + 2.0 * u, cy + 3.0 * u),
            (cx + 3.2 * u, cy + 0.5 * u),
            (cx + 7.5 * u, cy + 0.5 * u),
        ],
        false,
        1.7 * u,
        w,
    );
    stroke_points(pm, &[(cx, cy - 7.0 * u), (cx, cy - 0.5 * u)], false, 1.7 * u, w);
    fill_poly(pm, &[(cx - 3.2 * u, cy - 2.4 * u), (cx + 3.2 * u, cy - 2.4 * u), (cx, cy + 1.2 * u)], w);
}

/// A bell.
fn icon_bell(pm: &mut Pixmap, g: &Glyph) {
    let Glyph { cx, cy, u, w, .. } = *g;
    // A bell.
    let mut bell = arc_points(cx, cy - 1.0 * u, 5.4 * u, 180.0, 360.0, 14);
    bell.extend([(cx + 5.4 * u, cy + 3.8 * u), (cx + 7.4 * u, cy + 5.2 * u), (cx - 7.4 * u, cy + 5.2 * u), (cx - 5.4 * u, cy + 3.8 * u)]);
    fill_poly(pm, &bell, w);
    crate::fill_circle(pm, cx, cy + 7.2 * u, 1.5 * u, w);
    crate::fill_rounded(pm, cx - 0.9 * u, cy - 7.6 * u, 1.8 * u, 2.6 * u, 0.9 * u, w);
}

/// An "i" in a circle.
fn icon_info(pm: &mut Pixmap, g: &Glyph) {
    let Glyph { cx, cy, u, w, .. } = *g;
    // An "i" in a circle.
    stroke_points(pm, &arc_points(cx, cy, 7.4 * u, 0.0, 360.0, 36), true, 1.6 * u, w);
    crate::fill_circle(pm, cx, cy - 3.2 * u, 1.1 * u, w);
    crate::fill_rounded(pm, cx - 0.9 * u, cy - 0.8 * u, 1.8 * u, 5.4 * u, 0.9 * u, w);
}
