//! The work between events: timers, alerts, view changes and the next wake-up.

use crate::*;

const PROGRESS_TICK: Duration = Duration::from_millis(500);

/// How often the timer's ring is redrawn while it is on screen and running, so it empties smoothly.
const RING_TICK: Duration = Duration::from_millis(16);

impl App {
    /// The settings window: its timers and the frames it asks for. Gives when it wants to be woken next.
    pub(super) fn panel_tick(&mut self, now: Instant) -> Option<Instant> {
        let mut panel_wake = None;
        let frame = self.pacing.frame;
        if let Some(p) = &mut self.panel {
            if p.reset_armed.is_some_and(|t| now >= t) {
                p.reset_armed = None;
                p.redraw();
            }
            if p.cleared.is_some_and(|(_, t)| now >= t) {
                p.cleared = None;
                p.redraw();
            }
            if p.busy {
                // One frame for each frame of the screen, not one straight after the other: the window had been
                // drawing as fast as it could while a control settled, keeping the thread busy for no gain.
                let due = p.last_frame() + frame;
                if now >= due {
                    p.redraw();
                    panel_wake = Some(now + frame);
                } else {
                    panel_wake = Some(due);
                }
            } else if let Some(t) = p.reset_armed {
                panel_wake = Some(t);
            }
            if let Some((_, t)) = p.cleared {
                panel_wake = Some(panel_wake.map_or(t, |w| w.min(t)));
            }
        }
        panel_wake
    }

    /// The mouse entering or leaving takes effect, and the notch gets a moment to take it back when its sides pull in.
    pub(super) fn hover_due(&mut self, now: Instant) {
        if self.pointer.enter_at.is_some_and(|t| now >= t) {
            self.pointer.enter_at = None;
            self.pointer.hovered = true;
            self.last_tick = now;
        }
        if self.pointer.leave_at.is_some_and(|t| now >= t) {
            self.pointer.leave_at = None;
            self.pointer.hovered = false;
            self.last_tick = now;
        }

        // The notch has finished changing width. If the mouse is now outside it (closing the lyrics pulls
        // the sides in), give it a moment to come back before the notch closes.
        if self.lyric.settle && !self.animating() {
            self.lyric.settle = false;
            if self.pointer.hovered && !self.cursor_over_window() {
                self.pointer.leave_at = Some(now + LYRIC_GRACE);
            }
        }
    }

