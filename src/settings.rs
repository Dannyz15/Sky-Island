//! User settings, saved as JSON, plus the "start with Windows" registry entry.

use std::path::PathBuf;

use serde_json::{Map, Value, json};
use winreg::RegKey;
use crate::store;
use winreg::enums::HKEY_CURRENT_USER;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_NAME: &str = "Sky Island";

/// Which look the notch has.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThemeMode {
    Dark,
    Light,
    /// Light while Windows uses its light theme, dark otherwise.
    System,
}

impl ThemeMode {
    pub const ALL: [ThemeMode; 3] = [ThemeMode::Dark, ThemeMode::Light, ThemeMode::System];

    fn name(self) -> &'static str {
        match self {
            ThemeMode::Dark => "dark",
            ThemeMode::Light => "light",
            ThemeMode::System => "system",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.name() == name)
    }
}

/// How lively the animations are.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Motion {
    Calm,
    Normal,
    Snappy,
    /// No animation: things land at once.
    Reduced,
}

impl Motion {
    pub const ALL: [Motion; 4] = [Motion::Calm, Motion::Normal, Motion::Snappy, Motion::Reduced];

    /// How much faster than normal the springs run.
    pub fn speed(self) -> f64 {
        match self {
            Motion::Calm => 0.6,
            Motion::Normal => 1.0,
            Motion::Snappy => 1.7,
            Motion::Reduced => 8.0,
        }
    }

    /// Whether springs may swing a little past their goal.
    pub fn bounce(self) -> bool {
        self != Motion::Reduced
    }

    fn name(self) -> &'static str {
        match self {
            Motion::Calm => "calm",
            Motion::Normal => "normal",
            Motion::Snappy => "snappy",
            Motion::Reduced => "reduced",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }
}

/// What the eyes on the closed notch look like.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EyeStyle {
    /// White ovals with a black pupil.
    Classic,
    /// Plain white dots.
    Dots,
    /// Amber eyes with a slit pupil.
    Cat,
    /// Glowing blue pills.
    Glow,
}

impl EyeStyle {
    pub const ALL: [EyeStyle; 4] = [EyeStyle::Classic, EyeStyle::Dots, EyeStyle::Cat, EyeStyle::Glow];

    fn name(self) -> &'static str {
        match self {
            EyeStyle::Classic => "classic",
            EyeStyle::Dots => "dots",
            EyeStyle::Cat => "cat",
            EyeStyle::Glow => "glow",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }
}

/// How big the whole island is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NotchSize {
    /// Follows the screen: the island keeps the same share of the screen's width.
    Auto,
    X100,
    X115,
    X130,
    X150,
}

impl NotchSize {
    pub const ALL: [NotchSize; 5] = [NotchSize::Auto, NotchSize::X100, NotchSize::X115, NotchSize::X130, NotchSize::X150];

    /// The scale, or `None` for Auto.
    pub fn factor(self) -> Option<f64> {
        match self {
            NotchSize::Auto => None,
            NotchSize::X100 => Some(1.0),
            NotchSize::X115 => Some(1.15),
            NotchSize::X130 => Some(1.3),
            NotchSize::X150 => Some(1.5),
        }
    }

    fn name(self) -> &'static str {
        match self {
            NotchSize::Auto => "auto",
            NotchSize::X100 => "100",
            NotchSize::X115 => "115",
            NotchSize::X130 => "130",
            NotchSize::X150 => "150",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }
}

/// A number setting: smallest and largest value, and the step the slider moves in.
#[derive(Clone, Copy)]
pub struct Range {
    pub min: u32,
    pub max: u32,
    pub step: u32,
}

impl Range {
    pub fn clamp(self, v: u32) -> u32 {
        v.clamp(self.min, self.max)
    }
}

/// How long the mouse rests on the notch before it opens, in milliseconds.
pub const OPEN_DELAY: Range = Range { min: 0, max: 500, step: 10 };
/// How long the mouse is away before it closes, in milliseconds.
pub const CLOSE_DELAY: Range = Range { min: 100, max: 1000, step: 50 };
/// Loudness of the chimes, in percent.
pub const VOLUME: Range = Range { min: 10, max: 100, step: 10 };
/// How often the timer's alarm sounds again, in seconds.
pub const ALARM_EVERY: Range = Range { min: 1, max: 10, step: 1 };
/// How long the timer's alarm rings if nobody stops it, in seconds.
pub const ALARM_FOR: Range = Range { min: 10, max: 120, step: 10 };

