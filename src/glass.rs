//! The notch's body for every look (Settings > Appearance > Notch look and Glass): drawn from one liquid outline, a
//! signed distance with smooth joins (the flares where it meets the top of the screen, a notice's drop, and the liquid
//! effects of Settings > Liquid: a drop running down as it opens, bubbles, a swell towards the mouse). With no glass it
//! is solid; as glass it shows, blurred and with richer colour, what is on the screen behind it (how see-through and
//! how frosted: the Transparency and Frost settings). Its curved edge bends what lies just outside it and its rim
//! catches the light (which follows the mouse).
//!
//! What is behind it: a thread copies the part of the screen around the notch a few times a second (GDI `BitBlt`; the
//! copy mostly waits for the screen's next refresh, so it costs little processor time). The notch's own window is in
//! that copy too, so the part it covers is shown as it was when it was last uncovered (the thread remembers what it has
//! seen), only what was never seen is filled in from around it, and then everything is blurred. The notch still shows
//! in screenshots and screen recordings (a window that hides itself from screen copies would show the real thing, but
//! would vanish from those too).

use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use tiny_skia::{Pixmap, PremultipliedColorU8};
use winit::event_loop::EventLoopProxy;

use crate::UserEvent;
use crate::settings::NotchLook;

/// A rectangle on the screen in screen pixels: left, top, width, height.
pub type Area = (i32, i32, i32, i32);

/// The gap above the floating pill, in logical pixels.
pub const FLOAT_GAP: f64 = 6.0;

/// How far around the notch the screen is copied, in screen pixels: the edge of the glass bends what lies there.
pub const MARGIN: i32 = 40;

/// How far a drop hangs below the closed notch at most while a notice makes it swell, in logical pixels.
pub const DROP: f64 = 11.0;

/// What the copying thread is asked for.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Want {
    /// The part of the screen to copy, or `None` to stop copying.
    pub area: Option<Area>,
    /// The notch's own window inside it, which is filled in from around it.
    pub hole: Area,
    /// Blur radius, screen pixels.
    pub blur: usize,
    /// How long to wait between copies.
    pub every: Duration,
    /// The look and whether the light theme is on: the thread gives the backdrop the glass's colour once, so a frame
    /// only has to copy it.
    pub look: NotchLook,
    pub light: bool,
    /// How see-through the glass is (the Transparency setting, 0 to 100; 50 is the glass as designed).
    pub clear: u8,
}

impl Default for Want {
    fn default() -> Self {
        Self { area: None, hole: (0, 0, 0, 0), blur: 8, every: Duration::from_millis(250), look: NotchLook::Classic, light: false, clear: 50 }
    }
}

/// What is behind the notch: a blurred copy of a part of the screen, kept at half the screen's resolution (it is
/// blurred anyway, and that is four times less work).
pub struct Backdrop {
    pub area: Area,
    /// Colours 0 to 1, row by row, one for each 2 x 2 screen pixels.
    px: Vec<[f32; 3]>,
    /// Its size in those cells.
    cols: i32,
    rows: i32,
    /// How light it is where the notch is, 0 to 1.
    pub lum: f32,
    /// Counts up with each new copy, so a picture made from it can tell it is out of date.
    pub version: u64,
    /// The cells with the glass's colour already given (the tab's colour and the sheen are added per row), and the look
    /// and theme that colour is for.
    shaded: Vec<[u8; 3]>,
    shaded_for: (NotchLook, bool, u8),
}

impl Backdrop {
    /// The colour of a cell (the nearest one inside the copy).
    fn cell(&self, cx: i32, cy: i32) -> [f32; 3] {
        self.px[(cy.clamp(0, self.rows - 1) * self.cols + cx.clamp(0, self.cols - 1)) as usize]
    }

    /// The colour at a point of the screen, blended from the four cells around it.
    fn sample(&self, x: f32, y: f32) -> [f32; 3] {
        let (fx, fy) = ((x - self.area.0 as f32) * 0.5 - 0.5, (y - self.area.1 as f32) * 0.5 - 0.5);
        // `floor` is a library call on the plain x86-64 target; for the sizes here this is the same and quick.
        let floor = |v: f32| {
            let i = v as i32;
            if (i as f32) > v { i - 1 } else { i }
        };
        let (x0, y0) = (floor(fx), floor(fy));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let (a, b, c, d) = if x0 >= 0 && y0 >= 0 && x0 + 1 < self.cols && y0 + 1 < self.rows {
            // Inside: four reads next to each other, no clamping.
            let i = (y0 * self.cols + x0) as usize;
            let row = self.cols as usize;
            (self.px[i], self.px[i + 1], self.px[i + row], self.px[i + row + 1])
        } else {
            (self.cell(x0, y0), self.cell(x0 + 1, y0), self.cell(x0, y0 + 1), self.cell(x0 + 1, y0 + 1))
        };
        let mut o = [0.0; 3];
        for k in 0..3 {
            o[k] = (a[k] * (1.0 - tx) + b[k] * tx) * (1.0 - ty) + (c[k] * (1.0 - tx) + d[k] * tx) * ty;
        }
        o
    }
}

/// The thread that keeps a fresh [`Backdrop`].
pub struct Watcher {
    want: Arc<Mutex<Want>>,
    latest: Arc<Mutex<Option<Arc<Backdrop>>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Watcher {
    pub fn spawn(proxy: EventLoopProxy<UserEvent>) -> Watcher {
        let want = Arc::new(Mutex::new(Want::default()));
        let latest = Arc::new(Mutex::new(None));
        let (want2, latest2) = (want.clone(), latest.clone());
        thread::spawn(move || {
            // The last copy, with the notch's own window blanked out: a copy that is the same again changes nothing.
            let mut last: Option<(Want, Vec<u8>)> = None;
            // What has been seen around the notch, for the part of the screen it covers now.
            let mut seen: Option<Seen> = None;
            let mut version = 0;
            loop {
                let want = *lock(&want2);
                let Some(area) = want.area.filter(|a| a.2 > 0 && a.3 > 0) else {
                    last = None;
                    *lock(&latest2) = None;
                    thread::sleep(Duration::from_millis(200));
                    continue;
                };
                if let Some(mut raw) = copy_screen(area) {
                    mark_known(&mut raw);
                    blank_hole(&mut raw, area, want.hole);
                    let same = last.as_ref().is_some_and(|(w, r)| *w == want && *r == raw);
                    if !same {
                        version += 1;
                        last = Some((want, raw.clone()));
                        // Under the notch: what was there when it was last uncovered.
                        remember(&mut seen, area, &mut raw);
                        let backdrop = build(area, want.hole, want.blur, &raw, version, (want.look, want.light, want.clear));
                        *lock(&latest2) = Some(Arc::new(backdrop));
                        if proxy.send_event(UserEvent::Backdrop).is_err() {
                            return;
                        }
                    }
                }
                thread::sleep(want.every);
            }
        });
        Watcher { want, latest }
    }

    /// Ask for another part of the screen, or another rate (only changes anything when it differs).
    pub fn ask(&self, want: Want) {
        let mut w = lock(&self.want);
        if *w != want {
            *w = want;
        }
    }

    pub fn latest(&self) -> Option<Arc<Backdrop>> {
        lock(&self.latest).clone()
    }
}

/// A copy of a part of the screen as BGRA bytes, row by row from the top.
fn copy_screen(area: Area) -> Option<Vec<u8>> {
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HGDIOBJ, ReleaseDC,
        SRCCOPY, SelectObject,
    };
    let (x, y, w, h) = area;
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return None;
        }
        let mem = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, HGDIOBJ(bitmap.0));
        let copied = BitBlt(mem, 0, 0, w, h, Some(screen), x, y, SRCCOPY).is_ok();
        SelectObject(mem, old);
        let mut bytes = vec![0u8; (w * h * 4) as usize];
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let rows = if copied { GetDIBits(mem, bitmap, 0, h as u32, Some(bytes.as_mut_ptr().cast()), &mut info, DIB_RGB_COLORS) } else { 0 };
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        (rows == h).then_some(bytes)
    }
}

