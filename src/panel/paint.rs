//! Drawing the settings window: the background, the sidebar and a page with its title.

use super::*;

/// Whether the cached background was drawn for this size, page colour and look. Anything that changes how it is
/// drawn must be part of this, or the window keeps showing the old picture (the look was missing once, and the
/// Light theme left a dark background under dark text).
pub(super) fn backdrop_is_for(cached: &Option<(u32, u32, [u8; 3], bool, Pixmap)>, size: (u32, u32), tint: [u8; 3], light: bool) -> bool {
    cached.as_ref().is_some_and(|(w, h, t, l, _)| ((*w, *h), *t, *l) == (size, tint, light))
}

impl App {
    /// What the last update check found, with the colour to show it in.
    pub(super) fn update_note(&self) -> (String, [u8; 4]) {
        let (grey, green, amber, red) = ([165, 165, 175, 255], [90, 210, 130, 255], [255, 190, 70, 255], [255, 130, 120, 255]);
        match &self.updates.status {
            crate::update::Status::Idle => (format!("You have version {}", env!("CARGO_PKG_VERSION")), grey),
            crate::update::Status::Checking => ("Checking...".to_string(), grey),
            crate::update::Status::Latest => ("You have the latest version".to_string(), green),
            crate::update::Status::Available { version, .. } => (format!("Version {version} is available"), amber),
            crate::update::Status::Downloading { percent, .. } => (format!("Downloading... {percent}%"), grey),
            crate::update::Status::Ready { .. } => ("Downloaded. Ready to install".to_string(), green),
            crate::update::Status::Failed => ("Could not check. Are you online?".to_string(), red),
            crate::update::Status::BadName => ("The newest release has a wrong name, so it is not offered".to_string(), red),
            crate::update::Status::DownloadFailed => ("Download failed. Try again".to_string(), red),
        }
    }

    /// The line under a row's label as it is now, with its colour: the page's own line, or what the row has to
    /// say at the moment (the update check, the city, the hotkey, a click waiting for a second one).
    pub(super) fn row_text(&self, panel: &Panel, kind: Kind, desc: &str) -> (String, [u8; 4]) {
        match kind {
            Kind::Button(Act::CheckUpdate) => self.update_note(),
            Kind::Button(Act::Reset) if panel.reset_armed.is_some_and(|t| Instant::now() < t) => {
                ("Click Reset again within 3 seconds to confirm".to_string(), [255, 150, 140, 255])
            }
            Kind::Button(Act::ClearLyrics) if panel.cleared.is_some_and(|(_, t)| Instant::now() < t) => {
                let n = panel.cleared.map_or(0, |(n, _)| n);
                (format!("Removed {n} saved {}", if n == 1 { "song" } else { "songs" }), [120, 220, 150, 255])
            }
            Kind::Text(Field::City) => panel.city.note(),
            Kind::Text(Field::Hotkey) if panel.capture => ("Press Ctrl, Alt or Win and a key. Esc cancels".to_string(), [146, 146, 160, 255]),
            Kind::Text(Field::Hotkey) if self.settings.hotkey && !self.hotkey_ok => ("Another program already uses these keys. Choose others".to_string(), [255, 150, 140, 255]),
            _ => (desc.to_string(), [146, 146, 160, 255]),
        }
    }

