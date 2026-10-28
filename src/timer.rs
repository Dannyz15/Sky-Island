//! A countdown timer with a length the user sets (hours, minutes and seconds). Pure state, with
//! `now` passed in so it can be tested. The length that was last set is saved between runs.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const DEFAULT: Duration = Duration::from_secs(10 * 60);
const MIN_SECS: u64 = 1;
const MAX_SECS: u64 = 24 * 3600;

/// Quick choices shown as chips, in seconds.
pub const PRESETS: [u64; 5] = [5 * 60, 10 * 60, 15 * 60, 30 * 60, 60 * 60];

/// Which part of the length a plus or minus button changes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Unit {
    Hours,
    Minutes,
    Seconds,
}

impl Unit {
    pub const ALL: [Unit; 3] = [Unit::Hours, Unit::Minutes, Unit::Seconds];

    /// How many seconds one press changes the length by: exactly one of the unit.
    fn step(self) -> u64 {
        match self {
            Unit::Hours => 3600,
            Unit::Minutes => 60,
            Unit::Seconds => 1,
        }
    }

    /// The largest value the unit can hold when typed in.
    fn limit(self) -> u64 {
        match self {
            Unit::Hours => 24,
            Unit::Minutes | Unit::Seconds => 59,
        }
    }

    /// The letter shown next to the number.
    pub fn letter(self) -> &'static str {
        match self {
            Unit::Hours => "h",
            Unit::Minutes => "m",
            Unit::Seconds => "s",
        }
    }
}

enum State {
    /// Not started: the whole length is left.
    Idle,
    Running { ends: Instant },
    Paused { left: Duration },
}

pub struct Timer {
    total: Duration,
    state: State,
}

