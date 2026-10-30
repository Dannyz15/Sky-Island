//! To-do list with optional reminders at a chosen day and time. Pure state, with the current time (unix seconds) passed in
//! so it can be tested. Saved as JSON between runs.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::store;

const MAX_TASKS: usize = 100;
const MAX_LEN: usize = 80;

/// Tasks visible at once in the list.
pub const VISIBLE: usize = 4;

#[derive(Clone, PartialEq, Debug)]
pub struct Task {
    pub text: String,
    pub done: bool,
    /// Reminder time, unix seconds.
    pub due: Option<u64>,
    /// The reminder has already fired.
    pub notified: bool,
    /// The calendar day the task belongs to, as a day number (see `calendar`).
    pub date: Option<i64>,
}

pub struct Todo {
    pub tasks: Vec<Task>,
    /// First visible row.
    pub scroll: usize,
}

pub fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Todo {
    pub fn new() -> Self {
        let tasks = Self::path()
            .and_then(|p| store::read_json(&p))
            .and_then(|v| {
                v.as_array().map(|a| {
                    a.iter()
                        .filter_map(|t| {
                            Some(Task {
                                text: t.get("text")?.as_str()?.to_string(),
                                done: t.get("done").and_then(Value::as_bool).unwrap_or(false),
                                due: t.get("due").and_then(Value::as_u64),
                                notified: t.get("notified").and_then(Value::as_bool).unwrap_or(false),
                                date: t.get("date").and_then(Value::as_i64),
                            })
                        })
                        .collect()
                })
            })
            .unwrap_or_default();
        Self { tasks, scroll: 0 }
    }

    #[cfg(test)]
    fn empty() -> Self {
        Self { tasks: Vec::new(), scroll: 0 }
    }

    fn path() -> Option<PathBuf> {
        store::path("todo.json")
    }

    fn save(&self) {
        // Tests must not overwrite the user's real data.
        if cfg!(test) {
            return;
        }
        if let Some(p) = Self::path() {
            let body: Vec<Value> = self
                .tasks
                .iter()
                .map(|t| json!({ "text": t.text, "done": t.done, "due": t.due, "notified": t.notified, "date": t.date }))
                .collect();
            store::write(&p, &json!(body).to_string());
        }
    }

    /// Add a task at the top of the list. `due` is the reminder time and `date` the calendar day.
    pub fn add(&mut self, text: &str, due: Option<u64>, date: Option<i64>) -> bool {
        let text: String = text.trim().chars().take(MAX_LEN).collect();
        if text.is_empty() || self.tasks.len() >= MAX_TASKS {
            return false;
        }
        self.tasks.insert(0, Task { text, done: false, due, notified: false, date });
        self.scroll = 0;
        self.save();
        true
    }

    /// Change a task's text, day and reminder. A reminder that moved gets to fire again.
    pub fn update(&mut self, index: usize, text: &str, due: Option<u64>, date: Option<i64>) -> bool {
        let text: String = text.trim().chars().take(MAX_LEN).collect();
        let Some(task) = self.tasks.get_mut(index).filter(|_| !text.is_empty()) else { return false };
        if task.due != due {
            task.notified = false;
        }
        task.text = text;
        task.due = due;
        task.date = date;
        self.save();
        true
    }

    pub fn toggle(&mut self, index: usize) {
        if let Some(t) = self.tasks.get_mut(index) {
            t.done = !t.done;
            self.save();
        }
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.tasks.len() {
            self.tasks.remove(index);
            self.clamp_scroll();
            self.save();
        }
    }

    pub fn scroll_by(&mut self, rows: isize) {
        self.scroll = (self.scroll as isize + rows).max(0) as usize;
        self.clamp_scroll();
    }

    fn clamp_scroll(&mut self) {
        self.scroll = self.scroll.min(self.tasks.len().saturating_sub(VISIBLE));
    }

    /// Task index shown in a visible row, if there is one.
    pub fn index_at(&self, row: usize) -> Option<usize> {
        let i = self.scroll + row;
        (row < VISIBLE && i < self.tasks.len()).then_some(i)
    }

    /// Indexes of the tasks that belong to a calendar day, in list order.
    pub fn on_day(&self, day: i64) -> Vec<usize> {
        self.tasks.iter().enumerate().filter(|(_, t)| t.date == Some(day)).map(|(i, _)| i).collect()
    }

    /// (open, total) tasks on a calendar day.
    pub fn day_counts(&self, day: i64) -> (usize, usize) {
        let on_day = self.tasks.iter().filter(|t| t.date == Some(day));
        on_day.fold((0, 0), |(open, total), t| (open + usize::from(!t.done), total + 1))
    }

    /// The first open task whose reminder has come due and not fired yet. Marks it as fired.
    pub fn take_due(&mut self, now: u64) -> Option<String> {
        let t = self.tasks.iter_mut().find(|t| !t.done && !t.notified && t.due.is_some_and(|d| d <= now))?;
        t.notified = true;
        let text = t.text.clone();
        self.save();
        Some(text)
    }

    /// Unix time of the next reminder that has yet to fire, for scheduling a wake-up.
    pub fn next_due(&self) -> Option<u64> {
        self.tasks.iter().filter(|t| !t.done && !t.notified).filter_map(|t| t.due).min()
    }

    #[cfg(test)]
    fn open_count(&self) -> usize {
        self.tasks.iter().filter(|t| !t.done).count()
    }
}

