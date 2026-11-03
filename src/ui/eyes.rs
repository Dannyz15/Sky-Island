//! The eyes on the closed notch: how each Eye style is drawn, and how an eye moves (blinking, looking about).
//!
//! An eye is drawn from a `Pose`: how open it is, where it looks, and how far a blink has got. The notch keeps the
//! pose moving with springs and a clock (`companion.rs`); the settings window draws the tiles of its picker with the
//! resting pose. Every style is built from gradients, soft glows and highlights, and each one has a blink of its own:
//! a lid that falls, a heart that beats, a sparkle that turns.

use std::f32::consts::PI;
use std::time::Duration;

use tiny_skia::{
    Color, FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Mask, Paint, Path, PathBuilder, Pixmap, Point, RadialGradient, Rect,
    Shader, SpreadMode, Stroke, Transform,
};

use crate::gfx;
use crate::settings::EyeStyle;
use crate::theme;

/// What an eye is doing at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Pose {
    /// 1 is wide open, 0 is shut (it can swing a little past 1 as the eye opens again).
    pub open: f32,
    /// Where it looks: -1 (left) to 1 (right) across, and -1 (up) to 1 (down).
    pub look: (f32, f32),
    /// How far a blink has got, 0 to 1; 0 when it is not blinking.
    pub blink: f32,
}

impl Pose {
    /// Eyes wide open, looking straight ahead: the picker in the settings window.
    pub(crate) const REST: Pose = Pose { open: 1.0, look: (0.0, 0.0), blink: 0.0 };

    /// A flash of extra light just after a blink: 0 before and after, 1 in the middle of the opening.
    fn flare(self) -> f32 {
        if self.blink > 0.42 { (PI * (self.blink - 0.42) / 0.58).sin().max(0.0) } else { 0.0 }
    }
}

/// How long a blink lasts. A sleepy eye is slow; a heart beats twice; a robot is quick.
pub(crate) fn blink_len(style: EyeStyle) -> Duration {
    Duration::from_millis(match style {
        EyeStyle::Classic => 300,
        EyeStyle::Dots => 280,
        EyeStyle::Cat => 360,
        EyeStyle::Glow => 380,
        EyeStyle::Sleepy => 640,
        EyeStyle::Happy => 420,
        EyeStyle::Heart => 720,
        EyeStyle::Sparkle => 600,
        EyeStyle::Robot => 300,
        EyeStyle::Anime => 340,
    })
}

fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}

/// Eases out past the goal a little and settles back.
fn ease_out_back(t: f32) -> f32 {
    let (c1, t) = (1.1, t.clamp(0.0, 1.0));
    1.0 + (c1 + 1.0) * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

/// How open the eye is `p` (0 to 1) of the way through a blink: it shuts quickly, rests a moment and opens a
/// little slower, with a small overshoot. Hearts and sparkles do not shut; they have a move of their own.
pub(crate) fn lid(style: EyeStyle, p: f32) -> f32 {
    if matches!(style, EyeStyle::Heart | EyeStyle::Sparkle) {
        return 1.0;
    }
    let (shut, rest) = (0.34, 0.42);
    if p < shut {
        1.0 - ease_in_out(p / shut)
    } else if p < rest {
        0.0
    } else {
        ease_out_back((p - rest) / (1.0 - rest))
    }
}

/// A bump that is 1 at `at` and falls away over `width`.
fn bump(t: f32, at: f32, width: f32) -> f32 {
    (-((t - at) / width).powi(2)).exp()
}

// ---------------------------------------------------------------------------------------------------------------------
// Small drawing helpers. Colours go through the theme, so the eyes turn dark on the light look like everything else.
// ---------------------------------------------------------------------------------------------------------------------

fn tint(c: [u8; 3], alpha: f32) -> [u8; 4] {
    theme::map([c[0], c[1], c[2], (alpha.clamp(0.0, 1.0) * 255.0).round() as u8])
}

fn stop(at: f32, c: [u8; 3], alpha: f32) -> GradientStop {
    let t = tint(c, alpha);
    GradientStop::new(at, Color::from_rgba8(t[0], t[1], t[2], t[3]))
}

fn radial(centre: (f32, f32), radius: f32, stops: Vec<GradientStop>) -> Option<Shader<'static>> {
    let c = Point::from_xy(centre.0, centre.1);
    RadialGradient::new(c, 0.0, c, radius.max(0.01), stops, SpreadMode::Pad, Transform::identity())
}

