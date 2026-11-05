//! Drawing the controls of a page: the cards at the top of Calendar and About, switches, choices, sliders and buttons, and the page icons.

use super::*;

pub(super) fn fill_poly(pm: &mut Pixmap, points: &[(f32, f32)], c: [u8; 4]) {
    let mut pb = PathBuilder::new();
    for (i, (x, y)) in points.iter().enumerate() {
        if i == 0 {
            pb.move_to(*x, *y);
        } else {
            pb.line_to(*x, *y);
        }
    }
    pb.close();
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        paint.anti_alias = true;
        pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }
}

/// A line (or ring) of a given width.
pub(super) fn stroke_points(pm: &mut Pixmap, points: &[(f32, f32)], close: bool, width: f32, c: [u8; 4]) {
    let mut pb = PathBuilder::new();
    for (i, (x, y)) in points.iter().enumerate() {
        if i == 0 {
            pb.move_to(*x, *y);
        } else {
            pb.line_to(*x, *y);
        }
    }
    if close {
        pb.close();
    }
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        paint.anti_alias = true;
        let stroke = Stroke { width, line_cap: tiny_skia::LineCap::Round, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
        pm.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

/// Points along an arc of a circle, in degrees (0 is to the right, 90 is down).
pub(super) fn arc_points(cx: f32, cy: f32, r: f32, from: f32, to: f32, steps: usize) -> Vec<(f32, f32)> {
    (0..=steps)
        .map(|i| {
            let a = (from + (to - from) * i as f32 / steps as f32).to_radians();
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

/// The coloured tile with a white picture on it, for a page in the sidebar. `x`, `y` is its top left.
pub(super) fn draw_page_icon(pm: &mut Pixmap, page: Page, x: f32, y: f32, s: f32) {
    let t = page.tint();
    let from = gfx::mix([t[0], t[1], t[2], 255], [255, 255, 255, 255], 0.22);
    let to = [t[0], t[1], t[2], 255];
    let size = 26.0;
    gfx::gradient_rounded(pm, x * s, y * s, size * s, size * s, 7.0 * s, from, to, false);
    let (cx, cy) = ((x + size / 2.0) * s, (y + size / 2.0) * s);
    let w = [255, 255, 255, 255];
    let u = s; // one logical pixel
    match page {
        Page::General => {
            // A gear: a ring of teeth with a hole in the middle.
            let mut pts = Vec::new();
            for i in 0..16 {
                let a = (i as f32 * 22.5 - 11.25).to_radians();
                let r = if (i / 2) % 2 == 0 { 8.2 } else { 6.2 } * u;
                pts.push((cx + r * a.cos(), cy + r * a.sin()));
                let a2 = (i as f32 * 22.5 + 11.25).to_radians();
                pts.push((cx + r * a2.cos(), cy + r * a2.sin()));
            }
            fill_poly(pm, &pts, w);
            crate::fill_circle(pm, cx, cy, 2.8 * u, gfx::mix(from, to, 0.5));
        }
        Page::Notch => {
            // A screen with the notch at the top.
            gfx::stroke_rounded(pm, cx - 8.0 * u, cy - 6.0 * u, 16.0 * u, 12.0 * u, 2.5 * u, 1.6 * u, w);
            crate::fill_rounded(pm, cx - 3.4 * u, cy - 6.4 * u, 6.8 * u, 3.4 * u, 1.7 * u, w);
        }
        Page::Appearance => {
            // A circle, half of it filled.
            stroke_points(pm, &arc_points(cx, cy, 6.6 * u, 0.0, 360.0, 36), true, 1.6 * u, w);
            let mut half = arc_points(cx, cy, 6.6 * u, -90.0, 90.0, 18);
            half.push((cx, cy + 6.6 * u));
            fill_poly(pm, &half, w);
        }
        Page::Tabs => {
            // Four tiles.
            for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                crate::fill_rounded(pm, cx + dx * 3.5 * u - 3.0 * u, cy + dy * 3.5 * u - 3.0 * u, 6.0 * u, 6.0 * u, 1.6 * u, w);
            }
        }
        Page::Location => {
            // A map pin: a drop with a hole.
            let top = cy - 2.6 * u;
            let mut pts = arc_points(cx, top, 5.6 * u, 150.0, 390.0, 20);
            pts.push((cx, cy + 8.0 * u));
            fill_poly(pm, &pts, w);
            crate::fill_circle(pm, cx, top, 2.2 * u, gfx::mix(from, to, 0.5));
        }
        Page::Sounds => {
            // A speaker with two waves.
            fill_poly(pm, &[(cx - 7.5 * u, cy - 2.4 * u), (cx - 4.0 * u, cy - 2.4 * u), (cx + 0.6 * u, cy - 6.4 * u), (cx + 0.6 * u, cy + 6.4 * u), (cx - 4.0 * u, cy + 2.4 * u), (cx - 7.5 * u, cy + 2.4 * u)], w);
            stroke_points(pm, &arc_points(cx + 1.0 * u, cy, 4.2 * u, -48.0, 48.0, 8), false, 1.4 * u, w);
            stroke_points(pm, &arc_points(cx + 1.0 * u, cy, 7.4 * u, -48.0, 48.0, 8), false, 1.4 * u, w);
        }
        Page::Calendar => {
            // A page with a header strip and two rings, and a few dots for days.
            gfx::stroke_rounded(pm, cx - 7.5 * u, cy - 6.0 * u, 15.0 * u, 13.5 * u, 2.6 * u, 1.6 * u, w);
            crate::fill_rect(pm, cx - 7.5 * u, cy - 3.0 * u, 15.0 * u, 1.6 * u, w);
            crate::fill_rounded(pm, cx - 3.8 * u, cy - 8.0 * u, 1.8 * u, 3.4 * u, 0.9 * u, w);
            crate::fill_rounded(pm, cx + 2.0 * u, cy - 8.0 * u, 1.8 * u, 3.4 * u, 0.9 * u, w);
            for (dx, dy) in [(-3.4, 1.0), (0.0, 1.0), (3.4, 1.0), (-3.4, 4.2), (0.0, 4.2)] {
                crate::fill_circle(pm, cx + dx * u, cy + dy * u, 0.9 * u, w);
            }
        }
        Page::Shelf => {
            // An open tray with an arrow going in.
            stroke_points(pm, &[(cx - 7.5 * u, cy + 0.5 * u), (cx - 7.5 * u, cy + 6.5 * u), (cx + 7.5 * u, cy + 6.5 * u), (cx + 7.5 * u, cy + 0.5 * u)], false, 1.7 * u, w);
            stroke_points(pm, &[(cx - 7.5 * u, cy + 0.5 * u), (cx - 3.2 * u, cy + 0.5 * u), (cx - 2.0 * u, cy + 3.0 * u), (cx + 2.0 * u, cy + 3.0 * u), (cx + 3.2 * u, cy + 0.5 * u), (cx + 7.5 * u, cy + 0.5 * u)], false, 1.7 * u, w);
            stroke_points(pm, &[(cx, cy - 7.0 * u), (cx, cy - 0.5 * u)], false, 1.7 * u, w);
            fill_poly(pm, &[(cx - 3.2 * u, cy - 2.4 * u), (cx + 3.2 * u, cy - 2.4 * u), (cx, cy + 1.2 * u)], w);
        }
        Page::Notices => {
            // A bell.
            let mut bell = arc_points(cx, cy - 1.0 * u, 5.4 * u, 180.0, 360.0, 14);
            bell.extend([(cx + 5.4 * u, cy + 3.8 * u), (cx + 7.4 * u, cy + 5.2 * u), (cx - 7.4 * u, cy + 5.2 * u), (cx - 5.4 * u, cy + 3.8 * u)]);
            fill_poly(pm, &bell, w);
            crate::fill_circle(pm, cx, cy + 7.2 * u, 1.5 * u, w);
            crate::fill_rounded(pm, cx - 0.9 * u, cy - 7.6 * u, 1.8 * u, 2.6 * u, 0.9 * u, w);
        }
        Page::About => {
            // An "i" in a circle.
            stroke_points(pm, &arc_points(cx, cy, 7.4 * u, 0.0, 360.0, 36), true, 1.6 * u, w);
            crate::fill_circle(pm, cx, cy - 3.2 * u, 1.1 * u, w);
            crate::fill_rounded(pm, cx - 0.9 * u, cy - 0.8 * u, 1.8 * u, 5.4 * u, 0.9 * u, w);
        }
    }
}

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
                let ink = if is_today { white } else if m != month && !week { muted } else { [214, 214, 224, 255] };
                crate::draw_text_centered(pm, if is_today { heading } else { font }, &dom.to_string(), cx * s, (cy + if week { 6.0 } else { 4.5 }) * s, if week { 18.0 } else { 12.0 } * s, ink, cell_w * s);
                let (open, total) = self.todo.day_counts(day);
                if total > 0 {
                    let dot = if is_today { white } else if open > 0 { [255, 190, 70, 255] } else { [110, 110, 120, 255] };
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
            && let Some(mask) = gfx::mask_of(pm.width(), pm.height(), &path) {
                gfx::glow(pm, (x + w / 2.0) * s, (y + 44.0) * s, 190.0 * s, [tint[0], tint[1], tint[2], 36], Some(&mask));
            }
        let logo = appicon::pixmap(128);
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

    /// The control on the right of a row.
    pub(super) fn paint_control(&self, pm: &mut Pixmap, panel: &Panel, font: &fontdue::Font, heading: &fontdue::Font, item: &Item, key: u64, tint: [u8; 3], s: f32) {
        let white = [255, 255, 255, 255];
        let grey = [165, 165, 175, 255];
        let (cx, cy) = panel.cursor;
        let accent = tint;

        match item.kind {
            Kind::Link(url) => {
                // The value on the right, and an arrow when the line opens something.
                let right = (item.row.0 + item.row.2 - 18.0) * s;
                let (tw, y) = (crate::text_width(font, item.btn, 12.0 * s), (item.row.1 + item.row.3 / 2.0 + 4.5) * s);
                if url.is_empty() {
                    crate::draw_text(pm, font, item.btn, right - tw, y, 12.0 * s, grey, 240.0 * s);
                } else {
                    let tip = [(right - 5.0 * s, y - 9.0 * s), (right, y - 5.0 * s), (right - 5.0 * s, y - 1.0 * s)];
                    stroke_points(pm, &tip, false, 1.5 * s, [165, 165, 175, 255]);
                    crate::draw_text(pm, font, item.btn, right - 16.0 * s - tw, y, 12.0 * s, [200, 200, 212, 255], 240.0 * s);
                }
            }
            Kind::Toggle(t) => {
                let on = panel.glows.follow(key + 1, if self.toggle_value(t) { 1.0 } else { 0.0 }, 34.0, 0.62) as f32;
                // The shared switch is 32 x 18; it is drawn 1.25 times as big here.
                crate::draw_switch(pm, item.ctrl.0 * s, item.ctrl.1 * s, s * 1.25, on, 255);
            }
            Kind::Seg(seg) => {
                let names = seg_names(seg);
                let cells = seg_cells(seg, item.ctrl);
                let (x, y, w, h) = (item.ctrl.0 * s, item.ctrl.1 * s, item.ctrl.2 * s, item.ctrl.3 * s);
                crate::fill_rounded(pm, x, y, w, h, h / 2.0, [255, 255, 255, 18]);
                // The highlight slides (and changes width) to the chosen option, the way the tab bar's circle
                // does: it stretches in the direction it is moving, squashes a little, and has a soft halo.
                let pos = panel.glows.follow(key + 2, self.seg_value(seg) as f64, 28.0, 0.7) as f32;
                let stretch = (panel.glows.velocity(key + 2).abs() as f32 * 7.0).min(30.0);
                let (i, f) = ((pos.floor().max(0.0) as usize).min(cells.len() - 1), pos - pos.floor().max(0.0));
                let (a, b) = (cells[i], cells[(i + 1).min(cells.len() - 1)]);
                let (px, pw) = (a.0 + (b.0 - a.0) * f, a.2 + (b.2 - a.2) * f);
                let (px, pw) = ((px - stretch / 2.0) * s + 2.0 * s, (pw + stretch) * s - 4.0 * s);
                let squash = stretch * 0.1 * s;
                let (hy, hh) = (y + 2.0 * s + squash / 2.0, h - 4.0 * s - squash);
                gfx::gradient_rounded(pm, px - 2.0 * s, hy - 2.0 * s, pw + 4.0 * s, hh + 4.0 * s, (hh + 4.0 * s) / 2.0, [accent[0], accent[1], accent[2], 26], [accent[0], accent[1], accent[2], 10], false);
                gfx::gradient_rounded(pm, px, hy, pw, hh, hh / 2.0, [accent[0], accent[1], accent[2], 140], [accent[0], accent[1], accent[2], 86], false);
                gfx::stroke_rounded(pm, px + 0.5 * s, hy + 0.5 * s, pw - s, hh - s, hh / 2.0, s, [accent[0], accent[1], accent[2], 160]);
                for (i, name) in names.iter().enumerate() {
                    let cell = cells[i];
                    let near = 1.0 - (pos - i as f32).abs().min(1.0);
                    let over = panel.glows.fade(key + 3 + i as u64, inside(cell, cx, cy));
                    let ink = crate::pick(white, [150, 150, 160, 255], near.max(over * 0.7));
                    crate::draw_text_centered(pm, font, name, (cell.0 + cell.2 / 2.0) * s, (cell.1 + 18.5) * s, 12.0 * s, ink, (cell.2 - 6.0) * s);
                }
            }
            Kind::EyeGrid => {
                // A tile for each style with a pair of eyes in it, drawn the way the closed notch draws them; the chosen
                // one is lit in the page's colour, and the tile under the mouse brightens.
                for (i, style) in EyeStyle::ALL.iter().enumerate() {
                    let tile = eye_tile(item.ctrl, i);
                    let (x, y, w, h) = (tile.0 * s, tile.1 * s, tile.2 * s, tile.3 * s);
                    let over = panel.glows.fade(key + 20 + i as u64, inside(tile, cx, cy));
                    let lit = panel.glows.follow(key + 40 + i as u64, if *style == self.settings.eye_style { 1.0 } else { 0.0 }, 30.0, 0.8) as f32;
                    crate::fill_rounded(pm, x, y, w, h, 10.0 * s, [255, 255, 255, (14.0 + 20.0 * over) as u8]);
                    if lit > 0.01 {
                        gfx::gradient_rounded(pm, x, y, w, h, 10.0 * s, [accent[0], accent[1], accent[2], (140.0 * lit) as u8], [accent[0], accent[1], accent[2], (86.0 * lit) as u8], false);
                        gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, 10.0 * s, s, [accent[0], accent[1], accent[2], (170.0 * lit) as u8]);
                    }
                    // The eyes are drawn on a picture of the tile's own size, so their glow stays inside the tile.
                    if let Some(mut face) = Pixmap::new(w.ceil() as u32, h.ceil() as u32) {
                        for dx in [-13.0, 13.0] {
                            crate::ui::eyes::draw_eye(&mut face, *style, w / 2.0 + dx * s, h / 2.0, s, 255, crate::ui::eyes::Pose::REST);
                        }
                        pm.draw_pixmap(x.round() as i32, y.round() as i32, face.as_ref(), &tiny_skia::PixmapPaint::default(), Transform::identity(), None);
                    }
                }
            }
            Kind::ScreenMap => {
                // The connected screens as they sit in Windows' layout, each with its number; the one the notch is on
                // is lit in the page's colour with a little notch drawn at its top, and the tile under the mouse
                // brightens. At the right, All lights every screen.
                let screens = self.detected_screens();
                let rects = tiles(item.ctrl, &screens);
                // Its own range of keys for the glows, so they never meet those of another row.
                let own = key + (1u64 << 40);
                let all = self.settings.screen == Screen::All;
                let lights_all = self.settings.screen.follows();
                // The screen the notch is on right now (with All, the one the mouse last touched the top of).
                let here = self.window.as_ref().and_then(|w| self.island_monitor(w)).map(|m| (m.position().x, m.position().y));
                for (i, (d, t)) in screens.iter().zip(&rects).enumerate() {
                    let (x, y, w, h) = (t.0 * s, t.1 * s, t.2 * s, t.3 * s);
                    let over = panel.glows.fade(own + i as u64, inside(*t, cx, cy));
                    let target = if is_chosen(d, self.settings.screen) { 1.0 } else if lights_all { 0.6 } else { 0.0 };
                    let lit = panel.glows.follow(own + 16 + i as u64, target, 30.0, 0.8) as f32;
                    crate::fill_rounded(pm, x, y, w, h, 6.0 * s, [255, 255, 255, (14.0 + 22.0 * over) as u8]);
                    if lit > 0.01 {
                        gfx::gradient_rounded(pm, x, y, w, h, 6.0 * s, [accent[0], accent[1], accent[2], (140.0 * lit) as u8], [accent[0], accent[1], accent[2], (86.0 * lit) as u8], false);
                        gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, 6.0 * s, s, [accent[0], accent[1], accent[2], (170.0 * lit) as u8]);
                    }
                    // The notch, small, at the top middle of the screen it is on now. (With All every screen is lit the
                    // same, so this is what says where it is at the moment.)
                    let on = panel.glows.follow(own + 32 + i as u64, if here == Some((d.x, d.y)) { 1.0 } else { 0.0 }, 30.0, 0.8) as f32;
                    let pill = (t.2 * 0.3).clamp(8.0, 22.0);
                    let pill_colour = if lit > 0.3 { [10, 10, 14, (230.0 * on) as u8] } else { [255, 255, 255, (170.0 * on) as u8] };
                    crate::fill_rounded(pm, x + (w - pill * s) / 2.0, y + 1.0 * s, pill * s, 3.5 * s, 1.75 * s, pill_colour);
                    let ink = crate::pick(grey, white, lit.max(over * 0.6));
                    let mid = (x + w / 2.0, y + h / 2.0);
                    if t.2 >= 34.0 && t.3 >= 22.0 {
                        let sub = t.3 >= 40.0 && t.2 >= 52.0;
                        crate::draw_text_centered(pm, font, &d.order.to_string(), mid.0, mid.1 + if sub { 0.0 } else { 5.0 * s }, 15.0 * s, ink, w - 4.0 * s);
                        if sub {
                            let text = if d.primary { "Main".to_string() } else { format!("{}\u{00D7}{}", d.w, d.h) };
                            crate::draw_text_centered(pm, font, &text, mid.0, mid.1 + 12.0 * s, 9.0 * s, [ink[0], ink[1], ink[2], 200], w - 4.0 * s);
                        }
                    } else {
                        crate::draw_text_centered(pm, font, &d.order.to_string(), mid.0, mid.1 + 4.0 * s, 10.0 * s, ink, w);
                    }
                }
                if screens.len() > 1 {
                    // All: the notch follows the mouse. Every: a closed notch on each screen, the touched one opens.
                    let every = self.settings.screen == Screen::Every;
                    for (n, (tile, name, note, on)) in [(all_tile(item.ctrl), "All", "follows mouse", all), (every_tile(item.ctrl), "Every", "all closed", every)].into_iter().enumerate() {
                        let (x, y, w, h) = (tile.0 * s, tile.1 * s, tile.2 * s, tile.3 * s);
                        let over = panel.glows.fade(own + 48 + 2 * n as u64, inside(tile, cx, cy));
                        let lit = panel.glows.follow(own + 49 + 2 * n as u64, if on { 1.0 } else { 0.0 }, 30.0, 0.8) as f32;
                        crate::fill_rounded(pm, x, y, w, h, 8.0 * s, [255, 255, 255, (14.0 + 22.0 * over) as u8]);
                        if lit > 0.01 {
                            gfx::gradient_rounded(pm, x, y, w, h, 8.0 * s, [accent[0], accent[1], accent[2], (140.0 * lit) as u8], [accent[0], accent[1], accent[2], (86.0 * lit) as u8], false);
                            gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, 8.0 * s, s, [accent[0], accent[1], accent[2], (170.0 * lit) as u8]);
                        }
                        let ink = crate::pick(grey, white, lit.max(over * 0.6));
                        crate::draw_text_centered(pm, font, name, x + w / 2.0, y + h / 2.0 + 0.0 * s, 12.5 * s, ink, w - 6.0 * s);
                        crate::draw_text_centered(pm, font, note, x + w / 2.0, y + h / 2.0 + 11.0 * s, 8.0 * s, [ink[0], ink[1], ink[2], 200], w - 4.0 * s);
                    }
                } else {
                    // One screen: nothing to choose between, and nothing to follow.
                    let tile = (item.ctrl.0 + item.ctrl.2 - 66.0, item.ctrl.1, 66.0, item.ctrl.3);
                    let (x, y, w, h) = (tile.0 * s, tile.1 * s, tile.2 * s, tile.3 * s);
                    crate::draw_text_centered(pm, font, "One screen", x + w / 2.0, y + h / 2.0 - 2.0 * s, 9.5 * s, [150, 150, 160, 255], w - 4.0 * s);
                    crate::draw_text_centered(pm, font, "connected", x + w / 2.0, y + h / 2.0 + 10.0 * s, 9.5 * s, [150, 150, 160, 255], w - 4.0 * s);
                }
            }
            Kind::Slider(slide) => {
                let range = range_of(slide);
                let v = self.slider_get(slide);
                let frac = (v - range.min) as f32 / (range.max - range.min) as f32;
                let (x, y, w, h) = (item.ctrl.0 * s, item.ctrl.1 * s, item.ctrl.2 * s, item.ctrl.3 * s);
                let grab = panel.glows.fade(key + 8, panel.drag == Some(slide) || inside((item.ctrl.0 - 8.0, item.row.1, SLIDER_W + 16.0, item.row.3), cx, cy));
                crate::fill_rounded(pm, x, y, w, h, h / 2.0, [255, 255, 255, 34]);
                let fill_w = (w * frac).max(h);
                gfx::gradient_rounded(pm, x, y, fill_w, h, h / 2.0, [accent[0], accent[1], accent[2], 210], [255, 255, 255, 210], true);
                let kx = x + w * frac;
                gfx::glow(pm, kx, y + h / 2.0, (12.0 + 4.0 * grab) * s, [accent[0], accent[1], accent[2], (60.0 + 60.0 * grab) as u8], None);
                // White in both looks (the light look turns white into dark, so ask for black there).
                let knob = if theme::is_light() { [0, 0, 0, 255] } else { white };
                crate::fill_circle(pm, kx, y + h / 2.0, (6.5 + 1.5 * grab) * s, knob);
                let text = slider_text(slide, v);
                let tw = crate::text_width(font, &text, 12.0 * s);
                crate::draw_text(pm, font, &text, (item.row.0 + item.row.2 - 18.0) * s - tw, (item.row.1 + item.row.3 / 2.0 + 4.5) * s, 12.0 * s, grey, SLIDER_TEXT_W * s);
            }
            Kind::Pick => {
                let (x, y, w, h) = (item.ctrl.0 * s, item.ctrl.1 * s, item.ctrl.2 * s, item.ctrl.3 * s);
                let over = panel.glows.fade(key + 9, inside(item.ctrl, cx, cy));
                gfx::gradient_rounded(pm, x, y, w, h, h / 2.0, [accent[0], accent[1], accent[2], crate::pick(110.0, 64.0, over) as u8], [accent[0], accent[1], accent[2], crate::pick(66.0, 32.0, over) as u8], false);
                gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [accent[0], accent[1], accent[2], 130]);
                let custom = self.settings.alarm_sound.as_ref();
                let name = custom.and_then(|p| p.file_name()).map_or_else(|| "Built-in chime".to_string(), |n| n.to_string_lossy().into_owned());
                let text_w = if custom.is_some() { item.ctrl.2 - 44.0 } else { item.ctrl.2 - 20.0 };
                crate::draw_text_centered(pm, heading, &name, (item.ctrl.0 + 10.0 + text_w / 2.0) * s, (item.ctrl.1 + 18.5) * s, 12.0 * s, white, text_w * s);
                if custom.is_some() {
                    let (bx, by) = ((item.ctrl.0 + item.ctrl.2 - 16.0) * s, (item.ctrl.1 + item.ctrl.3 / 2.0) * s);
                    let hot_x = panel.glows.fade(key + 10, cx >= item.ctrl.0 + item.ctrl.2 - 30.0 && inside(item.ctrl, cx, cy));
                    crate::fill_circle(pm, bx, by, 9.0 * s, [255, 96, 86, (70.0 * hot_x) as u8]);
                    crate::draw_text_centered(pm, font, "\u{00D7}", bx, by + 5.0 * s, 15.0 * s, crate::pick(white, grey, hot_x), 14.0 * s);
                }
            }
            Kind::Cycle(stepper) => {
                let (x, y, w, h) = (item.ctrl.0 * s, item.ctrl.1 * s, item.ctrl.2 * s, item.ctrl.3 * s);
                // The zone is dimmed and does not answer while the method is not e-Solat.
                let live = stepper != Stepper::Zone || self.zone_applies();
                let strength = if live { 1.0 } else { 0.45 };
                let over = panel.glows.fade(key + 9, live && inside(item.ctrl, cx, cy));
                gfx::gradient_rounded(pm, x, y, w, h, h / 2.0, [accent[0], accent[1], accent[2], (crate::pick(110.0, 64.0, over) * strength) as u8], [accent[0], accent[1], accent[2], (crate::pick(66.0, 32.0, over) * strength) as u8], false);
                gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [accent[0], accent[1], accent[2], (130.0 * strength) as u8]);
                let name = match stepper {
                    Stepper::Method => crate::prayer::METHODS[crate::prayer::method_index(self.settings.prayer_method)].1.to_string(),
                    Stepper::Zone => self.zone_text(),
                };
                crate::draw_text_centered(pm, heading, &name, (item.ctrl.0 + item.ctrl.2 / 2.0) * s, (item.ctrl.1 + 18.5) * s, 12.0 * s, if live { white } else { grey }, (item.ctrl.2 - 56.0) * s);
                // The arrows at the two ends.
                let hot_left = over > 0.01 && cx < item.ctrl.0 + item.ctrl.2 / 3.0;
                for (bx, glyph) in [(item.ctrl.0 + 16.0, "\u{2039}"), (item.ctrl.0 + item.ctrl.2 - 16.0, "\u{203A}")] {
                    let ink = if hot_left == (glyph == "\u{2039}") && over > 0.01 { white } else { grey };
                    crate::draw_text_centered(pm, heading, glyph, bx * s, (item.ctrl.1 + 19.0) * s, 17.0 * s, ink, 16.0 * s);
                }
            }
            Kind::Text(Field::Hotkey) => {
                let (x, y, w, h) = (item.ctrl.0 * s, item.ctrl.1 * s, item.ctrl.2 * s, item.ctrl.3 * s);
                let over = panel.glows.fade(key + 9, inside(item.ctrl, cx, cy));
                let centre = (item.ctrl.0 + item.ctrl.2 / 2.0) * s;
                if panel.capture {
                    crate::fill_rounded(pm, x, y, w, h, h / 2.0, [0, 0, 0, 90]);
                    gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [accent[0], accent[1], accent[2], 210]);
                    crate::draw_text_centered(pm, font, "Press the keys...", centre, (item.ctrl.1 + 18.5) * s, 12.0 * s, grey, item.ctrl.2 * s);
                } else {
                    let strength = if self.settings.hotkey { 1.0 } else { 0.45 };
                    gfx::gradient_rounded(pm, x, y, w, h, h / 2.0, [accent[0], accent[1], accent[2], (crate::pick(110.0, 64.0, over) * strength) as u8], [accent[0], accent[1], accent[2], (crate::pick(66.0, 32.0, over) * strength) as u8], false);
                    gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [accent[0], accent[1], accent[2], (130.0 * strength) as u8]);
                    let name = crate::hotkey::name(self.settings.hotkey_mods, self.settings.hotkey_vk);
                    crate::draw_text_centered(pm, heading, &name, centre, (item.ctrl.1 + 18.5) * s, 12.0 * s, if self.settings.hotkey { white } else { grey }, (item.ctrl.2 - 20.0) * s);
                }
            }
            Kind::Text(Field::City) => {
                let (x, y, w, h) = (item.ctrl.0 * s, item.ctrl.1 * s, item.ctrl.2 * s, item.ctrl.3 * s);
                let over = panel.glows.fade(key + 9, inside(item.ctrl, cx, cy));
                let inner = (item.ctrl.2 - 24.0) * s;
                if let Some(typed) = &panel.city.typing {
                    // The box while typing: dark, ringed in the page's colour, with the text and a caret.
                    crate::fill_rounded(pm, x, y, w, h, h / 2.0, [0, 0, 0, 90]);
                    gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [accent[0], accent[1], accent[2], 210]);
                    let mut shown = typed.as_str();
                    while crate::text_width(font, shown, 12.5 * s) > inner - 4.0 * s && !shown.is_empty() {
                        shown = &shown[shown.chars().next().map_or(1, char::len_utf8)..];
                    }
                    crate::draw_text(pm, font, shown, x + 12.0 * s, (item.ctrl.1 + 18.5) * s, 12.5 * s, white, inner);
                    let caret_x = x + 12.0 * s + crate::text_width(font, shown, 12.5 * s) + 1.5 * s;
                    crate::fill_rect(pm, caret_x, y + 7.0 * s, 1.4 * s, h - 14.0 * s, white);
                } else {
                    gfx::gradient_rounded(pm, x, y, w, h, h / 2.0, [accent[0], accent[1], accent[2], crate::pick(110.0, 64.0, over) as u8], [accent[0], accent[1], accent[2], crate::pick(66.0, 32.0, over) as u8], false);
                    gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [accent[0], accent[1], accent[2], 130]);
                    let (name, ink) = match &self.settings.place {
                        Some(p) => (p.name.as_str(), white),
                        None => ("Not set", grey),
                    };
                    crate::draw_text_centered(pm, heading, name, (item.ctrl.0 + item.ctrl.2 / 2.0) * s, (item.ctrl.1 + 18.5) * s, 12.0 * s, ink, inner);
                }
            }
            Kind::Button(act) => {
                let armed = act == Act::Reset && panel.reset_armed.is_some_and(|t| Instant::now() < t);
                let over = panel.glows.fade(key + 9, inside(item.ctrl, cx, cy));
                let (x, y, w, h) = (item.ctrl.0 * s, item.ctrl.1 * s, item.ctrl.2 * s, item.ctrl.3 * s);
                let colour = if armed || act == Act::Reset { [255, 110, 96] } else { accent };
                gfx::gradient_rounded(pm, x, y, w, h, h / 2.0, [colour[0], colour[1], colour[2], crate::pick(110.0, 62.0, over) as u8], [colour[0], colour[1], colour[2], crate::pick(66.0, 30.0, over) as u8], false);
                gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [colour[0], colour[1], colour[2], 130]);
                let download = match &self.update {
                    _ if act != Act::CheckUpdate => None,
                    crate::update::Status::Available { version, .. } => Some(format!("Download {version}")),
                    crate::update::Status::Downloading { .. } => Some("Downloading...".to_string()),
                    crate::update::Status::Ready { version, .. } => Some(format!("Install {version}")),
                    _ => None,
                };
                let label = if armed { "Click again" } else { download.as_deref().unwrap_or(item.btn) };
                crate::draw_text_centered(pm, heading, label, (item.ctrl.0 + item.ctrl.2 / 2.0) * s, (item.ctrl.1 + 18.5) * s, 12.0 * s, white, (item.ctrl.2 - 14.0) * s);
            }
        }
    }
}
