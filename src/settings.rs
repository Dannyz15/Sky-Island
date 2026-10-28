//! User settings, saved as JSON, plus the "start with Windows" registry entry.

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};
use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_NAME: &str = "Sky Island";

#[derive(Clone, Copy)]
pub struct Settings {
    pub hide_fullscreen: bool,
    pub show_clock: bool,
    pub clock_24h: bool,
    pub idle_eyes: bool,
    pub sound: bool,
    /// Keep a history of copied text.
    pub clipboard: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { hide_fullscreen: true, show_clock: true, clock_24h: false, idle_eyes: true, sound: true, clipboard: true }
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
