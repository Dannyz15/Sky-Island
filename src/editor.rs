//! The task editor: the state and the button layout of the panel that opens when a task is added
//! or edited. It holds a day and a time of day for the reminder. This file has no drawing code;
//! `main.rs` paints the panel from these rectangles.

use crate::todo::{self, Task};

pub type Rect = (f32, f32, f32, f32);

// Layout, in logical pixels. The panel fills the notch below the tab bar.
pub const FIELD: Rect = (24.0, 34.0, 372.0, 26.0);
pub const DATE_Y: f32 = 70.0;
pub const REMIND_Y: f32 = 100.0;
pub const ROW_H: f32 = 24.0;
/// Where the date reads out, between the previous / next arrows.
pub const DATE_TEXT: Rect = (106.0, DATE_Y, 108.0, ROW_H);
/// The hour and minute read-outs between their buttons.
pub const HOUR_TEXT: Rect = (142.0, REMIND_Y, 26.0, ROW_H);
pub const MINUTE_TEXT: Rect = (220.0, REMIND_Y, 26.0, ROW_H);
/// The hour and minute controls as a whole, for the mouse wheel.
const HOUR_BOX: Rect = (122.0, REMIND_Y, 66.0, ROW_H);
const MINUTE_BOX: Rect = (200.0, REMIND_Y, 66.0, ROW_H);

/// A reminder can be moved in steps of this many minutes.
const MINUTE_STEP: u32 = 5;
/// Default reminder time on a day other than today: 9:00.
const MORNING: u32 = 9 * 60;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    DatePrev,
    DateNext,
    DateNone,
    DateToday,
    DateTomorrow,
    RemindToggle,
    HourDown,
    HourUp,
    MinuteDown,
    MinuteUp,
    /// Swap AM and PM (12-hour clock only).
    Meridiem,
    /// Remind me in this many minutes.
    In(u32),
    TomorrowMorning,
    Cancel,
    Save,
}

const ALL: [Action; 17] = [
    Action::DatePrev,
    Action::DateNext,
    Action::DateNone,
    Action::DateToday,
    Action::DateTomorrow,
    Action::RemindToggle,
    Action::HourDown,
    Action::HourUp,
    Action::MinuteDown,
    Action::MinuteUp,
    Action::Meridiem,
    Action::In(15),
    Action::In(60),
    Action::In(180),
    Action::TomorrowMorning,
    Action::Cancel,
    Action::Save,
];

pub fn rect(action: Action) -> Rect {
    let (y1, y2, y3) = (DATE_Y, REMIND_Y, 130.0);
    match action {
        Action::DatePrev => (82.0, y1, 22.0, ROW_H),
        Action::DateNext => (216.0, y1, 22.0, ROW_H),
        Action::DateNone => (246.0, y1, 42.0, ROW_H),
        Action::DateToday => (292.0, y1, 48.0, ROW_H),
        Action::DateTomorrow => (344.0, y1, 52.0, ROW_H),
        Action::RemindToggle => (80.0, y2, 40.0, ROW_H),
        Action::HourDown => (122.0, y2, 20.0, ROW_H),
        Action::HourUp => (168.0, y2, 20.0, ROW_H),
        Action::MinuteDown => (200.0, y2, 20.0, ROW_H),
        Action::MinuteUp => (246.0, y2, 20.0, ROW_H),
        Action::Meridiem => (274.0, y2, 42.0, ROW_H),
        Action::In(15) => (24.0, y3, 50.0, ROW_H),
        Action::In(60) => (78.0, y3, 44.0, ROW_H),
        Action::In(_) => (126.0, y3, 44.0, ROW_H),
        Action::TomorrowMorning => (174.0, y3, 84.0, ROW_H),
        Action::Cancel => (268.0, y3, 60.0, ROW_H),
        Action::Save => (332.0, y3, 64.0, ROW_H),
    }
}

fn inside(r: Rect, x: f32, y: f32) -> bool {
    x >= r.0 && x <= r.0 + r.2 && y >= r.1 && y <= r.1 + r.3
}

