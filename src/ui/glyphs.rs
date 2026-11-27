//! Characters the main font lacks, drawn with Windows GDI or Linux fontconfig + FreeType.
//!
//! The main UI font lacks Korean, Japanese, Chinese, Thai, Indian scripts, and many symbols/emoji. Reading an entire
//! CJK font into `fontdue` costs ~150–190 MB per font (it unpacks every glyph), so instead the platform rasterizer
//! is asked for one character at a time and each answer is cached at a few hundred bytes per character. Text that the
//! main font can draw never gets here. Emoji are drawn in one colour (the text colour), and bold text asks for the bold
//! weight of these fonts.

#[cfg(any(windows, target_os = "linux"))]
use std::collections::HashMap;
use std::sync::Arc;
#[cfg(any(windows, target_os = "linux"))]
use std::sync::{Mutex, PoisonError};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    ANTIALIASED_QUALITY, CLIP_DEFAULT_PRECIS, CreateCompatibleDC, CreateFontW, DEFAULT_CHARSET, DeleteObject, FIXED, GCP_GLYPHSHAPE, GCP_RESULTSW, GET_GLYPH_OUTLINE_FORMAT,
    GGO_GLYPH_INDEX, GGO_GRAY8_BITMAP, GLYPHMETRICS, GetCharacterPlacementW, GetGlyphIndicesW, GetGlyphOutlineW, GetTextFaceW, HDC, HFONT, HGDIOBJ, MAT2, OUT_DEFAULT_PRECIS,
    SelectObject,
};
#[cfg(windows)]
use windows::core::{HSTRING, PWSTR};

/// A writing system with a system fallback font of its own.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
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

#[cfg(windows)]
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
#[cfg(any(windows, target_os = "linux"))]
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
#[cfg(windows)]
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
#[cfg(windows)]
unsafe impl Send for Gdi {}

#[cfg(windows)]
static STATE: Mutex<Option<Gdi>> = Mutex::new(None);

/// Enough for any lyric in any language, a few hundred bytes each; past it the saved answers are dropped and
/// built again as needed.
#[cfg(any(windows, target_os = "linux"))]
const MAX_GLYPHS: usize = 30_000;

/// The font of `script` at `px` pixels, selected into the device context.
#[cfg(windows)]
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
#[cfg(windows)]
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
#[cfg(windows)]
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

#[cfg(windows)]
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

#[cfg(windows)]
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

/// The script whose system font draws `c`, or `None` when none has it (the caller then shows the main font's box).
/// `han` is the country chosen for the string (see `han_for`).
pub(crate) fn owner(c: char, han: Script) -> Option<Script> {
    #[cfg(windows)]
    {
        with_gdi(|gdi| {
            if let Some(known) = gdi.owners.get(&(han, c)) {
                return *known;
            }
            let found = chain(c, han).iter().copied().find(|&script| has_glyph(gdi, script, c));
            gdi.owners.insert((han, c), found);
            found
        })
    }
    #[cfg(target_os = "linux")]
    {
        linux::owner(c, han)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (c, han);
        None
    }
}

