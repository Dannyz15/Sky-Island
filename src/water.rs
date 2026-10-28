//! Water intake tracker with periodic reminders. Pure state, with `now` and `today` passed in
//! so it can be tested. The day count resets when the date changes.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// Reminder choices shown in the UI, in minutes. `None` turns reminders off.
pub const INTERVALS: [Option<u32>; 4] = [Some(30), Some(60), Some(120), None];
const DEFAULT_GOAL: u32 = 8;
const GOAL_RANGE: (u32, u32) = (1, 20);

pub struct Water {
    pub count: u32,
    pub goal: u32,
    /// Index into `INTERVALS`.
    pub interval: usize,
    /// Date the count belongs to, as yyyymmdd.
    day: u32,
    next: Option<Instant>,
}

impl Water {
    /// Load saved state; the count only carries over if it was saved today.
    pub fn new(today: u32, now: Instant) -> Self {
        let saved = Self::path()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<Value>(&t).ok());
        let get = |k: &str| saved.as_ref().and_then(|v| v.get(k)).and_then(Value::as_u64).map(|n| n as u32);

        let mut w = Self::with(
            today,
            get("goal").filter(|g| (GOAL_RANGE.0..=GOAL_RANGE.1).contains(g)).unwrap_or(DEFAULT_GOAL),
            get("interval").map_or(1, |i| (i as usize).min(INTERVALS.len() - 1)),
            now,
        );
        if get("day") == Some(today) {
            w.count = get("count").unwrap_or(0);
        }
        w
    }

    fn with(today: u32, goal: u32, interval: usize, now: Instant) -> Self {
        let mut w = Self { count: 0, goal, interval, day: today, next: None };
        w.reschedule(now);
        w
    }

    fn path() -> Option<PathBuf> {
        let dir = PathBuf::from(std::env::var_os("APPDATA")?).join("Sky-Island");
        fs::create_dir_all(&dir).ok()?;
        Some(dir.join("water.json"))
    }

    fn save(&self) {
        // Tests must not overwrite the user's real data.
        if cfg!(test) {
            return;
        }
        if let Some(p) = Self::path() {
            let body = json!({
                "day": self.day,
                "count": self.count,
                "goal": self.goal,
                "interval": self.interval,
            });
            let _ = fs::write(p, body.to_string());
        }
    }

    fn reschedule(&mut self, now: Instant) {
        self.next = INTERVALS[self.interval].map(|m| now + Duration::from_secs(m as u64 * 60));
    }

    /// Count the reminder interval from now, for when the tracker is switched on again.
    pub fn restart(&mut self, now: Instant) {
        self.reschedule(now);
    }

    /// Start a new day's count when the date has changed.
    pub fn roll(&mut self, today: u32) {
        if today != self.day {
            self.day = today;
            self.count = 0;
            self.save();
        }
    }

    pub fn add(&mut self, now: Instant) {
        self.count += 1;
        // Having a drink pushes the next reminder back.
        self.reschedule(now);
        self.save();
    }

    pub fn undo(&mut self) {
        self.count = self.count.saturating_sub(1);
        self.save();
    }

    pub fn reset(&mut self) {
        self.count = 0;
        self.save();
    }

    pub fn adjust_goal(&mut self, steps: i32) {
        self.goal = (self.goal as i32 + steps).clamp(GOAL_RANGE.0 as i32, GOAL_RANGE.1 as i32) as u32;
        self.save();
    }

    pub fn set_interval(&mut self, index: usize, now: Instant) {
        self.interval = index.min(INTERVALS.len() - 1);
        self.reschedule(now);
        self.save();
    }

    pub fn reached(&self) -> bool {
        self.count >= self.goal
    }

    /// Fraction of the daily goal reached, 0.0 to 1.0.
    pub fn progress(&self) -> f32 {
        (self.count as f32 / self.goal as f32).clamp(0.0, 1.0)
    }

    /// True once when a reminder is due. Reminders stop for the day once the goal is reached.
    pub fn due(&mut self, now: Instant) -> bool {
        let Some(next) = self.next else { return false };
        if now < next {
            return false;
        }
        self.reschedule(now);
        !self.reached()
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.next
    }

    /// Time until the next reminder, or `None` if reminders are off or the goal is already met.
    pub fn next_in(&self, now: Instant) -> Option<Duration> {
        if self.reached() {
            return None;
        }
        self.next.map(|n| n.saturating_duration_since(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODAY: u32 = 20261028;

    #[test]
    fn counts_and_clamps() {
        let t0 = Instant::now();
        let mut w = Water::with(TODAY, 8, 1, t0);
        w.add(t0);
        w.add(t0);
        assert_eq!(w.count, 2);
        w.undo();
        w.undo();
        w.undo();
        assert_eq!(w.count, 0, "undo never goes below zero");
        for _ in 0..40 {
            w.adjust_goal(1);
        }
        assert_eq!(w.goal, 20);
        assert!((w.progress() - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn reminder_fires_after_interval_and_drinking_postpones_it() {
        let t0 = Instant::now();
        let hour = Duration::from_secs(3600);
        let mut w = Water::with(TODAY, 8, 1, t0); // 1 hour
        assert!(!w.due(t0 + hour - Duration::from_secs(1)));
        assert!(w.due(t0 + hour));
        // Fired once; the next one is an hour later.
        assert!(!w.due(t0 + hour + Duration::from_secs(1)));

        let t1 = t0 + hour * 2;
        w.add(t1);
        assert!(!w.due(t1 + hour - Duration::from_secs(1)));
        assert!(w.due(t1 + hour));
    }

    #[test]
    fn no_reminders_when_off_or_goal_reached() {
        let t0 = Instant::now();
        let hour = Duration::from_secs(3600);

        let mut off = Water::with(TODAY, 8, INTERVALS.len() - 1, t0);
        assert!(off.deadline().is_none());
        assert!(!off.due(t0 + hour * 24));

        let mut done = Water::with(TODAY, 1, 1, t0);
        done.add(t0);
        assert!(done.reached());
        assert!(!done.due(t0 + hour * 2));
        assert!(done.next_in(t0).is_none());
    }

    #[test]
    fn new_day_resets_the_count() {
        let t0 = Instant::now();
        let mut w = Water::with(TODAY, 8, 1, t0);
        w.add(t0);
        w.roll(TODAY);
        assert_eq!(w.count, 1);
        w.roll(TODAY + 1);
        assert_eq!(w.count, 0);
    }
}
