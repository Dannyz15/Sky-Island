//! Focus timer logic (Pomodoro-style: focus sessions with short and long breaks).
//! Pure state, with `now` passed in so it can be tested.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const FOCUS: Duration = Duration::from_secs(25 * 60);
const SHORT_BREAK: Duration = Duration::from_secs(5 * 60);
const LONG_BREAK: Duration = Duration::from_secs(15 * 60);
/// Focus sessions before a long break.
pub const CYCLE: u32 = 4;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Phase {
    Focus,
    ShortBreak,
    LongBreak,
}

impl Phase {
    pub const ALL: [Phase; 3] = [Phase::Focus, Phase::ShortBreak, Phase::LongBreak];

    /// Short name for the phase picker.
    pub fn chip_label(self) -> &'static str {
        match self {
            Phase::Focus => "Focus",
            Phase::ShortBreak => "Short",
            Phase::LongBreak => "Long",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Phase::Focus => "Focus",
            Phase::ShortBreak => "Short break",
            Phase::LongBreak => "Long break",
        }
    }
}

/// How long each phase lasts. Adjustable from the UI and saved between runs.
#[derive(Clone, Copy)]
pub struct Lengths {
    pub focus: Duration,
    pub short_break: Duration,
    pub long_break: Duration,
}

impl Default for Lengths {
    fn default() -> Self {
        Self { focus: FOCUS, short_break: SHORT_BREAK, long_break: LONG_BREAK }
    }
}

impl Lengths {
    fn get(&self, phase: Phase) -> Duration {
        match phase {
            Phase::Focus => self.focus,
            Phase::ShortBreak => self.short_break,
            Phase::LongBreak => self.long_break,
        }
    }

    /// Step and allowed range, in minutes, for adjusting a phase.
    fn step_and_range(phase: Phase) -> (u64, u64, u64) {
        match phase {
            Phase::Focus => (5, 5, 90),
            Phase::ShortBreak => (1, 1, 30),
            Phase::LongBreak => (5, 5, 60),
        }
    }

    fn path() -> Option<PathBuf> {
        let dir = PathBuf::from(std::env::var_os("APPDATA")?).join("Sky-Island");
        fs::create_dir_all(&dir).ok()?;
        Some(dir.join("focus.json"))
    }

    pub fn load() -> Self {
        let mut out = Self::default();
        let Some(v) = Self::path()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        else {
            return out;
        };
        let mins = |k: &str, phase: Phase| {
            let (_, lo, hi) = Self::step_and_range(phase);
            v.get(k).and_then(Value::as_u64).filter(|m| (lo..=hi).contains(m)).map(|m| Duration::from_secs(m * 60))
        };
        out.focus = mins("focus", Phase::Focus).unwrap_or(out.focus);
        out.short_break = mins("short_break", Phase::ShortBreak).unwrap_or(out.short_break);
        out.long_break = mins("long_break", Phase::LongBreak).unwrap_or(out.long_break);
        out
    }

    pub fn save(&self) {
        if let Some(p) = Self::path() {
            let m = |d: Duration| d.as_secs() / 60;
            let body = json!({
                "focus": m(self.focus),
                "short_break": m(self.short_break),
                "long_break": m(self.long_break),
            });
            let _ = fs::write(p, body.to_string());
        }
    }
}

/// Focus sessions finished on one day.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Stats {
    /// The date these numbers belong to, as yyyymmdd.
    pub day: u32,
    pub sessions: u32,
    pub focused: Duration,
}

impl Stats {
    fn path() -> Option<PathBuf> {
        let dir = PathBuf::from(std::env::var_os("APPDATA")?).join("Sky-Island");
        fs::create_dir_all(&dir).ok()?;
        Some(dir.join("focus_stats.json"))
    }

    fn load() -> Self {
        let Some(v) = Self::path()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        else {
            return Self::default();
        };
        let get = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
        Self { day: get("day") as u32, sessions: get("sessions") as u32, focused: Duration::from_secs(get("focused_secs")) }
    }

    fn save(&self) {
        // Tests must not overwrite the user's real data.
        if cfg!(test) {
            return;
        }
        if let Some(p) = Self::path() {
            let body = json!({ "day": self.day, "sessions": self.sessions, "focused_secs": self.focused.as_secs() });
            let _ = fs::write(p, body.to_string());
        }
    }
}

/// "25m" or "1h 15m".
pub fn fmt_focused(d: Duration) -> String {
    let mins = d.as_secs() / 60;
    if mins >= 60 { format!("{}h {:02}m", mins / 60, mins % 60) } else { format!("{mins}m") }
}

