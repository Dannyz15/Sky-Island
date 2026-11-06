//! The Prayer view: the five prayer times of today, the one that is next and how long until it, and the notice
//! that says when a prayer time has come. The times are from `prayer.rs`.

use crate::prayer::{self, Request, Table};
use crate::*;

/// The button that opens the settings when no city is set.
const SET_CITY_BUTTON: (f32, f32, f32, f32) = (150.0, 114.0, 120.0, 22.0);

/// The six columns of the day's times: where each is centred.
const COL_X0: f32 = 24.0;
const COL_W: f32 = 62.0;

/// Seconds since midnight, local time.
fn now_secs() -> u32 {
    let (h, m, s, _) = local_time();
    h * 3600 + m * 60 + s
}

/// What the Prayer tab knows: the prayer times it shows, whether a fetch is on its way or failed, the thread that fetches
/// them (told which request to follow; `None` before the window exists), the prayer a notice was last shown for (day
/// and index), and the minute the Prayer view was last drawn.
#[derive(Default)]
pub(crate) struct PrayerTab {
    pub(crate) table: Option<prayer::Table>,
    pub(crate) loading: bool,
    pub(crate) failed: bool,
    pub(crate) tx: Option<std::sync::mpsc::Sender<Option<prayer::Request>>>,
    pub(crate) told: Option<(i64, usize)>,
    pub(crate) minute: u32,
}

impl App {
    /// What to ask for: the user's city, method and Asr school (and the e-Solat zone, for that method), while the Prayer
    /// tab is on.
    fn prayer_request(&self) -> Option<Request> {
        let place = self.settings.place.clone().filter(|_| self.settings.prayer)?;
        let method = self.settings.prayer_method;
        let zone = if method == prayer::ESOLAT_METHOD { self.settings.prayer_zone.clone() } else { String::new() };
        Some(Request { place, method, school: u32::from(self.settings.asr_hanafi), zone })
    }

