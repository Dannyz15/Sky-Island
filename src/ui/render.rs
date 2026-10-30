//! Putting the island on the screen: the draw pass, the layers views are drawn on, and the shared hover and fade helpers.

use crate::*;

impl App {
    /// The colour the current view is tinted with.
    pub(crate) fn accent(&self, view: View) -> [u8; 3] {
        match view {
            View::Player | View::Lyrics => self.art_shown.map(|c| c.round().clamp(0.0, 255.0) as u8),
            View::Timer => [255, 99, 71],
            View::Water => [64, 170, 255],
            View::Shelf => [125, 140, 255],
            View::Todo => TODO_AMBER,
            View::Calendar => [200, 120, 255],
            View::Clipboard => CLIP_TEAL,
        }
    }

    /// The title font.
    pub(crate) fn heading_font(&self) -> Option<&Font> {
        self.bold.as_ref().or(self.font.as_ref())
    }

    /// Whether the mouse is over a rectangle (logical pixels) of the open notch.
    pub(crate) fn hot(&self, r: (f32, f32, f32, f32), s: f32) -> bool {
        if self.progress < 0.9 || !self.hovered {
            return false;
        }
        let (x, y) = (self.cursor.0 as f32 / s, self.cursor.1 as f32 / s + TAB_SHIFT);
        x >= r.0 && x <= r.0 + r.2 && y >= r.1 && y <= r.1 + r.3
    }

    /// How strongly the mouse is over a rectangle, 0 to 1, fading in and out instead of switching.
    pub(crate) fn hot_t(&self, r: (f32, f32, f32, f32), s: f32) -> f32 {
        self.glows.fade(motion::rect_key(r), self.hot(r, s))
    }

    /// A toggle's position, 0 (off) to 1 (on), springing across and settling with a small bounce.
    pub(crate) fn toggle_t(&self, id: u64, on: bool) -> f32 {
        self.glows.follow(0x8000_0000_0000_0000 | id, if on { 1.0 } else { 0.0 }, 34.0, 0.62) as f32
    }

    /// What the open list looks like, to notice when it changed: the view, how far it is scrolled, how
    /// many entries it has, and (for the calendar) the chosen day.
    pub(crate) fn list_signature(&self) -> (View, usize, usize, i64) {
        match self.view {
            View::Todo => (View::Todo, self.todo.scroll, self.todo.tasks.len(), 0),
            View::Shelf => (View::Shelf, self.shelf.scroll, self.shelf.items.len(), 0),
            View::Clipboard => (View::Clipboard, self.clips.scroll, self.clips.items.len(), 0),
            View::Calendar => (View::Calendar, 0, self.todo.on_day(self.cal.selected).len(), self.cal.selected),
            v => (v, 0, 0, 0),
        }
    }

    /// How far row `row` of `rows` has come in, 0 to 1: each row a little after the one above it.
    pub(crate) fn row_in(&self, row: usize, rows: usize) -> f32 {
        const STAGGER: f32 = 0.07;
        let span = 1.0 - STAGGER * (rows.max(1) - 1) as f32;
        let t = ((self.list_t as f32 - row as f32 * STAGGER) / span).clamp(0.0, 1.0);
        1.0 - (1.0 - t).powi(3)
    }

    /// The calendar's month (or week): after the arrows were used it slides in from the side of the arrow.
    pub(crate) fn slide_in(&self, pm: &mut Pixmap, s: f32, draw: impl Fn(&mut Pixmap)) {
        let t = self.cal_t as f32;
        if t >= 0.999 {
            draw(pm);
            return;
        }
        let Some(mut layer) = Pixmap::new(pm.width(), pm.height()) else { return draw(pm) };
        draw(&mut layer);
        let paint = PixmapPaint { opacity: t.clamp(0.0, 1.0), quality: FilterQuality::Bilinear, ..Default::default() };
        pm.draw_pixmap(0, 0, layer.as_ref(), &paint, Transform::from_translate((1.0 - t) * 30.0 * self.cal_dir * s, 0.0), None);
    }