/// The button under a point, if any. The time controls only exist while a reminder is on, and the
/// AM / PM swap only on a 12-hour clock.
pub fn hit(x: f32, y: f32, remind: bool, clock_24h: bool) -> Option<Action> {
    ALL.iter().copied().find(|&a| {
        let available = match a {
            Action::HourDown | Action::HourUp | Action::MinuteDown | Action::MinuteUp => remind,
            Action::Meridiem => remind && !clock_24h,
            _ => true,
        };
        available && inside(rect(a), x, y)
    })
}

/// What the editor needs to know about "now". Local time is passed in so everything here can be tested.
#[derive(Clone, Copy)]
pub struct Clock {
    /// Today, as a day number (see `calendar`).
    pub today: i64,
    /// Minutes since local midnight.
    pub minutes: u32,
    /// Unix time in seconds.
    pub now: u64,
    /// Local time minus UTC, in seconds.
    pub offset: i64,
}

pub enum Outcome {
    Continue,
    Save,
    Cancel,
}

pub struct Editor {
    /// The task being changed, or `None` for a new one.
    pub index: Option<usize>,
    pub date: Option<i64>,
    pub remind: bool,
    /// Reminder time of day, in minutes since midnight.
    pub minutes: u32,
}

impl Editor {
    pub fn new_task(date: Option<i64>, clock: Clock) -> Self {
        Self { index: None, date, remind: false, minutes: default_minutes(date, clock) }
    }

    pub fn for_task(index: usize, task: &Task, clock: Clock) -> Self {
        match task.due {
            Some(due) => {
                let (day, minutes) = todo::split_due(due, clock.offset);
                Self { index: Some(index), date: Some(day), remind: true, minutes }
            }
            None => Self { index: Some(index), date: task.date, remind: false, minutes: default_minutes(task.date, clock) },
        }
    }

    /// The reminder time to store with the task.
    pub fn due(&self, clock: Clock) -> Option<u64> {
        if !self.remind {
            return None;
        }
        todo::compose_due(self.date?, self.minutes, clock.offset)
    }

    /// What to say next to the time: how far away it is, and whether it is already past.
    pub fn status(&self, clock: Clock) -> (String, bool) {
        match self.due(clock) {
            None => (String::new(), false),
            Some(due) if due <= clock.now => ("already past".to_string(), true),
            Some(due) => (todo::due_label(due, clock.now), false),
        }
    }

    pub fn apply(&mut self, action: Action, clock: Clock) -> Outcome {
        match action {
            Action::DatePrev => self.date = Some(self.date.unwrap_or(clock.today) - 1),
            Action::DateNext => self.date = Some(self.date.unwrap_or(clock.today) + 1),
            Action::DateToday => self.date = Some(clock.today),
            Action::DateTomorrow => self.date = Some(clock.today + 1),
            Action::DateNone => {
                // A reminder needs a day to belong to.
                self.date = None;
                self.remind = false;
            }
            Action::RemindToggle => {
                self.remind = !self.remind;
                if self.remind {
                    let date = *self.date.get_or_insert(clock.today);
                    if date == clock.today && self.minutes <= clock.minutes {
                        self.minutes = default_minutes(Some(date), clock);
                    }
                }
            }
            Action::HourUp => self.minutes = (self.minutes + 60) % 1440,
            Action::HourDown => self.minutes = (self.minutes + 1440 - 60) % 1440,
            Action::MinuteUp => self.minutes = self.minutes / 60 * 60 + (self.minutes % 60 + MINUTE_STEP) % 60,
            Action::MinuteDown => self.minutes = self.minutes / 60 * 60 + (self.minutes % 60 + 60 - MINUTE_STEP) % 60,
            Action::Meridiem => self.minutes = (self.minutes + 720) % 1440,
            Action::In(mins) => {
                // Round up to the next five minutes, so the time reads cleanly.
                let at = (clock.now + u64::from(mins) * 60).div_ceil(300) * 300;
                let (day, minutes) = todo::split_due(at, clock.offset);
                self.date = Some(day);
                self.minutes = minutes;
                self.remind = true;
            }
            Action::TomorrowMorning => {
                self.date = Some(clock.today + 1);
                self.minutes = MORNING;
                self.remind = true;
            }
            Action::Cancel => return Outcome::Cancel,
            Action::Save => return Outcome::Save,
        }
        Outcome::Continue
    }

