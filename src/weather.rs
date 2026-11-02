//! The weather and the place it is for. Both come from Open-Meteo (open-meteo.com: no account, no key). A
//! city the user types is looked up once and its coordinates are kept; the forecast for those coordinates is
//! fetched on a background thread, kept in `weather.json` so the tab has something to show at once and when
//! offline, and shown in the Weather tab.

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use winit::event_loop::EventLoopProxy;

use crate::{UserEvent, store};

const FORECAST: &str = "https://api.open-meteo.com/v1/forecast";
const GEOCODING: &str = "https://geocoding-api.open-meteo.com/v1/search";
/// Where the user's internet connection seems to be. Asked only when the user presses Detect.
const WHERE_AM_I: &str = "https://ipwho.is/";
/// How often the forecast is fetched again while the Weather tab is on.
const EVERY: Duration = Duration::from_secs(30 * 60);
/// A forecast older than this is fetched again when the tab is opened.
pub const STALE: Duration = Duration::from_secs(10 * 60);
/// How many days the forecast has, today included.
const DAYS: usize = 6;

/// A city: what to call it, where it is and which time zone it is in.
#[derive(Clone, PartialEq, Debug)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    /// "Asia/Kuala_Lumpur".
    pub tz: String,
    /// The country's two-letter code, "MY" (empty when it is not known).
    pub cc: String,
}

impl Place {
    pub fn to_json(&self) -> Value {
        json!({ "name": self.name, "lat": self.lat, "lon": self.lon, "tz": self.tz, "cc": self.cc })
    }

    pub fn from_json(v: &Value) -> Option<Place> {
        let name = v.get("name")?.as_str()?.trim().to_string();
        let (lat, lon) = (v.get("lat")?.as_f64()?, v.get("lon")?.as_f64()?);
        let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let (tz, cc) = (text("tz"), text("cc"));
        let on_earth = (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon);
        (!name.is_empty() && on_earth).then_some(Place { name, lat, lon, tz, cc })
    }

    /// Whether `other` is the same spot.
    pub fn same_spot(&self, lat: f64, lon: f64) -> bool {
        (self.lat - lat).abs() < 1e-6 && (self.lon - lon).abs() < 1e-6
    }
}

/// One day of the forecast. Temperatures are in degrees Celsius.
#[derive(Clone, PartialEq, Debug)]
pub struct Day {
    /// "2026-11-03", the day in the place's own time zone.
    pub date: String,
    pub code: u32,
    pub high: f32,
    pub low: f32,
}

/// What the weather is now and for the days ahead. Temperatures are in degrees Celsius, wind in km/h.
#[derive(Clone, PartialEq, Debug)]
pub struct Report {
    pub lat: f64,
    pub lon: f64,
    /// "2026-11-02T14:15": the local time of the reading.
    pub time: String,
    pub temp: f32,
    pub feels: f32,
    pub humidity: u32,
    pub wind: f32,
    pub code: u32,
    pub daytime: bool,
    /// Today first.
    pub days: Vec<Day>,
}

impl Report {
    /// "14:15", the local time of the reading.
    pub fn clock(&self) -> &str {
        self.time.split('T').nth(1).unwrap_or("")
    }
}

/// The kinds of sky the icons tell apart.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sky {
    Clear,
    PartlyCloudy,
    Cloudy,
    Fog,
    Drizzle,
    Rain,
    Snow,
    Thunder,
}

/// The sky a WMO weather code stands for.
pub fn sky(code: u32) -> Sky {
    match code {
        0 | 1 => Sky::Clear,
        2 => Sky::PartlyCloudy,
        3 => Sky::Cloudy,
        45 | 48 => Sky::Fog,
        51..=57 => Sky::Drizzle,
        61..=67 | 80..=82 => Sky::Rain,
        71..=77 | 85 | 86 => Sky::Snow,
        95..=99 => Sky::Thunder,
        _ => Sky::Cloudy,
    }
}

