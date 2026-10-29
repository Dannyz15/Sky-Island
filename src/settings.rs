//! User settings, saved as JSON, plus the "start with Windows" registry entry.

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};
use winreg::RegKey;
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

#[derive(Clone)]
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

impl Settings {
    pub fn load() -> Self {
        let mut out = Self::default();
        let Some(v) = Self::path()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        else {
            return out;
        };
        let get = |k: &str, default: bool| v.get(k).and_then(Value::as_bool).unwrap_or(default);
        out.hide_fullscreen = get("hide_fullscreen", out.hide_fullscreen);
        out.show_clock = get("show_clock", out.show_clock);
        out.clock_24h = get("clock_24h", out.clock_24h);
        out.eyes = get("eyes", out.eyes);
        out.idle_eyes = get("idle_eyes", out.idle_eyes);
        out.minimal = get("minimal", out.minimal);
        out.open_on_click = get("open_on_click", out.open_on_click);
        out.week_monday = get("week_monday", out.week_monday);
        out.calendar_week = get("calendar_week", out.calendar_week);
        out.empty_shelf = get("empty_shelf", out.empty_shelf);
        out.hud_volume = get("hud_volume", out.hud_volume);
        out.hud_brightness = get("hud_brightness", out.hud_brightness);
        out.hud_lock = get("hud_lock", out.hud_lock);
        out.hud_battery = get("hud_battery", out.hud_battery);
        out.hud_network = get("hud_network", out.hud_network);
        out.hud_audio = get("hud_audio", out.hud_audio);
        if let Some(z) = v.get("size").and_then(Value::as_str).and_then(NotchSize::from_name) {
            out.size = z;
        }
        if let Some(e) = v.get("eye_style").and_then(Value::as_str).and_then(EyeStyle::from_name) {
            out.eye_style = e;
        }
        out.sound = get("sound", out.sound);
        out.clipboard = get("clipboard", out.clipboard);
        out.hide_island = get("hide_island", out.hide_island);
        out.water = get("water", out.water);
        out.timer = get("timer", out.timer);
        // The first light-theme build had an on/off switch for following Windows.
        if get("follow_theme", false) {
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
        let dir = PathBuf::from(std::env::var_os("APPDATA")?).join("Sky-Island");
        fs::create_dir_all(&dir).ok()?;
        Some(dir.join("settings.json"))
    }

    pub fn save(&self) {
        if cfg!(test) {
            return;
        }
        if let Some(p) = Self::path() {
            let body = json!({
                "hide_fullscreen": self.hide_fullscreen,
                "show_clock": self.show_clock,
                "clock_24h": self.clock_24h,
                "eyes": self.eyes,
                "idle_eyes": self.idle_eyes,
                "minimal": self.minimal,
                "open_on_click": self.open_on_click,
                "week_monday": self.week_monday,
                "calendar_week": self.calendar_week,
                "empty_shelf": self.empty_shelf,
                "hud_volume": self.hud_volume,
                "hud_brightness": self.hud_brightness,
                "hud_lock": self.hud_lock,
                "hud_battery": self.hud_battery,
                "hud_network": self.hud_network,
                "hud_audio": self.hud_audio,
                "size": self.size.name(),
                "eye_style": self.eye_style.name(),
                "sound": self.sound,
                "clipboard": self.clipboard,
                "hide_island": self.hide_island,
                "water": self.water,
                "timer": self.timer,
                "theme": self.theme.name(),
                "motion": self.motion.name(),
                "open_delay": self.open_delay,
                "close_delay": self.close_delay,
                "volume": self.volume,
                "alarm_every": self.alarm_every,
                "alarm_for": self.alarm_for,
                "alarm_sound": self.alarm_sound.as_ref().map(|p| p.to_string_lossy().into_owned()),
            });
            let _ = fs::write(p, body.to_string());
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
