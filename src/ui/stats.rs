//! The Stats view: a dashboard of this PC, from `stats.rs`.
//!
//! From the top: a card with a round gauge for the processor, for the memory and for each graphics chip (a ring that
//! fills with a gradient, a bright head at its end and a glow in the colour of the load); a live graph of the last
//! minute of the processor and the graphics chip, beside a card for the network with its speeds and a small graph; and
//! a row for each drive. The notch grows taller to fit them. The numbers are looked at once a second, and when they
//! change the gauges and bars glide to the new values and the graphs step one place to the left, all in the same third
//! of a second; between two looks nothing is drawn, so the open view costs little.

use std::collections::VecDeque;

use crate::*;
use tiny_skia::{GradientStop, LinearGradient, Point, SpreadMode};

type Area = (f32, f32, f32, f32);

/// What the Stats tab keeps: the numbers, when they were last looked at, when the view was last drawn for its gliding
/// graphs (the next is a frame later), and the parts of the view that do not change from one frame to the next, drawn
/// once and copied in (see `blit_backdrop`).
#[derive(Default)]
pub(crate) struct StatsTab {
    pub(crate) data: stats::Stats,
    pub(crate) at: Option<Instant>,
    pub(crate) frame: Option<Instant>,
    pub(crate) backdrops: std::cell::RefCell<StatsBackdrops>,
}

/// The pictures of the parts of the view that stay the same from one frame to the next, one for each card, each with
/// what it was drawn for. Drawing them is most of what a frame of the view costs, so they are drawn once and copied.
#[derive(Default)]
pub(crate) struct StatsBackdrops {
    slots: Vec<Option<Backdrop>>,
}

/// A picture with what it was drawn for: its size, whether the look was Light, and a number for what else it depends on.
type Backdrop = ((u32, u32, bool, u32), Pixmap);

/// The slots: the gauge cards first, then the graph and the network card.
const SLOT_GRAPH: usize = 5;
const SLOT_NETWORK: usize = 6;

/// Where the content sits across the notch.
const MARGIN: f32 = 18.0;
const WIDTH: f32 = 384.0;
/// Where the first card starts, in the view's own coordinates (the view is drawn `TAB_SHIFT` higher than laid out).
const TOP: f32 = 38.0;
const GAP: f32 = 8.0;
/// The gauge cards.
const CARD_H: f32 = 110.0;
const CARD_R: f32 = 14.0;
const RING_W: f32 = 6.5;
/// The graph and the network card, side by side.
const MID_H: f32 = 98.0;
const GRAPH_W: f32 = 236.0;
/// The storage heading and a row for each drive.
const STORAGE_HEAD: f32 = 22.0;
const DRIVE_ROW: f32 = 26.0;
/// How long the gauges and bars take to glide from the numbers of the look before to those of the last one.
const GLIDE: Duration = Duration::from_millis(350);
/// How often the view is drawn while they glide. A frame of the open notch is not free, so there is no more than this,
/// and none at all between two glides.
const FRAME: Duration = Duration::from_millis(66);

/// The colours of the two lines of the graph, and of the two directions of the network.
const CPU_LINE: [u8; 3] = [255, 110, 150];
const GPU_LINE: [u8; 3] = [96, 172, 255];
const DOWN: [u8; 3] = [64, 214, 190];
const UP: [u8; 3] = [255, 190, 90];

/// Green while there is plenty of room, through amber, to red when it is nearly full.
fn load_color(frac: f32, a: u8) -> [u8; 4] {
    let (green, amber, red) = ([80, 200, 120, a], [255, 190, 60, a], [255, 90, 80, a]);
    if frac < 0.6 {
        green
    } else if frac < 0.85 {
        gfx::mix(green, amber, (frac - 0.6) / 0.25)
    } else {
        gfx::mix(amber, red, ((frac - 0.85) / 0.15).clamp(0.0, 1.0))
    }
}

/// The look of a card, drawn on a picture of its own size: a soft white wash that is a little stronger at the top, a
/// hairline border and a brighter hairline just inside the top edge, like the edge of a pane of glass.
fn glass(picture: &mut Pixmap, w: f32, h: f32, s: f32) {
    gfx::gradient_rounded(picture, 0.0, 0.0, w, h, CARD_R * s, [255, 255, 255, 17], [255, 255, 255, 6], false);
    gfx::stroke_rounded(picture, 0.5 * s, 0.5 * s, w - s, h - s, CARD_R * s, s, [255, 255, 255, 18]);
    // The highlight along the top, fading out toward the corners.
    let half = (w - 2.0 * CARD_R * s) / 2.0;
    gfx::gradient_rounded(picture, CARD_R * s, s, half, s, 0.5 * s, [255, 255, 255, 0], [255, 255, 255, 34], true);
    gfx::gradient_rounded(picture, CARD_R * s + half, s, half, s, 0.5 * s, [255, 255, 255, 34], [255, 255, 255, 0], true);
}