/// `c` drawn by the font of `script` at about `px` pixels, in bold when `bold`.
pub(crate) fn glyph(script: Script, c: char, px: f32, bold: bool) -> Option<Arc<Glyph>> {
    #[cfg(windows)]
    {
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
    #[cfg(target_os = "linux")]
    {
        linux::glyph(script, c, px, bold)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (script, c, px, bold);
        None
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{Arc, Glyph, HashMap, MAX_GLYPHS, Mutex, PoisonError, Script, chain};
    use std::ffi::{CString, c_char, c_int, c_long, c_short, c_uint, c_ulong, c_ushort, c_void};
    use std::process::Command;

    const RTLD_LAZY: c_int = 0x1;
    const FT_FACE_FLAG_SCALABLE: c_long = 1;
    const FT_LOAD_RENDER: c_int = 1 << 2;
    const FT_LOAD_COLOR: c_int = 1 << 20;
    const FT_PIXEL_MODE_MONO: u8 = 1;
    const FT_PIXEL_MODE_GRAY: u8 = 2;
    const FT_PIXEL_MODE_BGRA: u8 = 7;

    unsafe extern "C" {
        fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    #[repr(C)]
    #[derive(Copy, Clone)]
    struct FtVector {
        x: c_long,
        y: c_long,
    }

    #[repr(C)]
    #[derive(Copy, Clone)]
    struct FtGeneric {
        data: *mut c_void,
        finalizer: *const c_void,
    }

    #[repr(C)]
    #[derive(Copy, Clone)]
    struct FtBBox {
        x_min: c_long,
        y_min: c_long,
        x_max: c_long,
        y_max: c_long,
    }

    #[repr(C)]
    #[derive(Copy, Clone)]
    struct FtBitmapSize {
        height: c_short,
        width: c_short,
        size: c_long,
        x_ppem: c_long,
        y_ppem: c_long,
    }

    #[repr(C)]
    #[derive(Copy, Clone)]
    struct FtGlyphMetrics {
        width: c_long,
        height: c_long,
        hori_bearing_x: c_long,
        hori_bearing_y: c_long,
        hori_advance: c_long,
        vert_bearing_x: c_long,
        vert_bearing_y: c_long,
        vert_advance: c_long,
    }

    #[repr(C)]
    #[derive(Copy, Clone)]
    struct FtBitmap {
        rows: c_uint,
        width: c_uint,
        pitch: c_int,
        buffer: *mut u8,
        num_grays: c_ushort,
        pixel_mode: u8,
        palette_mode: u8,
        palette: *mut c_void,
    }

    #[repr(C)]
    struct FtGlyphSlotRec {
        library: *mut c_void,
        face: *mut FtFaceRec,
        next: *mut FtGlyphSlotRec,
        glyph_index: c_uint,
        generic: FtGeneric,
        metrics: FtGlyphMetrics,
        linear_hori_advance: c_long,
        linear_vert_advance: c_long,
        advance: FtVector,
        format: c_uint,
        bitmap: FtBitmap,
        bitmap_left: c_int,
        bitmap_top: c_int,
    }

    #[repr(C)]
    struct FtFaceRec {
        num_faces: c_long,
        face_index: c_long,
        face_flags: c_long,
        style_flags: c_long,
        num_glyphs: c_long,
        family_name: *mut c_char,
        style_name: *mut c_char,
        num_fixed_sizes: c_int,
        available_sizes: *mut FtBitmapSize,
        num_charmaps: c_int,
        charmaps: *mut *mut c_void,
        generic: FtGeneric,
        bbox: FtBBox,
        units_per_em: c_ushort,
        ascender: c_short,
        descender: c_short,
        height: c_short,
        max_advance_width: c_short,
        max_advance_height: c_short,
        underline_position: c_short,
        underline_thickness: c_short,
        glyph: *mut FtGlyphSlotRec,
    }

    type FnInit = unsafe extern "C" fn(*mut *mut c_void) -> c_int;
    type FnNewFace = unsafe extern "C" fn(*mut c_void, *const c_char, c_long, *mut *mut FtFaceRec) -> c_int;
    type FnDoneFace = unsafe extern "C" fn(*mut FtFaceRec) -> c_int;
    type FnSetPixelSizes = unsafe extern "C" fn(*mut FtFaceRec, c_uint, c_uint) -> c_int;
    type FnSelectSize = unsafe extern "C" fn(*mut FtFaceRec, c_int) -> c_int;
    type FnGetCharIndex = unsafe extern "C" fn(*mut FtFaceRec, c_ulong) -> c_uint;
    type FnLoadGlyph = unsafe extern "C" fn(*mut FtFaceRec, c_uint, c_int) -> c_int;

    struct FtApi {
        _lib_handle: *mut c_void,
        ft_lib: *mut c_void,
        new_face: FnNewFace,
        done_face: FnDoneFace,
        set_pixel_sizes: FnSetPixelSizes,
        select_size: FnSelectSize,
        get_char_index: FnGetCharIndex,
        load_glyph: FnLoadGlyph,
    }

    struct FtState {
        api: FtApi,
        faces: HashMap<(Script, bool), Option<*mut FtFaceRec>>,
        owners: HashMap<(Script, char), Option<Script>>,
        glyphs: HashMap<(Script, char, i32, bool), Option<Arc<Glyph>>>,
    }

    // SAFETY: `FtState` and its FreeType handles are only ever accessed while holding the `STATE` mutex.
    unsafe impl Send for FtState {}

    static STATE: Mutex<Option<Option<FtState>>> = Mutex::new(None);

    unsafe fn sym<T>(handle: *mut c_void, name: &std::ffi::CStr) -> Option<T> {
        let ptr = unsafe { dlsym(handle, name.as_ptr()) };
        if ptr.is_null() { None } else { Some(unsafe { std::mem::transmute_copy::<*mut c_void, T>(&ptr) }) }
    }

    fn load_freetype() -> Option<FtApi> {
        unsafe {
            let handle = dlopen(c"libfreetype.so.6".as_ptr(), RTLD_LAZY);
            if handle.is_null() {
                return None;
            }
            let init: FnInit = sym(handle, c"FT_Init_FreeType")?;
            let new_face: FnNewFace = sym(handle, c"FT_New_Face")?;
            let done_face: FnDoneFace = sym(handle, c"FT_Done_Face")?;
            let set_pixel_sizes: FnSetPixelSizes = sym(handle, c"FT_Set_Pixel_Sizes")?;
            let select_size: FnSelectSize = sym(handle, c"FT_Select_Size")?;
            let get_char_index: FnGetCharIndex = sym(handle, c"FT_Get_Char_Index")?;
            let load_glyph: FnLoadGlyph = sym(handle, c"FT_Load_Glyph")?;

            let mut ft_lib = std::ptr::null_mut();
            if init(&mut ft_lib) != 0 || ft_lib.is_null() {
                return None;
            }
            Some(FtApi { _lib_handle: handle, ft_lib, new_face, done_face, set_pixel_sizes, select_size, get_char_index, load_glyph })
        }
    }

    fn with_ft<T>(f: impl FnOnce(&mut FtState) -> Option<T>) -> Option<T> {
        let mut guard = STATE.lock().unwrap_or_else(PoisonError::into_inner);
        let outer = guard.get_or_insert_with(|| load_freetype().map(|api| FtState { api, faces: HashMap::new(), owners: HashMap::new(), glyphs: HashMap::new() }));
        f(outer.as_mut()?)
    }

    fn script_patterns(script: Script) -> (char, &'static [&'static str]) {
        match script {
            Script::Korean => ('\u{BE14}', &[":charset=be14:lang=ko"]),
            Script::Japanese => ('\u{3042}', &[":charset=3042:lang=ja"]),
            Script::Chinese => ('\u{4E2D}', &[":charset=4e2d:lang=zh-cn"]),
            Script::TraditionalChinese => ('\u{4E2D}', &[":charset=4e2d:lang=zh-tw"]),
            Script::Thai => ('\u{0E2A}', &[":charset=0e2a:lang=th"]),
            Script::Indic => ('\u{0928}', &[":charset=0928:lang=hi"]),
            Script::Symbols => ('\u{2605}', &["Noto Sans Symbols2:charset=2605", "DejaVu Sans:charset=2605", ":charset=2605"]),
            Script::Emoji => ('\u{1F600}', &[":charset=1f600", "Noto Color Emoji", "Symbola"]),
            Script::HanExt => ('\u{20000}', &[":charset=20000:lang=zh-cn", ":charset=20000"]),
        }
    }

    pub(super) fn parse_fc_match_output(raw: &str) -> Option<(String, c_long)> {
        let line = raw.lines().find(|l| !l.trim().is_empty())?.trim();
        let (path, idx_str) = match line.rsplit_once('\t') {
            Some((p, i)) => (p.trim(), i.trim()),
            None => (line, "0"),
        };
        if path.is_empty() {
            return None;
        }
        let idx = idx_str.parse::<c_long>().unwrap_or(0);
        Some((path.to_string(), idx))
    }

    fn face_for(state: &mut FtState, script: Script, bold: bool) -> Option<*mut FtFaceRec> {
        if let Some(&cached) = state.faces.get(&(script, bold)) {
            return cached;
        }
        let (sample, patterns) = script_patterns(script);
        let mut loaded = None;
        for &pat in patterns {
            let query = if bold { format!("{pat}:weight=bold") } else { pat.to_string() };
            let Ok(out) = Command::new("fc-match").args(["-f", "%{file}\t%{index}\n", &query]).output() else {
                continue;
            };
            if !out.status.success() {
                continue;
            }
            let Some((path, index)) = parse_fc_match_output(&String::from_utf8_lossy(&out.stdout)) else {
                continue;
            };
            let Ok(c_path) = CString::new(path) else {
                continue;
            };
            let mut face = std::ptr::null_mut();
            // SAFETY: `ft_lib` and `c_path` are valid; `face` receives the new FT_Face handle on success.
            let rc = unsafe { (state.api.new_face)(state.api.ft_lib, c_path.as_ptr(), index, &mut face) };
            if rc != 0 || face.is_null() {
                continue;
            }
            // SAFETY: `face` was initialized by `FT_New_Face`.
            let has_sample = unsafe { (state.api.get_char_index)(face, sample as c_ulong) != 0 };
            if has_sample {
                loaded = Some(face);
                break;
            }
            unsafe {
                (state.api.done_face)(face);
            }
        }
        state.faces.insert((script, bold), loaded);
        loaded
    }

    fn has_glyph(state: &mut FtState, script: Script, c: char) -> bool {
        let Some(face) = face_for(state, script, false) else {
            return false;
        };
        // SAFETY: `face` is a live `FT_Face` owned by `FtState`.
        unsafe { (state.api.get_char_index)(face, c as c_ulong) != 0 }
    }

    pub(super) fn scale_coverage(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
        if dw == 0 || dh == 0 || sw == 0 || sh == 0 {
            return Vec::new();
        }
        let mut dst = vec![0u8; dw * dh];
        for dy in 0..dh {
            let sy0 = dy * sh / dh;
            let sy1 = ((dy + 1) * sh).div_ceil(dh).min(sh).max(sy0 + 1);
            for dx in 0..dw {
                let sx0 = dx * sw / dw;
                let sx1 = ((dx + 1) * sw).div_ceil(dw).min(sw).max(sx0 + 1);
                let mut sum = 0u32;
                let mut count = 0u32;
                for sy in sy0..sy1 {
                    for sx in sx0..sx1 {
                        sum += src[sy * sw + sx] as u32;
                        count += 1;
                    }
                }
                dst[dy * dw + dx] = (sum / count.max(1)).min(255) as u8;
            }
        }
        dst
    }

    fn extract_coverage(bitmap: &FtBitmap) -> Option<Vec<u8>> {
        let w = bitmap.width as usize;
        let h = bitmap.rows as usize;
        if w == 0 || h == 0 {
            return Some(Vec::new());
        }
        if bitmap.buffer.is_null() || w > 4096 || h > 4096 {
            return None;
        }
        let pitch_abs = bitmap.pitch.unsigned_abs() as usize;
        let mut coverage = vec![0u8; w * h];
        for row in 0..h {
            let src_row = if bitmap.pitch >= 0 { row } else { h - 1 - row };
            let row_offset = src_row * pitch_abs;
            match bitmap.pixel_mode {
                FT_PIXEL_MODE_GRAY => {
                    let divisor = (bitmap.num_grays.max(2) - 1) as u32;
                    for col in 0..w {
                        // SAFETY: `bitmap.buffer` holds at least `h * pitch_abs` bytes per FreeType guarantees.
                        let val = unsafe { *bitmap.buffer.add(row_offset + col) } as u32;
                        coverage[row * w + col] = if divisor == 255 { val as u8 } else { (val * 255 / divisor).min(255) as u8 };
                    }
                }
                FT_PIXEL_MODE_MONO => {
                    for col in 0..w {
                        // SAFETY: `col / 8 < pitch_abs` for MONO bitmaps.
                        let byte = unsafe { *bitmap.buffer.add(row_offset + (col / 8)) };
                        coverage[row * w + col] = if (byte >> (7 - (col % 8))) & 1 != 0 { 255 } else { 0 };
                    }
                }
                FT_PIXEL_MODE_BGRA => {
                    for col in 0..w {
                        // SAFETY: 4 bytes (B, G, R, A) per pixel for BGRA color emoji strikes.
                        coverage[row * w + col] = unsafe { *bitmap.buffer.add(row_offset + col * 4 + 3) };
                    }
                }
                _ => return None,
            }
        }
        Some(coverage)
    }

    fn render(state: &mut FtState, script: Script, c: char, px: i32, bold: bool) -> Option<Glyph> {
        let face = face_for(state, script, bold)?;
        unsafe {
            let idx = (state.api.get_char_index)(face, c as c_ulong);
            if idx == 0 {
                return None;
            }
            let face_ref = &*face;
            let scalable = (face_ref.face_flags & FT_FACE_FLAG_SCALABLE) != 0;
            let mut strike_px: Option<i32> = None;
            let load_flags = if scalable {
                if (state.api.set_pixel_sizes)(face, 0, px as c_uint) != 0 {
                    return None;
                }
                FT_LOAD_RENDER
            } else if face_ref.num_fixed_sizes > 0 && !face_ref.available_sizes.is_null() {
                let n = face_ref.num_fixed_sizes as usize;
                let sizes = std::slice::from_raw_parts(face_ref.available_sizes, n);
                let best = sizes
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, s)| {
                        let h = s.height as i32;
                        if h >= px { (0, h - px) } else { (1, px - h) }
                    })
                    .map(|(i, _)| i)?;
                if (state.api.select_size)(face, best as c_int) != 0 {
                    return None;
                }
                let sh = sizes[best].height as i32;
                if sh > 0 {
                    strike_px = Some(sh);
                }
                FT_LOAD_COLOR | FT_LOAD_RENDER
            } else {
                return None;
            };

            if (state.api.load_glyph)(face, idx, load_flags) != 0 {
                return None;
            }
            let slot = (*face).glyph;
            if slot.is_null() {
                return None;
            }
            let slot_ref = &*slot;
            let sw = slot_ref.bitmap.width as usize;
            let sh = slot_ref.bitmap.rows as usize;
            let coverage = extract_coverage(&slot_ref.bitmap)?;
            let advance = slot_ref.advance.x as f32 / 64.0;
            let left = slot_ref.bitmap_left;
            let top = slot_ref.bitmap_top;

            if let Some(strike_h) = strike_px
                && strike_h > px + 2
                && sw > 0
                && sh > 0
            {
                let scale = px as f32 / strike_h as f32;
                let dw = ((sw as f32 * scale).round() as usize).max(1);
                let dh = ((sh as f32 * scale).round() as usize).max(1);
                let scaled = scale_coverage(&coverage, sw, sh, dw, dh);
                return Some(Glyph {
                    advance: advance * scale,
                    left: (left as f32 * scale).round() as i32,
                    top: (top as f32 * scale).round() as i32,
                    width: dw,
                    height: dh,
                    coverage: scaled,
                });
            }

            Some(Glyph { advance, left, top, width: sw, height: sh, coverage })
        }
    }

    pub(super) fn owner(c: char, han: Script) -> Option<Script> {
        with_ft(|state| {
            if let Some(known) = state.owners.get(&(han, c)) {
                return *known;
            }
            let found = chain(c, han).iter().copied().find(|&script| has_glyph(state, script, c));
            state.owners.insert((han, c), found);
            found
        })
    }

    pub(super) fn glyph(script: Script, c: char, px: f32, bold: bool) -> Option<Arc<Glyph>> {
        let px = (px.round() as i32).clamp(4, 400);
        with_ft(|state| {
            if let Some(known) = state.glyphs.get(&(script, c, px, bold)) {
                return known.clone();
            }
            if state.glyphs.len() >= MAX_GLYPHS {
                state.glyphs.clear();
            }
            let made = render(state, script, c, px, bold).map(Arc::new);
            state.glyphs.insert((script, c, px, bold), made.clone());
            made
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn fc_match_parser_reads_path_and_index() {
            assert_eq!(parse_fc_match_output("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc\t1\n"), Some(("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc".into(), 1)));
            assert_eq!(parse_fc_match_output("/usr/share/fonts/noto/NotoSansThai-Regular.ttf\n"), Some(("/usr/share/fonts/noto/NotoSansThai-Regular.ttf".into(), 0)));
            assert_eq!(parse_fc_match_output("  \n"), None);
        }

        #[test]
        fn coverage_downscaler_averages_blocks() {
            let src = [
                100, 100, 200, 200, //
                100, 100, 200, 200, //
                0, 0, 50, 50, //
                0, 0, 50, 50, //
            ];
            assert_eq!(scale_coverage(&src, 4, 4, 2, 2), vec![100, 200, 0, 50]);
        }

        #[test]
        fn system_fallback_fonts_render_non_latin_and_emoji_glyphs() {
            let samples = [("Korean", "블랙핑크"), ("Japanese", "こんにちは世界"), ("Chinese", "这是中文"), ("Thai", "สวัสดี"), ("Hindi", "नमस्ते"), ("Emoji", "😀")];
            for (name, text) in samples {
                let han = super::super::han_for(text);
                for c in text.chars() {
                    let Some(script) = owner(c, han) else { continue };
                    let g = glyph(script, c, 17.0, false).unwrap_or_else(|| panic!("{name}: missing glyph for {c:?}"));
                    assert!(g.advance > 3.0 || g.advance == 0.0, "{name}: {c:?} odd advance {}", g.advance);
                    assert!(g.height <= 40, "{name}: {c:?} height {} too large for 17px", g.height);
                    assert!(g.coverage.iter().any(|&v| v > 40), "{name}: {c:?} empty ink");
                }
            }
        }
    }
}
