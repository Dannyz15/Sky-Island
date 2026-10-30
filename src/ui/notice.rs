//! The notices (volume, brightness, battery and so on) the closed notch shows.

use crate::*;

/// The notch while it shows a notice (volume, battery...); the notice is laid out for 300 wide and scaled to fit.
pub(crate) const OVERLAY: (f64, f64) = (240.0, 42.0);

const NOTICE_TIME: Duration = Duration::from_millis(2200);

impl App {
    pub(crate) fn show_notice(&mut self, notice: sys::Notice) {
        use sys::Notice;
        let wanted = match &notice {
            Notice::Volume { .. } => self.settings.hud_volume,
            Notice::Brightness { .. } => self.settings.hud_brightness,
            Notice::Lock { .. } => self.settings.hud_lock,
            Notice::Battery { .. } => self.settings.hud_battery,
            Notice::Network { .. } => self.settings.hud_network,
            Notice::Audio { .. } => self.settings.hud_audio,
        };
        if !wanted {
            return;
        }
        let now = Instant::now();
        self.notice = Some(notice);
        self.notice_until = Some(now + NOTICE_TIME);
        self.last_tick = now;
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    pub(crate) fn draw_notice(&self, pm: &mut Pixmap, notice: &sys::Notice, real_s: f32, alpha: f32, fw: f32) {
        use sys::{BatteryKind, Notice};
        let Some(font) = &self.font else { return };
        // Laid out for a 300 wide notch, shrunk to the size of the notch it is shown in.
        let s = real_s * (OVERLAY.0 / 300.0) as f32;
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [160, 160, 165, a]);
        let (green, red, blue) = ([80, 200, 120, a], [255, 90, 80, a], [64, 170, 255, a]);

        // Content block, centered in the notch.
        let (left, right, cy) = (fw / 2.0 - 120.0 * s, fw / 2.0 + 120.0 * s, OVERLAY.1 as f32 / 2.0 * real_s);
        let icon_x = left + 10.0 * s;
        let text_x = left + 34.0 * s;
        let right_text = |pm: &mut Pixmap, text: &str, color: [u8; 4]| {
            let w = text_width(font, text, 13.0 * s);
            draw_text(pm, font, text, right - w, cy + 5.0 * s, 13.0 * s, color, 120.0 * s);
        };

        match notice {
            Notice::Volume { percent, muted } => {
                draw_speaker(pm, icon_x, cy, s, if *muted { grey } else { white }, *muted);
                draw_bar(pm, text_x, cy, 140.0 * s, if *muted { 0.0 } else { *percent as f32 / 100.0 }, blue, s, a);
                right_text(pm, &if *muted { "Muted".to_string() } else { format!("{percent}%") }, if *muted { grey } else { white });
            }
            Notice::Brightness { percent } => {
                draw_sun(pm, icon_x, cy, s, white);
                draw_bar(pm, text_x, cy, 140.0 * s, *percent as f32 / 100.0, [255, 200, 80, a], s, a);
                right_text(pm, &format!("{percent}%"), white);
            }
            Notice::Lock { name, on } => {
                fill_rounded(pm, icon_x - 10.0 * s, cy - 9.0 * s, 20.0 * s, 18.0 * s, 4.0 * s, [255, 255, 255, (alpha * 45.0) as u8]);
                let glyph = if *name == "Caps Lock" { "A" } else { "1" };
                draw_text_centered(pm, font, glyph, icon_x, cy + 5.0 * s, 13.0 * s, white, 18.0 * s);
                draw_text(pm, font, name, text_x, cy + 5.0 * s, 14.0 * s, white, 120.0 * s);
                right_text(pm, if *on { "On" } else { "Off" }, if *on { green } else { grey });
            }
            Notice::Battery { percent, kind } => {
                let color = match kind {
                    BatteryKind::Low => red,
                    BatteryKind::Plugged => green,
                    BatteryKind::Unplugged => white,
                };
                draw_battery(pm, icon_x, cy, s, *percent as f32 / 100.0, color, a);
                let label = match kind {
                    BatteryKind::Plugged => "Charging",
                    BatteryKind::Unplugged => "On battery",
                    BatteryKind::Low => "Battery low",
                };
                draw_text(pm, font, label, text_x, cy + 5.0 * s, 14.0 * s, if *kind == BatteryKind::Low { red } else { white }, 120.0 * s);
                right_text(pm, &format!("{percent}%"), color);
            }
            Notice::Network { connected, name, wifi } => {
                draw_wifi(pm, icon_x, cy, s, if *connected { blue } else { grey }, !*connected);
                let label = match (*connected, name.is_empty()) {
                    (true, false) => name.as_str(),
                    (true, true) => "Connected",
                    (false, _) => "No internet",
                };
                draw_text(pm, font, label, text_x, cy + 5.0 * s, 14.0 * s, white, 110.0 * s);
                let kind = match (*connected, *wifi) {
                    (false, _) => "Offline",
                    (true, true) => "Wi-Fi",
                    (true, false) => "Online",
                };
                right_text(pm, kind, if *connected { grey } else { red });
            }
            Notice::Audio { name, headphones } => {
                draw_headphones(pm, icon_x, cy, s, white);
                draw_text(pm, font, name, text_x, cy + 5.0 * s, 14.0 * s, white, 120.0 * s);
                right_text(pm, if *headphones { "Headphones" } else { "Output" }, grey);
            }
        }
    }
}