/// The part of `area` that `hole` covers, as columns and rows of the copy: (x0, y0, x1, y1), possibly empty.
fn hole_in(area: Area, hole: Area) -> (usize, usize, usize, usize) {
    let x0 = (hole.0 - area.0).clamp(0, area.2) as usize;
    let y0 = (hole.1 - area.1).clamp(0, area.3) as usize;
    let x1 = (hole.0 + hole.2 - area.0).clamp(0, area.2) as usize;
    let y1 = (hole.1 + hole.3 - area.1).clamp(0, area.3) as usize;
    (x0, y0, x1.max(x0), y1.max(y0))
}

/// Mark every pixel of a fresh copy as seen (its alpha byte, which the copy leaves unset); `blank_hole` then marks the
/// notch's own pixels as not seen.
fn mark_known(raw: &mut [u8]) {
    for px in raw.as_chunks_mut::<4>().0 {
        px[3] = 255;
    }
}

/// What has been seen of the screen in the copied area, in its own pixels (alpha 255: seen at some time).
struct Seen {
    area: Area,
    px: Vec<u8>,
}

/// Keep what a copy shows outside the notch, and put back under the notch what was seen there before, when it was not
/// covered (the screen under the open notch, while it was closed). Only what was never seen is left to `fill_hole`.
fn remember(seen: &mut Option<Seen>, area: Area, raw: &mut [u8]) {
    let (w, h) = (area.2 as usize, area.3 as usize);
    // The copied area moves and grows with the notch: carry over what of it was seen before.
    if seen.as_ref().is_none_or(|s| s.area != area) {
        let mut px = vec![0u8; w * h * 4];
        if let Some(old) = seen.as_ref() {
            let (ow, oh) = (old.area.2 as usize, old.area.3 as usize);
            let (dx, dy) = (old.area.0 - area.0, old.area.1 - area.1);
            for oy in 0..oh {
                let y = oy as i32 + dy;
                if y < 0 || y >= h as i32 {
                    continue;
                }
                let (from, to) = ((-dx).max(0) as usize, ((w as i32 - dx).min(ow as i32)).max(0) as usize);
                if from >= to {
                    continue;
                }
                let x = (from as i32 + dx) as usize;
                px[(y as usize * w + x) * 4..(y as usize * w + x + to - from) * 4].copy_from_slice(&old.px[(oy * ow + from) * 4..(oy * ow + to) * 4]);
            }
        }
        *seen = Some(Seen { area, px });
    }
    let Some(seen) = seen.as_mut() else { return };
    for (now, kept) in raw.as_chunks_mut::<4>().0.iter_mut().zip(seen.px.as_chunks_mut::<4>().0) {
        if now[3] == 255 {
            *kept = *now;
        } else if kept[3] == 255 {
            *now = *kept;
        }
    }
}

/// Zero the notch's own pixels in a copy, so that what it draws does not count as a change behind it.
fn blank_hole(raw: &mut [u8], area: Area, hole: Area) {
    let (x0, y0, x1, y1) = hole_in(area, hole);
    let w = area.2 as usize;
    for y in y0..y1 {
        raw[(y * w + x0) * 4..(y * w + x1) * 4].fill(0);
    }
}

/// Turn a copy into a backdrop: halved, the notch's own window filled in from what is around it, then blurred.
fn build(area: Area, hole: Area, blur: usize, raw: &[u8], version: u64, shaded_for: (NotchLook, bool, u8)) -> Backdrop {
    let (sw, sh) = (area.2 as usize, area.3 as usize);
    let (w, h) = (sw.div_ceil(2), sh.div_ceil(2));
    // Each cell is the mean of its 2 x 2 screen pixels that were seen (fewer at the right and bottom edges); a cell with
    // none is unknown.
    let mut px = vec![[0.0f32; 3]; w * h];
    let mut known = vec![true; w * h];
    for (i, cell) in px.iter_mut().enumerate() {
        let (cx, cy) = (i % w, i / w);
        let (mut sum, mut n) = ([0.0f32; 3], 0.0);
        for y in cy * 2..(cy * 2 + 2).min(sh) {
            for x in cx * 2..(cx * 2 + 2).min(sw) {
                let p = &raw[(y * sw + x) * 4..(y * sw + x) * 4 + 4];
                if p[3] == 0 {
                    continue;
                }
                sum[0] += p[2] as f32;
                sum[1] += p[1] as f32;
                sum[2] += p[0] as f32;
                n += 1.0;
            }
        }
        if n > 0.0 {
            *cell = sum.map(|v| v / (255.0 * n));
        } else {
            known[i] = false;
        }
    }
    // The hole in cells: every cell that holds any pixel of it; what of it was never seen is filled in.
    let (x0, y0, x1, y1) = hole_in(area, hole);
    let (x0, y0, x1, y1) = (x0 / 2, y0 / 2, x1.div_ceil(2).min(w), y1.div_ceil(2).min(h));
    fill_unknown(&mut px, &known, w, h, (x0, y0, x1, y1));
    let px = blur_box(&px, w, h, blur.div_ceil(2));
    let lum = {
        let (mut sum, mut n) = (0.0, 0);
        for y in (y0..y1).step_by(2) {
            for x in (x0..x1).step_by(2) {
                let c = px[y * w + x];
                sum += 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
                n += 1;
            }
        }
        if n > 0 { sum / n as f32 } else { 0.0 }
    };
    let base = base_map(&tuned(shaded_for.0, shaded_for.1, shaded_for.2), shaded_for.1, lum);
    let shaded = px.iter().map(|c| base.apply(*c).map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)).collect();
    Backdrop { area, px, cols: w as i32, rows: h as i32, lum, version, shaded, shaded_for }
}

/// Fill a hole with what is around it (every cell of it, for the tests).
#[cfg(test)]
fn fill_hole(px: &mut [[f32; 3]], w: usize, h: usize, hole: (usize, usize, usize, usize)) {
    let (x0, y0, x1, y1) = hole;
    let mut known = vec![true; w * h];
    for y in y0..y1 {
        known[y * w + x0..y * w + x1].fill(false);
    }
    fill_unknown(px, &known, w, h, hole);
}

/// Fill the unknown cells inside `hole` with what is around them: across from the known cells left and right of each
/// unknown stretch of a row, blending lower down towards a wide, soft average of the row below the hole (a small bright
/// thing just below must not run up the glass as a stripe). Under a blur this passes for what is behind.
fn fill_unknown(px: &mut [[f32; 3]], known: &[bool], w: usize, h: usize, hole: (usize, usize, usize, usize)) {
    let (x0, y0, x1, y1) = hole;
    if x0 >= x1 || y0 >= y1 || known[y0 * w..y1 * w].iter().all(|k| *k) {
        return;
    }
    let mean = |cells: &mut dyn Iterator<Item = [f32; 3]>| {
        let (mut s, mut n) = ([0.0f32; 3], 0);
        for c in cells {
            for k in 0..3 {
                s[k] += c[k];
            }
            n += 1;
        }
        (n > 0).then(|| s.map(|v| v / n as f32))
    };
    // The row below the hole, averaged over a stretch as wide as the hole is tall on each side.
    let below = (y1 < h).then(|| {
        let line: Vec<[f32; 3]> = (0..w).map(|x| mean(&mut (y1..(y1 + 3).min(h)).map(|y| px[y * w + x])).unwrap_or([0.08, 0.08, 0.1])).collect();
        let r = (y1 - y0).max(2);
        (x0..x1).map(|x| mean(&mut (x.saturating_sub(r)..(x + r + 1).min(w)).map(|i| line[i])).unwrap_or([0.08, 0.08, 0.1])).collect::<Vec<_>>()
    });
    for y in y0..y1 {
        let down = ((y - y0) as f32 + 0.5) / (y1 - y0) as f32;
        let row = y * w;
        let mut x = x0;
        while x < x1 {
            if known[row + x] {
                x += 1;
                continue;
            }
            // A stretch of unknown cells, x..end.
            let start = x;
            while x < x1 && !known[row + x] {
                x += 1;
            }
            let end = x;
            let left = mean(&mut (start.saturating_sub(3)..start).filter(|&i| known[row + i]).map(|i| px[row + i]));
            let right = mean(&mut (end..(end + 3).min(w)).filter(|&i| known[row + i]).map(|i| px[row + i]));
            for i in start..end {
                let across = ((i - start) as f32 + 0.5) / (end - start) as f32;
                let side = match (left, right) {
                    (Some(l), Some(r)) => Some([0, 1, 2].map(|k| l[k] + (r[k] - l[k]) * across)),
                    (Some(l), None) => Some(l),
                    (None, Some(r)) => Some(r),
                    (None, None) => None,
                };
                let bottom = below.as_ref().map(|b| b[i - x0]);
                px[row + i] = match (side, bottom) {
                    (Some(s), Some(b)) => {
                        let t = down * down * 0.5;
                        [0, 1, 2].map(|k| s[k] + (b[k] - s[k]) * t)
                    }
                    (Some(s), None) => s,
                    (None, Some(b)) => b,
                    (None, None) => [0.08, 0.08, 0.1],
                };
            }
        }
    }
}

