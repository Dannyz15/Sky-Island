//! The closed notch while a song plays. Everything sits in one row, centred: a spinning record with the album cover on one
//! side of whatever is in the middle (the clock, the eyes, the timer) and an equaliser on the other. The two sides are
//! exactly alike in size, and the notch is only as wide as it needs to be for them.
//!
//! The record turns slowly and a thin ring round it fills as the song goes on. The five bars rise and fall from the middle
//! line, each in its own shade of the cover's colour, over a soft glow of the same colour. A thin line of that colour along
//! the notch's edge breathes with the beat. The bars and the beat follow the real loudness of the sound the PC plays
//! (`loopback.rs`) unless that is switched off or cannot be listened to, and then they move to a made-up beat.

use std::sync::OnceLock;

use tiny_skia::{Color, FillRule, GradientStop, LinearGradient, Paint, PathBuilder, Point, Rect, SpreadMode, Stroke, Transform};

use crate::*;

/// How often the closed notch is drawn again while the bars run: 30 times a second keeps the motion smooth.
pub(crate) const BARS_TICK: Duration = Duration::from_millis(33);

/// The record: the cover as a round disc, the ring round it, and the room each side needs (a square slot, as wide as the
/// block of bars, so the two sides match).
const DISC: f32 = 19.0;
const RING_R: f32 = 11.3;
const RING_W: f32 = 1.4;
const SLOT: f32 = 24.0;

const EDGE: f32 = 9.0;
/// The space between the record or the bars and what is in the middle.
const GAP: f32 = 14.0;

const BARS: usize = 5;
const _: () = assert!(crate::loopback::BANDS == BARS, "a band of sound for each bar");
const BAR_W: f32 = 3.0;
const BAR_GAP: f32 = 2.25;
const BAR_MAX: f32 = 20.0;
const BAR_MIN: f32 = 3.0;

/// How wide the block of bars is: the same as the slot of the record.
const BARS_BLOCK: f32 = BARS as f32 * BAR_W + (BARS as f32 - 1.0) * BAR_GAP;
const _: () = assert!(BARS_BLOCK == SLOT, "the bars take as much room as the record");

// What has to fit, checked when the program is built: the ring and the bars inside the height of the notch, the disc
// inside the ring, the slot round the ring, and (with eyes in the middle) the record, the bars and the gaps inside the
// usual width, or only a little more.
const _: () = assert!((RING_R + RING_W / 2.0) * 2.0 <= COLLAPSED.1 as f32 - 2.0, "the ring fits with room above and below");
const _: () = assert!(DISC / 2.0 < RING_R - RING_W, "the disc is inside the ring");
const _: () = assert!(BAR_MAX <= COLLAPSED.1 as f32 - 6.0, "the bars fit the height");
const _: () = assert!(SLOT >= (RING_R + RING_W / 2.0) * 2.0 - 0.5, "the slot holds the ring");
const _: () = assert!(2.0 * (EDGE + SLOT + GAP) + 40.0 <= COLLAPSED.0 as f32 + 20.0, "with eyes in the middle the notch hardly widens");

/// How fast the record turns, degrees a second (one turn in about thirteen seconds).
const SPIN: f32 = 28.0;

/// The tempo the bars pulse to, beats a second. A slow wobble keeps it from being a metronome.
const TEMPO: f32 = 1.9;

fn beat_time(t: f32) -> f32 {
    t + 0.18 * (t * 0.45).sin()
}

/// A pulse that is 0 to 1: a quick rise at the start of each beat and a slower fall after it.
pub(crate) fn pulse(t: f32, delay: f32) -> f32 {
    let phase = (beat_time(t - delay) * TEMPO).rem_euclid(1.0);
    const ATTACK: f32 = 0.07;
    if phase < ATTACK { phase / ATTACK } else { (-(phase - ATTACK) * 4.6).exp() }
}

