//! Finding out where the user is, for the "Detect my location" button. The first choice is Windows Location (the
//! system's own position, from Wi-Fi or GPS, accurate to a street or so), which needs Windows' Location setting
//! switched on for desktop apps. The place name for those coordinates comes from OpenStreetMap's Nominatim. If
//! Windows Location is off, the connection's place is asked from ipwho.is instead, which is only approximate.

use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use windows::Devices::Geolocation::Geolocator;
use windows::Foundation::TimeSpan;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use winit::event_loop::EventLoopProxy;

use crate::UserEvent;
use crate::net::BodyExt;
use crate::weather::{self, Place, SearchError};

const REVERSE: &str = "https://nominatim.openstreetmap.org/reverse";
/// A position this old is as good as a new one.
const MAX_AGE: Duration = Duration::from_secs(5 * 60);
/// How long Windows may take to find the position.
const FIND_TIME: Duration = Duration::from_secs(12);

/// How well a place was found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum How {
    /// By Windows Location.
    Exact,
    /// By the internet connection only.
    Approximate,
}

fn span(d: Duration) -> TimeSpan {
    // A TimeSpan counts in units of 100 nanoseconds.
    TimeSpan { Duration: (d.as_nanos() / 100) as i64 }
}

/// The position Windows Location gives, as (latitude, longitude). `None` when the Location setting is off for desktop
/// apps, or no position could be found in time.
fn windows_position() -> Option<(f64, f64)> {
    // SAFETY: plain COM initialisation for this thread; an error only means it was already done.
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
    let locator = Geolocator::new().ok()?;
    let position = locator.GetGeopositionAsyncWithAgeAndTimeout(span(MAX_AGE), span(FIND_TIME)).ok()?.join().ok()?;
    let p = position.Coordinate().ok()?.Point().ok()?.Position().ok()?;
    let on_earth = (-90.0..=90.0).contains(&p.Latitude) && (-180.0..=180.0).contains(&p.Longitude);
    // Only to about a kilometre: the city is all that is needed, not the house.
    on_earth.then_some((weather::coarse(p.Latitude), weather::coarse(p.Longitude)))
}

/// The place Nominatim names for coordinates, to the city. `None` when it could not be asked.
fn reverse(lat: f64, lon: f64) -> Option<Value> {
    let agent: ureq::Agent = ureq::Agent::config_builder().https_only(true).timeout_global(Some(Duration::from_secs(10))).build().into();
    agent
        .get(REVERSE)
        .header("User-Agent", concat!("Sky-Island/", env!("CARGO_PKG_VERSION"), " (https://github.com/Dannyz15/Sky-Island)"))
        .query("lat", lat.to_string())
        .query("lon", lon.to_string())
        .query("format", "jsonv2")
        .query("zoom", "10")
        .query("accept-language", "en")
        .call()
        .ok()?
        .body_mut()
        .read_limited_json()
        .ok()
}

/// A place for coordinates, named by what Nominatim answered (or by the coordinates when it said nothing useful).
fn place_at(lat: f64, lon: f64, answer: Option<&Value>) -> Place {
    let address = answer.and_then(|a| a.get("address"));
    let text = |k: &str| address.and_then(|a| a.get(k)).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let city = ["city", "town", "municipality", "village", "suburb", "city_district", "county", "state_district"]
        .iter()
        .map(|k| text(k))
        .find(|t| !t.is_empty())
        .unwrap_or_default();
    let name = if city.is_empty() { format!("{lat:.2}, {lon:.2}") } else { weather::label(&city, &text("state"), &text("country")) };
    let cc = text("country_code").to_ascii_uppercase();
    Place::from_json(&json!({ "name": name, "lat": lat, "lon": lon, "tz": "", "cc": cc })).unwrap_or(Place { name: format!("{lat:.2}, {lon:.2}"), lat, lon, tz: String::new(), cc: String::new() })
}

/// Find where the user is, on a background thread; the answer arrives as `UserEvent::Located`.
pub fn spawn_detect(proxy: EventLoopProxy<UserEvent>) {
    thread::spawn(move || {
        let found = match windows_position() {
            Some((lat, lon)) => Ok((place_at(lat, lon, reverse(lat, lon).as_ref()), How::Exact)),
            None => weather::detect_by_ip().map(|p| (p, How::Approximate)),
        };
        let _ = proxy.send_event(UserEvent::Located(found));
    });
}

/// What a detection found, or why it found nothing (the errors a search has too).
pub type Found = Result<(Place, How), SearchError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_place_is_named_to_its_city() {
        let answer = json!({ "address": { "city": "Kuala Lumpur", "state": "Kuala Lumpur", "country": "Malaysia", "country_code": "my", "suburb": "Sentul" } });
        let p = place_at(3.18, 101.69, Some(&answer));
        assert_eq!((p.name.as_str(), p.cc.as_str()), ("Kuala Lumpur, Malaysia", "MY"));
        assert!(p.same_spot(3.18, 101.69));
    }

    #[test]
    fn a_town_or_a_suburb_will_do_when_there_is_no_city() {
        let answer = json!({ "address": { "town": "Kajang", "state": "Selangor", "country": "Malaysia", "country_code": "my" } });
        assert_eq!(place_at(2.99, 101.79, Some(&answer)).name, "Kajang, Selangor, Malaysia");
        let answer = json!({ "address": { "suburb": "Sentul", "country": "Malaysia", "country_code": "my" } });
        assert_eq!(place_at(3.18, 101.69, Some(&answer)).name, "Sentul, Malaysia");
    }

    #[test]
    fn without_an_answer_the_place_is_named_by_its_coordinates() {
        let p = place_at(3.1412, 101.6865, None);
        assert_eq!((p.name.as_str(), p.cc.as_str()), ("3.14, 101.69", ""));
        assert_eq!(place_at(1.0, 2.0, Some(&json!({ "error": "Unable to geocode" }))).name, "1.00, 2.00");
    }

    #[test]
    fn a_time_span_counts_in_hundreds_of_nanoseconds() {
        assert_eq!(span(Duration::from_secs(1)).Duration, 10_000_000);
        assert_eq!(span(Duration::from_secs(300)).Duration, 3_000_000_000);
    }
}
