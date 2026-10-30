//! Fonts and text drawing.

use crate::*;

pub(crate) fn load_font() -> Option<Font> {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    for name in ["segoeui.ttf", "arial.ttf"] {
        if let Ok(bytes) = std::fs::read(format!("{windir}\\Fonts\\{name}"))
            && let Ok(font) = Font::from_bytes(bytes, FontSettings::default()) {
                return Some(font);
            }
    }
    None
}

pub(crate) fn load_bold_font() -> Option<Font> {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    for name in ["seguisb.ttf", "segoeuib.ttf"] {
        if let Ok(bytes) = std::fs::read(format!("{windir}\\Fonts\\{name}"))
            && let Ok(font) = Font::from_bytes(bytes, FontSettings::default()) {
                return Some(font);
            }
    }
    None
}

pub(crate) fn text_width(font: &Font, text: &str, px: f32) -> f32 {
    text.chars().map(|c| font.metrics(c, px).advance_width).sum()
}

/// Draw `text` with its baseline at `y`, truncating with an ellipsis past `max_w`.
pub(crate) fn draw_text(pm: &mut Pixmap, font: &Font, text: &str, x: f32, y: f32, px: f32, rgba: [u8; 4], max_w: f32) {
    let rgba = theme::map(rgba);
    let mut s = text.to_string();
    if text_width(font, &s, px) > max_w {
        while !s.is_empty() && text_width(font, &format!("{s}…"), px) > max_w {
            s.pop();
        }
        s.push('…');
    }

    let (pw, ph) = (pm.width() as i32, pm.height() as i32);
    let data = pm.pixels_mut();
    let mut pen = x;
    for c in s.chars() {
        let (m, bitmap) = font.rasterize(c, px);
        let gx = (pen + m.xmin as f32).round() as i32;
        let gy = (y - m.height as f32 - m.ymin as f32).round() as i32;
        for row in 0..m.height {
            for col in 0..m.width {
                let cov = bitmap[row * m.width + col] as u32;
                if cov == 0 {
                    continue;
                }
                let (dx, dy) = (gx + col as i32, gy + row as i32);
                if dx < 0 || dy < 0 || dx >= pw || dy >= ph {
                    continue;
                }
                let sa = cov * rgba[3] as u32 / 255;
                let dst = &mut data[(dy * pw + dx) as usize];
                let inv = 255 - sa;
                let blend = |src: u8, d: u8| ((src as u32 * sa + d as u32 * inv) / 255) as u8;
                *dst = tiny_skia::PremultipliedColorU8::from_rgba(
                    blend(rgba[0], dst.red()),
                    blend(rgba[1], dst.green()),
                    blend(rgba[2], dst.blue()),
                    (sa + dst.alpha() as u32 * inv / 255).min(255) as u8,
                )
                .unwrap_or(*dst);
            }
        }
        pen += m.advance_width;
    }
}

/// Greedy word wrap. An empty line (instrumental gap) becomes three dots (the system font has no music note).
pub(crate) fn wrap(font: &Font, text: &str, px: f32, max_w: f32) -> Vec<String> {
    let mut rows = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let candidate = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
        if cur.is_empty() || text_width(font, &candidate, px) <= max_w {
            cur = candidate;
        } else {
            rows.push(std::mem::take(&mut cur));
            cur = word.to_string();
        }
    }
    if !cur.is_empty() {
        rows.push(cur);
    }
    if rows.is_empty() {
        rows.push("\u{00B7} \u{00B7} \u{00B7}".to_string());
    }
    rows
}

pub(crate) fn draw_text_centered(pm: &mut Pixmap, font: &Font, text: &str, cx: f32, y: f32, px: f32, rgba: [u8; 4], max_w: f32) {
    let w = text_width(font, text, px).min(max_w);
    draw_text(pm, font, text, cx - w / 2.0, y, px, rgba, max_w);
}

/// The end of `text` that fits in `max_w`, so the caret stays visible while typing a long task.
pub(crate) fn tail_fit(font: &Font, text: &str, px: f32, max_w: f32) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut start = 0;
    while start < chars.len() && text_width(font, &chars[start..].iter().collect::<String>(), px) > max_w {
        start += 1;
    }
    chars[start..].iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every non-ASCII character the UI draws must exist in the loaded font, or it shows as a box.
    #[test]
    fn ui_glyphs_exist_in_font() {
        let font = load_font().expect("a system font");
        let missing: Vec<char> = "\u{00D7}\u{00B7}\u{2026}\u{2014}"
            .chars()
            .filter(|&c| font.lookup_glyph_index(c) == 0)
            .collect();
        assert!(missing.is_empty(), "missing glyphs: {missing:?}");
    }
}