    /// Turn the mouse wheel over the hour or minute controls. `dir` is 1 for up, -1 for down.
    pub fn wheel(&mut self, x: f32, y: f32, dir: i32, clock: Clock) -> bool {
        if !self.remind {
            return false;
        }
        let action = if inside(HOUR_BOX, x, y) {
            if dir > 0 { Action::HourUp } else { Action::HourDown }
        } else if inside(MINUTE_BOX, x, y) {
            if dir > 0 { Action::MinuteUp } else { Action::MinuteDown }
        } else {
            return false;
        };
        self.apply(action, clock);
        true
    }
}

/// A sensible reminder time for a new reminder: the next half hour today, or the morning of a later day.
fn default_minutes(date: Option<i64>, clock: Clock) -> u32 {
    match date {
        Some(d) if d != clock.today => MORNING,
        _ => ((clock.minutes / 30 + 1) * 30).min(24 * 60 - MINUTE_STEP),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 28 Oct 2026, 14:37 local, at UTC+8.
    fn clock() -> Clock {
        let today = crate::calendar::days_from_civil(2026, 10, 28);
        let offset = 8 * 3600;
        Clock { today, minutes: 14 * 60 + 37, now: (today * 86_400 + (14 * 60 + 37) * 60 - offset) as u64, offset }
    }

    #[test]
    fn turning_a_reminder_on_picks_the_next_half_hour_today() {
        let c = clock();
        let mut e = Editor::new_task(None, c);
        assert!(!e.remind && e.date.is_none());
        e.apply(Action::RemindToggle, c);
        assert!(e.remind);
        assert_eq!(e.date, Some(c.today), "a reminder needs a day");
        assert_eq!(e.minutes, 15 * 60);
        assert_eq!(e.status(c), ("in 23m".to_string(), false));
    }

    #[test]
    fn clearing_the_date_also_clears_the_reminder() {
        let c = clock();
        let mut e = Editor::new_task(Some(c.today), c);
        e.apply(Action::RemindToggle, c);
        e.apply(Action::DateNone, c);
        assert!(!e.remind);
        assert_eq!(e.due(c), None);
    }

    #[test]
    fn steppers_wrap_without_touching_the_other_part() {
        let c = clock();
        let mut e = Editor::new_task(Some(c.today), c);
        e.minutes = 23 * 60 + 55;
        e.apply(Action::MinuteUp, c);
        assert_eq!(e.minutes, 23 * 60, "minutes wrap inside the hour");
        e.apply(Action::HourUp, c);
        assert_eq!(e.minutes, 0, "hours wrap at midnight");
        e.apply(Action::HourDown, c);
        assert_eq!(e.minutes, 23 * 60);
        e.apply(Action::MinuteDown, c);
        assert_eq!(e.minutes, 23 * 60 + 55);
        e.apply(Action::Meridiem, c);
        assert_eq!(e.minutes, 11 * 60 + 55);
    }

    #[test]
    fn quick_choices_round_up_to_five_minutes() {
        let c = clock();
        let mut e = Editor::new_task(None, c);
        e.apply(Action::In(15), c); // 14:52 -> 14:55
        assert_eq!((e.date, e.minutes, e.remind), (Some(c.today), 14 * 60 + 55, true));
        e.apply(Action::In(180), c); // 17:37 -> 17:40
        assert_eq!(e.minutes, 17 * 60 + 40);
        e.apply(Action::TomorrowMorning, c);
        assert_eq!((e.date, e.minutes), (Some(c.today + 1), 9 * 60));
    }

    #[test]
    fn quick_choice_after_midnight_moves_to_the_next_day() {
        let mut c = clock();
        c.minutes = 23 * 60 + 50;
        c.now = (c.today * 86_400 + c.minutes as i64 * 60 - c.offset) as u64;
        let mut e = Editor::new_task(None, c);
        e.apply(Action::In(60), c);
        assert_eq!(e.date, Some(c.today + 1));
        assert_eq!(e.minutes, 50);
    }

    #[test]
    fn due_matches_the_chosen_local_time_and_detects_the_past() {
        let c = clock();
        let mut e = Editor::new_task(Some(c.today), c);
        e.remind = true;
        e.minutes = 9 * 60;
        let due = e.due(c).unwrap();
        assert_eq!(todo::split_due(due, c.offset), (c.today, 9 * 60));
        assert_eq!(e.status(c), ("already past".to_string(), true));
    }

    #[test]
    fn editing_a_task_loads_its_date_and_time() {
        let c = clock();
        let due = todo::compose_due(c.today + 2, 18 * 60 + 30, c.offset).unwrap();
        let task = Task { text: "x".into(), done: false, due: Some(due), notified: false, date: None };
        let e = Editor::for_task(3, &task, c);
        assert_eq!((e.index, e.date, e.remind, e.minutes), (Some(3), Some(c.today + 2), true, 18 * 60 + 30));
        let plain = Task { text: "y".into(), done: false, due: None, notified: false, date: Some(c.today + 5) };
        let e = Editor::for_task(0, &plain, c);
        assert_eq!((e.date, e.remind, e.minutes), (Some(c.today + 5), false, 9 * 60));
    }

    #[test]
    fn hit_testing_hides_controls_that_are_not_shown() {
        let centre = |a: Action| {
            let r = rect(a);
            (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0)
        };
        let (x, y) = centre(Action::HourUp);
        assert_eq!(hit(x, y, true, false), Some(Action::HourUp));
        assert_eq!(hit(x, y, false, false), None, "no time controls without a reminder");
        let (x, y) = centre(Action::Meridiem);
        assert_eq!(hit(x, y, true, false), Some(Action::Meridiem));
        assert_eq!(hit(x, y, true, true), None, "no AM / PM on a 24-hour clock");
        let (x, y) = centre(Action::Save);
        assert_eq!(hit(x, y, false, true), Some(Action::Save));
    }

    #[test]
    fn buttons_do_not_overlap_each_other() {
        let all = [
            Action::DatePrev, Action::DateNext, Action::DateNone, Action::DateToday, Action::DateTomorrow,
            Action::RemindToggle, Action::HourDown, Action::HourUp, Action::MinuteDown, Action::MinuteUp,
            Action::Meridiem, Action::In(15), Action::In(60), Action::In(180), Action::TomorrowMorning,
            Action::Cancel, Action::Save,
        ];
        for (i, &a) in all.iter().enumerate() {
            let (ra, right) = (rect(a), rect(a).0 + rect(a).2);
            assert!(right <= 396.5 && ra.1 + ra.3 <= 160.0, "{a:?} leaves the panel");
            for &b in &all[i + 1..] {
                let rb = rect(b);
                let apart = right <= rb.0 || rb.0 + rb.2 <= ra.0 || ra.1 + ra.3 <= rb.1 || rb.1 + rb.3 <= ra.1;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn wheel_changes_hours_and_minutes_only_while_a_reminder_is_on() {
        let c = clock();
        let mut e = Editor::new_task(Some(c.today), c);
        assert!(!e.wheel(150.0, 112.0, 1, c));
        e.apply(Action::RemindToggle, c);
        let before = e.minutes;
        assert!(e.wheel(150.0, 112.0, 1, c), "over the hour");
        assert_eq!(e.minutes, (before + 60) % 1440);
        assert!(e.wheel(230.0, 112.0, -1, c), "over the minutes");
        assert!(!e.wheel(30.0, 40.0, 1, c), "over the text field");
    }
}
