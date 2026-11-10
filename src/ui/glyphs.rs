//! Characters the main font lacks, drawn with Windows' own fonts through GDI.
//!
//! The main font (Segoe UI) has no Korean, Japanese or Chinese writing, no Thai, no Indian scripts and few symbols.
//! Reading a Windows CJK font into `fontdue` would cost 150 to 190 MB of memory for each font (it unpacks every
//! glyph), so instead Windows is asked for one character at a time (`GetGlyphOutlineW`) and each answer is kept,
//! a few hundred bytes a character. Text that the main font can draw never gets here. Emoji are drawn in one colour
//! (the text colour), and bold text asks for the bold weight of these fonts.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use windows::Win32::Graphics::Gdi::{
    ANTIALIASED_QUALITY, CLIP_DEFAULT_PRECIS, CreateCompatibleDC, CreateFontW, DEFAULT_CHARSET, DeleteObject, FIXED, GCP_GLYPHSHAPE, GCP_RESULTSW, GET_GLYPH_OUTLINE_FORMAT,
    GGO_GLYPH_INDEX, GGO_GRAY8_BITMAP, GLYPHMETRICS, GetCharacterPlacementW, GetGlyphIndicesW, GetGlyphOutlineW, GetTextFaceW, HDC, HFONT, HGDIOBJ, MAT2, OUT_DEFAULT_PRECIS,
    SelectObject,
};
use windows::core::{HSTRING, PWSTR};

/// A writing system with a Windows font of its own.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Script {
    Korean,
    Japanese,
    Chinese,
    TraditionalChinese,
    Thai,
    Indic,
    Symbols,
    /// Emoji, drawn in one colour (the colour of the text).
    Emoji,
    /// The rare Chinese characters beyond U+FFFF.
    HanExt,
}

impl Script {
    /// The Windows font names to try, best first.
    fn faces(self) -> &'static [&'static str] {
        match self {
            Script::Korean => &["Malgun Gothic"],
            Script::Japanese => &["Yu Gothic", "Meiryo", "MS Gothic"],
            Script::Chinese => &["Microsoft YaHei", "SimSun"],
            Script::TraditionalChinese => &["Microsoft JhengHei"],
            Script::Thai => &["Leelawadee UI", "Leelawadee"],
            Script::Indic => &["Nirmala UI"],
            Script::Symbols => &["Segoe UI Symbol"],
            Script::Emoji => &["Segoe UI Emoji"],
            Script::HanExt => &["SimSun-ExtB", "MingLiU-ExtB"],
        }
    }
}

/// Which country's shapes Chinese characters get in `text`: Japanese when it has kana, Korean when it has Hangul,
/// else Chinese. (The same character is drawn a little differently in each.)
pub(crate) fn han_for(text: &str) -> Script {
    let (mut kana, mut hangul) = (false, false);
    for c in text.chars() {
        match c as u32 {
            0x3040..=0x30FF => kana = true,
            0xAC00..=0xD7AF => hangul = true,
            _ => {}
        }
    }
    if kana {
        Script::Japanese
    } else if hangul {
        Script::Korean
    } else {
        Script::Chinese
    }
}

/// The fonts to try, in order, for a character the main font lacks.
fn chain(c: char, han: Script) -> &'static [Script] {
    use Script::*;
    match c as u32 {
        0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7FF => &[Korean],
        0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F => &[Japanese],
        0x0E00..=0x0E7F => &[Thai],
        0x0900..=0x0DFF => &[Indic],
        0x2E80..=0x2FDF | 0x3000..=0x303F | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF => match han {
            Japanese => &[Japanese, Chinese, TraditionalChinese, Korean],
            Korean => &[Korean, Chinese, TraditionalChinese, Japanese],
            _ => &[Chinese, TraditionalChinese, Japanese, Korean],
        },
        0x20000..=0x2FA1F => &[HanExt],
        0x1_0000.. => &[Emoji, Symbols],
        _ => &[Symbols, Emoji],
    }
}

/// One character as Windows draws it: a tight picture of how much ink covers each pixel, and where it sits.
pub(crate) struct Glyph {
    /// How far the pen moves on.
    pub advance: f32,
    /// Pixels from the pen to the picture's left edge.
    pub left: i32,
    /// Pixels from the baseline up to the picture's top edge.
    pub top: i32,
    pub width: usize,
    pub height: usize,
    /// `width * height` ink amounts, 0 to 255, row by row.
    pub coverage: Vec<u8>,
}

