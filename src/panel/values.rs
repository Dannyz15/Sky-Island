//! The values behind the controls: reading and changing a setting, the names of the choices, the cells of a choice control and the range of a slider.

use super::*;

pub(super) const THEME_NAMES: [&str; 3] = ["Dark", "Light", "Follow System"];

pub(super) const LOOK_NAMES: [&str; 2] = ["Classic", "Floating"];

pub(super) const GLASS_NAMES: [&str; 4] = ["Off", "Clear", "Smoky", "Black"];

pub(super) const MOTION_NAMES: [&str; 4] = ["Calm", "Normal", "Snappy", "None"];

pub(super) const SIZE_NAMES: [&str; 5] = ["Auto", "100%", "115%", "130%", "150%"];

pub(super) const OPEN_NAMES: [&str; 2] = ["Hover", "Click"];

pub(super) const LIQUID_ON_NAMES: [&str; 2] = ["All looks", "Glass only"];

pub(super) const WEEK_NAMES: [&str; 2] = ["Sunday", "Monday"];

pub(super) const VIEW_NAMES: [&str; 2] = ["Month", "Week"];

pub(super) const KEEP_NAMES: [&str; 4] = ["Never", "5 min", "30 min", "1 hour"];

pub(super) const ASR_NAMES: [&str; 2] = ["Standard", "Hanafi"];

pub(super) const UNITS_NAMES: [&str; 2] = ["\u{00B0}C", "\u{00B0}F"];

/// How wide a choice is compared with the others in the same row ("Follow System" is the longest).
pub(super) fn seg_weights(seg: Seg) -> &'static [f32] {
    match seg {
        Seg::Theme => &[1.0, 1.0, 2.0],
        Seg::Look => &[1.0, 1.0],
        Seg::Glass => &[1.0, 1.0, 1.0, 1.0],
        Seg::Motion => &[1.0, 1.0, 1.0, 1.0],
        Seg::Size => &[1.0, 1.0, 1.0, 1.0, 1.0],
        Seg::Open | Seg::WeekStart | Seg::CalView | Seg::Units | Seg::Asr | Seg::LiquidOn => &[1.0, 1.0],
        Seg::ClipKeep => &[1.0, 1.0, 1.0, 1.0],
    }
}

pub(super) fn seg_width(seg: Seg) -> f32 {
    match seg {
        Seg::Theme => 260.0,
        Seg::Look => 200.0,
        Seg::Glass => 260.0,
        Seg::Motion => 240.0,
        Seg::Size => 300.0,
        Seg::Open | Seg::CalView => 170.0,
        Seg::LiquidOn => 220.0,
        Seg::WeekStart => 190.0,
        Seg::ClipKeep => 270.0,
        Seg::Units => 130.0,
        Seg::Asr => 190.0,
    }
}

/// The area of each choice inside a segmented control.
pub(super) fn seg_cells(seg: Seg, ctrl: Rect) -> Vec<Rect> {
    let weights = seg_weights(seg);
    let total: f32 = weights.iter().sum();
    let mut x = ctrl.0;
    weights
        .iter()
        .map(|w| {
            let cell = (x, ctrl.1, ctrl.2 * w / total, ctrl.3);
            x += cell.2;
            cell
        })
        .collect()
}

pub(super) fn seg_names(seg: Seg) -> &'static [&'static str] {
    match seg {
        Seg::Theme => &THEME_NAMES,
        Seg::Look => &LOOK_NAMES,
        Seg::Glass => &GLASS_NAMES,
        Seg::Motion => &MOTION_NAMES,
        Seg::Size => &SIZE_NAMES,
        Seg::Open => &OPEN_NAMES,
        Seg::LiquidOn => &LIQUID_ON_NAMES,
        Seg::WeekStart => &WEEK_NAMES,
        Seg::CalView => &VIEW_NAMES,
        Seg::ClipKeep => &KEEP_NAMES,
        Seg::Units => &UNITS_NAMES,
        Seg::Asr => &ASR_NAMES,
    }
}

