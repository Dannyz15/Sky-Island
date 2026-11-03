//! The Stats view: the processor, memory, disk and network, from `stats.rs`.

use crate::*;

const KEY_CPU: u64 = 0x4000_0000_0000_0010;
const KEY_MEM: u64 = 0x4000_0000_0000_0011;
const KEY_DISK: u64 = 0x4000_0000_0000_0100; // plus the number of the drive

/// The ring for the processor: its middle and size.
const RING: (f32, f32, f32) = (70.0, 90.0, 34.0);
const RING_W: f32 = 7.0;

/// Each drive beyond the first needs this much more room, and the notch grows by it.
const ROW: f32 = 34.0;

/// The right-hand column.
const COL_X: f32 = 136.0;
const COL_W: f32 = 260.0;

/// Green while there is plenty of room, through amber, to red when it is nearly full.
fn load_color(frac: f32, a: u8) -> [u8; 4] {
    let (green, amber, red) = ([80, 200, 120, a], [255, 190, 60, a], [255, 90, 80, a]);
    if frac < 0.6 {
        green
    } else if frac < 0.85 {
        gfx::mix(green, amber, (frac - 0.6) / 0.25)
    } else {
        gfx::mix(amber, red, ((frac - 0.85) / 0.15).clamp(0.0, 1.0))
    }
}

impl App {
    /// How many bars the disks take: one per drive, and one ("not available") if none were found.
    fn disk_rows(&self) -> usize {
        self.stats.last.disks.len().clamp(1, stats::MAX_DRIVES)
    }

    /// How much taller than normal the notch is while the Stats view is open, to fit one bar for each drive.
    pub(crate) fn stats_extra(&self) -> f64 {
        if self.view == View::Stats { (ROW * (self.disk_rows() - 1) as f32) as f64 } else { 0.0 }
    }

    /// While the Stats view is open, look at the numbers (the first two looks come quickly, then once a second) and
    /// say when to wake for the next one. Nothing is looked at while it is closed; its numbers are forgotten, so
    /// that it starts afresh next time.
    pub(crate) fn stats_tick(&mut self, now: Instant) -> Option<Instant> {
        if self.progress < 1.0 || self.view != View::Stats {
            if self.stats.started {
                // The drives stay, so the view keeps its shape while it slides out and opens at the right height.
                let disks = std::mem::take(&mut self.stats.last.disks);
                self.stats = stats::Stats::default();
                self.stats.disks_known = !disks.is_empty();
                self.stats.last.disks = disks;
                self.stats_at = None;
            }
            return None;
        }
        let gap = if self.stats.ready() { stats::EVERY } else { stats::FIRST_AGAIN };
        let due = self.stats_at.map_or(now, |at| at + gap);
        if now >= due {
            self.stats.sample(now);
            self.stats_at = Some(now);
            if let Some(w) = &self.window {
                w.request_redraw();
            }
            return Some(now + if self.stats.ready() { stats::EVERY } else { stats::FIRST_AGAIN });
        }
        Some(due)
    }

    pub(crate) fn draw_stats(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let Some(font) = &self.font else { return };
        let heading = self.heading_font().unwrap_or(font);
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [165, 165, 175, a]);
        let rgb = self.accent(View::Stats);
        let last = &self.stats.last;
        let rows = self.disk_rows();
        let more = ROW * (rows - 1) as f32;

        // The processor: a ring that fills with the load, and the number in it. With more drives it stays in the middle.
        let (rx, ry, rr) = RING;
        let ry = ry + more / 2.0;
        let cpu = last.cpu.unwrap_or(0.0);
        let shown = self.glows.follow(KEY_CPU, cpu as f64, 14.0, 0.9).clamp(0.0, 1.0) as f32;
        gfx::glow(pm, rx * s, ry * s, (rr + 26.0) * s, [rgb[0], rgb[1], rgb[2], (alpha * 28.0) as u8], None);
        stroke_arc(pm, rx * s, ry * s, rr * s, RING_W * s, 1.0, [255, 255, 255, (alpha * 22.0) as u8]);
        stroke_arc(pm, rx * s, ry * s, rr * s, RING_W * s, shown, load_color(shown, a));
        let text = if last.cpu.is_some() { format!("{}%", (cpu * 100.0).round() as u32) } else { "...".to_string() };
        draw_text_centered(pm, heading, &text, rx * s, (ry + 7.0) * s, 22.0 * s, white, (rr * 2.0 - 12.0) * s);
        draw_text_centered(pm, font, "Processor", rx * s, (ry + rr + 20.0) * s, 10.5 * s, grey, 90.0 * s);