/// A WMO weather code in words.
pub fn describe(code: u32) -> &'static str {
    match code {
        0 => "Clear sky",
        1 => "Mostly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 | 48 => "Fog",
        51 | 53 | 55 => "Drizzle",
        56 | 57 => "Freezing drizzle",
        61 => "Light rain",
        63 => "Rain",
        65 => "Heavy rain",
        66 | 67 => "Freezing rain",
        71 => "Light snow",
        73 => "Snow",
        75 => "Heavy snow",
        77 => "Snow grains",
        80 => "Light showers",
        81 => "Showers",
        82 => "Heavy showers",
        85 | 86 => "Snow showers",
        95 => "Thunderstorm",
        96 | 99 => "Thunderstorm with hail",
        _ => "Cloudy",
    }
}

/// A temperature in the unit the user chose, as shown: "31" (the degree sign is drawn by the caller).
pub fn degrees(celsius: f32, fahrenheit: bool) -> i32 {
    let t = if fahrenheit { celsius * 9.0 / 5.0 + 32.0 } else { celsius };
    t.round() as i32
}

/// A wind speed in the unit that goes with the temperature: "12 km/h" or "7 mph".
pub fn wind_text(kmh: f32, fahrenheit: bool) -> String {
    if fahrenheit { format!("{} mph", (kmh * 0.621_371).round() as i32) } else { format!("{} km/h", kmh.round() as i32) }
}

/// The forecast Open-Meteo answered with. `None` when anything the tab needs is missing.
fn parse(v: &Value, lat: f64, lon: f64) -> Option<Report> {
    let cur = v.get("current")?;
    let num = |o: &Value, k: &str| o.get(k).and_then(Value::as_f64);
    let daily = v.get("daily")?;
    let list = |k: &str| daily.get(k).and_then(Value::as_array);
    let (dates, codes, highs, lows) = (list("time")?, list("weather_code")?, list("temperature_2m_max")?, list("temperature_2m_min")?);
    let days: Vec<Day> = (0..dates.len().min(codes.len()).min(highs.len()).min(lows.len()).min(DAYS))
        .filter_map(|i| {
            Some(Day {
                date: dates[i].as_str()?.to_string(),
                code: codes[i].as_u64()? as u32,
                high: highs[i].as_f64()? as f32,
                low: lows[i].as_f64()? as f32,
            })
        })
        .collect();
    if days.is_empty() {
        return None;
    }
    Some(Report {
        lat,
        lon,
        time: cur.get("time")?.as_str()?.to_string(),
        temp: num(cur, "temperature_2m")? as f32,
        feels: num(cur, "apparent_temperature").or_else(|| num(cur, "temperature_2m"))? as f32,
        humidity: num(cur, "relative_humidity_2m").unwrap_or(0.0).clamp(0.0, 100.0) as u32,
        wind: num(cur, "wind_speed_10m").unwrap_or(0.0).max(0.0) as f32,
        code: cur.get("weather_code")?.as_u64()? as u32,
        daytime: cur.get("is_day").and_then(Value::as_u64).unwrap_or(1) != 0,
        days,
    })
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(10))).build().into()
}

fn user_agent() -> &'static str {
    concat!("Sky-Island/", env!("CARGO_PKG_VERSION"))
}

fn fetch(place: &Place) -> Option<Report> {
    let json = agent()
        .get(FORECAST)
        .header("User-Agent", user_agent())
        .query("latitude", place.lat.to_string())
        .query("longitude", place.lon.to_string())
        .query("current", "temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,wind_speed_10m,is_day")
        .query("daily", "weather_code,temperature_2m_max,temperature_2m_min")
        .query("timezone", "auto")
        .query("forecast_days", DAYS.to_string())
        .call()
        .ok()?
        .body_mut()
        .read_json::<Value>()
        .ok()?;
    parse(&json, place.lat, place.lon)
}