/// A number gliding from `from` to `to`, `t` of the way (0.0 to 1.0), slowing as it arrives. With no number before it
/// is at `to` at once, and with none now there is none.
fn glide(from: Option<f32>, to: Option<f32>, t: f32) -> Option<f32> {
    let to = to?;
    let from = from.unwrap_or(to);
    let t = t.clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t);
    Some(from + (to - from) * eased)
}

/// Where everything sits, in the view's coordinates, for this many gauge cards and drives.
#[derive(Debug, PartialEq)]
struct Plan {
    cards: Vec<Area>,
    ring: f32,
    graph: Area,
    net: Area,
    storage_y: f32,
    end: f32,
}

fn plan(cards: usize, drives: usize) -> Plan {
    let n = cards.clamp(2, 5);
    // Up to four gauges side by side; five go three and two.
    let cols = if n <= 4 { n } else { 3 };
    let rows = n.div_ceil(cols);
    let cw = (WIDTH - GAP * (cols - 1) as f32) / cols as f32;
    let cards: Vec<Area> = (0..n)
        .map(|i| {
            let (row, col) = (i / cols, i % cols);
            // A row with fewer cards than the others is centred.
            let in_row = if row == rows - 1 { n - row * cols } else { cols };
            let row_w = in_row as f32 * cw + GAP * (in_row - 1) as f32;
            let x0 = MARGIN + (WIDTH - row_w) / 2.0;
            (x0 + col as f32 * (cw + GAP), TOP + row as f32 * (CARD_H + GAP), cw, CARD_H)
        })
        .collect();
    let mid_y = TOP + rows as f32 * (CARD_H + GAP);
    let graph = (MARGIN, mid_y, GRAPH_W, MID_H);
    let net = (MARGIN + GRAPH_W + GAP, mid_y, WIDTH - GRAPH_W - GAP, MID_H);
    let storage_y = mid_y + MID_H + 12.0;
    let end = storage_y + STORAGE_HEAD + DRIVE_ROW * drives.clamp(1, stats::MAX_DRIVES) as f32;
    Plan { cards, ring: if cw >= 110.0 { 30.0 } else { 26.0 }, graph, net, storage_y, end }
}

/// How much taller than the normal notch the view needs the notch to be, for content that ends at `end`.
fn extra_height(end: f32) -> f32 {
    (end + TAB_BOTTOM - TAB_SHIFT + 6.0 - EXPANDED.1 as f32).max(0.0)
}

/// The points of a line through `values` (oldest first, each 0.0 to 1.0) for a graph `step` apart whose right edge is
/// at `right`, as (x, value). `frac` is how far the next second is on its way (0.0 just after a look, 1.0 when the next
/// is due): the line shows the seconds one behind and glides towards the newest, so it moves smoothly instead of in
/// jumps. Points left of `left` are dropped, except the one that the line enters from.
fn graph_points(values: &[f32], frac: f32, step: f32, right: f32, left: f32) -> Vec<(f32, f32)> {
    let n = values.len();
    if n < 2 {
        return Vec::new();
    }
    let frac = frac.clamp(0.0, 1.0);
    // The moment shown is `frac` of the way from the second newest value to the newest.
    let shown = (n - 2) as f32 + frac;
    let x_of = |i: usize| right - (shown - i as f32) * step;
    let mut points: Vec<(f32, f32)> = Vec::new();
    for (i, v) in values.iter().enumerate() {
        let x = x_of(i);
        if x > right + 0.001 {
            break;
        }
        if x >= left - step {
            points.push((x, *v));
        }
    }
    // The tip is where the line meets the right edge.
    let tip = values[n - 2] + (values[n - 1] - values[n - 2]) * frac;
    if points.last().is_none_or(|(x, _)| *x < right - 0.001) {
        points.push((right, tip));
    }
    points
}

/// A smooth curve through the points (a Catmull-Rom spline, `steps` pieces between each two), so that a line of one
/// value a second does not look like a saw.
fn smooth(points: &[(f32, f32)], steps: usize) -> Vec<(f32, f32)> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut out = Vec::with_capacity(points.len() * steps);
    for i in 0..points.len() - 1 {
        let p0 = points[i.saturating_sub(1)];
        let (p1, p2) = (points[i], points[i + 1]);
        let p3 = points[(i + 2).min(points.len() - 1)];
        for k in 0..steps {
            let t = k as f32 / steps as f32;
            let (t2, t3) = (t * t, t * t * t);
            let at = |a: f32, b: f32, c: f32, d: f32| 0.5 * (2.0 * b + (c - a) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (3.0 * b - a - 3.0 * c + d) * t3);
            out.push((at(p0.0, p1.0, p2.0, p3.0), at(p0.1, p1.1, p2.1, p3.1)));
        }
    }
    out.push(*points.last().unwrap_or(&(0.0, 0.0)));
    out
}

