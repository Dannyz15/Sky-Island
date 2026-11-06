//! The Weather view: what it is like now in the user's city and the days ahead, from `weather.rs`.

use crate::weather::{self, Report, Sky};
use crate::*;

/// The button that opens the settings when no city is set.
const SET_CITY_BUTTON: (f32, f32, f32, f32) = (150.0, 114.0, 120.0, 22.0);

/// The strip of days under the current weather: where each column is centred.
const DAY_COLUMNS: usize = 5;
const DAY_X0: f32 = 52.0;
const DAY_STEP: f32 = 79.0;

/// What the Weather tab knows: the forecast it shows, when it was fetched, whether a fetch is on its way or failed, and
/// the thread that fetches it (told which place to watch; `None` before the window exists).
#[derive(Default)]
pub(crate) struct WeatherTab {
    pub(crate) report: Option<Report>,
    pub(crate) at: Option<Instant>,
    pub(crate) loading: bool,
    pub(crate) failed: bool,
    pub(crate) tx: Option<std::sync::mpsc::Sender<Option<weather::Place>>>,
}

impl App {
    /// Tell the weather thread which city to follow: the user's, while the Weather tab is on, and nothing
    /// otherwise. Run when the window starts and when the tab, the city or the settings change. What was kept
    /// from last time is shown at once while a new forecast is on its way.
    pub(crate) fn sync_weather(&mut self) {
        let place = self.settings.place.clone().filter(|_| self.settings.weather);
        if self.weather.report.as_ref().is_some_and(|r| !place.as_ref().is_some_and(|p| p.same_spot(r.lat, r.lon))) {
            self.weather.report = None;
            self.weather.at = None;
        }
        if self.weather.report.is_none()
            && let Some(p) = &place
        {
            self.weather.report = weather::load_cache(p);
        }
        self.weather.failed = false;
        self.weather.loading = place.is_some();
        if let Some(tx) = &self.weather.tx {
            let _ = tx.send(place);
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// A forecast (or the news that fetching it failed) arrived.
    pub(crate) fn weather_arrived(&mut self, report: Option<Report>) {
        self.weather.loading = false;
        match report {
            Some(r) if self.settings.weather && self.settings.place.as_ref().is_some_and(|p| p.same_spot(r.lat, r.lon)) => {
                self.weather.report = Some(r);
                self.weather.at = Some(Instant::now());
                self.weather.failed = false;
            }
            // An answer for a city that is no longer the one, or while the tab is off, is dropped.
            Some(_) => return,
            None => self.weather.failed = true,
        }
        self.last_tick = Instant::now();
        if self.view == View::Weather
            && let Some(w) = &self.window
        {
            w.request_redraw();
        }
    }

    /// The tab was opened: look again if what it has is more than ten minutes old.
    pub(crate) fn weather_opened(&mut self) {
        let old = self.weather.at.is_none_or(|t| t.elapsed() > weather::STALE);
        if self.settings.weather && self.settings.place.is_some() && old && !self.weather.loading {
            self.sync_weather();
        }
    }

    pub(crate) fn draw_weather(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let Some(font) = &self.font else { return };
        let heading = self.heading_font().unwrap_or(font);
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [165, 165, 175, a]);
        let rgb = self.accent(View::Weather);

        let Some(place) = &self.settings.place else {
            draw_sky(pm, Sky::PartlyCloudy, true, 210.0 * s, 62.0 * s, 1.1 * s, a);
            draw_text_centered(pm, font, "Set your city to see the weather", 210.0 * s, 98.0 * s, 12.0 * s, grey, 340.0 * s);
            draw_chip(pm, font, SET_CITY_BUTTON, "Open Settings", false, self.hot_t(SET_CITY_BUTTON, s), rgb, s, alpha);
            return;
        };
        let Some(report) = &self.weather.report else {
            let text = if self.weather.failed { "Weather is not available. Are you online?" } else { "Loading the weather..." };
            draw_sky(pm, Sky::Cloudy, true, 210.0 * s, 70.0 * s, 1.1 * s, a);
            draw_text_centered(pm, font, text, 210.0 * s, 108.0 * s, 12.0 * s, grey, 340.0 * s);
            draw_text_centered(pm, font, &place.name, 210.0 * s, 126.0 * s, 10.5 * s, [130, 130, 144, a], 340.0 * s);
            return;
        };

        let f = self.settings.fahrenheit;
        let sky = weather::sky(report.code);
        let tint = |c: [u8; 3], k: f32| [c[0], c[1], c[2], (alpha * k) as u8];
        gfx::glow(pm, 68.0 * s, 64.0 * s, 70.0 * s, tint(rgb, 30.0), None);

        // The current weather: the sky, the temperature, and a few details.
        draw_sky(pm, sky, report.daytime, 68.0 * s, 64.0 * s, 1.3 * s, a);
        let temp = format!("{}\u{00B0}", weather::degrees(report.temp, f));
        draw_text(pm, heading, &temp, 116.0 * s, 78.0 * s, 38.0 * s, white, 120.0 * s);
        let tw = text_width(heading, &temp, 38.0 * s);
        draw_text(pm, font, if f { "F" } else { "C" }, 116.0 * s + tw + 2.0 * s, 62.0 * s, 12.0 * s, grey, 20.0 * s);
        draw_text(pm, font, weather::describe(report.code), 116.0 * s, 98.0 * s, 12.5 * s, white, 150.0 * s);

        let right = 396.0 * s;
        let mut line = |y: f32, text: &str, c: [u8; 4]| {
            let w = text_width(font, text, 11.0 * s).min(160.0 * s);
            draw_text(pm, font, text, right - w, y * s, 11.0 * s, c, 160.0 * s);
        };
        line(50.0, &place.name, [200, 200, 210, a]);
        let today = report.days.first();
        if let Some(d) = today {
            line(68.0, &format!("High {}\u{00B0}  Low {}\u{00B0}", weather::degrees(d.high, f), weather::degrees(d.low, f)), white);
        }
        line(84.0, &format!("Feels like {}\u{00B0}", weather::degrees(report.feels, f)), grey);
        line(100.0, &format!("Humidity {}%", report.humidity), grey);
        line(116.0, &format!("Wind {}", weather::wind_text(report.wind, f)), grey);

        // The days ahead.
        fill_rect(pm, 24.0 * s, 118.0 * s, 372.0 * s, s, [255, 255, 255, (alpha * 14.0) as u8]);
        for (i, day) in report.days.iter().skip(1).take(DAY_COLUMNS).enumerate() {
            let cx = (DAY_X0 + i as f32 * DAY_STEP) * s;
            let name = day_name(&day.date);
            draw_text_centered(pm, font, name, cx, 131.0 * s, 10.5 * s, grey, 70.0 * s);
            draw_sky(pm, weather::sky(day.code), true, cx, 144.0 * s, 0.42 * s, a);
            let range = format!("{}\u{00B0} / {}\u{00B0}", weather::degrees(day.high, f), weather::degrees(day.low, f));
            draw_text_centered(pm, font, &range, cx, 161.0 * s, 10.0 * s, [215, 215, 225, a], 76.0 * s);
        }
    }

    /// A click in the Weather view.
    pub(crate) fn click_weather(&mut self, cx: f32, cy: f32) {
        let r = SET_CITY_BUTTON;
        if self.settings.place.is_none() && cx >= r.0 && cx <= r.0 + r.2 && cy >= r.1 && cy <= r.1 + r.3 {
            self.open_settings_at(panel::Page::Location);
        }
    }
}

/// "Mon", "Tue"... for a date written "2026-11-03"; nothing if it is not a date.
fn day_name(date: &str) -> &'static str {
    const NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let mut parts = date.split('-').map(|p| p.parse::<u32>().ok());
    let (Some(Some(y)), Some(Some(m)), Some(Some(d))) = (parts.next(), parts.next(), parts.next()) else { return "" };
    let days = calendar::days_from_civil(y as i32, m, d);
    NAMES[calendar::weekday(days)]
}