        // Memory and disk: a line of words and a bar each.
        let bar = |pm: &mut Pixmap, y: f32, label: &str, used_total: Option<(u64, u64)>, key: u64| {
            draw_text(pm, font, label, COL_X * s, y * s, 11.5 * s, white, 100.0 * s);
            let Some((used, total)) = used_total else {
                draw_text(pm, font, "not available", (COL_X + 110.0) * s, y * s, 11.0 * s, grey, 150.0 * s);
                return;
            };
            let frac = used as f32 / total as f32;
            let words = format!("{} of {}", stats::gigabytes(used), stats::gigabytes(total));
            let w = text_width(font, &words, 11.0 * s);
            draw_text(pm, font, &words, (COL_X + COL_W) * s - w, y * s, 11.0 * s, grey, 170.0 * s);
            let shown = self.glows.follow(key, frac as f64, 14.0, 0.9).clamp(0.0, 1.0) as f32;
            draw_bar(pm, COL_X * s, (y + 9.0) * s, COL_W * s, shown, load_color(shown, a), s * 0.8, a);
        };
        bar(pm, 50.0, "Memory", last.memory, KEY_MEM);
        if last.disks.is_empty() && self.stats.disks_known {
            bar(pm, 84.0, "Disk", None, KEY_DISK);
        }
        for (i, d) in last.disks.iter().enumerate() {
            bar(pm, 84.0 + ROW * i as f32, &format!("Disk {}:", d.letter), Some((d.used, d.total)), KEY_DISK + i as u64);
        }

        // The network.
        draw_text(pm, font, "Network", COL_X * s, (118.0 + more) * s, 11.5 * s, white, 100.0 * s);
        let (down, up) = last.network.map_or(("...".to_string(), "...".to_string()), |(d, u)| (stats::speed(d), stats::speed(u)));
        let words = format!("\u{2193} {down}    \u{2191} {up}");
        let w = text_width(font, &words, 11.0 * s);
        draw_text(pm, font, &words, (COL_X + COL_W) * s - w, (118.0 + more) * s, 11.0 * s, grey, 190.0 * s);

        // The processor over the last minute, as a line.
        let (gx, gy, gw, gh) = (COL_X, 128.0 + more, COL_W, 28.0);
        fill_rounded(pm, gx * s, gy * s, gw * s, gh * s, 6.0 * s, [255, 255, 255, (alpha * 9.0) as u8]);
        let n = self.stats.history.len();
        if n >= 2 {
            let step = gw / (stats::HISTORY - 1) as f32;
            let x0 = gx + gw - step * (n - 1) as f32;
            let pts: Vec<(f32, f32)> =
                self.stats.history.iter().enumerate().map(|(i, v)| ((x0 + i as f32 * step) * s, (gy + gh - 3.0 - v * (gh - 6.0)) * s)).collect();
            stroke_poly(pm, &pts, 1.5 * s, [rgb[0], rgb[1], rgb[2], a], false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_colour_goes_from_green_to_red_as_things_fill_up() {
        let (low, mid, high) = (load_color(0.1, 255), load_color(0.7, 255), load_color(1.0, 255));
        assert!(low[1] > low[0] && high[0] > high[1], "green when empty, red when full");
        assert!(mid[0] > low[0] && mid[1] > high[1], "amber in between");
    }

    #[test]
    fn each_drive_makes_the_notch_taller_by_one_row() {
        let mut stats = stats::Stats::default();
        let drive = |letter| stats::Drive { letter, used: 1, total: 2 };
        let rows = |n: usize| (ROW * (n.clamp(1, stats::MAX_DRIVES) - 1) as f32) as f64;
        stats.last.disks = vec![drive('C')];
        assert_eq!(rows(stats.last.disks.len()), 0.0, "one drive fits the normal height");
        stats.last.disks = vec![drive('C'), drive('D'), drive('E')];
        assert_eq!(rows(stats.last.disks.len()), 2.0 * ROW as f64);
        assert_eq!(rows(40), rows(stats::MAX_DRIVES), "there is a limit");
        assert_eq!(rows(0), 0.0, "no drive found still shows one row");
    }

    #[test]
    fn the_stats_fit_the_notch() {
        assert!(COL_X + COL_W <= EXPANDED.0 as f32 - 24.0);
        assert!(RING.0 + RING.2 + RING_W < COL_X);
    }
}