/// The device context and everything made with it. GDI handles are plain numbers that belong to the process, not
/// to a thread, and every use is behind the one lock below.
struct Gdi {
    dc: HDC,
    /// Per script, pixel size and weight (bold or not): the font Windows made, or `None` when the machine has none of
    /// its names.
    fonts: HashMap<(Script, i32, bool), Option<HFONT>>,
    /// Which script draws a character, for a given Chinese-character country.
    owners: HashMap<(Script, char), Option<Script>>,
    glyphs: HashMap<(Script, char, i32, bool), Option<Arc<Glyph>>>,
}

// SAFETY: `HDC` and `HFONT` are handles (numbers) that are valid on any thread, and `Gdi` is only ever used while
// holding the `STATE` lock, so no two threads touch the device context at once.
unsafe impl Send for Gdi {}

static STATE: Mutex<Option<Gdi>> = Mutex::new(None);

/// Enough for any lyric in any language, a few hundred bytes each; past it the saved answers are dropped and
/// built again as needed.
const MAX_GLYPHS: usize = 30_000;

/// The font of `script` at `px` pixels, selected into the device context.
fn font_for(gdi: &mut Gdi, script: Script, px: i32, bold: bool) -> Option<HFONT> {
    let dc = gdi.dc;
    let weight = if bold { 700 } else { 400 };
    let made = *gdi.fonts.entry((script, px, bold)).or_insert_with(|| {
        script.faces().iter().find_map(|name| {
            // SAFETY: plain GDI calls with a valid device context and a name that lives through the calls. Windows
            // silently gives another font for a name it does not have, so the name it picked is compared with the
            // one asked for; a font that is not wanted is deleted.
            unsafe {
                let font = CreateFontW(-px, 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, ANTIALIASED_QUALITY, 0, &HSTRING::from(*name));
                if font.is_invalid() {
                    return None;
                }
                let old = SelectObject(dc, HGDIOBJ(font.0));
                let mut got = [0u16; 64];
                let len = GetTextFaceW(dc, Some(&mut got)).clamp(0, 63) as usize;
                SelectObject(dc, old);
                if String::from_utf16_lossy(&got[..len]).trim_end_matches('\0').eq_ignore_ascii_case(name) {
                    Some(font)
                } else {
                    let _ = DeleteObject(HGDIOBJ(font.0));
                    None
                }
            }
        })
    });
    let font = made?;
    // SAFETY: `dc` and `font` are valid handles made above.
    unsafe { SelectObject(dc, HGDIOBJ(font.0)) };
    Some(font)
}

/// The glyph number of `c` in the font selected into `dc`, for a character beyond U+FFFF (which `GetGlyphIndicesW`
/// cannot take, as it reads one UTF-16 unit at a time); `None` when the font has no glyph for it.
fn wide_index(dc: HDC, c: char) -> Option<u32> {
    let mut units = [0u16; 2];
    let units = c.encode_utf16(&mut units);
    let mut glyphs = [0u16; 2];
    let mut results = GCP_RESULTSW { lStructSize: size_of::<GCP_RESULTSW>() as u32, lpGlyphs: PWSTR(glyphs.as_mut_ptr()), nGlyphs: 2, ..Default::default() };
    // SAFETY: `dc` holds the font; the text is the two units of one character, and `results` points at an output
    // array that has room for the two glyph numbers it was told about.
    let got = unsafe { GetCharacterPlacementW(dc, units, 0, &mut results, GCP_GLYPHSHAPE) };
    (got != 0 && results.nGlyphs >= 1 && glyphs[0] != 0 && glyphs[0] != 0xFFFF).then_some(glyphs[0] as u32)
}