/// A blur close to a Gaussian one: three box blurs across and down.
fn blur_box(px: &[[f32; 3]], w: usize, h: usize, r: usize) -> Vec<[f32; 3]> {
    if r == 0 || w == 0 || h == 0 {
        return px.to_vec();
    }
    let mut cur = px.to_vec();
    for _ in 0..3 {
        cur = box_pass(&cur, w, h, r, true);
        cur = box_pass(&cur, w, h, r, false);
    }
    cur
}

fn box_pass(src: &[[f32; 3]], w: usize, h: usize, r: usize, across: bool) -> Vec<[f32; 3]> {
    let mut out = vec![[0.0; 3]; src.len()];
    let (len, lines) = if across { (w, h) } else { (h, w) };
    let at = |line: usize, i: usize| if across { line * w + i } else { i * w + line };
    let n = (2 * r + 1) as f32;
    for line in 0..lines {
        let mut sum = [0.0f32; 3];
        for i in 0..=2 * r {
            let c = src[at(line, i.saturating_sub(r).min(len - 1))];
            for k in 0..3 {
                sum[k] += c[k];
            }
        }
        for i in 0..len {
            out[at(line, i)] = sum.map(|v| v / n);
            let (add, sub) = (src[at(line, (i + r + 1).min(len - 1))], src[at(line, i.saturating_sub(r))]);
            for k in 0..3 {
                sum[k] += add[k] - sub[k];
            }
        }
    }
    out
}

/// How a look is drawn.
struct Params {
    /// Colour laid over the glass and how much of it (0 clear, 1 solid).
    tint: [f32; 3],
    tint_a: f32,
    saturate: f32,
    brighten: f32,
    /// How far the edge pulls in what is outside it, as a share of the bezel.
    refract: f32,
    /// Width of the curved edge, logical pixels.
    bezel: f32,
    rim: f32,
    /// The black (or white) liquid: a soft light across it.
    sheen: f32,
    /// Darker (or lighter) by itself over a light (or dark) background, so what is on it stays readable.
    adaptive: bool,
    /// How much of that darkening (or lightening) there is (1 as designed).
    adapt: f32,
}

/// A look's `params` with the Transparency setting (`clear`, 0 to 100) applied, for the glass that shows what is behind
/// it: 50 is the glass as designed; towards 100 its own colour and its darkening thin out until it is nearly clear;
/// towards 0 it turns milky, nearly solid.
fn tuned(look: NotchLook, light: bool, clear: u8) -> Params {
    let mut p = params(look, light);
    if wants_backdrop(look) {
        let k = clear.min(100) as f32 / 100.0;
        if k >= 0.5 {
            let thin = 1.0 - 1.8 * (k - 0.5);
            p.tint_a *= thin;
            p.adapt *= thin;
        } else {
            // Towards solid in the theme's own colour (dark, or light in the light theme), so what is on it stays
            // readable: clear glass's white tint would turn white text unreadable.
            let milky = 1.0 - 2.0 * k;
            let solid = if light { [0.96, 0.96, 0.98] } else { [0.07, 0.07, 0.09] };
            // The colour laid over it, weighted by how much of it there is, moved towards the solid one.
            let (a0, a1) = (p.tint_a, p.tint_a + (0.9 - p.tint_a) * milky);
            for (c, s) in p.tint.iter_mut().zip(solid) {
                *c = (*c * a0 * (1.0 - milky) + s * (a1 - a0 * (1.0 - milky))) / a1;
            }
            p.tint_a = a1;
        }
    }
    p
}

fn params(look: NotchLook, light: bool) -> Params {
    let (dark_tint, white) = ([0.03, 0.03, 0.05], [0.97, 0.97, 0.99]);
    match (look, light) {
        (NotchLook::Clear, false) => {
            Params { tint: [1.0; 3], tint_a: 0.05, saturate: 1.6, brighten: 1.05, refract: 0.9, bezel: 14.0, rim: 1.0, sheen: 0.0, adaptive: true, adapt: 1.0 }
        }
        (NotchLook::Clear, true) => {
            Params { tint: white, tint_a: 0.38, saturate: 1.5, brighten: 1.04, refract: 0.9, bezel: 14.0, rim: 0.8, sheen: 0.0, adaptive: true, adapt: 1.0 }
        }
        (NotchLook::Smoky, false) => {
            Params { tint: dark_tint, tint_a: 0.58, saturate: 1.5, brighten: 1.0, refract: 0.75, bezel: 14.0, rim: 0.9, sheen: 0.0, adaptive: true, adapt: 1.0 }
        }
        (NotchLook::Smoky, true) => {
            Params { tint: white, tint_a: 0.62, saturate: 1.4, brighten: 1.0, refract: 0.75, bezel: 14.0, rim: 0.7, sheen: 0.0, adaptive: true, adapt: 1.0 }
        }
        (NotchLook::Floating, false) => {
            Params { tint: [0.05, 0.05, 0.08], tint_a: 0.35, saturate: 1.6, brighten: 1.04, refract: 0.9, bezel: 16.0, rim: 1.0, sheen: 0.0, adaptive: true, adapt: 1.0 }
        }
        (NotchLook::Floating, true) => {
            Params { tint: white, tint_a: 0.5, saturate: 1.5, brighten: 1.02, refract: 0.9, bezel: 16.0, rim: 0.8, sheen: 0.0, adaptive: true, adapt: 1.0 }
        }
        // No glass: solid, with no light on the edge and nothing bent.
        (NotchLook::Classic, false) => {
            Params { tint: [0.0; 3], tint_a: 1.0, saturate: 1.0, brighten: 1.0, refract: 0.0, bezel: 14.0, rim: 0.0, sheen: 0.0, adaptive: false, adapt: 1.0 }
        }
        (NotchLook::Classic, true) => {
            Params { tint: [0.973, 0.973, 0.98], tint_a: 1.0, saturate: 1.0, brighten: 1.0, refract: 0.0, bezel: 14.0, rim: 0.0, sheen: 0.0, adaptive: false, adapt: 1.0 }
        }
        (NotchLook::FloatingSmoky, light) => Params { bezel: 16.0, ..params(NotchLook::Smoky, light) },
        (NotchLook::FloatingBlack, light) => Params { bezel: 16.0, ..params(NotchLook::Black, light) },
        // No glass: solid, with no light on the edge and nothing bent.
        (NotchLook::FloatingSolid, false) => {
            Params { tint: [0.0; 3], tint_a: 1.0, saturate: 1.0, brighten: 1.0, refract: 0.0, bezel: 16.0, rim: 0.0, sheen: 0.0, adaptive: false, adapt: 1.0 }
        }
        (NotchLook::FloatingSolid, true) => {
            Params { tint: [0.973, 0.973, 0.98], tint_a: 1.0, saturate: 1.0, brighten: 1.0, refract: 0.0, bezel: 16.0, rim: 0.0, sheen: 0.0, adaptive: false, adapt: 1.0 }
        }
        (_, false) => Params { tint: [0.0; 3], tint_a: 0.94, saturate: 1.0, brighten: 1.0, refract: 0.5, bezel: 12.0, rim: 0.85, sheen: 1.0, adaptive: false, adapt: 1.0 },
        (_, true) => Params { tint: white, tint_a: 0.94, saturate: 1.0, brighten: 1.0, refract: 0.5, bezel: 12.0, rim: 0.7, sheen: 1.0, adaptive: false, adapt: 1.0 },
    }
}

