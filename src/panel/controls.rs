//! Drawing the controls of a page: switches, choices, sliders and buttons.

use super::*;

/// What every control needs to be drawn: the window, the two fonts, the row, the keys of its animations, the page's colour and the scale.
#[derive(Clone, Copy)]
struct Ctl<'a> {
    panel: &'a Panel,
    font: &'a fontdue::Font,
    heading: &'a fontdue::Font,
    item: &'a Item,
    key: u64,
    accent: [u8; 3],
    s: f32,
}

impl App {
    /// The control on the right of a row.
    pub(super) fn paint_control(&self, pm: &mut Pixmap, panel: &Panel, font: &fontdue::Font, heading: &fontdue::Font, item: &Item, key: u64, accent: [u8; 3], s: f32) {
        let c = Ctl { panel, font, heading, item, key, accent, s };
        match item.kind {
            Kind::Link(url) => self.paint_link(pm, &c, url),
            Kind::Toggle(t) => self.paint_toggle(pm, &c, t),
            Kind::Seg(seg) => self.paint_seg(pm, &c, seg),
            Kind::EyeGrid => self.paint_eye_grid(pm, &c),
            Kind::TabGrid => self.paint_tab_grid(pm, &c),
            Kind::ScreenMap => {
                let screens = self.detected_screens();
                self.paint_screen_tiles(pm, &c, &screens);
                self.paint_screen_choices(pm, &c, screens.len());
            }
            Kind::Slider(slide) => self.paint_slider(pm, &c, slide),
            Kind::Pick => self.paint_pick(pm, &c),
            Kind::Cycle(stepper) => self.paint_cycle(pm, &c, stepper),
            Kind::Text(Field::Hotkey) => self.paint_hotkey(pm, &c),
            Kind::Text(Field::City) => self.paint_city(pm, &c),
            Kind::Button(act) => self.paint_button(pm, &c, act),
        }
    }

