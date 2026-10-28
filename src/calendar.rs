//! Month calendar maths and the state of the Calendar tab. A date is a day number: the days
//! since 1970-01-01 in local time, so tasks can store one plain integer.

const MONTHS: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
/// Column heads of the grid; weeks start on Monday.
pub const WEEK_HEADS: [&str; 7] = ["M", "T", "W", "T", "F", "S", "S"];
/// Six weeks always cover a month.
pub const CELLS: usize = 42;

/// Day number of a calendar date (proleptic Gregorian).
pub fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = i64::from(if month <= 2 { year - 1 } else { year });
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (i64::from(month) + 9) % 12; // March is 0
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The (year, month, day) of a day number.
pub fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year as i32, month as u32, day as u32)
}

/// 0 = Monday ... 6 = Sunday.
pub fn weekday(days: i64) -> usize {
    (days + 3).rem_euclid(7) as usize // 1970-01-01 was a Thursday
}

pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// "Wed, 28 Oct" for the heading of the day panel.
pub fn long_date(days: i64) -> String {
    let (_, m, d) = civil_from_days(days);
    format!("{}, {} {}", WEEKDAYS[weekday(days)], d, &MONTHS[m as usize - 1][..3])
}

/// "28 Oct", for a task row.
pub fn short_date(days: i64) -> String {
    let (_, m, d) = civil_from_days(days);
    format!("{} {}", d, &MONTHS[m as usize - 1][..3])
}

pub struct Calendar {
    pub year: i32,
    pub month: u32,
    pub selected: i64,
}

impl Calendar {
    /// Showing the month of `today`, with today selected.
    pub fn new(today: i64) -> Self {
        let (year, month, _) = civil_from_days(today);
        Self { year, month, selected: today }
    }

    /// "October 2026".
    pub fn title(&self) -> String {
        format!("{} {}", MONTHS[self.month as usize - 1], self.year)
    }

    /// Show another month, keeping the same day of the month where it has one.
    pub fn shift_month(&mut self, delta: i32) {
        let (_, _, day) = civil_from_days(self.selected);
        let index = self.year * 12 + self.month as i32 - 1 + delta;
        self.year = index.div_euclid(12);
        self.month = index.rem_euclid(12) as u32 + 1;
        self.selected = days_from_civil(self.year, self.month, day.min(days_in_month(self.year, self.month)));
    }

    /// The day number in grid cell `i` (row by row), or `None` for the blanks around the month.
    pub fn cell(&self, i: usize) -> Option<i64> {
        let first = days_from_civil(self.year, self.month, 1);
        let day = i as i64 - weekday(first) as i64;
        (0..i64::from(days_in_month(self.year, self.month))).contains(&day).then_some(first + day)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_numbers_round_trip() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        for days in (-800..40_000).step_by(7) {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days, "{y}-{m}-{d}");
        }
    }

    #[test]
    fn weekdays_are_right() {
        assert_eq!(weekday(0), 3, "1970-01-01 was a Thursday");
        assert_eq!(weekday(days_from_civil(2026, 10, 28)), 2, "28 Oct 2026 is a Wednesday");
        assert_eq!(long_date(days_from_civil(2026, 10, 28)), "Wed, 28 Oct");
        assert_eq!(short_date(days_from_civil(2026, 1, 5)), "5 Jan");
    }

    #[test]
    fn month_lengths_follow_leap_rules() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2100, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(2026, 4), 30);
        assert_eq!(days_in_month(2026, 12), 31);
    }

    #[test]
    fn grid_starts_on_the_right_weekday_and_has_every_day() {
        let cal = Calendar::new(days_from_civil(2026, 10, 28));
        // 1 Oct 2026 is a Thursday, so the first three cells are blank.
        assert_eq!((0..3).filter_map(|i| cal.cell(i)).count(), 0);
        assert_eq!(cal.cell(3), Some(days_from_civil(2026, 10, 1)));
        assert_eq!((0..CELLS).filter_map(|i| cal.cell(i)).count(), 31);
        assert_eq!(cal.title(), "October 2026");
    }

    #[test]
    fn changing_month_clamps_the_selected_day_and_wraps_years() {
        let mut cal = Calendar::new(days_from_civil(2026, 1, 31));
        cal.shift_month(1);
        assert_eq!(civil_from_days(cal.selected), (2026, 2, 28));
        cal.shift_month(-2);
        assert_eq!((cal.year, cal.month), (2025, 12));
        assert_eq!(civil_from_days(cal.selected), (2025, 12, 28));
        cal.shift_month(14);
        assert_eq!((cal.year, cal.month), (2027, 2));
    }
}