/// The reminder time, in unix seconds, for a local day and minute of the day. `offset` is local
/// time minus UTC in seconds. `None` for a time before 1970.
pub fn compose_due(day: i64, minutes: u32, offset: i64) -> Option<u64> {
    u64::try_from(day * 86_400 + i64::from(minutes) * 60 - offset).ok()
}

/// The local day number and minute of the day of a reminder time; the inverse of `compose_due`.
pub fn split_due(due: u64, offset: i64) -> (i64, u32) {
    let local = due as i64 + offset;
    (local.div_euclid(86_400), (local.rem_euclid(86_400) / 60) as u32)
}

/// Short text for when a reminder is due: "in 15m", "in 2h 10m", "in 1d 3h", or "overdue".
pub fn due_label(due: u64, now: u64) -> String {
    let Some(left) = due.checked_sub(now).filter(|&l| l > 0) else {
        return "overdue".to_string();
    };
    let (days, hours, mins) = (left / 86_400, left % 86_400 / 3600, left % 3600 / 60);
    match (days, hours) {
        (0, 0) => format!("in {}m", mins.max(1)),
        (0, h) => format!("in {h}h {mins}m"),
        (d, h) => format!("in {d}d {h}h"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;

    #[test]
    fn add_trims_rejects_blank_and_puts_newest_first() {
        let mut t = Todo::empty();
        assert!(!t.add("   ", None, None));
        assert!(t.add("  first ", None, None));
        assert!(t.add("second", None, None));
        assert_eq!(t.tasks[0].text, "second");
        assert_eq!(t.tasks[1].text, "first");
        assert!(t.add(&"x".repeat(200), None, None));
        assert_eq!(t.tasks[0].text.chars().count(), MAX_LEN);
    }

    #[test]
    fn reminder_fires_once_when_due() {
        let mut t = Todo::empty();
        t.add("call mum", Some(NOW + 900), None);
        assert_eq!(t.next_due(), Some(NOW + 900));
        assert_eq!(t.take_due(NOW + 899), None);
        assert_eq!(t.take_due(NOW + 900), Some("call mum".to_string()));
        assert_eq!(t.take_due(NOW + 5000), None, "fires only once");
        assert_eq!(t.next_due(), None);
    }

    #[test]
    fn editing_a_task_keeps_it_in_place_and_rearms_a_moved_reminder() {
        let mut t = Todo::empty();
        t.add("older", None, None);
        t.add("call mum", Some(NOW + 900), Some(20_000));
        assert_eq!(t.take_due(NOW + 900), Some("call mum".to_string()));
        t.toggle(0);

        assert!(t.update(0, "  call dad ", Some(NOW + 900), Some(20_000)));
        assert_eq!((t.tasks[0].text.as_str(), t.tasks[1].text.as_str()), ("call dad", "older"));
        assert!(t.tasks[0].done, "editing does not tick or untick");
        assert!(t.tasks[0].notified, "same time: still fired");

        assert!(t.update(0, "call dad", Some(NOW + 7200), Some(20_001)));
        assert!(!t.tasks[0].notified, "a new time may fire again");
        assert_eq!(t.tasks[0].date, Some(20_001));

        assert!(!t.update(0, "   ", None, None), "blank text is refused");
        assert!(!t.update(9, "nothing there", None, None));
        assert_eq!(t.tasks[0].text, "call dad");
        assert!(t.update(0, "call dad", None, None));
        assert_eq!(t.tasks[0].due, None, "a reminder can be taken away");
    }

    #[test]
    fn local_times_convert_to_reminder_times_and_back() {
        let offset = 8 * 3600; // UTC+8
        let day = 20_754; // 28 Oct 2026
        let due = compose_due(day, 9 * 60 + 30, offset).unwrap();
        assert_eq!(due, day as u64 * 86_400 + (9 * 60 + 30) * 60 - 8 * 3600);
        assert_eq!(split_due(due, offset), (day, 9 * 60 + 30));
        // 00:10 local at UTC+8 is still the day before in UTC.
        let early = compose_due(day, 10, offset).unwrap();
        assert_eq!(split_due(early, 0).0, day - 1);
        assert_eq!(split_due(early, offset), (day, 10));
        // Negative offsets work too.
        let west = compose_due(day, 23 * 60, -5 * 3600).unwrap();
        assert_eq!(split_due(west, -5 * 3600), (day, 23 * 60));
        assert_eq!(compose_due(0, 0, 3600), None, "before 1970");
    }

    #[test]
    fn done_tasks_do_not_remind() {
        let mut t = Todo::empty();
        t.add("pay bill", Some(NOW + 900), None);
        t.toggle(0);
        assert_eq!(t.take_due(NOW + 10_000), None);
        assert_eq!(t.next_due(), None);
        assert_eq!(t.open_count(), 0);
    }

    #[test]
    fn scroll_and_rows() {
        let mut t = Todo::empty();
        for i in 0..7 {
            t.add(&format!("task {i}"), None, None);
        }
        t.scroll_by(10);
        assert_eq!(t.scroll, 3, "7 tasks, 4 visible: at most 3 rows of scroll");
        assert_eq!(t.index_at(0), Some(3));
        assert_eq!(t.index_at(VISIBLE), None);
        t.remove(6);
        t.remove(5);
        t.remove(4);
        assert_eq!(t.scroll, 0);
    }

    #[test]
    fn tasks_belong_to_calendar_days() {
        let mut t = Todo::empty();
        t.add("on the 5th", None, Some(20_000));
        t.add("also the 5th", None, Some(20_000));
        t.add("no day", None, None);
        assert_eq!(t.on_day(20_000), vec![1, 2], "newest first, so the undated task is row 0");
        assert_eq!(t.day_counts(20_000), (2, 2));
        t.toggle(1);
        assert_eq!(t.day_counts(20_000), (1, 2));
        assert_eq!(t.day_counts(20_001), (0, 0));
        assert_eq!(t.tasks[0].date, None);
    }

    #[test]
    fn due_labels() {
        assert_eq!(due_label(NOW + 30, NOW), "in 1m");
        assert_eq!(due_label(NOW + 15 * 60, NOW), "in 15m");
        assert_eq!(due_label(NOW + 2 * 3600 + 600, NOW), "in 2h 10m");
        assert_eq!(due_label(NOW + 27 * 3600, NOW), "in 1d 3h");
        assert_eq!(due_label(NOW, NOW), "overdue");
        assert_eq!(due_label(NOW - 5, NOW), "overdue");
    }
}
