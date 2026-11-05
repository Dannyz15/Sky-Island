//! Local time against UTC, for the times of reminders. How far the local clock is from UTC is not one number for the
//! whole year where the clocks change (summer time), so it is asked of Windows for the very moment that is converted:
//! a reminder set in March for a day in April is changed with April's offset, not March's.

use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Time::{SystemTimeToTzSpecificLocalTime, TIME_ZONE_INFORMATION, TzSpecificLocalTimeToSystemTime};

use crate::{calendar, todo};

/// Which clock local times are read on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Zone {
    /// A fixed offset, local time minus UTC in seconds (the tests).
    #[cfg_attr(not(test), allow(dead_code))]
    Fixed(i64),
    /// The time zone of this PC, with its summer time.
    System,
}

impl Zone {
    /// Local time minus UTC, in seconds, at the UTC moment `unix`.
    pub fn at_utc(self, unix: u64) -> i64 {
        match self {
            Zone::Fixed(offset) => offset,
            Zone::System => offset_at_utc(None, unix as i64).unwrap_or_else(offset_now),
        }
    }

    /// Local time minus UTC, in seconds, at a local `day` and minute of the day.
    pub fn at_local(self, day: i64, minutes: u32) -> i64 {
        match self {
            Zone::Fixed(offset) => offset,
            Zone::System => offset_at_local(None, day * 86_400 + i64::from(minutes) * 60).unwrap_or_else(offset_now),
        }
    }

    /// The reminder time (unix seconds) of a local day and minute of the day; `None` for a time before 1970.
    pub fn compose(self, day: i64, minutes: u32) -> Option<u64> {
        todo::compose_due(day, minutes, self.at_local(day, minutes))
    }

    /// The local day number and minute of the day of a reminder time; the inverse of `compose`.
    pub fn split(self, due: u64) -> (i64, u32) {
        todo::split_due(due, self.at_utc(due))
    }
}

fn system_time(secs: i64) -> SYSTEMTIME {
    let (year, month, day) = calendar::civil_from_days(secs.div_euclid(86_400));
    let rest = secs.rem_euclid(86_400);
    SYSTEMTIME {
        wYear: year as u16,
        wMonth: month as u16,
        wDayOfWeek: 0,
        wDay: day as u16,
        wHour: (rest / 3600) as u16,
        wMinute: (rest % 3600 / 60) as u16,
        wSecond: (rest % 60) as u16,
        wMilliseconds: 0,
    }
}

fn seconds(t: &SYSTEMTIME) -> i64 {
    calendar::days_from_civil(i32::from(t.wYear), u32::from(t.wMonth), u32::from(t.wDay)) * 86_400
        + i64::from(t.wHour) * 3600
        + i64::from(t.wMinute) * 60
        + i64::from(t.wSecond)
}

/// Local minus UTC at the UTC moment `unix`, in `zone` (`None`: the time zone of this PC).
fn offset_at_utc(zone: Option<&TIME_ZONE_INFORMATION>, unix: i64) -> Option<i64> {
    let utc = system_time(unix);
    let mut local = SYSTEMTIME::default();
    // SAFETY: the pointers are to values that live through the call; Windows only reads `zone` and `utc`.
    unsafe { SystemTimeToTzSpecificLocalTime(zone.map(std::ptr::from_ref), &utc, &mut local).ok()? };
    Some(seconds(&local) - unix)
}

/// Local minus UTC at a local moment (seconds since 1970 on the local clock), in `zone`. A time the clocks skip over
/// or show twice gets the offset Windows picks for it.
fn offset_at_local(zone: Option<&TIME_ZONE_INFORMATION>, local_secs: i64) -> Option<i64> {
    let local = system_time(local_secs);
    let mut utc = SYSTEMTIME::default();
    // SAFETY: as above.
    unsafe { TzSpecificLocalTimeToSystemTime(zone.map(std::ptr::from_ref), &local, &mut utc).ok()? };
    Some(local_secs - seconds(&utc))
}