pub struct FocusTimer {
    pub phase: Phase,
    pub lengths: Lengths,
    stats: Stats,
    /// Focus sessions finished so far (counts up forever; the cycle position is `completed % CYCLE`).
    pub completed: u32,
    /// Time left when the timer was last started or stopped.
    left: Duration,
    started: Option<Instant>,
}

impl FocusTimer {
    pub fn new() -> Self {
        let lengths = Lengths::load();
        Self { phase: Phase::Focus, lengths, stats: Stats::load(), completed: 0, left: lengths.focus, started: None }
    }

    #[cfg(test)]
    fn with_defaults() -> Self {
        let lengths = Lengths::default();
        Self { phase: Phase::Focus, lengths, stats: Stats::default(), completed: 0, left: lengths.focus, started: None }
    }

    pub fn running(&self) -> bool {
        self.started.is_some()
    }

    pub fn remaining(&self, now: Instant) -> Duration {
        match self.started {
            Some(t) => self.left.saturating_sub(now.saturating_duration_since(t)),
            None => self.left,
        }
    }

    /// Fraction of the current phase that has elapsed, 0.0 to 1.0.
    pub fn progress(&self, now: Instant) -> f32 {
        1.0 - self.remaining(now).as_secs_f32() / self.lengths.get(self.phase).as_secs_f32()
    }

    #[cfg(test)]
    /// Filled dots to show in the cycle indicator.
    pub fn dots(&self) -> u32 {
        match self.phase {
            Phase::LongBreak => CYCLE,
            _ => self.completed % CYCLE,
        }
    }

    pub fn toggle(&mut self, now: Instant) {
        if let Some(t) = self.started.take() {
            self.left = self.left.saturating_sub(now.saturating_duration_since(t));
        } else {
            self.started = Some(now);
        }
    }

    /// Lengthen or shorten the current phase by one step while stopped. Returns true if it changed.
    pub fn adjust(&mut self, steps: i32) -> bool {
        if self.running() {
            return false;
        }
        let (step, lo, hi) = Lengths::step_and_range(self.phase);
        let cur = self.lengths.get(self.phase).as_secs() / 60;
        let new = (cur as i64 + steps as i64 * step as i64).clamp(lo as i64, hi as i64) as u64;
        if new == cur {
            return false;
        }
        let d = Duration::from_secs(new * 60);
        match self.phase {
            Phase::Focus => self.lengths.focus = d,
            Phase::ShortBreak => self.lengths.short_break = d,
            Phase::LongBreak => self.lengths.long_break = d,
        }
        self.left = d;
        // Tests must not overwrite the user's real settings file.
        if !cfg!(test) {
            self.lengths.save();
        }
        true
    }

    /// Restart the current phase from its full length, stopped.
    pub fn reset(&mut self) {
        self.left = self.lengths.get(self.phase);
        self.started = None;
    }

    /// Start a new day's totals when the date has changed.
    pub fn roll(&mut self, today: u32) {
        if self.stats.day != today {
            self.stats = Stats { day: today, ..Stats::default() };
            self.stats.save();
        }
    }

    /// Sessions finished and time focused on `today`, zero if the saved numbers are from another day.
    pub fn stats_for(&self, today: u32) -> Stats {
        if self.stats.day == today { self.stats } else { Stats { day: today, ..Stats::default() } }
    }

    /// Jump straight to a phase, stopped, without counting anything as finished.
    pub fn select(&mut self, phase: Phase) {
        self.enter(phase);
    }

    /// If the running phase has run out, move to the next one (stopped) and return the finished phase.
    pub fn tick(&mut self, now: Instant) -> Option<Phase> {
        if !self.running() || !self.remaining(now).is_zero() {
            return None;
        }
        let finished = self.phase;
        let next = self.next_phase();
        if finished == Phase::Focus {
            self.completed += 1;
            self.stats.sessions += 1;
            self.stats.focused += self.lengths.focus;
            self.stats.save();
        }
        self.enter(next);
        Some(finished)
    }

    /// When the running phase ends, for scheduling a wake-up.
    pub fn deadline(&self, now: Instant) -> Option<Instant> {
        self.started.map(|_| now + self.remaining(now))
    }

    fn next_phase(&self) -> Phase {
        match self.phase {
            Phase::Focus if (self.completed + 1).is_multiple_of(CYCLE) => Phase::LongBreak,
            Phase::Focus => Phase::ShortBreak,
            _ => Phase::Focus,
        }
    }

