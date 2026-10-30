//! The Timer view, its time editing and the alarm.

use crate::*;

const KEY_TIMER_PLAY: u64 = 0x4000_0000_0000_0002;

// Ring and controls shared by the timer and water views, in logical pixels.
pub(crate) const RING: (f32, f32, f32) = (104.0, 100.0, 48.0); // center x, center y, radius

pub(crate) const RING_W: f32 = 7.0;

// Timer view: preset chips on top, plus / minus buttons above and below the hours, minutes and
// seconds, then start / pause and reset side by side.
pub(crate) const TIMER_COLS: [f32; 3] = [237.0, 293.0, 349.0];

pub(crate) const TIMER_PLUS_Y: f32 = 64.0;

const TIMER_VALUE_Y: f32 = 91.0; // baseline

pub(crate) const TIMER_MINUS_Y: f32 = 108.0;

const TIMER_BTN_R: f32 = 10.0;

const TIMER_CTRL_Y: f32 = 140.0;

const TIMER_START_X: f32 = 263.0;

/// The red Stop button shown while the timer's alarm rings.
const STOP_BUTTON: (f32, f32, f32, f32) = (TIMER_COLS[1] - 62.0, TIMER_CTRL_Y - 16.0, 124.0, 32.0);

const TIMER_RESET_X: f32 = 323.0;

const TIMER_CHIP_W: f32 = 34.0;

pub(crate) const CHIP_GAP: f32 = 4.0;

pub(crate) const ALERT: Duration = Duration::from_secs(6);

/// The banner text when the timer ends; the timer view recognises it by this.
pub(crate) const TIMES_UP: &str = "Time's up!";

/// The area of one timer number that can be clicked to type a value.
fn timer_value_rect(i: usize) -> (f32, f32, f32, f32) {
    (TIMER_COLS[i] - 21.0, TIMER_VALUE_Y - 17.0, 40.0, 24.0)
}

/// Preset chip, centered over the right-hand column of the timer view.
fn timer_chip_rect(i: usize) -> (f32, f32, f32, f32) {
    let n = timer::PRESETS.len() as f32;
    let total = n * TIMER_CHIP_W + (n - 1.0) * CHIP_GAP;
    (TIMER_COLS[1] - total / 2.0 + i as f32 * (TIMER_CHIP_W + CHIP_GAP), 34.0, TIMER_CHIP_W, 18.0)
}

/// "5m", "30m" or "1h" for a preset length.
fn preset_label(secs: u64) -> String {
    if secs.is_multiple_of(3600) { format!("{}h", secs / 3600) } else { format!("{}m", secs / 60) }
}

impl App {
    /// How long the timer's alarm keeps ringing if nobody stops it, and how often it sounds again.
    pub(crate) fn alarm_for(&self) -> Duration {
        Duration::from_secs(self.settings.alarm_for as u64)
    }

    pub(crate) fn alarm_every(&self) -> Duration {
        Duration::from_secs(self.settings.alarm_every as u64)
    }

    pub(crate) fn draw_timer(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let Some(font) = &self.font else { return };
        let heading = self.heading_font().unwrap_or(font);
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [165, 165, 175, a]);
        let rgb = self.accent(View::Timer);
        let accent = [rgb[0], rgb[1], rgb[2], a];
        let light = gfx::mix(accent, white, 0.35);
        let now = Instant::now();
        let idle = self.timer.is_idle();
        let finished = self.alert_until.is_some() && self.alert_msg == TIMES_UP;

        // Quick choices for the length, or a reminder of the keys while a number is being typed.
        if self.timer_field.is_some() && !finished {
            draw_text_centered(pm, font, "Type digits \u{00B7} Enter to set \u{00B7} Tab for next", TIMER_COLS[1] * s, 47.0 * s, 10.0 * s, grey, 200.0 * s);
        } else if !finished {
            for (i, secs) in timer::PRESETS.iter().enumerate() {
                let r = timer_chip_rect(i);
                let on = idle && self.timer.total().as_secs() == *secs;
                draw_chip(pm, font, r, &preset_label(*secs), on, if idle { self.hot_t(r, s) } else { 0.0 }, rgb, s, alpha);
            }
        }

