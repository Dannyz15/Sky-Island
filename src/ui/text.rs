//! Fonts and text drawing.

use super::glyphs::{self, Script};
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

/// A character's measurements and its picture.
type Picture = std::rc::Rc<(fontdue::Metrics, Vec<u8>)>;

thread_local! {
    /// Characters of the main fonts already drawn into a picture, by font, character and size: a frame of the settings
    /// window draws hundreds of characters, and drawing each one again every frame was a good part of its cost.
    static RASTERS: std::cell::RefCell<std::collections::HashMap<(usize, char, u32), Picture>> = Default::default();
}

/// The most characters kept: far more than a few fonts at a few sizes use; past it they are made again as needed.
const MAX_RASTERS: usize = 6000;

/// `c` of `font` at `px` pixels: its measurements and its picture, made once.
fn raster(font: &Font, c: char, px: f32) -> Picture {
    RASTERS.with(|cache| {
        let mut cache = cache.borrow_mut();
        let key = (font.file_hash(), c, px.to_bits());
        if let Some(found) = cache.get(&key) {
            return found.clone();
        }
        if cache.len() >= MAX_RASTERS {
            cache.clear();
        }
        let made = std::rc::Rc::new(font.rasterize(c, px));
        cache.insert(key, made.clone());
        made
    })
}

/// The fonts one piece of text is drawn with: the main font, and for any character it lacks a Windows font of that
/// script (see `glyphs`). Made once per string, so the whole string is measured and drawn the same way.
pub(crate) struct Faces<'a> {
    main: &'a Font,
    /// The country whose shapes Chinese characters get in this string.
    han: Script,
    /// Whether the main font is a bold one, so the Windows fonts are asked for bold too.
    bold: bool,
}

impl<'a> Faces<'a> {
    pub(crate) fn new(main: &'a Font, text: &str) -> Self {
        let bold = main.name().is_some_and(|name| name.to_ascii_lowercase().contains("bold"));
        Faces { main, han: glyphs::han_for(text), bold }
    }

    /// Characters that only join or shape their neighbours (zero-width joiner, variation selectors, skin tones):
    /// not drawn, as the one-colour emoji have no use for them.
    fn hidden(c: char) -> bool {
        matches!(c as u32, 0x200D | 0xFE00..=0xFE0F | 0x1F3FB..=0x1F3FF)
    }

    fn glyph(&self, c: char, px: f32) -> Option<std::sync::Arc<glyphs::Glyph>> {
        self.system(c).and_then(|script| glyphs::glyph(script, c, px, self.bold))
    }

    /// The Windows font script that draws `c`, or `None` for the main font. Characters below U+0900 (Latin, Greek,
    /// Cyrillic, Hebrew, Arabic) are the main font's, and so are spaces and control and zero-width characters, so
    /// the common case costs nothing.
    fn system(&self, c: char) -> Option<Script> {
        if (c as u32) < 0x0900
            || c.is_whitespace()
            || c.is_control()
            || matches!(c as u32, 0x200B..=0x200F | 0x2028..=0x202F | 0xFE00..=0xFE0F | 0xFEFF)
            || self.main.lookup_glyph_index(c) != 0
        {
            return None;
        }
        glyphs::owner(c, self.han)
    }

    fn advance(&self, c: char, px: f32) -> f32 {
        if Self::hidden(c) {
            return 0.0;
        }
        match self.glyph(c, px) {
            Some(glyph) => glyph.advance,
            None => raster(self.main, c, px).0.advance_width,
        }
    }

    fn width(&self, text: &str, px: f32) -> f32 {
        text.chars().map(|c| self.advance(c, px)).sum()
    }
}

pub(crate) fn text_width(font: &Font, text: &str, px: f32) -> f32 {
    Faces::new(font, text).width(text, px)
}

/// The most characters one `draw_text` call looks at: far more than any row can show, so a huge string (a
/// media title from a web page, a lyric line) can never make one frame slow.
const MAX_DRAW_CHARS: usize = 1000;