/// Start the thread that fetches the forecast. It fetches when it is told which place to watch (`Some`), and
/// again every half hour; `None` stops it until the next place arrives. Each answer comes back as
/// `UserEvent::Weather`: `None` when it did not work.
pub fn spawn(proxy: EventLoopProxy<UserEvent>) -> Sender<Option<Place>> {
    let (tx, rx) = mpsc::channel::<Option<Place>>();
    thread::spawn(move || {
        let mut place: Option<Place> = None;
        loop {
            match rx.recv_timeout(EVERY) {
                Ok(next) => place = next,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            let Some(p) = &place else { continue };
            let report = fetch(p);
            if let Some(r) = &report {
                save_cache(r);
            }
            if proxy.send_event(UserEvent::Weather(report)).is_err() {
                return;
            }
        }
    });
    tx
}

fn from_cache(v: &Value) -> Option<Report> {
    parse(v.get("answer")?, v.get("lat")?.as_f64()?, v.get("lon")?.as_f64()?)
}

/// The forecast kept from the last time, if it is for this place.
pub fn load_cache(place: &Place) -> Option<Report> {
    let v = store::read_json(&store::path("weather.json")?)?;
    let r = from_cache(&v)?;
    place.same_spot(r.lat, r.lon).then_some(r)
}

/// What goes into weather.json: the answer in the shape Open-Meteo gave it, so reading it back is the same code.
fn cache_json(r: &Report) -> Value {
    let days = &r.days;
    json!({
        "lat": r.lat,
        "lon": r.lon,
        "answer": {
            "current": {
                "time": r.time, "temperature_2m": r.temp, "apparent_temperature": r.feels, "relative_humidity_2m": r.humidity,
                "weather_code": r.code, "wind_speed_10m": r.wind, "is_day": u8::from(r.daytime),
            },
            "daily": {
                "time": days.iter().map(|d| d.date.clone()).collect::<Vec<_>>(),
                "weather_code": days.iter().map(|d| d.code).collect::<Vec<_>>(),
                "temperature_2m_max": days.iter().map(|d| d.high).collect::<Vec<_>>(),
                "temperature_2m_min": days.iter().map(|d| d.low).collect::<Vec<_>>(),
            },
        },
    })
}

fn save_cache(r: &Report) {
    if cfg!(test) {
        return;
    }
    if let Some(path) = store::path("weather.json") {
        store::write(&path, &cache_json(r).to_string());
    }
}

/// Why a search for a city found nothing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SearchError {
    /// The search worked and there is no such city.
    NotFound,
    /// The search could not be reached.
    Offline,
    /// The internet connection could not be placed.
    Undetected,
}

/// Look a city up on a background thread; the answer arrives as `UserEvent::Place`.
pub fn spawn_search(query: String, proxy: EventLoopProxy<UserEvent>) {
    thread::spawn(move || {
        let _ = proxy.send_event(UserEvent::Place(search(&query)));
    });
}

fn search(query: &str) -> Result<Place, SearchError> {
    let (name, _) = split_query(query);
    if name.is_empty() {
        return Err(SearchError::NotFound);
    }
    let json = agent()
        .get(GEOCODING)
        .header("User-Agent", user_agent())
        .query("name", name)
        .query("count", "10")
        .query("language", "en")
        .query("format", "json")
        .call()
        .ok()
        .and_then(|mut r| r.body_mut().read_json::<Value>().ok())
        .ok_or(SearchError::Offline)?;
    pick_place(&json, query).ok_or(SearchError::NotFound)
}

/// What was typed, as the name to search for and what the user added after a comma to say which one they mean:
/// "Springfield, Illinois" or "Johor Bahru, MY".
fn split_query(query: &str) -> (&str, String) {
    let mut parts = query.splitn(2, ',');
    let name = parts.next().unwrap_or("").trim();
    (name, parts.next().unwrap_or("").trim().to_lowercase())
}

/// The city the user meant among the search results (Open-Meteo lists the biggest first): the first one, or the
/// first that matches the country or region typed after a comma.
fn pick_place(json: &Value, query: &str) -> Option<Place> {
    let (_, hint) = split_query(query);
    let text = |r: &Value, k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let matches = |r: &Value| {
        hint.is_empty()
            || ["country", "country_code", "admin1", "admin2"].iter().any(|k| {
                let v = text(r, k).to_lowercase();
                !v.is_empty() && (v == hint || (hint.len() >= 3 && v.contains(&hint)))
            })
    };
    let result = json.get("results")?.as_array()?.iter().find(|r| matches(r))?;
    let label = label(&text(result, "name"), &text(result, "admin1"), &text(result, "country"));
    Place::from_json(&json!({ "name": label, "lat": result.get("latitude")?, "lon": result.get("longitude")?, "tz": text(result, "timezone"), "cc": text(result, "country_code") }))
}

