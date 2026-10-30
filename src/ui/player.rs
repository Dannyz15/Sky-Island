//! The Music view: the cover, the controls and the scrolling lyrics.

use crate::*;

/// Keys for the animated values that are not tied to a rectangle (see `motion::Glows`).
const KEY_PLAYER_PLAY: u64 = 0x4000_0000_0000_0001;

const KEY_BAR: u64 = 0x4000_0000_0000_0003;

const KEY_LYRIC_SYNC: u64 = 0x4000_0000_0000_0005;

// Expanded layout, in logical pixels.
const CTRL_Y: f32 = 142.0;

const CTRL_R: f32 = 16.0;

/// The Sync button that appears over the lyrics after they were scrolled away from the music.
const SYNC_RECT: (f32, f32, f32, f32) = (LYRIC_X + LYRIC_W - 62.0, 26.0, 62.0, 20.0);

pub(crate) enum LyricsState {
    None,
    Loading,
    NotFound,
    /// The lyrics site could not be reached, which is not the same as the song having no lyrics.
    Unreachable,
    Found(Vec<Line>),
}

/// How long to wait before asking the lyrics site again after it could not be reached.
pub(crate) const LYRICS_RETRY: Duration = Duration::from_secs(30);

// Lyrics layout, in logical pixels. The notch grows wider and the lyrics sit to the right of the player.
pub(crate) const LYRICS_W: f64 = 660.0; // notch width while the lyrics are open

pub(crate) const LYRICS_H: f64 = 176.0; // notch height while the lyrics are open

/// After the lyrics open or close, how long the notch waits for the mouse to come back if it now sits outside.
pub(crate) const LYRIC_GRACE: Duration = Duration::from_millis(1500);

pub(crate) const LYRIC_X: f32 = 292.0; // left edge of the lyrics

pub(crate) const LYRIC_AREA: (f32, f32) = (34.0, 172.0); // top, bottom of the scrolling lyrics area

const LYRIC_W: f32 = 344.0; // wrap width

const LYRIC_GAP: f32 = 12.0;

struct LyricItem {
    idx: usize,
    top: f32,
    lh: f32,
    px: f32,
    /// How many lines this one is from the middle of the view (0 when it is the middle line).
    near: f32,
    /// This is the line being sung right now.
    current: bool,
    rows: Vec<String>,
}

/// Text size of a lyric line by how far it is from the middle of the view.
fn lyric_px(near: f32) -> f32 {
    if near < 1.0 {
        18.0 - 4.0 * near
    } else if near < 2.0 {
        14.0 - (near - 1.0)
    } else {
        13.0
    }
}

/// Grey level and strength of a lyric line by how far it is from the middle of the view.
fn lyric_tone(near: f32) -> (u8, f32) {
    const KNOTS: [(f32, f32, f32); 4] = [(0.0, 255.0, 1.0), (1.0, 175.0, 0.85), (2.0, 135.0, 0.6), (3.0, 110.0, 0.45)];
    let near = near.clamp(0.0, 3.0);
    let i = (near.floor() as usize).min(2);
    let t = near - KNOTS[i].0;
    let (a, b) = (KNOTS[i], KNOTS[i + 1]);
    ((a.1 + (b.1 - a.1) * t) as u8, a.2 + (b.2 - a.2) * t)
}