/// `text` as it is drawn in `max_w`: unchanged when it fits, else its longest start that fits together with an
/// ellipsis. One pass over the characters (cutting one character at a time and measuring the whole text again
/// each time took time that grew with the square of the length).
#[cfg(test)]
fn fit(font: &Font, text: &str, px: f32, max_w: f32) -> String {
    let text = crate::lyrics::clip(text, MAX_DRAW_CHARS);
    fit_with(&Faces::new(font, text), text, px, max_w)
}

fn fit_with(faces: &Faces, text: &str, px: f32, max_w: f32) -> String {
    if faces.width(text, px) <= max_w {
        return text.to_string();
    }
    let ellipsis = faces.width("…", px);
    let (mut width, mut end) = (0.0_f32, 0);
    for (at, c) in text.char_indices() {
        let next = width + faces.advance(c, px);
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
    let text = crate::lyrics::clip(text, MAX_DRAW_CHARS);
    let faces = Faces::new(font, text);
    let s = fit_with(&faces, text, px, max_w);

    let (pw, ph) = (pm.width() as i32, pm.height() as i32);
    let data = pm.pixels_mut();
    // Lay one picture of ink (`w` by `h`, its top left at `gx`, `gy`) over the pixels.
    let mut put = |gx: i32, gy: i32, w: usize, h: usize, ink: &[u8]| {
        for row in 0..h {
            for col in 0..w {
                let cov = ink[row * w + col] as u32;
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
    };
    let mut pen = x;
    for c in s.chars() {
        if Faces::hidden(c) {
            continue;
        }
        if let Some(glyph) = faces.glyph(c, px) {
            put((pen + glyph.left as f32).round() as i32, (y - glyph.top as f32).round() as i32, glyph.width, glyph.height, &glyph.coverage);
            pen += glyph.advance;
        } else {
            let cached = raster(faces.main, c, px);
            let (m, bitmap) = (&cached.0, &cached.1);
            put((pen + m.xmin as f32).round() as i32, (y - m.height as f32 - m.ymin as f32).round() as i32, m.width, m.height, bitmap);
            pen += m.advance_width;
        }
    }
}

/// A character a line may break before: Japanese and Chinese have no spaces between words.
fn breaks_before(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F)
}

/// `word` cut into pieces a line may break between: the whole word, except that each Chinese character or kana
/// starts a new piece (with the marks that follow it, so a comma never starts a line).
fn pieces(word: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    for (at, c) in word.char_indices() {
        if at > start && breaks_before(c) {
            out.push(&word[start..at]);
            start = at;
        }
    }
    out.push(&word[start..]);
    out
}

/// Greedy word wrap, which also breaks between Japanese and Chinese characters. An empty line (instrumental gap)
/// becomes three dots (the system font has no music note).
pub(crate) fn wrap(font: &Font, text: &str, px: f32, max_w: f32) -> Vec<String> {
    let mut rows = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        for (n, piece) in pieces(word).into_iter().enumerate() {
            // The first piece of a word follows a space; the rest follow their neighbour directly.
            let candidate = if cur.is_empty() {
                piece.to_string()
            } else if n == 0 {
                format!("{cur} {piece}")
            } else {
                format!("{cur}{piece}")
            };
            if cur.is_empty() || text_width(font, &candidate, px) <= max_w {
                cur = candidate;
            } else {
                rows.push(std::mem::take(&mut cur));
                cur = piece.to_string();
            }
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

    /// Korean, Japanese, Chinese, Thai and Hindi text is drawn with real glyphs, not boxes (a script is skipped when
    /// this Windows has no font for it).
    #[test]
    fn other_writing_systems_are_drawn_with_windows_fonts() {
        let main = load_font().expect("a system font");
        let samples = [
            ("Korean", "블랙핑크 사랑해요"),
            ("Japanese", "こんにちは 世界 ありがとう"),
            ("Chinese", "这是中文歌词 我们"),
            ("Traditional Chinese", "這是繁體字"),
            ("Thai", "สวัสดี"),
            ("Hindi", "नमस्ते"),
            ("Symbols", "♪ ♫ → ★"),
            ("Emoji", "😀 🎵 ❤ 🚀"),
            ("Rare Chinese", "𠀀 𠮷"),
        ];
        let mut tested = 0;
        for (name, text) in samples {
            let faces = Faces::new(&main, text);
            for c in text.chars().filter(|c| !c.is_whitespace()) {
                let Some(script) = faces.system(c) else {
                    // Either the main font has it, or this Windows has no font for it.
                    assert!(main.lookup_glyph_index(c) != 0 || glyphs::owner(c, faces.han).is_none(), "{name}: {c:?}");
                    continue;
                };
                let glyph = glyphs::glyph(script, c, 17.0, false).unwrap_or_else(|| panic!("{name}: no picture for {c:?}"));
                // A combining mark (Thai vowels above a letter) has no advance of its own; every other letter does.
                assert!(glyph.advance > 3.0 || glyph.advance == 0.0, "{name}: {c:?} has an odd advance {}", glyph.advance);
                assert!(glyph.coverage.iter().any(|&v| v > 40), "{name}: {c:?} draws nothing");
                tested += 1;
            }
        }
        assert!(tested > 20, "only {tested} characters were drawn by Windows fonts");
    }

    /// An answer from Windows is kept: asking again gives the same picture.
    #[test]
    fn a_character_is_made_once() {
        let a = glyphs::glyph(Script::Korean, '\u{BE14}', 17.0, false);
        let b = glyphs::glyph(Script::Korean, '\u{BE14}', 17.0, false);
        if let (Some(a), Some(b)) = (a, b) {
            assert!(std::sync::Arc::ptr_eq(&a, &b));
        }
    }

    /// Text the main font can already draw never touches Windows fonts.
    #[test]
    fn latin_text_uses_the_main_font_only() {
        let main = load_font().expect("a system font");
        let text = "Hello, W\u{F6}rld \u{41F}\u{440}\u{438}\u{432}\u{435}\u{442} \u{3B5}\u{3BB}\u{3BB}\u{3B7}\u{3BD}\u{3B9}\u{3BA}\u{3AC} \u{200B}";
        let faces = Faces::new(&main, text);
        for c in text.chars() {
            assert!(faces.system(c).is_none(), "{c:?}");
        }
    }

    /// A bold title gets bold Korean, and joiners and variation selectors take no room.
    #[test]
    fn bold_titles_get_bold_windows_glyphs() {
        let (regular, bold) = (glyphs::glyph(Script::Korean, '\u{BE14}', 17.0, false), glyphs::glyph(Script::Korean, '\u{BE14}', 17.0, true));
        if let (Some(r), Some(b)) = (regular, bold) {
            let ink = |g: &glyphs::Glyph| g.coverage.iter().map(|&v| v as u32).sum::<u32>();
            assert!(ink(&b) > ink(&r), "bold {} is not heavier than regular {}", ink(&b), ink(&r));
        }
        let main = load_font().expect("a system font");
        let faces = Faces::new(&main, "x");
        assert_eq!(faces.advance('\u{200D}', 17.0), 0.0);
        assert_eq!(faces.advance('\u{FE0F}', 17.0), 0.0);
        let bold_main = load_bold_font().expect("a bold system font");
        assert!(Faces::new(&bold_main, "x").bold && !faces.bold);
    }

    #[test]
    fn long_japanese_and_chinese_lines_wrap_between_characters() {
        let font = load_font().expect("a system font");
        let line = "これは長い歌詞の一行で、スペースがありません。それでも行は折り返されるべきです";
        let rows = wrap(&font, line, 17.0, 120.0);
        assert!(rows.len() > 1, "{rows:?}");
        assert_eq!(rows.concat(), line, "nothing lost, no space added");
        for row in &rows {
            assert!(text_width(&font, row, 17.0) <= 120.0 || row.chars().count() == 1, "{row:?} is wider than its room");
            assert!(!row.starts_with('、') && !row.starts_with('。'), "a mark starts a line: {row:?}");
        }
        // Words that have spaces still break at the spaces and keep them.
        assert_eq!(wrap(&font, "one two three", 17.0, 2000.0), vec!["one two three"]);
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