        // Left: a ring that empties as the time runs down, with the time inside.
        let (rx, ry, rr) = RING;
        gfx::glow(pm, rx * s, ry * s, (rr + 28.0) * s, [rgb[0], rgb[1], rgb[2], (alpha * if self.timer.running() { 44.0 } else { 26.0 }) as u8], None);
        stroke_arc(pm, rx * s, ry * s, rr * s, RING_W * s, 1.0, [255, 255, 255, (alpha * 22.0) as u8]);
        gfx::stroke_arc_gradient(pm, rx * s, ry * s, rr * s, RING_W * s, self.timer.fraction_left(now), light, accent);
        let time = fmt_remaining(self.timer.remaining(now));
        let size = if time.len() > 5 { 22.0 } else { 30.0 };
        draw_text_centered(pm, heading, &time, rx * s, (ry + size / 3.0) * s, size * s, white, (rr * 2.0 - 8.0) * s);
        let caption = if self.timer.running() {
            "Running"
        } else if self.timer.paused() {
            "Paused"
        } else {
            "Ready"
        };
        draw_text_centered(pm, font, caption, rx * s, (ry + 28.0) * s, 10.0 * s, if idle { grey } else { light }, (rr * 2.0 - 8.0) * s);

        // Right: the length as hours, minutes and seconds, each with a plus above and a minus below.
        if finished {
            draw_text_centered(pm, heading, TIMES_UP, TIMER_COLS[1] * s, 92.0 * s, 22.0 * s, accent, 190.0 * s);
            draw_text_centered(pm, font, "Press Stop to silence it", TIMER_COLS[1] * s, 112.0 * s, 11.0 * s, grey, 190.0 * s);
        } else {
            let (h, m, sec) = self.timer.parts();
            for (i, unit) in Unit::ALL.iter().enumerate() {
                let (x, value) = (TIMER_COLS[i], [h, m, sec][i]);
                for (y, plus) in [(TIMER_PLUS_Y, true), (TIMER_MINUS_Y, false)] {
                    let hot = if idle { self.hot_t((x - TIMER_BTN_R, y - TIMER_BTN_R, TIMER_BTN_R * 2.0, TIMER_BTN_R * 2.0), s) } else { 0.0 };
                    let ink = if idle { white } else { [90, 90, 100, a] };
                    draw_round_button(pm, x * s, y * s, TIMER_BTN_R * s, hot, idle, alpha);
                    fill_rect(pm, (x - 3.5) * s, (y - 0.75) * s, 7.0 * s, 1.5 * s, ink);
                    if plus {
                        fill_rect(pm, (x - 0.75) * s, (y - 3.5) * s, 1.5 * s, 7.0 * s, ink);
                    }
                }
                let ink = if idle { white } else { [120, 120, 130, a] };
                let area = timer_value_rect(i);
                let typing = self.timer_field == Some(i);
                if typing {
                    gfx::gradient_rounded(pm, area.0 * s, area.1 * s, area.2 * s, area.3 * s, 8.0 * s, [rgb[0], rgb[1], rgb[2], (alpha * 64.0) as u8], [rgb[0], rgb[1], rgb[2], (alpha * 28.0) as u8], false);
                    gfx::stroke_rounded(pm, area.0 * s + 0.5 * s, area.1 * s + 0.5 * s, area.2 * s - s, area.3 * s - s, 8.0 * s, s, [rgb[0], rgb[1], rgb[2], (alpha * 170.0) as u8]);
                } else if idle {
                    let glow = self.hot_t(area, s);
                    if glow > 0.01 {
                        fill_rounded(pm, area.0 * s, area.1 * s, area.2 * s, area.3 * s, 8.0 * s, [255, 255, 255, (alpha * 16.0 * glow) as u8]);
                    }
                }
                // While typing, the digits so far replace the number (the old one stays until the first digit).
                let shown = if typing && !self.timer_input.is_empty() { self.timer_input.clone() } else { format!("{value:02}") };
                let ink = if typing && self.timer_input.is_empty() { [150, 150, 165, a] } else { ink };
                draw_text_centered(pm, heading, &shown, (x - 5.0) * s, TIMER_VALUE_Y * s, 22.0 * s, ink, 40.0 * s);
                if typing && (self.edit_started.elapsed().as_millis() / 500).is_multiple_of(2) {
                    let cx = (x - 5.0) * s + text_width(heading, &shown, 22.0 * s) / 2.0 + 1.5 * s;
                    fill_rect(pm, cx, (TIMER_VALUE_Y - 16.0) * s, 1.5 * s, 20.0 * s, accent);
                }
                draw_text(pm, font, unit.letter(), (x + 11.0) * s, (TIMER_VALUE_Y - 1.0) * s, 11.0 * s, grey, 12.0 * s);
            }
        }