    /// The cover and the name of the track: after a new track began they fade in and slide a little.
    pub(crate) fn fade_in_track(&self, pm: &mut Pixmap, s: f32, draw: impl Fn(&mut Pixmap)) {
        let t = self.track_t as f32;
        if t >= 0.999 {
            draw(pm);
            return;
        }
        let Some(mut layer) = Pixmap::new(pm.width(), pm.height()) else { return draw(pm) };
        draw(&mut layer);
        let paint = PixmapPaint { opacity: t.clamp(0.0, 1.0), quality: FilterQuality::Bilinear, ..Default::default() };
        pm.draw_pixmap(0, 0, layer.as_ref(), &paint, Transform::from_translate((1.0 - t) * 12.0 * s, 0.0), None);
    }

    /// Draw one row of a list. While the list is changing the row is drawn on a layer of its own, which
    /// fades in and slides a little from the side the list moved from.
    pub(crate) fn animated_row(&self, pm: &mut Pixmap, s: f32, row: usize, rows: usize, draw: impl Fn(&mut Pixmap)) {
        let e = self.row_in(row, rows);
        if e >= 0.999 {
            draw(pm);
            return;
        }
        let Some(mut layer) = Pixmap::new(pm.width(), pm.height()) else { return draw(pm) };
        draw(&mut layer);
        let paint = PixmapPaint { opacity: e, quality: FilterQuality::Bilinear, ..Default::default() };
        pm.draw_pixmap(0, 0, layer.as_ref(), &paint, Transform::from_translate(0.0, (1.0 - e) * 14.0 * self.list_dir * s), None);
    }

    /// Whether the light look is on: the setting is on and Windows is using its light theme.
    pub(crate) fn light(&self) -> bool {
        match self.settings.theme {
            ThemeMode::Dark => false,
            ThemeMode::Light => true,
            ThemeMode::System => self.system_light,
        }
    }

    pub(crate) fn draw(&mut self) {
        theme::set_light(self.light());
        let frame_start = Instant::now();
        self.glows.set_motion(self.settings.motion.speed(), self.settings.motion.bounce());
        self.glows.begin(frame_start.duration_since(self.last_draw).as_secs_f64().min(1.0 / 30.0));
        self.last_draw = frame_start;
        if self.view == View::Shelf {
            self.load_icons();
        }
        let Some(window) = self.window.clone() else { return };
        let size = window.inner_size();
        let (Some(full_w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return;
        };
        // The notch itself is drawn in the middle; the flares on its sides are added at the end.
        let s = self.ui_scale() as f32;
        let ear = (EAR as f32 * s).round();
        let Some(w) = NonZeroU32::new(full_w.get().saturating_sub(2 * ear as u32)) else { return };
        let Some(mut pixmap) = Pixmap::new(w.get(), h.get()) else { return };
        pixmap.fill(Color::TRANSPARENT);

        // The notch: square top edge, rounded bottom corners, a dark gradient and a hairline border.
        let (fw, fh) = (w.get() as f32, h.get() as f32);
        let r = (fh / 2.0).min(28.0 * s);
        let mut pb = PathBuilder::new();
        pb.move_to(0.0, 0.0);
        pb.line_to(fw, 0.0);
        pb.line_to(fw, fh - r);
        pb.quad_to(fw, fh, fw - r, fh);
        pb.line_to(r, fh);
        pb.quad_to(0.0, fh, 0.0, fh - r);
        pb.close();
        let open = self.progress.clamp(0.0, 1.0) as f32;
        if let Some(path) = pb.finish() {
            let body = theme::body();
            let stops = vec![
                tiny_skia::GradientStop::new(0.0, Color::from_rgba8(body[0][0], body[0][1], body[0][2], 255)),
                tiny_skia::GradientStop::new(0.5, Color::from_rgba8(body[1][0], body[1][1], body[1][2], 255)),
                tiny_skia::GradientStop::new(1.0, Color::from_rgba8(body[2][0], body[2][1], body[2][2], 255)),
            ];
            let shader = tiny_skia::LinearGradient::new(
                tiny_skia::Point::from_xy(0.0, 0.0),
                tiny_skia::Point::from_xy(0.0, fh),
                stops,
                tiny_skia::SpreadMode::Pad,
                Transform::identity(),
            );
            let mut paint = Paint::default();
            match shader {
                Some(shader) => paint.shader = shader,
                None => paint.set_color_rgba8(body[0][0], body[0][1], body[0][2], 255),
            }
            paint.anti_alias = true;
            pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);

            // A faint wash of the tab's colour rising from the bottom edge.
            if open > 0.0 {
                let mask = gfx::mask_of(w.get(), h.get(), &path);
                let [ar, ag, ab] = self.accent(self.view);
                gfx::glow(&mut pixmap, fw / 2.0, fh * 1.05, fw * 0.62, [ar, ag, ab, (open * 40.0) as u8], mask.as_ref());
            }
        }
        // Border along the sides and bottom; the top edge is flush with the screen.
        let hw = 0.5 * s;
        let mut pb = PathBuilder::new();
        pb.move_to(hw, 0.0);
        pb.line_to(hw, fh - r);
        pb.quad_to(hw, fh - hw, r, fh - hw);
        pb.line_to(fw - r, fh - hw);
        pb.quad_to(fw - hw, fh - hw, fw - hw, fh - r);
        pb.line_to(fw - hw, 0.0);
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            let edge = theme::border();
            paint.set_color_rgba8(edge[0], edge[1], edge[2], edge[3]);
            // The line fades in below the flares, so it does not show beside them.
            let (clear, solid) = (Color::from_rgba8(edge[0], edge[1], edge[2], 0), Color::from_rgba8(edge[0], edge[1], edge[2], edge[3]));
            if let Some(shader) = tiny_skia::LinearGradient::new(
                tiny_skia::Point::from_xy(0.0, ear),
                tiny_skia::Point::from_xy(0.0, ear * 3.0),
                vec![tiny_skia::GradientStop::new(0.0, clear), tiny_skia::GradientStop::new(1.0, solid)],
                tiny_skia::SpreadMode::Pad,
                Transform::identity(),
            ) {
                paint.shader = shader;
            }
            paint.anti_alias = true;
            let stroke = Stroke { width: 1.0 * s, ..Default::default() };
            pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }

        // Fade content in over the second half of the expand animation.
        let alpha = ((self.progress.clamp(0.0, 1.0) - 0.5) * 2.0).clamp(0.0, 1.0) as f32;
        if alpha > 0.0 {
            if self.progress < 0.995 {
                // While opening or closing the content also grows from slightly smaller, so it seems to
                // unfold with the notch.
                if let Some(mut layer) = Pixmap::new(w.get(), h.get()) {
                    self.draw_content(&mut layer, s, alpha, (w.get(), h.get()));
                    let k = (0.93 + 0.07 * self.progress.clamp(0.0, 1.0)) as f32;
                    let at = Transform::from_scale(k, k).post_translate(fw * (1.0 - k) / 2.0, 0.0);
                    let paint = PixmapPaint { quality: FilterQuality::Bilinear, ..Default::default() };
                    pixmap.draw_pixmap(0, 0, layer.as_ref(), &paint, at, None);
                }
            } else {
                self.draw_content(&mut pixmap, s, alpha, (w.get(), h.get()));
            }
        }

        // Collapsed: show the countdown while the timer runs.
        let collapsed_alpha = ((1.0 - self.progress.clamp(0.0, 1.0) * 2.0).clamp(0.0, 1.0) * (1.0 - self.ov_t.clamp(0.0, 1.0))) as f32;
        if collapsed_alpha > 0.0 {
            if let (true, Some(font)) = (self.timer.running(), &self.font) {
                let text = fmt_remaining(self.timer.remaining(Instant::now()));
                let color = [255, 255, 255, (collapsed_alpha * 255.0) as u8];
                draw_text_centered(&mut pixmap, self.heading_font().unwrap_or(font), &text, fw / 2.0, (self.closed_size().1 as f32 / 2.0 + 5.0) * s, 13.0 * s, color, fw);
            } else {
                self.draw_companion(&mut pixmap, s, collapsed_alpha, fw);
            }
        }

        // System notice (volume, battery...) in the compact notch.
        let notice_alpha = self.ov_t.clamp(0.0, 1.0) as f32 * (1.0 - self.progress.clamp(0.0, 1.0) as f32 * 2.0).clamp(0.0, 1.0);
        if let (true, Some(notice)) = (notice_alpha > 0.02, &self.notice) {
            self.draw_notice(&mut pixmap, notice, s, notice_alpha, fw);
        }

