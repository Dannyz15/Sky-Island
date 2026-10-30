//! The icons, drawn as shapes.

use crate::*;

/// A round "minus" or "plus" button for the time steppers.
pub(crate) fn draw_step(pm: &mut Pixmap, r: editor::Rect, plus: bool, s: f32, alpha: f32, hot: f32) {
    let (cx, cy) = ((r.0 + r.2 / 2.0) * s, (r.1 + r.3 / 2.0) * s);
    fill_circle(pm, cx, cy, 8.5 * s, [255, 255, 255, (alpha * pick(40.0, 18.0, hot)) as u8]);
    let ink = [210, 210, 215, (alpha * 255.0) as u8];
    stroke_line(pm, cx - 3.0 * s, cy, cx + 3.0 * s, cy, 1.5 * s, ink);
    if plus {
        stroke_line(pm, cx, cy - 3.0 * s, cx, cy + 3.0 * s, 1.5 * s, ink);
    }
}

/// A small pencil, the edit button on a task.
pub(crate) fn draw_pencil(pm: &mut Pixmap, cx: f32, cy: f32, s: f32, c: [u8; 4]) {
    stroke_line(pm, cx - 2.2 * s, cy + 2.2 * s, cx + 3.0 * s, cy - 3.0 * s, 2.2 * s, c);
    fill_path(pm, &[(cx - 4.2 * s, cy + 4.2 * s), (cx - 4.2 * s, cy + 1.4 * s), (cx - 1.4 * s, cy + 4.2 * s)], c);
}

/// A speech bubble with two lines of text: the lyrics button.
pub(crate) fn draw_lyrics_icon(pm: &mut Pixmap, cx: f32, cy: f32, s: f32, c: [u8; 4]) {
    let p = |dx: f32, dy: f32| (cx + dx * s, cy + dy * s);
    stroke_poly(pm, &[p(-7.0, -6.0), p(7.0, -6.0), p(7.0, 3.5), p(1.5, 3.5), p(-2.5, 7.0), p(-2.5, 3.5), p(-7.0, 3.5)], 1.5 * s, c, true);
    stroke_line(pm, cx - 3.5 * s, cy - 2.5 * s, cx + 3.5 * s, cy - 2.5 * s, 1.3 * s, c);
    stroke_line(pm, cx - 3.5 * s, cy + 0.2 * s, cx + 1.5 * s, cy + 0.2 * s, 1.3 * s, c);
}

pub(crate) fn draw_speaker(pm: &mut Pixmap, cx: f32, cy: f32, s: f32, c: [u8; 4], muted: bool) {
    fill_rect(pm, cx - 9.0 * s, cy - 3.0 * s, 4.0 * s, 6.0 * s, c);
    fill_path(pm, &[(cx - 5.0 * s, cy - 3.0 * s), (cx + 1.0 * s, cy - 8.0 * s), (cx + 1.0 * s, cy + 8.0 * s), (cx - 5.0 * s, cy + 3.0 * s)], c);
    if muted {
        stroke_line(pm, cx + 4.0 * s, cy - 4.0 * s, cx + 10.0 * s, cy + 4.0 * s, 1.6 * s, c);
        stroke_line(pm, cx + 10.0 * s, cy - 4.0 * s, cx + 4.0 * s, cy + 4.0 * s, 1.6 * s, c);
    } else {
        for r in [5.0, 9.0] {
            stroke_arc_from(pm, cx + 1.0 * s, cy, r * s, 1.6 * s, -0.7, 1.4, c);
        }
    }
}

pub(crate) fn draw_sun(pm: &mut Pixmap, cx: f32, cy: f32, s: f32, c: [u8; 4]) {
    fill_circle(pm, cx, cy, 4.0 * s, c);
    for i in 0..8 {
        let ang = i as f32 * std::f32::consts::FRAC_PI_4;
        let (dx, dy) = (ang.cos(), ang.sin());
        stroke_line(pm, cx + dx * 7.0 * s, cy + dy * 7.0 * s, cx + dx * 10.0 * s, cy + dy * 10.0 * s, 1.6 * s, c);
    }
}

