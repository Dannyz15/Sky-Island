//! Putting the island on the screen: the draw pass, the layers views are drawn on, and the shared hover and fade helpers.

use crate::*;

/// The smallest rectangle (left, top, width, height) that holds every pixel of `layer` that is not fully transparent,
/// with `margin` pixels of room all round; `None` when the layer is empty.
fn drawn_bounds(layer: &Pixmap, margin: u32) -> Option<tiny_skia::IntRect> {
    let (w, h) = (layer.width() as usize, layer.height() as usize);
    let pixels = layer.pixels();
    let row = |y: usize| &pixels[y * w..(y + 1) * w];
    let top = (0..h).find(|&y| row(y).iter().any(|p| p.alpha() != 0))?;
    let bottom = (top..h).rev().find(|&y| row(y).iter().any(|p| p.alpha() != 0))?;
    let (mut left, mut right) = (w, 0);
    for y in top..=bottom {
        let r = row(y);
        if let Some(first) = r.iter().position(|p| p.alpha() != 0) {
            left = left.min(first);
            right = right.max(r.iter().rposition(|p| p.alpha() != 0).unwrap_or(first));
        }
    }
    let m = margin as usize;
    let (x0, y0) = (left.saturating_sub(m), top.saturating_sub(m));
    let (x1, y1) = ((right + 1 + m).min(w), (bottom + 1 + m).min(h));
    tiny_skia::IntRect::from_xywh(x0 as i32, y0 as i32, (x1 - x0) as u32, (y1 - y0) as u32)
}

/// Paste `layer` onto `pm` through `paint` and `transform`, like `draw_pixmap(0, 0, ..)` but only the part of the
/// layer that has anything in it: a row of a list is a small part of a layer as big as the whole picture, and pasting
/// all of it for every row of every frame took about a millisecond a row. The part has a margin of transparent
/// pixels all round, so the soft edge that a move by a fraction of a pixel makes comes out the same.
fn paste_drawn_part(pm: &mut Pixmap, layer: &Pixmap, paint: &PixmapPaint, transform: Transform) {
    let Some(bounds) = drawn_bounds(layer, 2) else { return };
    if let Some(part) = layer.clone_rect(bounds) {
        pm.draw_pixmap(bounds.x(), bounds.y(), part.as_ref(), paint, transform, None);
    }
}

impl App {
    /// The colour the current view is tinted with.
    pub(crate) fn accent(&self, view: View) -> [u8; 3] {
        match view {
            View::Player | View::Lyrics => self.art.shown.map(|c| c.round().clamp(0.0, 255.0) as u8),
            View::Timer => [255, 99, 71],
            View::Water => [64, 170, 255],
            View::Weather => [255, 190, 70],
            View::Prayer => [90, 200, 170],
            View::Stats => [255, 110, 150],
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
        if self.spring.progress < 0.9 || !self.pointer.hovered {
            return false;
        }
        let (x, y) = (self.pointer.cursor.0 as f32 / s, self.pointer.cursor.1 as f32 / s + TAB_SHIFT);
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
            View::Clipboard => (View::Clipboard, self.clip.history.scroll, self.clip.history.items.len(), 0),
            View::Calendar => (View::Calendar, 0, self.todo.on_day(self.cal.selected).len(), self.cal.selected),
            v => (v, 0, 0, 0),
        }
    }

    /// How far row `row` of `rows` has come in, 0 to 1: each row a little after the one above it.
    fn row_in(&self, row: usize, rows: usize) -> f32 {
        const STAGGER: f32 = 0.07;
        let span = 1.0 - STAGGER * (rows.max(1) - 1) as f32;
        let t = ((self.slides.list_t as f32 - row as f32 * STAGGER) / span).clamp(0.0, 1.0);
        1.0 - (1.0 - t).powi(3)
    }

    /// Draw on a layer of its own and paste it back faded to `opacity` and moved by `shift` (pixels): how
    /// something eases in while a list, a month or a track changes.
    fn eased_in(&self, pm: &mut Pixmap, opacity: f32, shift: (f32, f32), draw: impl Fn(&mut Pixmap)) {
        let Some(mut layer) = Pixmap::new(pm.width(), pm.height()) else { return draw(pm) };
        draw(&mut layer);
        let paint = PixmapPaint { opacity: opacity.clamp(0.0, 1.0), quality: FilterQuality::Bilinear, ..Default::default() };
        paste_drawn_part(pm, &layer, &paint, Transform::from_translate(shift.0, shift.1));
    }

