//! The Water view.

use crate::*;

const KEY_WATER: u64 = 0x4000_0000_0000_0004;

const WATER_CTRL_Y: f32 = 136.0;

const WATER_X: [f32; 3] = [232.0, 293.0, 354.0]; // left, center, right controls (used by the water view)

const ADJUST: [(f32, f32); 2] = [(231.0, 70.0), (355.0, 70.0)]; // minus, plus button centers

const ADJUST_R: f32 = 11.0;

const WATER_CHIP_W: f32 = 46.0;

/// Reminder interval chip, centered over the right-hand column of the water view.
pub(crate) fn water_chip_rect(i: usize) -> (f32, f32, f32, f32) {
    let n = water::INTERVALS.len() as f32;
    let total = n * WATER_CHIP_W + (n - 1.0) * CHIP_GAP;
    (WATER_X[1] - total / 2.0 + i as f32 * (WATER_CHIP_W + CHIP_GAP), 34.0, WATER_CHIP_W, 18.0)
}

impl App {
    pub(crate) fn draw_water(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let Some(font) = &self.font else { return };
        let heading = self.heading_font().unwrap_or(font);
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [165, 165, 175, a]);
        let rgb = self.accent(View::Water);
        let accent = [rgb[0], rgb[1], rgb[2], a];
        let now = Instant::now();

        // Reminder interval picker.
        for (i, interval) in water::INTERVALS.iter().enumerate() {
            let r = water_chip_rect(i);
            let label = match interval {
                Some(m) if m % 60 == 0 => format!("{}h", m / 60),
                Some(m) => format!("{m}m"),
                None => "Off".to_string(),
            };
            draw_chip(pm, font, r, &label, i == self.water.interval, self.hot_t(r, s), rgb, s, alpha);
        }

        // Left: a ring around a glass that fills with today's count.
        let (rx, ry, rr) = RING;
        gfx::glow(pm, rx * s, ry * s, (rr + 28.0) * s, [rgb[0], rgb[1], rgb[2], (alpha * 34.0) as u8], None);
        stroke_arc(pm, rx * s, ry * s, rr * s, RING_W * s, 1.0, [255, 255, 255, (alpha * 22.0) as u8]);
        // The level springs to its new height, so it sloshes a little when a glass is added.
        let level = (self.glows.follow(KEY_WATER, self.water.progress() as f64, 16.0, 0.55) as f32).clamp(0.0, 1.0);
        gfx::stroke_arc_gradient(pm, rx * s, ry * s, rr * s, RING_W * s, level, gfx::mix(accent, white, 0.35), accent);
        draw_water_fill(pm, rx * s, ry * s, (rr - RING_W / 2.0 - 4.0) * s, level, rgb, s, alpha);
        let count = format!("{}/{}", self.water.count, self.water.goal);
        draw_text_centered(pm, heading, &count, rx * s, (ry + 6.0) * s, 28.0 * s, white, (rr * 2.0 - 8.0) * s);
        draw_text_centered(pm, font, "glasses", rx * s, (ry + 24.0) * s, 10.0 * s, [215, 225, 240, a], (rr * 2.0 - 8.0) * s);

        // Right: title with goal adjusters, and status.
        let cx = WATER_X[1] * s;
        draw_text_centered(pm, heading, "Water", cx, 76.0 * s, 18.0 * s, white, 190.0 * s);
        for (i, (x, y)) in ADJUST.iter().enumerate() {
            let hot = self.hot_t((x - ADJUST_R, y - ADJUST_R, ADJUST_R * 2.0, ADJUST_R * 2.0), s);
            draw_round_button(pm, x * s, y * s, ADJUST_R * s, hot, true, alpha);
            fill_rect(pm, (x - 4.0) * s, (y - 0.75) * s, 8.0 * s, 1.5 * s, white);
            if i == 1 {
                fill_rect(pm, (x - 0.75) * s, (y - 4.0) * s, 1.5 * s, 8.0 * s, white);
            }
        }
        if self.alert_until.is_some() {
            for (i, row) in wrap(font, &self.alert_msg, 12.0, 190.0).iter().take(2).enumerate() {
                draw_text_centered(pm, font, row, cx, (96.0 + i as f32 * 15.0) * s, 12.0 * s, accent, 200.0 * s);
            }
        } else {
            let goal = format!("Daily goal: {} glasses", self.water.goal);
            draw_text_centered(pm, font, &goal, cx, 96.0 * s, 12.0 * s, grey, 190.0 * s);
            let (text, color) = if self.water.reached() {
                ("Goal reached!".to_string(), accent)
            } else {
                match self.water.next_in(now) {
                    // Round up so it never shows "0 min" before the reminder fires.
                    Some(d) => (format!("Next reminder in {} min", d.as_secs().div_ceil(60).max(1)), grey),
                    None => ("Reminders off".to_string(), grey),
                }
            };
            draw_text_centered(pm, font, &text, cx, 114.0 * s, 11.0 * s, color, 190.0 * s);
        }

        // Undo and Reset, and the big add button.
        for (x, label) in [(WATER_X[0], "Undo"), (WATER_X[2], "Reset")] {
            let r = (x - 26.0, WATER_CTRL_Y - 11.0, 52.0, 22.0);
            let hot = self.hot_t(r, s);
            if hot > 0.01 {
                fill_rounded(pm, r.0 * s, r.1 * s, r.2 * s, r.3 * s, 11.0 * s, [255, 255, 255, (alpha * 18.0 * hot) as u8]);
            }
            draw_text_centered(pm, font, label, x * s, (WATER_CTRL_Y + 4.0) * s, 12.0 * s, pick(white, grey, hot), 60.0 * s);
        }
        let big = self.hot_t((WATER_X[1] - 20.0, WATER_CTRL_Y - 20.0, 40.0, 40.0), s);
        draw_big_button(pm, cx, WATER_CTRL_Y * s, 18.0 * s, big, rgb, alpha);
        let black = [0, 0, 0, a];
        fill_rect(pm, cx - 7.0 * s, (WATER_CTRL_Y - 1.25) * s, 14.0 * s, 2.5 * s, black);
        fill_rect(pm, cx - 1.25 * s, (WATER_CTRL_Y - 7.0) * s, 2.5 * s, 14.0 * s, black);
    }

    /// A click in the Water view. (`cx`, `cy`) is the spot in the view's own coordinates.
    pub(crate) fn click_water(&mut self, cx: f32, cy: f32, window: &Window) {
        let now = Instant::now();
        let in_label = |x: f32| (cx - x).abs() <= 26.0 && (cy - WATER_CTRL_Y).abs() <= 14.0;
        let chip = (0..water::INTERVALS.len()).find(|&i| {
            let r = water_chip_rect(i);
            cx >= r.0 && cx <= r.0 + r.2 && cy >= r.1 && cy <= r.1 + r.3
        });
        let adjust = ADJUST.iter().position(|(x, y)| (cx - x).hypot(cy - y) <= ADJUST_R);
        if let Some(i) = chip {
            self.water.set_interval(i, now);
        } else if let Some(i) = adjust {
            // Minus is index 0, plus is index 1.
            self.water.adjust_goal(if i == 0 { -1 } else { 1 });
        } else if (cx - WATER_X[1]).hypot(cy - WATER_CTRL_Y) <= 18.0 {
            self.water.add(now);
        } else if in_label(WATER_X[0]) {
            self.water.undo();
        } else if in_label(WATER_X[2]) {
            self.water.reset();
        } else {
            return;
        }
        self.alert_until = None;
        window.request_redraw();
    }
}
