//! Local time against UTC, for the times of reminders. How far the local clock is from UTC is not one number for the
//! whole year where the clocks change (summer time), so it is asked of Windows for the very moment that is converted:
//! a reminder set in March for a day in April is changed with April's offset, not March's.

#[cfg(windows)]
use windows::Win32::Foundation::SYSTEMTIME;
#[cfg(windows)]
use windows::Win32::System::SystemInformation::GetLocalTime;
#[cfg(windows)]
use windows::Win32::System::Time::{SystemTimeToTzSpecificLocalTime, TIME_ZONE_INFORMATION, TzSpecificLocalTimeToSystemTime};

#[cfg(any(windows, target_os = "linux", test))]
use crate::calendar;
use crate::todo;

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
            #[cfg(windows)]
            Zone::System => offset_at_utc(None, unix as i64).unwrap_or_else(offset_now),
            #[cfg(target_os = "linux")]
            Zone::System => linux::offset_at_utc(unix as i64),
            #[cfg(not(any(windows, target_os = "linux")))]
            Zone::System => {
                let _ = unix;
                0
            }
        }
    }

    /// Local time minus UTC, in seconds, at a local `day` and minute of the day.
    pub fn at_local(self, day: i64, minutes: u32) -> i64 {
        match self {
            Zone::Fixed(offset) => offset,
            #[cfg(windows)]
            Zone::System => offset_at_local(None, day * 86_400 + i64::from(minutes) * 60).unwrap_or_else(offset_now),
            #[cfg(target_os = "linux")]
            Zone::System => linux::offset_at_local(day * 86_400 + i64::from(minutes) * 60),
            #[cfg(not(any(windows, target_os = "linux")))]
            Zone::System => {
                let _ = (day, minutes);
                0
            }
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

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::path::Path;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    const CACHE_TTL: Duration = Duration::from_secs(5);

    #[derive(Clone, Copy, Debug)]
    pub(super) struct TzType {
        utoff: i64,
        is_dst: bool,
    }

    #[derive(Clone, Copy, Debug)]
    pub(super) enum DstDay {
        MonthWeekDay { month: u32, week: u32, weekday: u32 },
        JulianNoLeap(u32),
        JulianZero(u32),
    }

    #[derive(Clone, Copy, Debug)]
    pub(super) struct DstDate {
        day: DstDay,
        time_secs: i64,
    }

    impl DstDate {
        fn day_number(self, year: i32) -> i64 {
            match self.day {
                DstDay::MonthWeekDay { month, week, weekday } => {
                    let first_day = calendar::days_from_civil(year, month, 1);
                    let w1 = ((calendar::weekday(first_day) + 1) % 7) as u32;
                    let first_occ = 1 + (weekday + 7 - w1) % 7;
                    let dom = if week < 5 {
                        first_occ + (week - 1) * 7
                    } else {
                        let dim = calendar::days_in_month(year, month);
                        let candidate = first_occ + 28;
                        if candidate > dim { candidate - 7 } else { candidate }
                    };
                    calendar::days_from_civil(year, month, dom)
                }
                DstDay::JulianNoLeap(n) => {
                    let jan1 = calendar::days_from_civil(year, 1, 1);
                    if n <= 59 { jan1 + i64::from(n - 1) } else { calendar::days_from_civil(year, 3, 1) + i64::from(n - 60) }
                }
                DstDay::JulianZero(n) => calendar::days_from_civil(year, 1, 1) + i64::from(n),
            }
        }
    }

    #[derive(Clone, Debug)]
    pub(super) struct TzRule {
        pub(super) std_offset: i64,
        pub(super) dst_offset: Option<i64>,
        pub(super) dst_range: Option<(DstDate, DstDate)>,
    }

    impl TzRule {
        pub(super) fn offset_at_utc(&self, unix: i64) -> i64 {
            let (Some(dst_off), Some((start, end))) = (self.dst_offset, self.dst_range) else {
                return self.std_offset;
            };
            let (year, _, _) = calendar::civil_from_days(unix.saturating_add(self.std_offset.max(dst_off)).div_euclid(86_400));
            let start_utc = start.day_number(year) * 86_400 + start.time_secs - self.std_offset;
            let end_utc = end.day_number(year) * 86_400 + end.time_secs - dst_off;
            let in_dst = if start_utc <= end_utc { unix >= start_utc && unix < end_utc } else { unix < end_utc || unix >= start_utc };
            if in_dst { dst_off } else { self.std_offset }
        }
    }

    #[derive(Clone, Debug, Default)]
    pub(super) struct TzData {
        transitions: Vec<i64>,
        type_indices: Vec<u8>,
        types: Vec<TzType>,
        footer: Option<TzRule>,
    }

    impl TzData {
        fn utc() -> Self {
            Self { transitions: Vec::new(), type_indices: Vec::new(), types: vec![TzType { utoff: 0, is_dst: false }], footer: None }
        }

        pub(super) fn from_rule(rule: TzRule) -> Self {
            Self { transitions: Vec::new(), type_indices: Vec::new(), types: vec![TzType { utoff: rule.std_offset, is_dst: false }], footer: Some(rule) }
        }

        pub(super) fn parse_tzif(bytes: &[u8]) -> Option<Self> {
            let mut pos = 0usize;
            let hdr1 = parse_header(bytes, &mut pos)?;
            let skip_v1 = |count: usize, elem: usize| count.checked_mul(elem);
            if matches!(hdr1.version, b'2' | b'3' | b'4') {
                let v1_body = skip_v1(hdr1.timecnt, 5)?
                    .checked_add(skip_v1(hdr1.typecnt, 6)?)?
                    .checked_add(hdr1.charcnt)?
                    .checked_add(skip_v1(hdr1.leapcnt, 8)?)?
                    .checked_add(hdr1.isstdcnt)?
                    .checked_add(hdr1.isutcnt)?;
                pos = pos.checked_add(v1_body)?;
                let hdr2 = parse_header(bytes, &mut pos)?;
                let mut data = parse_body(bytes, &mut pos, &hdr2, 8)?;
                let v2_tail = hdr2.charcnt.checked_add(hdr2.leapcnt.checked_mul(12)?)?.checked_add(hdr2.isstdcnt)?.checked_add(hdr2.isutcnt)?;
                pos = pos.checked_add(v2_tail)?;
                if pos <= bytes.len()
                    && let Ok(rest) = std::str::from_utf8(&bytes[pos..])
                {
                    let trimmed = rest.trim();
                    if !trimmed.is_empty() {
                        data.footer = parse_posix_tz(trimmed);
                    }
                }
                Some(data)
            } else {
                parse_body(bytes, &mut pos, &hdr1, 4)
            }
        }

        pub(super) fn offset_at_utc(&self, unix: i64) -> i64 {
            if let Some(&first) = self.transitions.first() {
                if unix < first {
                    return self.types.iter().find(|t| !t.is_dst).or_else(|| self.types.first()).map_or(0, |t| t.utoff);
                }
                let last = *self.transitions.last().unwrap_or(&first);
                if unix < last || self.footer.is_none() {
                    let idx = self.transitions.partition_point(|&t| t <= unix);
                    let ti = self.type_indices.get(idx.saturating_sub(1)).copied().unwrap_or(0) as usize;
                    return self.types.get(ti).or_else(|| self.types.first()).map_or(0, |t| t.utoff);
                }
            }
            if let Some(rule) = &self.footer {
                return rule.offset_at_utc(unix);
            }
            self.types.first().map_or(0, |t| t.utoff)
        }

        pub(super) fn offset_at_local(&self, local_secs: i64) -> i64 {
            let o1 = self.offset_at_utc(local_secs);
            let o2 = self.offset_at_utc(local_secs - o1);
            if o2 != o1 {
                let o3 = self.offset_at_utc(local_secs - o2);
                if o3 == o2 {
                    return o2;
                }
            }
            o2
        }
    }

    struct TzHeader {
        version: u8,
        isutcnt: usize,
        isstdcnt: usize,
        leapcnt: usize,
        timecnt: usize,
        typecnt: usize,
        charcnt: usize,
    }

    fn take<'a>(bytes: &'a [u8], pos: &mut usize, len: usize) -> Option<&'a [u8]> {
        let end = pos.checked_add(len)?;
        let slice = bytes.get(*pos..end)?;
        *pos = end;
        Some(slice)
    }

    fn be_u32(bytes: &[u8], pos: &mut usize) -> Option<u32> {
        let b = take(bytes, pos, 4)?;
        Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn be_i32(bytes: &[u8], pos: &mut usize) -> Option<i32> {
        let b = take(bytes, pos, 4)?;
        Some(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn be_i64(bytes: &[u8], pos: &mut usize) -> Option<i64> {
        let b = take(bytes, pos, 8)?;
        Some(i64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    fn parse_header(bytes: &[u8], pos: &mut usize) -> Option<TzHeader> {
        let magic = take(bytes, pos, 4)?;
        if magic != b"TZif" {
            return None;
        }
        let version = *take(bytes, pos, 1)?.first()?;
        let _ = take(bytes, pos, 15)?;
        let isutcnt = be_u32(bytes, pos)? as usize;
        let isstdcnt = be_u32(bytes, pos)? as usize;
        let leapcnt = be_u32(bytes, pos)? as usize;
        let timecnt = be_u32(bytes, pos)? as usize;
        let typecnt = be_u32(bytes, pos)? as usize;
        let charcnt = be_u32(bytes, pos)? as usize;
        if typecnt == 0 {
            return None;
        }
        Some(TzHeader { version, isutcnt, isstdcnt, leapcnt, timecnt, typecnt, charcnt })
    }

    fn parse_body(bytes: &[u8], pos: &mut usize, hdr: &TzHeader, time_size: usize) -> Option<TzData> {
        let mut transitions = Vec::with_capacity(hdr.timecnt.min(4096));
        for _ in 0..hdr.timecnt {
            let t = if time_size == 8 { be_i64(bytes, pos)? } else { i64::from(be_i32(bytes, pos)?) };
            transitions.push(t);
        }
        let type_indices = take(bytes, pos, hdr.timecnt)?.to_vec();
        let mut types = Vec::with_capacity(hdr.typecnt.min(64));
        for _ in 0..hdr.typecnt {
            let utoff = i64::from(be_i32(bytes, pos)?);
            let is_dst = *take(bytes, pos, 1)?.first()? != 0;
            let _ = take(bytes, pos, 1)?;
            types.push(TzType { utoff, is_dst });
        }
        Some(TzData { transitions, type_indices, types, footer: None })
    }

    fn consume_abbrev(s: &str) -> Option<&str> {
        if let Some(rest) = s.strip_prefix('<') {
            let (_, after) = rest.split_once('>')?;
            Some(after)
        } else {
            let end = s.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(s.len());
            (end >= 3).then(|| &s[end..])
        }
    }

    fn parse_uint(s: &str) -> Option<(u32, &str)> {
        let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
        (end > 0).then(|| (s[..end].parse::<u32>().ok(), &s[end..])).and_then(|(v, rest)| Some((v?, rest)))
    }

    /// Parse a POSIX TZ offset `[+-]hh[:mm[:ss]]` into seconds to *add* to UTC (POSIX flips the sign).
    fn parse_posix_offset(mut s: &str) -> Option<(i64, &str)> {
        let sign: i64 = if let Some(rest) = s.strip_prefix('-') {
            s = rest;
            1
        } else if let Some(rest) = s.strip_prefix('+') {
            s = rest;
            -1
        } else {
            -1
        };
        let (hh, rest) = parse_uint(s)?;
        s = rest;
        let (mm, rest) = if let Some(after) = s.strip_prefix(':') { parse_uint(after)? } else { (0, s) };
        s = rest;
        let (ss, rest) = if let Some(after) = s.strip_prefix(':') { parse_uint(after)? } else { (0, s) };
        Some((sign * (i64::from(hh) * 3600 + i64::from(mm) * 60 + i64::from(ss)), rest))
    }

    fn parse_dst_time(mut s: &str) -> Option<(i64, &str)> {
        let neg = if let Some(rest) = s.strip_prefix('-') {
            s = rest;
            true
        } else {
            false
        };
        let (hh, rest) = parse_uint(s)?;
        s = rest;
        let (mm, rest) = if let Some(after) = s.strip_prefix(':') { parse_uint(after)? } else { (0, s) };
        s = rest;
        let (ss, rest) = if let Some(after) = s.strip_prefix(':') { parse_uint(after)? } else { (0, s) };
        let secs = i64::from(hh) * 3600 + i64::from(mm) * 60 + i64::from(ss);
        Some((if neg { -secs } else { secs }, rest))
    }

    fn parse_dst_date(s: &str) -> Option<(DstDate, &str)> {
        let (day, mut rest) = if let Some(after_m) = s.strip_prefix('M') {
            let (month, r) = parse_uint(after_m)?;
            let (week, r) = parse_uint(r.strip_prefix('.')?)?;
            let (weekday, r) = parse_uint(r.strip_prefix('.')?)?;
            if !(1..=12).contains(&month) || !(1..=5).contains(&week) || weekday > 6 {
                return None;
            }
            (DstDay::MonthWeekDay { month, week, weekday }, r)
        } else if let Some(after_j) = s.strip_prefix('J') {
            let (n, r) = parse_uint(after_j)?;
            if !(1..=365).contains(&n) {
                return None;
            }
            (DstDay::JulianNoLeap(n), r)
        } else {
            let (n, r) = parse_uint(s)?;
            if n > 365 {
                return None;
            }
            (DstDay::JulianZero(n), r)
        };
        let time_secs = if let Some(after_slash) = rest.strip_prefix('/') {
            let (t, r) = parse_dst_time(after_slash)?;
            rest = r;
            t
        } else {
            2 * 3600
        };
        Some((DstDate { day, time_secs }, rest))
    }

    pub(super) fn parse_posix_tz(s: &str) -> Option<TzRule> {
        let rest = consume_abbrev(s.trim())?;
        let (std_offset, rest) = parse_posix_offset(rest)?;
        if rest.is_empty() {
            return Some(TzRule { std_offset, dst_offset: None, dst_range: None });
        }
        let rest = consume_abbrev(rest)?;
        let (dst_offset, rest) = if rest.starts_with(',') || rest.is_empty() { (std_offset + 3600, rest) } else { parse_posix_offset(rest)? };
        if rest.is_empty() {
            return Some(TzRule { std_offset, dst_offset: Some(dst_offset), dst_range: None });
        }
        let rest = rest.strip_prefix(',')?;
        let (start, rest) = parse_dst_date(rest)?;
        let rest = rest.strip_prefix(',')?;
        let (end, rest) = parse_dst_date(rest)?;
        if !rest.is_empty() {
            return None;
        }
        Some(TzRule { std_offset, dst_offset: Some(dst_offset), dst_range: Some((start, end)) })
    }

    fn read_tzif_file(path: &Path) -> Option<TzData> {
        let bytes = std::fs::read(path).ok()?;
        TzData::parse_tzif(&bytes)
    }

    fn load_system_tz(tz_env: Option<&str>) -> TzData {
        if let Some(val) = tz_env.map(str::trim).filter(|s| !s.is_empty()) {
            let spec = val.strip_prefix(':').unwrap_or(val);
            if spec.starts_with('/') {
                if let Some(data) = read_tzif_file(Path::new(spec)) {
                    return data;
                }
            } else {
                let candidate = Path::new("/usr/share/zoneinfo").join(spec);
                if let Some(data) = read_tzif_file(&candidate) {
                    return data;
                }
                if let Some(rule) = parse_posix_tz(spec) {
                    return TzData::from_rule(rule);
                }
            }
        }
        read_tzif_file(Path::new("/etc/localtime")).unwrap_or_else(TzData::utc)
    }

    struct CachedTz {
        tz_env: Option<String>,
        loaded_at: Instant,
        data: TzData,
    }

    static CACHE: Mutex<Option<CachedTz>> = Mutex::new(None);

    fn with_system_tz<T>(f: impl FnOnce(&TzData) -> T) -> T {
        let tz_env = std::env::var("TZ").ok();
        let now = Instant::now();
        let Ok(mut guard) = CACHE.lock() else {
            return f(&load_system_tz(tz_env.as_deref()));
        };
        let stale = match guard.as_ref() {
            Some(c) => c.tz_env != tz_env || now.duration_since(c.loaded_at) >= CACHE_TTL,
            None => true,
        };
        if stale {
            let data = load_system_tz(tz_env.as_deref());
            *guard = Some(CachedTz { tz_env, loaded_at: now, data });
        }
        f(&guard.as_ref().unwrap().data)
    }

    pub(super) fn offset_at_utc(unix: i64) -> i64 {
        with_system_tz(|tz| tz.offset_at_utc(unix))
    }

    pub(super) fn offset_at_local(local_secs: i64) -> i64 {
        with_system_tz(|tz| tz.offset_at_local(local_secs))
    }
}

#[cfg(windows)]
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

#[cfg(windows)]
fn seconds(t: &SYSTEMTIME) -> i64 {
    calendar::days_from_civil(i32::from(t.wYear), u32::from(t.wMonth), u32::from(t.wDay)) * 86_400 + i64::from(t.wHour) * 3600 + i64::from(t.wMinute) * 60 + i64::from(t.wSecond)
}

/// Local minus UTC at the UTC moment `unix`, in `zone` (`None`: the time zone of this PC).
#[cfg(windows)]
fn offset_at_utc(zone: Option<&TIME_ZONE_INFORMATION>, unix: i64) -> Option<i64> {
    let utc = system_time(unix);
    let mut local = SYSTEMTIME::default();
    // SAFETY: the pointers are to values that live through the call; Windows only reads `zone` and `utc`.
    unsafe { SystemTimeToTzSpecificLocalTime(zone.map(std::ptr::from_ref), &utc, &mut local).ok()? };
    Some(seconds(&local) - unix)
}

/// Local minus UTC at a local moment (seconds since 1970 on the local clock), in `zone`. A time the clocks skip over
/// or show twice gets the offset Windows picks for it.
#[cfg(windows)]
fn offset_at_local(zone: Option<&TIME_ZONE_INFORMATION>, local_secs: i64) -> Option<i64> {
    let local = system_time(local_secs);
    let mut utc = SYSTEMTIME::default();
    // SAFETY: as above.
    unsafe { TzSpecificLocalTimeToSystemTime(zone.map(std::ptr::from_ref), &local, &mut utc).ok()? };
    Some(local_secs - seconds(&utc))
}

/// Local minus UTC right now, from the local clock against the system clock, rounded to a quarter hour. Only used
/// if Windows cannot convert.
#[cfg(windows)]
fn offset_now() -> i64 {
    // SAFETY: a plain call that fills in a structure.
    let t = unsafe { GetLocalTime() };
    ((seconds(&t) - todo::unix_now() as i64) as f64 / 900.0).round() as i64 * 900
}

#[cfg(test)]
mod tests {
    use super::*;

    /// US Eastern time: UTC-5, and UTC-4 from the second Sunday of March to the first Sunday of November.
    #[cfg(windows)]
    fn eastern() -> TIME_ZONE_INFORMATION {
        let rule = |month: u16, week: u16| SYSTEMTIME { wMonth: month, wDayOfWeek: 0, wDay: week, wHour: 2, ..Default::default() };
        TIME_ZONE_INFORMATION { Bias: 300, StandardDate: rule(11, 1), StandardBias: 0, DaylightDate: rule(3, 2), DaylightBias: -60, ..Default::default() }
    }

    #[cfg(windows)]
    fn utc(year: i32, month: u32, day: u32, hour: i64) -> i64 {
        calendar::days_from_civil(year, month, day) * 86_400 + hour * 3600
    }

    #[cfg(windows)]
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

    #[cfg(windows)]
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

    #[cfg(target_os = "linux")]
    #[test]
    fn posix_tz_handles_fixed_northern_and_southern_dst() {
        let sg = linux::TzData::from_rule(linux::parse_posix_tz("<+08>-8").unwrap());
        assert_eq!(sg.offset_at_utc(calendar::days_from_civil(2026, 6, 1) * 86_400), 8 * 3600);

        let ist = linux::TzData::from_rule(linux::parse_posix_tz("IST-5:30").unwrap());
        assert_eq!(ist.offset_at_utc(0), 5 * 3600 + 1800);

        let est = linux::TzData::from_rule(linux::parse_posix_tz("EST5EDT,M3.2.0,M11.1.0").unwrap());
        let utc = |y: i32, m: u32, d: u32, h: i64| calendar::days_from_civil(y, m, d) * 86_400 + h * 3600;
        assert_eq!(est.offset_at_utc(utc(2026, 3, 1, 12)), -5 * 3600);
        assert_eq!(est.offset_at_utc(utc(2026, 4, 2, 12)), -4 * 3600);
        assert_eq!(est.offset_at_utc(utc(2026, 12, 1, 12)), -5 * 3600);

        let apr2 = calendar::days_from_civil(2026, 4, 2);
        let local_apr2 = apr2 * 86_400 + 9 * 3600;
        let off = est.offset_at_local(local_apr2);
        assert_eq!(off, -4 * 3600);
        let due = todo::compose_due(apr2, 9 * 60, off).unwrap();
        assert_eq!(due as i64, utc(2026, 4, 2, 13));
        assert_eq!(todo::split_due(due, est.offset_at_utc(due as i64)), (apr2, 9 * 60));

        let syd = linux::TzData::from_rule(linux::parse_posix_tz("AEST-10AEDT,M10.1.0,M4.1.0/3").unwrap());
        assert_eq!(syd.offset_at_utc(utc(2026, 1, 15, 0)), 11 * 3600);
        assert_eq!(syd.offset_at_utc(utc(2026, 7, 15, 0)), 10 * 3600);
        assert_eq!(syd.offset_at_utc(utc(2026, 12, 15, 0)), 11 * 3600);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn tzif_parser_reads_system_zoneinfo_and_rejects_garbage() {
        assert!(linux::TzData::parse_tzif(b"").is_none());
        assert!(linux::TzData::parse_tzif(b"NOTZ").is_none());
        assert!(linux::TzData::parse_tzif(b"TZif2\0").is_none());

        let ny_path = std::path::Path::new("/usr/share/zoneinfo/America/New_York");
        if let Ok(bytes) = std::fs::read(ny_path) {
            let ny = linux::TzData::parse_tzif(&bytes).expect("parse America/New_York");
            let utc = |y: i32, m: u32, d: u32, h: i64| calendar::days_from_civil(y, m, d) * 86_400 + h * 3600;
            assert_eq!(ny.offset_at_utc(utc(2026, 3, 1, 12)), -5 * 3600);
            assert_eq!(ny.offset_at_utc(utc(2026, 4, 2, 12)), -4 * 3600);
            assert_eq!(ny.offset_at_local(calendar::days_from_civil(2026, 4, 2) * 86_400 + 9 * 3600), -4 * 3600);
            assert_eq!(ny.offset_at_local(calendar::days_from_civil(2026, 3, 8) * 86_400 + 3 * 3600 + 1800), -4 * 3600);
        }

        let local_path = std::path::Path::new("/etc/localtime");
        if let Ok(bytes) = std::fs::read(local_path) {
            let local = linux::TzData::parse_tzif(&bytes).expect("parse /etc/localtime");
            let now = todo::unix_now();
            assert_eq!(local.offset_at_utc(now as i64), Zone::System.at_utc(now));
        }
    }
}