    /// The calendar's month (or week): after the arrows were used it slides in from the side of the arrow.
    pub(crate) fn slide_in(&self, pm: &mut Pixmap, s: f32, draw: impl Fn(&mut Pixmap)) {
        let t = self.slides.cal_t as f32;
        if t >= 0.999 {
            return draw(pm);
        }
        self.eased_in(pm, t, ((1.0 - t) * 30.0 * self.slides.cal_dir * s, 0.0), draw);
    }

    /// The cover and the name of the track: after a new track began they fade in and slide a little.
    pub(crate) fn fade_in_track(&self, pm: &mut Pixmap, s: f32, draw: impl Fn(&mut Pixmap)) {
        let t = self.art.track_t as f32;
        if t >= 0.999 {
            return draw(pm);
        }
        self.eased_in(pm, t, ((1.0 - t) * 12.0 * s, 0.0), draw);
    }

    /// Draw one row of a list. While the list is changing the row is drawn on a layer of its own, which
    /// fades in and slides a little from the side the list moved from.
    pub(crate) fn animated_row(&self, pm: &mut Pixmap, s: f32, row: usize, rows: usize, draw: impl Fn(&mut Pixmap)) {
        let e = self.row_in(row, rows);
        if e >= 0.999 {
            return draw(pm);
        }
        self.eased_in(pm, e, (0.0, (1.0 - e) * 14.0 * self.slides.list_dir * s), draw);
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
        theme::set_glass(self.settings.look.glassy_controls());
        let frame_start = Instant::now();
        self.glows.set_motion(self.settings.motion.speed(), self.settings.motion.bounce());
        self.glows.begin(frame_start.duration_since(self.pacing.last_draw).as_secs_f64().min(1.0 / 30.0));
        self.pacing.last_draw = frame_start;
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
        let look = self.settings.look;
        // The floating pill hangs a little below the top edge; what is on it is drawn below that gap.
        let gap = (crate::glass::gap(look) as f32 * s).round() as u32;
        // While a drop hangs below the closed glass notch the window has room for it under the notch.
        let room = (self.room_below() as f32 * s).round() as u32;
        let Some(ch) = NonZeroU32::new(h.get().saturating_sub(gap + room)) else { return };
        let Some(mut pixmap) = Pixmap::new(w.get(), ch.get()) else { return };
        pixmap.fill(Color::TRANSPARENT);

        // The notch's body, glow and border only change with its size, the look and the tab's colour. Once it
        // is at rest (fully open or closed) it is drawn once and copied in the following frames.
        let fw = w.get() as f32;
        let open = self.spring.progress.clamp(0.0, 1.0) as f32;
        // The liquid parts of the outline: the drop while it opens, bubbles, the swelling towards the mouse.
        let liquid = self.liquid(s, fw, true);
        let body = self.glass_body(&window, (full_w.get(), h.get()), s, ear, gap as f32, room as f32, open, liquid);
        // The body ends above the drop running down below it.
        let body_h = (ch.get() as f32 - liquid.drip).max(1.0);

        // Fade the content in as the notch opens, easing in and out so it does not pop in or stop short, and bring it in
        // from a blur. The same for every look.
        let p = self.spring.progress.clamp(0.0, 1.0) as f32;
        let alpha = smoothstep(((p - 0.3) / 0.6).clamp(0.0, 1.0));
        if alpha > 0.0 {
            if self.spring.progress < 0.995 {
                // While opening or closing it is laid out at the open size and that picture is shrunk into the notch as
                // it is, so it grows with the notch, and blurred while it is faint.
                let (ow, oh) = self.open_size();
                let (lw, lh) = ((ow as f32 * s).round().max(fw) as u32, (oh as f32 * s).round().max(ch.get() as f32) as u32);
                if let Some(mut layer) = Pixmap::new(lw, lh) {
                    self.layout_h.set(Some(lh as f32 / s));
                    self.draw_content(&mut layer, s, alpha, (lw, lh));
                    self.layout_h.set(None);
                    // A strong blur is done on a smaller copy, which is then drawn that much larger.
                    let r = (1.0 - alpha) * CONTENT_BLUR * s;
                    let f = if r < 0.5 {
                        1
                    } else if r < 6.0 {
                        2
                    } else {
                        4
                    };
                    if f > 1
                        && let Some(small) = gfx::shrink(&layer, f)
                    {
                        layer = small;
                    }
                    gfx::blur(&mut layer, (r / f as f32).round() as usize);
                    let (k, f) = ((fw / lw as f32).min(body_h / lh as f32) * (0.94 + 0.06 * p), f as f32);
                    let at = Transform::from_scale(k * f, k * f).post_translate((fw - lw as f32 * k) / 2.0, 0.0);
                    let paint = PixmapPaint { quality: FilterQuality::Bilinear, ..Default::default() };
                    pixmap.draw_pixmap(0, 0, layer.as_ref(), &paint, at, None);
                }
            } else {
                self.draw_content(&mut pixmap, s, alpha, (w.get(), ch.get()));
            }
        }

        // What the closed notch shows (the clock or eyes, the song, the timer's countdown) and a notice, while closed.
        // Eased like the open content, so the clock or the eyes melt away as it opens instead of fading at one rate.
        let closed_fade = smoothstep((1.0 - p * 2.0).clamp(0.0, 1.0));
        let ov = smoothstep(self.notices.t.clamp(0.0, 1.0) as f32);
        let collapsed_alpha = closed_fade * (1.0 - ov);
        let notice_alpha = ov * closed_fade;
        self.draw_closed_content(&mut pixmap, s, fw, collapsed_alpha, notice_alpha);

        // Put what is on the notch in the middle of the window, over its body. While the outline is liquid (a drop
        // running down, bubbles) what is on it is kept inside the outline.
        let Some(mut out) = body.or_else(|| Pixmap::new(full_w.get(), h.get())) else { return };
        if liquid.drip > 0.0 || liquid.bubbles.is_some() {
            gfx::clip_to(&mut pixmap, &out, ear as i32, gap as i32);
        }
        if open > 0.0 {
            // The open notch casts a soft shadow on what is below it (the floating pill all round, below the gap above
            // it; the others from the top edge of the screen).
            let r = (body_h / 2.0).min(30.0 * s);
            let strength = (open * if self.light() { 70.0 } else { 150.0 }) as u8;
            let top = if look.floating() { gap as f32 } else { -40.0 * s };
            gfx::soft_shadow_behind(&mut out, ear, top, fw, gap as f32 + body_h - top, r, 10.0 * s, strength);
        }
        gfx::blit_over(&mut out, &pixmap, ear as i32, gap as i32);

        let Some(surface) = self.surface.as_mut() else { return };
        surface.resize(full_w, h).ok();
        let Ok(mut buffer) = surface.buffer_mut() else { return };
        // tiny-skia is premultiplied RGBA; softbuffer wants 0RGB/ARGB u32.
        for (dst, src) in buffer.iter_mut().zip(out.pixels()) {
            *dst = (src.alpha() as u32) << 24 | (src.red() as u32) << 16 | (src.green() as u32) << 8 | src.blue() as u32;
        }
        buffer.present().ok();
        // The copies of this notch on other screens are drawn with it, so they always show the same.
        self.draw_mirrors(false);
    }