/// A bright, readable colour taken from cover art (RGBA pixels): the average of its colourful
/// pixels, or of all of them when it has none.
pub(crate) fn art_tint(rgba: &[u8]) -> [u8; 3] {
    let (mut vivid, mut vivid_n) = ([0u64; 3], 0u64);
    let (mut all, mut all_n) = ([0u64; 3], 0u64);
    for px in rgba.as_chunks::<4>().0.iter().step_by(7) {
        let c = [px[0] as u64, px[1] as u64, px[2] as u64];
        let (hi, lo) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
        for i in 0..3 {
            all[i] += c[i];
        }
        all_n += 1;
        if hi > 70 && (hi - lo) * 100 / hi > 30 {
            for i in 0..3 {
                vivid[i] += c[i];
            }
            vivid_n += 1;
        }
    }
    let (sum, n) = if vivid_n > 8 { (vivid, vivid_n) } else { (all, all_n.max(1)) };
    let avg = [sum[0] as f32 / n as f32, sum[1] as f32 / n as f32, sum[2] as f32 / n as f32];
    // Lift it so that even a dark cover gives a visible accent.
    let peak = avg[0].max(avg[1]).max(avg[2]).max(1.0);
    let k = (235.0 / peak).min(2.4);
    let lifted = [(avg[0] * k).min(255.0) as u8, (avg[1] * k).min(255.0) as u8, (avg[2] * k).min(255.0) as u8];
    // A dark or grey cover has no colour to borrow, so lean towards a soft silver.
    let brightest = lifted[0].max(lifted[1]).max(lifted[2]);
    if brightest >= 170 {
        return lifted;
    }
    let t = (170 - brightest) as f32 / 170.0;
    let mix = |c: u8, silver: u8| (c as f32 + (silver as f32 - c as f32) * t) as u8;
    [mix(lifted[0], 200), mix(lifted[1], 200), mix(lifted[2], 212)]
}

/// Where the parts of the player sit. `t` is how far the lyrics have opened (0 closed, 1 open): the
/// player slides into a narrower column on the left to make room for them.
pub(crate) struct PlayerGeo {
    art: (f32, f32, f32), // x, y, size
    text_x: f32,
    left: f32,
    right: f32,
    bar: (f32, f32, f32), // x, y, width
    ctrl_x: [f32; 4],     // lyrics, previous, play/pause, next
}

pub(crate) fn player_geo(t: f32) -> PlayerGeo {
    let right = 396.0 + (264.0 - 396.0) * t;
    let mid = (24.0 + right) / 2.0;
    PlayerGeo {
        art: (24.0, 30.0, 64.0),
        text_x: 104.0,
        left: 24.0,
        right,
        bar: (64.0, 106.0, right - 104.0),
        ctrl_x: [mid - 100.0, mid - 50.0, mid, mid + 50.0],
    }
}

impl App {
    /// The line the lyrics view is heading for: where the wheel took it, otherwise the line being sung
    /// (-1 before the first one starts).
    pub(crate) fn lyric_target(&self) -> f64 {
        self.lyric_manual.unwrap_or_else(|| self.current_line().map_or(-1.0, |c| c as f64))
    }

    /// Put the middle of the view on its target at once, without gliding there.
    pub(crate) fn snap_lyrics(&mut self) {
        self.lyric_pos = self.lyric_target();
        self.lyric_v = 0.0;
    }

    /// Whether the lyrics were scrolled to somewhere other than the line being sung, so Sync is offered.
    fn lyric_off_sync(&self) -> bool {
        let sung = self.current_line().map_or(-1.0, |c| c as f64);
        self.lyric_manual.is_some_and(|m| (m - sung).abs() >= 1.0)
    }

    /// Index of the lyric line being sung at the current position.
    pub(crate) fn current_line(&self) -> Option<usize> {
        let LyricsState::Found(lines) = &self.lyrics else { return None };
        let pos = self.position();
        lines.iter().rposition(|l| l.at <= pos)
    }

    /// The song position under a cursor x (logical pixels), clamped to the ends of the bar.
    pub(crate) fn scrub_at(&self, cx: f32) -> Duration {
        let (bx, _, bw) = self.geo().bar;
        let frac = ((cx - bx) / bw).clamp(0.0, 1.0);
        self.media.duration.mul_f32(frac)
    }

