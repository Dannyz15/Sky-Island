//! The eyes on the closed notch and their idle animation.

use crate::*;

pub(crate) const GREETING: Duration = Duration::from_millis(1600);

const BLINK: Duration = Duration::from_millis(160);

impl App {
    /// Schedule and trigger the redraws the closed notch needs: clock minute changes, blinking
    /// eyes, dozing animation and the greeting. Does nothing when something else is showing.
    /// Whether the awake eyes are drawn on the closed notch right now: while you are working they take
    /// the place of the clock (so only when the clock is off), and after a while without use they come
    /// from "Idle eyes".
    pub(crate) fn eyes_open(&self) -> bool {
        match self.presence {
            Presence::Active => self.settings.eyes && !self.settings.show_clock,
            Presence::Idle => self.settings.idle_eyes,
            Presence::Dozing => false,
        }
    }

    pub(crate) fn companion_tick(&mut self, now: Instant, wake: &mut Option<Instant>) {
        if self.hidden || self.tuck_t >= 1.0 || self.progress > 0.0 || self.notice_until.is_some() || self.timer.running() {
            return;
        }
        let mut redraw = false;

        if let Some(g) = self.greeting_until {
            if now >= g {
                self.greeting_until = None;
                redraw = true;
            } else {
                earliest(wake, g);
                return;
            }
        }

        match self.presence {
            Presence::Active if self.settings.show_clock => {
                let (_, minute, sec, ms) = local_time();
                if minute != self.last_minute {
                    self.last_minute = minute;
                    redraw = true;
                }
                // Wake just after the next minute starts.
                let to_next = (59 - sec) * 1000 + (1000 - ms) + 30;
                earliest(wake, now + Duration::from_millis(to_next as u64));
            }
            Presence::Active | Presence::Idle if self.eyes_open() => {
                if let Some(b) = self.blink_until {
                    if now >= b {
                        self.blink_until = None;
                        redraw = true;
                    } else {
                        earliest(wake, b);
                    }
                }
                if self.blink_until.is_none() {
                    if now >= self.next_blink {
                        self.blink_until = Some(now + BLINK);
                        self.blink_count += 1;
                        // Irregular gaps between blinks, 3 to 7 seconds.
                        self.next_blink = now + Duration::from_millis(3000 + (self.blink_count as u64 * 1373) % 4000);
                        redraw = true;
                        earliest(wake, now + BLINK);
                    } else {
                        earliest(wake, self.next_blink);
                    }
                }
            }
            Presence::Dozing if self.settings.idle_eyes => {
                if now >= self.next_zz {
                    self.zz_frame = !self.zz_frame;
                    self.next_zz = now + Duration::from_millis(1100);
                    redraw = true;
                }
                earliest(wake, self.next_zz);
            }
            _ => {}
        }

        if redraw
            && let Some(w) = &self.window {
                w.request_redraw();
            }
    }

    /// What the closed notch shows when nothing else needs it: eyes when idle, otherwise the clock.
    /// One open (or blinking) eye on the closed notch, in the chosen style, centred at `x`, `cy`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_eye(&self, pm: &mut Pixmap, x: f32, cy: f32, s: f32, a: u8, blink: bool, glance: f32) {
        let c = eye_colour(self.settings.eye_style);
        let ink = theme::map([c[0], c[1], c[2], a]);
        let mut paint = Paint::default();
        paint.set_color_rgba8(ink[0], ink[1], ink[2], ink[3]);
        paint.anti_alias = true;
        // (width, height) of the eye; a blink squashes it to a thin line.
        let (w, open_h) = match self.settings.eye_style {
            EyeStyle::Classic => (9.0, 12.0),
            EyeStyle::Dots => (8.0, 8.0),
            EyeStyle::Cat => (10.0, 12.0),
            EyeStyle::Glow => (7.0, 13.0),
        };
        let (w, h) = (w * s, if blink { 1.6 } else { open_h } * s);
        if self.settings.eye_style == EyeStyle::Glow && !blink {
            gfx::glow(pm, x, cy, 13.0 * s, [c[0], c[1], c[2], (a as f32 * 0.45) as u8], None);
        }
        if let Some(rect) = Rect::from_xywh(x - w / 2.0, cy - h / 2.0, w, h)
            && let Some(path) = PathBuilder::from_oval(rect) {
                pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
            }
        if blink {
            return;
        }
        match self.settings.eye_style {
            EyeStyle::Classic => fill_circle(pm, x + glance * 1.5 * s, cy + 1.0 * s, 3.0 * s, [0, 0, 0, a]),
            EyeStyle::Cat => {
                // A narrow slit that drifts with the glance.
                if let Some(rect) = Rect::from_xywh(x + glance * 1.2 * s - 1.2 * s, cy - 5.0 * s, 2.4 * s, 10.0 * s)
                    && let Some(path) = PathBuilder::from_oval(rect) {
                        let mut slit = Paint::default();
                        slit.set_color_rgba8(0, 0, 0, a);
                        slit.anti_alias = true;
                        pm.fill_path(&path, &slit, FillRule::Winding, Transform::identity(), None);
                    }
            }
            EyeStyle::Dots | EyeStyle::Glow => {}
        }
    }

    pub(crate) fn draw_companion(&self, pm: &mut Pixmap, s: f32, alpha: f32, fw: f32) {
        let Some(font) = &self.font else { return };
        let a = (alpha * 255.0) as u8;
        let (cx, cy) = (fw / 2.0, self.closed_size().1 as f32 / 2.0 * s);
        let (left, right) = (cx - 13.0 * s, cx + 13.0 * s);
        let eyes = self.settings.idle_eyes;
        let white = {
            let c = eye_colour(self.settings.eye_style);
            [c[0], c[1], c[2], a]
        };

        if eyes && self.greeting_until.is_some() {
            for x in [left, right] {
                stroke_arc_from(pm, x, cy + 3.0 * s, 5.0 * s, 2.0 * s, -std::f32::consts::PI + 0.35, std::f32::consts::PI - 0.7, white);
            }
            draw_text(pm, font, "hi!", cx + 28.0 * s, cy + 4.0 * s, 11.0 * s, white, 30.0 * s);
            return;
        }

        match self.presence {
            Presence::Active | Presence::Idle if self.eyes_open() => {
                let blink = self.blink_until.is_some();
                let glance = (self.blink_count % 3) as f32 - 1.0;
                for x in [left, right] {
                    self.draw_eye(pm, x, cy, s, a, blink, glance);
                }
            }
            Presence::Dozing if eyes => {
                for x in [left, right] {
                    stroke_arc_from(pm, x, cy - 2.0 * s, 5.0 * s, 2.0 * s, 0.3, std::f32::consts::PI - 0.6, white);
                }
                let grey = [160, 160, 165, a];
                draw_text(pm, font, "z", cx + 26.0 * s, cy + 2.0 * s, 9.0 * s, grey, 12.0 * s);
                if self.zz_frame {
                    draw_text(pm, font, "Z", cx + 34.0 * s, cy - 5.0 * s, 11.0 * s, grey, 14.0 * s);
                }
            }
            _ if self.settings.show_clock => {
                let (hour, minute, _, _) = local_time();
                let text = fmt_clock(hour, minute, self.settings.clock_24h);
                draw_text_centered(pm, self.heading_font().unwrap_or(font), &text, cx, (self.closed_size().1 as f32 / 2.0 + 5.0) * s, 13.0 * s, [222, 222, 230, a], fw);
            }
            _ => {}
        }
    }
}