    /// The timer, to-do reminders, water, chimes and the notices that are due.
    pub(super) fn alerts_due(&mut self, now: Instant) {
        if self.timer.tick(now) {
            self.notices.alert_msg = TIMES_UP.into();
            self.notices.alert_until = Some(now + self.alarm_for());
            self.notices.next_chime = Some(now + self.alarm_every());
            self.view = View::Timer;
            self.last_tick = now;
            self.play_alarm();
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
        if let Some(text) = self.todo.take_due(todo::unix_now()) {
            let short: String = text.chars().take(40).collect();
            self.notices.alert_msg = format!("Reminder: {short}");
            self.notices.alert_until = Some(now + ALERT);
            self.view = View::Todo;
            self.last_tick = now;
            self.play_slot(sound::Slot::Todo);
        }
        // Copied items are forgotten after the time the user chose.
        if let Some(keep) = self.settings.clip_keep.duration()
            && self.clip.history.expire(now, keep)
            && self.view == View::Clipboard
            && self.spring.progress > 0.0
            && let Some(w) = &self.window
        {
            w.request_redraw();
        }
        self.check_prayer();
        self.water.roll(today());
        if self.settings.water && self.water.due(now) {
            self.notices.alert_msg = "Time to drink some water!".into();
            self.notices.alert_until = Some(now + ALERT);
            self.view = View::Water;
            self.last_tick = now;
            self.play_slot(sound::Slot::Water);
        }
        if self.notices.next_chime.is_some_and(|t| now >= t) {
            if self.notices.alert_until.is_some() && self.notices.alert_msg == TIMES_UP {
                self.play_alarm();
                self.notices.next_chime = Some(now + self.alarm_every());
            } else {
                self.notices.next_chime = None;
            }
        }
        if self.notices.alert_until.is_some_and(|t| now >= t) {
            if self.notices.alert_msg == TIMES_UP {
                sound::stop_file(sound::Slot::Timer);
            }
            self.notices.alert_until = None;
            self.last_tick = now;
        }
        if self.notices.until.is_some_and(|t| now >= t) {
            self.notices.until = None;
            self.last_tick = now;
        }
        if self.clip.flash.as_ref().is_some_and(|(_, t)| now >= *t) {
            self.clip.flash = None;
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
    }

    /// A change of view starts its slide, and a change of the tuck goal wakes the animation.
    pub(super) fn sync_view_switch(&mut self, now: Instant) {
        if self.view != self.switch.shown {
            // Moving between Music and its lyrics is the same page; any other change is a transition in
            // which the old view slides out as the new one slides in from the side of its tab.
            let tab = |v: View| self.visible_tabs().iter().position(|(_, t)| *t == if v == View::Lyrics { View::Player } else { v });
            let (from, to) = (tab(self.switch.shown), tab(self.view));
            if from != to || from.is_none() {
                self.switch.prev = Some(self.switch.shown);
                self.switch.dir = match (from, to) {
                    (Some(a), Some(b)) if b < a => -1.0,
                    _ => 1.0,
                };
                self.switch.t = 0.0;
                self.switch.v = 0.0;
            }
            self.switch.shown = self.view;
            self.last_tick = now;
        }
        let goal = self.tuck_target();
        if goal != self.spring.tuck_goal {
            self.spring.tuck_goal = goal;
            self.last_tick = now;
        }
    }

    /// Redraws that are due while nothing animates, and the moment the loop should wake next.
    pub(super) fn next_wake(&mut self, now: Instant, panel_wake: Option<Instant>) -> Option<Instant> {
        let mut wake: Option<Instant> = None;
        self.companion_tick(now, &mut wake);
        let mut at = |t: Instant| earliest(&mut wake, t);

        // Periodic refresh for the progress bar / countdown. Only redraw when due, since a redraw
        // itself triggers another about_to_wait.
        let expanded = self.spring.progress >= 1.0;
        if (expanded && (self.media.playing || self.edit.on || self.timer_entry.field.is_some() || matches!(self.view, View::Water | View::Todo))) || self.timer.running() {
            let tick = if expanded && self.view == View::Timer && self.timer.running() { RING_TICK } else { PROGRESS_TICK };
            let due = self.last_ui_tick + tick;
            if now >= due {
                self.last_ui_tick = now;
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
                at(now + tick);
            } else {
                at(due);
            }
        }
        // The bars on the closed notch move a few times a second while a song plays.
        if self.bars_running() {
            let (tick, due) = (self.music_tick(), self.music.bars_at + self.music_tick());
            if now >= due {
                self.music.bars_at = now;
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
                at(now + tick);
            } else {
                at(due);
            }
        }
        if let Some(wait) = self.prayer_minute_tick() {
            at(now + wait);
        }
        if let Some(t) = self.prayer_wake(now) {
            at(t);
        }
        if let Some(t) = self.stats_tick(now) {
            at(t);
        }
        // Wake exactly when the next lyric line starts, so the glide begins on time.
        if expanded
            && self.view == View::Lyrics
            && self.media.playing
            && let LyricsState::Found(lines) = &self.lyric.state
        {
            let next = self.current_line().map_or(0, |c| c + 1);
            if let Some(line) = lines.get(next) {
                at(now + line.at.saturating_sub(self.position()));
            }
        }
        if let Some(d) = self.timer.deadline() {
            at(d);
        }
        if let Some(d) = self.water.deadline().filter(|_| self.settings.water) {
            at(d);
        }
        if let Some(due) = self.todo.next_due() {
            at(now + Duration::from_secs(due.saturating_sub(todo::unix_now())));
        }
        if let Some(t) = self.notices.alert_until {
            at(t);
        }
        if let Some(t) = self.notices.next_chime {
            at(t);
        }
        if let Some(t) = panel_wake {
            at(t);
        }
        for t in [self.pointer.enter_at, self.pointer.leave_at].into_iter().flatten() {
            at(t);
        }
        if let Some(t) = self.notices.until {
            at(t);
        }
        if let Some((_, t)) = self.clip.flash {
            at(t);
        }
        if let Some(t) = self.settings.clip_keep.duration().and_then(|keep| self.clip.history.next_expiry(keep)) {
            at(t);
        }
        wake
    }
}
