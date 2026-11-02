//! Prayer times. They come from Aladhan (aladhan.com, no account, no key) for the user's city and the calculation
//! method they chose, a month at a time. The days are kept in `prayer.json`, so the tab and the reminders work
//! offline, and fetched again once in a while to stay ahead of the calendar.

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use winit::event_loop::EventLoopProxy;

use crate::weather::Place;
use crate::{UserEvent, calendar, store};

const CALENDAR: &str = "https://api.aladhan.com/v1/calendar";
/// How often the days are fetched again while the Prayer tab is on.
const EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// The table has to reach this many days past today, or it is fetched again.
pub const AHEAD: i64 = 2;

/// The times of a day, in this order. Sunrise is shown but is not a prayer.
pub const NAMES: [&str; 6] = ["Fajr", "Sunrise", "Dhuhr", "Asr", "Maghrib", "Isha"];
pub const SUNRISE: usize = 1;

/// The calculation methods Aladhan has (its numbers), the common ones first.
pub const METHODS: [(u32, &str); 23] = [
    (17, "JAKIM (Malaysia)"),
    (11, "MUIS (Singapore)"),
    (20, "Kemenag (Indonesia)"),
    (3, "Muslim World League"),
    (4, "Umm al-Qura (Makkah)"),
    (2, "ISNA (North America)"),
    (5, "Egyptian General Authority"),
    (1, "Karachi (UISK)"),
    (13, "Diyanet (Turkey)"),
    (8, "Gulf Region"),
    (9, "Kuwait"),
    (10, "Qatar"),
    (16, "Dubai"),
    (12, "France (UOIF)"),
    (14, "Russia (SAMR)"),
    (15, "Moonsighting Committee"),
    (0, "Shia Ithna-Ashari (Jafari)"),
    (7, "Tehran (Geophysics)"),
    (18, "Tunisia"),
    (19, "Algeria"),
    (21, "Morocco"),
    (22, "Portugal"),
    (23, "Jordan"),
];

/// The method that goes with a country (its ISO code), for when a city is chosen. Anywhere else: the Muslim World League's.
pub fn default_method(country: &str) -> u32 {
    match country.to_ascii_uppercase().as_str() {
        "MY" => 17,
        "SG" => 11,
        "ID" => 20,
        "SA" => 4,
        "US" | "CA" => 2,
        "EG" => 5,
        "PK" | "IN" | "BD" | "AF" => 1,
        "TR" => 13,
        "AE" => 16,
        "QA" => 10,
        "KW" => 9,
        "BH" | "OM" => 8,
        "FR" => 12,
        "RU" => 14,
        "IR" => 7,
        "TN" => 18,
        "DZ" => 19,
        "MA" => 21,
        "PT" => 22,
        "JO" => 23,
        _ => 3,
    }
}

/// A method number if it is one Aladhan has, else the Muslim World League's.
pub fn known_method(id: u32) -> u32 {
    if METHODS.iter().any(|(m, _)| *m == id) { id } else { 3 }
}

/// The position of a method in `METHODS`.
pub fn method_index(id: u32) -> usize {
    METHODS.iter().position(|(m, _)| *m == known_method(id)).unwrap_or(0)
}

/// One day: the minutes since midnight of each time in `NAMES`, and the Hijri date as (day, month, year).
#[derive(Clone, PartialEq, Debug)]
pub struct Day {
    /// The day number, as the calendar counts them.
    pub date: i64,
    pub times: [u32; 6],
    pub hijri: (u32, u32, u32),
}

/// What to ask for: the place, the calculation method and the Asr school (0 standard, 1 Hanafi).
#[derive(Clone, PartialEq, Debug)]
pub struct Request {
    pub place: Place,
    pub method: u32,
    pub school: u32,
}

/// The days that were fetched for a request.
#[derive(Clone, PartialEq, Debug)]
pub struct Table {
    pub lat: f64,
    pub lon: f64,
    pub method: u32,
    pub school: u32,
    /// In order of date.
    pub days: Vec<Day>,
}