    /// Tell the prayer thread what to follow. Run when the window starts and when the tab, the city or a prayer setting
    /// changes. What was kept from last time shows at once while the days are fetched again.
    pub(crate) fn sync_prayer(&mut self) {
        let request = self.prayer_request();
        if self.prayer.table.as_ref().is_some_and(|t| !request.as_ref().is_some_and(|r| t.is_for(r))) {
            self.prayer.table = None;
        }
        if self.prayer.table.is_none()
            && let Some(r) = &request
        {
            self.prayer.table = prayer::load_cache(r);
        }
        self.prayer.failed = false;
        self.prayer.loading = request.is_some();
        if let Some(tx) = &self.prayer.tx {
            let _ = tx.send(request);
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Days (or the news that fetching them failed) arrived.
    pub(crate) fn prayer_arrived(&mut self, table: Option<Table>) {
        self.prayer.loading = false;
        match table {
            Some(t) if self.prayer_request().is_some_and(|r| t.is_for(&r)) => {
                self.prayer.table = Some(t);
                self.prayer.failed = false;
            }
            // Days for a city or a method that is no longer the one, or while the tab is off, are dropped.
            Some(_) => return,
            None => self.prayer.failed = true,
        }
        self.last_tick = Instant::now();
        if self.view == View::Prayer
            && let Some(w) = &self.window
        {
            w.request_redraw();
        }
    }

    /// The tab was opened: fetch again if the days do not reach far enough.
    pub(crate) fn prayer_opened(&mut self) {
        let short = self.prayer.table.as_ref().is_none_or(|t| !t.covers(today_day(), prayer::AHEAD));
        if self.settings.prayer && self.settings.place.is_some() && short && !self.prayer.loading {
            self.sync_prayer();
        }
    }

    /// Say so when a prayer time has come (a notice and a chime), once for each.
    pub(crate) fn check_prayer(&mut self) {
        if !self.settings.prayer || !self.settings.hud_prayer {
            return;
        }
        let Some(table) = &self.prayer.table else { return };
        let today = today_day();
        let Some((index, minute)) = prayer::just_arrived(table, today, now_secs()) else { return };
        if self.prayer.told == Some((today, index)) {
            return;
        }
        self.prayer.told = Some((today, index));
        self.show_notice(sys::Notice::Prayer { name: prayer::NAMES[index], minute });
        self.play_sound(sound::Chime::Reminder);
    }

    /// When the loop has to wake for the next prayer time (a second after it, so the clock has reached it).
    pub(crate) fn prayer_wake(&self, now: Instant) -> Option<Instant> {
        if !self.settings.prayer || !self.settings.hud_prayer {
            return None;
        }
        let (today, secs) = (today_day(), now_secs());
        let next = prayer::next_prayer(self.prayer.table.as_ref()?, today, secs)?;
        Some(now + Duration::from_secs(prayer::secs_until(&next, today, secs).max(0) as u64 + 1))
    }

    /// While the Prayer view is open its countdown changes with the minute: redraw then, and say when to wake.
    pub(crate) fn prayer_minute_tick(&mut self) -> Option<Duration> {
        if self.progress < 1.0 || self.view != View::Prayer {
            return None;
        }
        let (h, m, s, ms) = local_time();
        let minute = h * 60 + m;
        if minute != self.prayer.minute {
            self.prayer.minute = minute;
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
        Some(Duration::from_millis(u64::from(59 - s) * 1000 + u64::from(1000 - ms) + 20))
    }

    pub(crate) fn draw_prayer(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let Some(font) = &self.font else { return };
        let heading = self.heading_font().unwrap_or(font);
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [165, 165, 175, a]);
        let rgb = self.accent(View::Prayer);
        let centred = |pm: &mut Pixmap, text: &str, y: f32, ink: [u8; 4]| draw_text_centered(pm, font, text, 210.0 * s, y * s, 12.0 * s, ink, 340.0 * s);

        let Some(place) = &self.settings.place else {
            draw_moon(pm, 210.0 * s, 62.0 * s, 20.0 * s, [236, 236, 210, a]);
            centred(pm, "Set your city to see the prayer times", 98.0, grey);
            draw_chip(pm, font, SET_CITY_BUTTON, "Open Settings", false, self.hot_t(SET_CITY_BUTTON, s), rgb, s, alpha);
            return;
        };
        let (today, secs) = (today_day(), now_secs());
        let day = self.prayer.table.as_ref().and_then(|t| t.day(today));
        let (Some(table), Some(day)) = (&self.prayer.table, day) else {
            let text = if self.prayer.failed || (self.prayer.table.is_some() && !self.prayer.loading) {
                "Prayer times are not available. Are you online?"
            } else {
                "Loading the prayer times..."
            };
            draw_moon(pm, 210.0 * s, 66.0 * s, 20.0 * s, [236, 236, 210, a]);
            centred(pm, text, 108.0, grey);
            draw_text_centered(pm, font, &place.name, 210.0 * s, 126.0 * s, 10.5 * s, [130, 130, 144, a], 340.0 * s);
            return;
        };

        let f24 = self.settings.clock_24h;
        let time = |minute: u32| fmt_clock(minute / 60, minute % 60, f24);
        let next = prayer::next_prayer(table, today, secs);
        gfx::glow(pm, 70.0 * s, 62.0 * s, 70.0 * s, [rgb[0], rgb[1], rgb[2], (alpha * 26.0) as u8], None);

        // Left: the next prayer and how long until it.
        if let Some(n) = next {
            let when = if n.date > today { format!("tomorrow at {}", time(n.minute)) } else { format!("at {}", time(n.minute)) };
            draw_text(pm, font, &format!("Next: {}", prayer::NAMES[n.index]), 24.0 * s, 50.0 * s, 12.0 * s, [rgb[0], rgb[1], rgb[2], a], 200.0 * s);
            draw_text(pm, heading, &prayer::countdown(prayer::secs_until(&n, today, secs)), 24.0 * s, 80.0 * s, 26.0 * s, white, 220.0 * s);
            draw_text(pm, font, &when, 24.0 * s, 98.0 * s, 11.0 * s, grey, 200.0 * s);
        }
        // Right: the city and the Hijri date.
        let mut line = |y: f32, text: &str, c: [u8; 4]| {
            let w = text_width(font, text, 11.0 * s).min(170.0 * s);
            draw_text(pm, font, text, 396.0 * s - w, y * s, 11.0 * s, c, 170.0 * s);
        };
        line(50.0, &place.name, [200, 200, 210, a]);
        line(66.0, &prayer::hijri_text(day.hijri), grey);

        // The day's times, the next one marked.
        fill_rect(pm, 24.0 * s, 110.0 * s, 372.0 * s, s, [255, 255, 255, (alpha * 14.0) as u8]);
        for (i, name) in prayer::NAMES.iter().enumerate() {
            let cx = COL_X0 + COL_W * (i as f32 + 0.5);
            let is_next = next.is_some_and(|n| n.date == today && n.index == i);
            let passed = day.times[i] * 60 <= secs;
            if is_next {
                let (x, y, w, h) = ((cx - 29.0) * s, 117.0 * s, 58.0 * s, 44.0 * s);
                gfx::gradient_rounded(pm, x, y, w, h, 10.0 * s, [rgb[0], rgb[1], rgb[2], (alpha * 70.0) as u8], [rgb[0], rgb[1], rgb[2], (alpha * 30.0) as u8], false);
                gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, 10.0 * s, s, [rgb[0], rgb[1], rgb[2], (alpha * 100.0) as u8]);
            }
            let (name_ink, time_ink) = match (is_next, passed || i == prayer::SUNRISE) {
                (true, _) => (white, white),
                (false, true) => ([125, 125, 138, a], [150, 150, 162, a]),
                (false, false) => (grey, [225, 225, 232, a]),
            };
            draw_text_centered(pm, font, name, cx * s, 133.0 * s, 10.5 * s, name_ink, (COL_W - 4.0) * s);
            draw_text_centered(pm, font, &time(day.times[i]), cx * s, 151.0 * s, 11.5 * s, time_ink, (COL_W - 2.0) * s);
        }
    }

    /// A click in the Prayer view.
    pub(crate) fn click_prayer(&mut self, cx: f32, cy: f32) {
        let r = SET_CITY_BUTTON;
        if self.settings.place.is_none() && cx >= r.0 && cx <= r.0 + r.2 && cy >= r.1 && cy <= r.1 + r.3 {
            self.open_settings_at(panel::Page::Location);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_six_columns_fit_the_notch() {
        assert!(COL_X0 >= 8.0 && COL_X0 + COL_W * 6.0 <= EXPANDED.0 as f32 - 8.0);
    }
}