    /// Lay out the visible lyric lines around the middle of the view, wrapped, in logical pixels.
    fn layout_lyrics(&self) -> Vec<LyricItem> {
        let (Some(font), LyricsState::Found(lines)) = (&self.font, &self.lyrics) else {
            return Vec::new();
        };
        let cur = self.current_line();
        // The middle of the view is a fractional line while it glides. Before the first line starts it
        // is -1, so the upcoming lines start below the middle.
        let pos = self.lyric_pos.clamp(-1.0, lines.len() as f64 - 1.0);
        let base = pos.floor() as isize;
        let frac = (pos - base as f64) as f32;
        let mid = (LYRIC_AREA.0 + LYRIC_AREA.1) / 2.0;

        let make = |d: isize| -> Option<LyricItem> {
            let idx = usize::try_from(base + d).ok().filter(|&i| i < lines.len())?;
            let near = (idx as f64 - pos).abs() as f32;
            let px = lyric_px(near);
            // The middle line is drawn bold, which is wider, so it is wrapped with the bold face too.
            let face = if near < 0.5 { self.heading_font().unwrap_or(font) } else { font };
            Some(LyricItem { idx, top: 0.0, lh: px * 1.3, px, near, current: cur == Some(idx), rows: wrap(face, &lines[idx].text, px, LYRIC_W) })
        };

        let mut items = Vec::new();
        let (mut below, h0) = match make(0) {
            Some(mut it) => {
                let h = it.rows.len() as f32 * it.lh;
                it.top = mid - h / 2.0;
                let bottom = it.top + h;
                let top = it.top;
                items.push(it);
                (bottom, (top, h))
            }
            None => (mid, (mid, 0.0)),
        };
        let mut above = h0.0;
        for d in 1..=5 {
            if let Some(mut it) = make(d) {
                it.top = below + LYRIC_GAP;
                below = it.top + it.rows.len() as f32 * it.lh;
                items.push(it);
            }
            if let Some(mut it) = make(-d) {
                let h = it.rows.len() as f32 * it.lh;
                it.top = above - LYRIC_GAP - h;
                above = it.top;
                items.push(it);
            }
        }

        // Between two lines the whole stack is part of the way from one to the next.
        let center = |it: &LyricItem| it.top + it.rows.len() as f32 * it.lh / 2.0;
        let here = items.iter().find(|i| i.idx as isize == base).map_or(mid, center);
        if let Some(next) = items.iter().find(|i| i.idx as isize == base + 1).map(center) {
            let shift = -(next - here) * frac;
            for it in &mut items {
                it.top += shift;
            }
        }
        items
    }

    fn draw_art(&self, pm: &mut Pixmap, rect: (f32, f32, f32), s: f32, alpha: f32) {
        let (ax, ay, asz) = rect;
        let radius = (asz * 0.16).min(10.0);
        let [ar, ag, ab] = self.art_tint;
        if asz > 40.0 {
            // The cover's colour spreads softly behind it.
            gfx::glow(pm, (ax + asz / 2.0) * s, (ay + asz / 2.0) * s, asz * 0.95 * s, [ar, ag, ab, (alpha * 52.0) as u8], None);
            gfx::soft_shadow(pm, ax * s, ay * s, asz * s, asz * s, radius * s, 7.0 * s, (alpha * 110.0) as u8);
        }
        let Some(shape) = gfx::rounded_path(ax * s, ay * s, asz * s, asz * s, radius * s) else { return };
        match (&self.media.art, gfx::mask_of(pm.width(), pm.height(), &shape)) {
            (Some(rgba), Some(mask)) => {
                let n = media::art_size();
                if let Some(mut art) = Pixmap::new(n, n) {
                    for (dst, px) in art.pixels_mut().iter_mut().zip(rgba.as_chunks::<4>().0) {
                        *dst = tiny_skia::ColorU8::from_rgba(px[0], px[1], px[2], 255).premultiply();
                    }
                    let scale = asz * s / n as f32;
                    let paint = PixmapPaint { opacity: alpha, quality: FilterQuality::Bilinear, ..Default::default() };
                    let at = Transform::from_scale(scale, scale).post_translate(ax * s, ay * s);
                    pm.draw_pixmap(0, 0, art.as_ref(), &paint, at, Some(&mask));
                }
            }
            _ => {
                // No cover: a soft tile with a note on it.
                let (top, bottom) = ([58, 58, 68, (alpha * 255.0) as u8], [30, 30, 38, (alpha * 255.0) as u8]);
                gfx::gradient_rounded(pm, ax * s, ay * s, asz * s, asz * s, radius * s, top, bottom, false);
                let note = [150, 150, 165, (alpha * 255.0) as u8];
                draw_tab_icon(pm, View::Player, (ax + asz / 2.0) * s, (ay + asz / 2.0) * s, s * (asz / 40.0).clamp(0.6, 1.8), note);
            }
        }
        gfx::stroke_rounded(pm, ax * s, ay * s, asz * s, asz * s, radius * s, 1.0 * s, [255, 255, 255, (alpha * 26.0) as u8]);
    }