pub(super) fn range_of(slide: Slide) -> Range {
    match slide {
        Slide::OpenDelay => settings::OPEN_DELAY,
        Slide::CloseDelay => settings::CLOSE_DELAY,
        Slide::Volume | Slide::Own(_) => settings::VOLUME,
        Slide::AlarmEvery => settings::ALARM_EVERY,
        Slide::AlarmFor => settings::ALARM_FOR,
        Slide::GlassClear => settings::GLASS_CLEAR,
        Slide::GlassFrost => settings::GLASS_FROST,
    }
}

pub(super) fn slider_text(slide: Slide, v: u32) -> String {
    match slide {
        Slide::OpenDelay if v == 0 => "Instant".to_string(),
        Slide::OpenDelay | Slide::CloseDelay => format!("{v} ms"),
        Slide::Volume | Slide::Own(_) | Slide::GlassClear | Slide::GlassFrost => format!("{v}%"),
        Slide::AlarmEvery | Slide::AlarmFor => format!("{v} s"),
    }
}

/// The value a slider has when the mouse is at `x`, on a track starting at `track.0`.
pub(super) fn slider_value(range: Range, track: Rect, x: f32) -> u32 {
    let t = ((x - track.0) / track.2).clamp(0.0, 1.0);
    let steps = ((range.max - range.min) / range.step) as f32;
    range.min + (t * steps).round() as u32 * range.step
}

impl App {
    /// Whether an on/off setting is on.
    pub(super) fn toggle_value(&self, t: Toggle) -> bool {
        match t {
            Toggle::Autostart => self.autostart,
            Toggle::HideFullscreen => self.settings.hide_fullscreen,
            Toggle::ShowClock => self.settings.show_clock,
            Toggle::Clock24 => self.settings.clock_24h,
            Toggle::Eyes => self.settings.eyes,
            Toggle::IdleEyes => self.settings.idle_eyes,
            Toggle::Minimal => self.settings.minimal,
            Toggle::ClosedMusic => self.settings.closed_music,
            Toggle::ClosedBars => self.settings.closed_bars,
            Toggle::ClosedSpin => self.settings.closed_spin,
            Toggle::LiquidDrip => self.settings.liquid_drip,
            Toggle::LiquidBubbles => self.settings.liquid_bubbles,
            Toggle::LiquidBulge => self.settings.liquid_bulge,
            Toggle::LiquidJelly => self.settings.liquid_jelly,
            Toggle::ClosedEdge => self.settings.closed_edge,
            Toggle::ClosedSound => self.settings.closed_sound,
            Toggle::LyricsOnline => self.settings.lyrics_online,
            Toggle::EmptyShelf => self.settings.empty_shelf,
            Toggle::HudVolume => self.settings.hud_volume,
            Toggle::HudBrightness => self.settings.hud_brightness,
            Toggle::HudLock => self.settings.hud_lock,
            Toggle::HudBattery => self.settings.hud_battery,
            Toggle::HudNetwork => self.settings.hud_network,
            Toggle::HudAudio => self.settings.hud_audio,
            Toggle::HudUpdate => self.settings.hud_update,
            Toggle::Sound => self.settings.sound,
            Toggle::Clipboard => self.settings.clipboard,
            Toggle::HideIsland => self.settings.hide_island,
            Toggle::Water => self.settings.water,
            Toggle::Timer => self.settings.timer,
            Toggle::Weather => self.settings.weather,
            Toggle::Prayer => self.settings.prayer,
            Toggle::HudPrayer => self.settings.hud_prayer,
            Toggle::Hotkey => self.settings.hotkey,
            Toggle::TabMusic => self.settings.tab_music,
            Toggle::TabStats => self.settings.tab_stats,
            Toggle::TabShelf => self.settings.tab_shelf,
            Toggle::TabTodo => self.settings.tab_todo,
            Toggle::TabCalendar => self.settings.tab_calendar,
            Toggle::TabClipboard => self.settings.tab_clipboard,
        }
    }