    /// What the closed notch shows on top of its body: the countdown while the timer runs, else the clock or the eyes,
    /// with the record and the bars while a song plays, and then a system notice (volume, battery...) in the compact
    /// notch. `s` is the scale it is drawn at and `fw` the width of the notch's picture. Used for the real notch and for
    /// the copies of it on the other screens.
    pub(crate) fn draw_closed_content(&self, pixmap: &mut Pixmap, s: f32, fw: f32, collapsed_alpha: f32, notice_alpha: f32) {
        if collapsed_alpha > 0.0 {
            if let (true, Some(font)) = (self.timer.running(), &self.font) {
                let text = fmt_remaining(self.timer.remaining(Instant::now()));
                let color = [255, 255, 255, (collapsed_alpha * 255.0) as u8];
                draw_text_centered(pixmap, self.heading_font().unwrap_or(font), &text, fw / 2.0, (self.closed_size().1 as f32 / 2.0 + 5.0) * s, 13.0 * s, color, fw);
            } else {
                self.draw_companion(pixmap, s, collapsed_alpha, fw);
            }
            self.draw_closed_music(pixmap, s, collapsed_alpha, fw);
        }
        if let (true, Some(notice)) = (notice_alpha > 0.02, &self.notices.current) {
            self.draw_notice(pixmap, notice, s, notice_alpha, fw);
        }
    }