/// How high bar `i` is at `t` seconds, 0 to 1. Two slow waves of different speeds are laid over each other so the bars
/// rise and fall by themselves, and the beat lifts them one after the other, the low ones the most.
pub(crate) fn bar_level(i: usize, t: f32) -> f32 {
    const SLOW: [f32; BARS] = [3.1, 4.3, 2.6, 3.7, 4.9];
    const FAST: [f32; BARS] = [7.3, 5.9, 8.1, 6.4, 7.7];
    const PUNCH: [f32; BARS] = [1.0, 0.85, 0.62, 0.48, 0.36];
    let i_f = i as f32;
    let wave = 0.55 * (t * SLOW[i % BARS] + i_f * 1.7).sin() + 0.45 * (t * FAST[i % BARS] + i_f * 2.9).sin();
    (0.3 + 0.26 * wave + 0.55 * PUNCH[i % BARS] * pulse(t, i_f * 0.05)).clamp(0.0, 1.0)
}

fn seconds() -> f32 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f32()
}

/// A colour with its hue turned by `degrees`, made lively (saturated and bright) so it shows on the black notch.
pub(crate) fn vivid(rgb: [u8; 3], degrees: f32) -> [u8; 3] {
    let (r, g, b) = (rgb[0] as f32 / 255.0, rgb[1] as f32 / 255.0, rgb[2] as f32 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let delta = max - min;
    let hue = if delta < 1e-5 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    let sat = if max <= 1e-5 { 0.0 } else { delta / max };
    let (hue, sat, val) = ((hue + degrees).rem_euclid(360.0), sat.max(0.55), max.max(0.9));
    // HSV back to RGB.
    let c = val * sat;
    let x = c * (1.0 - ((hue / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = val - c;
    let (r, g, b) = match (hue / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [((r + m) * 255.0).round() as u8, ((g + m) * 255.0).round() as u8, ((b + m) * 255.0).round() as u8]
}

/// A bar with a gradient along its length: light at the tips, strong in the middle.
fn glossy_bar(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, colour: [u8; 3], a: u8) {
    let Some(path) = gfx::rounded_path(x, y, w, h, w / 2.0) else { return };
    let light = gfx::mix([colour[0], colour[1], colour[2], a], [255, 255, 255, a], 0.5);
    let strong = [colour[0], colour[1], colour[2], a];
    let stop = |at: f32, c: [u8; 4]| {
        let m = theme::map(c);
        GradientStop::new(at, Color::from_rgba8(m[0], m[1], m[2], m[3]))
    };
    let stops = vec![stop(0.0, light), stop(0.5, strong), stop(1.0, light)];
    let Some(shader) = LinearGradient::new(Point::from_xy(x, y), Point::from_xy(x, y + h), stops, SpreadMode::Pad, Transform::identity()) else { return };
    pm.fill_path(&path, &Paint { shader, anti_alias: true, ..Default::default() }, FillRule::Winding, Transform::identity(), None);
}

fn disc_path(cx: f32, cy: f32, r: f32) -> Option<tiny_skia::Path> {
    PathBuilder::from_oval(Rect::from_xywh(cx - r, cy - r, r * 2.0, r * 2.0)?)
}

impl App {
    /// 1 while the closed notch should show the record and the bars (the setting is on and a song is playing), else 0.
    pub(crate) fn music_target(&self) -> f64 {
        if self.settings.closed_music && self.media.has_session && self.media.playing { 1.0 } else { 0.0 }
    }

    /// How wide what is in the middle of the closed notch is (the clock, the eyes or the timer), which decides how far
    /// the record and the bars sit from it.
    fn centre_width(&self) -> f32 {
        if self.timer.running() {
            return 54.0;
        }
        // Asleep, the eyes have a "z" and a "Z" beside them.
        if matches!(self.presence, Presence::Dozing) && self.settings.idle_eyes {
            return 84.0;
        }
        if self.centre_is_eyes() || !self.settings.show_clock {
            return 40.0;
        }
        match &self.font {
            Some(font) => ClockText::widest(self.heading_font().unwrap_or(font), self.settings.clock_24h).ceil(),
            None => 70.0,
        }
    }

    /// How much wider than usual the closed notch is while the record and the bars show: just enough that they have room
    /// on both sides of what is in the middle, and none if the usual width has room already.
    pub(crate) fn music_extra_goal(&self) -> f64 {
        if self.music_target() == 0.0 {
            return 0.0;
        }
        let needed = 2.0 * (EDGE + SLOT + GAP) + self.centre_width();
        (needed as f64 - COLLAPSED.0 * self.closed_k).max(0.0)
    }

    /// Whether the music is on the closed notch and something on it moves, so it needs a frame now and then.
    pub(crate) fn bars_running(&self) -> bool {
        self.music_k > 0.02 && self.media.playing && !self.hidden && self.tuck_t < 1.0 && self.progress <= 0.0 && self.notice_until.is_none()
    }

    /// How often it is drawn again: 30 times a second while the bars, the turning record or the edge line move, and only
    /// once a second (for the ring that fills as the song goes on) when all three are switched off.
    pub(crate) fn music_tick(&self) -> Duration {
        if self.settings.closed_bars || self.settings.closed_spin || self.settings.closed_edge { BARS_TICK } else { Duration::from_secs(1) }
    }

    /// Draw the record, the bars and the glowing edge on the closed notch. `alpha` is how visible its content is.
    pub(crate) fn draw_closed_music(&self, pm: &mut Pixmap, s: f32, alpha: f32, fw: f32) {
        let k = self.music_k.clamp(0.0, 1.0) as f32;
        let a = alpha * k;
        if a <= 0.01 {
            return;
        }
        let height = self.closed_size().1 as f32;
        let (mid, cy) = (fw / s / 2.0, height / 2.0);
        let half = self.centre_width() / 2.0;
        let t = seconds();
        // The loudness of each band of the music, when it can be listened to; else the made-up beat and waves.
        let sound = if self.settings.closed_sound && (self.settings.closed_bars || self.settings.closed_edge) { loopback::listen() } else { None };
        let beat = match sound {
            Some(l) => (0.7 * l[0] + 0.3 * l[1]).powf(1.4),
            None => pulse(t, 0.0),
        };
        let [ar, ag, ab] = self.art_shown.map(|c| c.round().clamp(0.0, 255.0) as u8);
        let accent = vivid([ar, ag, ab], 0.0);
        let rgba = |alpha: f32| [accent[0], accent[1], accent[2], (alpha.clamp(0.0, 1.0) * 255.0) as u8];

        // A thin line of the cover's colour along the edge of the notch, breathing with the beat.
        if self.settings.closed_edge {
            self.draw_music_edge(pm, s, fw, height * s, rgba((0.18 + 0.32 * beat) * a));
        }

        // The record, in the left slot, with a halo and a ring that fills as the song goes on. With the turning
        // switched off it keeps still and its halo does not breathe.
        let disc_cx = mid - half - GAP - SLOT / 2.0;
        let spin = self.settings.closed_spin;
        gfx::glow(pm, disc_cx * s, cy * s, 19.0 * s, rgba((0.12 + 0.3 * if spin { beat } else { 0.35 }) * a), None);
        self.draw_record(pm, s, (disc_cx, cy), t, a, rgba(1.0));

        if !self.settings.closed_bars {
            return;
        }

        // The bars in the right slot, with one soft glow behind all of them that follows how high they are together.
        let bars_x = mid + half + GAP;
        let levels: Vec<f32> = match sound {
            Some(l) => l.to_vec(),
            None => (0..BARS).map(|i| bar_level(i, t)).collect(),
        };
        let energy = levels.iter().sum::<f32>() / BARS as f32;
        gfx::glow(pm, (bars_x + BARS_BLOCK / 2.0) * s, cy * s, 17.0 * s, rgba((0.12 + 0.5 * energy) * a), None);
        for (i, level) in levels.iter().enumerate() {
            // They settle as the notch narrows, so a song that stops does not leave them jumping.
            let h = BAR_MIN + (BAR_MAX - BAR_MIN) * level * k;
            let x = bars_x + i as f32 * (BAR_W + BAR_GAP);
            let colour = vivid([ar, ag, ab], (i as f32 - 2.0) * 14.0);
            glossy_bar(pm, x * s, (cy - h / 2.0) * s, BAR_W * s, h * s, colour, (a * 255.0) as u8);
        }
    }

    /// The accent line along the sides and the bottom of the notch, faint at the top and strongest at the bottom.
    fn draw_music_edge(&self, pm: &mut Pixmap, s: f32, fw: f32, fh: f32, c: [u8; 4]) {
        let (hw, r) = (0.7 * s, fh / 2.0);
        let mut pb = PathBuilder::new();
        pb.move_to(hw, 0.0);
        pb.line_to(hw, fh - r);
        pb.quad_to(hw, fh - hw, r, fh - hw);
        pb.line_to(fw - r, fh - hw);
        pb.quad_to(fw - hw, fh - hw, fw - hw, fh - r);
        pb.line_to(fw - hw, 0.0);
        let Some(path) = pb.finish() else { return };
        let stop = |at: f32, alpha: u8| {
            let m = theme::map([c[0], c[1], c[2], alpha]);
            GradientStop::new(at, Color::from_rgba8(m[0], m[1], m[2], m[3]))
        };
        let stops = vec![stop(0.0, 0), stop(0.55, (c[3] as f32 * 0.5) as u8), stop(1.0, c[3])];
        let Some(shader) = LinearGradient::new(Point::from_xy(0.0, 0.0), Point::from_xy(0.0, fh), stops, SpreadMode::Pad, Transform::identity()) else { return };
        let stroke = Stroke { width: 1.4 * s, ..Default::default() };
        pm.stroke_path(&path, &Paint { shader, anti_alias: true, ..Default::default() }, &stroke, Transform::identity(), None);
    }

    /// The record: the cover as a round disc that turns, a small spindle hole, and the ring round it that fills with the
    /// song's progress. With no cover it is a dark disc with a note on it, and it keeps still.
    fn draw_record(&self, pm: &mut Pixmap, s: f32, centre: (f32, f32), t: f32, a: f32, accent: [u8; 4]) {
        let (cx, cy) = (centre.0 * s, centre.1 * s);
        let r = DISC / 2.0 * s;
        let Some(disc) = disc_path(cx, cy, r) else { return };
        let mask = gfx::mask_of(pm.width(), pm.height(), &disc);
        match (&self.media.art, &mask) {
            (Some(rgba), Some(mask)) => {
                let n = media::art_size();
                if let Some(mut art) = Pixmap::new(n, n) {
                    for (dst, px) in art.pixels_mut().iter_mut().zip(rgba.as_chunks::<4>().0) {
                        *dst = tiny_skia::ColorU8::from_rgba(px[0], px[1], px[2], 255).premultiply();
                    }
                    let scale = 2.0 * r / n as f32;
                    let turn = if self.settings.closed_spin { (t * SPIN).rem_euclid(360.0) } else { 0.0 };
                    let at = Transform::from_translate(-(n as f32) / 2.0, -(n as f32) / 2.0).post_scale(scale, scale).post_rotate(turn).post_translate(cx, cy);
                    let paint = tiny_skia::PixmapPaint { opacity: a, quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
                    pm.draw_pixmap(0, 0, art.as_ref(), &paint, at, Some(mask));
                }
            }
            _ => {
                gfx::gradient_circle(pm, cx, cy, r, [58, 58, 68, (a * 255.0) as u8], [30, 30, 38, (a * 255.0) as u8]);
                draw_tab_icon(pm, View::Player, cx, cy, s * 0.55, [150, 150, 165, (a * 255.0) as u8]);
            }
        }
        // The grooves, a faint shine on the upper left, and the spindle hole.
        gfx::stroke_rounded(pm, cx - r * 0.62, cy - r * 0.62, r * 1.24, r * 1.24, r * 0.62, 0.6 * s, [255, 255, 255, (a * 22.0) as u8]);
        gfx::stroke_rounded(pm, cx - r + 0.5 * s, cy - r + 0.5 * s, r * 2.0 - s, r * 2.0 - s, r, 0.9 * s, [255, 255, 255, (a * 40.0) as u8]);
        gfx::glow(pm, cx - r * 0.45, cy - r * 0.5, r * 0.8, [255, 255, 255, (a * 55.0) as u8], mask.as_ref());
        fill_circle(pm, cx, cy, 2.1 * s, [10, 10, 14, (a * 235.0) as u8]);
        fill_circle(pm, cx, cy, 0.8 * s, [255, 255, 255, (a * 90.0) as u8]);

        // The ring: a faint track all the way round, and the part that is done in the cover's colour.
        let ring = RING_R * s;
        gfx::stroke_arc_gradient(pm, cx, cy, ring, RING_W * s, 1.0, [255, 255, 255, (a * 30.0) as u8], [255, 255, 255, (a * 30.0) as u8]);
        let done = {
            let total = self.media.duration.as_secs_f32();
            if total > 0.5 { (self.position().as_secs_f32() / total).clamp(0.0, 1.0) } else { 0.0 }
        };
        let strong = [accent[0], accent[1], accent[2], (a * 245.0) as u8];
        gfx::stroke_arc_gradient(pm, cx, cy, ring, RING_W * s, done, strong, gfx::mix(strong, [255, 255, 255, strong[3]], 0.45));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bars_stay_between_their_least_and_their_most() {
        for i in 0..BARS {
            for step in 0..4000 {
                let level = bar_level(i, step as f32 * 0.025);
                assert!((0.0..=1.0).contains(&level), "bar {i} at {step}: {level}");
            }
        }
    }

    #[test]
    fn the_bars_are_not_all_the_same_and_they_move() {
        let at = |t: f32| (0..BARS).map(|i| bar_level(i, t)).collect::<Vec<_>>();
        let (a, b) = (at(1.0), at(1.4));
        assert!(a.iter().any(|x| (x - a[0]).abs() > 0.04), "the bars differ from each other");
        assert!(a.iter().zip(&b).any(|(x, y)| (x - y).abs() > 0.05), "the bars move in time");
    }

    #[test]
    fn the_beat_rises_fast_and_falls_slowly_and_comes_round_again() {
        // Over a few seconds the pulse reaches nearly 1 on every beat and drops well below a half between them.
        let samples: Vec<f32> = (0..600).map(|i| pulse(i as f32 * 0.01 + 2.0, 0.0)).collect();
        assert!(samples.iter().cloned().fold(0.0, f32::max) > 0.95);
        assert!(samples.iter().cloned().fold(1.0, f32::min) < 0.1);
        assert!(samples.iter().all(|p| (0.0..=1.0).contains(p)));
        let beats = samples.windows(2).filter(|w| w[0] < 0.5 && w[1] >= 0.5).count();
        assert!((9..=14).contains(&beats), "{beats} beats in 6 seconds at about 1.9 a second");
    }

    #[test]
    fn a_hue_turn_changes_the_colour_and_keeps_it_lively() {
        let base = [200, 60, 60];
        let turned = vivid(base, 60.0);
        assert!(turned != vivid(base, 0.0), "the hue moved");
        for deg in [-28.0, 0.0, 28.0, 120.0] {
            let c = vivid(base, deg);
            let (max, min) = (*c.iter().max().unwrap(), *c.iter().min().unwrap());
            assert!(max >= 220 && (max - min) as f32 / max as f32 >= 0.5, "{deg}: {c:?} is bright and saturated");
        }
        // Grey gets a colour too (a cover with no colour in it still gets bars you can see).
        let grey = vivid([128, 128, 128], 0.0);
        assert!(*grey.iter().max().unwrap() >= 220);
    }

}