/// Local minus UTC right now, from the local clock against the system clock, rounded to a quarter hour. Only used
/// if Windows cannot convert.
fn offset_now() -> i64 {
    // SAFETY: a plain call that fills in a structure.
    let t = unsafe { GetLocalTime() };
    ((seconds(&t) - todo::unix_now() as i64) as f64 / 900.0).round() as i64 * 900
}

#[cfg(test)]
mod tests {
    use super::*;

    /// US Eastern time: UTC-5, and UTC-4 from the second Sunday of March to the first Sunday of November.
    fn eastern() -> TIME_ZONE_INFORMATION {
        let rule = |month: u16, week: u16| SYSTEMTIME { wMonth: month, wDayOfWeek: 0, wDay: week, wHour: 2, ..Default::default() };
        TIME_ZONE_INFORMATION {
            Bias: 300,
            StandardDate: rule(11, 1),
            StandardBias: 0,
            DaylightDate: rule(3, 2),
            DaylightBias: -60,
            ..Default::default()
        }
    }

    fn utc(year: i32, month: u32, day: u32, hour: i64) -> i64 {
        calendar::days_from_civil(year, month, day) * 86_400 + hour * 3600
    }

    #[test]
    fn the_offset_is_the_one_of_the_moment_asked_about() {
        let tz = eastern();
        // Before summer time starts (8 March 2026) it is UTC-5, after it UTC-4.
        assert_eq!(offset_at_utc(Some(&tz), utc(2026, 3, 1, 12)), Some(-5 * 3600));
        assert_eq!(offset_at_utc(Some(&tz), utc(2026, 4, 2, 12)), Some(-4 * 3600));
        assert_eq!(offset_at_utc(Some(&tz), utc(2026, 12, 1, 12)), Some(-5 * 3600));
        // The same from the local side: 09:00 on 2 April is summer time even though today is not.
        let local = |m, d| calendar::days_from_civil(2026, m, d) * 86_400 + 9 * 3600;
        assert_eq!(offset_at_local(Some(&tz), local(3, 1)), Some(-5 * 3600));
        assert_eq!(offset_at_local(Some(&tz), local(4, 2)), Some(-4 * 3600));
    }

    #[test]
    fn a_reminder_across_a_clock_change_keeps_its_local_time() {
        let tz = eastern();
        // 09:00 on 2 April, made in March: 13:00 UTC, because that day is on summer time.
        let day = calendar::days_from_civil(2026, 4, 2);
        let off = offset_at_local(Some(&tz), day * 86_400 + 9 * 3600).unwrap();
        let due = todo::compose_due(day, 9 * 60, off).unwrap();
        assert_eq!(due as i64, utc(2026, 4, 2, 13));
        // And read back with the offset of that moment, it is 09:00 again.
        let back = offset_at_utc(Some(&tz), due as i64).unwrap();
        assert_eq!(todo::split_due(due, back), (day, 9 * 60));
    }

    #[test]
    fn a_fixed_zone_is_the_plain_offset() {
        let zone = Zone::Fixed(8 * 3600);
        let day = calendar::days_from_civil(2026, 10, 28);
        let due = zone.compose(day, 9 * 60 + 30).unwrap();
        assert_eq!(due as i64, day * 86_400 + (9 * 60 + 30) * 60 - 8 * 3600);
        assert_eq!(zone.split(due), (day, 9 * 60 + 30));
        assert_eq!(Zone::Fixed(3600).compose(0, 0), None, "before 1970");
    }

    #[test]
    fn this_pcs_zone_gives_a_sane_offset() {
        let now = todo::unix_now();
        let off = Zone::System.at_utc(now);
        assert!(off.abs() <= 14 * 3600 && off % 900 == 0, "{off}");
        let (day, minutes) = Zone::System.split(now);
        assert_eq!(Zone::System.compose(day, minutes), Some(now / 60 * 60), "there and back");
    }
}