/// Whether the font of `script` has a glyph for `c`.
fn has_glyph(gdi: &mut Gdi, script: Script, c: char) -> bool {
    if font_for(gdi, script, 16, false).is_none() {
        return false;
    }
    if c as u32 > 0xFFFF {
        return wide_index(gdi.dc, c).is_some();
    }
    let mut index = [0u16; 1];
    // SAFETY: the device context holds the font chosen just above; the text and the output array are valid for
    // the one character asked about. 1 is GGI_MARK_NONEXISTING_GLYPHS: 0xFFFF marks a missing glyph.
    let got = unsafe { GetGlyphIndicesW(gdi.dc, &HSTRING::from(c.to_string()), 1, index.as_mut_ptr(), 1) };
    got == 1 && index[0] != 0xFFFF
}

fn render(gdi: &mut Gdi, script: Script, c: char, px: i32, bold: bool) -> Option<Glyph> {
    font_for(gdi, script, px, bold)?;
    // A character beyond U+FFFF is asked for by its glyph number.
    let (code, format) = if c as u32 > 0xFFFF { (wide_index(gdi.dc, c)?, GET_GLYPH_OUTLINE_FORMAT(GGO_GRAY8_BITMAP.0 | GGO_GLYPH_INDEX.0)) } else { (c as u32, GGO_GRAY8_BITMAP) };
    let one = FIXED { fract: 0, value: 1 };
    let identity = MAT2 { eM11: one, eM12: FIXED::default(), eM21: FIXED::default(), eM22: one };
    let mut m = GLYPHMETRICS::default();
    // SAFETY: the device context holds the font; `m` and `identity` outlive the calls, and the buffer is as long
    // as the size Windows reported for this glyph.
    unsafe {
        let need = GetGlyphOutlineW(gdi.dc, code, format, &mut m, 0, None, &identity);
        if need == u32::MAX {
            return None;
        }
        let (width, height) = (m.gmBlackBoxX as usize, m.gmBlackBoxY as usize);
        let mut coverage = vec![0u8; width * height];
        if need > 0 {
            let mut raw = vec![0u8; need as usize];
            if GetGlyphOutlineW(gdi.dc, code, format, &mut m, need, Some(raw.as_mut_ptr().cast()), &identity) == u32::MAX {
                return None;
            }
            // Rows are padded to four bytes; the ink goes from 0 to 64.
            let pitch = (width + 3) & !3;
            for row in 0..height {
                for col in 0..width {
                    let ink = raw.get(row * pitch + col).copied().unwrap_or(0) as u32;
                    coverage[row * width + col] = (ink * 255 / 64).min(255) as u8;
                }
            }
        }
        Some(Glyph { advance: m.gmCellIncX as f32, left: m.gmptGlyphOrigin.x, top: m.gmptGlyphOrigin.y, width, height, coverage })
    }
}

fn with_gdi<T>(f: impl FnOnce(&mut Gdi) -> Option<T>) -> Option<T> {
    let mut state = STATE.lock().unwrap_or_else(PoisonError::into_inner);
    if state.is_none() {
        // SAFETY: a memory device context compatible with the screen; kept for the life of the process.
        let dc = unsafe { CreateCompatibleDC(None) };
        if dc.is_invalid() {
            return None;
        }
        *state = Some(Gdi { dc, fonts: HashMap::new(), owners: HashMap::new(), glyphs: HashMap::new() });
    }
    f(state.as_mut()?)
}

/// The script whose Windows font draws `c`, or `None` when none has it (the caller then shows the main font's box).
/// `han` is the country chosen for the string (see `han_for`).
pub(crate) fn owner(c: char, han: Script) -> Option<Script> {
    with_gdi(|gdi| {
        if let Some(known) = gdi.owners.get(&(han, c)) {
            return *known;
        }
        let found = chain(c, han).iter().copied().find(|&script| has_glyph(gdi, script, c));
        gdi.owners.insert((han, c), found);
        found
    })
}

/// `c` drawn by the font of `script` at about `px` pixels, in bold when `bold`.
pub(crate) fn glyph(script: Script, c: char, px: f32, bold: bool) -> Option<Arc<Glyph>> {
    let px = (px.round() as i32).clamp(4, 400);
    with_gdi(|gdi| {
        if let Some(known) = gdi.glyphs.get(&(script, c, px, bold)) {
            return known.clone();
        }
        if gdi.glyphs.len() >= MAX_GLYPHS {
            gdi.glyphs.clear();
        }
        let made = render(gdi, script, c, px, bold).map(Arc::new);
        gdi.glyphs.insert((script, c, px, bold), made.clone());
        made
    })
}