/// One drawing of the sky, centred on (`cx`, `cy`) with a size of `k` pixels per unit: the picture is about
/// 40 units across. Clear nights show a moon instead of the sun.
pub(crate) fn draw_sky(pm: &mut Pixmap, sky: Sky, daytime: bool, cx: f32, cy: f32, k: f32, a: u8) {
    let sun = [255, 196, 64, a];
    let moon = [236, 236, 210, a];
    let cloud = [226, 231, 240, a];
    let grey_cloud = [160, 168, 184, a];
    let blue = [90, 170, 255, a];
    let p = |dx: f32, dy: f32| (cx + dx * k, cy + dy * k);
    let body = |pm: &mut Pixmap, x: f32, y: f32, r: f32| {
        if daytime { draw_sun_disc(pm, cx + x * k, cy + y * k, r * k, sun) } else { draw_moon(pm, cx + x * k, cy + y * k, r * k, moon) }
    };
    match sky {
        Sky::Clear => body(pm, 0.0, 0.0, 10.0),
        Sky::PartlyCloudy => {
            body(pm, -7.0, -7.0, 8.0);
            draw_cloud(pm, cx + 3.0 * k, cy + 5.0 * k, 0.85 * k, cloud);
        }
        Sky::Cloudy => {
            draw_cloud(pm, cx + 5.0 * k, cy - 6.0 * k, 0.6 * k, grey_cloud);
            draw_cloud(pm, cx - 1.0 * k, cy + 2.0 * k, 1.0 * k, cloud);
        }
        Sky::Fog => {
            draw_cloud(pm, cx, cy - 6.0 * k, 0.95 * k, grey_cloud);
            for (i, (x1, x2)) in [(-14.0, 14.0), (-10.0, 18.0), (-14.0, 8.0)].into_iter().enumerate() {
                let (a1, b1) = p(x1, 8.0 + i as f32 * 5.0);
                let (a2, _) = p(x2, 0.0);
                stroke_line(pm, a1, b1, a2, b1, 2.2 * k, [190, 196, 208, a]);
            }
        }
        Sky::Drizzle | Sky::Rain => {
            draw_cloud(pm, cx, cy - 6.0 * k, 0.95 * k, grey_cloud);
            let len = if sky == Sky::Rain { 8.0 } else { 4.5 };
            for x in [-9.0, 0.0, 9.0] {
                let (x1, y1) = p(x, 8.0);
                stroke_line(pm, x1, y1, x1 - 2.4 * k, y1 + len * k, 2.0 * k, blue);
            }
        }
        Sky::Snow => {
            draw_cloud(pm, cx, cy - 6.0 * k, 0.95 * k, grey_cloud);
            for (x, y) in [(-9.0, 10.0), (0.0, 14.0), (9.0, 10.0)] {
                let (x1, y1) = p(x, y);
                fill_circle(pm, x1, y1, 1.9 * k, [255, 255, 255, a]);
            }
        }
        Sky::Thunder => {
            draw_cloud(pm, cx, cy - 6.0 * k, 0.95 * k, [120, 128, 146, a]);
            fill_path(pm, &[p(1.0, 4.0), p(-5.0, 13.0), p(-0.5, 13.0), p(-3.0, 20.0), p(5.0, 10.0), p(0.5, 10.0), p(3.5, 4.0)], sun);
        }
    }
}