    /// The lyrics to the right of the player: the line being sung is large and bright, the rest fade away.
    /// `dx` pushes the panel to the right, which is how it slides in while the notch widens.
    fn draw_lyric_panel(&self, pm: &mut Pixmap, s: f32, alpha: f32, dx: f32) {
        let Some(font) = &self.font else { return };
        let a = (alpha * 255.0) as u8;
        let grey = [160, 160, 165, a];

        if matches!(self.lyrics, LyricsState::Found(_)) {
            for it in self.layout_lyrics() {
                let (rgb, strength) = lyric_tone(it.near);
                for (i, row) in it.rows.iter().enumerate() {
                    let top = it.top + i as f32 * it.lh;
                    let bottom = top + it.lh;
                    // Fade rows out towards the edges of the lyrics area; skip anything outside it.
                    let edge = (top - LYRIC_AREA.0).min(LYRIC_AREA.1 - bottom);
                    if edge <= 0.0 {
                        continue;
                    }
                    let fade = (edge / 28.0).clamp(0.0, 1.0);
                    let mut color = [rgb, rgb, rgb, (a as f32 * strength * fade) as u8];
                    if it.current {
                        // The line being sung is warmed with the cover's colour, and still shows when the
                        // lyrics were scrolled away from it.
                        let [ar, ag, ab] = self.accent(View::Lyrics);
                        let warmth = if it.near < 0.5 { 0.22 } else { 0.55 };
                        color = gfx::mix(color, [ar, ag, ab, (a as f32 * strength.max(0.75) * fade) as u8], warmth);
                    }
                    let face = if it.near < 0.5 { self.heading_font().unwrap_or(font) } else { font };
                    let baseline = (top + it.px * 1.05) * s;
                    draw_text(pm, face, row, (LYRIC_X + dx) * s, baseline, it.px * s, color, (LYRIC_W + 8.0) * s);
                }
            }
            self.draw_lyric_sync(pm, font, s, alpha, dx);
        } else {
            let msg = match &self.lyrics {
                LyricsState::Loading => "Searching for lyrics\u{2026}",
                LyricsState::NotFound => "No synced lyrics found",
                LyricsState::Unreachable => "Lyrics site unreachable",
                _ => "Play something to see lyrics",
            };
            let y = (LYRIC_AREA.0 + LYRIC_AREA.1) / 2.0 + 5.0;
            draw_text_centered(pm, font, msg, (LYRIC_X + dx + LYRIC_W / 2.0) * s, y * s, 13.0 * s, grey, LYRIC_W * s);
        }
    }