pub(crate) fn draw_battery(pm: &mut Pixmap, cx: f32, cy: f32, s: f32, frac: f32, c: [u8; 4], a: u8) {
    let (x, y, w, h) = (cx - 11.0 * s, cy - 6.0 * s, 20.0 * s, 12.0 * s);
    fill_rounded(pm, x, y, w, h, 3.0 * s, [150, 150, 155, a]);
    fill_rounded(pm, x + 1.5 * s, y + 1.5 * s, w - 3.0 * s, h - 3.0 * s, 2.0 * s, [0, 0, 0, 255]);
    fill_rounded(pm, x + 2.5 * s, y + 2.5 * s, ((w - 5.0 * s) * frac.clamp(0.0, 1.0)).max(1.0), h - 5.0 * s, 1.0 * s, c);
    fill_rect(pm, x + w, cy - 2.5 * s, 2.5 * s, 5.0 * s, [150, 150, 155, a]);
}

pub(crate) fn draw_wifi(pm: &mut Pixmap, cx: f32, cy: f32, s: f32, c: [u8; 4], slashed: bool) {
    let base_y = cy + 7.0 * s;
    fill_circle(pm, cx, base_y, 1.8 * s, c);
    for r in [5.0, 9.0, 13.0] {
        stroke_arc_from(pm, cx, base_y, r * s, 1.8 * s, -2.36, 1.57, c);
    }
    if slashed {
        stroke_line(pm, cx - 9.0 * s, cy - 9.0 * s, cx + 9.0 * s, cy + 9.0 * s, 1.8 * s, [255, 90, 80, c[3]]);
    }
}

pub(crate) fn draw_headphones(pm: &mut Pixmap, cx: f32, cy: f32, s: f32, c: [u8; 4]) {
    stroke_arc_from(pm, cx, cy + 1.0 * s, 8.0 * s, 2.0 * s, -std::f32::consts::PI, std::f32::consts::PI, c);
    fill_rounded(pm, cx - 11.0 * s, cy + 1.0 * s, 5.0 * s, 9.0 * s, 2.0 * s, c);
    fill_rounded(pm, cx + 6.0 * s, cy + 1.0 * s, 5.0 * s, 9.0 * s, 2.0 * s, c);
}