        // Put the notch in the middle of the window and add the flares on both sides of its top edge.
        let Some(mut out) = Pixmap::new(full_w.get(), h.get()) else { return };
        out.draw_pixmap(ear as i32, 0, pixmap.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
        draw_ears(&mut out, ear);

        let Some(surface) = self.surface.as_mut() else { return };
        surface.resize(full_w, h).ok();
        let Ok(mut buffer) = surface.buffer_mut() else { return };
        // tiny-skia is premultiplied RGBA; softbuffer wants 0RGB/ARGB u32.
        for (dst, src) in buffer.iter_mut().zip(out.pixels()) {
            *dst = (src.alpha() as u32) << 24
                | (src.red() as u32) << 16
                | (src.green() as u32) << 8
                | src.blue() as u32;
        }
        buffer.present().ok();
    }

    /// The tab bar and the open view. While the view is changing, the old one slides out and the new one
    /// slides in, towards the side of the tab it is on.
    pub(crate) fn draw_content(&self, pm: &mut Pixmap, s: f32, alpha: f32, size: (u32, u32)) {
        self.draw_tabs(pm, s, alpha);
        self.draw_shelf_clear(pm, s, alpha);
        // The other views are drawn for the normal width, so keep them centred while it is still wide.
        let dx = if matches!(self.view, View::Player | View::Lyrics) { 0.0 } else { self.tab_dx() };
        let t = self.switch_t.clamp(0.0, 1.0) as f32;
        let moving = t < 0.999;
        if !moving && dx <= 0.5 {
            self.draw_view(pm, s, alpha, self.view);
            return;
        }
        // Between two tabs the views move as one strip: the old one slides out to the side its tab is on
        // while the new one slides in behind it, across the notch. The notch's own outline clips them.
        let width = size.0 as f32;
        let clip = if self.prev_view.is_some() && moving {
            let r = (size.1 as f32 / 2.0).min(28.0 * s);
            gfx::rounded_path(0.0, -r, width, size.1 as f32 + r, r).and_then(|shape| gfx::mask_of(size.0, size.1, &shape))
        } else {
            None
        };
        if let (true, Some(old)) = (moving, self.prev_view)
            && let Some(mut layer) = Pixmap::new(size.0, size.1) {
                self.draw_view(&mut layer, s, alpha, old);
                let paint = PixmapPaint { opacity: 1.0 - 0.6 * t, quality: FilterQuality::Bilinear, ..Default::default() };
                let at = Transform::from_translate(-self.switch_dir * width * t, 0.0);
                pm.draw_pixmap(0, 0, layer.as_ref(), &paint, at, clip.as_ref());
            }
        if let Some(mut layer) = Pixmap::new(size.0, size.1) {
            self.draw_view(&mut layer, s, alpha, self.view);
            let fade_in = if self.prev_view.is_some() { 0.4 + 0.6 * t } else { t };
            let paint = PixmapPaint { opacity: fade_in, quality: FilterQuality::Bilinear, ..Default::default() };
            let slide = if self.prev_view.is_some() { self.switch_dir * width * (1.0 - t) } else { 0.0 };
            let at = Transform::from_translate(dx * s + slide, if self.prev_view.is_some() { 0.0 } else { (1.0 - t) * 8.0 * s });
            pm.draw_pixmap(0, 0, layer.as_ref(), &paint, at, clip.as_ref());
        }
    }

    /// A view, drawn a little higher than it is laid out so the tab bar fits under it.
    pub(crate) fn draw_view(&self, pm: &mut Pixmap, s: f32, alpha: f32, view: View) {
        let Some(mut layer) = Pixmap::new(pm.width(), pm.height()) else { return };
        self.draw_view_in_place(&mut layer, s, alpha, view);
        pm.draw_pixmap(0, -(TAB_SHIFT * s).round() as i32, layer.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    }

    pub(crate) fn draw_view_in_place(&self, pm: &mut Pixmap, s: f32, alpha: f32, view: View) {
        match view {
            View::Player | View::Lyrics => self.draw_player(pm, s, alpha),
            View::Timer => self.draw_timer(pm, s, alpha),
            View::Water => self.draw_water(pm, s, alpha),
            View::Shelf => self.draw_shelf(pm, s, alpha),
            View::Todo => self.draw_todo(pm, s, alpha),
            View::Calendar => self.draw_calendar(pm, s, alpha),
            View::Clipboard => self.draw_clipboard(pm, s, alpha),
        }
    }
}