/// Whether a look shows what is behind it (and so needs the screen copied).
pub fn wants_backdrop(look: NotchLook) -> bool {
    matches!(look, NotchLook::Clear | NotchLook::Smoky | NotchLook::Floating | NotchLook::FloatingSmoky)
}

/// How strongly a look blurs what is behind it, in screen pixels at scale `s`, with the Frost setting (`frost`, 0 to
/// 100): 50 is the look as designed, 0 sharp, 100 twice as soft.
pub fn blur_px(look: NotchLook, s: f64, frost: u8) -> usize {
    let logical = match look {
        NotchLook::Clear => 7.0,
        NotchLook::Floating => 9.0,
        _ => 12.0,
    };
    (logical * s / 1.6 * frost.min(100) as f64 / 50.0).round() as usize
}

/// The gap above the notch in logical pixels: only the floating pill has one.
pub fn gap(look: NotchLook) -> f64 {
    if look.floating() { FLOAT_GAP } else { 0.0 }
}

fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
    b * (1.0 - h) + a * h - k * h * (1.0 - h)
}

/// Signed distance to a box with rounded corners (negative inside).
fn sd_box(px: f32, py: f32, c: (f32, f32), half: (f32, f32), r: f32) -> f32 {
    let (qx, qy) = ((px - c.0).abs() - half.0 + r, (py - c.1).abs() - half.1 + r);
    (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r
}

/// The liquid parts of the notch's outline, in pixels of the window. All zero or none: the plain shape.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Liquid {
    /// While it opens or closes: how far above the bottom of the shape the body ends; a drop runs down into that room.
    pub drip: f32,
    /// How much narrower than its room the body is (both sides together), with a round bubble at each end: how far
    /// their centres are from the middle and their radius. The music's bubbles, and a notice's bubbles running ahead.
    pub narrow: f32,
    pub bubbles: Option<(f32, f32)>,
    /// The bottom edge swelling towards the mouse: where (x) and how far below the edge it reaches.
    pub bulge: Option<(f32, f32)>,
}

impl Liquid {
    /// Whether the shape is still the same on both sides and its distance only falls towards the middle of each row
    /// (what lets `paint` search each row instead of looking at every pixel).
    fn simple(&self) -> bool {
        self.bubbles.is_none() && self.bulge.is_none()
    }
}

/// The outline of the glass in a window of `w` x `h` pixels, as a signed distance (negative inside).
pub struct Shape {
    centre: (f32, f32),
    half: (f32, f32),
    r: f32,
    /// How round the join with the top edge is; 0 for the floating pill, which has none.
    join: f32,
    /// Round blobs flowing into the body (centre, radius, how round the neck is): a notice's drop, the drop that runs
    /// down while it opens, bubbles and the bulge towards the mouse.
    blobs: Vec<((f32, f32), f32, f32)>,
    /// See `Liquid::simple`.
    simple: bool,
}

impl Shape {
    /// `ear` is the room on each side of the notch (the flares grow into it), `gap` the room above a floating pill,
    /// `stretch` 0 to 1 while the notch is moving fast (its corners grow rounder, like a liquid). `h` is the height of
    /// the notch itself; `drop` how far (pixels) a drop hangs below it, which the window has room for under it. `liquid`
    /// adds the drip, the bubbles and the bulge.
    pub fn new(w: f32, h: f32, s: f32, ear: f32, gap: f32, stretch: f32, drop: f32, liquid: &Liquid) -> Shape {
        let body_w = (w - 2.0 * ear - liquid.narrow).max(1.0);
        // The body ends higher while a drop runs down below it.
        let drip = liquid.drip.clamp(0.0, (h - gap) * 0.9);
        let bottom = h - drip;
        let mut blobs = Vec::new();
        // The drop: a round bead in the middle, as wide as about a third of the notch's height, pulled out below it
        // and joined to it by a soft neck.
        if drop > 0.5 {
            let r = ((bottom - gap) * 0.36).min(13.0 * s).max(2.0);
            blobs.push(((w / 2.0, bottom - r * 0.7 + drop), r, r * 1.1));
        }
        // The drop running down while it opens: as long as the room below the body, at most a quarter as wide as it.
        if drip > 0.5 {
            let r = (drip * 0.6).min(body_w * 0.22).min(44.0 * s).max(1.0);
            blobs.push(((w / 2.0, h - r), r, r * 1.4));
        }
        let mid_y = (gap + bottom) / 2.0;
        if let Some((off, r)) = liquid.bubbles {
            for x in [w / 2.0 - off, w / 2.0 + off] {
                blobs.push(((x, mid_y), r, 5.0 * s));
            }
        }
        if let Some((x, reach)) = liquid.bulge {
            let r = 10.0 * s;
            blobs.push(((x, bottom - r + reach), r, 8.0 * s));
        }
        let simple = liquid.simple();
        if gap > 0.0 {
            let bh = (bottom - gap).max(1.0);
            let r = (bh / 2.0).min(30.0 * s);
            return Shape { centre: (w / 2.0, gap + bh / 2.0), half: (body_w / 2.0, bh / 2.0), r, join: 0.0, blobs, simple };
        }
        // A box whose top is far above the screen, so its top corners never show, rounded at the bottom.
        const ABOVE: f32 = 400.0;
        let r = ((bottom / 2.0).min(28.0 * s) * (1.0 + 0.35 * stretch.clamp(0.0, 1.0))).min(bottom / 2.0).min(body_w / 2.0);
        Shape { centre: (w / 2.0, (bottom - ABOVE) / 2.0), half: (body_w / 2.0, (bottom + ABOVE) / 2.0), r, join: ear.max(1.0), blobs, simple }
    }

    pub fn sd(&self, x: f32, y: f32) -> f32 {
        let body = sd_box(x, y, self.centre, self.half, self.r);
        // Joined to the screen above the top edge with a smooth union: that join is the flares on its sides.
        let mut d = if self.join > 0.0 { smin(body, y, self.join) } else { body };
        for &(c, r, k) in &self.blobs {
            d = smin(d, ((x - c.0).powi(2) + (y - c.1).powi(2)).sqrt() - r, k);
        }
        d
    }

    /// The distance and the direction straight out of the glass at that point, worked out together.
    fn sd_normal(&self, x: f32, y: f32) -> (f32, (f32, f32)) {
        let (dx, dy) = (x - self.centre.0, y - self.centre.1);
        let (sx, sy) = (if dx < 0.0 { -1.0 } else { 1.0 }, if dy < 0.0 { -1.0 } else { 1.0 });
        let (qx, qy) = (dx.abs() - self.half.0 + self.r, dy.abs() - self.half.1 + self.r);
        let (body, grad) = if qx > 0.0 || qy > 0.0 {
            let (mx, my) = (qx.max(0.0), qy.max(0.0));
            let l = (mx * mx + my * my).sqrt();
            let d = l + qx.max(qy).min(0.0) - self.r;
            if l > 1e-6 { (d, (sx * mx / l, sy * my / l)) } else { (d, (0.0, sy)) }
        } else if qx > qy {
            (qx - self.r, (sx, 0.0))
        } else {
            (qy - self.r, (0.0, sy))
        };
        // A smooth union's slope is the two slopes mixed by how much each counts there.
        let union = |a: f32, ga: (f32, f32), b: f32, gb: (f32, f32), k: f32| {
            let h = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
            let d = b * (1.0 - h) + a * h - k * h * (1.0 - h);
            let (gx, gy) = (ga.0 * h + gb.0 * (1.0 - h), ga.1 * h + gb.1 * (1.0 - h));
            let l = (gx * gx + gy * gy).sqrt().max(1e-6);
            (d, (gx / l, gy / l))
        };
        // The screen above the top edge, whose slope is straight down.
        let (mut d, mut grad) = if self.join > 0.0 { union(body, grad, y, (0.0, 1.0), self.join) } else { (body, grad) };
        for &(c, r, k) in &self.blobs {
            let (dx, dy) = (x - c.0, y - c.1);
            let l = (dx * dx + dy * dy).sqrt().max(1e-6);
            (d, grad) = union(d, grad, l - r, (dx / l, dy / l), k);
        }
        (d, grad)
    }
}