#[derive(Clone, PartialEq, Debug)]
pub struct Settings {
    pub hide_fullscreen: bool,
    pub show_clock: bool,
    pub clock_24h: bool,
    /// Show the eyes on the closed notch while you are working, when the clock is not using that place.
    pub eyes: bool,
    pub idle_eyes: bool,
    pub eye_style: EyeStyle,
    /// How big the whole island is.
    pub size: NotchSize,
    /// A smaller closed notch while nothing is playing.
    pub minimal: bool,
    /// The notch opens when it is clicked instead of when the mouse rests on it.
    pub open_on_click: bool,
    /// The calendar's weeks start on Monday (otherwise Sunday), and it shows one week instead of a month.
    pub week_monday: bool,
    pub calendar_week: bool,
    /// Forget the shelf's files when the app starts.
    pub empty_shelf: bool,
    /// Which notices (volume, brightness, Caps Lock and Num Lock, battery, network, audio devices) show.
    pub hud_volume: bool,
    pub hud_brightness: bool,
    pub hud_lock: bool,
    pub hud_battery: bool,
    pub hud_network: bool,
    pub hud_audio: bool,
    pub sound: bool,
    /// Keep a history of copied text.
    pub clipboard: bool,
    /// Slide the island out of view when idle; hovering the top edge brings it back.
    pub hide_island: bool,
    /// Show the Water tab and send water reminders. Off until the user turns it on.
    pub water: bool,
    /// Show the Timer tab. Off until the user turns it on.
    pub timer: bool,
    pub theme: ThemeMode,
    pub motion: Motion,
    pub open_delay: u32,
    pub close_delay: u32,
    pub volume: u32,
    pub alarm_every: u32,
    pub alarm_for: u32,
    /// A sound file to play instead of the built-in chime when the timer ends.
    pub alarm_sound: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { hide_fullscreen: true, show_clock: true, clock_24h: false, eyes: false, idle_eyes: true, eye_style: EyeStyle::Classic, size: NotchSize::Auto, minimal: false,
            open_on_click: false,
            week_monday: false,
            calendar_week: false,
            empty_shelf: false,
            hud_volume: true,
            hud_brightness: true,
            hud_lock: true,
            hud_battery: true,
            hud_network: true,
            hud_audio: true, sound: true, clipboard: true, hide_island: false, water: false,
            timer: false,
            theme: ThemeMode::Dark,
            motion: Motion::Normal,
            open_delay: 90,
            close_delay: 250,
            volume: 100,
            alarm_every: 3,
            alarm_for: 60,
            alarm_sound: None,
        }
    }
}

/// The on/off settings, listed once: each is kept in settings.json under its own name, so adding one here
/// is all it takes for it to be loaded and saved.
macro_rules! on_off_settings {
    ($($field:ident),* $(,)?) => {
        impl Settings {
            fn load_on_off(&mut self, v: &Value) {
                $( self.$field = v.get(stringify!($field)).and_then(Value::as_bool).unwrap_or(self.$field); )*
            }

            fn save_on_off(&self, body: &mut Map<String, Value>) {
                $( body.insert(stringify!($field).to_string(), Value::Bool(self.$field)); )*
            }
        }
    };
}

on_off_settings!(
    hide_fullscreen, show_clock, clock_24h, eyes, idle_eyes, minimal, open_on_click, week_monday, calendar_week,
    empty_shelf, hud_volume, hud_brightness, hud_lock, hud_battery, hud_network, hud_audio, sound, clipboard,
    hide_island, water, timer,
);

impl Settings {
    pub fn load() -> Self {
        match Self::path().and_then(|p| store::read_json(&p)) {
            Some(v) => Self::from_json(&v),
            None => Self::default(),
        }
    }

    /// The settings a settings.json says; whatever it leaves out, or gets wrong, stays at its default.
    fn from_json(v: &Value) -> Self {
        let mut out = Self::default();
        out.load_on_off(v);
        if let Some(z) = v.get("size").and_then(Value::as_str).and_then(NotchSize::from_name) {
            out.size = z;
        }
        if let Some(e) = v.get("eye_style").and_then(Value::as_str).and_then(EyeStyle::from_name) {
            out.eye_style = e;
        }
        // The first light-theme build had an on/off switch for following Windows.
        if v.get("follow_theme").and_then(Value::as_bool).unwrap_or(false) {
            out.theme = ThemeMode::System;
        }
        if let Some(t) = v.get("theme").and_then(Value::as_str).and_then(ThemeMode::from_name) {
            out.theme = t;
        }
        if let Some(m) = v.get("motion").and_then(Value::as_str).and_then(Motion::from_name) {
            out.motion = m;
        }
        let num = |k: &str, range: Range, default: u32| v.get(k).and_then(Value::as_u64).map_or(default, |n| range.clamp(n.min(u32::MAX as u64) as u32));
        out.open_delay = num("open_delay", OPEN_DELAY, out.open_delay);
        out.close_delay = num("close_delay", CLOSE_DELAY, out.close_delay);
        out.volume = num("volume", VOLUME, out.volume);
        out.alarm_every = num("alarm_every", ALARM_EVERY, out.alarm_every);
        out.alarm_for = num("alarm_for", ALARM_FOR, out.alarm_for);
        out.alarm_sound = v.get("alarm_sound").and_then(Value::as_str).filter(|p| !p.is_empty()).map(PathBuf::from);
        out
    }

