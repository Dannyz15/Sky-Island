//! The City row of the Location page: a box the user types a city in, and the search it starts.

use super::*;
use crate::location::How;
use crate::weather::{self, Place, SearchError};

/// The longest a city can be typed.
const MAX_LEN: usize = 60;

/// How the last search for a city went.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) enum CityStatus {
    #[default]
    Idle,
    Searching,
    /// Finding out where the internet connection is.
    Detecting,
    NotFound,
    Offline,
    Undetected,
    /// Found by the internet connection only, because Windows Location is off.
    Approximate,
}

/// The state of the City box.
#[derive(Default)]
pub(super) struct CityField {
    /// What has been typed so far; `None` while the box is not being typed in.
    pub(super) typing: Option<String>,
    pub(super) status: CityStatus,
}

impl CityField {
    /// The line under the label of the row, and its colour.
    pub(super) fn note(&self) -> (String, [u8; 4]) {
        let grey = [146, 146, 160, 255];
        let red = [255, 150, 140, 255];
        match (&self.typing, self.status) {
            (Some(_), _) => ("Enter to search, Esc to cancel".to_string(), grey),
            (None, CityStatus::Searching) => ("Searching...".to_string(), grey),
            (None, CityStatus::Detecting) => ("Finding where you are...".to_string(), grey),
            (None, CityStatus::NotFound) => ("No such city. Try City, Country".to_string(), red),
            (None, CityStatus::Offline) => ("Could not reach the search. Are you online?".to_string(), red),
            (None, CityStatus::Undetected) => ("Could not tell where you are. Type your city".to_string(), red),
            (None, CityStatus::Approximate) => ("Approximate. Turn on Windows Location".to_string(), [240, 190, 110, 255]),
            (None, CityStatus::Idle) => ("Type a city and press Enter".to_string(), grey),
        }
    }
}

impl App {
    /// A key pressed while the city is being typed. True when the box took it (every key but F11).
    pub(super) fn city_key(&mut self, event: &winit::event::KeyEvent) -> bool {
        let proxy = self.proxy.clone();
        let Some(p) = &mut self.panel else { return false };
        let Some(text) = &mut p.city.typing else { return false };
        match &event.logical_key {
            Key::Named(NamedKey::F11) => return false,
            Key::Named(NamedKey::Enter) => {
                let query = text.trim().to_string();
                p.city.typing = None;
                if !query.is_empty() {
                    p.city.status = CityStatus::Searching;
                    weather::spawn_search(query, proxy);
                }
            }
            Key::Named(NamedKey::Escape) => p.city.typing = None,
            Key::Named(NamedKey::Backspace) => {
                text.pop();
            }
            _ => {
                if let Some(typed) = &event.text {
                    for ch in typed.chars().filter(|c| !c.is_control()) {
                        if text.chars().count() < MAX_LEN {
                            text.push(ch);
                        }
                    }
                }
            }
        }
        p.redraw();
        true
    }

    /// "Detect": ask where the internet connection is.
    pub(super) fn detect_city(&mut self) {
        if let Some(p) = &mut self.panel {
            p.city.typing = None;
            p.city.status = CityStatus::Detecting;
            p.redraw();
        }
        crate::location::spawn_detect(self.proxy.clone());
    }

    /// "Clear": forget the city and what was fetched for it. The prayer method goes back to the default too, until
    /// the next city says which country it is in.
    pub(super) fn clear_city(&mut self) {
        self.settings.place = None;
        self.settings.prayer_method = Settings::default().prayer_method;
        self.settings.prayer_zone.clear();
        weather::clear_cache();
        crate::prayer::clear_cache();
        if let Some(p) = &mut self.panel {
            p.city.typing = None;
            p.city.status = CityStatus::Idle;
        }
        self.settings_changed();
        self.sync_weather();
        self.sync_prayer();
    }

    /// The answer to a search for a city arrived: the city is kept and the weather follows it, or the box says why not.
    pub(crate) fn place_found(&mut self, found: Result<Place, SearchError>, how: How) {
        let status = match found {
            Ok(place) => {
                if !place.cc.is_empty() {
                    self.settings.prayer_method = crate::prayer::default_method(&place.cc);
                }
                // A zone chosen by hand was for the old city.
                self.settings.prayer_zone.clear();
                self.settings.place = Some(place);
                self.settings_changed();
                self.sync_weather();
                self.sync_prayer();
                if how == How::Approximate { CityStatus::Approximate } else { CityStatus::Idle }
            }
            Err(SearchError::NotFound) => CityStatus::NotFound,
            Err(SearchError::Offline) => CityStatus::Offline,
            Err(SearchError::Undetected) => CityStatus::Undetected,
        };
        if let Some(p) = &mut self.panel {
            p.city.status = status;
            p.redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_box_says_what_is_going_on() {
        let detecting = CityField { status: CityStatus::Detecting, ..CityField::default() };
        assert!(detecting.note().0.starts_with("Finding where"));
        let mut field = CityField::default();
        assert_eq!(field.note().0, "Type a city and press Enter");
        field.status = CityStatus::NotFound;
        assert!(field.note().0.starts_with("No such city"));
        field.typing = Some("Kuala".into());
        assert!(field.note().0.starts_with("Enter to search"), "typing wins over an old answer");
    }
}
