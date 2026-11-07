//! The eyes on the closed notch and their idle animation: when they blink and look about, the sleeping eyes and the
//! greeting. How an eye is drawn is in `eyes.rs`.

use crate::*;

pub(crate) const GREETING: Duration = Duration::from_millis(1600);

/// How long the closed notch shows the eyes after an Eye style is picked, so the choice can be seen.
const PREVIEW: Duration = Duration::from_millis(3500);

/// Where the eyes can look, one pair of (across, down) after another; each blink picks the next.
const LOOKS_X: [f64; 7] = [-1.0, 0.7, 0.0, 1.0, -0.4, 0.4, 0.0];
const LOOKS_Y: [f64; 5] = [0.0, -0.5, 0.3, 0.0, 0.5];

/// The clock on the closed notch: the time in white, and on a 12-hour clock AM or PM small and dim beside it.
const CLOCK_PX: f32 = 14.0;
const SUFFIX_PX: f32 = 9.5;
const SUFFIX_GAP: f32 = 3.5;

/// The pieces of the clock and how wide each is, so the whole can be centred and the notch made wide enough.
pub(crate) struct ClockText {
    pub time: String,
    pub suffix: Option<&'static str>,
    pub time_w: f32,
    pub suffix_w: f32,
}

impl ClockText {
    /// The clock for `hour` and `minute`, measured in the notch's fonts.
    pub(crate) fn new(font: &Font, hour: u32, minute: u32, clock_24h: bool) -> Self {
        let (time, suffix) = if clock_24h {
            (format!("{hour:02}:{minute:02}"), None)
        } else {
            (format!("{}:{minute:02}", if hour.is_multiple_of(12) { 12 } else { hour % 12 }), Some(if hour < 12 { "AM" } else { "PM" }))
        };
        let (time_w, suffix_w) = (text_width(font, &time, CLOCK_PX), suffix.map_or(0.0, |s| text_width(font, s, SUFFIX_PX)));
        Self { time, suffix, time_w, suffix_w }
    }

    /// The widest the clock gets for this many digits, so the notch does not change width as the minutes go by.
    pub(crate) fn widest(font: &Font, clock_24h: bool) -> f32 {
        let time_w = text_width(font, "00:00", CLOCK_PX);
        if clock_24h { time_w } else { time_w + SUFFIX_GAP + text_width(font, "PM", SUFFIX_PX) }
    }

    pub(crate) fn width(&self) -> f32 {
        self.time_w + if self.suffix.is_some() { SUFFIX_GAP + self.suffix_w } else { 0.0 }
    }
}

/// The closed notch's eyes: when they greet or preview, and the blink, gaze and glance clocks.
pub(crate) struct Eyes {
    /// The closed notch shows the greeting eyes until then, after the notch wakes from dozing.
    pub(crate) greeting_until: Option<Instant>,
    /// The closed notch shows the eyes until then, after an Eye style is picked in the settings window.
    pub(crate) preview_until: Option<Instant>,
    /// When the eyes next blink.
    pub(crate) next_blink: Instant,
    /// When the blink in progress began (it lasts `eyes::blink_len`), if the eyes are blinking.
    pub(crate) blink_since: Option<Instant>,
    /// Where the eyes look now, how fast that is changing, and where they are turning to (across, down; -1 to 1).
    pub(crate) look: (f64, f64),
    /// How fast `look` is changing (the spring's speed).
    pub(crate) look_v: (f64, f64),
    /// Where the eyes are turning to.
    pub(crate) look_goal: (f64, f64),
    /// When the eyes next look somewhere without a blink, and how many times they have.
    pub(crate) next_glance: Instant,
    /// How many times the eyes have looked somewhere without a blink (it varies the next glance).
    pub(crate) glances: u32,
    /// How many times the eyes have blinked (it varies the pause and the direction of the next look).
    pub(crate) blink_count: u32,
    /// When the sleeping eyes' "z" next changes, and which of its two frames shows.
    pub(crate) next_zz: Instant,
    pub(crate) zz_frame: bool,
}

impl Default for Eyes {
    fn default() -> Self {
        Self {
            greeting_until: None,
            preview_until: None,
            next_blink: Instant::now(),
            blink_since: None,
            look: (0.0, 0.0),
            look_v: (0.0, 0.0),
            look_goal: (0.0, 0.0),
            next_glance: Instant::now(),
            glances: 0,
            blink_count: 0,
            next_zz: Instant::now(),
            zz_frame: false,
        }
    }
}