    /// The closed notch as a finished picture (the flares on its sides included) for a screen that is drawn at scale
    /// `s`, as the window of that screen shows it. `None` if there is no room for it.
    pub(crate) fn closed_picture(&self, s: f32) -> Option<Pixmap> {
        let ov = self.notices.t.clamp(0.0, 1.0);
        let closed = self.closed_size();
        let (lw, lh) = (lerp(closed.0, OVERLAY.0, ov), lerp(closed.1, OVERLAY.1, ov));
        let ear = (EAR as f32 * s).round();
        let full_w = ((lw as f32 + 2.0 * EAR as f32) * s).round().max(1.0) as u32;
        let h = (lh as f32 * s).round().max(1.0) as u32;
        let w = full_w.checked_sub(2 * ear as u32).filter(|w| *w > 0)?;
        let look = self.settings.look;
        let gap = (crate::glass::gap(look) as f32 * s).round() as u32;
        let mut pixmap = Pixmap::new(w, h)?;
        pixmap.fill(Color::TRANSPARENT);
        self.draw_closed_content(&mut pixmap, s, w as f32, (1.0 - ov) as f32, ov as f32);
        let mut out = Pixmap::new(full_w, h + gap)?;
        // Nothing is copied from behind the copies; their glass shows a plain dark (or light) background. The swelling
        // towards the mouse is only on the notch the mouse is over.
        let liquid = self.liquid(s, w as f32, false);
        let g = crate::glass::Glass {
            look,
            light: self.light(),
            backdrop: None,
            origin: (0.0, 0.0),
            s,
            ear,
            gap: gap as f32,
            stretch: 0.0,
            sun: SUN_REST,
            wash: None,
            room: 0.0,
            drop: 0.0,
            liquid,
            clear: self.settings.glass_clear as u8,
        };
        crate::glass::paint(&mut out, &g);
        if liquid.bubbles.is_some() {
            gfx::clip_to(&mut pixmap, &out, ear as i32, gap as i32);
        }
        gfx::blit_over(&mut out, &pixmap, ear as i32, gap as i32);
        Some(out)
    }

    /// The glass body of the notch for a window of `size` pixels (flares included), kept while nothing about it changes.
    fn glass_body(&mut self, window: &Window, size: (u32, u32), s: f32, ear: f32, gap: f32, room: f32, open: f32, liquid: crate::glass::Liquid) -> Option<Pixmap> {
        let look = self.settings.look;
        let backdrop = if crate::glass::wants_backdrop(look) { self.backdrop.as_ref().and_then(|b| b.latest()) } else { None };
        let origin = window.outer_position().map_or((0, 0), |p| (p.x, p.y));
        let (sun, bucket) = self.sun();
        let accent = self.accent(self.view);
        // Kept at rest only: open or closed, no drop hanging, nothing running down or swelling.
        let still = (open == 0.0 || open >= 1.0) && self.drop_room() == 0.0 && liquid.drip == 0.0 && liquid.bulge.is_none();
        let key = still.then(|| BodyKey {
            size,
            scale: s.to_bits(),
            light: self.light(),
            open: open >= 1.0,
            accent: if open >= 1.0 { accent } else { [0; 3] },
            look,
            backdrop: backdrop.as_ref().map_or(0, |b| b.version),
            sun: bucket,
            origin,
            liquid,
            room: room.to_bits(),
        });
        if let (Some(key), Some((kept, picture))) = (&key, &self.body)
            && key == kept
        {
            return Some(picture.clone());
        }
        let mut pm = Pixmap::new(size.0, size.1)?;
        // While the notch moves fast its corners grow rounder, like a liquid.
        let stretch = (self.spring.progress_v.abs() / 10.0).min(1.0) as f32;
        let g = crate::glass::Glass {
            look,
            light: self.light(),
            backdrop: backdrop.as_deref(),
            origin: (origin.0 as f32, origin.1 as f32),
            s,
            ear,
            gap,
            stretch,
            sun,
            // The tab's colour rises in the glass, not in a notch with no glass.
            wash: (open > 0.0 && look.glassy_controls()).then_some((accent, open)),
            room,
            drop: (self.drop_hang() as f32 * s).min((room - 1.0).max(0.0)),
            liquid,
            clear: self.settings.glass_clear as u8,
        };
        crate::glass::paint(&mut pm, &g);
        if let Some(key) = key {
            self.body = Some((key, pm.clone()));
        }
        Some(pm)
    }