impl Table {
    /// Whether the table was fetched for this request.
    pub fn is_for(&self, req: &Request) -> bool {
        req.place.same_spot(self.lat, self.lon) && self.method == req.method && self.school == req.school
    }

    pub fn day(&self, date: i64) -> Option<&Day> {
        self.days.iter().find(|d| d.date == date)
    }

    /// Whether it has `today` and the `ahead` days after it.
    pub fn covers(&self, today: i64, ahead: i64) -> bool {
        (today..=today + ahead).all(|d| self.day(d).is_some())
    }
}

/// A prayer that is still to come.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Next {
    /// An index into `NAMES`.
    pub index: usize,
    pub date: i64,
    /// Minutes since midnight.
    pub minute: u32,
}

/// The five prayers' places in `NAMES`.
fn is_prayer(index: usize) -> bool {
    index != SUNRISE
}

/// The next prayer after a moment (`now_secs` seconds into `today`): the first of today's still to come, else tomorrow's first.
pub fn next_prayer(table: &Table, today: i64, now_secs: u32) -> Option<Next> {
    for date in [today, today + 1] {
        let Some(day) = table.day(date) else { continue };
        let found = (0..NAMES.len()).filter(|i| is_prayer(*i)).find(|&i| date > today || day.times[i] * 60 > now_secs);
        if let Some(index) = found {
            return Some(Next { index, date, minute: day.times[index] });
        }
    }
    None
}

/// How many seconds from the moment to a prayer.
pub fn secs_until(next: &Next, today: i64, now_secs: u32) -> i64 {
    (next.date - today) * 86_400 + i64::from(next.minute) * 60 - i64::from(now_secs)
}

/// A prayer whose time came in the last two minutes, for the notice. (A time passed long ago, such as when the
/// PC wakes in the afternoon, is not announced.)
pub fn just_arrived(table: &Table, today: i64, now_secs: u32) -> Option<(usize, u32)> {
    let day = table.day(today)?;
    (0..NAMES.len())
        .filter(|i| is_prayer(*i))
        .filter(|&i| day.times[i] * 60 <= now_secs && now_secs - day.times[i] * 60 < 120)
        .max_by_key(|&i| day.times[i])
        .map(|i| (i, day.times[i]))
}

/// "in 1 h 23 min": the time until a moment, to the minute and rounded up.
pub fn countdown(secs: i64) -> String {
    let mins = (secs.max(0) + 59) / 60;
    match (mins / 60, mins % 60) {
        (0, m) => format!("in {} min", m.max(1)),
        (h, 0) => format!("in {h} h"),
        (h, m) => format!("in {h} h {m} min"),
    }
}

/// The Hijri months' names.
const HIJRI_MONTHS: [&str; 12] = [
    "Muharram",
    "Safar",
    "Rabi al-awwal",
    "Rabi al-thani",
    "Jumada al-ula",
    "Jumada al-akhirah",
    "Rajab",
    "Shaban",
    "Ramadan",
    "Shawwal",
    "Dhu al-Qadah",
    "Dhu al-Hijjah",
];

/// "21 Jumada al-ula 1448".
pub fn hijri_text(h: (u32, u32, u32)) -> String {
    match HIJRI_MONTHS.get((h.1 as usize).wrapping_sub(1)) {
        Some(month) => format!("{} {month} {}", h.0, h.2),
        None => String::new(),
    }
}