/// "Springfield, Illinois, United States": the city, then its region and country, leaving out what repeats.
pub(crate) fn label(city: &str, region: &str, country: &str) -> String {
    let mut label = city.to_string();
    for part in [region, country] {
        if !part.is_empty() && !label.split(", ").any(|l| l == part) {
            label.push_str(", ");
            label.push_str(part);
        }
    }
    label
}

/// Where the user's internet connection seems to be. It is approximate (the city of the provider's gateway) and
/// sends the user's IP address to ipwho.is; it is the way to detect the place when Windows Location is off.
pub fn detect_by_ip() -> Result<Place, SearchError> {
    let json = agent()
        .get(WHERE_AM_I)
        .header("User-Agent", user_agent())
        .query("fields", "success,city,region,country,country_code,latitude,longitude,timezone.id")
        .call()
        .ok()
        .and_then(|mut r| r.body_mut().read_json::<Value>().ok())
        .ok_or(SearchError::Offline)?;
    place_from_ip(&json).ok_or(SearchError::Undetected)
}

/// The place ipwho.is answered with, if it knew one.
fn place_from_ip(v: &Value) -> Option<Place> {
    if v.get("success").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let city = if text("city").is_empty() { text("region") } else { text("city") };
    if city.is_empty() {
        return None;
    }
    let tz = v.get("timezone").and_then(|t| t.get("id")).and_then(Value::as_str).unwrap_or("");
    Place::from_json(&json!({
        "name": label(&city, &text("region"), &text("country")),
        "lat": v.get("latitude")?,
        "lon": v.get("longitude")?,
        "tz": tz,
        "cc": text("country_code"),
    }))
}

