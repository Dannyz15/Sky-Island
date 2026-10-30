//! The Shelf view: dropped files, selecting them and dragging them out.

use crate::*;

// Shelf tiles, in logical pixels.
const TILE: (f32, f32) = (68.0, 58.0);

const TILE_GAP: (f32, f32) = (8.0, 6.0);

const TILES_ORIGIN: (f32, f32) = (24.0, 36.0);

/// Cursor travel, in physical pixels, before a press on a tile becomes a drag.
pub(crate) const DRAG_THRESHOLD: f64 = 6.0;

/// The shelf's Clear button: in the free corner of the tab row, level with the pill. Window coordinates,
/// for a window `height` logical pixels high (it is not part of the view, which is drawn higher).
pub(crate) fn shelf_clear_rect(height: f32) -> (f32, f32, f32, f32) {
    (360.0, height - TAB_BOTTOM + (TAB_PILL_H - 16.0) / 2.0, 36.0, 16.0)
}

pub(crate) fn tile_rect(slot: usize) -> (f32, f32, f32, f32) {
    let (col, row) = ((slot % COLS) as f32, (slot / COLS) as f32);
    (
        TILES_ORIGIN.0 + col * (TILE.0 + TILE_GAP.0),
        TILES_ORIGIN.1 + row * (TILE.1 + TILE_GAP.1),
        TILE.0,
        TILE.1,
    )
}