    /// The Sync button: it fades in once the lyrics were scrolled away from the music, with an arrow
    /// that points to where the line being sung is.
    fn draw_lyric_sync(&self, pm: &mut Pixmap, font: &Font, s: f32, alpha: f32, dx: f32) {
        let show = self.glows.fade(KEY_LYRIC_SYNC, self.lyric_off_sync());
        if show < 0.01 {
            return;
        }
        let r = (SYNC_RECT.0 + dx, SYNC_RECT.1, SYNC_RECT.2, SYNC_RECT.3);
        let hot = self.hot_t(r, s);
        let k = alpha * show;
        let a = (k * 255.0) as u8;
        let [ar, ag, ab] = self.accent(View::Lyrics);
        let (x, y, w, h) = (r.0 * s, r.1 * s, r.2 * s, r.3 * s);
        // A soft backing so the button stays readable over the lines behind it.
        fill_rounded(pm, x - 3.0 * s, y - 3.0 * s, w + 6.0 * s, h + 6.0 * s, (h + 6.0 * s) / 2.0, [0, 0, 0, (k * 120.0) as u8]);
        gfx::gradient_rounded(pm, x, y, w, h, h / 2.0, [ar, ag, ab, (k * pick(120.0, 84.0, hot)) as u8], [ar, ag, ab, (k * pick(80.0, 46.0, hot)) as u8], false);
        gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [ar, ag, ab, (k * 150.0) as u8]);
        // The arrow points up when the line being sung is above the middle, down when it is below.
        let sung = self.current_line().map_or(-1.0, |c| c as f64);
        let dir = if sung < self.lyric_manual.unwrap_or(sung) { -1.0 } else { 1.0 };
        let (ax, ay) = (x + 12.0 * s, y + h / 2.0);
        fill_path(pm, &[(ax - 3.5 * s, ay - 1.5 * s * dir), (ax + 3.5 * s, ay - 1.5 * s * dir), (ax, ay + 2.8 * s * dir)], [255, 255, 255, a]);
        draw_text(pm, font, "Sync", x + 22.0 * s, y + h / 2.0 + 4.0 * s, 11.0 * s, [255, 255, 255, a], 36.0 * s);
    }

    pub(crate) fn draw_player(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let a = (alpha * 255.0) as u8;
        let rgb = self.accent(View::Player);
        let accent = [rgb[0], rgb[1], rgb[2], a];
        let white = [255, 255, 255, a];
        let grey = [165, 165, 175, a];
        let open = self.view_t.clamp(0.0, 1.0) as f32;
        let geo = self.geo();

        let (title, sub) = if self.media.has_session && !self.media.title.is_empty() {
            let sub = if self.media.album.is_empty() {
                self.media.artist.clone()
            } else {
                format!("{} \u{b7} {}", self.media.artist, self.media.album)
            };
            (self.media.title.as_str(), sub)
        } else {
            ("Nothing playing", "Play something to see it here".to_string())
        };

        // The lyrics fade in on the right as the notch widens and the player moves into its column.
        // They wait until the player has moved out of the way, then fade in and slide into place.
        let reveal = ((open - 0.45) / 0.55).clamp(0.0, 1.0);
        if reveal > 0.0 {
            self.draw_lyric_panel(pm, s, alpha * reveal, (1.0 - reveal) * 28.0);
        }

        self.fade_in_track(pm, s, |pm| {
            self.draw_art(pm, geo.art, s, alpha);
            if let Some(font) = &self.font {
                let heading = self.heading_font().unwrap_or(font);
                let max_w = (geo.right - geo.text_x) * s;
                draw_text(pm, heading, title, geo.text_x * s, 56.0 * s, 17.0 * s, white, max_w);
                draw_text(pm, font, &sub, geo.text_x * s, 78.0 * s, 13.0 * s, gfx::mix(grey, accent, 0.4), max_w);
            }
        });

        let (bx, by, bw) = geo.bar;
        if let Some(font) = &self.font {

            // Time played on the left of the bar, time left on the right.
            let pos = fmt_time(self.position());
            let left = format!("-{}", fmt_time(self.media.duration.saturating_sub(self.position())));
            let base = (by + 5.5) * s;
            draw_text(pm, font, &pos, geo.left * s, base, 10.0 * s, grey, 42.0 * s);
            let left_w = text_width(font, &left, 10.0 * s);
            draw_text(pm, font, &left, geo.right * s - left_w, base, 10.0 * s, grey, 46.0 * s);
        }

        // Progress bar: a rounded track, a filled part shaded with the cover's colour, and a knob.
        let grab = self.hot((bx - 4.0, by - 9.0, bw + 8.0, 21.0), s) || self.scrub.is_some();
        let grow = self.glows.follow(KEY_BAR, if grab { 1.0 } else { 0.0 }, 30.0, 0.8).clamp(0.0, 1.0) as f32;
        let bh = 5.0 + 2.0 * grow;
        let ty = by + 2.0 - bh / 2.0;
        fill_rounded(pm, bx * s, ty * s, bw * s, bh * s, bh * s / 2.0, [255, 255, 255, (alpha * 30.0) as u8]);
        let frac = if self.media.duration.is_zero() {
            0.0
        } else {
            (self.position().as_secs_f32() / self.media.duration.as_secs_f32()).clamp(0.0, 1.0)
        };
        if frac > 0.0 {
            let fill_w = (bw * frac).max(bh);
            gfx::gradient_rounded(pm, bx * s, ty * s, fill_w * s, bh * s, bh * s / 2.0, gfx::mix(accent, white, 0.15), gfx::mix(accent, white, 0.7), true);
            let knob_x = (bx + fill_w - bh / 2.0) * s;
            gfx::glow(pm, knob_x, (by + 2.0) * s, (12.0 + 5.0 * grow) * s, [rgb[0], rgb[1], rgb[2], (alpha * (90.0 + 50.0 * grow)) as u8], None);
            fill_circle(pm, knob_x, (by + 2.0) * s, (4.0 + 2.0 * grow) * s, white);
        }

        // Transport controls: the lyrics button, then previous, a glowing round play button, and next.
        let py = CTRL_Y * s;
        let [lyrics_x, prev_x, play_x, next_x] = geo.ctrl_x;
        for (i, x) in [lyrics_x, prev_x, next_x].into_iter().enumerate() {
            let enabled = i != 2 || self.media.next_enabled;
            let glow = if enabled { self.hot_t((x - 18.0, CTRL_Y - 18.0, 36.0, 36.0), s) } else { 0.0 };
            if glow > 0.01 {
                fill_circle(pm, x * s, py, 18.0 * s, [255, 255, 255, (alpha * 24.0 * glow) as u8]);
            }
        }
        // The lyrics button is lit while the lyrics are open.
        if open > 0.01 {
            fill_circle(pm, lyrics_x * s, py, 16.0 * s, [rgb[0], rgb[1], rgb[2], (alpha * open * 70.0) as u8]);
        }
        draw_lyrics_icon(pm, lyrics_x * s, py, s, gfx::mix(grey, white, open));
        let dim = [90, 90, 98, a];
        draw_prev(pm, prev_x * s, py, s, white);
        draw_next(pm, next_x * s, py, s, if self.media.next_enabled { white } else { dim });
        let play = self.hot_t((play_x - 22.0, CTRL_Y - 22.0, 44.0, 44.0), s);
        gfx::glow(pm, play_x * s, py, 34.0 * s, [rgb[0], rgb[1], rgb[2], (alpha * 56.0) as u8], None);
        let radius = (pick(21.0, 19.5, play)) * s;
        gfx::gradient_circle(pm, play_x * s, py, radius, white, [214, 214, 226, a]);
        let pause = self.glows.follow(KEY_PLAYER_PLAY, if self.media.playing { 1.0 } else { 0.0 }, 30.0, 0.85) as f32;
        draw_play_pause(pm, play_x * s, py, s * 0.82, [12, 12, 16, a], pause);
    }

    /// A click in the Music view, with or without the lyrics. (`cx`, `cy`) is the spot in the view's own coordinates.
    pub(crate) fn click_player(&mut self, cx: f32, cy: f32, window: &Window) {
        let geo = self.geo();
        // Press on the bar (a taller target than the thin line) and drag to jump in the song.
        let (bx, by, bw) = geo.bar;
        if !self.media.duration.is_zero()
            && self.media_handle.is_some()
            && cx >= bx - 6.0
            && cx <= bx + bw + 6.0
            && (by - 9.0..=by + 12.0).contains(&cy)
        {
            self.scrub = Some(self.scrub_at(cx));
            window.request_redraw();
            return;
        }
        // The speech-bubble button opens and closes the lyrics.
        if (cx - geo.ctrl_x[0]).hypot(cy - CTRL_Y) <= CTRL_R {
            self.view = if self.view == View::Lyrics { View::Player } else { View::Lyrics };
            if self.view == View::Lyrics {
                // Opening: start on the line being sung, not wherever it was left.
                self.lyric_manual = None;
                self.snap_lyrics();
            }
            self.lyric_settle = true;
            self.last_tick = Instant::now();
            window.request_redraw();
            return;
        }
        if let Some(handle) = &self.media_handle {
            for (i, x) in geo.ctrl_x[1..].iter().enumerate() {
                if (cx - x).hypot(cy - CTRL_Y) <= CTRL_R {
                    handle.send(match i {
                        0 => Command::Previous,
                        1 => Command::TogglePlay,
                        _ => Command::Next,
                    });
                    return;
                }
            }
        }
        // Sync takes the lyrics back to the line being sung.
        if self.view == View::Lyrics
            && self.lyric_off_sync()
            && cx >= SYNC_RECT.0
            && cx <= SYNC_RECT.0 + SYNC_RECT.2
            && cy >= SYNC_RECT.1
            && cy <= SYNC_RECT.1 + SYNC_RECT.3
        {
            self.lyric_manual = None;
            window.request_redraw();
            return;
        }
        // Click a lyric line to jump to it.
        if self.view != View::Lyrics || cx < LYRIC_X - 8.0 || cy < LYRIC_AREA.0 || cy > LYRIC_AREA.1 {
            return;
        }
        let LyricsState::Found(lines) = &self.lyrics else { return };
        let hit = self
            .layout_lyrics()
            .into_iter()
            .find(|it| cy >= it.top && cy <= it.top + it.rows.len() as f32 * it.lh);
        if let Some(at) = hit.and_then(|it| lines.get(it.idx)).map(|l| l.at) {
            if let Some(handle) = &self.media_handle {
                handle.send(Command::Seek(at));
            }
            // Show the jump immediately instead of waiting for the player to report it.
            self.media.position = at;
            self.media_at = Instant::now();
            // The lyrics follow the music again from the line that was picked.
            self.lyric_manual = None;
            window.request_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lyric_lines_shrink_and_fade_with_distance_from_the_middle() {
        assert_eq!(lyric_px(0.0), 18.0);
        assert_eq!(lyric_px(1.0), 14.0);
        assert_eq!(lyric_px(2.0), 13.0);
        assert_eq!(lyric_px(5.0), 13.0);
        assert!(lyric_px(0.5) > 14.0 && lyric_px(0.5) < 18.0, "smooth in between");
        assert_eq!(lyric_tone(0.0), (255, 1.0));
        assert_eq!(lyric_tone(9.0), (110, 0.45));
        let (grey, strength) = lyric_tone(0.5);
        assert!(grey > 175 && grey < 255 && strength > 0.85 && strength < 1.0);
    }

    #[test]
    fn art_tint_follows_the_cover_colour() {
        let solid = |c: [u8; 4]| c.iter().copied().cycle().take(4 * 400).collect::<Vec<u8>>();
        let [r, g, b] = art_tint(&solid([200, 30, 30, 255]));
        assert!(r > 200 && g < 80 && b < 80, "red cover gives a red accent: {r},{g},{b}");
        let [r, g, b] = art_tint(&solid([10, 10, 10, 255]));
        assert!(r > 150 && (b as i32 - r as i32).abs() < 20 && g > 150, "a dark grey cover still gives a visible, neutral accent: {r},{g},{b}");
        assert!(art_tint(&[]).iter().all(|&c| c > 150), "no pixels: no panic, a neutral accent");
    }
}