/// Everything the glass needs to be drawn.
pub struct Glass<'a> {
    pub look: NotchLook,
    pub light: bool,
    pub backdrop: Option<&'a Backdrop>,
    /// Where the window's top-left corner is on the screen, in screen pixels.
    pub origin: (f32, f32),
    pub s: f32,
    pub ear: f32,
    pub gap: f32,
    pub stretch: f32,
    /// The direction the light comes from (towards the mouse while it is over the notch).
    pub sun: (f32, f32),
    /// The tab's colour rising from the bottom while open, and how much of it.
    pub wash: Option<([u8; 3], f32)>,
    /// The room below the notch in the window (pixels) and how far a drop hangs into it.
    pub room: f32,
    pub drop: f32,
    /// The liquid parts of the outline (in pixels).
    pub liquid: Liquid,
    /// How see-through the glass is (the Transparency setting, 0 to 100).
    pub clear: u8,
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A linear map of colours: `a` x colour + `b`.
#[derive(Clone, Copy)]
struct Map {
    a: [[f32; 3]; 3],
    b: [f32; 3],
}

impl Map {
    fn apply(&self, c: [f32; 3]) -> [f32; 3] {
        let m = &self.a;
        [0, 1, 2].map(|k| m[k][0] * c[0] + m[k][1] * c[1] + m[k][2] * c[2] + self.b[k])
    }

    /// This map followed by: times `scale`, plus `add`.
    fn then(mut self, scale: f32, add: [f32; 3]) -> Map {
        for ((row, b), add) in self.a.iter_mut().zip(&mut self.b).zip(add) {
            for v in row {
                *v *= scale;
            }
            *b = *b * scale + add;
        }
        self
    }
}

/// How much a glass darkens itself over a light background (or a light glass lightens over a dark one), 0 to 1, for
/// a background of lightness `lum`.
fn adapt_for(p: &Params, light: bool, lum: f32) -> f32 {
    if !p.adaptive {
        0.0
    } else if light {
        0.7 * p.adapt * smoothstep(0.7, 0.25, lum)
    } else {
        0.55 * p.adapt * smoothstep(0.3, 0.75, lum)
    }
}

/// What the glass does to the colour behind it everywhere (richer colour and a little lighter, the tint, and the
/// darkening or lightening over the background), as one linear map.
fn base_map(p: &Params, light: bool, lum: f32) -> Map {
    const LUM: [f32; 3] = [0.2126, 0.7152, 0.0722];
    let mut a = [[0.0f32; 3]; 3];
    for (k, row) in a.iter_mut().enumerate() {
        for (j, w) in LUM.iter().enumerate() {
            row[j] = p.brighten * ((if j == k { p.saturate } else { 0.0 }) + (1.0 - p.saturate) * w);
        }
    }
    let adapt = adapt_for(p, light, lum);
    let map = Map { a, b: [0.0; 3] }.then(1.0 - p.tint_a, p.tint.map(|v| v * p.tint_a));
    if light { map.then(1.0 - adapt, [adapt; 3]) } else { map.then(1.0 - adapt, [0.0; 3]) }
}