impl App {
    /// Schedule and trigger the redraws the closed notch needs: clock minute changes, blinking
    /// eyes, dozing animation and the greeting. Does nothing when something else is showing.
    /// Whether the awake eyes are drawn on the closed notch right now: while you are working they take
    /// the place of the clock (so only when the clock is off), and after a while without use they come
    /// from "Idle eyes".
    fn eyes_open(&self) -> bool {
        match self.vis.presence {
            Presence::Active => self.settings.eyes && !self.settings.show_clock,
            Presence::Idle => self.settings.idle_eyes,
            Presence::Dozing => false,
        }
    }

    /// Whether the middle of the closed notch shows eyes (open, asleep, the greeting, or the preview) and not the clock.
    pub(crate) fn centre_is_eyes(&self) -> bool {
        let greeting = self.settings.idle_eyes && self.eyes.greeting_until.is_some();
        let asleep = matches!(self.vis.presence, Presence::Dozing) && self.settings.idle_eyes;
        self.previewing() || greeting || asleep || (!matches!(self.vis.presence, Presence::Dozing) && self.eyes_open())
    }

    /// Whether the closed notch is showing the eyes as a preview of the Eye style.
    pub(crate) fn previewing(&self) -> bool {
        self.eyes.preview_until.is_some_and(|t| Instant::now() < t)
    }