        // While the alarm rings: one big red Stop button, like a phone's timer (only this button stops it).
        if finished {
            let r = STOP_BUTTON;
            let hot = self.hot_t(r, s);
            let red = [255, 82, 72];
            gfx::glow(pm, (r.0 + r.2 / 2.0) * s, (r.1 + r.3 / 2.0) * s, 70.0 * s, [red[0], red[1], red[2], (alpha * pick(70.0, 44.0, hot)) as u8], None);
            let top = gfx::mix([red[0], red[1], red[2], a], white, pick(0.28, 0.12, hot));
            gfx::gradient_rounded(pm, r.0 * s, r.1 * s, r.2 * s, r.3 * s, r.3 * s / 2.0, top, [red[0], red[1], red[2], a], false);
            let (cx, cy) = (r.0 + 34.0, r.1 + r.3 / 2.0);
            fill_rounded(pm, (cx - 5.0) * s, (cy - 5.0) * s, 10.0 * s, 10.0 * s, 2.0 * s, white);
            draw_text_centered(pm, heading, "Stop", (r.0 + r.2 / 2.0 + 10.0) * s, (cy + 5.0) * s, 15.0 * s, white, 70.0 * s);
            return;
        }

        // Reset, and the big start / pause button.
        let reset = (TIMER_RESET_X - 26.0, TIMER_CTRL_Y - 11.0, 52.0, 22.0);
        let reset_hot = self.hot_t(reset, s);
        if reset_hot > 0.01 {
            fill_rounded(pm, reset.0 * s, reset.1 * s, reset.2 * s, reset.3 * s, 11.0 * s, [255, 255, 255, (alpha * 18.0 * reset_hot) as u8]);
        }
        draw_text_centered(pm, font, "Reset", TIMER_RESET_X * s, (TIMER_CTRL_Y + 4.0) * s, 12.0 * s, pick(white, grey, reset_hot), 60.0 * s);
        let big = self.hot_t((TIMER_START_X - 20.0, TIMER_CTRL_Y - 20.0, 40.0, 40.0), s);
        draw_big_button(pm, TIMER_START_X * s, TIMER_CTRL_Y * s, 18.0 * s, big, rgb, alpha);
        let pause = self.glows.follow(KEY_TIMER_PLAY, if self.timer.running() { 1.0 } else { 0.0 }, 30.0, 0.85) as f32;
        draw_play_pause(pm, TIMER_START_X * s, TIMER_CTRL_Y * s, s * 0.8, [0, 0, 0, a], pause);
    }

    /// Start typing one of the timer's numbers: take the keyboard, remembering where it came from.
    fn begin_timer_edit(&mut self, field: usize) {
        use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
        if self.timer_field.is_none() && self.prev_foreground.is_none() {
            let fg = unsafe { GetForegroundWindow() };
            self.prev_foreground = (!fg.is_invalid()).then_some(fg);
        }
        self.timer_field = Some(field);
        self.timer_input.clear();
        self.edit_started = Instant::now();
        if let Some(w) = &self.window {
            w.focus_window();
            w.request_redraw();
        }
    }

    /// Stop typing a timer number, keeping what was typed if `keep`, and give the keyboard back.
    pub(crate) fn end_timer_edit(&mut self, keep: bool) {
        use windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;
        if keep {
            self.apply_timer_input();
        }
        self.timer_field = None;
        self.timer_input.clear();
        self.last_tick = Instant::now();
        if let Some(h) = self.prev_foreground.take() {
            let _ = unsafe { SetForegroundWindow(h) };
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Put the digits typed so far into the timer's length.
    fn apply_timer_input(&mut self) {
        if let (Some(field), Ok(value)) = (self.timer_field, self.timer_input.parse::<u64>()) {
            self.timer.set_part(Unit::ALL[field], value);
        }
        self.timer_input.clear();
    }

    /// Carry on typing in the next (`dir` = 1) or previous (-1) number.
    fn move_timer_field(&mut self, dir: isize) {
        let Some(field) = self.timer_field else { return };
        self.apply_timer_input();
        self.timer_field = Some((field as isize + dir).rem_euclid(Unit::ALL.len() as isize) as usize);
        self.edit_started = Instant::now();
    }

    /// A key pressed while a timer number is being typed.
    pub(crate) fn timer_key(&mut self, event: &winit::event::KeyEvent) {
        let Some(field) = self.timer_field else { return };
        match &event.logical_key {
            Key::Named(NamedKey::Enter) => self.end_timer_edit(true),
            Key::Named(NamedKey::Escape) => self.end_timer_edit(false),
            Key::Named(NamedKey::Backspace) => {
                self.timer_input.pop();
            }
            Key::Named(NamedKey::Tab) | Key::Named(NamedKey::ArrowRight) => self.move_timer_field(1),
            Key::Named(NamedKey::ArrowLeft) => self.move_timer_field(-1),
            Key::Named(key @ (NamedKey::ArrowUp | NamedKey::ArrowDown)) => {
                // Up and down change the number by one, a second at a time.
                self.apply_timer_input();
                self.timer.adjust(Unit::ALL[field], if *key == NamedKey::ArrowUp { 1 } else { -1 });
            }
            _ => {
                if let Some(text) = &event.text {
                    for c in text.chars() {
                        if c.is_ascii_digit() {
                            if self.timer_input.len() < 2 {
                                self.timer_input.push(c);
                            }
                        } else if matches!(c, ':' | '.' | ',' | ' ') {
                            // Typing 1:30:00 moves along as you go.
                            self.move_timer_field(1);
                        }
                    }
                }
            }
        }
    }

    /// The timer's alarm: the sound file chosen in the settings window, or the built-in chime when there
    /// is none (or it can't be played). A long file that is still playing is left to finish.
    pub(crate) fn play_alarm(&self) {
        if !self.settings.sound {
            return;
        }
        if let Some(path) = &self.settings.alarm_sound
            && sound::play_file(path, self.settings.volume) {
                return;
            }
        sound::play(sound::Chime::TimerDone, self.settings.volume);
    }

    pub(crate) fn play_sound(&self, chime: sound::Chime) {
        if self.settings.sound {
            sound::play(chime, self.settings.volume);
        }
    }

    /// A click in the Timer view. (`cx`, `cy`) is the spot in the view's own coordinates.
    pub(crate) fn click_timer(&mut self, cx: f32, cy: f32, window: &Window) {
        let now = Instant::now();
        let in_label = |x: f32| (cx - x).abs() <= 26.0 && (cy - TIMER_CTRL_Y).abs() <= 14.0;
        let chip = (0..timer::PRESETS.len()).find(|&i| {
            let r = timer_chip_rect(i);
            cx >= r.0 && cx <= r.0 + r.2 && cy >= r.1 && cy <= r.1 + r.3
        });
        // Plus is above a number, minus below it.
        let step = Unit::ALL.iter().enumerate().find_map(|(i, unit)| {
            let x = TIMER_COLS[i];
            if (cx - x).hypot(cy - TIMER_PLUS_Y) <= TIMER_BTN_R + 2.0 {
                Some((*unit, 1))
            } else if (cx - x).hypot(cy - TIMER_MINUS_Y) <= TIMER_BTN_R + 2.0 {
                Some((*unit, -1))
            } else {
                None
            }
        });
        let value = (0..Unit::ALL.len()).find(|&i| {
            let r = timer_value_rect(i);
            cx >= r.0 && cx <= r.0 + r.2 && cy >= r.1 && cy <= r.1 + r.3
        });
        if let Some(i) = chip {
            if !self.timer.set(timer::PRESETS[i]) {
                return;
            }
        } else if let Some(i) = value.filter(|_| self.timer.is_idle() && step.is_none()) {
            // Click a number to type it.
            self.begin_timer_edit(i);
        } else if let Some((unit, dir)) = step {
            if !self.timer.adjust(unit, dir) {
                return;
            }
        } else if (cx - TIMER_START_X).hypot(cy - TIMER_CTRL_Y) <= 18.0 {
            self.timer.toggle(now);
        } else if in_label(TIMER_RESET_X) {
            self.timer.reset();
        } else {
            return;
        }
        self.alert_until = None;
        window.request_redraw();
    }

    /// While the timer's alarm rings, only its Stop button stops it. True when the click was dealt with here: the alarm was stopped, or the click fell on a control that stays dead for now (the tab bar still works).
    pub(crate) fn click_alarm_guard(&mut self, cx: f32, cy: f32, ry: f32, bar_y: f32, window: &Window) -> bool {
        if self.alert_until.is_some() && self.alert_msg == TIMES_UP && self.view == View::Timer {
            if cx >= STOP_BUTTON.0 && cx <= STOP_BUTTON.0 + STOP_BUTTON.2 && cy >= STOP_BUTTON.1 && cy <= STOP_BUTTON.1 + STOP_BUTTON.3 {
                self.alert_until = None;
                self.next_chime = None;
                sound::stop_file();
                window.request_redraw();
                return true;
            }
            // The hidden controls under the Stop button stay dead; only the tab bar still works.
            if ry < bar_y - TAB_BOTTOM - 4.0 {
                return true;
            }
        }
        false
    }
}
