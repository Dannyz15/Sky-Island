//! Fonts and text drawing.

use crate::*;

pub(crate) fn load_font() -> Option<Font> {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    for name in ["segoeui.ttf", "arial.ttf"] {
        if let Ok(bytes) = std::fs::read(format!("{windir}\\Fonts\\{name}"))
            && let Ok(font) = Font::from_bytes(bytes, FontSettings::default())
        {
            return Some(font);
        }
    }
    None
}

pub(crate) fn load_bold_font() -> Option<Font> {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    for name in ["seguisb.ttf", "segoeuib.ttf"] {
        if let Ok(bytes) = std::fs::read(format!("{windir}\\Fonts\\{name}"))
            && let Ok(font) = Font::from_bytes(bytes, FontSettings::default())
        {
            return Some(font);
        }
    }
    None
}

pub(crate) fn text_width(font: &Font, text: &str, px: f32) -> f32 {
    text.chars().map(|c| font.metrics(c, px).advance_width).sum()
}

/// The most characters one `draw_text` call looks at: far more than any row can show, so a huge string (a
/// media title from a web page, a lyric line) can never make one frame slow.
const MAX_DRAW_CHARS: usize = 1000;

/// `text` as it is drawn in `max_w`: unchanged when it fits, else its longest start that fits together with an
/// ellipsis. One pass over the characters (cutting one character at a time and measuring the whole text again
/// each time took time that grew with the square of the length).
fn fit(font: &Font, text: &str, px: f32, max_w: f32) -> String {
    let text = crate::lyrics::clip(text, MAX_DRAW_CHARS);
    if text_width(font, text, px) <= max_w {
        return text.to_string();
    }
    let ellipsis = text_width(font, "…", px);
    let (mut width, mut end) = (0.0_f32, 0);
    for (at, c) in text.char_indices() {
        let next = width + font.metrics(c, px).advance_width;
        if next + ellipsis > max_w {
            break;
        }
        (width, end) = (next, at + c.len_utf8());
    }
    format!("{}…", &text[..end])
}

/// Draw `text` with its baseline at `y`, truncating with an ellipsis past `max_w`.
pub(crate) fn draw_text(pm: &mut Pixmap, font: &Font, text: &str, x: f32, y: f32, px: f32, rgba: [u8; 4], max_w: f32) {
    let rgba = theme::map(rgba);
    let s = fit(font, text, px, max_w);

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
        let missing: Vec<char> = "\u{00D7}\u{00B7}\u{2026}\u{2014}".chars().filter(|&c| font.lookup_glyph_index(c) == 0).collect();
        assert!(missing.is_empty(), "missing glyphs: {missing:?}");
    }

    /// The loop `fit` replaced, kept here to prove both give the same text.
    fn fit_by_popping(font: &Font, text: &str, px: f32, max_w: f32) -> String {
        let mut s = text.to_string();
        if text_width(font, &s, px) > max_w {
            while !s.is_empty() && text_width(font, &format!("{s}…"), px) > max_w {
                s.pop();
            }
            s.push('…');
        }
        s
    }

    #[test]
    fn the_one_pass_cut_gives_the_same_text_as_cutting_one_character_at_a_time() {
        let font = load_font().expect("a system font");
        let texts = [
            "",
            "Hi",
            "A fairly long song title that does not fit",
            "Artist \u{00B7} Album with a very long name that goes on and on",
            "\u{4E2D}\u{6587}\u{6B4C}\u{8BCD}\u{4E2D}\u{6587}\u{6B4C}\u{8BCD}\u{4E2D}\u{6587}\u{6B4C}\u{8BCD}",
            "WWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWW",
            "iiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiii",
        ];
        for text in texts {
            for px in [11.0, 15.0, 17.0, 24.0] {
                for max_w in [0.0, 5.0, 12.0, 40.0, 100.0, 250.0, 344.0, 2000.0] {
                    assert_eq!(fit(&font, text, px, max_w), fit_by_popping(&font, text, px, max_w), "{text:?} px {px} max_w {max_w}");
                }
            }
        }
    }

    #[test]
    fn a_huge_string_is_cut_to_a_small_one_quickly() {
        let font = load_font().expect("a system font");
        let started = std::time::Instant::now();
        let out = fit(&font, &"W".repeat(1_000_000), 17.0, 250.0);
        assert!(out.ends_with('…') && out.chars().count() < 40, "{out:?}");
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "took {:?}", started.elapsed());
        // Even zero-width characters are looked at only up to the limit.
        let zero = fit(&font, &"\u{200B}".repeat(5000), 17.0, 250.0);
        assert!(zero.chars().count() <= MAX_DRAW_CHARS);
    }
}