    fn enter(&mut self, phase: Phase) {
        self.phase = phase;
        self.left = self.lengths.get(phase);
        self.started = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_down_and_pauses() {
        let t0 = Instant::now();
        let mut p = FocusTimer::with_defaults();
        assert!(!p.running());
        p.toggle(t0);
        assert_eq!(p.remaining(t0 + Duration::from_secs(60)), FOCUS - Duration::from_secs(60));
        p.toggle(t0 + Duration::from_secs(60));
        // Paused: time no longer passes.
        assert_eq!(p.remaining(t0 + Duration::from_secs(600)), FOCUS - Duration::from_secs(60));
    }

    #[test]
    fn finishing_focus_starts_short_break_stopped() {
        let t0 = Instant::now();
        let mut p = FocusTimer::with_defaults();
        p.toggle(t0);
        assert_eq!(p.tick(t0 + FOCUS - Duration::from_secs(1)), None);
        assert_eq!(p.tick(t0 + FOCUS), Some(Phase::Focus));
        assert_eq!(p.phase, Phase::ShortBreak);
        assert_eq!(p.completed, 1);
        assert!(!p.running());
    }

    #[test]
    fn fourth_focus_leads_to_long_break() {
        let t0 = Instant::now();
        let mut p = FocusTimer::with_defaults();
        for _ in 0..3 {
            p.toggle(t0);
            p.tick(t0 + FOCUS);
            assert_eq!(p.phase, Phase::ShortBreak);
            p.select(Phase::Focus);
            assert_eq!(p.phase, Phase::Focus);
        }
        p.toggle(t0);
        p.tick(t0 + FOCUS);
        assert_eq!(p.phase, Phase::LongBreak);
        assert_eq!(p.dots(), CYCLE);
    }

    #[test]
    fn adjust_steps_within_range_and_only_when_stopped() {
        let t0 = Instant::now();
        let mut p = FocusTimer::with_defaults();
        assert!(p.adjust(1));
        assert_eq!(p.lengths.focus, Duration::from_secs(30 * 60));
        assert_eq!(p.remaining(t0), Duration::from_secs(30 * 60));
        for _ in 0..40 {
            p.adjust(-1);
        }
        assert_eq!(p.lengths.focus, Duration::from_secs(5 * 60), "clamped at the minimum");
        p.toggle(t0);
        assert!(!p.adjust(1), "cannot adjust while running");
    }

    #[test]
    fn select_jumps_to_any_phase_and_stops() {
        let t0 = Instant::now();
        let mut p = FocusTimer::with_defaults();
        p.toggle(t0);
        p.select(Phase::LongBreak);
        assert_eq!(p.phase, Phase::LongBreak);
        assert!(!p.running());
        assert_eq!(p.remaining(t0), LONG_BREAK);
        assert_eq!(p.completed, 0);
        p.select(Phase::Focus);
        assert_eq!(p.phase, Phase::Focus);
    }

    #[test]
    fn stats_count_finished_focus_sessions_per_day() {
        const DAY1: u32 = 20261028;
        let t0 = Instant::now();
        let mut p = FocusTimer::with_defaults();
        p.roll(DAY1);
        assert_eq!(p.stats_for(DAY1).sessions, 0);

        p.toggle(t0);
        p.tick(t0 + FOCUS);
        assert_eq!(p.stats_for(DAY1).sessions, 1);
        assert_eq!(p.stats_for(DAY1).focused, FOCUS);

        // A break finishing is not a focus session.
        p.toggle(t0);
        p.tick(t0 + SHORT_BREAK);
        assert_eq!(p.stats_for(DAY1).sessions, 1);

        // Choosing a phase never counts, and a new day starts from zero.
        p.select(Phase::Focus);
        assert_eq!(p.stats_for(DAY1).sessions, 1);
        p.roll(DAY1 + 1);
        assert_eq!(p.stats_for(DAY1 + 1).sessions, 0);
        assert_eq!(p.stats_for(DAY1).sessions, 0, "numbers from another day are not shown");
    }

    #[test]
    fn focused_time_formatting() {
        assert_eq!(fmt_focused(Duration::from_secs(0)), "0m");
        assert_eq!(fmt_focused(Duration::from_secs(25 * 60)), "25m");
        assert_eq!(fmt_focused(Duration::from_secs(75 * 60)), "1h 15m");
        assert_eq!(fmt_focused(Duration::from_secs(125 * 60)), "2h 05m");
    }
}