/// Fill the area between a line and the level `base` with a colour that fades from `top` to nothing.
fn fill_under(pm: &mut Pixmap, line: &[(f32, f32)], top: f32, base: f32, c: [u8; 4]) {
    let (Some(first), Some(last)) = (line.first(), line.last()) else { return };
    let mut pb = PathBuilder::new();
    pb.move_to(first.0, base);
    for p in line {
        pb.line_to(p.0, p.1);
    }
    pb.line_to(last.0, base);
    pb.close();
    let Some(path) = pb.finish() else { return };
    let c = theme::map(c);
    let (strong, none) = (Color::from_rgba8(c[0], c[1], c[2], c[3]), Color::from_rgba8(c[0], c[1], c[2], 0));
    let stops = vec![GradientStop::new(0.0, strong), GradientStop::new(1.0, none)];
    let Some(shader) = LinearGradient::new(Point::from_xy(0.0, top), Point::from_xy(0.0, base), stops, SpreadMode::Pad, Transform::identity()) else { return };
    let paint = Paint { shader, anti_alias: true, ..Default::default() };
    pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
}

impl App {
    /// How much taller than normal the notch is while the Stats view is open, to fit the cards, the graphs and a row
    /// for each drive.
    pub(crate) fn stats_extra(&self) -> f64 {
        if self.view != View::Stats {
            return 0.0;
        }
        let p = plan(2 + self.stats.data.last.gpus.len().min(stats::MAX_CHIPS), self.stats.data.last.disks.len());
        extra_height(p.end) as f64
    }

