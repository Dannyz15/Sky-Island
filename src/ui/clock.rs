//! Dates, times and how they are written.

use crate::*;

/// Today's local date as yyyymmdd.
pub(crate) fn today() -> u32 {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe { GetLocalTime() };
    t.wYear as u32 * 10000 + t.wMonth as u32 * 100 + t.wDay as u32
}

/// Today's day number, for the calendar.
pub(crate) fn today_day() -> i64 {
    let t = today();
    calendar::days_from_civil((t / 10_000) as i32, t / 100 % 100, t % 100)
}

/// Local time of day as (hour, minute, second, millisecond).
pub(crate) fn local_time() -> (u32, u32, u32, u32) {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe { GetLocalTime() };
    (t.wHour as u32, t.wMinute as u32, t.wSecond as u32, t.wMilliseconds as u32)
}

pub(crate) fn fmt_clock(hour: u32, minute: u32, clock_24h: bool) -> String {
    if clock_24h {
        format!("{hour:02}:{minute:02}")
    } else {
        format!("{}:{minute:02} {}", if hour.is_multiple_of(12) { 12 } else { hour % 12 }, if hour < 12 { "AM" } else { "PM" })
    }
}

/// Local time minus UTC in seconds, read from the system clock (rounded to a quarter hour).
pub(crate) fn tz_offset() -> i64 {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe { GetLocalTime() };
    let local = calendar::days_from_civil(t.wYear as i32, t.wMonth as u32, t.wDay as u32) * 86_400
        + i64::from(t.wHour) * 3600
        + i64::from(t.wMinute) * 60
        + i64::from(t.wSecond);
    ((local - todo::unix_now() as i64) as f64 / 900.0).round() as i64 * 900
}

/// Everything the task editor needs to know about the present moment.
pub(crate) fn clock_now() -> editor::Clock {
    let (hour, minute, _, _) = local_time();
    editor::Clock { today: today_day(), minutes: hour * 60 + minute, now: todo::unix_now(), offset: tz_offset() }
}

/// The time of a reminder as text, and whether it is overdue. Finished tasks are never overdue.
pub(crate) fn split_clock(due: u64, now: u64, offset: i64, clock_24h: bool, done: bool) -> (String, bool) {
    if !done && due <= now {
        return ("overdue".to_string(), true);
    }
    let (_, minutes) = todo::split_due(due, offset);
    (fmt_clock(minutes / 60, minutes % 60, clock_24h), false)
}

/// What to show at the right of a task: when it is due, or else its day.
pub(crate) fn task_when(task: &todo::Task, today: i64, now: u64, offset: i64, clock_24h: bool) -> Option<(String, bool)> {
    if let Some(due) = task.due {
        let (time, overdue) = split_clock(due, now, offset, clock_24h, task.done);
        if overdue {
            return Some((time, true));
        }
        let (day, _) = todo::split_due(due, offset);
        let label = match day - today {
            0 => time,
            1 => format!("Tomorrow {time}"),
            _ => format!("{} {time}", calendar::short_date(day)),
        };
        return Some((label, false));
    }
    task.date.map(|day| (calendar::short_date(day), false))
}

/// mm:ss, rounded up so the display hits 0:00 exactly when the phase ends.
pub(crate) fn fmt_remaining(d: Duration) -> String {
    let secs = d.as_secs() + (d.subsec_nanos() > 0) as u64;
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, secs % 3600 / 60, secs % 60)
    } else {
        format!("{:02}:{:02}", secs / 60, secs % 60)
    }
}

pub(crate) fn fmt_time(d: Duration) -> String {
    let t = d.as_secs();
    format!("{:02}:{:02}", t / 60, t % 60)
}