/// Draw the glass body on `pm`, the whole window (flares included); what is outside the glass is left transparent.
///
/// Only the band along the edge needs the bending, the rim and the outline worked out; the rest of a row is the
/// backdrop with the glass's colour. Each row is searched for where the glass starts and where its plain middle
/// starts (the shape is the same on both sides, and its distance only falls towards the middle), so the far larger
/// middle costs one lookup and a little arithmetic a pixel, and the outside costs nothing.
pub fn paint(pm: &mut Pixmap, g: &Glass) {
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    let (fw, fh) = (w as f32, h as f32);
    let p = tuned(g.look, g.light, g.clear);
    let s = g.s;
    let shape = Shape::new(fw, fh - g.room, s, g.ear, g.gap, g.stretch, g.drop, &g.liquid);
    let bezel = p.bezel * s;
    let fallback = if g.light { [0.9, 0.9, 0.93] } else { [0.07, 0.07, 0.09] };
    // Over a light background a dark glass darkens itself (and a light glass lightens over a dark one).
    let lum = g.backdrop.map_or(if g.light { 1.0 } else { 0.0 }, |b| b.lum);
    let base = base_map(&p, g.light, lum);
    let sun = {
        let l = (g.sun.0 * g.sun.0 + g.sun.1 * g.sun.1).sqrt().max(1e-6);
        (g.sun.0 / l, g.sun.1 / l)
    };
    let wash = g.wash.map(|(c, amount)| ([c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0], amount));
    // Everything the glass does to the colour behind it is linear, so for a row it is one map: new = A x old + b. On top
    // of the base map: the liquid's sheen (a soft light from above across its top half and a lighter floor) and the
    // tab's colour rising from the bottom, which change down the notch. Gives the map, the sheen and how much tab colour.
    let row_map = |fy: f32| -> (Map, f32, f32) {
        let v = (fy / fh).clamp(0.0, 1.0);
        let lift = if p.sheen > 0.0 { 0.05 * (1.0 - v).powi(2) + 0.035 * ((v - 0.55) / 0.45).max(0.0).powi(2) } else { 0.0 };
        let lift = if g.light { -lift * 0.6 } else { lift };
        let mut map = base.then(1.0, [lift; 3]);
        let mut wash_t = 0.0;
        if let Some((c, amount)) = wash {
            wash_t = amount * 0.16 * smoothstep(0.35, 1.05, v);
            map = map.then(1.0 - wash_t, c.map(|v| v * wash_t));
        }
        (map, lift, wash_t)
    };
    let behind = |x: f32, y: f32| g.backdrop.map_or(fallback, |b| b.sample(g.origin.0 + x, g.origin.1 + y));
    let to_pixel = |c: [f32; 3], a: f32| {
        let to = |v: f32| (v.clamp(0.0, 1.0) * a * 255.0 + 0.5) as u8;
        PremultipliedColorU8::from_rgba(to(c[0]), to(c[1]), to(c[2]), (a * 255.0 + 0.5) as u8).unwrap_or(PremultipliedColorU8::TRANSPARENT)
    };
    // How the light falls off going into the glass, looked up by depth (in quarter pixels) instead of worked out.
    let depth_cells = ((bezel + 2.0) * 4.0) as usize + 2;
    let falloff = |scale: f32| -> Vec<f32> { (0..depth_cells).map(|i| (-(i as f32 / 4.0) / scale).exp()).collect() };
    let (near_t, band_t, ao_t, outline_t) = (falloff(1.1 * s), falloff(bezel * 0.45), falloff(3.0 * s), falloff(0.6 * s));
    // A pixel of the band along the edge: the bending, the rim and the soft outline.
    let edge = |x: usize, y: usize, map: &Map, rim_fade: f32| -> PremultipliedColorU8 {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let (d, n) = shape.sd_normal(fx, fy);
        let cover = (0.5 - d).clamp(0.0, 1.0);
        if cover <= 0.0 {
            return PremultipliedColorU8::TRANSPARENT;
        }
        let inside = (-d).max(0.0);
        let at = ((inside * 4.0) as usize).min(depth_cells - 1);
        // The curved edge pulls in what lies just outside it, red, green and blue a little apart like a prism.
        let t = (1.0 - inside / bezel).max(0.0);
        let disp = p.refract * bezel * t * t;
        // The colours come apart where the bending is strong: red from where it bends to, blue from a little further
        // and green between them. Deeper in, where it bends less than half a pixel, the backdrop is read as it is.
        let c = if disp > 1.5 {
            let (near_side, far_side) = (behind(fx + n.0 * disp, fy + n.1 * disp), behind(fx + n.0 * disp * 1.14, fy + n.1 * disp * 1.14));
            [near_side[0], (near_side[1] + far_side[1]) * 0.5, far_side[2]]
        } else if disp > 0.5 {
            behind(fx + n.0 * disp, fy + n.1 * disp)
        } else {
            behind(fx, fy)
        };
        let mut c = map.apply(c);
        // Thick glass: a bright rim where the light falls on it, a fainter one opposite, darker just inside.
        let (near, band) = (near_t[at], band_t[at]);
        let facing = (n.0 * sun.0 + n.1 * sun.1).max(0.0);
        let back = (-(n.0 * sun.0 + n.1 * sun.1)).max(0.0);
        let shine = rim_fade * p.rim * (0.30 * near + 0.75 * near * facing * facing.sqrt() + 0.40 * near * back * back + 0.10 * band * facing);
        let ao = rim_fade * 0.12 * ao_t[at] * (1.0 - near);
        for v in &mut c {
            *v *= 1.0 - ao;
            *v += (1.0 - *v) * shine;
        }
        if g.light {
            // A thin darker outline, so a light glass still stands out on a light background.
            let outline = rim_fade * 0.22 * outline_t[at];
            for v in &mut c {
                *v *= 1.0 - outline;
            }
        }
        to_pixel(c, cover)
    };
    let half = w / 2;
    // The first column from the left (up to the middle) where the distance is below `limit`.
    let first_below = |fy: f32, limit: f32| -> usize {
        let (mut lo, mut hi) = (0, half);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if shape.sd(mid as f32 + 0.5, fy) < limit {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        lo
    };
    // One row of the picture.
    let paint_row = |y: usize, row: &mut [PremultipliedColorU8]| {
        let fy = y as f32 + 0.5;
        if !shape.simple {
            // Bubbles and the bulge: the row cannot be searched, so each pixel is looked at (only the small closed notch
            // has them).
            let (map, _, _) = row_map(fy);
            let rim_fade = if g.gap > 0.0 { 1.0 } else { (fy / (6.0 * s)).clamp(0.0, 1.0) };
            for (x, cell) in row.iter_mut().enumerate() {
                if shape.sd(x as f32 + 0.5, fy) < 0.5 {
                    *cell = edge(x, y, &map, rim_fade);
                }
            }
            return;
        }
        let middle = shape.sd(fw / 2.0, fy);
        if middle >= 0.5 {
            return;
        }
        let (map, lift, wash_t) = row_map(fy);
        // The rim fades out where the glass meets the top of the screen (there is no edge there).
        let rim_fade = if g.gap > 0.0 { 1.0 } else { (fy / (6.0 * s)).clamp(0.0, 1.0) };
        let start = first_below(fy, 0.5);
        let plain = middle < -(bezel + 1.0);
        let inner = if plain { first_below(fy, -(bezel + 1.0)).max(start) } else { half };
        for x in start..inner {
            row[x] = edge(x, y, &map, rim_fade);
            row[w - 1 - x] = edge(w - 1 - x, y, &map, rim_fade);
        }
        if !plain {
            for (x, cell) in row.iter_mut().enumerate().take(w - inner).skip(inner) {
                *cell = edge(x, y, &map, rim_fade);
            }
            return;
        }
        // The plain middle: the backdrop's cells are two pixels wide, so each colour is worked out once for both.
        match g.backdrop {
            Some(b) if b.shaded_for == (g.look, g.light, g.clear) => {
                // The cells already have the glass's colour: add the sheen and the tab's colour, and copy.
                let (ox, cy) = (g.origin.0 as i32 - b.area.0, ((g.origin.1 as i32 + y as i32 - b.area.1) >> 1).clamp(0, b.rows - 1));
                let cells = &b.shaded[(cy * b.cols) as usize..((cy + 1) * b.cols) as usize];
                let tab = wash.map_or([0.0; 3], |(c, _)| c.map(|v| v * 255.0 * wash_t));
                let (keep, add) = (1.0 - wash_t, lift * 255.0);
                let mut last = (i32::MIN, PremultipliedColorU8::TRANSPARENT);
                for (x, cell) in row.iter_mut().enumerate().take(w - inner).skip(inner) {
                    let cx = ((ox + x as i32) >> 1).clamp(0, b.cols - 1);
                    if cx != last.0 {
                        let c = cells[cx as usize];
                        let ch = |k: usize| (c[k] as f32 * keep + tab[k] + add).clamp(0.0, 255.0) as u8;
                        last = (cx, PremultipliedColorU8::from_rgba(ch(0), ch(1), ch(2), 255).unwrap_or(PremultipliedColorU8::TRANSPARENT));
                    }
                    *cell = last.1;
                }
            }
            Some(b) => {
                // Coloured for another look (a new copy is on its way): work the colour out here.
                let (ox, cy) = (g.origin.0 as i32 - b.area.0, (g.origin.1 as i32 + y as i32 - b.area.1) >> 1);
                let mut last = (i32::MIN, PremultipliedColorU8::TRANSPARENT);
                for (x, cell) in row.iter_mut().enumerate().take(w - inner).skip(inner) {
                    let cx = (ox + x as i32) >> 1;
                    if cx != last.0 {
                        last = (cx, to_pixel(map.apply(b.cell(cx, cy)), 1.0));
                    }
                    *cell = last.1;
                }
            }
            None => row[inner..w - inner].fill(to_pixel(map.apply(fallback), 1.0)),
        }
    };
    pm.fill(tiny_skia::Color::TRANSPARENT);
    let rows = pm.pixels_mut().chunks_mut(w).enumerate();
    // A big picture (the open notch, while it moves) is shared between a few cores, every few rows to each, so the
    // window waits less for each frame; a small one is not worth starting them for.
    let cores = thread::available_parallelism().map_or(1, |n| n.get()).min(4);
    if cores < 2 || w * h < 60_000 {
        for (y, row) in rows {
            paint_row(y, row);
        }
        return;
    }
    let mut shares: Vec<Vec<(usize, &mut [PremultipliedColorU8])>> = (0..cores).map(|_| Vec::new()).collect();
    for (y, row) in rows {
        shares[y % cores].push((y, row));
    }
    let paint_row = &paint_row;
    thread::scope(|scope| {
        for share in shares {
            scope.spawn(move || {
                for (y, row) in share {
                    paint_row(y, row);
                }
            });
        }
    });
}

/// [`paint`], checking afterwards that the shortcuts skipped no pixel of the shape and drew none outside it (for the tests).
#[cfg(test)]
fn paint_checked(pm: &mut Pixmap, g: &Glass) {
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    let shape = Shape::new(w as f32, h as f32, g.s, g.ear, g.gap, g.stretch, 0.0, &g.liquid);
    let mut full = Pixmap::new(w as u32, h as u32).unwrap();
    paint(&mut full, g);
    for y in 0..h {
        for x in 0..w {
            let d = shape.sd(x as f32 + 0.5, y as f32 + 0.5);
            let px = full.pixels()[y * w + x];
            assert_eq!(px.alpha() > 0, d < 0.5, "pixel {x},{y}: covered {}, distance {d}", px.alpha());
        }
    }
    pm.data_mut().copy_from_slice(full.data());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shape_hangs_from_the_top_edge_with_round_flares() {
        let (w, h, ear) = (300.0, 60.0, 15.0);
        let shape = Shape::new(w, h, 1.0, ear, 0.0, 0.0, 0.0, &Liquid::default());
        assert!(shape.sd(150.0, 30.0) < 0.0, "the middle is glass");
        assert!(shape.sd(150.0, 61.0) > 0.0, "below the bottom edge is not");
        assert!(shape.sd(5.0, 40.0) > 0.0, "beside the notch, lower down, is not");
        assert!(shape.sd(ear - 1.0, 0.5) < 0.0, "right beside the top of the notch is the flare");
        assert!(shape.sd(1.0, 0.5) > 0.0, "the flare ends inside the room for it");
        assert!(shape.sd(ear - 1.0, ear) > 0.0, "the flare is round: lower down it is gone");
        assert!(shape.sd(0.5, 0.5) > 0.4, "at the side of the window it has faded out");
    }

    #[test]
    fn the_direction_out_of_the_glass_matches_the_slope_of_the_distance() {
        for (gap, stretch) in [(0.0, 0.0), (0.0, 1.0), (7.5, 0.0)] {
            let shape = Shape::new(300.0, 80.0, 1.25, 15.0, gap, stretch, 0.0, &Liquid::default());
            for (x, y) in [(20.0, 40.0), (16.0, 3.0), (150.0, 78.0), (30.0, 70.0), (280.0, 60.0), (13.0, 1.5), (150.0, 20.0), (150.0, 9.0)] {
                let (d, n) = shape.sd_normal(x, y);
                assert!((d - shape.sd(x, y)).abs() < 1e-3, "distance at {x},{y}");
                let e = 0.01;
                let (gx, gy) = (shape.sd(x + e, y) - shape.sd(x - e, y), shape.sd(x, y + e) - shape.sd(x, y - e));
                let l = (gx * gx + gy * gy).sqrt();
                if l > 1e-4 {
                    let dot = (gx / l) * n.0 + (gy / l) * n.1;
                    assert!(dot > 0.98, "direction at {x},{y}: {n:?} against {:?}", (gx / l, gy / l));
                }
            }
        }
    }

    #[test]
    fn a_drop_hangs_from_the_middle_of_the_bottom_and_flows_into_it() {
        let (w, h) = (240.0, 42.0);
        let plain = Shape::new(w, h, 1.0, 15.0, 0.0, 0.0, 0.0, &Liquid::default());
        let dropping = Shape::new(w, h, 1.0, 15.0, 0.0, 0.0, 9.0, &Liquid::default());
        assert!(plain.sd(120.0, 46.0) > 0.0 && dropping.sd(120.0, 46.0) < 0.0, "below the middle is the drop");
        assert!(dropping.sd(60.0, 46.0) > 0.0, "beside it is not");
        assert!(dropping.sd(120.0, 41.5) < 0.0, "the neck joins it to the notch");
        let (d, n) = dropping.sd_normal(120.0, 52.0);
        assert!((d - dropping.sd(120.0, 52.0)).abs() < 1e-3 && n.1 > 0.9, "its bottom faces down");
    }

    #[test]
    fn a_drip_runs_down_from_the_middle_and_the_body_ends_above_it() {
        let liquid = Liquid { drip: 40.0, ..Liquid::default() };
        let shape = Shape::new(300.0, 120.0, 1.0, 12.0, 0.0, 0.0, 0.0, &liquid);
        assert!(shape.sd(150.0, 115.0) < 0.0, "the drop reaches the bottom in the middle");
        assert!(shape.sd(40.0, 100.0) > 0.0, "beside it the body has not come down yet");
        assert!(shape.sd(150.0, 82.0) < 0.0, "the neck joins it to the body");
        assert!(shape.simple, "still the same on both sides");
    }

    #[test]
    fn bubbles_sit_apart_from_the_body_and_a_bulge_swells_under_the_mouse() {
        let liquid = Liquid { narrow: 80.0, bubbles: Some((100.0, 13.0)), ..Liquid::default() };
        let shape = Shape::new(260.0, 28.0, 1.0, 12.0, 0.0, 0.0, 0.0, &liquid);
        assert!(shape.sd(30.0, 14.0) < 0.0 && shape.sd(230.0, 14.0) < 0.0, "a bubble on each side");
        assert!(shape.sd(130.0, 14.0) < 0.0, "the body in the middle");
        assert!(shape.sd(46.5, 20.0) > 0.0, "a gap between them, low down");
        assert!(!shape.simple);
        let bulge = Liquid { bulge: Some((60.0, 5.0)), ..Liquid::default() };
        let shape = Shape::new(260.0, 28.0, 1.0, 12.0, 0.0, 0.0, 0.0, &bulge);
        assert!(shape.sd(60.0, 31.0) < 0.0 && shape.sd(200.0, 31.0) > 0.0, "it swells where the mouse is only");
        for (x, y) in [(60.0, 30.0), (52.0, 29.0), (70.0, 27.5)] {
            let (d, n) = shape.sd_normal(x, y);
            assert!((d - shape.sd(x, y)).abs() < 1e-3, "distance at {x},{y}");
            let e = 0.01;
            let (gx, gy) = (shape.sd(x + e, y) - shape.sd(x - e, y), shape.sd(x, y + e) - shape.sd(x, y - e));
            let l = (gx * gx + gy * gy).sqrt();
            assert!((gx / l) * n.0 + (gy / l) * n.1 > 0.98, "direction at {x},{y}");
        }
    }

    #[test]
    fn the_transparency_setting_thins_or_thickens_only_the_see_through_glass() {
        let a = |look, clear| tuned(look, false, clear).tint_a;
        assert_eq!(a(NotchLook::Smoky, 50), params(NotchLook::Smoky, false).tint_a, "50 is the glass as designed");
        assert!(a(NotchLook::Smoky, 100) < 0.1 && a(NotchLook::Smoky, 0) > 0.85);
        assert!(a(NotchLook::Smoky, 75) < a(NotchLook::Smoky, 50) && a(NotchLook::Smoky, 25) > a(NotchLook::Smoky, 50));
        assert_eq!(a(NotchLook::Black, 100), params(NotchLook::Black, false).tint_a, "black glass shows nothing behind it anyway");
        assert_eq!(a(NotchLook::Classic, 0), 1.0);
        // Nearly solid, the clear glass is dark in the dark theme (its own tint is white).
        let milky = tuned(NotchLook::Clear, false, 0);
        assert!(milky.tint.iter().all(|c| *c < 0.2), "{:?}", milky.tint);
        assert!(tuned(NotchLook::Clear, true, 0).tint.iter().all(|c| *c > 0.9));
        assert_eq!(blur_px(NotchLook::Clear, 1.6, 0), 0, "no frost: sharp");
        assert_eq!(blur_px(NotchLook::Clear, 1.6, 100), 2 * blur_px(NotchLook::Clear, 1.6, 50));
    }

    #[test]
    fn the_classic_look_is_solid_black_or_nearly_white() {
        for (light, colour) in [(false, 0u8), (true, 248u8)] {
            let mut pm = Pixmap::new(200, 50).unwrap();
            let g = Glass {
                look: NotchLook::Classic,
                light,
                backdrop: None,
                origin: (0.0, 0.0),
                s: 1.0,
                ear: 12.0,
                gap: 0.0,
                stretch: 0.0,
                sun: (-0.75, 0.66),
                wash: None,
                room: 0.0,
                drop: 0.0,
                liquid: Liquid::default(),
                clear: 50,
            };
            paint(&mut pm, &g);
            let px = pm.pixel(100, 20).unwrap();
            assert_eq!((px.red(), px.green(), px.alpha()), (colour, colour, 255), "light {light}");
        }
    }

    #[test]
    fn the_floating_pill_leaves_a_gap_above_it() {
        let shape = Shape::new(300.0, 66.0, 1.0, 15.0, 6.0, 0.0, 0.0, &Liquid::default());
        assert!(shape.sd(150.0, 2.0) > 0.0, "the gap");
        assert!(shape.sd(150.0, 30.0) < 0.0);
        assert!(shape.sd(16.0, 7.0) > 0.0, "its corners are round");
    }

    #[test]
    fn a_hole_is_filled_from_around_it_and_a_flat_colour_stays_flat_under_the_blur() {
        let (w, h) = (20, 12);
        let mut px = vec![[0.2, 0.4, 0.6]; w * h];
        for y in 0..8 {
            for x in 5..15 {
                px[y * w + x] = [1.0, 0.0, 0.0];
            }
        }
        fill_hole(&mut px, w, h, (5, 0, 15, 8));
        assert!(px.iter().all(|c| (c[0] - 0.2).abs() < 1e-5 && (c[2] - 0.6).abs() < 1e-5), "filled with what is around it");
        let soft = blur_box(&px, w, h, 3);
        assert!(soft.iter().all(|c| (c[1] - 0.4).abs() < 1e-4));
    }

    #[test]
    fn a_small_bright_thing_below_the_hole_does_not_run_up_it_as_a_stripe() {
        let (w, h) = (60, 30);
        let mut px = vec![[0.1, 0.1, 0.1]; w * h];
        // A bright purple square just below the middle of a hole 40 wide and 20 tall.
        for y in 20..24 {
            for x in 28..32 {
                px[y * w + x] = [0.6, 0.3, 1.0];
            }
        }
        fill_hole(&mut px, w, h, (10, 0, 50, 20));
        let at = |x: usize, y: usize| px[y * w + x][2];
        assert!(at(30, 19) - at(15, 19) < 0.05, "just above it, the middle is hardly bluer than the side: {} {}", at(30, 19), at(15, 19));
    }

    #[test]
    fn what_was_seen_under_the_notch_is_put_back_and_only_the_never_seen_is_unknown() {
        let mut seen = None;
        // First the notch is small (covers x 4..6): the rest, x 2..4 included, is seen.
        let area = (0, 0, 8, 2);
        let mut first: Vec<u8> = (0..8 * 2).flat_map(|i| [i as u8, 0, 0, 0]).collect();
        mark_known(&mut first);
        blank_hole(&mut first, area, (4, 0, 2, 2));
        remember(&mut seen, area, &mut first);
        // Then it grows over x 2..6 and the area moves one pixel right: x 2..4 comes back as it was, x 4..6 stays unknown.
        let area = (1, 0, 8, 2);
        let mut second: Vec<u8> = (0..8 * 2).flat_map(|i| [100 + i as u8, 0, 0, 0]).collect();
        mark_known(&mut second);
        blank_hole(&mut second, area, (2, 0, 4, 2));
        remember(&mut seen, area, &mut second);
        let px = |x: usize| &second[x * 4..x * 4 + 4];
        assert_eq!(px(1), &[2, 0, 0, 255], "screen x 2: as seen before");
        assert_eq!(px(2), &[3, 0, 0, 255], "screen x 3: as seen before");
        assert_eq!(px(3)[3], 0, "screen x 4 was never seen");
        assert_eq!(px(0), &[100, 0, 0, 255], "outside the notch: the new copy");
    }

    #[test]
    fn the_notch_s_own_pixels_do_not_count_as_a_change() {
        let area = (100, 0, 10, 4);
        let mut a = vec![7u8; 10 * 4 * 4];
        let mut b = a.clone();
        b[(10 + 5) * 4] = 99; // a pixel of the notch itself changed
        blank_hole(&mut a, area, (103, 0, 4, 3));
        blank_hole(&mut b, area, (103, 0, 4, 3));
        assert_eq!(a, b);
    }

    #[test]
    fn the_glass_is_opaque_inside_clear_outside_and_soft_at_the_edge() {
        let mut pm = Pixmap::new(200, 50).unwrap();
        for look in [NotchLook::Clear, NotchLook::Smoky, NotchLook::Black, NotchLook::Floating, NotchLook::FloatingSmoky, NotchLook::FloatingBlack, NotchLook::FloatingSolid] {
            for light in [false, true] {
                let g = Glass {
                    look,
                    light,
                    backdrop: None,
                    origin: (0.0, 0.0),
                    s: 1.0,
                    ear: 12.0,
                    gap: gap(look) as f32,
                    stretch: 0.0,
                    sun: (-0.75, 0.66),
                    wash: None,
                    room: 0.0,
                    drop: 0.0,
                    liquid: Liquid::default(),
                    clear: 50,
                };
                paint(&mut pm, &g);
                let at = |x: u32, y: u32| pm.pixel(x, y).unwrap().alpha();
                assert_eq!(at(100, 30), 255, "{look:?}");
                assert_eq!(at(2, 45), 0, "{look:?}: outside");
                let soft = (0..50).flat_map(|y| (0..200).map(move |x| (x, y))).filter(|&(x, y)| at(x, y) > 0 && at(x, y) < 255).count();
                assert!(soft >= 8, "{look:?}: the round corners are smooth");
            }
        }
    }

    #[test]
    fn every_pixel_of_the_shape_is_drawn_and_nothing_outside_it() {
        for (w, h) in [(200u32, 50u32), (201, 51), (525, 220), (185, 35), (77, 40)] {
            for look in [NotchLook::Clear, NotchLook::Black, NotchLook::Floating] {
                for stretch in [0.0, 1.0] {
                    let mut pm = Pixmap::new(w, h).unwrap();
                    let g = Glass {
                        look,
                        light: false,
                        backdrop: None,
                        origin: (0.0, 0.0),
                        s: 1.25,
                        ear: 15.0,
                        gap: (gap(look) * 1.25) as f32,
                        stretch,
                        sun: (-0.75, 0.66),
                        wash: None,
                        room: 0.0,
                        drop: 0.0,
                        liquid: Liquid::default(),
                        clear: 50,
                    };
                    paint_checked(&mut pm, &g);
                }
            }
        }
    }

    #[test]
    fn dark_glass_darkens_itself_over_a_light_background() {
        let mut light_bg = build((0, 0, 60, 40), (10, 0, 40, 30), 2, &[240u8; 60 * 40 * 4], 1, (NotchLook::Clear, false, 50));
        light_bg.lum = 0.95;
        let dark_bg = build((0, 0, 60, 40), (10, 0, 40, 30), 2, &[20u8; 60 * 40 * 4], 1, (NotchLook::Clear, false, 50));
        let middle = |b: &Backdrop| {
            let mut pm = Pixmap::new(60, 40).unwrap();
            let g = Glass {
                look: NotchLook::Clear,
                light: false,
                backdrop: Some(b),
                origin: (0.0, 0.0),
                s: 1.0,
                ear: 10.0,
                gap: 0.0,
                stretch: 0.0,
                sun: (-0.75, 0.66),
                wash: None,
                room: 0.0,
                drop: 0.0,
                liquid: Liquid::default(),
                clear: 50,
            };
            paint(&mut pm, &g);
            pm.pixel(30, 12).unwrap().red()
        };
        assert!(middle(&light_bg) < 160, "white behind clear glass is toned down so white text stays readable");
        assert!(middle(&dark_bg) < 60);
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    use std::time::Instant;

    #[test]
    #[ignore]
    fn time_the_glass() {
        let (w, h) = (862u32, 337u32);
        let area = (0, 0, w as i32 + 80, h as i32 + 40);
        let raw = vec![90u8; (area.2 * area.3 * 4) as usize];
        let b = build(area, (40, 0, w as i32, h as i32), 6, &raw, 1, (NotchLook::Clear, false, 50));
        let mut pm = Pixmap::new(w, h).unwrap();
        for (name, backdrop) in [("no backdrop", None), ("backdrop", Some(&b))] {
            let g = Glass {
                look: NotchLook::Clear,
                light: false,
                backdrop,
                origin: (40.0, 0.0),
                s: 1.25,
                ear: 15.0,
                gap: 0.0,
                stretch: 0.5,
                sun: (-0.75, 0.66),
                wash: Some(([255, 90, 120], 0.6)),
                room: 0.0,
                drop: 0.0,
                liquid: Liquid::default(),
                clear: 50,
            };
            let t = Instant::now();
            for _ in 0..50 {
                paint(&mut pm, &g);
            }
            println!("{name}: {:.3} ms a frame", t.elapsed().as_secs_f64() * 1000.0 / 50.0);
        }
        let t = Instant::now();
        for _ in 0..20 {
            let _ = build(area, (40, 0, w as i32, h as i32), 6, &raw, 1, (NotchLook::Clear, false, 50));
        }
        println!("build a backdrop: {:.3} ms", t.elapsed().as_secs_f64() * 1000.0 / 20.0);
    }
}