/// "05:37 (+08)" as 337 minutes.
fn minutes_of(text: &str) -> Option<u32> {
    let (h, m) = text.get(..5)?.split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// The days of one month of Aladhan's calendar answer. A day that cannot be read is left out.
fn parse_month(v: &Value) -> Vec<Day> {
    let Some(list) = v.get("data").and_then(Value::as_array) else { return Vec::new() };
    list.iter().filter_map(parse_day).collect()
}

fn parse_day(d: &Value) -> Option<Day> {
    let timings = d.get("timings")?;
    let mut times = [0u32; 6];
    for (slot, key) in times.iter_mut().zip(["Fajr", "Sunrise", "Dhuhr", "Asr", "Maghrib", "Isha"]) {
        *slot = minutes_of(timings.get(key)?.as_str()?)?;
    }
    let mut g = d.get("date")?.get("gregorian")?.get("date")?.as_str()?.split('-').map(|p| p.parse::<u32>().ok());
    let (Some(Some(day)), Some(Some(month)), Some(Some(year))) = (g.next(), g.next(), g.next()) else { return None };
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let hijri = d.get("date")?.get("hijri");
    let number = |v: Option<&Value>| v.and_then(Value::as_str).and_then(|s| s.parse::<u32>().ok()).or_else(|| v.and_then(Value::as_u64).map(|n| n as u32));
    let hijri = (
        number(hijri.and_then(|h| h.get("day"))).unwrap_or(0),
        hijri.and_then(|h| h.get("month")).and_then(|m| m.get("number")).and_then(Value::as_u64).unwrap_or(0) as u32,
        number(hijri.and_then(|h| h.get("year"))).unwrap_or(0),
    );
    Some(Day { date: calendar::days_from_civil(year as i32, month, day), times, hijri })
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(10))).build().into()
}

fn fetch_month(req: &Request, year: i32, month: u32) -> Option<Vec<Day>> {
    let mut call = agent()
        .get(format!("{CALENDAR}/{year}/{month}"))
        .header("User-Agent", concat!("Sky-Island/", env!("CARGO_PKG_VERSION")))
        .query("latitude", req.place.lat.to_string())
        .query("longitude", req.place.lon.to_string())
        .query("method", req.method.to_string())
        .query("school", req.school.to_string());
    if !req.place.tz.is_empty() {
        call = call.query("timezonestring", &req.place.tz);
    }
    let json = call.call().ok()?.body_mut().read_json::<Value>().ok()?;
    let days = parse_month(&json);
    (!days.is_empty()).then_some(days)
}

/// This month and the next, from Aladhan. `None` unless this month came.
fn fetch(req: &Request, today: i64) -> Option<Table> {
    let (year, month, _) = calendar::civil_from_days(today);
    let mut days = fetch_month(req, year, month)?;
    let (ny, nm) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    // Next month is only a head start: this month alone is still a table.
    days.extend(fetch_month(req, ny, nm).unwrap_or_default());
    days.sort_by_key(|d| d.date);
    days.dedup_by_key(|d| d.date);
    Some(Table { lat: req.place.lat, lon: req.place.lon, method: req.method, school: req.school, days })
}

