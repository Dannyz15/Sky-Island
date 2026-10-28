//! To-do list with optional reminders. Pure state, with the current time (unix seconds) passed in
//! so it can be tested. Saved as JSON between runs.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

const MAX_TASKS: usize = 100;
const MAX_LEN: usize = 80;

/// Reminder choices when adding a task: label and delay in seconds. `None` = no reminder.
pub const REMINDERS: [(&str, Option<u64>); 5] = [
    ("None", None),
    ("15m", Some(15 * 60)),
    ("1h", Some(60 * 60)),
    ("3h", Some(3 * 60 * 60)),
    ("1d", Some(24 * 60 * 60)),
];
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
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| {
                v.as_array().map(|a| {
                    a.iter()
                        .filter_map(|t| {
                            Some(Task {
                                text: t.get("text")?.as_str()?.to_string(),
                                done: t.get("done").and_then(Value::as_bool).unwrap_or(false),
                                due: t.get("due").and_then(Value::as_u64),
                                notified: t.get("notified").and_then(Value::as_bool).unwrap_or(false),
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
        let dir = PathBuf::from(std::env::var_os("APPDATA")?).join("Sky-Island");
        fs::create_dir_all(&dir).ok()?;
        Some(dir.join("todo.json"))
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
                .map(|t| json!({ "text": t.text, "done": t.done, "due": t.due, "notified": t.notified }))
                .collect();
            let _ = fs::write(p, json!(body).to_string());
        }
    }

    /// Add a task at the top of the open ones. `remind` indexes `REMINDERS`.
    pub fn add(&mut self, text: &str, remind: usize, now: u64) -> bool {
        let text: String = text.trim().chars().take(MAX_LEN).collect();
        if text.is_empty() || self.tasks.len() >= MAX_TASKS {
            return false;
        }
        let due = REMINDERS.get(remind).and_then(|(_, d)| *d).map(|d| now + d);
        self.tasks.insert(0, Task { text, done: false, due, notified: false });
        self.scroll = 0;
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
        assert!(!t.add("   ", 0, NOW));
        assert!(t.add("  first ", 0, NOW));
        assert!(t.add("second", 0, NOW));
        assert_eq!(t.tasks[0].text, "second");
        assert_eq!(t.tasks[1].text, "first");
        assert!(t.add(&"x".repeat(200), 0, NOW));
        assert_eq!(t.tasks[0].text.chars().count(), MAX_LEN);
    }

    #[test]
    fn reminder_fires_once_when_due() {
        let mut t = Todo::empty();
        t.add("call mum", 1, NOW); // 15m
        assert_eq!(t.next_due(), Some(NOW + 900));
        assert_eq!(t.take_due(NOW + 899), None);
        assert_eq!(t.take_due(NOW + 900), Some("call mum".to_string()));
        assert_eq!(t.take_due(NOW + 5000), None, "fires only once");
        assert_eq!(t.next_due(), None);
    }

    #[test]
    fn done_tasks_do_not_remind() {
        let mut t = Todo::empty();
        t.add("pay bill", 1, NOW);
        t.toggle(0);
        assert_eq!(t.take_due(NOW + 10_000), None);
        assert_eq!(t.next_due(), None);
        assert_eq!(t.open_count(), 0);
    }

    #[test]
    fn scroll_and_rows() {
        let mut t = Todo::empty();
        for i in 0..7 {
            t.add(&format!("task {i}"), 0, NOW);
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
    fn due_labels() {
        assert_eq!(due_label(NOW + 30, NOW), "in 1m");
        assert_eq!(due_label(NOW + 15 * 60, NOW), "in 15m");
        assert_eq!(due_label(NOW + 2 * 3600 + 600, NOW), "in 2h 10m");
        assert_eq!(due_label(NOW + 27 * 3600, NOW), "in 1d 3h");
        assert_eq!(due_label(NOW, NOW), "overdue");
        assert_eq!(due_label(NOW - 5, NOW), "overdue");
    }
}