/// A cloud about 34 units wide, its middle at (`cx`, `cy`).
fn draw_cloud(pm: &mut Pixmap, cx: f32, cy: f32, k: f32, c: [u8; 4]) {
    fill_rounded(pm, cx - 15.0 * k, cy - 2.0 * k, 30.0 * k, 11.0 * k, 5.5 * k, c);
    fill_circle(pm, cx - 6.0 * k, cy - 1.0 * k, 6.5 * k, c);
    fill_circle(pm, cx + 3.0 * k, cy - 4.5 * k, 8.5 * k, c);
    fill_circle(pm, cx + 10.0 * k, cy + 0.5 * k, 5.0 * k, c);
}

/// A sun: a disc with rays.
fn draw_sun_disc(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, c: [u8; 4]) {
    fill_circle(pm, cx, cy, r * 0.62, c);
    for i in 0..8 {
        let ang = i as f32 * std::f32::consts::FRAC_PI_4;
        let (dx, dy) = (ang.cos(), ang.sin());
        stroke_line(pm, cx + dx * r * 0.85, cy + dy * r * 0.85, cx + dx * r * 1.15, cy + dy * r * 1.15, r * 0.17, c);
    }
}

/// A crescent moon of radius `r`, opening towards the top right.
pub(crate) fn draw_moon(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, c: [u8; 4]) {
    fill_path(pm, &crescent(cx, cy, r), c);
}