    fn path() -> Option<PathBuf> {
        store::path("settings.json")
    }

    /// What goes into settings.json.
    fn to_json(&self) -> Value {
        let mut body = Map::new();
        self.save_on_off(&mut body);
        body.insert("size".into(), json!(self.size.name()));
        body.insert("eye_style".into(), json!(self.eye_style.name()));
        body.insert("theme".into(), json!(self.theme.name()));
        body.insert("motion".into(), json!(self.motion.name()));
        body.insert("open_delay".into(), json!(self.open_delay));
        body.insert("close_delay".into(), json!(self.close_delay));
        body.insert("volume".into(), json!(self.volume));
        body.insert("alarm_every".into(), json!(self.alarm_every));
        body.insert("alarm_for".into(), json!(self.alarm_for));
        body.insert("alarm_sound".into(), json!(self.alarm_sound.as_ref().map(|p| p.to_string_lossy().into_owned())));
        Value::Object(body)
    }

    pub fn save(&self) {
        if cfg!(test) {
            return;
        }
        if let Some(p) = Self::path() {
            store::write(&p, &self.to_json().to_string());
        }
    }
}

/// Whether the app is registered to start when the user signs in.
pub fn autostart_enabled() -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .and_then(|k| k.get_value::<String, _>(RUN_NAME))
        .is_ok()
}

/// Add or remove the per-user startup entry. Only touches this app's own value.
pub fn set_autostart(on: bool) {
    let Ok((key, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY) else { return };
    if on {
        if let Ok(exe) = std::env::current_exe() {
            let _ = key.set_value(RUN_NAME, &format!("\"{}\"", exe.display()));
        }
    } else {
        let _ = key.delete_value(RUN_NAME);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every setting, each different from its default. It is written out field by field on purpose: a setting
    /// added to the struct has to be added here too, and then this fails until it is also saved and loaded.
    fn every_setting_changed() -> Settings {
        Settings {
            hide_fullscreen: false,
            show_clock: false,
            clock_24h: true,
            eyes: true,
            idle_eyes: false,
            eye_style: EyeStyle::Cat,
            size: NotchSize::X130,
            minimal: true,
            open_on_click: true,
            week_monday: true,
            calendar_week: true,
            empty_shelf: true,
            hud_volume: false,
            hud_brightness: false,
            hud_lock: false,
            hud_battery: false,
            hud_network: false,
            hud_audio: false,
            sound: false,
            clipboard: false,
            hide_island: true,
            water: true,
            timer: true,
            theme: ThemeMode::System,
            motion: Motion::Snappy,
            open_delay: 150,
            close_delay: 400,
            volume: 40,
            alarm_every: 5,
            alarm_for: 30,
            alarm_sound: Some(PathBuf::from("C:/Sounds/ring.wav")),
        }
    }

    #[test]
    fn every_setting_survives_a_save_and_a_load() {
        let changed = every_setting_changed();
        assert_ne!(changed, Settings::default(), "the test must change something");
        assert_eq!(Settings::from_json(&changed.to_json()), changed);
        assert_eq!(Settings::from_json(&Settings::default().to_json()), Settings::default());
    }

    #[test]
    fn a_missing_or_wrong_setting_keeps_its_default() {
        let defaults = Settings::default();
        assert_eq!(Settings::from_json(&json!({})), defaults);
        let wrong = json!({ "show_clock": "yes", "theme": 5, "open_delay": "fast", "size": "huge" });
        assert_eq!(Settings::from_json(&wrong), defaults);
        // Numbers are held to their range.
        assert_eq!(Settings::from_json(&json!({ "volume": 5000 })).volume, VOLUME.max);
    }

    #[test]
    fn the_first_light_theme_switch_still_means_follow_windows() {
        assert_eq!(Settings::from_json(&json!({ "follow_theme": true })).theme, ThemeMode::System);
        assert_eq!(Settings::from_json(&json!({ "follow_theme": true, "theme": "light" })).theme, ThemeMode::Light);
    }
}