    /// Flip an on/off setting, with everything that has to follow from it. Used by the notch's Settings
    /// tab and by this window.
    pub(crate) fn toggle(&mut self, t: Toggle) {
        match t {
            Toggle::Autostart => {
                let on = !self.autostart;
                settings::set_autostart(on);
                self.autostart = settings::autostart_enabled();
            }
            Toggle::HideFullscreen => self.settings.hide_fullscreen = !self.settings.hide_fullscreen,
            // The clock and the awake eyes share the middle of the closed notch, so turning one on turns the other off.
            Toggle::ShowClock => {
                self.settings.show_clock = !self.settings.show_clock;
                if self.settings.show_clock {
                    self.settings.eyes = false;
                }
            }
            Toggle::Clock24 => self.settings.clock_24h = !self.settings.clock_24h,
            Toggle::Eyes => {
                self.settings.eyes = !self.settings.eyes;
                if self.settings.eyes {
                    self.settings.show_clock = false;
                }
            }
            Toggle::IdleEyes => self.settings.idle_eyes = !self.settings.idle_eyes,
            Toggle::Minimal => self.settings.minimal = !self.settings.minimal,
            Toggle::ClosedMusic => self.settings.closed_music = !self.settings.closed_music,
            Toggle::ClosedBars => self.settings.closed_bars = !self.settings.closed_bars,
            Toggle::ClosedSpin => self.settings.closed_spin = !self.settings.closed_spin,
            Toggle::LiquidDrip => self.settings.liquid_drip = !self.settings.liquid_drip,
            Toggle::LiquidBubbles => self.settings.liquid_bubbles = !self.settings.liquid_bubbles,
            Toggle::LiquidBulge => self.settings.liquid_bulge = !self.settings.liquid_bulge,
            Toggle::LiquidJelly => self.settings.liquid_jelly = !self.settings.liquid_jelly,
            Toggle::ClosedEdge => self.settings.closed_edge = !self.settings.closed_edge,
            Toggle::ClosedSound => self.settings.closed_sound = !self.settings.closed_sound,
            Toggle::LyricsOnline => {
                self.settings.lyrics_online = !self.settings.lyrics_online;
                // Look again for the song that plays, now with or without the lyrics site.
                self.restart_lyrics();
            }
            Toggle::EmptyShelf => self.settings.empty_shelf = !self.settings.empty_shelf,
            Toggle::HudVolume => self.settings.hud_volume = !self.settings.hud_volume,
            Toggle::HudBrightness => self.settings.hud_brightness = !self.settings.hud_brightness,
            Toggle::HudLock => self.settings.hud_lock = !self.settings.hud_lock,
            Toggle::HudBattery => self.settings.hud_battery = !self.settings.hud_battery,
            Toggle::HudNetwork => self.settings.hud_network = !self.settings.hud_network,
            Toggle::HudAudio => self.settings.hud_audio = !self.settings.hud_audio,
            Toggle::HudUpdate => {
                self.settings.hud_update = !self.settings.hud_update;
                crate::update::set_auto(self.settings.hud_update, &self.proxy);
            }
            Toggle::Sound => self.settings.sound = !self.settings.sound,
            Toggle::Clipboard => self.settings.clipboard = !self.settings.clipboard,
            Toggle::HideIsland => self.settings.hide_island = !self.settings.hide_island,
            Toggle::Water => {
                if !self.set_tab(View::Water, !self.settings.water) {
                    return;
                }
                if self.settings.water {
                    self.water.restart(Instant::now());
                }
                self.tabs_changed();
            }
            Toggle::Timer => {
                if !self.set_tab(View::Timer, !self.settings.timer) {
                    return;
                }
                if !self.settings.timer {
                    // Switching it off stops a running timer and puts the tab away.
                    self.end_timer_edit(false);
                    self.timer.reset();
                }
                self.tabs_changed();
            }
            Toggle::Weather => {
                if !self.set_tab(View::Weather, !self.settings.weather) {
                    return;
                }
                self.tabs_changed();
                self.sync_weather();
            }
            Toggle::Prayer => {
                if !self.set_tab(View::Prayer, !self.settings.prayer) {
                    return;
                }
                self.tabs_changed();
                self.sync_prayer();
            }
            Toggle::TabMusic | Toggle::TabStats | Toggle::TabShelf | Toggle::TabTodo | Toggle::TabCalendar | Toggle::TabClipboard => {
                let view = match t {
                    Toggle::TabMusic => View::Player,
                    Toggle::TabStats => View::Stats,
                    Toggle::TabShelf => View::Shelf,
                    Toggle::TabTodo => View::Todo,
                    Toggle::TabCalendar => View::Calendar,
                    _ => View::Clipboard,
                };
                if !self.set_tab(view, !self.tab_enabled(view)) {
                    return;
                }
                self.tabs_changed();
            }
            Toggle::HudPrayer => self.settings.hud_prayer = !self.settings.hud_prayer,
            Toggle::Hotkey => {
                self.settings.hotkey = !self.settings.hotkey;
                self.apply_hotkey();
            }
        }
        self.settings_changed();
    }