    /// A page laid out for `frame`, with every row as tall as its text needs: a long line wraps onto more lines
    /// instead of being cut off.
    pub(super) fn layout_of(&self, panel: &Panel, page: Page, frame: Frame) -> Layout {
        let lines = |kind: Kind, desc: &'static str, room: f32| match &self.font {
            Some(font) => {
                let text = self.row_text(panel, kind, desc).0;
                if text.is_empty() { 1 } else { crate::wrap(font, &text, DESC_PX, room).len() }
            }
            None => 1,
        };
        layout_with(page, frame, &lines)
    }

    pub(super) fn paint_panel(&self, panel: &mut Panel) {
        theme::set_light(self.light());
        let now = Instant::now();
        let speed = self.settings.motion.speed() as f32;
        panel.glows.set_motion(speed as f64, self.settings.motion.bounce());
        panel.glows.begin(now.duration_since(panel.last_draw).as_secs_f64().min(1.0 / 30.0));
        panel.last_draw = now;

        let size = panel.window.inner_size();
        let (Some(w), Some(h)) = (std::num::NonZeroU32::new(size.width), std::num::NonZeroU32::new(size.height)) else {
            return;
        };
        let s = panel.scale();
        let (fw, fh) = (w.get() as f32, h.get() as f32);
        let (lh, frame) = (fh / s, frame_for(fw / s));
        let tint = panel.page.tint();

        // The background is drawn once for a size, page colour and look and copied in each frame; the picture
        // it is copied onto is the same one every time, so nothing big is allocated per frame.
        let light = theme::is_light();
        if !backdrop_is_for(&panel.backdrop, (w.get(), h.get()), tint, light) {
            let Some(mut back) = Pixmap::new(w.get(), h.get()) else { return };
            // A dark gradient with a soft glow of the page's colour in the top right corner.
            gfx::gradient_rounded(&mut back, 0.0, 0.0, fw, fh, 0.0, [13, 13, 18, 255], [24, 24, 32, 255], false);
            gfx::glow(&mut back, fw * 0.92, fh * 0.04, fw * 0.55, [tint[0], tint[1], tint[2], 20], None);
            panel.backdrop = Some((w.get(), h.get(), tint, light, back));
        }
        let mut pm = match panel.canvas.take() {
            Some(canvas) if (canvas.width(), canvas.height()) == (w.get(), h.get()) => canvas,
            _ => match Pixmap::new(w.get(), h.get()) {
                Some(canvas) => canvas,
                None => return,
            },
        };
        if let Some((_, _, _, _, back)) = &panel.backdrop {
            pm.data_mut().copy_from_slice(back.data());
        }

        // While the window is opening or a page is fading in, what fades is drawn on a layer that is moved into
        // place. The rest of the time everything is drawn straight onto the picture, which is much cheaper.
        let page_x = (panel.page_since.elapsed().as_secs_f32() * speed / PAGE_FADE.as_secs_f32()).clamp(0.0, 1.0);
        let page_ease = 1.0 - (1.0 - page_x).powi(3);
        let open_x = (panel.opened.elapsed().as_secs_f32() * speed / ENTER.as_secs_f32()).clamp(0.0, 1.0);
        let open_ease = 1.0 - (1.0 - open_x).powi(3);
        let opening = open_ease < 1.0;
        if let Some(font) = &self.font {
            let heading = self.heading_font().unwrap_or(font);
            let blank = || Pixmap::new(w.get(), h.get()).unwrap_or_else(|| Pixmap::new(1, 1).unwrap());
            if opening {
                // Opening: the sidebar and the page are drawn on one layer, which rises into place and fades in.
                let (mut layer, mut content) = (blank(), blank());
                self.paint_sidebar(&mut layer, panel, font, heading, s, fh);
                self.paint_content(&mut content, panel, font, heading, tint, s, frame, lh);
                let paint = tiny_skia::PixmapPaint { opacity: page_ease, ..Default::default() };
                layer.draw_pixmap(0, 0, content.as_ref(), &paint, Transform::from_translate((1.0 - page_ease) * 14.0 * s, 0.0), None);
                let paint = tiny_skia::PixmapPaint { opacity: open_ease, ..Default::default() };
                pm.draw_pixmap(0, 0, layer.as_ref(), &paint, Transform::from_translate(0.0, (1.0 - open_ease) * 16.0 * s), None);
            } else if page_ease < 1.0 {
                // A page fading in: only the page is on a layer; the sidebar does not move.
                self.paint_sidebar(&mut pm, panel, font, heading, s, fh);
                let mut content = blank();
                self.paint_content(&mut content, panel, font, heading, tint, s, frame, lh);
                let paint = tiny_skia::PixmapPaint { opacity: page_ease, ..Default::default() };
                pm.draw_pixmap(0, 0, content.as_ref(), &paint, Transform::from_translate((1.0 - page_ease) * 14.0 * s, 0.0), None);
            } else {
                self.paint_sidebar(&mut pm, panel, font, heading, s, fh);
                self.paint_content(&mut pm, panel, font, heading, tint, s, frame, lh);
            }
        }

        // How far the changelog can scroll depends on its length, which is known once it has been laid out.
        if panel.show_changelog
            && let Some(font) = &self.font
        {
            let (_, total) = self.changelog_lines(font, self.heading_font().unwrap_or(font), frame.w - 44.0);
            panel.changelog_max = (total - (lh - CHANGELOG_TOP - 28.0)).max(0.0);
            panel.changelog_scroll = panel.changelog_scroll.clamp(0.0, panel.changelog_max);
        }

        if panel.surface.resize(w, h).is_ok()
            && let Ok(mut buffer) = panel.surface.buffer_mut()
        {
            for (dst, src) in buffer.iter_mut().zip(pm.pixels()) {
                *dst = (src.alpha() as u32) << 24 | (src.red() as u32) << 16 | (src.green() as u32) << 8 | src.blue() as u32;
            }
            buffer.present().ok();
        }
        panel.canvas = Some(pm);
        panel.busy = panel.glows.busy() || panel.opened.elapsed().mul_f32(speed) < ENTER || panel.page_since.elapsed().mul_f32(speed) < PAGE_FADE;
    }

    /// The part of the window beside the sidebar: the changelog, or a page with its title and its rows,
    /// which scroll under the title. `target` is a layer or the picture itself.
    pub(super) fn paint_content(&self, target: &mut Pixmap, panel: &mut Panel, font: &fontdue::Font, heading: &fontdue::Font, tint: [u8; 3], s: f32, frame: Frame, lh: f32) {
        let (w, h) = (target.width(), target.height());
        let (fw, fh) = (w as f32, h as f32);
        let page = panel.page;
        if panel.show_changelog {
            self.paint_changelog(target, panel, font, heading, tint, s, frame, lh);
            return;
        }
        // The page's own icon, big, beside its title, with a soft glow of its colour behind it.
        gfx::glow(target, (frame.x + 22.0) * s, 50.0 * s, 46.0 * s, [tint[0], tint[1], tint[2], 34], None);
        draw_page_icon(target, page, frame.x, 28.0, 44.0, s);
        let text_x = frame.x + 60.0;
        crate::draw_text(target, heading, page.title(), text_x * s, 52.0 * s, 24.0 * s, [255, 255, 255, 255], (frame.w - 60.0) * s);
        crate::draw_text(target, font, page.subtitle(), text_x * s, 72.0 * s, 11.5 * s, [150, 150, 164, 255], (frame.w - 60.0) * s);
        let lay = self.layout_of(panel, page, frame);
        panel.scroll_max = scroll_limit(&lay, lh);
        panel.scroll = panel.scroll.clamp(0.0, panel.scroll_max);
        // Dragging the scroll bar moves the page with the mouse, without the gliding of the wheel and the keys.
        let omega = if panel.bar_drag.is_some() { 400.0 } else { 40.0 };
        panel.scroll_shown = panel.glows.follow(KEY_SCROLL, panel.scroll as f64, omega, 1.0) as f32;
        let lay = lay.shifted(-panel.scroll_shown);
        if panel.scroll_shown < 0.5 {
            // Not scrolled: nothing reaches under the title, so the rows are drawn straight on.
            self.paint_page(target, panel, font, heading, &lay, tint, s);
        } else {
            // Scrolled: the rows go on a layer of their own that is cut off at the line under the title.
            let mut rows = Pixmap::new(w, h).unwrap_or_else(|| Pixmap::new(1, 1).unwrap());
            self.paint_page(&mut rows, panel, font, heading, &lay, tint, s);
            let below_title = tiny_skia::Rect::from_xywh(0.0, SCROLL_TOP * s, fw, (fh - SCROLL_TOP * s).max(1.0)).map(PathBuilder::from_rect);
            let mask = below_title.and_then(|path| gfx::mask_of(w, h, &path));
            target.draw_pixmap(0, 0, rows.as_ref(), &tiny_skia::PixmapPaint::default(), Transform::identity(), mask.as_ref());
        }
        if panel.scroll_max > 0.0 {
            let accent = [tint[0], tint[1], tint[2], 255];
            let bar = page_bar(fw / s, lh);
            let hot = panel.bar_drag.is_some() || bar.grabs(panel.cursor.0, panel.cursor.1);
            paint_scrollbar(target, bar, panel.scroll_shown, panel.scroll_max, accent, s, hot);
        }
    }

    /// The sidebar: the app's name, the pages with a highlight that glides between them, and Quit.
    pub(super) fn paint_sidebar(&self, pm: &mut Pixmap, panel: &Panel, font: &fontdue::Font, heading: &fontdue::Font, s: f32, fh: f32) {
        let white = [255, 255, 255, 255];
        let (cx, cy) = panel.cursor;
        // The sidebar is a slightly darker panel with a line on its right edge.
        crate::fill_rect(pm, 0.0, 0.0, SIDEBAR_W * s, fh, [0, 0, 0, 70]);
        crate::fill_rect(pm, (SIDEBAR_W - 1.0) * s, 0.0, s, fh, [255, 255, 255, 14]);

        // The name at the top.
        let logo = appicon::pixmap(64);
        let scale = 26.0 * s / logo.width() as f32;
        let paint = tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
        pm.draw_pixmap(0, 0, logo.as_ref(), &paint, Transform::from_scale(scale, scale).post_translate(18.0 * s, 16.0 * s), None);
        crate::draw_text(pm, heading, "Sky Island", 52.0 * s, 35.0 * s, 15.0 * s, white, 150.0 * s);

        // The highlight glides to the chosen page, tinted with that page's colour.
        let pos = panel.glows.follow(KEY_SIDE, panel.page.index() as f64, 30.0, 0.8) as f32;
        let tint = panel.page.tint();
        // Between two pages the highlight is part way between their rows (which a heading can push apart).
        let (lo, hi) = (pos.floor().clamp(0.0, (Page::ALL.len() - 1) as f32), pos.ceil().clamp(0.0, (Page::ALL.len() - 1) as f32));
        let (a, b) = (side_rect(Page::ALL[lo as usize]), side_rect(Page::ALL[hi as usize]));
        let hy = a.1 + (b.1 - a.1) * (pos - lo);
        let (hx, hw, hh) = (12.0, SIDEBAR_W - 24.0, SIDE_ITEM_H - 4.0);
        gfx::gradient_rounded(pm, hx * s, hy * s, hw * s, hh * s, 10.0 * s, [255, 255, 255, 30], [255, 255, 255, 16], false);
        gfx::stroke_rounded(pm, hx * s + 0.5 * s, hy * s + 0.5 * s, hw * s - s, hh * s - s, 10.0 * s, s, [tint[0], tint[1], tint[2], 90]);

        for (i, page) in Page::ALL.into_iter().enumerate() {
            let r = side_rect(page);
            let hot = panel.glows.fade(KEY_SIDE + 1 + i as u64, inside(r, cx, cy));
            if hot > 0.01 && page != panel.page {
                crate::fill_rounded(pm, r.0 * s, r.1 * s, r.2 * s, r.3 * s, 10.0 * s, [255, 255, 255, (12.0 * hot) as u8]);
            }
            let near = 1.0 - (pos - i as f32).abs().min(1.0);
            // A section of pages starts with its heading.
            if let Some(heading) = page.section() {
                crate::draw_text(pm, font, heading, (r.0 + 10.0) * s, (r.1 - 9.0) * s, 10.0 * s, [118, 118, 134, 255], (r.2 - 20.0) * s);
            }
            draw_page_icon(pm, page, r.0 + 6.0, r.1 + (r.3 - 24.0) / 2.0, 24.0, s);
            let ink = crate::pick(white, [200, 200, 212, 255], near.max(hot * 0.6));
            crate::draw_text(pm, font, page.title(), (r.0 + 40.0) * s, (r.1 + r.3 / 2.0 + 4.5) * s, 13.0 * s, ink, (r.2 - 48.0) * s);
            // A dot on About while a new version is waiting.
            if page == Page::About
                && matches!(self.updates.status, crate::update::Status::Available { .. } | crate::update::Status::Downloading { .. } | crate::update::Status::Ready { .. })
            {
                crate::fill_circle(pm, (r.0 + r.2 - 16.0) * s, (r.1 + r.3 / 2.0) * s, 4.0 * s, [255, 190, 60, 255]);
            }
        }

        // The version, at the bottom. (Quitting is in the tray menu.)
        let version = format!("Version {}", env!("CARGO_PKG_VERSION"));
        crate::draw_text_centered(pm, font, &version, SIDEBAR_W / 2.0 * s, fh - VERSION_FROM_BOTTOM * s, 10.5 * s, [130, 130, 144, 255], (SIDEBAR_W - 24.0) * s);
    }

    /// The groups of a page as cards with their rows.
    pub(super) fn paint_page(&self, pm: &mut Pixmap, panel: &Panel, font: &fontdue::Font, heading: &fontdue::Font, lay: &Layout, tint: [u8; 3], s: f32) {
        let white = [255, 255, 255, 255];
        let grey = [160, 160, 172, 255];

        if let Some(hero) = lay.hero {
            if panel.page == Page::Calendar {
                crate::draw_text(pm, font, "Preview", (hero.0 + 4.0) * s, (hero.1 - 8.0) * s, 11.5 * s, grey, 300.0 * s);
                self.paint_calendar_preview(pm, font, heading, hero, tint, s);
            } else {
                self.paint_hero(pm, font, heading, hero, tint, s);
            }
        }
        for g in &lay.groups {
            let ink = gfx::mix(grey, [tint[0], tint[1], tint[2], 255], 0.45);
            crate::draw_text(pm, heading, g.title, (g.rect.0 + 4.0) * s, (g.rect.1 - 8.0) * s, 12.0 * s, ink, 300.0 * s);
            crate::draw_card(pm, g.rect, 14.0, s, 1.0, 0.0, tint);
        }
        for (idx, item) in lay.items.iter().enumerate() {
            let key = 0x2000_0000 + idx as u64 * 16;
            let (cx, cy) = panel.cursor;
            let row_hot = !matches!(item.kind, Kind::Link("")) && inside(item.row, cx, cy);
            let hot = panel.glows.fade(key, row_hot);
            if hot > 0.01 {
                crate::fill_rounded(
                    pm,
                    (item.row.0 + 4.0) * s,
                    (item.row.1 + 3.0) * s,
                    (item.row.2 - 8.0) * s,
                    (item.row.3 - 6.0) * s,
                    10.0 * s,
                    [255, 255, 255, (11.0 * hot) as u8],
                );
            }
            if item.after_row {
                crate::fill_rect(pm, (item.row.0 + 18.0) * s, item.row.1 * s, (item.row.2 - 36.0) * s, s * 0.7, [255, 255, 255, 13]);
            }
            // The label and the line under it. The About page's update row shows what the check found.
            let (desc, desc_ink) = self.row_text(panel, item.kind, item.desc);
            let text_w = if under_label(item.kind) { item.row.2 - 36.0 } else { item.ctrl.0 - item.row.0 - 36.0 };
            // A row with no line under its label has the label in the middle.
            let label_y = if desc.is_empty() { item.row.3 / 2.0 + 5.0 } else { 24.0 };
            crate::draw_text(pm, font, item.label, (item.row.0 + 18.0) * s, (item.row.1 + label_y) * s, 13.5 * s, crate::pick(white, [232, 232, 240, 255], hot), text_w * s);
            // A long line goes on under itself instead of being cut off.
            if !desc.is_empty() {
                for (n, line) in crate::wrap(font, &desc, DESC_PX, text_w).iter().enumerate() {
                    crate::draw_text(pm, font, line, (item.row.0 + 18.0) * s, (item.row.1 + 42.0 + n as f32 * DESC_LINE_H) * s, DESC_PX * s, desc_ink, text_w * s);
                }
            }
            self.paint_control(pm, panel, font, heading, item, key, tint, s);
        }
    }
}