fn linear(from: (f32, f32), to: (f32, f32), stops: Vec<GradientStop>) -> Option<Shader<'static>> {
    LinearGradient::new(Point::from_xy(from.0, from.1), Point::from_xy(to.0, to.1), stops, SpreadMode::Pad, Transform::identity())
}

fn oval(cx: f32, cy: f32, rx: f32, ry: f32) -> Option<Path> {
    PathBuilder::from_oval(Rect::from_xywh(cx - rx, cy - ry, rx * 2.0, ry * 2.0)?)
}

/// A scale of `sx` across and `sy` down about the point (`cx`, `cy`).
fn scale_about(cx: f32, cy: f32, sx: f32, sy: f32) -> Transform {
    Transform::from_row(sx, 0.0, 0.0, sy, cx * (1.0 - sx), cy * (1.0 - sy))
}

fn paint_with(shader: Shader<'static>) -> Paint<'static> {
    Paint { shader, anti_alias: true, ..Default::default() }
}

fn fill_shader(pm: &mut Pixmap, path: Option<Path>, shader: Option<Shader<'static>>, tf: Transform, mask: Option<&Mask>) {
    if let (Some(path), Some(shader)) = (path, shader) {
        pm.fill_path(&path, &paint_with(shader), FillRule::Winding, tf, mask);
    }
}

fn fill_solid(pm: &mut Pixmap, path: Option<Path>, c: [u8; 3], alpha: f32, tf: Transform, mask: Option<&Mask>) {
    let Some(path) = path else { return };
    let t = tint(c, alpha);
    let mut paint = Paint::default();
    paint.set_color_rgba8(t[0], t[1], t[2], t[3]);
    paint.anti_alias = true;
    pm.fill_path(&path, &paint, FillRule::Winding, tf, mask);
}

fn stroke_path(pm: &mut Pixmap, path: &Path, width: f32, c: [u8; 3], alpha: f32, tf: Transform) {
    let t = tint(c, alpha);
    let mut paint = Paint::default();
    paint.set_color_rgba8(t[0], t[1], t[2], t[3]);
    paint.anti_alias = true;
    let stroke = Stroke { width, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
    pm.stroke_path(path, &paint, &stroke, tf, None);
}

/// The part of the picture inside `path` (moved by `tf`), to keep the iris and the highlights inside the eye.
fn mask_of(pm: &Pixmap, path: &Path, tf: Transform) -> Option<Mask> {
    let shape = path.clone().transform(tf)?;
    let mut mask = Mask::new(pm.width(), pm.height())?;
    mask.fill_path(&shape, FillRule::Winding, true, Transform::identity());
    Some(mask)
}

/// A soft glow round the eye.
fn bloom(pm: &mut Pixmap, cx: f32, cy: f32, radius: f32, c: [u8; 3], alpha: f32) {
    if alpha > 0.004 {
        gfx::glow(pm, cx, cy, radius, [c[0], c[1], c[2], (alpha.clamp(0.0, 1.0) * 255.0) as u8], None);
    }
}

/// Everything a style needs to draw one eye.
struct Eye {
    x: f32,
    cy: f32,
    /// Pixels to one logical pixel.
    s: f32,
    /// 0 to 1: how visible the eyes are (they fade in and out with the notch).
    a: f32,
    pose: Pose,
}

impl Eye {
    /// How open it is, never quite flat so a shut eye still shows as a line.
    fn open(&self) -> f32 {
        self.pose.open.max(0.07)
    }

    /// The squash of a blink: down in height, a little out in width.
    fn blink_squash(&self, about_y: f32) -> Transform {
        let open = self.open();
        scale_about(self.x, about_y, 1.0 + (1.0 - open.min(1.0)) * 0.07, open)
    }
}

/// One eye in the given style, centred at (`x`, `cy`), `s` pixels to a logical pixel, `alpha` 0 to 255.
pub(crate) fn draw_eye(pm: &mut Pixmap, style: EyeStyle, x: f32, cy: f32, s: f32, alpha: u8, pose: Pose) {
    let e = Eye { x, cy, s, a: alpha as f32 / 255.0, pose };
    match style {
        EyeStyle::Classic => classic(pm, &e),
        EyeStyle::Dots => dots(pm, &e),
        EyeStyle::Cat => cat(pm, &e),
        EyeStyle::Glow => glow(pm, &e),
        EyeStyle::Sleepy => sleepy(pm, &e),
        EyeStyle::Happy => happy(pm, &e),
        EyeStyle::Heart => heart(pm, &e),
        EyeStyle::Sparkle => sparkle(pm, &e),
        EyeStyle::Robot => robot(pm, &e),
        EyeStyle::Anime => anime(pm, &e),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The styles
// ---------------------------------------------------------------------------------------------------------------------

/// A white eyeball with a blue iris, a dark pupil and two glints, shaded top to bottom.
fn classic(pm: &mut Pixmap, e: &Eye) {
    let (s, a, p) = (e.s, e.a, e.pose);
    let tf = e.blink_squash(e.cy);
    bloom(pm, e.x, e.cy, 12.0 * s, [255, 255, 255], 0.16 * a * e.open().min(1.0));
    let (rx, ry) = (5.0 * s, 6.6 * s);
    let ball = oval(e.x, e.cy, rx, ry);
    fill_shader(
        pm,
        ball.clone(),
        linear((e.x, e.cy - ry), (e.x, e.cy + ry), vec![stop(0.0, [255, 255, 255], a), stop(0.6, [236, 241, 252], a), stop(1.0, [196, 208, 232], a)]),
        tf,
        None,
    );
    let Some(ball) = ball else { return };
    let mask = mask_of(pm, &ball, tf);
    let (lx, ly) = (e.x + p.look.0 * 1.7 * s, e.cy + 0.5 * s + p.look.1 * 1.5 * s);
    fill_shader(
        pm,
        oval(lx, ly, 3.9 * s, 3.9 * s),
        radial((lx, ly), 3.9 * s, vec![stop(0.0, [86, 128, 214], a), stop(0.6, [42, 62, 138], a), stop(1.0, [16, 22, 58], a)]),
        tf,
        mask.as_ref(),
    );
    fill_solid(pm, oval(lx, ly, 1.9 * s, 1.9 * s), [5, 7, 16], a, tf, mask.as_ref());
    fill_solid(pm, oval(lx - 1.2 * s, ly - 1.4 * s, 1.05 * s, 1.05 * s), [255, 255, 255], a, tf, mask.as_ref());
    fill_solid(pm, oval(lx + 1.3 * s, ly + 1.3 * s, 0.5 * s, 0.5 * s), [255, 255, 255], 0.6 * a, tf, mask.as_ref());
    stroke_path(pm, &ball, 0.6 * s, [150, 166, 204], 0.4 * a, tf);
}

/// A soft pearl with a glint, and a halo.
fn dots(pm: &mut Pixmap, e: &Eye) {
    let (s, a, p) = (e.s, e.a, e.pose);
    let (cx, cy) = (e.x + p.look.0 * 1.1 * s, e.cy + p.look.1 * 0.7 * s);
    let tf = scale_about(cx, cy, 1.0 + (1.0 - e.open().min(1.0)) * 0.12, e.open());
    bloom(pm, cx, cy, 12.0 * s, [255, 255, 255], 0.22 * a * e.open().min(1.0));
    let r = 4.5 * s;
    fill_shader(
        pm,
        oval(cx, cy, r, r),
        radial((cx - 1.2 * s, cy - 1.4 * s), r * 1.5, vec![stop(0.0, [255, 255, 255], a), stop(0.55, [230, 238, 253], a), stop(1.0, [160, 182, 226], a)]),
        tf,
        None,
    );
    fill_solid(pm, oval(cx - 1.4 * s, cy - 1.6 * s, 1.0 * s, 0.8 * s), [255, 255, 255], 0.85 * a, tf, None);
}

/// An amber almond eye with a slit pupil that opens wide for a moment after a blink.
fn cat(pm: &mut Pixmap, e: &Eye) {
    let (s, a, p) = (e.s, e.a, e.pose);
    let tf = e.blink_squash(e.cy);
    bloom(pm, e.x, e.cy, 13.0 * s, [255, 190, 70], 0.2 * a * e.open().min(1.0));
    let (rx, ry) = (5.4 * s, 6.8 * s);
    let almond = oval(e.x, e.cy, rx, ry);
    fill_shader(
        pm,
        almond.clone(),
        radial((e.x, e.cy - 1.0 * s), 7.4 * s, vec![stop(0.0, [255, 238, 150], a), stop(0.5, [255, 192, 72], a), stop(1.0, [196, 104, 18], a)]),
        tf,
        None,
    );
    let Some(almond) = almond else { return };
    let mask = mask_of(pm, &almond, tf);
    let px = e.x + p.look.0 * 1.4 * s;
    let wide = (1.0 + 1.1 * p.flare()) * s;
    fill_solid(pm, oval(px, e.cy + p.look.1 * 0.8 * s, wide, 5.6 * s), [26, 10, 0], a, tf, mask.as_ref());
    fill_solid(pm, oval(e.x - 2.3 * s, e.cy - 3.4 * s, 1.0 * s, 0.75 * s), [255, 255, 255], 0.85 * a, tf, mask.as_ref());
    stroke_path(pm, &almond, 0.7 * s, [150, 72, 8], 0.55 * a, tf);
}

/// A glowing blue pill, bright in the middle, that flares after a blink.
fn glow(pm: &mut Pixmap, e: &Eye) {
    let (s, a, p) = (e.s, e.a, e.pose);
    let (cx, cy) = (e.x + p.look.0 * 1.0 * s, e.cy + p.look.1 * 0.6 * s);
    let flare = p.flare();
    let open = e.open();
    bloom(pm, cx, cy, 18.0 * s, [70, 140, 255], (0.4 + 0.35 * flare) * a * open.min(1.0));
    bloom(pm, cx, cy, 9.5 * s, [165, 210, 255], (0.5 + 0.3 * flare) * a * open.min(1.0));
    let tf = scale_about(cx, cy, 1.0 + (1.0 - open.min(1.0)) * 0.25, open);
    let (rx, ry) = (3.6 * s, 7.0 * s);
    fill_shader(
        pm,
        oval(cx, cy, rx, ry),
        linear((cx, cy - ry), (cx, cy + ry), vec![stop(0.0, [212, 236, 255], a), stop(0.45, [120, 184, 255], a), stop(1.0, [66, 116, 248], a)]),
        tf,
        None,
    );
    fill_solid(pm, oval(cx - 0.7 * s, cy - 2.6 * s, 1.0 * s, 2.6 * s), [255, 255, 255], (0.55 + 0.35 * flare) * a, tf, None);
}

/// A half-moon eye under a heavy lid, with a small pupil. The lid comes down when it blinks.
fn sleepy(pm: &mut Pixmap, e: &Eye) {
    let (s, a, p) = (e.s, e.a, e.pose);
    let (rx, depth, top) = (5.2 * s, 8.0 * s, e.cy - 2.6 * s);
    // The bottom stays where it is and the lid (the flat top) comes down to meet it.
    let tf = scale_about(e.x, top + depth, 1.0 + (1.0 - e.open().min(1.0)) * 0.08, e.open());
    bloom(pm, e.x, e.cy, 12.0 * s, [235, 232, 255], 0.14 * a * e.open().min(1.0));
    let mut pb = PathBuilder::new();
    let ctrl = top + depth / 0.75;
    pb.move_to(e.x - rx, top);
    pb.line_to(e.x + rx, top);
    pb.cubic_to(e.x + rx, ctrl, e.x - rx, ctrl, e.x - rx, top);
    pb.close();
    let moon = pb.finish();
    fill_shader(
        pm,
        moon.clone(),
        linear((e.x, top), (e.x, top + depth), vec![stop(0.0, [255, 255, 255], a), stop(1.0, [200, 206, 238], a)]),
        tf,
        None,
    );
    let Some(moon) = moon else { return };
    let mask = mask_of(pm, &moon, tf);
    let (lx, ly) = (e.x + p.look.0 * 1.4 * s, top + 4.1 * s + p.look.1 * 0.8 * s);
    fill_shader(
        pm,
        oval(lx, ly, 2.7 * s, 2.7 * s),
        radial((lx, ly), 2.7 * s, vec![stop(0.0, [70, 78, 150], a), stop(1.0, [18, 20, 52], a)]),
        tf,
        mask.as_ref(),
    );
    fill_solid(pm, oval(lx - 0.8 * s, ly - 0.9 * s, 0.8 * s, 0.8 * s), [255, 255, 255], a, tf, mask.as_ref());
    // The lid line.
    let mut line = PathBuilder::new();
    line.move_to(e.x - rx, top);
    line.line_to(e.x + rx, top);
    if let Some(line) = line.finish() {
        stroke_path(pm, &line, 1.0 * s, [170, 176, 220], 0.7 * a, tf);
    }
}

/// An arc of points along a circle, for the smiling eyes.
fn arc_path(cx: f32, cy: f32, r: f32, from: f32, sweep: f32) -> Option<Path> {
    let steps = 28;
    let mut pb = PathBuilder::new();
    for i in 0..=steps {
        let ang = from + sweep * i as f32 / steps as f32;
        let (px, py) = (cx + r * ang.cos(), cy + r * ang.sin());
        if i == 0 {
            pb.move_to(px, py);
        } else {
            pb.line_to(px, py);
        }
    }
    pb.finish()
}

/// A smiling arc, warm white, with a blush under it. It hops a little when it blinks.
fn happy(pm: &mut Pixmap, e: &Eye) {
    let (s, a, p) = (e.s, e.a, e.pose);
    let hop = -1.4 * s * (PI * p.blink).sin();
    let (cx, base) = (e.x + p.look.0 * 0.9 * s, e.cy + 3.0 * s + hop);
    let open = e.open();
    // Flattens to a line and comes back, about the ends of the arc.
    let tf = scale_about(cx, base - 1.2 * s, 1.0, 0.25 + 0.75 * open.min(1.0));
    bloom(pm, cx, e.cy, 11.0 * s, [255, 230, 190], 0.16 * a);
    // The blush.
    fill_shader(
        pm,
        oval(cx, e.cy + 7.4 * s, 3.8 * s, 1.9 * s),
        radial((cx, e.cy + 7.4 * s), 3.8 * s, vec![stop(0.0, [255, 118, 150], 0.4 * a), stop(1.0, [255, 118, 150], 0.0)]),
        Transform::identity(),
        None,
    );
    if let Some(arc) = arc_path(cx, base, 5.4 * s, -PI + 0.38, PI - 0.76)
        && let Some(shader) = linear((cx, base - 5.4 * s), (cx, base), vec![stop(0.0, [255, 250, 232], a), stop(1.0, [255, 214, 160], a)])
    {
        let stroke = Stroke { width: 3.0 * s, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
        pm.stroke_path(&arc, &paint_with(shader), &stroke, tf, None);
    }
}

/// A rosy heart with a gloss. It beats twice when it blinks.
fn heart(pm: &mut Pixmap, e: &Eye) {
    let (s, a, p) = (e.s, e.a, e.pose);
    let beat = 0.2 * (bump(p.blink, 0.16, 0.09) + 0.75 * bump(p.blink, 0.42, 0.09));
    let (cx, cy) = (e.x + p.look.0 * 0.7 * s, e.cy + 0.4 * s);
    let tf = scale_about(cx, cy, 1.0 + beat, 1.0 + beat);
    bloom(pm, cx, cy, 15.0 * s, [255, 90, 140], (0.3 + 1.2 * beat) * a);
    let r = 6.2 * s;
    let mut pb = PathBuilder::new();
    pb.move_to(cx, cy + r * 1.0);
    pb.cubic_to(cx - r * 1.7, cy - r * 0.05, cx - r * 1.0, cy - r * 1.25, cx, cy - r * 0.4);
    pb.cubic_to(cx + r * 1.0, cy - r * 1.25, cx + r * 1.7, cy - r * 0.05, cx, cy + r * 1.0);
    pb.close();
    fill_shader(
        pm,
        pb.finish(),
        linear((cx - r, cy - r), (cx + r * 0.8, cy + r), vec![stop(0.0, [255, 168, 200], a), stop(0.5, [255, 98, 150], a), stop(1.0, [210, 28, 92], a)]),
        tf,
        None,
    );
    // The gloss, a small tilted glint on the upper left lobe.
    let tilt = Transform::from_rotate_at(-35.0, cx - 2.6 * s, cy - 2.1 * s);
    fill_solid(pm, oval(cx - 2.6 * s, cy - 2.1 * s, 1.7 * s, 0.9 * s), [255, 255, 255], 0.55 * a, tilt.post_concat(tf), None);
}

/// The path of a four-point star centred at (`cx`, `cy`) with points `r` away.
fn star_path(cx: f32, cy: f32, r: f32) -> Option<Path> {
    let k = 0.2 * r;
    let mut pb = PathBuilder::new();
    pb.move_to(cx, cy - r);
    pb.quad_to(cx + k, cy - k, cx + r, cy);
    pb.quad_to(cx + k, cy + k, cx, cy + r);
    pb.quad_to(cx - k, cy + k, cx - r, cy);
    pb.quad_to(cx - k, cy - k, cx, cy - r);
    pb.close();
    pb.finish()
}

/// A golden four-point star with a white heart and a little twin that twinkles. It turns when it blinks.
fn sparkle(pm: &mut Pixmap, e: &Eye) {
    let (s, a, p) = (e.s, e.a, e.pose);
    let wave = (PI * p.blink).sin();
    let (cx, cy) = (e.x + p.look.0 * 0.8 * s, e.cy + p.look.1 * 0.5 * s);
    let turn = Transform::from_rotate_at(45.0 * ease_in_out(p.blink), cx, cy);
    let tf = scale_about(cx, cy, 1.0 - 0.25 * wave, 1.0 - 0.25 * wave).post_concat(turn);
    bloom(pm, cx, cy, 15.0 * s, [255, 205, 80], (0.32 + 0.3 * wave) * a);
    let r = 8.2 * s;
    fill_shader(
        pm,
        star_path(cx, cy, r),
        radial((cx, cy), r, vec![stop(0.0, [255, 255, 252], a), stop(0.38, [255, 241, 176], a), stop(1.0, [255, 196, 66], a)]),
        tf,
        None,
    );
    // The twin.
    let (tx, ty) = (cx + 6.8 * s, cy - 6.0 * s);
    let twin = Transform::from_rotate_at(-30.0 * wave, tx, ty);
    fill_shader(
        pm,
        star_path(tx, ty, 2.8 * s),
        radial((tx, ty), 2.8 * s, vec![stop(0.0, [255, 255, 255], (0.5 + 0.5 * wave) * a), stop(1.0, [255, 214, 110], (0.45 + 0.5 * wave) * a)]),
        twin,
        None,
    );
}

/// A green LED with a bright lens that follows where the robot looks. It shuts like a shutter and flashes.
fn robot(pm: &mut Pixmap, e: &Eye) {
    let (s, a, p) = (e.s, e.a, e.pose);
    let flare = p.flare();
    let (cx, cy) = (e.x + p.look.0 * 0.5 * s, e.cy);
    let open = e.open();
    bloom(pm, cx, cy, 14.0 * s, [60, 235, 140], (0.3 + 0.35 * flare) * a * open.min(1.0));
    let tf = scale_about(cx, cy, 1.0 + (1.0 - open.min(1.0)) * 0.1, open);
    let half = 4.6 * s;
    let body = gfx::rounded_path(cx - half, cy - half, half * 2.0, half * 2.0, 2.3 * s);
    fill_shader(
        pm,
        body.clone(),
        linear((cx, cy - half), (cx, cy + half), vec![stop(0.0, [178, 255, 208], a), stop(1.0, [36, 204, 106], a)]),
        tf,
        None,
    );
    let Some(body) = body else { return };
    let mask = mask_of(pm, &body, tf);
    // Faint scan lines.
    for k in [-2.6, 0.2, 3.0] {
        fill_solid(pm, Rect::from_xywh(cx - half, cy + k * s, half * 2.0, 0.7 * s).and_then(|r| {
            let mut pb = PathBuilder::new();
            pb.push_rect(r);
            pb.finish()
        }), [8, 80, 40], 0.22 * a, tf, mask.as_ref());
    }
    // The lens.
    let (lx, ly) = (cx + p.look.0 * 1.5 * s, cy + p.look.1 * 1.0 * s);
    fill_shader(
        pm,
        oval(lx, ly, 2.0 * s, 2.0 * s),
        radial((lx, ly), 2.0 * s, vec![stop(0.0, [244, 255, 250], a), stop(1.0, [150, 255, 196], a)]),
        tf,
        mask.as_ref(),
    );
    fill_solid(pm, Some(body.clone()), [255, 255, 255], 0.5 * flare * a, tf, None);
    stroke_path(pm, &body, 0.8 * s, [8, 80, 40], 0.5 * a, tf);
}

/// A big white eye with a violet iris, two glints and a dark lash line along the top.
fn anime(pm: &mut Pixmap, e: &Eye) {
    let (s, a, p) = (e.s, e.a, e.pose);
    let tf = e.blink_squash(e.cy + 1.5 * s);
    bloom(pm, e.x, e.cy, 13.0 * s, [170, 140, 255], 0.16 * a * e.open().min(1.0));
    let (rx, ry) = (5.6 * s, 7.3 * s);
    // The lash line: a dark shape a little bigger than the eye and a little higher, so it shows along the top.
    fill_solid(pm, oval(e.x, e.cy - 0.7 * s, rx + 0.6 * s, ry + 0.5 * s), [16, 10, 38], a, tf, None);
    let ball = oval(e.x, e.cy, rx, ry);
    fill_shader(
        pm,
        ball.clone(),
        linear((e.x, e.cy - ry), (e.x, e.cy + ry), vec![stop(0.0, [255, 255, 255], a), stop(1.0, [214, 214, 240], a)]),
        tf,
        None,
    );
    let Some(ball) = ball else { return };
    let mask = mask_of(pm, &ball, tf);
    let (lx, ly) = (e.x + p.look.0 * 1.5 * s, e.cy + 0.9 * s + p.look.1 * 1.2 * s);
    fill_shader(
        pm,
        oval(lx, ly, 4.7 * s, 4.7 * s),
        radial((lx, ly), 4.7 * s, vec![stop(0.0, [212, 198, 255], a), stop(0.5, [142, 108, 252], a), stop(1.0, [70, 46, 168], a)]),
        tf,
        mask.as_ref(),
    );
    fill_solid(pm, oval(lx, ly, 2.2 * s, 2.2 * s), [22, 10, 52], a, tf, mask.as_ref());
    fill_solid(pm, oval(lx - 1.9 * s, ly - 2.1 * s, 1.6 * s, 1.6 * s), [255, 255, 255], a, tf, mask.as_ref());
    fill_solid(pm, oval(lx + 1.7 * s, ly + 2.0 * s, 0.8 * s, 0.8 * s), [255, 255, 255], 0.8 * a, tf, mask.as_ref());
}

// ---------------------------------------------------------------------------------------------------------------------
// The resting looks: asleep, and the greeting
// ---------------------------------------------------------------------------------------------------------------------

/// A closed eye drawn as an arc: `up` is the happy one of the greeting (a hill), otherwise the sleeping one (a cup).
/// It takes the colour of the Eye style and has a soft glow.
pub(crate) fn draw_resting(pm: &mut Pixmap, style: EyeStyle, x: f32, cy: f32, s: f32, alpha: u8, up: bool) {
    let a = alpha as f32 / 255.0;
    let c = crate::ui::eye_colour(style);
    bloom(pm, x, cy, 11.0 * s, c, 0.18 * a);
    let arc = if up { arc_path(x, cy + 3.0 * s, 5.0 * s, -PI + 0.35, PI - 0.7) } else { arc_path(x, cy - 2.0 * s, 5.0 * s, 0.3, PI - 0.6) };
    if let Some(arc) = arc {
        stroke_path(pm, &arc, 2.2 * s, c, a, Transform::identity());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blink_shuts_rests_and_opens_again() {
        for style in EyeStyle::ALL {
            assert!((lid(style, 0.0) - 1.0).abs() < 1e-4, "{style:?} starts open");
            assert!((lid(style, 1.0) - 1.0).abs() < 0.02, "{style:?} ends open");
        }
        assert!(lid(EyeStyle::Classic, 0.38) < 0.01, "shut in the middle");
        assert!(lid(EyeStyle::Classic, 0.17) < 1.0 && lid(EyeStyle::Classic, 0.17) > 0.0, "on its way");
        assert_eq!(lid(EyeStyle::Heart, 0.4), 1.0, "a heart does not shut");
        // It never swings far past open.
        assert!((0..=100).all(|i| lid(EyeStyle::Anime, i as f32 / 100.0) < 1.12));
        // And it only goes down on the way in.
        let down: Vec<f32> = (0..=34).map(|i| lid(EyeStyle::Dots, i as f32 / 100.0)).collect();
        assert!(down.windows(2).all(|w| w[1] <= w[0] + 1e-6), "closing never reopens half way");
    }

    #[test]
    fn every_style_has_a_blink_of_a_sensible_length() {
        for style in EyeStyle::ALL {
            let ms = blink_len(style).as_millis();
            assert!((200..=900).contains(&ms), "{style:?} blinks for {ms} ms");
        }
        assert!(blink_len(EyeStyle::Sleepy) > blink_len(EyeStyle::Robot), "a sleepy eye is the slower one");
    }

    #[test]
    fn the_flare_comes_after_the_eye_has_opened_and_is_gone_at_the_end() {
        let at = |blink| Pose { open: 1.0, look: (0.0, 0.0), blink }.flare();
        assert_eq!(at(0.0), 0.0);
        assert_eq!(at(0.3), 0.0);
        assert!(at(0.7) > 0.9);
        assert!(at(1.0) < 0.01);
    }

    #[test]
    fn every_style_draws_in_every_pose_without_leaving_the_picture() {
        // A blank picture the size of the closed notch: nothing should paint past its edges (the drawing would
        // panic on an index) and each style should leave some mark.
        for style in EyeStyle::ALL {
            for (open, look, blink) in [(1.0, (0.0, 0.0), 0.0), (0.0, (1.0, 1.0), 0.4), (1.1, (-1.0, -1.0), 0.8), (0.5, (0.3, -0.2), 0.2)] {
                let mut pm = Pixmap::new(120, 40).unwrap();
                draw_eye(&mut pm, style, 60.0, 20.0, 1.0, 255, Pose { open, look, blink });
                assert!(pm.pixels().iter().any(|p| p.alpha() > 0), "{style:?} drew nothing at open {open}");
            }
        }
    }

    #[test]
    fn a_style_is_drawn_in_its_own_colours() {
        // The hearts are pink (red above blue), the robot green, the sparkle golden.
        let mean = |style| {
            let mut pm = Pixmap::new(60, 40).unwrap();
            draw_eye(&mut pm, style, 30.0, 20.0, 1.0, 255, Pose::REST);
            let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
            for p in pm.pixels().iter().filter(|p| p.alpha() > 200) {
                let c = p.demultiply();
                (r, g, b, n) = (r + c.red() as u32, g + c.green() as u32, b + c.blue() as u32, n + 1);
            }
            (r / n.max(1), g / n.max(1), b / n.max(1))
        };
        let heart = mean(EyeStyle::Heart);
        assert!(heart.0 > heart.1 + 60 && heart.0 > heart.2, "the heart is pink {heart:?}");
        let robot = mean(EyeStyle::Robot);
        assert!(robot.1 > robot.0 + 40 && robot.1 > robot.2 + 40, "the robot is green {robot:?}");
        let spark = mean(EyeStyle::Sparkle);
        assert!(spark.0 > spark.2 + 40 && spark.1 > spark.2 + 30, "the sparkle is golden {spark:?}");
    }
}
