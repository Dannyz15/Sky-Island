//! The Clipboard view.

use crate::*;

// Clipboard view, in logical pixels.
const CLIP_CLEAR: (f32, f32, f32, f32) = (324.0, 34.0, 72.0, 22.0);

const CLIP_ROWS_Y: f32 = 64.0;

const CLIP_ROW_H: f32 = 32.0;

pub(crate) const CLIP_TEAL: [u8; 3] = [70, 200, 210];

/// How long the "Copied" / "Cleared" message stays in the clipboard header.
const FLASH: Duration = Duration::from_millis(1400);

impl App {
    pub(crate) fn draw_clipboard(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let Some(font) = &self.font else { return };
        let heading = self.heading_font().unwrap_or(font);
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [165, 165, 175, a]);
        let rgb = CLIP_TEAL;
        let teal = [rgb[0], rgb[1], rgb[2], a];
        let width = EXPANDED.0 as f32 * s;

        // Header: what the list holds, or a short message after copying or clearing.
        let count = self.clips.items.len();
        let (title, color) = match self.clip_flash {
            Some((msg, _)) => (msg.to_string(), teal),
            None if !self.settings.clipboard => ("Clipboard history is off".to_string(), grey),
            None => (format!("Clipboard \u{00B7} {count} {}", if count == 1 { "item" } else { "items" }), gfx::mix(grey, white, 0.4)),
        };
        draw_text(pm, heading, &title, 24.0 * s, 51.0 * s, 12.5 * s, color, 280.0 * s);

        // Clear button: empties the history and the Windows clipboard.
        let clear = CLIP_CLEAR;
        let (x, y, w, h) = clear;
        let hot = self.hot_t(clear, s);
        let red = [255, 96, 86];
        gfx::gradient_rounded(pm, x * s, y * s, w * s, h * s, h * s / 2.0, [red[0], red[1], red[2], (alpha * pick(80.0, 48.0, hot)) as u8], [red[0], red[1], red[2], (alpha * pick(46.0, 22.0, hot)) as u8], false);
        gfx::stroke_rounded(pm, x * s + 0.5 * s, y * s + 0.5 * s, w * s - s, h * s - s, h * s / 2.0, s, [red[0], red[1], red[2], (alpha * 90.0) as u8]);
        draw_text_centered(pm, font, "Clear", (x + w / 2.0) * s, (y + 15.0) * s, 12.0 * s, [255, 130, 120, a], w * s);

        if !self.settings.clipboard || count == 0 {
            let (title, hint) = if self.settings.clipboard {
                ("Nothing copied yet", "Text and pictures you copy will show up here")
            } else {
                ("Clipboard history is off", "Turn it on in Settings")
            };
            gfx::glow(pm, width / 2.0, 100.0 * s, 70.0 * s, [rgb[0], rgb[1], rgb[2], (alpha * 34.0) as u8], None);
            draw_tab_icon(pm, View::Clipboard, width / 2.0, 92.0 * s, s * 2.4, teal);
            draw_text_centered(pm, heading, title, width / 2.0, 128.0 * s, 15.0 * s, white, width - 80.0 * s);
            draw_text_centered(pm, font, hint, width / 2.0, 147.0 * s, 11.0 * s, grey, width - 80.0 * s);
            return;
        }