impl App {
    /// Take the keyboard while files are highlighted (the notch never takes focus on its own), so that
    /// Ctrl+A and Esc reach it, and give it back to the app the user was in once nothing is highlighted.
    pub(crate) fn sync_selection_focus(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SetForegroundWindow};
        if !self.selected.is_empty() && !self.selecting && !self.editing && self.timer_field.is_none() {
            let fg = unsafe { GetForegroundWindow() };
            self.prev_foreground = (!fg.is_invalid()).then_some(fg);
            self.selecting = true;
            if let Some(w) = &self.window {
                w.focus_window();
            }
        } else if self.selected.is_empty() && self.selecting {
            self.selecting = false;
            if let Some(h) = self.prev_foreground.take() {
                let _ = unsafe { SetForegroundWindow(h) };
            }
        }
    }

    /// Start dragging a shelf file out to another app. Blocks until the drag ends.
    pub(crate) fn drag_out(&mut self, idx: usize) {
        let Some(path) = self.shelf.items.get(idx).cloned() else { return };
        // Dragging a highlighted file takes every highlighted file; any other file goes alone.
        let paths = if self.selected.contains(&path) {
            self.shelf.items.iter().filter(|p| self.selected.contains(p)).cloned().collect()
        } else {
            self.selected.clear();
            self.sync_selection_focus();
            vec![path]
        };
        let dropped = dragdrop::drag_files(&paths).unwrap_or(false);
        // No cursor events arrive during the drag, so work out whether it ended over the notch.
        let back_on_notch = self.cursor_over_window();
        // Files that were dropped somewhere have left the shelf, whether the drop moved or copied them
        // (a copy to another drive or into a chat app leaves the original where it was). Letting go over
        // nothing, cancelling, or dropping them back on the notch keeps them.
        if dropped && !back_on_notch {
            for path in &paths {
                if let Some(i) = self.shelf.items.iter().position(|p| p == path) {
                    self.shelf.remove(i);
                }
            }
            self.selected.clear();
            self.sync_selection_focus();
        }
        // The file may also have been moved by the drop.
        self.shelf.prune();
        self.hovered = back_on_notch;
        self.enter_at = None;
        self.leave_at = None;
        self.last_tick = Instant::now();
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Fetch icons for any shelf files that don't have one cached yet, and forget removed files.
    pub(crate) fn load_icons(&mut self) {
        self.icons.retain(|p, _| self.shelf.items.contains(p));
        for path in &self.shelf.items {
            if !self.icons.contains_key(path) {
                self.icons.insert(path.clone(), icon::load(path));
            }
        }
    }

    /// The shelf's Clear button, beside the tab bar: it fades in when the shelf is the open view and holds files.
    pub(crate) fn draw_shelf_clear(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let Some(font) = &self.font else { return };
        if self.view != View::Shelf || self.shelf.items.is_empty() {
            return;
        }
        let alpha = alpha * (self.switch_t as f32).clamp(0.0, 1.0);
        let a = (alpha * 255.0) as u8;
        let r = shelf_clear_rect(pm.height() as f32 / s);
        let hot = self.hot_t((r.0, r.1 + TAB_SHIFT, r.2, r.3), s);
        let (x, y, w, h) = (r.0 * s, r.1 * s, r.2 * s, r.3 * s);
        let red = [255, 96, 86];
        gfx::gradient_rounded(pm, x, y, w, h, h / 2.0, [red[0], red[1], red[2], (alpha * pick(80.0, 48.0, hot)) as u8], [red[0], red[1], red[2], (alpha * pick(46.0, 22.0, hot)) as u8], false);
        gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [red[0], red[1], red[2], (alpha * 90.0) as u8]);
        draw_text_centered(pm, font, "Clear", x + w / 2.0, y + 11.5 * s, 10.0 * s, [255, 130, 120, a], w);
    }

    pub(crate) fn draw_shelf(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let Some(font) = &self.font else { return };
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [165, 165, 175, a]);
        let width = EXPANDED.0 as f32 * s;
        let rgb = self.accent(View::Shelf);
        let heading = self.heading_font().unwrap_or(font);

        let empty = self.shelf.items.is_empty();
        let zone = (24.0, 36.0, 372.0, 124.0);
        if self.drag_hover {
            gfx::glow(pm, width / 2.0, 98.0 * s, 150.0 * s, [rgb[0], rgb[1], rgb[2], (alpha * 40.0) as u8], None);
            gfx::gradient_rounded(pm, zone.0 * s, zone.1 * s, zone.2 * s, zone.3 * s, 16.0 * s, [rgb[0], rgb[1], rgb[2], (alpha * 62.0) as u8], [rgb[0], rgb[1], rgb[2], (alpha * 26.0) as u8], false);
            gfx::stroke_rounded(pm, zone.0 * s, zone.1 * s, zone.2 * s, zone.3 * s, 16.0 * s, 1.5 * s, [rgb[0], rgb[1], rgb[2], (alpha * 170.0) as u8]);
        } else if empty {
            draw_card(pm, zone, 16.0, s, alpha, 0.0, rgb);
        }

        if empty {
            let ink = if self.drag_hover { white } else { [190, 195, 235, a] };
            draw_tab_icon(pm, View::Shelf, width / 2.0, 70.0 * s, s * 2.2, ink);
            let title = if self.drag_hover { "Release to add" } else { "Drop files here" };
            draw_text_centered(pm, heading, title, width / 2.0, 108.0 * s, 15.0 * s, white, width - 80.0 * s);
            let hint = "They stay until you drag them out or remove them";
            draw_text_centered(pm, font, hint, width / 2.0, 128.0 * s, 11.0 * s, grey, width - 80.0 * s);
            return;
        }

        const PALETTE: [[u8; 3]; 6] =
            [[255, 99, 71], [80, 200, 120], [90, 150, 255], [255, 190, 70], [190, 120, 255], [70, 200, 210]];
        for slot in 0..COLS * ROWS {
            self.animated_row(pm, s, slot / COLS, ROWS, |pm| {
                let Some(i) = self.shelf.index_at(slot) else { return };
                let path = &self.shelf.items[i];
                let rect = tile_rect(slot);
                let (x, y, w, h) = rect;
                let hot = self.hot_t(rect, s);
                draw_card(pm, rect, 10.0, s, alpha, hot, rgb);
                // A highlighted file glows in the accent colour; it eases in and out.
                let chosen = self.toggle_t(motion::rect_key(rect) >> 1, self.selected.contains(path)).clamp(0.0, 1.0);
                if chosen > 0.01 {
                    let tint = |v: f32| [rgb[0], rgb[1], rgb[2], (alpha * v * chosen) as u8];
                    gfx::gradient_rounded(pm, x * s, y * s, w * s, h * s, 10.0 * s, tint(90.0), tint(40.0), false);
                    gfx::stroke_rounded(pm, x * s + s, y * s + s, w * s - 2.0 * s, h * s - 2.0 * s, 10.0 * s, 1.5 * s, tint(230.0));
                }

                if let Some(Some(icon)) = self.icons.get(path) {
                    let size = 32.0 * s;
                    let scale = size / icon.width() as f32;
                    let paint = PixmapPaint { opacity: alpha, quality: FilterQuality::Bilinear, ..Default::default() };
                    let at = Transform::from_scale(scale, scale).post_translate((x + (w - 32.0) / 2.0) * s, (y + 7.0) * s);
                    pm.draw_pixmap(0, 0, icon.as_ref(), &paint, at, None);
                } else {
                    // No shell icon available: fall back to a colored extension badge.
                    let badge = shelf::badge(path);
                    let c = PALETTE[badge.bytes().map(usize::from).sum::<usize>() % PALETTE.len()];
                    let top = [c[0], c[1], c[2], a];
                    gfx::gradient_rounded(pm, (x + (w - 34.0) / 2.0) * s, (y + 8.0) * s, 34.0 * s, 18.0 * s, 5.0 * s, gfx::mix(top, white, 0.25), top, false);
                    draw_text_centered(pm, font, &badge, (x + w / 2.0) * s, (y + 21.0) * s, 10.0 * s, [0, 0, 0, a], 32.0 * s);
                }

                draw_text_centered(pm, font, &shelf::file_name(path), (x + w / 2.0) * s, (y + h - 10.0) * s, 10.0 * s, pick(white, [225, 225, 232, a], hot), (w - 8.0) * s);
                // Remove button (takes the file off the shelf, never deletes it).
                let over_x = self.hot_t((x + w - 16.0, y, 16.0, 16.0), s);
                if over_x > 0.01 {
                    fill_circle(pm, (x + w - 8.0) * s, (y + 9.0) * s, 7.0 * s, [255, 90, 80, (alpha * 70.0 * over_x) as u8]);
                }
                draw_text_centered(pm, font, "\u{00D7}", (x + w - 8.0) * s, (y + 13.0) * s, 13.0 * s, pick(white, grey, over_x), 12.0 * s);
            });
        }

        // Scroll indicator when there are more rows than fit.
        let total_rows = self.shelf.items.len().div_ceil(COLS);
        if total_rows > ROWS {
            let (top, height) = (TILES_ORIGIN.1, 122.0);
            fill_rounded(pm, 406.0 * s, top * s, 3.0 * s, height * s, 1.5 * s, [255, 255, 255, (alpha * 22.0) as u8]);
            let thumb = height * ROWS as f32 / total_rows as f32;
            let at = top + (height - thumb) * self.shelf.scroll as f32 / (total_rows - ROWS) as f32;
            fill_rounded(pm, 406.0 * s, at * s, 3.0 * s, thumb * s, 1.5 * s, [rgb[0], rgb[1], rgb[2], a]);
        }
    }

    /// A click in the Shelf view. (`cx`, `cy`) is the spot in the view's own coordinates.
    pub(crate) fn click_shelf(&mut self, cx: f32, cy: f32, window: &Window) {
        let hit = (0..COLS * ROWS).find_map(|slot| {
            let r = tile_rect(slot);
            let inside = cx >= r.0 && cx <= r.0 + r.2 && cy >= r.1 && cy <= r.1 + r.3;
            inside.then(|| self.shelf.index_at(slot).map(|i| (i, r))).flatten()
        });
        let Some((idx, r)) = hit else { return };
        if cx >= r.0 + r.2 - 16.0 && cy <= r.1 + 16.0 {
            self.shelf.remove(idx);
            window.request_redraw();
        } else if ctrl_down() {
            // Ctrl+click highlights or un-highlights one file without opening it.
            let path = self.shelf.items[idx].clone();
            match self.selected.iter().position(|p| *p == path) {
                Some(i) => {
                    self.selected.remove(i);
                }
                None => self.selected.push(path),
            }
            self.sync_selection_focus();
            window.request_redraw();
        } else {
            self.press = Some((idx, self.cursor));
        }
    }

    /// The shelf's Clear button sits beside the tab bar. True when it was pressed.
    pub(crate) fn click_shelf_clear(&mut self, cx: f32, ry: f32, bar_y: f32, window: &Window) -> bool {
        if self.view == View::Shelf && !self.shelf.items.is_empty() {
            let r = shelf_clear_rect(bar_y);
            if cx >= r.0 && cx <= r.0 + r.2 && ry >= r.1 && ry <= r.1 + r.3 {
                // Takes every file off the shelf; the files themselves are never touched.
                self.shelf.clear();
                self.selected.clear();
                self.sync_selection_focus();
                window.request_redraw();
                return true;
            }
        }
        false
    }
}