/// The outline of a crescent: a circle of radius `r` with a smaller circle taken out of its upper right.
fn crescent(cx: f32, cy: f32, r: f32) -> Vec<(f32, f32)> {
    use std::f32::consts::PI;
    let (inner_r, d) = (r * 0.82, r * 0.55);
    let dir = -PI / 4.0; // towards the upper right (y grows downwards)
    let (ox, oy) = (d * dir.cos(), d * dir.sin());
    // Where the two circles cross, as angles round the outer circle.
    let alpha = ((d * d + r * r - inner_r * inner_r) / (2.0 * d * r)).clamp(-1.0, 1.0).acos();
    let mut pts = Vec::new();
    let steps = 28;
    // The part of the outer circle that is not inside the inner one: from one crossing, the long way round, to the other.
    for i in 0..=steps {
        let ang = dir + alpha + (2.0 * PI - 2.0 * alpha) * i as f32 / steps as f32;
        pts.push((cx + r * ang.cos(), cy + r * ang.sin()));
    }
    // Back along the inner circle, on the side that faces the middle of the outer one, starting at the crossing
    // the outer arc ended on.
    let beta = ((inner_r * inner_r + d * d - r * r) / (2.0 * inner_r * d)).clamp(-1.0, 1.0).acos();
    let centre = dir + PI;
    let on_inner = |ang: f32| (cx + ox + inner_r * ang.cos(), cy + oy + inner_r * ang.sin());
    let last = pts[pts.len() - 1];
    let near = |p: (f32, f32)| (p.0 - last.0).hypot(p.1 - last.1);
    let (from, to) = if near(on_inner(centre - beta)) <= near(on_inner(centre + beta)) { (centre - beta, centre + beta) } else { (centre + beta, centre - beta) };
    for i in 1..steps {
        pts.push(on_inner(from + (to - from) * i as f32 / steps as f32));
    }
    pts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_names_its_weekday() {
        // 2026-11-02 is a Monday.
        assert_eq!(day_name("2026-11-02"), "Mon");
        assert_eq!(day_name("2026-11-08"), "Sun");
        assert_eq!(day_name("soon"), "");
    }

    #[test]
    fn the_crescent_stays_inside_its_circle() {
        for (x, y) in crescent(0.0, 0.0, 10.0) {
            assert!(x.hypot(y) <= 10.01, "({x}, {y}) is outside the circle");
        }
        // It is a crescent, not the whole disc: the upper right of the disc is not in it.
        assert!(!crescent(0.0, 0.0, 10.0).iter().any(|&(x, y)| x > 6.0 && y < -6.0));
    }

    #[test]
    fn the_weather_day_columns_fit_the_notch() {
        let last = DAY_X0 + (DAY_COLUMNS as f32 - 1.0) * DAY_STEP;
        assert!(last + 38.0 <= EXPANDED.0 as f32 - 8.0 && DAY_X0 - 38.0 >= 8.0);
    }
}