    /// While the Stats view is open, look at the numbers (the first two looks come quickly, then once a second) and
    /// say when to wake for the next one, which is soon: the graphs glide between two looks, so they are drawn about
    /// thirty times a second. Nothing is looked at, or drawn, while it is closed; its numbers are forgotten, so that
    /// it starts afresh next time.
    pub(crate) fn stats_tick(&mut self, now: Instant) -> Option<Instant> {
        if self.progress < 1.0 || self.view != View::Stats {
            if self.stats.data.started {
                // The drives and the names of the graphics chips stay, so the view keeps its shape while it slides out
                // and opens at the right height.
                let disks = std::mem::take(&mut self.stats.data.last.disks);
                let chips: Vec<stats::Chip> = self.stats.data.last.gpus.iter().map(|c| stats::Chip { name: c.name.clone(), load: None }).collect();
                self.stats.data = stats::Stats::default();
                self.stats.data.disks_known = !disks.is_empty();
                self.stats.data.last.disks = disks;
                self.stats.data.last.gpus = chips;
                self.stats.at = None;
                self.stats.frame = None;
            }
            return None;
        }
        let gap = if self.stats.data.ready() { stats::EVERY } else { stats::FIRST_AGAIN };
        let due = self.stats.at.map_or(now, |at| at + gap);
        let mut frame_due = false;
        if now >= due {
            self.stats.data.sample(now);
            self.stats.at = Some(now);
            // The numbers have just changed: draw at once, so that the glide starts.
            frame_due = true;
        }
        let Some(looked_at) = self.stats.at else { return Some(due) };
        // A frame is drawn only when one is due: drawing makes the event loop look again at once, and asking every time
        // would draw as fast as the machine can. Frames follow each other until one has been drawn at the end of the
        // glide (the last of them shows the final picture), and then there are none until the next look.
        let glided = self.stats.frame.is_some_and(|at| at.saturating_duration_since(looked_at) >= GLIDE);
        let next_frame = self.stats.frame.map_or(now, |at| at + FRAME);
        if frame_due || (!glided && now >= next_frame) {
            self.stats.frame = Some(now);
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
        let glided = self.stats.frame.is_some_and(|at| at.saturating_duration_since(looked_at) >= GLIDE);
        if glided {
            return Some(due);
        }
        // The next frame of the glide, or the next look if that comes first.
        Some(self.stats.frame.map_or(now, |at| at + FRAME).min(due).max(now))
    }

    /// How far the glide from the look before to the last one has got, 0.0 just after a look and 1.0 when it is over.
    fn glide_progress(&self) -> f32 {
        self.stats.at.map_or(1.0, |at| (at.elapsed().as_secs_f32() / GLIDE.as_secs_f32()).clamp(0.0, 1.0))
    }

    /// Copy a picture of the part of a card that does not move onto `target` at (`x`, `y`) (in pixels), drawing it first
    /// with `paint` if it has not been drawn yet for this `size`, look and `variant`. `slot` is where it is kept.
    #[allow(clippy::too_many_arguments)]
    fn blit_backdrop(&self, target: &mut Pixmap, slot: usize, (x, y): (i32, i32), size: (u32, u32), variant: u32, alpha: f32, paint: impl FnOnce(&mut Pixmap)) {
        let key = (size.0, size.1, theme::is_light(), variant);
        let mut cache = self.stats.backdrops.borrow_mut();
        if cache.slots.len() <= slot {
            cache.slots.resize_with(slot + 1, || None);
        }
        if cache.slots[slot].as_ref().is_none_or(|(kept, _)| *kept != key)
            && let Some(mut picture) = Pixmap::new(size.0, size.1)
        {
            paint(&mut picture);
            cache.slots[slot] = Some((key, picture));
        }
        let Some((_, picture)) = &cache.slots[slot] else { return };
        if alpha >= 0.999 {
            gfx::blit_over(target, picture, x, y);
        } else {
            // While the view fades in or out.
            let paint = PixmapPaint { opacity: alpha, ..Default::default() };
            target.draw_pixmap(x, y, picture.as_ref(), &paint, Transform::identity(), None);
        }
    }

    /// One gauge card: a ring that fills with `frac`, the figure in it, a name under it and a line of words.
    fn draw_gauge(&self, pm: &mut Pixmap, s: f32, alpha: f32, slot: usize, card: Area, ring: f32, frac: Option<f32>, figure: &str, label: &str, sub: &str) {
        let Some(font) = &self.font else { return };
        let heading = self.heading_font().unwrap_or(font);
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [165, 165, 175, a]);
        let shown = frac.map_or(0.0, |f| f.clamp(0.0, 1.0));
        let col = load_color(shown, a);
        // The card, the glow behind the ring (in the colour of the load, stronger the busier it is) and the grey track
        // of the ring are drawn once for each size and each twentieth of the load, and copied in.
        let (px, py) = ((card.0 * s).round(), (card.1 * s).round());
        let size = ((card.2 * s).ceil() as u32, (card.3 * s).ceil() as u32);
        let bucket = (shown * 20.0).round() as u32;
        let (cx, cy) = (px + card.2 / 2.0 * s, py + (12.0 + ring) * s);
        self.blit_backdrop(pm, slot, (px as i32, py as i32), size, bucket, alpha, |picture| {
            let still = load_color(bucket as f32 / 20.0, 255);
            glass(picture, card.2 * s, card.3 * s, s);
            let (rx, ry) = (card.2 / 2.0 * s, (12.0 + ring) * s);
            gfx::glow(picture, rx, ry, (ring + 16.0) * s, [still[0], still[1], still[2], (20.0 + 34.0 * bucket as f32 / 20.0) as u8], None);
            stroke_arc(picture, rx, ry, ring * s, RING_W * s, 1.0, [255, 255, 255, 22]);
        });
        if frac.is_some() {
            gfx::stroke_arc_gradient(pm, cx, cy, ring * s, RING_W * s, shown, gfx::mix(col, white, 0.5), col);
            // A bright head at the end of the arc.
            if shown > 0.03 && shown < 0.995 {
                let ang = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * shown;
                let (hx, hy) = (cx + ring * s * ang.cos(), cy + ring * s * ang.sin());
                gfx::glow(pm, hx, hy, 9.0 * s, [col[0], col[1], col[2], (alpha * 90.0) as u8], None);
                fill_circle(pm, hx, hy, RING_W * 0.34 * s, [255, 255, 255, a]);
            }
        }
        draw_text_centered(pm, heading, figure, cx, cy + 6.5 * s, (if ring >= 30.0 { 19.0 } else { 17.0 }) * s, white, (ring * 2.0 - 8.0) * s);
        let (mid, w) = (cx, (card.2 - 10.0) * s);
        draw_text_centered(pm, heading, label, mid, (card.1 + 12.0 + ring * 2.0 + 20.0) * s, 11.0 * s, white, w);
        draw_text_centered(pm, font, sub, mid, (card.1 + 12.0 + ring * 2.0 + 34.0) * s, 9.5 * s, grey, w);
    }