/// Start the thread that fetches the days. It fetches when it is told what to ask for (`Some`) if the table it
/// has is not for that or does not reach far enough, and checks again every few hours; `None` stops it. Each answer
/// comes back as `UserEvent::Prayer` (`None` when it did not work).
pub fn spawn(proxy: EventLoopProxy<UserEvent>) -> Sender<Option<Request>> {
    let (tx, rx) = mpsc::channel::<Option<Request>>();
    thread::spawn(move || {
        let mut request: Option<Request> = None;
        loop {
            match rx.recv_timeout(EVERY) {
                Ok(next) => request = next,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            let Some(req) = &request else { continue };
            let table = fetch(req, crate::today_day());
            if let Some(t) = &table {
                save_cache(t);
            }
            if proxy.send_event(UserEvent::Prayer(table)).is_err() {
                return;
            }
        }
    });
    tx
}

/// Delete the days that were kept for the city.
pub fn clear_cache() {
    if let Some(path) = store::path("prayer.json") {
        let _ = std::fs::remove_file(path);
    }
}

fn cache_json(t: &Table) -> Value {
    json!({
        "lat": t.lat, "lon": t.lon, "method": t.method, "school": t.school,
        "days": t.days.iter().map(|d| json!({ "date": d.date, "times": d.times, "hijri": [d.hijri.0, d.hijri.1, d.hijri.2] })).collect::<Vec<_>>(),
    })
}

fn from_cache(v: &Value) -> Option<Table> {
    let days = v
        .get("days")?
        .as_array()?
        .iter()
        .filter_map(|d| {
            let times: Vec<u32> = d.get("times")?.as_array()?.iter().filter_map(|t| t.as_u64().filter(|m| *m < 1440).map(|m| m as u32)).collect();
            let h: Vec<u32> = d.get("hijri")?.as_array()?.iter().filter_map(|n| n.as_u64().map(|n| n as u32)).collect();
            Some(Day { date: d.get("date")?.as_i64()?, times: times.try_into().ok()?, hijri: (*h.first()?, *h.get(1)?, *h.get(2)?) })
        })
        .collect();
    Some(Table {
        lat: v.get("lat")?.as_f64()?,
        lon: v.get("lon")?.as_f64()?,
        method: v.get("method")?.as_u64()? as u32,
        school: v.get("school")?.as_u64()? as u32,
        days,
    })
}

/// The days kept from last time, if they are for this request.
pub fn load_cache(req: &Request) -> Option<Table> {
    let t = from_cache(&store::read_json(&store::path("prayer.json")?)?)?;
    t.is_for(req).then_some(t)
}

fn save_cache(t: &Table) {
    if cfg!(test) {
        return;
    }
    if let Some(path) = store::path("prayer.json") {
        store::write(&path, &cache_json(t).to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(date: i64, times: [u32; 6]) -> Day {
        Day { date, times, hijri: (21, 5, 1448) }
    }

    /// Fajr 5:37, sunrise 6:57, Dhuhr 12:57, Asr 16:19, Maghrib 18:57, Isha 20:08.
    const T: [u32; 6] = [337, 417, 777, 979, 1137, 1208];

    fn table() -> Table {
        Table { lat: 3.1412, lon: 101.69, method: 17, school: 0, days: vec![day(100, T), day(101, T), day(102, T)] }
    }

    fn secs(h: u32, m: u32) -> u32 {
        h * 3600 + m * 60
    }

    #[test]
    fn the_next_prayer_is_the_first_still_to_come() {
        let t = table();
        let n = next_prayer(&t, 100, secs(4, 0)).unwrap();
        assert_eq!((NAMES[n.index], n.date, n.minute), ("Fajr", 100, 337));
        assert_eq!(NAMES[next_prayer(&t, 100, secs(6, 0)).unwrap().index], "Dhuhr", "sunrise is not a prayer");
        assert_eq!(NAMES[next_prayer(&t, 100, secs(13, 0)).unwrap().index], "Asr");
        assert_eq!(NAMES[next_prayer(&t, 100, secs(19, 0)).unwrap().index], "Isha");
    }

    #[test]
    fn after_isha_the_next_prayer_is_fajr_tomorrow() {
        let n = next_prayer(&table(), 100, secs(21, 0)).unwrap();
        assert_eq!((NAMES[n.index], n.date), ("Fajr", 101));
        assert_eq!(secs_until(&n, 100, secs(21, 0)), 8 * 3600 + 37 * 60);
        let last = Table { days: vec![day(100, T)], ..table() };
        assert!(next_prayer(&last, 100, secs(21, 0)).is_none(), "no tomorrow in the table");
    }

    #[test]
    fn at_the_minute_a_prayer_is_no_longer_to_come() {
        let t = table();
        assert_eq!(NAMES[next_prayer(&t, 100, secs(16, 18) + 59).unwrap().index], "Asr");
        assert_eq!(NAMES[next_prayer(&t, 100, secs(16, 19)).unwrap().index], "Maghrib");
    }

    #[test]
    fn a_prayer_is_announced_when_it_arrives_and_not_long_after() {
        let t = table();
        assert_eq!(just_arrived(&t, 100, secs(16, 19)), Some((3, 979)));
        assert_eq!(just_arrived(&t, 100, secs(16, 20) + 59), Some((3, 979)));
        assert_eq!(just_arrived(&t, 100, secs(16, 21)), None, "two minutes later it is old news");
        assert_eq!(just_arrived(&t, 100, secs(16, 18)), None, "not yet");
        assert_eq!(just_arrived(&t, 100, secs(6, 57)), None, "sunrise is not announced");
        assert_eq!(just_arrived(&t, 105, secs(16, 19)), None, "a day the table lacks");
    }

    #[test]
    fn the_countdown_rounds_up_to_the_minute() {
        assert_eq!(countdown(0), "in 1 min");
        assert_eq!(countdown(1), "in 1 min");
        assert_eq!(countdown(60), "in 1 min");
        assert_eq!(countdown(61), "in 2 min");
        assert_eq!(countdown(3600), "in 1 h");
        assert_eq!(countdown(3600 + 23 * 60), "in 1 h 23 min");
        assert_eq!(countdown(-5), "in 1 min");
    }

    #[test]
    fn times_are_read_from_aladhans_text() {
        assert_eq!(minutes_of("05:37 (+08)"), Some(337));
        assert_eq!(minutes_of("20:08"), Some(1208));
        assert_eq!(minutes_of("24:00"), None);
        assert_eq!(minutes_of("5:37"), None);
        assert_eq!(minutes_of("soon"), None);
    }

    fn answer() -> Value {
        json!({ "data": [{
            "timings": { "Fajr": "05:37 (+08)", "Sunrise": "06:57 (+08)", "Dhuhr": "12:57 (+08)", "Asr": "16:19 (+08)", "Maghrib": "18:57 (+08)", "Isha": "20:08 (+08)" },
            "date": { "gregorian": { "date": "01-11-2026" }, "hijri": { "day": "21", "month": { "number": 5 }, "year": "1448" } },
        }, { "timings": { "Fajr": "bad" }, "date": { "gregorian": { "date": "02-11-2026" } } }] })
    }

    #[test]
    fn a_month_is_read_and_a_bad_day_is_left_out() {
        let days = parse_month(&answer());
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].times, T);
        assert_eq!(days[0].hijri, (21, 5, 1448));
        assert_eq!(calendar::civil_from_days(days[0].date), (2026, 11, 1));
        assert!(parse_month(&json!({})).is_empty());
    }

    #[test]
    fn the_hijri_date_is_written_out() {
        assert_eq!(hijri_text((21, 5, 1448)), "21 Jumada al-ula 1448");
        assert_eq!(hijri_text((1, 0, 1448)), "");
        assert_eq!(hijri_text((1, 13, 1448)), "");
    }

    #[test]
    fn a_saved_table_reads_back_the_same() {
        let t = table();
        assert_eq!(from_cache(&cache_json(&t)), Some(t));
        assert!(from_cache(&json!({ "lat": 1.0 })).is_none());
    }

    #[test]
    fn a_table_is_for_one_request_and_covers_its_days() {
        let t = table();
        let place = Place { name: "KL".into(), lat: 3.1412, lon: 101.69, tz: String::new(), cc: String::new() };
        let req = Request { place: place.clone(), method: 17, school: 0 };
        assert!(t.is_for(&req));
        assert!(!t.is_for(&Request { method: 3, ..req.clone() }));
        assert!(!t.is_for(&Request { school: 1, ..req.clone() }));
        assert!(!t.is_for(&Request { place: Place { lat: 1.0, ..place }, ..req }));
        assert!(t.covers(100, 2) && !t.covers(100, 3) && !t.covers(99, 1));
    }

    #[test]
    fn every_method_is_listed_once_and_the_countries_have_one() {
        for (i, (id, name)) in METHODS.iter().enumerate() {
            assert!(!name.is_empty());
            assert!(!METHODS[i + 1..].iter().any(|(other, _)| other == id), "method {id} twice");
        }
        for country in ["MY", "SG", "ID", "SA", "US", "my", "ZZ", ""] {
            let m = default_method(country);
            assert_eq!(known_method(m), m);
        }
        assert_eq!((default_method("MY"), default_method("ZZ")), (17, 3));
        assert_eq!(known_method(99), 3);
        assert_eq!(METHODS[method_index(11)].0, 11);
    }
}