    /// The tab bar has one tab more or fewer, so its highlight jumps instead of sliding.
    pub(super) fn tabs_changed(&mut self) {
        if let Some(target) = self.tab_target() {
            self.tab_bar.pos = target;
            self.tab_bar.vel = 0.0;
        }
    }

    /// Save the settings and let the notch (and this window) pick them up.
    pub(crate) fn settings_changed(&mut self) {
        self.settings.save();
        crate::clipboard::set_enabled(self.settings.clipboard);
        self.refresh_edges();
        self.apply_visibility();
        self.last_tick = Instant::now();
        if let Some(w) = &self.window {
            w.request_redraw();
        }
        if let Some(p) = &self.panel {
            p.redraw();
        }
    }

    pub(super) fn seg_value(&self, seg: Seg) -> usize {
        match seg {
            Seg::Theme => ThemeMode::ALL.iter().position(|t| *t == self.settings.theme).unwrap_or(0),
            Seg::Look => NotchShape::ALL.iter().position(|l| *l == self.settings.shape).unwrap_or(0),
            Seg::Glass => NotchGlass::ALL.iter().position(|g| *g == self.settings.glass).unwrap_or(1),
            Seg::Motion => Motion::ALL.iter().position(|m| *m == self.settings.motion).unwrap_or(1),
            Seg::Size => NotchSize::ALL.iter().position(|z| *z == self.settings.size).unwrap_or(0),
            Seg::Open => usize::from(self.settings.open_on_click),
            Seg::LiquidOn => usize::from(self.settings.liquid_glass_only),
            Seg::WeekStart => usize::from(self.settings.week_monday),
            Seg::CalView => usize::from(self.settings.calendar_week),
            Seg::ClipKeep => ClipKeep::ALL.iter().position(|k| *k == self.settings.clip_keep).unwrap_or(0),
            Seg::Units => usize::from(self.settings.fahrenheit),
            Seg::Asr => usize::from(self.settings.asr_hanafi),
        }
    }

    pub(super) fn set_seg(&mut self, seg: Seg, i: usize) {
        match seg {
            Seg::Theme => self.settings.theme = ThemeMode::ALL[i.min(2)],
            Seg::Look => {
                self.settings.shape = NotchShape::ALL[i.min(NotchShape::ALL.len() - 1)];
                self.settings.set_look();
            }
            Seg::Glass => {
                self.settings.glass = NotchGlass::ALL[i.min(NotchGlass::ALL.len() - 1)];
                self.settings.set_look();
            }
            Seg::Motion => self.settings.motion = Motion::ALL[i.min(3)],
            Seg::Size => self.settings.size = NotchSize::ALL[i.min(4)],
            Seg::Open => self.settings.open_on_click = i == 1,
            Seg::LiquidOn => self.settings.liquid_glass_only = i == 1,
            Seg::WeekStart => self.settings.week_monday = i == 1,
            Seg::CalView => self.settings.calendar_week = i == 1,
            Seg::ClipKeep => self.settings.clip_keep = ClipKeep::ALL[i.min(3)],
            Seg::Units => self.settings.fahrenheit = i == 1,
            Seg::Asr => self.settings.asr_hanafi = i == 1,
        }
        self.settings_changed();
        if seg == Seg::Asr {
            self.sync_prayer();
        }
        if matches!(seg, Seg::Look | Seg::Glass) {
            // The floating pill's window is taller, and the glass may now need what is behind it.
            self.step();
        }
    }

    /// Choose an Eye style from the picker: save it and show it on the closed notch for a moment.
    pub(super) fn set_eye_style(&mut self, i: usize) {
        self.settings.eye_style = EyeStyle::ALL[i.min(EyeStyle::ALL.len() - 1)];
        self.settings_changed();
        self.preview_eyes();
    }

