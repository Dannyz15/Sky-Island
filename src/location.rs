//! Finding out where the user is, for the "Detect my location" button. On Windows the first choice is Windows Location
//! (from Wi-Fi or GPS, accurate to a street or so), which needs Windows' Location setting switched on for desktop
//! apps; on Linux it asks GeoClue2 over the system D-Bus. The place name for those coordinates comes from OpenStreetMap's
//! Nominatim. If system location is off or unavailable, the connection's place is asked from ipwho.is instead, which is
//! only approximate.

use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
#[cfg(windows)]
use windows::Devices::Geolocation::Geolocator;
#[cfg(windows)]
use windows::Foundation::TimeSpan;
#[cfg(windows)]
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use winit::event_loop::EventLoopProxy;

use crate::UserEvent;
use crate::net::BodyExt;
use crate::weather::{self, Place, SearchError};

const REVERSE: &str = "https://nominatim.openstreetmap.org/reverse";
/// A position this old is as good as a new one.
#[cfg(windows)]
const MAX_AGE: Duration = Duration::from_secs(5 * 60);
/// How long system location may take to find the position.
#[cfg(any(windows, target_os = "linux"))]
const FIND_TIME: Duration = Duration::from_secs(12);

/// How well a place was found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum How {
    /// By system location (Windows Location or GeoClue2).
    Exact,
    /// By the internet connection only.
    Approximate,
}

#[cfg(any(windows, target_os = "linux", test))]
fn coarse_coords(lat: f64, lon: f64) -> Option<(f64, f64)> {
    let valid = lat.is_finite() && lon.is_finite() && (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon);
    // Only to about a kilometre: the city is all that is needed, not the house.
    valid.then(|| (weather::coarse(lat), weather::coarse(lon)))
}

#[cfg(windows)]
fn span(d: Duration) -> TimeSpan {
    // A TimeSpan counts in units of 100 nanoseconds.
    TimeSpan { Duration: (d.as_nanos() / 100) as i64 }
}

/// The position Windows Location gives, as (latitude, longitude). `None` when the Location setting is off for desktop
/// apps, or no position could be found in time.
#[cfg(windows)]
fn system_position() -> Option<(f64, f64)> {
    // SAFETY: plain COM initialisation for this thread; an error only means it was already done.
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
    let locator = Geolocator::new().ok()?;
    let position = locator.GetGeopositionAsyncWithAgeAndTimeout(span(MAX_AGE), span(FIND_TIME)).ok()?.join().ok()?;
    let p = position.Coordinate().ok()?.Point().ok()?.Position().ok()?;
    coarse_coords(p.Latitude, p.Longitude)
}