    /// Where the light on the glass comes from, and that direction rounded to one of 64 (which the kept body is
    /// keyed by): towards the mouse while it is over the open notch, else from the lower left. It glides round.
    fn sun(&self) -> ((f32, f32), i32) {
        let s = self.ui_scale();
        let goal = if self.pointer.hovered && self.spring.progress > 0.5 {
            let w = self.window.as_ref().map_or(1.0, |w| w.inner_size().width as f64) - 2.0 * EAR * s;
            let h = self.window_h() as f64 * s;
            (self.pointer.cursor.0 - w / 2.0, self.pointer.cursor.1 - h / 2.0)
        } else {
            (SUN_REST.0 as f64, SUN_REST.1 as f64)
        };
        let l = (goal.0 * goal.0 + goal.1 * goal.1).sqrt().max(1e-6);
        let x = self.glows.follow(KEY_SUN, goal.0 / l, 14.0, 1.0);
        let y = self.glows.follow(KEY_SUN + 1, goal.1 / l, 14.0, 1.0);
        let bucket = ((y.atan2(x) / std::f64::consts::TAU * 64.0).round() as i32).rem_euclid(64);
        let angle = bucket as f64 / 64.0 * std::f64::consts::TAU;
        ((angle.cos() as f32, angle.sin() as f32), bucket)
    }
    /// The tab bar and the open view. While the view is changing, the old one slides out and the new one
    /// slides in, towards the side of the tab it is on.
    fn draw_content(&self, pm: &mut Pixmap, s: f32, alpha: f32, size: (u32, u32)) {
        self.draw_tabs(pm, s, alpha);
        self.draw_shelf_clear(pm, s, alpha);
        // The other views are drawn for the normal width, so keep them centred while it is still wide.
        let dx = if matches!(self.view, View::Player | View::Lyrics) { 0.0 } else { self.tab_dx() };
        let t = self.switch.t.clamp(0.0, 1.0) as f32;
        let moving = t < 0.999;
        if !moving && dx <= 0.5 {
            self.draw_view(pm, s, alpha, self.view);
            return;
        }
        // Between two tabs the views move as one strip: the old one slides out to the side its tab is on
        // while the new one slides in behind it, across the notch. The notch's own outline clips them.
        let width = size.0 as f32;
        let clip = if self.switch.prev.is_some() && moving {
            let r = (size.1 as f32 / 2.0).min(28.0 * s);
            gfx::rounded_path(0.0, -r, width, size.1 as f32 + r, r).and_then(|shape| gfx::mask_of(size.0, size.1, &shape))
        } else {
            None
        };
        if let (true, Some(old)) = (moving, self.switch.prev)
            && let Some(mut layer) = Pixmap::new(size.0, size.1)
        {
            self.draw_view(&mut layer, s, alpha, old);
            let paint = PixmapPaint { opacity: 1.0 - 0.6 * t, quality: FilterQuality::Bilinear, ..Default::default() };
            let at = Transform::from_translate(-self.switch.dir * width * t, 0.0);
            pm.draw_pixmap(0, 0, layer.as_ref(), &paint, at, clip.as_ref());
        }
        if let Some(mut layer) = Pixmap::new(size.0, size.1) {
            self.draw_view(&mut layer, s, alpha, self.view);
            let fade_in = if self.switch.prev.is_some() { 0.4 + 0.6 * t } else { t };
            let paint = PixmapPaint { opacity: fade_in, quality: FilterQuality::Bilinear, ..Default::default() };
            let slide = if self.switch.prev.is_some() { self.switch.dir * width * (1.0 - t) } else { 0.0 };
            let at = Transform::from_translate(dx * s + slide, if self.switch.prev.is_some() { 0.0 } else { (1.0 - t) * 8.0 * s });
            pm.draw_pixmap(0, 0, layer.as_ref(), &paint, at, clip.as_ref());
        }
    }

    /// A view, drawn a little higher than it is laid out so the tab bar fits under it.
    fn draw_view(&self, pm: &mut Pixmap, s: f32, alpha: f32, view: View) {
        let Some(mut layer) = Pixmap::new(pm.width(), pm.height()) else { return };
        self.draw_view_in_place(&mut layer, s, alpha, view);
        gfx::blit_over(pm, &layer, 0, -(TAB_SHIFT * s).round() as i32);
    }

    fn draw_view_in_place(&self, pm: &mut Pixmap, s: f32, alpha: f32, view: View) {
        match view {
            View::Player | View::Lyrics => self.draw_player(pm, s, alpha),
            View::Timer => self.draw_timer(pm, s, alpha),
            View::Water => self.draw_water(pm, s, alpha),
            View::Weather => self.draw_weather(pm, s, alpha),
            View::Prayer => self.draw_prayer(pm, s, alpha),
            View::Stats => self.draw_stats(pm, s, alpha),
            View::Shelf => self.draw_shelf(pm, s, alpha),
            View::Todo => self.draw_todo(pm, s, alpha),
            View::Calendar => self.draw_calendar(pm, s, alpha),
            View::Clipboard => self.draw_clipboard(pm, s, alpha),
        }
    }
}

