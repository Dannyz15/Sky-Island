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
    /// No bounce, and quick.
    Reduced,
}

impl Motion {
    pub const ALL: [Motion; 4] = [Motion::Calm, Motion::Normal, Motion::Snappy, Motion::Reduced];

    /// How much faster than normal the springs run.
    pub fn speed(self) -> f64 {
        match self {
            Motion::Calm => 0.75,
            Motion::Normal => 1.0,
            Motion::Snappy => 1.35,
            Motion::Reduced => 1.6,
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

#[derive(Clone, Copy)]
pub struct Settings {
    pub hide_fullscreen: bool,
    pub show_clock: bool,
    pub clock_24h: bool,
    pub idle_eyes: bool,
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
}

impl Default for Settings {
    fn default() -> Self {
        Self { hide_fullscreen: true, show_clock: true, clock_24h: false, idle_eyes: true, sound: true, clipboard: true, hide_island: false, water: false,
            timer: false,
            theme: ThemeMode::Dark,
            motion: Motion::Normal,
            open_delay: 90,
            close_delay: 250,
            volume: 100,
            alarm_every: 3,
            alarm_for: 60,
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
        out.idle_eyes = get("idle_eyes", out.idle_eyes);
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
                "idle_eyes": self.idle_eyes,
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
