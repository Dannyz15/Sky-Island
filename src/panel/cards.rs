//! The cards at the top of the Calendar and About pages.

use super::*;

impl App {
    /// A small calendar for today's month (or week) as the settings below make it look.
    pub(super) fn paint_calendar_preview(&self, pm: &mut Pixmap, font: &fontdue::Font, heading: &fontdue::Font, hero: Rect, tint: [u8; 3], s: f32) {
        let (x, y, w, _) = hero;
        crate::draw_card(pm, hero, 16.0, s, 1.0, 0.0, tint);
        let (white, grey, muted) = ([255, 255, 255, 255], [160, 160, 172, 255], [96, 96, 108, 255]);
        let (monday, week) = (self.settings.week_monday, self.settings.calendar_week);
        let today = crate::today_day();
        let (year, month, _) = crate::calendar::civil_from_days(today);
        let first = crate::calendar::days_from_civil(year, month, 1);
        let heads = crate::calendar::week_heads(monday);
        let (cell_w, cell_h) = (42.0, if week { 62.0 } else { 27.0 });
        let left = x + (w - 7.0 * cell_w) / 2.0;
        let title = if week {
            crate::calendar::week_title(crate::calendar::week_start(today, monday))
        } else {
            format!("{} {year}", ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"][month as usize - 1])
        };
        crate::draw_text_centered(pm, heading, &title, (x + w / 2.0) * s, (y + 28.0) * s, 14.0 * s, [210, 210, 220, 255], (w - 40.0) * s);
        for (c, head) in heads.iter().enumerate() {
            let ink = if crate::calendar::is_weekend(c, monday) { gfx::mix(grey, [tint[0], tint[1], tint[2], 255], 0.5) } else { grey };
            crate::draw_text_centered(pm, font, head, (left + (c as f32 + 0.5) * cell_w) * s, (y + 52.0) * s, 10.5 * s, ink, cell_w * s);
        }
        let top = y + 62.0;
        let start = if week { crate::calendar::week_start(today, monday) } else { first - crate::calendar::column(first, monday) as i64 };
        let rows = if week { 1 } else { 6 };
        for r in 0..rows {
            for c in 0..7 {
                let day = start + (r * 7 + c) as i64;
                let (_, m, dom) = crate::calendar::civil_from_days(day);
                let (cx, cy) = (left + (c as f32 + 0.5) * cell_w, top + r as f32 * cell_h + cell_h / 2.0);
                let is_today = day == today;
                if is_today {
                    let r = if week { 20.0 } else { 11.0 };
                    gfx::glow(pm, cx * s, cy * s, (r + 8.0) * s, [tint[0], tint[1], tint[2], 60], None);
                    crate::fill_circle(pm, cx * s, cy * s, r * s, [tint[0], tint[1], tint[2], 255]);
                }
                let ink = if is_today {
                    white
                } else if m != month && !week {
                    muted
                } else {
                    [214, 214, 224, 255]
                };
                crate::draw_text_centered(
                    pm,
                    if is_today { heading } else { font },
                    &dom.to_string(),
                    cx * s,
                    (cy + if week { 6.0 } else { 4.5 }) * s,
                    if week { 18.0 } else { 12.0 } * s,
                    ink,
                    cell_w * s,
                );
                let (open, total) = self.todo.day_counts(day);
                if total > 0 {
                    let dot = if is_today {
                        white
                    } else if open > 0 {
                        [255, 190, 70, 255]
                    } else {
                        [110, 110, 120, 255]
                    };
                    crate::fill_circle(pm, cx * s, (cy + if week { 20.0 } else { 11.0 }) * s, 1.6 * s, dot);
                }
            }
        }
    }

    /// The card at the top of the About page.
    pub(super) fn paint_hero(&self, pm: &mut Pixmap, font: &fontdue::Font, heading: &fontdue::Font, hero: Rect, tint: [u8; 3], s: f32) {
        let (x, y, w, h) = hero;
        crate::draw_card(pm, hero, 16.0, s, 1.0, 0.0, tint);
        if let Some(path) = gfx::rounded_path(x * s, y * s, w * s, h * s, 16.0 * s)
            && let Some(mask) = gfx::mask_of(pm.width(), pm.height(), &path)
        {
            gfx::glow(pm, (x + w / 2.0) * s, (y + 44.0) * s, 190.0 * s, [tint[0], tint[1], tint[2], 36], Some(&mask));
        }
        let logo = appicon::cached(128);
        // The logo at the top, the name, the description and the version centred under it.
        let side = 64.0;
        let scale = side * s / logo.width() as f32;
        let paint = tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
        pm.draw_pixmap(0, 0, logo.as_ref(), &paint, Transform::from_scale(scale, scale).post_translate((x + (w - side) / 2.0) * s, (y + 12.0) * s), None);
        let middle = |font: &fontdue::Font, text: &str, size: f32| (x + w / 2.0) * s - crate::text_width(font, text, size * s) / 2.0;
        let name = "Sky Island";
        crate::draw_text(pm, heading, name, middle(heading, name, 21.0), (y + 100.0) * s, 21.0 * s, [255, 255, 255, 255], w * s);
        let tagline = "A notch for the top of your Windows screen.";
        crate::draw_text(pm, font, tagline, middle(font, tagline, 11.5), (y + 120.0) * s, 11.5 * s, [170, 170, 182, 255], w * s);
        let version = format!("Version {}", env!("CARGO_PKG_VERSION"));
        crate::draw_text(pm, font, &version, middle(font, &version, 11.0), (y + 138.0) * s, 11.0 * s, [tint[0], tint[1], tint[2], 255], w * s);
    }
}