        for row in 0..clipboard::VISIBLE {
            self.animated_row(pm, s, row, clipboard::VISIBLE, |pm| {
                let Some(i) = self.clips.index_at(row) else { return };
                let y = CLIP_ROWS_Y + row as f32 * CLIP_ROW_H;
                let rect = (24.0, y + 1.0, 372.0, CLIP_ROW_H - 2.0);
                let hot = self.hot_t(rect, s);
                draw_card(pm, rect, 8.0, s, alpha, hot, rgb);
                // A teal tab on the left edge marks the row under the mouse.
                if hot > 0.01 {
                    let tab = [teal[0], teal[1], teal[2], (teal[3] as f32 * hot) as u8];
                    gfx::gradient_rounded(pm, 24.0 * s, (y + 8.0) * s, 3.0 * s, (CLIP_ROW_H - 16.0) * s, 1.5 * s, tab, [rgb[0], rgb[1], rgb[2], 0], false);
                }
                match &self.clips.items[i] {
                    clipboard::Content::Text(text) => {
                        let baseline = (y + CLIP_ROW_H / 2.0 + 4.0) * s;
                        draw_text(pm, font, &clipboard::preview(text), 34.0 * s, baseline, 12.0 * s, white, 326.0 * s);
                    }
                    clipboard::Content::Image(img) => {
                        // The preview is stored at twice its drawn size, so it stays sharp on high-DPI screens.
                        if let Some(mut thumb) = Pixmap::new(img.thumb_w, img.thumb_h) {
                            for (dst, px) in thumb.pixels_mut().iter_mut().zip(img.thumb.as_chunks::<4>().0) {
                                *dst = tiny_skia::ColorU8::from_rgba(px[0], px[1], px[2], px[3]).premultiply();
                            }
                            let (dw, dh) = (img.thumb_w as f32 / 2.0, img.thumb_h as f32 / 2.0);
                            let (tx, ty) = (34.0 + (48.0 - dw) / 2.0, y + (CLIP_ROW_H - dh) / 2.0);
                            let paint = PixmapPaint { opacity: alpha, quality: FilterQuality::Bilinear, ..Default::default() };
                            let at = Transform::from_scale(s / 2.0, s / 2.0).post_translate(tx * s, ty * s);
                            let mask = gfx::rounded_path(tx * s, ty * s, dw * s, dh * s, 3.0 * s).and_then(|shape| gfx::mask_of(pm.width(), pm.height(), &shape));
                            pm.draw_pixmap(0, 0, thumb.as_ref(), &paint, at, mask.as_ref());
                            gfx::stroke_rounded(pm, tx * s, ty * s, dw * s, dh * s, 3.0 * s, s, [255, 255, 255, (alpha * 40.0) as u8]);
                        }
                        draw_text(pm, font, "Image", 94.0 * s, (y + 14.0) * s, 12.0 * s, white, 200.0 * s);
                        let size = format!("{} \u{00D7} {}", img.width, img.height);
                        draw_text(pm, font, &size, 94.0 * s, (y + 27.0) * s, 10.0 * s, grey, 200.0 * s);
                    }
                }
                let over_remove = self.hot_t((372.0, y, 24.0, CLIP_ROW_H), s);
                draw_text_centered(pm, font, "\u{00D7}", 384.0 * s, (y + CLIP_ROW_H / 2.0 + 5.0) * s, 13.0 * s, pick([255, 120, 110, a], grey, over_remove), 14.0 * s);
            });
        }

        // Scroll indicator when there are more entries than rows.
        if count > clipboard::VISIBLE {
            let (top, height) = (CLIP_ROWS_Y, CLIP_ROW_H * clipboard::VISIBLE as f32);
            fill_rounded(pm, 410.0 * s, top * s, 3.0 * s, height * s, 1.5 * s, [255, 255, 255, (alpha * 22.0) as u8]);
            let thumb = height * clipboard::VISIBLE as f32 / count as f32;
            let at = top + (height - thumb) * self.clips.scroll as f32 / (count - clipboard::VISIBLE) as f32;
            fill_rounded(pm, 410.0 * s, at * s, 3.0 * s, thumb * s, 1.5 * s, teal);
        }
    }

    /// A click in the Clipboard view. (`cx`, `cy`) is the spot in the view's own coordinates.
    pub(crate) fn click_clipboard(&mut self, cx: f32, cy: f32, window: &Window) {
        let inside = |r: (f32, f32, f32, f32)| cx >= r.0 && cx <= r.0 + r.2 && cy >= r.1 && cy <= r.1 + r.3;
        if inside(CLIP_CLEAR) {
            // Clears the history here and the Windows clipboard itself.
            self.clips.clear();
            clipboard::empty();
            self.clip_flash = Some(("Cleared", Instant::now() + FLASH));
        } else {
            let row = ((cy - CLIP_ROWS_Y) / CLIP_ROW_H).floor();
            let Some(idx) = (row >= 0.0).then(|| self.clips.index_at(row as usize)).flatten() else { return };
            if cx >= 372.0 {
                self.clips.remove(idx);
            } else if let Some(content) = self.clips.items.get(idx).cloned() {
                // The watcher will see this change and move the entry to the top.
                if clipboard::set_content(&content) {
                    self.clip_flash = Some(("Copied", Instant::now() + FLASH));
                }
            } else {
                return;
            }
        }
        window.request_redraw();
    }
}