/// What the notch's body depends on: when none of it changed, the picture of it can be reused.
/// How far the content is blurred when it starts to come in, in logical pixels.
const CONTENT_BLUR: f32 = 10.0;

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

#[derive(PartialEq)]
pub(crate) struct BodyKey {
    size: (u32, u32),
    scale: u32,
    light: bool,
    open: bool,
    accent: [u8; 3],
    look: NotchLook,
    /// For the glass: which copy of what is behind it, where the light comes from, and where the window is.
    backdrop: u64,
    sun: i32,
    origin: (i32, i32),
    liquid: crate::glass::Liquid,
    room: u32,
}

/// Where the light on the glass comes from while the mouse is not over the notch: the lower left.
const SUN_REST: (f32, f32) = (-0.75, 0.66);

/// The glows that make the light on the glass glide round (two in a row).
const KEY_SUN: u64 = 0x6C69_6768_7400_0001;

#[cfg(test)]
mod paste_tests {
    use super::*;

    fn background() -> Pixmap {
        let mut pm = Pixmap::new(200, 120).unwrap();
        pm.fill(Color::from_rgba8(20, 24, 40, 255));
        pm
    }

    fn layer_with(rects: &[(f32, f32, f32, f32, [u8; 4])]) -> Pixmap {
        let mut layer = Pixmap::new(200, 120).unwrap();
        for &(x, y, w, h, c) in rects {
            let mut paint = Paint::default();
            paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
            layer.fill_rect(Rect::from_xywh(x, y, w, h).unwrap(), &paint, Transform::identity(), None);
        }
        layer
    }

    /// Pasting only the drawn part gives the very same picture as pasting the whole layer, for any move and any fade.
    #[test]
    fn pasting_the_drawn_part_gives_the_same_picture_as_pasting_the_whole_layer() {
        let layers = [
            layer_with(&[(30.5, 40.25, 90.0, 22.5, [255, 255, 255, 200])]),
            layer_with(&[(0.0, 0.0, 200.0, 120.0, [200, 90, 40, 255])]),
            layer_with(&[(0.0, 0.0, 3.0, 3.0, [255, 0, 0, 255]), (190.0, 110.0, 10.0, 10.0, [0, 255, 0, 128])]),
            layer_with(&[(100.0, 60.0, 1.0, 1.0, [255, 255, 255, 255])]),
        ];
        for layer in &layers {
            for opacity in [0.0, 0.3, 0.77, 1.0] {
                for shift in [(0.0, 0.0), (3.0, 0.0), (0.3, 5.1), (-7.6, 2.2), (30.0, -14.0), (-1.0, 13.9)] {
                    let paint = PixmapPaint { opacity, quality: FilterQuality::Bilinear, ..Default::default() };
                    let transform = Transform::from_translate(shift.0, shift.1);
                    let (mut whole, mut part) = (background(), background());
                    whole.draw_pixmap(0, 0, layer.as_ref(), &paint, transform, None);
                    paste_drawn_part(&mut part, layer, &paint, transform);
                    assert!(whole.data() == part.data(), "differs at opacity {opacity}, shift {shift:?}");
                }
            }
        }
    }

    #[test]
    fn an_empty_layer_pastes_nothing_and_the_bounds_hold_what_was_drawn() {
        let empty = Pixmap::new(50, 50).unwrap();
        assert!(drawn_bounds(&empty, 2).is_none());
        let mut pm = background();
        let before = pm.data().to_vec();
        paste_drawn_part(&mut pm, &Pixmap::new(200, 120).unwrap(), &PixmapPaint::default(), Transform::from_translate(4.0, 4.0));
        assert!(pm.data() == before.as_slice());
        let b = drawn_bounds(&layer_with(&[(30.0, 40.0, 10.0, 6.0, [255, 255, 255, 255])]), 0).unwrap();
        assert_eq!((b.x(), b.y(), b.width(), b.height()), (30, 40, 10, 6));
        let wide = drawn_bounds(&layer_with(&[(0.0, 0.0, 200.0, 120.0, [1, 2, 3, 255])]), 2).unwrap();
        assert_eq!((wide.x(), wide.y(), wide.width(), wide.height()), (0, 0, 200, 120), "the margin stops at the edge of the layer");
    }
}