    /// Step the calculation method to the one before (`-1`) or after (`1`) in the list.
    pub(super) fn step_method(&mut self, delta: isize) {
        let n = crate::prayer::METHODS.len() as isize;
        let i = (crate::prayer::method_index(self.settings.prayer_method) as isize + delta).rem_euclid(n) as usize;
        self.settings.prayer_method = crate::prayer::METHODS[i].0;
        self.settings_changed();
        self.sync_prayer();
    }

    /// Whether the zone applies: only the e-Solat method looks at it.
    pub(super) fn zone_applies(&self) -> bool {
        self.settings.prayer_method == crate::prayer::ESOLAT_METHOD
    }

    /// Step the e-Solat zone to the one before (`-1`) or after (`1`) in the list, "Automatic" being before the first.
    pub(super) fn step_zone(&mut self, delta: isize) {
        if !self.zone_applies() {
            return;
        }
        // 0 is Automatic, 1 the first zone.
        let at = crate::zones::index_of(&self.settings.prayer_zone).map_or(0, |i| i as isize + 1);
        let n = crate::zones::ZONES.len() as isize + 1;
        let to = (at + delta).rem_euclid(n);
        self.settings.prayer_zone = if to == 0 { String::new() } else { crate::zones::ZONES[to as usize - 1].code.to_string() };
        self.settings_changed();
        self.sync_prayer();
    }

    /// What the Zone control says: "Automatic: SGR01 ..." or the chosen zone.
    pub(super) fn zone_text(&self) -> String {
        match crate::zones::find(&self.settings.prayer_zone) {
            Some(z) => crate::zones::describe(z),
            None => match self.settings.place.as_ref().and_then(crate::zones::nearest) {
                Some(z) => format!("{}{}", crate::zones::AUTOMATIC, crate::zones::describe(z)),
                None => "Automatic".to_string(),
            },
        }
    }

    pub(super) fn slider_get(&self, slide: Slide) -> u32 {
        match slide {
            Slide::OpenDelay => self.settings.open_delay,
            Slide::CloseDelay => self.settings.close_delay,
            Slide::Volume => self.settings.volume,
            Slide::Own(slot) => self.settings.slot_volume[slot.index()],
            Slide::AlarmEvery => self.settings.alarm_every,
            Slide::AlarmFor => self.settings.alarm_for,
            Slide::GlassClear => self.settings.glass_clear,
            Slide::GlassFrost => self.settings.glass_frost,
        }
    }

    /// Change a slider's value. It takes effect at once; it is saved when the mouse is let go.
    pub(super) fn slider_set(&mut self, slide: Slide, v: u32) {
        let v = range_of(slide).clamp(v);
        match slide {
            Slide::OpenDelay => self.settings.open_delay = v,
            Slide::CloseDelay => self.settings.close_delay = v,
            Slide::Volume => self.settings.volume = v,
            Slide::Own(slot) => self.settings.slot_volume[slot.index()] = v,
            Slide::AlarmEvery => self.settings.alarm_every = v,
            Slide::AlarmFor => self.settings.alarm_for = v,
            Slide::GlassClear => self.settings.glass_clear = v,
            Slide::GlassFrost => self.settings.glass_frost = v,
        }
    }

    /// Put every setting back to what a fresh install has.
    pub(super) fn reset_settings(&mut self) {
        settings::set_autostart(false);
        self.autostart = false;
        // What the old settings left behind goes too, as when the city is cleared: a sound still playing from its own
        // file, and the forecast and prayer times fetched for the city.
        for slot in crate::sound::Slot::ALL {
            crate::sound::stop_file(slot);
        }
        crate::weather::clear_cache();
        crate::prayer::clear_cache();
        self.settings = Settings::default();
        crate::update::set_auto(self.settings.hud_update, &self.proxy);
        self.end_timer_edit(false);
        self.timer.reset();
        self.apply_hotkey();
        self.ensure_view();
        self.sync_weather();
        self.sync_prayer();
        self.tabs_changed();
        self.settings_changed();
    }
}