/// The small icon that stands in for a tab's name, about 14 px across. Each is drawn in two tones: the
/// body in a soft wash of the colour, and the details and the outline in the colour itself.
pub(crate) fn draw_tab_icon(pm: &mut Pixmap, view: View, cx: f32, cy: f32, s: f32, c: [u8; 4]) {
    let p = |dx: f32, dy: f32| (cx + dx * s, cy + dy * s);
    let w = 1.5 * s;
    let soft = [c[0], c[1], c[2], (c[3] as f32 * 0.3) as u8];
    match view {
        View::Player => {
            // Two notes under one beam.
            fill_circle(pm, cx - 4.3 * s, cy + 5.2 * s, 2.5 * s, c);
            fill_circle(pm, cx + 3.9 * s, cy + 3.4 * s, 2.5 * s, c);
            stroke_line(pm, cx - 2.1 * s, cy + 4.8 * s, cx - 2.1 * s, cy - 4.2 * s, w, c);
            stroke_line(pm, cx + 6.3 * s, cy + 3.0 * s, cx + 6.3 * s, cy - 6.0 * s, w, c);
            fill_path(pm, &[p(-2.4, -4.4), p(6.6, -6.4), p(6.6, -3.0), p(-2.4, -1.0)], c);
        }
        View::Timer => {
            // A stopwatch: round body, a button on top and one at the side, and a hand.
            fill_circle(pm, cx, cy + 1.4 * s, 6.4 * s, soft);
            stroke_arc(pm, cx, cy + 1.4 * s, 6.4 * s, w, 1.0, c);
            fill_rounded(pm, cx - 1.9 * s, cy - 8.2 * s, 3.8 * s, 2.0 * s, 0.9 * s, c);
            stroke_line(pm, cx + 4.6 * s, cy - 4.4 * s, cx + 6.0 * s, cy - 5.8 * s, w, c);
            stroke_line(pm, cx, cy + 1.4 * s, cx + 2.6 * s, cy - 1.4 * s, w, c);
            fill_circle(pm, cx, cy + 1.4 * s, 1.0 * s, c);
        }
        View::Water => {
            // A drop with a little shine.
            let mut pb = PathBuilder::new();
            pb.move_to(cx, cy - 7.4 * s);
            pb.cubic_to(cx + 1.2 * s, cy - 3.4 * s, cx + 5.8 * s, cy - 0.8 * s, cx + 5.8 * s, cy + 2.4 * s);
            pb.cubic_to(cx + 5.8 * s, cy + 5.8 * s, cx + 3.2 * s, cy + 7.4 * s, cx, cy + 7.4 * s);
            pb.cubic_to(cx - 3.2 * s, cy + 7.4 * s, cx - 5.8 * s, cy + 5.8 * s, cx - 5.8 * s, cy + 2.4 * s);
            pb.cubic_to(cx - 5.8 * s, cy - 0.8 * s, cx - 1.2 * s, cy - 3.4 * s, cx, cy - 7.4 * s);
            pb.close();
            if let Some(path) = pb.finish() {
                paint_path(pm, &path, Some(soft), Some((w, c)));
            }
            stroke_poly(pm, &[p(-3.0, 2.4), p(-2.8, 4.4), p(-1.4, 5.6)], 1.2 * s, c, false);
        }
        View::Shelf => {
            // An inbox tray with a slot.
            shape(pm, &[p(-7.0, 0.6), p(-4.8, -5.6), p(4.8, -5.6), p(7.0, 0.6), p(7.0, 5.0), p(-7.0, 5.0)], soft, w, c);
            stroke_poly(pm, &[p(-7.0, 0.6), p(-3.4, 0.6), p(-2.4, 2.6), p(2.4, 2.6), p(3.4, 0.6), p(7.0, 0.6)], w, c, false);
        }
        View::Todo => {
            // A check mark on a round badge.
            fill_circle(pm, cx, cy, 6.8 * s, soft);
            stroke_arc(pm, cx, cy, 6.8 * s, w, 1.0, c);
            stroke_poly(pm, &[p(-2.8, 0.2), p(-0.8, 2.4), p(3.0, -2.2)], 1.8 * s, c, false);
        }
        View::Clipboard => {
            // A board with a clip and two lines of text.
            soft_box(pm, cx - 6.4 * s, cy - 5.0 * s, 12.8 * s, 12.8 * s, 2.6 * s, soft, w, c);
            fill_rounded(pm, cx - 2.8 * s, cy - 7.6 * s, 5.6 * s, 3.2 * s, 1.5 * s, c);
            stroke_line(pm, cx - 2.6 * s, cy + 0.6 * s, cx + 2.6 * s, cy + 0.6 * s, 1.3 * s, c);
            stroke_line(pm, cx - 2.6 * s, cy + 3.8 * s, cx + 1.2 * s, cy + 3.8 * s, 1.3 * s, c);
        }
        View::Calendar => {
            // A page with two rings, a heading line and a few dates.
            soft_box(pm, cx - 6.6 * s, cy - 5.0 * s, 13.2 * s, 12.6 * s, 2.6 * s, soft, w, c);
            stroke_line(pm, cx - 6.6 * s, cy - 1.0 * s, cx + 6.6 * s, cy - 1.0 * s, 1.3 * s, c);
            stroke_line(pm, cx - 3.2 * s, cy - 7.4 * s, cx - 3.2 * s, cy - 3.6 * s, w, c);
            stroke_line(pm, cx + 3.2 * s, cy - 7.4 * s, cx + 3.2 * s, cy - 3.6 * s, w, c);
            for (dx, dy) in [(-3.2, 2.2), (0.0, 2.2), (3.2, 2.2), (-3.2, 5.0), (0.0, 5.0)] {
                fill_circle(pm, cx + dx * s, cy + dy * s, 0.9 * s, c);
            }
        }
        View::Lyrics => {}
    }
}