/// Delete the forecast that was kept for the city.
pub fn clear_cache() {
    if let Some(path) = store::path("weather.json") {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer() -> Value {
        json!({
            "current": { "time": "2026-11-02T16:45", "temperature_2m": 28.0, "apparent_temperature": 33.7, "relative_humidity_2m": 82,
                "weather_code": 80, "wind_speed_10m": 5.0, "is_day": 1 },
            "daily": { "time": ["2026-11-02", "2026-11-03"], "weather_code": [81, 55], "temperature_2m_max": [31.4, 31.8], "temperature_2m_min": [25.0, 23.8] },
        })
    }

    #[test]
    fn the_forecast_is_read() {
        let r = parse(&answer(), 3.1412, 101.69).expect("a report");
        assert_eq!((r.temp, r.feels, r.humidity, r.code, r.daytime), (28.0, 33.7, 82, 80, true));
        assert_eq!(r.clock(), "16:45");
        assert_eq!(r.days.len(), 2);
        assert_eq!(r.days[1], Day { date: "2026-11-03".into(), code: 55, high: 31.8, low: 23.8 });
    }

    #[test]
    fn an_answer_with_something_missing_is_not_a_report() {
        let mut v = answer();
        v["current"].as_object_mut().unwrap().remove("temperature_2m");
        assert!(parse(&v, 0.0, 0.0).is_none());
        let mut v = answer();
        v["daily"]["time"] = json!([]);
        assert!(parse(&v, 0.0, 0.0).is_none(), "no days");
        assert!(parse(&json!({}), 0.0, 0.0).is_none());
    }

    #[test]
    fn a_saved_forecast_reads_back_the_same() {
        let r = parse(&answer(), 3.1412, 101.69).unwrap();
        assert_eq!(from_cache(&cache_json(&r)), Some(r));
        assert!(from_cache(&json!({ "lat": 1.0 })).is_none());
    }

    #[test]
    fn units_follow_the_choice() {
        assert_eq!(degrees(31.4, false), 31);
        assert_eq!(degrees(0.0, true), 32);
        assert_eq!(degrees(-0.4, false), 0);
        assert_eq!(wind_text(12.0, false), "12 km/h");
        assert_eq!(wind_text(16.0, true), "10 mph");
    }

    #[test]
    fn weather_codes_have_a_sky_and_words() {
        for code in [0, 1, 2, 3, 45, 48, 51, 55, 56, 61, 65, 66, 71, 75, 77, 80, 82, 85, 95, 96, 99, 1234] {
            assert!(!describe(code).is_empty());
        }
        assert_eq!(sky(0), Sky::Clear);
        assert_eq!(sky(63), Sky::Rain);
        assert_eq!(sky(81), Sky::Rain);
        assert_eq!(sky(73), Sky::Snow);
        assert_eq!(sky(95), Sky::Thunder);
        assert_eq!(sky(45), Sky::Fog);
    }

    fn results() -> Value {
        json!({ "results": [
            { "name": "Springfield", "latitude": 39.8, "longitude": -89.6, "country": "United States", "country_code": "US", "admin1": "Illinois", "timezone": "America/Chicago" },
            { "name": "Springfield", "latitude": 42.1, "longitude": -72.6, "country": "United States", "country_code": "US", "admin1": "Massachusetts", "timezone": "America/New_York" },
            { "name": "Springfield", "latitude": 51.7, "longitude": 0.5, "country": "United Kingdom", "country_code": "GB", "admin1": "England", "timezone": "Europe/London" },
        ] })
    }

    #[test]
    fn the_first_result_is_the_city() {
        let p = pick_place(&results(), "Springfield").unwrap();
        assert_eq!(p.name, "Springfield, Illinois, United States");
        assert_eq!((p.tz.as_str(), p.cc.as_str()), ("America/Chicago", "US"));
        assert!(p.same_spot(39.8, -89.6));
    }

    #[test]
    fn a_word_after_a_comma_picks_the_region_or_country() {
        assert_eq!(pick_place(&results(), "Springfield, Massachusetts").unwrap().tz, "America/New_York");
        assert_eq!(pick_place(&results(), "springfield, gb").unwrap().tz, "Europe/London");
        assert_eq!(pick_place(&results(), "Springfield, United Kingdom").unwrap().tz, "Europe/London");
        assert!(pick_place(&results(), "Springfield, Narnia").is_none());
        assert!(pick_place(&json!({}), "x").is_none(), "no results");
    }

    #[test]
    fn the_place_an_internet_connection_seems_to_be_in_is_read() {
        let kl = json!({ "success": true, "country": "Malaysia", "country_code": "MY", "region": "Kuala Lumpur", "city": "Kuala Lumpur",
            "latitude": 3.1411762, "longitude": 101.6865331, "timezone": { "id": "Asia/Kuala_Lumpur" } });
        let p = place_from_ip(&kl).unwrap();
        assert_eq!((p.name.as_str(), p.cc.as_str(), p.tz.as_str()), ("Kuala Lumpur, Malaysia", "MY", "Asia/Kuala_Lumpur"));
        assert!(p.same_spot(3.1411762, 101.6865331));
        // No city: the region will do. Nothing at all, or a refusal: no place.
        let region = json!({ "success": true, "country": "Malaysia", "country_code": "MY", "region": "Selangor", "city": "", "latitude": 3.0, "longitude": 101.5 });
        assert_eq!(place_from_ip(&region).unwrap().name, "Selangor, Malaysia");
        assert!(place_from_ip(&json!({ "success": false, "message": "Invalid IP address" })).is_none());
        assert!(place_from_ip(&json!({ "success": true, "city": "X" })).is_none(), "no coordinates");
        assert!(place_from_ip(&json!({})).is_none());
    }

    #[test]
    fn a_name_that_repeats_its_region_is_not_said_twice() {
        let v = json!({ "results": [{ "name": "Kuala Lumpur", "latitude": 3.1412, "longitude": 101.69, "country": "Malaysia", "admin1": "Kuala Lumpur", "timezone": "Asia/Kuala_Lumpur" }] });
        assert_eq!(pick_place(&v, "Kuala Lumpur").unwrap().name, "Kuala Lumpur, Malaysia");
    }

    #[test]
    fn a_place_is_kept_only_if_it_is_on_earth() {
        let p = Place { name: "Here".into(), lat: 3.0, lon: 101.0, tz: "Asia/Kuala_Lumpur".into(), cc: "MY".into() };
        assert_eq!(Place::from_json(&p.to_json()), Some(p));
        assert!(Place::from_json(&json!({ "name": "X", "lat": 91.0, "lon": 0.0 })).is_none());
        assert!(Place::from_json(&json!({ "name": " ", "lat": 1.0, "lon": 0.0 })).is_none());
        assert!(Place::from_json(&json!({ "name": "X", "lat": "1", "lon": 0.0 })).is_none());
    }
}