    /// Show the eyes on the closed notch for a moment, in the style that was just picked, whatever else the closed
    /// notch shows (the clock, or nothing). The notch comes down for it if it is tucked away.
    pub(crate) fn preview_eyes(&mut self) {
        let now = Instant::now();
        self.eyes.preview_until = Some(now + PREVIEW);
        self.eyes.blink_since = None;
        self.eyes.next_blink = now + Duration::from_millis(1400);
        self.eyes.next_glance = now + Duration::from_millis(900);
        self.last_tick = now;
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// What the eyes are doing right now: how open they are, where they look and how far a blink has got.
    pub(crate) fn eye_pose(&self) -> eyes::Pose {
        let style = self.settings.eye_style;
        let blink = self.eyes.blink_since.map_or(0.0, |t| (t.elapsed().as_secs_f32() / eyes::blink_len(style).as_secs_f32()).clamp(0.0, 1.0));
        let open = if self.eyes.blink_since.is_some() { eyes::lid(style, blink) } else { 1.0 };
        eyes::Pose { open, look: (self.eyes.look.0 as f32, self.eyes.look.1 as f32), blink }
    }

    /// Whether the eyes are mid-blink or still turning to look somewhere, so frames are needed.
    pub(crate) fn eyes_moving(&self) -> bool {
        self.eyes.blink_since.is_some() || self.eyes.look != self.eyes.look_goal || self.eyes.look_v != (0.0, 0.0)
    }

    /// Move the eyes on by `dt` seconds: the blink ends when its time is up, and the gaze glides to where it is going
    /// with a spring (a small overshoot, like a quick glance).
    pub(crate) fn eyes_step(&mut self, dt: f64, speed: f64, bounce: bool) {
        if self.eyes.blink_since.is_some_and(|t| t.elapsed() >= eyes::blink_len(self.settings.eye_style)) {
            self.eyes.blink_since = None;
        }
        let zeta = if bounce { 0.7 } else { 1.0 };
        spring(&mut self.eyes.look.0, &mut self.eyes.look_v.0, self.eyes.look_goal.0, dt, 15.0 * speed, zeta);
        spring(&mut self.eyes.look.1, &mut self.eyes.look_v.1, self.eyes.look_goal.1, dt, 15.0 * speed, zeta);
    }

    /// The next place to look, different from the one before.
    fn next_look(&self) -> (f64, f64) {
        let n = self.eyes.blink_count as usize + self.eyes.glances as usize;
        let goal = (LOOKS_X[(n * 3 + 1) % LOOKS_X.len()], LOOKS_Y[(n * 2 + 3) % LOOKS_Y.len()]);
        if goal == self.eyes.look_goal { (-goal.0, goal.1) } else { goal }
    }

    /// Blink and look about now and then. A blink starts the animation (`eyes_step` ends it); a double blink comes
    /// every fifth time. Says when to wake for the next one.
    fn blink_step(&mut self, now: Instant, wake: &mut Option<Instant>, redraw: &mut bool) {
        if self.eyes.blink_since.is_some() {
            return;
        }
        if now >= self.eyes.next_blink {
            self.eyes.blink_since = Some(now);
            self.eyes.blink_count += 1;
            // The eyes turn while the lids are down, so they open already looking somewhere else.
            self.eyes.look_goal = self.next_look();
            let n = self.eyes.blink_count as u64;
            let gap = if n.is_multiple_of(5) { 140 } else { 2600 + (n * 1373) % 4200 };
            self.eyes.next_blink = now + eyes::blink_len(self.settings.eye_style) + Duration::from_millis(gap);
            self.eyes.next_glance = now + Duration::from_millis(1800 + (n * 911) % 2800);
            *redraw = true;
            return;
        }
        if now >= self.eyes.next_glance {
            self.eyes.glances += 1;
            self.eyes.look_goal = self.next_look();
            self.eyes.next_glance = now + Duration::from_millis(2200 + (self.eyes.glances as u64 * 733) % 3000);
            *redraw = true;
        }
        earliest(wake, self.eyes.next_blink.min(self.eyes.next_glance));
    }

    pub(crate) fn companion_tick(&mut self, now: Instant, wake: &mut Option<Instant>) {
        if self.vis.hidden || self.spring.tuck_t >= 1.0 || self.spring.progress > 0.0 || self.notices.until.is_some() || self.timer.running() {
            return;
        }
        let mut redraw = false;

        if let Some(until) = self.eyes.preview_until {
            if now >= until {
                self.eyes.preview_until = None;
                redraw = true;
            } else {
                // The preview: blink and look about like the eyes do, and wake when it is over to put the clock (or
                // nothing) back.
                self.blink_step(now, wake, &mut redraw);
                earliest(wake, until);
                if redraw && let Some(w) = &self.window {
                    w.request_redraw();
                }
                return;
            }
        }

        if let Some(g) = self.eyes.greeting_until {
            if now >= g {
                self.eyes.greeting_until = None;
                redraw = true;
            } else {
                earliest(wake, g);
                return;
            }
        }

        match self.vis.presence {
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
            Presence::Active | Presence::Idle if self.eyes_open() => self.blink_step(now, wake, &mut redraw),
            Presence::Dozing if self.settings.idle_eyes => {
                if now >= self.eyes.next_zz {
                    self.eyes.zz_frame = !self.eyes.zz_frame;
                    self.eyes.next_zz = now + Duration::from_millis(1100);
                    redraw = true;
                }
                earliest(wake, self.eyes.next_zz);
            }
            _ => {}
        }

        if redraw && let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// What the closed notch shows when nothing else needs it: eyes when idle, otherwise the clock.
    pub(crate) fn draw_companion(&self, pm: &mut Pixmap, s: f32, alpha: f32, fw: f32) {
        let Some(font) = &self.font else { return };
        let a = (alpha * 255.0) as u8;
        let (cx, cy) = (fw / 2.0, self.closed_size().1 as f32 / 2.0 * s);
        let (left, right) = (cx - 13.0 * s, cx + 13.0 * s);
        let eyes_on = self.settings.idle_eyes;
        let style = self.settings.eye_style;
        let white = {
            let c = eye_colour(style);
            [c[0], c[1], c[2], a]
        };

        if eyes_on && self.eyes.greeting_until.is_some() {
            for x in [left, right] {
                eyes::draw_resting(pm, style, x, cy, s, a, true);
            }
            draw_text(pm, font, "hi!", cx + 28.0 * s, cy + 4.0 * s, 11.0 * s, white, 30.0 * s);
            return;
        }

        match self.vis.presence {
            _ if self.previewing() => {
                let pose = self.eye_pose();
                for x in [left, right] {
                    eyes::draw_eye(pm, style, x, cy, s, a, pose);
                }
            }
            Presence::Active | Presence::Idle if self.eyes_open() => {
                let pose = self.eye_pose();
                for x in [left, right] {
                    eyes::draw_eye(pm, style, x, cy, s, a, pose);
                }
            }
            Presence::Dozing if eyes_on => {
                for x in [left, right] {
                    eyes::draw_resting(pm, style, x, cy, s, a, false);
                }
                let grey = [160, 160, 165, a];
                draw_text(pm, font, "z", cx + 26.0 * s, cy + 2.0 * s, 9.0 * s, grey, 12.0 * s);
                if self.eyes.zz_frame {
                    draw_text(pm, font, "Z", cx + 34.0 * s, cy - 5.0 * s, 11.0 * s, grey, 14.0 * s);
                }
            }
            _ if self.settings.show_clock => {
                let (hour, minute, _, _) = local_time();
                let heading = self.heading_font().unwrap_or(font);
                let clock = ClockText::new(heading, hour, minute, self.settings.clock_24h);
                let baseline = (self.closed_size().1 as f32 / 2.0 + 5.2) * s;
                let x = cx - clock.width() * s / 2.0;
                draw_text(pm, heading, &clock.time, x, baseline, CLOCK_PX * s, [236, 236, 244, a], fw);
                if let Some(suffix) = clock.suffix {
                    draw_text(pm, heading, suffix, x + (clock.time_w + SUFFIX_GAP) * s, baseline, SUFFIX_PX * s, [150, 152, 170, a], fw);
                }
            }
            _ => {}
        }
    }
}