#[cfg(target_os = "linux")]
fn system_position() -> Option<(f64, f64)> {
    use std::collections::HashMap;
    use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

    const BUS: &str = "org.freedesktop.GeoClue2";
    const MANAGER_PATH: &str = "/org/freedesktop/GeoClue2/Manager";
    const MANAGER_IFACE: &str = "org.freedesktop.GeoClue2.Manager";
    const CLIENT_IFACE: &str = "org.freedesktop.GeoClue2.Client";
    const LOCATION_IFACE: &str = "org.freedesktop.GeoClue2.Location";
    const PROPS_IFACE: &str = "org.freedesktop.DBus.Properties";
    const CITY_ACCURACY: u32 = 4;

    let conn = zbus::blocking::Connection::system().ok()?;
    let dbus = zbus::blocking::fdo::DBusProxy::new(&conn).ok()?;
    let geoclue = zbus::names::BusName::try_from(BUS).ok()?;
    let available = dbus.name_has_owner(geoclue).unwrap_or(false) || dbus.list_activatable_names().ok().is_some_and(|names| names.iter().any(|name| name.as_str() == BUS));
    if !available {
        return None;
    }

    let client_path: OwnedObjectPath = conn.call_method(Some(BUS), MANAGER_PATH, Some(MANAGER_IFACE), "GetClient", &()).ok()?.body().deserialize().ok()?;
    let set_prop = |prop: &str, value: Value<'_>| conn.call_method(Some(BUS), client_path.as_str(), Some(PROPS_IFACE), "Set", &(CLIENT_IFACE, prop, value)).is_ok();
    if !set_prop("DesktopId", Value::from("sky-island")) {
        return None;
    }
    let _ = set_prop("RequestedAccuracyLevel", Value::from(CITY_ACCURACY));
    let _ = set_prop("TimeThreshold", Value::from(0u32));

    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(BUS)
        .ok()?
        .path(client_path.as_str())
        .ok()?
        .interface(CLIENT_IFACE)
        .ok()?
        .member("LocationUpdated")
        .ok()?
        .build();
    let iter = zbus::blocking::MessageIterator::for_match_rule(rule, &conn, Some(8)).ok()?;
    let (tx, rx) = std::sync::mpsc::channel::<OwnedObjectPath>();
    thread::spawn(move || {
        for item in iter {
            let Ok(msg) = item else { break };
            if let Ok((_old, new_loc)) = msg.body().deserialize::<(OwnedObjectPath, OwnedObjectPath)>() {
                let _ = tx.send(new_loc);
                break;
            }
        }
    });

    let coords = (|| {
        conn.call_method(Some(BUS), client_path.as_str(), Some(CLIENT_IFACE), "Start", &()).ok()?;

        let existing = conn
            .call_method(Some(BUS), client_path.as_str(), Some(PROPS_IFACE), "Get", &(CLIENT_IFACE, "Location"))
            .ok()
            .and_then(|msg| msg.body().deserialize::<OwnedValue>().ok())
            .and_then(|val| OwnedObjectPath::try_from(val).ok())
            .filter(|path| path.as_str() != "/");

        let loc_path = match existing {
            Some(path) => path,
            None => rx.recv_timeout(FIND_TIME).ok().filter(|path| path.as_str() != "/")?,
        };

        let props: HashMap<String, OwnedValue> = conn.call_method(Some(BUS), loc_path.as_str(), Some(PROPS_IFACE), "GetAll", &(LOCATION_IFACE,)).ok()?.body().deserialize().ok()?;
        let lat = props.get("Latitude").and_then(|v| f64::try_from(v).ok())?;
        let lon = props.get("Longitude").and_then(|v| f64::try_from(v).ok())?;
        coarse_coords(lat, lon)
    })();

    let _ = conn.call_method(Some(BUS), client_path.as_str(), Some(CLIENT_IFACE), "Stop", &());
    let _ = conn.close();
    coords
}

#[cfg(not(any(windows, target_os = "linux")))]
fn system_position() -> Option<(f64, f64)> {
    None
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
    let city =
        ["city", "town", "municipality", "village", "suburb", "city_district", "county", "state_district"].iter().map(|k| text(k)).find(|t| !t.is_empty()).unwrap_or_default();
    let name = if city.is_empty() { format!("{lat:.2}, {lon:.2}") } else { weather::label(&city, &text("state"), &text("country")) };
    let cc = text("country_code").to_ascii_uppercase();
    Place::from_json(&json!({ "name": name, "lat": lat, "lon": lon, "tz": "", "cc": cc })).unwrap_or(Place {
        name: format!("{lat:.2}, {lon:.2}"),
        lat,
        lon,
        tz: String::new(),
        cc: String::new(),
    })
}

/// Find where the user is, on a background thread; the answer arrives as `UserEvent::Located`.
pub fn spawn_detect(proxy: EventLoopProxy<UserEvent>) {
    thread::spawn(move || {
        let found = match system_position() {
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

    #[cfg(windows)]
    #[test]
    fn a_time_span_counts_in_hundreds_of_nanoseconds() {
        assert_eq!(span(Duration::from_secs(1)).Duration, 10_000_000);
        assert_eq!(span(Duration::from_secs(300)).Duration, 3_000_000_000);
    }

    #[test]
    fn coordinates_are_validated_and_rounded_to_city_precision() {
        assert_eq!(coarse_coords(3.1412, 101.6869), Some((weather::coarse(3.1412), weather::coarse(101.6869))));
        assert_eq!(coarse_coords(-90.0, 180.0), Some((weather::coarse(-90.0), weather::coarse(180.0))));
        assert_eq!(coarse_coords(f64::NAN, 10.0), None);
        assert_eq!(coarse_coords(10.0, f64::INFINITY), None);
        assert_eq!(coarse_coords(-90.1, 0.0), None);
        assert_eq!(coarse_coords(0.0, 180.1), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_system_position_falls_back_cleanly_when_geoclue_absent() {
        let started = std::time::Instant::now();
        if let Some((lat, lon)) = system_position() {
            assert!((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon));
        } else {
            assert!(started.elapsed() < Duration::from_secs(3), "missing GeoClue2 took too long: {:?}", started.elapsed());
        }
    }
}