impl Timer {
    pub fn load() -> Self {
        let secs = Self::path()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v.get("secs").and_then(Value::as_u64))
            .filter(|s| (MIN_SECS..=MAX_SECS).contains(s))
            .map_or(DEFAULT, Duration::from_secs);
        Self { total: secs, state: State::Idle }
    }

    #[cfg(test)]
    fn with(total: Duration) -> Self {
        Self { total, state: State::Idle }
    }

    fn path() -> Option<PathBuf> {
        let dir = PathBuf::from(std::env::var_os("APPDATA")?).join("Sky-Island");
        fs::create_dir_all(&dir).ok()?;
        Some(dir.join("timer.json"))
    }

    fn save(&self) {
        // Tests must not overwrite the user's real data.
        if cfg!(test) {
            return;
        }
        if let Some(p) = Self::path() {
            let _ = fs::write(p, json!({ "secs": self.total.as_secs() }).to_string());
        }
    }

    /// The length that is set.
    pub fn total(&self) -> Duration {
        self.total
    }

    pub fn is_idle(&self) -> bool {
        matches!(self.state, State::Idle)
    }

    pub fn running(&self) -> bool {
        matches!(self.state, State::Running { .. })
    }

    pub fn paused(&self) -> bool {
        matches!(self.state, State::Paused { .. })
    }

    /// Time left; the whole length while it has not been started.
    pub fn remaining(&self, now: Instant) -> Duration {
        match self.state {
            State::Idle => self.total,
            State::Running { ends } => ends.saturating_duration_since(now),
            State::Paused { left } => left,
        }
    }

    /// The part of the length still left, 1.0 at the start and 0.0 at the end.
    pub fn fraction_left(&self, now: Instant) -> f32 {
        (self.remaining(now).as_secs_f32() / self.total.as_secs_f32().max(1.0)).clamp(0.0, 1.0)
    }

    /// Start it, pause it, or carry on after a pause.
    pub fn toggle(&mut self, now: Instant) {
        self.state = match self.state {
            State::Idle => State::Running { ends: now + self.total },
            State::Running { ends } => State::Paused { left: ends.saturating_duration_since(now) },
            State::Paused { left } => State::Running { ends: now + left },
        };
    }

    /// Stop and go back to the full length.
    pub fn reset(&mut self) {
        self.state = State::Idle;
    }

    /// True once, when the time has run out. The timer then goes back to the full length.
    pub fn tick(&mut self, now: Instant) -> bool {
        match self.state {
            State::Running { ends } if now >= ends => {
                self.state = State::Idle;
                true
            }
            _ => false,
        }
    }

    /// When it will finish, for scheduling a wake-up.
    pub fn deadline(&self) -> Option<Instant> {
        match self.state {
            State::Running { ends } => Some(ends),
            _ => None,
        }
    }

    /// Set the length, in seconds. Only while it is not running or paused.
    pub fn set(&mut self, secs: u64) -> bool {
        let secs = secs.clamp(MIN_SECS, MAX_SECS);
        if !self.is_idle() || Duration::from_secs(secs) == self.total {
            return false;
        }
        self.total = Duration::from_secs(secs);
        self.save();
        true
    }

    /// Make the length one step longer (`dir` > 0) or shorter. `false` when nothing changed.
    pub fn adjust(&mut self, unit: Unit, dir: i32) -> bool {
        let secs = self.total.as_secs();
        let next = if dir > 0 { secs.saturating_add(unit.step()) } else { secs.saturating_sub(unit.step()) };
        self.set(next)
    }

    /// Replace one part of the length with a typed value, keeping the other two. Values past what
    /// the part can hold (59 for minutes and seconds, 24 for hours) are cut back to it.
    pub fn set_part(&mut self, unit: Unit, value: u64) -> bool {
        let (h, m, s) = self.parts();
        let value = value.min(unit.limit());
        let (h, m, s) = match unit {
            Unit::Hours => (value, m, s),
            Unit::Minutes => (h, value, s),
            Unit::Seconds => (h, m, value),
        };
        self.set(h * 3600 + m * 60 + s)
    }

    /// The set length as (hours, minutes, seconds).
    pub fn parts(&self) -> (u64, u64, u64) {
        let secs = self.total.as_secs();
        (secs / 3600, secs % 3600 / 60, secs % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn starts_pauses_and_resumes_where_it_left_off() {
        let t0 = Instant::now();
        let mut t = Timer::with(secs(600));
        assert!(t.is_idle());
        assert_eq!(t.remaining(t0), secs(600), "idle shows the full length");
        assert_eq!(t.fraction_left(t0), 1.0);

        t.toggle(t0);
        assert!(t.running());
        assert_eq!(t.remaining(t0 + secs(100)), secs(500));
        assert!((t.fraction_left(t0 + secs(300)) - 0.5).abs() < 1e-6);

        t.toggle(t0 + secs(100)); // pause
        assert!(t.paused());
        assert_eq!(t.remaining(t0 + secs(9999)), secs(500), "time does not pass while paused");

        t.toggle(t0 + secs(1000)); // resume
        assert_eq!(t.remaining(t0 + secs(1200)), secs(300));
        assert_eq!(t.deadline(), Some(t0 + secs(1500)));
    }

    #[test]
    fn finishes_once_and_goes_back_to_the_full_length() {
        let t0 = Instant::now();
        let mut t = Timer::with(secs(60));
        t.toggle(t0);
        assert!(!t.tick(t0 + secs(59)));
        assert!(t.tick(t0 + secs(60)), "fires when the time is up");
        assert!(!t.tick(t0 + secs(61)), "only once");
        assert!(t.is_idle());
        assert_eq!(t.remaining(t0 + secs(100)), secs(60));
        assert_eq!(t.deadline(), None);
    }

    #[test]
    fn reset_stops_and_restores_the_length() {
        let t0 = Instant::now();
        let mut t = Timer::with(secs(90));
        t.toggle(t0);
        t.reset();
        assert!(t.is_idle());
        assert_eq!(t.remaining(t0 + secs(5)), secs(90));
        t.toggle(t0);
        t.toggle(t0 + secs(30));
        t.reset();
        assert!(t.is_idle() && !t.paused());
    }

    #[test]
    fn the_length_can_only_change_while_idle_and_stays_in_range() {
        let t0 = Instant::now();
        let mut t = Timer::with(secs(25 * 60));
        assert_eq!(t.parts(), (0, 25, 0));
        assert!(t.adjust(Unit::Hours, 1));
        assert_eq!(t.parts(), (1, 25, 0));
        assert!(t.adjust(Unit::Seconds, 1));
        assert_eq!(t.parts(), (1, 25, 1), "seconds move one at a time");
        assert!(t.adjust(Unit::Seconds, -1));
        assert!(t.adjust(Unit::Seconds, -1));
        assert_eq!(t.parts(), (1, 24, 59), "and borrow from the minutes");
        assert!(t.adjust(Unit::Minutes, -1));
        assert_eq!(t.parts(), (1, 23, 59));

        assert!(t.set(1));
        assert!(!t.adjust(Unit::Seconds, -1), "never below one second");
        assert!(!t.adjust(Unit::Minutes, -1));
        assert_eq!(t.total(), secs(1));
        assert!(t.set(MAX_SECS + 5000) && t.total() == secs(MAX_SECS), "never above a day");
        assert!(!t.adjust(Unit::Hours, 1));

        t.set(300);
        t.toggle(t0);
        assert!(!t.adjust(Unit::Minutes, 1), "not while running");
        assert!(!t.set(600));
        t.toggle(t0 + secs(10));
        assert!(!t.adjust(Unit::Minutes, 1), "not while paused");
        assert_eq!(t.total(), secs(300));
    }

    #[test]
    fn typed_parts_replace_one_part_and_keep_the_others() {
        let mut t = Timer::with(secs(3600 + 5 * 60 + 9));
        assert!(t.set_part(Unit::Seconds, 42));
        assert_eq!(t.parts(), (1, 5, 42));
        assert!(t.set_part(Unit::Minutes, 30));
        assert_eq!(t.parts(), (1, 30, 42));
        assert!(t.set_part(Unit::Hours, 0));
        assert_eq!(t.parts(), (0, 30, 42));
        assert!(t.set_part(Unit::Seconds, 99), "too big: cut back to 59");
        assert_eq!(t.parts(), (0, 30, 59));
        assert!(!t.set_part(Unit::Seconds, 59), "same value, nothing changes");
        assert!(t.set_part(Unit::Hours, 40));
        assert_eq!(t.parts(), (24, 0, 0), "never more than a day");
        t.set(60);
        assert!(t.set_part(Unit::Minutes, 0));
        assert_eq!(t.total(), secs(MIN_SECS), "all zeros becomes one second");
    }

    #[test]
    fn presets_are_all_valid_lengths() {
        for p in PRESETS {
            assert!((MIN_SECS..=MAX_SECS).contains(&p));
        }
        let mut t = Timer::with(secs(7));
        assert!(t.set(PRESETS[0]));
        assert!(!t.set(PRESETS[0]), "choosing the same length again changes nothing");
    }
}