/// The play triangle and the pause bars. `pause` goes from 0 (play) to 1 (pause); in between, one icon
/// shrinks away while the other grows in, so pressing the button looks like a change instead of a swap.
pub(crate) fn draw_play_pause(pm: &mut Pixmap, cx: f32, cy: f32, s: f32, c: [u8; 4], pause: f32) {
    let pause = pause.clamp(0.0, 1.0);
    let tint = |t: f32| [c[0], c[1], c[2], (c[3] as f32 * t) as u8];
    if pause > 0.01 {
        let k = (0.55 + 0.45 * pause) * s;
        fill_rect(pm, cx - 7.0 * k, cy - 9.0 * k, 5.0 * k, 18.0 * k, tint(pause));
        fill_rect(pm, cx + 2.0 * k, cy - 9.0 * k, 5.0 * k, 18.0 * k, tint(pause));
    }
    if pause < 0.99 {
        let k = (0.55 + 0.45 * (1.0 - pause)) * s;
        fill_path(pm, &[(cx - 6.0 * k, cy - 10.0 * k), (cx + 9.0 * k, cy), (cx - 6.0 * k, cy + 10.0 * k)], tint(1.0 - pause));
    }
}

pub(crate) fn draw_next(pm: &mut Pixmap, cx: f32, cy: f32, s: f32, c: [u8; 4]) {
    fill_path(pm, &[(cx - 8.0 * s, cy - 8.0 * s), (cx + 4.0 * s, cy), (cx - 8.0 * s, cy + 8.0 * s)], c);
    fill_rect(pm, cx + 5.0 * s, cy - 8.0 * s, 3.0 * s, 16.0 * s, c);
}

pub(crate) fn draw_prev(pm: &mut Pixmap, cx: f32, cy: f32, s: f32, c: [u8; 4]) {
    fill_path(pm, &[(cx + 8.0 * s, cy - 8.0 * s), (cx - 4.0 * s, cy), (cx + 8.0 * s, cy + 8.0 * s)], c);
    fill_rect(pm, cx - 8.0 * s, cy - 8.0 * s, 3.0 * s, 16.0 * s, c);
}

/// The curved flares on both sides of the notch's top edge, where it meets the top of the screen: the
/// outer corners curve inwards, like a notch cut into the screen. `ear` is their width and height.
pub(crate) fn draw_ears(pm: &mut Pixmap, ear: f32) {
    if ear < 1.0 {
        return;
    }
    let width = pm.width() as f32;
    let body = theme::body()[0];
    let mut paint = Paint::default();
    paint.set_color_rgba8(body[0], body[1], body[2], 255);
    paint.anti_alias = true;
    for left in [true, false] {
        // Drawn for the left side, then mirrored for the right.
        let x = |v: f32| if left { v } else { width - v };
        let mut pb = PathBuilder::new();
        pb.move_to(x(0.0), 0.0);
        pb.line_to(x(ear), 0.0);
        pb.line_to(x(ear), ear);
        pb.quad_to(x(ear), 0.0, x(0.0), 0.0);
        pb.close();
        if let Some(path) = pb.finish() {
            pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }
}

/// The colour of the eyes (and of their sleepy and happy faces) in a style.
pub(crate) fn eye_colour(style: EyeStyle) -> [u8; 3] {
    match style {
        EyeStyle::Classic | EyeStyle::Dots => [255, 255, 255],
        EyeStyle::Cat => [255, 196, 80],
        EyeStyle::Glow => [110, 175, 255],
    }
}