    /// A line with a value on the right, and an arrow when it opens a page.
    fn paint_link(&self, pm: &mut Pixmap, c: &Ctl, url: &str) {
        let Ctl { font, item, s, .. } = *c;
        let grey = [165, 165, 175, 255];
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

    /// The switch of a toggle row.
    fn paint_toggle(&self, pm: &mut Pixmap, c: &Ctl, t: Toggle) {
        let Ctl { panel, item, key, s, .. } = *c;
        let on = panel.glows.follow(key + 1, if self.toggle_value(t) { 1.0 } else { 0.0 }, 34.0, 0.62) as f32;
        // The shared switch is 32 x 18; it is drawn 1.25 times as big here.
        crate::draw_switch(pm, item.ctrl.0 * s, item.ctrl.1 * s, s * 1.25, on, 255);
    }

    /// A row of options with a highlight that slides to the chosen one.
    fn paint_seg(&self, pm: &mut Pixmap, c: &Ctl, seg: Seg) {
        let Ctl { panel, font, item, key, accent, s, .. } = *c;
        let (cx, cy) = panel.cursor;
        let white = [255, 255, 255, 255];
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
        gfx::gradient_rounded(
            pm,
            px - 2.0 * s,
            hy - 2.0 * s,
            pw + 4.0 * s,
            hh + 4.0 * s,
            (hh + 4.0 * s) / 2.0,
            [accent[0], accent[1], accent[2], 26],
            [accent[0], accent[1], accent[2], 10],
            false,
        );
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

    /// The eye picker.
    fn paint_eye_grid(&self, pm: &mut Pixmap, c: &Ctl) {
        let Ctl { panel, item, key, accent, s, .. } = *c;
        let (cx, cy) = panel.cursor;
        // A tile for each style with a pair of eyes in it, drawn the way the closed notch draws them; the chosen
        // one is lit in the page's colour, and the tile under the mouse brightens.
        for (i, style) in EyeStyle::ALL.iter().enumerate() {
            let tile = eye_tile(item.ctrl, i);
            let (x, y, w, h) = (tile.0 * s, tile.1 * s, tile.2 * s, tile.3 * s);
            let over = panel.glows.fade(key + 20 + i as u64, inside(tile, cx, cy));
            let lit = panel.glows.follow(key + 40 + i as u64, if *style == self.settings.eye_style { 1.0 } else { 0.0 }, 30.0, 0.8) as f32;
            crate::fill_rounded(pm, x, y, w, h, 10.0 * s, [255, 255, 255, (14.0 + 20.0 * over) as u8]);
            if lit > 0.01 {
                gfx::gradient_rounded(
                    pm,
                    x,
                    y,
                    w,
                    h,
                    10.0 * s,
                    [accent[0], accent[1], accent[2], (140.0 * lit) as u8],
                    [accent[0], accent[1], accent[2], (86.0 * lit) as u8],
                    false,
                );
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

    /// The tiles for the tabs of the notch.
    fn paint_tab_grid(&self, pm: &mut Pixmap, c: &Ctl) {
        let Ctl { panel, font, heading, item, key, s, .. } = *c;
        let (cx, cy) = panel.cursor;
        let white = [255, 255, 255, 255];
        // A tile for each tab of the notch: its icon on a round badge, its name and a few words, and a light
        // in the corner. A tab that is on is lit in its own colour (the one the notch gives it); one that is off
        // is dim and grey. The tile under the mouse brightens.
        for (i, (view, toggle, name, words)) in TAB_TILES.iter().enumerate() {
            let tile = tab_tile(item.ctrl, i);
            let (x, y, w, h) = (tile.0 * s, tile.1 * s, tile.2 * s, tile.3 * s);
            let on = panel.glows.follow(key + 60 + i as u64, if self.toggle_value(*toggle) { 1.0 } else { 0.0 }, 30.0, 0.8) as f32;
            let over = panel.glows.fade(key + 80 + i as u64, inside(tile, cx, cy));
            let c = match view {
                // The notch takes the Music tab's colour from the cover; here it has one of its own.
                View::Player => [255, 82, 124],
                other => self.accent(*other),
            };
            let (r, g, b) = (c[0], c[1], c[2]);
            crate::fill_rounded(pm, x, y, w, h, 14.0 * s, [255, 255, 255, (9.0 + 12.0 * over) as u8]);
            if on > 0.01 {
                gfx::gradient_rounded(pm, x, y, w, h, 14.0 * s, [r, g, b, (58.0 * on) as u8], [r, g, b, (16.0 * on) as u8], false);
            }
            let edge = gfx::mix([255, 255, 255, 22], [r, g, b, 150], on);
            gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, 14.0 * s, s, edge);
            // The badge: the tab's colour when on, a dark grey when off.
            let (bx, by) = (x + w / 2.0, y + 32.0 * s);
            if on > 0.01 {
                gfx::glow(pm, bx, by, 30.0 * s, [r, g, b, (40.0 * on) as u8], None);
            }
            let badge_top = gfx::mix([70, 70, 82, 255], [r, g, b, 255], on);
            let badge_bottom = gfx::mix([52, 52, 62, 255], gfx::mix([r, g, b, 255], [0, 0, 0, 255], 0.25), on);
            gfx::gradient_rounded(pm, bx - 18.0 * s, by - 18.0 * s, 36.0 * s, 36.0 * s, 18.0 * s, badge_top, badge_bottom, false);
            let ink = gfx::mix([150, 150, 162, 255], white, on);
            crate::draw_tab_icon(pm, *view, bx, by, s * 1.05, ink);
            let name_ink = crate::pick(white, [170, 170, 182, 255], on.max(over * 0.5));
            crate::draw_text_centered(pm, heading, name, bx, y + 70.0 * s, 12.5 * s, name_ink, w - 10.0 * s);
            crate::draw_text_centered(pm, font, words, bx, y + 85.0 * s, 10.0 * s, [128, 128, 142, 255], w - 10.0 * s);
            // The light: green when the tab is on.
            let light = gfx::mix([80, 80, 92, 255], [90, 220, 130, 255], on);
            crate::fill_circle(pm, x + w - 13.0 * s, y + 13.0 * s, 3.5 * s, light);
        }
    }

    /// The connected screens as tiles.
    fn paint_screen_tiles(&self, pm: &mut Pixmap, c: &Ctl, screens: &[Detected]) {
        let Ctl { panel, font, item, key, accent, s, .. } = *c;
        let (cx, cy) = panel.cursor;
        let white = [255, 255, 255, 255];
        let grey = [165, 165, 175, 255];
        // The connected screens as they sit in Windows' layout, each with its number; the one the notch is on
        // is lit in the page's colour with a little notch drawn at its top, and the tile under the mouse
        // brightens. At the right, All lights every screen.
        let rects = tiles(item.ctrl, screens);
        // Its own range of keys for the glows, so they never meet those of another row.
        let own = key + (1u64 << 40);
        let lights_all = self.settings.screen.follows();
        // The screen the notch is on right now (with All, the one the mouse last touched the top of).
        let here = self.window.as_ref().and_then(|w| self.island_monitor(w)).map(|m| (m.position().x, m.position().y));
        for (i, (d, t)) in screens.iter().zip(&rects).enumerate() {
            let (x, y, w, h) = (t.0 * s, t.1 * s, t.2 * s, t.3 * s);
            let over = panel.glows.fade(own + i as u64, inside(*t, cx, cy));
            let target = if is_chosen(d, self.settings.screen) {
                1.0
            } else if lights_all {
                0.6
            } else {
                0.0
            };
            let lit = panel.glows.follow(own + 16 + i as u64, target, 30.0, 0.8) as f32;
            crate::fill_rounded(pm, x, y, w, h, 6.0 * s, [255, 255, 255, (14.0 + 22.0 * over) as u8]);
            if lit > 0.01 {
                gfx::gradient_rounded(
                    pm,
                    x,
                    y,
                    w,
                    h,
                    6.0 * s,
                    [accent[0], accent[1], accent[2], (140.0 * lit) as u8],
                    [accent[0], accent[1], accent[2], (86.0 * lit) as u8],
                    false,
                );
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
    }

    /// All and Every at the right of the map, or a note that there is one screen.
    fn paint_screen_choices(&self, pm: &mut Pixmap, c: &Ctl, screens: usize) {
        let Ctl { panel, font, item, key, accent, s, .. } = *c;
        let (cx, cy) = panel.cursor;
        let white = [255, 255, 255, 255];
        let grey = [165, 165, 175, 255];
        // Its own range of keys for the glows, so they never meet those of another row.
        let own = key + (1u64 << 40);
        let all = self.settings.screen == Screen::All;
        if screens > 1 {
            // All: the notch follows the mouse. Every: a closed notch on each screen, the touched one opens.
            let every = self.settings.screen == Screen::Every;
            for (n, (tile, name, note, on)) in [(all_tile(item.ctrl), "All", "follows mouse", all), (every_tile(item.ctrl), "Every", "all closed", every)].into_iter().enumerate() {
                let (x, y, w, h) = (tile.0 * s, tile.1 * s, tile.2 * s, tile.3 * s);
                let over = panel.glows.fade(own + 48 + 2 * n as u64, inside(tile, cx, cy));
                let lit = panel.glows.follow(own + 49 + 2 * n as u64, if on { 1.0 } else { 0.0 }, 30.0, 0.8) as f32;
                crate::fill_rounded(pm, x, y, w, h, 8.0 * s, [255, 255, 255, (14.0 + 22.0 * over) as u8]);
                if lit > 0.01 {
                    gfx::gradient_rounded(
                        pm,
                        x,
                        y,
                        w,
                        h,
                        8.0 * s,
                        [accent[0], accent[1], accent[2], (140.0 * lit) as u8],
                        [accent[0], accent[1], accent[2], (86.0 * lit) as u8],
                        false,
                    );
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

    /// A slider with its value at the right.
    fn paint_slider(&self, pm: &mut Pixmap, c: &Ctl, slide: Slide) {
        let Ctl { panel, font, item, key, accent, s, .. } = *c;
        let (cx, cy) = panel.cursor;
        let white = [255, 255, 255, 255];
        let grey = [165, 165, 175, 255];
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

    /// The button that shows the chosen alarm sound.
    fn paint_pick(&self, pm: &mut Pixmap, c: &Ctl) {
        let Ctl { panel, font, heading, item, key, accent, s } = *c;
        let (cx, cy) = panel.cursor;
        let white = [255, 255, 255, 255];
        let grey = [165, 165, 175, 255];
        let (x, y, w, h) = (item.ctrl.0 * s, item.ctrl.1 * s, item.ctrl.2 * s, item.ctrl.3 * s);
        let over = panel.glows.fade(key + 9, inside(item.ctrl, cx, cy));
        gfx::gradient_rounded(
            pm,
            x,
            y,
            w,
            h,
            h / 2.0,
            [accent[0], accent[1], accent[2], crate::pick(110.0, 64.0, over) as u8],
            [accent[0], accent[1], accent[2], crate::pick(66.0, 32.0, over) as u8],
            false,
        );
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

    /// A choice from a long list with an arrow at each end.
    fn paint_cycle(&self, pm: &mut Pixmap, c: &Ctl, stepper: Stepper) {
        let Ctl { panel, heading, item, key, accent, s, .. } = *c;
        let (cx, cy) = panel.cursor;
        let white = [255, 255, 255, 255];
        let grey = [165, 165, 175, 255];
        let (x, y, w, h) = (item.ctrl.0 * s, item.ctrl.1 * s, item.ctrl.2 * s, item.ctrl.3 * s);
        // The zone is dimmed and does not answer while the method is not e-Solat.
        let live = stepper != Stepper::Zone || self.zone_applies();
        let strength = if live { 1.0 } else { 0.45 };
        let over = panel.glows.fade(key + 9, live && inside(item.ctrl, cx, cy));
        gfx::gradient_rounded(
            pm,
            x,
            y,
            w,
            h,
            h / 2.0,
            [accent[0], accent[1], accent[2], (crate::pick(110.0, 64.0, over) * strength) as u8],
            [accent[0], accent[1], accent[2], (crate::pick(66.0, 32.0, over) * strength) as u8],
            false,
        );
        gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [accent[0], accent[1], accent[2], (130.0 * strength) as u8]);
        let name = match stepper {
            Stepper::Method => crate::prayer::METHODS[crate::prayer::method_index(self.settings.prayer_method)].1.to_string(),
            Stepper::Zone => self.zone_text(),
        };
        crate::draw_text_centered(
            pm,
            heading,
            &name,
            (item.ctrl.0 + item.ctrl.2 / 2.0) * s,
            (item.ctrl.1 + 18.5) * s,
            12.0 * s,
            if live { white } else { grey },
            (item.ctrl.2 - 56.0) * s,
        );
        // The arrows at the two ends.
        let hot_left = over > 0.01 && cx < item.ctrl.0 + item.ctrl.2 / 3.0;
        for (bx, glyph) in [(item.ctrl.0 + 16.0, "\u{2039}"), (item.ctrl.0 + item.ctrl.2 - 16.0, "\u{203A}")] {
            let ink = if hot_left == (glyph == "\u{2039}") && over > 0.01 { white } else { grey };
            crate::draw_text_centered(pm, heading, glyph, bx * s, (item.ctrl.1 + 19.0) * s, 17.0 * s, ink, 16.0 * s);
        }
    }

    /// The box that shows the hotkey.
    fn paint_hotkey(&self, pm: &mut Pixmap, c: &Ctl) {
        let Ctl { panel, font, heading, item, key, accent, s } = *c;
        let (cx, cy) = panel.cursor;
        let white = [255, 255, 255, 255];
        let grey = [165, 165, 175, 255];
        let (x, y, w, h) = (item.ctrl.0 * s, item.ctrl.1 * s, item.ctrl.2 * s, item.ctrl.3 * s);
        let over = panel.glows.fade(key + 9, inside(item.ctrl, cx, cy));
        let centre = (item.ctrl.0 + item.ctrl.2 / 2.0) * s;
        if panel.capture {
            crate::fill_rounded(pm, x, y, w, h, h / 2.0, [0, 0, 0, 90]);
            gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [accent[0], accent[1], accent[2], 210]);
            crate::draw_text_centered(pm, font, "Press the keys...", centre, (item.ctrl.1 + 18.5) * s, 12.0 * s, grey, item.ctrl.2 * s);
        } else {
            let strength = if self.settings.hotkey { 1.0 } else { 0.45 };
            gfx::gradient_rounded(
                pm,
                x,
                y,
                w,
                h,
                h / 2.0,
                [accent[0], accent[1], accent[2], (crate::pick(110.0, 64.0, over) * strength) as u8],
                [accent[0], accent[1], accent[2], (crate::pick(66.0, 32.0, over) * strength) as u8],
                false,
            );
            gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [accent[0], accent[1], accent[2], (130.0 * strength) as u8]);
            let name = crate::hotkey::name(self.settings.hotkey_mods, self.settings.hotkey_vk);
            crate::draw_text_centered(pm, heading, &name, centre, (item.ctrl.1 + 18.5) * s, 12.0 * s, if self.settings.hotkey { white } else { grey }, (item.ctrl.2 - 20.0) * s);
        }
    }

    /// The box for the city.
    fn paint_city(&self, pm: &mut Pixmap, c: &Ctl) {
        let Ctl { panel, font, heading, item, key, accent, s } = *c;
        let (cx, cy) = panel.cursor;
        let white = [255, 255, 255, 255];
        let grey = [165, 165, 175, 255];
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
            gfx::gradient_rounded(
                pm,
                x,
                y,
                w,
                h,
                h / 2.0,
                [accent[0], accent[1], accent[2], crate::pick(110.0, 64.0, over) as u8],
                [accent[0], accent[1], accent[2], crate::pick(66.0, 32.0, over) as u8],
                false,
            );
            gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [accent[0], accent[1], accent[2], 130]);
            let (name, ink) = match &self.settings.place {
                Some(p) => (p.name.as_str(), white),
                None => ("Not set", grey),
            };
            crate::draw_text_centered(pm, heading, name, (item.ctrl.0 + item.ctrl.2 / 2.0) * s, (item.ctrl.1 + 18.5) * s, 12.0 * s, ink, inner);
        }
    }

    /// A button.
    fn paint_button(&self, pm: &mut Pixmap, c: &Ctl, act: Act) {
        let Ctl { panel, heading, item, key, accent, s, .. } = *c;
        let (cx, cy) = panel.cursor;
        let white = [255, 255, 255, 255];
        let armed = act == Act::Reset && panel.reset_armed.is_some_and(|t| Instant::now() < t);
        let over = panel.glows.fade(key + 9, inside(item.ctrl, cx, cy));
        let (x, y, w, h) = (item.ctrl.0 * s, item.ctrl.1 * s, item.ctrl.2 * s, item.ctrl.3 * s);
        let colour = if armed || act == Act::Reset { [255, 110, 96] } else { accent };
        gfx::gradient_rounded(
            pm,
            x,
            y,
            w,
            h,
            h / 2.0,
            [colour[0], colour[1], colour[2], crate::pick(110.0, 62.0, over) as u8],
            [colour[0], colour[1], colour[2], crate::pick(66.0, 30.0, over) as u8],
            false,
        );
        gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [colour[0], colour[1], colour[2], 130]);
        let download = match &self.updates.status {
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