    /// The processor and the graphics chip over the last minute, as two lines that fill below themselves.
    fn draw_activity(&self, pm: &mut Pixmap, s: f32, alpha: f32, area: Area) {
        let Some(font) = &self.font else { return };
        let heading = self.heading_font().unwrap_or(font);
        let a = (alpha * 255.0) as u8;
        let grey = [165, 165, 175, a];
        let (x, y, w, h) = area;
        self.blit_backdrop(pm, SLOT_GRAPH, ((x * s).round() as i32, (y * s).round() as i32), ((w * s).ceil() as u32, (h * s).ceil() as u32), 0, alpha, |picture| {
            glass(picture, w * s, h * s, s);
        });
        draw_text(pm, heading, "Last minute", (x + 12.0) * s, (y + 19.0) * s, 11.0 * s, [255, 255, 255, a], 100.0 * s);

        // The key to the two lines, on the right.
        let percent = |load: f32| format!("{}%", (load * 100.0).round() as u32);
        let gpu_now = self.stats.data.last.gpus.iter().filter_map(|chip| chip.load).reduce(f32::max);
        let keys = [
            ("GPU", GPU_LINE, (!self.stats.data.gpu_history.is_empty()).then(|| gpu_now.map_or("...".to_string(), percent))),
            ("CPU", CPU_LINE, Some(self.stats.data.last.cpu.map_or("...".to_string(), percent))),
        ];
        let mut kx = x + w - 12.0;
        for (name, rgb, value) in keys {
            let Some(value) = value else { continue };
            let vw = text_width(font, &value, 9.5 * s) / s;
            let nw = text_width(font, name, 9.5 * s) / s;
            draw_text(pm, heading, &value, (kx - vw) * s, (y + 18.5) * s, 9.5 * s, [255, 255, 255, a], 50.0 * s);
            draw_text(pm, font, name, (kx - vw - 4.0 - nw) * s, (y + 18.5) * s, 9.5 * s, grey, 40.0 * s);
            fill_circle(pm, (kx - vw - 4.0 - nw - 7.0) * s, (y + 15.5) * s, 3.0 * s, [rgb[0], rgb[1], rgb[2], a]);
            kx -= vw + nw + 22.0;
        }

        // The plot is drawn on a picture of its own, which cuts the lines off at its edges.
        let (px, py, pw, ph) = (x + 12.0, y + 30.0, w - 24.0, h - 30.0 - 12.0);
        let Some(mut plot) = Pixmap::new((pw * s).ceil().max(1.0) as u32, (ph * s).ceil().max(1.0) as u32) else { return };
        for k in 1..4 {
            let gy = ph * k as f32 / 4.0;
            fill_rect(&mut plot, 0.0, gy * s, pw * s, s * 0.7, [255, 255, 255, (alpha * if k == 2 { 22.0 } else { 12.0 }) as u8]);
        }
        let step = pw / (stats::HISTORY - 1) as f32;
        // The graphs step one place to the left in the glide after each look.
        let frac = glide(Some(0.0), Some(1.0), self.glide_progress()).unwrap_or(1.0);
        let line = |values: &VecDeque<f32>| -> Vec<(f32, f32)> {
            let v: Vec<f32> = values.iter().copied().collect();
            let pts = graph_points(&v, frac, step, pw, 0.0);
            smooth(&pts, 2).into_iter().map(|(gx, value)| (gx * s, (ph - 2.0 - value.clamp(0.0, 1.0) * (ph - 4.0)) * s)).collect()
        };
        for (values, rgb) in [(&self.stats.data.gpu_history, GPU_LINE), (&self.stats.data.history, CPU_LINE)] {
            let pts = line(values);
            if pts.len() < 2 {
                continue;
            }
            fill_under(&mut plot, &pts, 0.0, ph * s, [rgb[0], rgb[1], rgb[2], (alpha * 96.0) as u8]);
            stroke_poly(&mut plot, &pts, 1.7 * s, [rgb[0], rgb[1], rgb[2], a], false);
            // A dot at the tip, with a glow.
            if let Some((tx, ty)) = pts.last() {
                gfx::glow(&mut plot, *tx - 3.0 * s, *ty, 8.0 * s, [rgb[0], rgb[1], rgb[2], (alpha * 90.0) as u8], None);
                fill_circle(&mut plot, *tx - 3.0 * s, *ty, 2.4 * s, [255, 255, 255, a]);
            }
        }
        pm.draw_pixmap((px * s).round() as i32, (py * s).round() as i32, plot.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
        if self.stats.data.history.len() < 2 {
            draw_text_centered(pm, font, "Looking...", (px + pw / 2.0) * s, (py + ph / 2.0 + 3.0) * s, 10.5 * s, grey, pw * s);
        }
    }

    /// The network: what comes in and what goes out, each in its own colour, and a small graph of both.
    fn draw_network(&self, pm: &mut Pixmap, s: f32, alpha: f32, area: Area) {
        let Some(font) = &self.font else { return };
        let heading = self.heading_font().unwrap_or(font);
        let a = (alpha * 255.0) as u8;
        let grey = [165, 165, 175, a];
        let (x, y, w, h) = area;
        self.blit_backdrop(pm, SLOT_NETWORK, ((x * s).round() as i32, (y * s).round() as i32), ((w * s).ceil() as u32, (h * s).ceil() as u32), 0, alpha, |picture| {
            glass(picture, w * s, h * s, s);
        });
        draw_text(pm, heading, "Network", (x + 12.0) * s, (y + 19.0) * s, 11.0 * s, [255, 255, 255, a], 100.0 * s);

        let speeds = self.stats.data.last.network.map(|(d, u)| (stats::speed(d), stats::speed(u)));
        let (down, up) = speeds.unwrap_or(("...".to_string(), "...".to_string()));
        for (i, (arrow, text, rgb)) in [("\u{2193}", down, DOWN), ("\u{2191}", up, UP)].into_iter().enumerate() {
            let by = y + 38.0 + i as f32 * 17.0;
            fill_circle(pm, (x + 18.0) * s, (by - 4.0) * s, 6.5 * s, [rgb[0], rgb[1], rgb[2], (alpha * 46.0) as u8]);
            draw_text_centered(pm, font, arrow, (x + 18.0) * s, (by - 0.5) * s, 10.0 * s, [rgb[0], rgb[1], rgb[2], a], 12.0 * s);
            draw_text(pm, heading, &text, (x + 30.0) * s, by * s, 12.5 * s, [255, 255, 255, a], (w - 38.0) * s);
        }

        // Both directions on one scale that eases to what the last minute needed, never below a quiet link.
        let scale = glide(Some(self.stats.data.net_scale_prev), Some(self.stats.data.net_scale), self.glide_progress()).unwrap_or(stats::QUIET_LINK).max(stats::QUIET_LINK);
        let (px, py, pw, ph) = (x + 12.0, y + 68.0, w - 24.0, 20.0);
        let Some(mut plot) = Pixmap::new((pw * s).ceil().max(1.0) as u32, (ph * s).ceil().max(1.0) as u32) else { return };
        let step = pw / (stats::HISTORY - 1) as f32;
        let frac = glide(Some(0.0), Some(1.0), self.glide_progress()).unwrap_or(1.0);
        for (pick_up, rgb) in [(false, DOWN), (true, UP)] {
            let v: Vec<f32> = self.stats.data.net_history.iter().map(|(d, u)| (if pick_up { *u } else { *d }) / scale).collect();
            let pts: Vec<(f32, f32)> =
                smooth(&graph_points(&v, frac, step, pw, 0.0), 2).into_iter().map(|(gx, value)| (gx * s, (ph - 1.5 - value.clamp(0.0, 1.0) * (ph - 3.0)) * s)).collect();
            if pts.len() < 2 {
                continue;
            }
            if !pick_up {
                fill_under(&mut plot, &pts, 0.0, ph * s, [rgb[0], rgb[1], rgb[2], (alpha * 80.0) as u8]);
            }
            stroke_poly(&mut plot, &pts, 1.4 * s, [rgb[0], rgb[1], rgb[2], a], false);
        }
        pm.draw_pixmap((px * s).round() as i32, (py * s).round() as i32, plot.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
        if self.stats.data.net_history.len() < 2 {
            draw_text_centered(pm, font, "...", (px + pw / 2.0) * s, (py + ph / 2.0 + 3.0) * s, 10.0 * s, grey, pw * s);
        }
    }

    /// A row for each drive: its letter, a bar that fills with what is used, and the figures.
    fn draw_storage(&self, pm: &mut Pixmap, s: f32, alpha: f32, top: f32) {
        let Some(font) = &self.font else { return };
        let heading = self.heading_font().unwrap_or(font);
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [165, 165, 175, a]);
        draw_text(pm, heading, "Storage", (MARGIN + 2.0) * s, (top + 12.0) * s, 11.0 * s, white, 120.0 * s);
        let last = &self.stats.data.last;
        if last.disks.is_empty() {
            let text = if self.stats.data.disks_known { "No drives found" } else { "..." };
            draw_text(pm, font, text, (MARGIN + 2.0) * s, (top + STORAGE_HEAD + 16.0) * s, 11.0 * s, grey, 200.0 * s);
            return;
        }
        for (i, d) in last.disks.iter().enumerate() {
            let y = top + STORAGE_HEAD + DRIVE_ROW * i as f32;
            let frac = if d.total > 0 { d.used as f32 / d.total as f32 } else { 0.0 };
            let before = self.stats.data.prev.disks.get(i).filter(|p| p.letter == d.letter && p.total > 0).map(|p| p.used as f32 / p.total as f32);
            let shown = glide(before, Some(frac), self.glide_progress()).unwrap_or(frac).clamp(0.0, 1.0);
            // The letter in a small tile that takes the colour of how full the drive is.
            let col = load_color(shown, a);
            fill_rounded(pm, MARGIN * s, y * s, 22.0 * s, 20.0 * s, 6.0 * s, [col[0], col[1], col[2], (alpha * 40.0) as u8]);
            draw_text_centered(pm, heading, &d.letter.to_string(), (MARGIN + 11.0) * s, (y + 14.5) * s, 12.0 * s, white, 20.0 * s);
            let words = stats::used_of(d.used, d.total);
            let tw = text_width(font, &words, 10.5 * s);
            draw_text(pm, font, &words, (MARGIN + WIDTH) * s - tw, (y + 14.0) * s, 10.5 * s, grey, 100.0 * s);
            let bar_x = MARGIN + 32.0;
            let bar_w = WIDTH - 32.0 - tw / s - 10.0;
            draw_bar(pm, bar_x * s, (y + 10.0) * s, bar_w * s, shown, col, s, a);
        }
    }

    pub(crate) fn draw_stats(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        if self.font.is_none() {
            return;
        }
        let last = &self.stats.data.last;
        let chips = last.gpus.len().min(stats::MAX_CHIPS);
        let p = plan(2 + chips, last.disks.len());

        // The gauges: the processor, the memory and each graphics chip.
        let t = self.glide_progress();
        let prev = &self.stats.data.prev;
        let cpu = glide(prev.cpu, last.cpu, t);
        let cpu_sub = format!("{} threads", stats::threads());
        let percent = |frac: Option<f32>| frac.map_or("...".to_string(), |f| format!("{}%", (f * 100.0).round() as u32));
        self.draw_gauge(pm, s, alpha, 0, p.cards[0], p.ring, cpu, &percent(cpu), "Processor", &cpu_sub);
        let fraction = |m: Option<(u64, u64)>| m.map(|(used, total)| used as f32 / total as f32);
        let memory = glide(fraction(prev.memory), fraction(last.memory), t);
        let memory_sub = last.memory.map_or("not available".to_string(), |(used, total)| format!("{} / {}", stats::gigabytes(used), stats::gigabytes(total)));
        self.draw_gauge(pm, s, alpha, 1, p.cards[1], p.ring, memory, &percent(memory), "Memory", &memory_sub);
        for (i, chip) in last.gpus.iter().take(chips).enumerate() {
            let before = prev.gpus.get(i).filter(|p| p.name == chip.name).and_then(|p| p.load);
            let load = glide(before, chip.load, t);
            self.draw_gauge(pm, s, alpha, 2 + i, p.cards[2 + i], p.ring, load, &percent(load), &stats::short_gpu_name(&chip.name), "Graphics");
        }

        self.draw_activity(pm, s, alpha, p.graph);
        self.draw_network(pm, s, alpha, p.net);
        self.draw_storage(pm, s, alpha, p.storage_y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_colour_goes_from_green_to_red_as_things_fill_up() {
        let (low, mid, high) = (load_color(0.1, 255), load_color(0.7, 255), load_color(1.0, 255));
        assert!(low[1] > low[0] && high[0] > high[1], "green when empty, red when full");
        assert!(mid[0] > low[0] && mid[1] > high[1], "amber in between");
    }

    #[test]
    fn a_number_glides_to_its_new_value_and_slows_as_it_arrives() {
        assert_eq!(glide(Some(0.2), Some(0.8), 0.0), Some(0.2));
        assert_eq!(glide(Some(0.2), Some(0.8), 1.0), Some(0.8));
        let half = glide(Some(0.2), Some(0.8), 0.5).unwrap();
        assert!(half > 0.5 && half < 0.8, "more than half way after half the time: {half}");
        let (a, b, c) = (glide(Some(0.0), Some(1.0), 0.2).unwrap(), glide(Some(0.0), Some(1.0), 0.4).unwrap(), glide(Some(0.0), Some(1.0), 0.6).unwrap());
        assert!(b - a > c - b, "slowing down");
        assert_eq!(glide(None, Some(0.7), 0.3), Some(0.7), "nothing before it: no glide");
        assert_eq!(glide(Some(0.7), None, 0.3), None, "no number now: none");
        assert_eq!(glide(Some(0.0), Some(1.0), 7.0), Some(1.0), "clamped");
    }

    #[test]
    fn the_cards_fit_inside_the_notch_whatever_their_number() {
        for cards in 2..=5 {
            let p = plan(cards, 1);
            assert_eq!(p.cards.len(), cards);
            for c in &p.cards {
                assert!(c.0 >= MARGIN - 0.01 && c.0 + c.2 <= MARGIN + WIDTH + 0.01, "{cards} cards: {c:?}");
                assert!(c.2 >= 80.0, "wide enough for a name: {c:?}");
            }
            for pair in p.cards.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                let same_row = (a.1 - b.1).abs() < 0.01;
                assert!(!same_row || a.0 + a.2 + GAP <= b.0 + 0.01, "no overlap in a row");
            }
            assert!(p.graph.1 >= p.cards.iter().map(|c| c.1 + c.3).fold(0.0, f32::max) + GAP - 0.01, "the graph is under the cards");
        }
        assert!(MARGIN + WIDTH <= EXPANDED.0 as f32 - MARGIN + 0.01, "the content fits the notch");
        // Five cards are three and two, the last two in the middle.
        let five = plan(5, 1);
        assert_eq!(five.cards[0].1, five.cards[2].1);
        assert!(five.cards[3].1 > five.cards[2].1);
        assert!((five.cards[3].0 - five.cards[0].0).abs() > 1.0, "the short row is centred");
    }

    #[test]
    fn each_drive_and_each_extra_row_of_cards_make_the_notch_taller() {
        let one = extra_height(plan(3, 1).end);
        assert!(one > 0.0, "the dashboard needs more than the normal notch");
        assert!((extra_height(plan(3, 3).end) - one - 2.0 * DRIVE_ROW).abs() < 0.01, "a row for each drive");
        assert!(extra_height(plan(5, 1).end) > extra_height(plan(4, 1).end), "a second row of gauges");
        assert!(extra_height(plan(2, 0).end) > 0.0, "no drive found still shows one row");
        let p = plan(4, 6);
        assert!(p.graph.1 + p.graph.3 < p.storage_y && p.storage_y < p.end, "the parts follow one another");
        assert!(extra_height(p.end) < 700.0, "even the fullest dashboard stays a sensible height");
    }

    #[test]
    fn a_graph_line_ends_at_the_right_edge_and_glides_with_the_time() {
        let values = [0.1, 0.5, 0.9];
        let (step, right) = (10.0, 100.0);
        // Just after a look the line shows the second newest value at the edge; at the next look the newest.
        let start = graph_points(&values, 0.0, step, right, 0.0);
        assert_eq!(start.last().copied(), Some((right, 0.5)));
        let end = graph_points(&values, 1.0, step, right, 0.0);
        assert_eq!(end.last().copied(), Some((right, 0.9)));
        let half = graph_points(&values, 0.5, step, right, 0.0);
        assert!((half.last().unwrap().1 - 0.7).abs() < 1e-5, "half way between the two");
        assert!(half.windows(2).all(|w| w[0].0 < w[1].0), "left to right");
        // The line moves left as time passes: the same value sits further left a little later.
        let later = graph_points(&values, 0.75, step, right, 0.0);
        assert!(later[0].0 < half[0].0);
        // Too little to draw a line.
        assert!(graph_points(&[0.5], 0.5, step, right, 0.0).is_empty());
        assert!(graph_points(&[], 0.5, step, right, 0.0).is_empty());
    }

    #[test]
    fn a_long_history_is_cut_at_the_left_edge() {
        let values = vec![0.5; 60];
        let points = graph_points(&values, 0.3, 4.0, 236.0, 0.0);
        assert!(points.iter().all(|(x, _)| *x >= -4.0 - 0.001 && *x <= 236.0 + 0.001));
        assert!(points.len() <= 62);
    }

    #[test]
    fn the_smooth_curve_passes_through_the_points_and_stays_on_the_same_course() {
        let points = [(0.0, 0.0), (10.0, 5.0), (20.0, 1.0), (30.0, 8.0)];
        let curve = smooth(&points, 4);
        assert_eq!(curve.first().copied(), Some((0.0, 0.0)));
        assert_eq!(curve.last().copied(), Some((30.0, 8.0)));
        for p in &points[..3] {
            assert!(curve.iter().any(|c| (c.0 - p.0).abs() < 1e-4 && (c.1 - p.1).abs() < 1e-4), "{p:?} is on the curve");
        }
        assert!(curve.windows(2).all(|w| w[0].0 <= w[1].0), "never goes backwards");
        assert_eq!(smooth(&points[..2], 4), points[..2].to_vec(), "two points are a straight line");
    }
}
