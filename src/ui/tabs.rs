//! The tab bar at the bottom, and moving between views.

use crate::*;

/// Tab bar entries, left to right.
const TABS: [(&str, View); 10] = [
    ("Music", View::Player),
    ("Timer", View::Timer),
    ("Water", View::Water),
    ("Weather", View::Weather),
    ("Prayer", View::Prayer),
    ("Stats", View::Stats),
    ("Shelf", View::Shelf),
    ("To-do", View::Todo),
    ("Calendar", View::Calendar),
    ("Clipboard", View::Clipboard),
];

const TAB_W: f32 = 26.0;

const TAB_GAP: f32 = 2.0;

/// The tab bar sits at the bottom of the notch: the top of its pill is this far above the bottom edge.
pub(crate) const TAB_BOTTOM: f32 = 34.0;

/// The pill behind the tabs is this high, and each tab is a round button this big inside it.
pub(crate) const TAB_PILL_H: f32 = 28.0;

const TAB_BUTTON: f32 = 24.0;

/// The open view is drawn this much higher than where it is laid out, which leaves room for the tab bar
/// under it. Everything that reads the mouse position for the view adds this back.
pub(crate) const TAB_SHIFT: f32 = 22.0;

/// Tab `i` of `count`, centred in the notch, at the bottom of a window `height` logical pixels high.
fn tab_rect(i: usize, count: usize, height: f32) -> (f32, f32, f32, f32) {
    let total = count as f32 * TAB_W + (count as f32 - 1.0) * TAB_GAP;
    let x0 = (EXPANDED.0 as f32 - total) / 2.0;
    (x0 + i as f32 * (TAB_W + TAB_GAP), height - TAB_BOTTOM + (TAB_PILL_H - TAB_BUTTON) / 2.0, TAB_W, TAB_BUTTON)
}

/// The change from one view to another: how far the new view has taken over, the view fading out, and the way the new one slides from.
pub(crate) struct ViewSwitch {
    /// 0 right after the view changed, 1 once the new view has taken over.
    pub(crate) t: f64,
    /// How fast `t` is changing (the spring's speed).
    pub(crate) v: f64,
    /// The view that is fading out while the new one comes in, and which way the new one slides from.
    pub(crate) prev: Option<View>,
    pub(crate) dir: f32,
    /// The view the last switch was for: a change of `App::view` starts the next one from here.
    pub(crate) shown: View,
}

/// The highlight under the tabs.
pub(crate) struct TabHighlight {
    /// The tab highlight's position, as a (fractional) tab index, so it can slide between tabs.
    pub(crate) pos: f32,
    /// How fast it is moving, in tabs per second; the highlight stretches while it moves.
    pub(crate) vel: f32,
}

impl Default for TabHighlight {
    fn default() -> Self {
        Self { pos: 0.0, vel: 0.0 }
    }
}

impl Default for ViewSwitch {
    fn default() -> Self {
        Self { t: 1.0, v: 0.0, prev: None, dir: 1.0, shown: View::Player }
    }
}

impl App {
    /// The tab index the highlight is heading for, if the current view has a tab.
    pub(crate) fn tab_target(&self) -> Option<f32> {
        // The lyrics are part of the Music tab.
        let view = if self.view == View::Lyrics { View::Player } else { self.view };
        self.visible_tabs().iter().position(|(_, v)| *v == view).map(|i| i as f32)
    }

    /// How far the tab bar (and the views other than Music) shift right to stay centred while the
    /// notch is wider than usual because the lyrics are open.
    pub(crate) fn tab_dx(&self) -> f32 {
        ((LYRICS_W - EXPANDED.0) * self.spring.view_t.clamp(0.0, 1.0) / 2.0) as f32
    }

    /// The tabs in the bar: the ones that are switched on in the settings.
    pub(crate) fn visible_tabs(&self) -> Vec<(&'static str, View)> {
        TABS.iter().copied().filter(|(_, v)| self.tab_enabled(*v)).collect()
    }

    /// Whether a tab is switched on. The lyrics belong to the Music tab.
    pub(crate) fn tab_enabled(&self, view: View) -> bool {
        let s = &self.settings;
        match view {
            View::Player | View::Lyrics => s.tab_music,
            View::Timer => s.timer,
            View::Water => s.water,
            View::Weather => s.weather,
            View::Prayer => s.prayer,
            View::Stats => s.tab_stats,
            View::Shelf => s.tab_shelf,
            View::Todo => s.tab_todo,
            View::Calendar => s.tab_calendar,
            View::Clipboard => s.tab_clipboard,
        }
    }

    /// Switch a tab on or off. The notch always keeps at least one tab, so the last one cannot be switched
    /// off: false when that is why nothing changed. If the tab that is showing goes away, the first one shows.
    pub(crate) fn set_tab(&mut self, view: View, on: bool) -> bool {
        if !on && self.tab_enabled(view) && self.visible_tabs().len() <= 1 {
            return false;
        }
        let s = &mut self.settings;
        *match view {
            View::Player | View::Lyrics => &mut s.tab_music,
            View::Timer => &mut s.timer,
            View::Water => &mut s.water,
            View::Weather => &mut s.weather,
            View::Prayer => &mut s.prayer,
            View::Stats => &mut s.tab_stats,
            View::Shelf => &mut s.tab_shelf,
            View::Todo => &mut s.tab_todo,
            View::Calendar => &mut s.tab_calendar,
            View::Clipboard => &mut s.tab_clipboard,
        } = on;
        self.ensure_view();
        true
    }

    /// Show the first tab if the one that is showing is switched off.
    pub(crate) fn ensure_view(&mut self) {
        if !self.tab_enabled(self.view)
            && let Some((_, first)) = self.visible_tabs().first().copied()
        {
            self.view = first;
        }
    }

    /// Switch to a tab, from a click on it or from the mouse wheel.
    pub(crate) fn go_to_view(&mut self, view: View) {
        if self.view == view {
            return;
        }
        self.stop_editing();
        self.view = view;
        if view == View::Calendar {
            self.cal = calendar::Calendar::new(today_day());
        }
        if view == View::Weather {
            self.weather_opened();
        }
        if view == View::Prayer {
            self.prayer_opened();
        }
        self.last_tick = Instant::now();
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Whether the mouse wheel changes the tab where the mouse is: always over the tab bar, and over
    /// the tabs whose content has no scrolling of its own.
    pub(crate) fn wheel_changes_tab(&self) -> bool {
        let over_bar = self.cursor.1 as f32 / self.ui_scale() as f32 >= self.window_h() - TAB_BOTTOM - 4.0;
        over_bar || matches!(self.view, View::Player | View::Water | View::Weather | View::Prayer | View::Stats | View::Calendar)
    }

    /// The tab bar: a dark pill with a round button for each tab. A light circle slides to the open tab,
    /// and the icons are grey, white on the open one.
    pub(crate) fn draw_tabs(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let a = (alpha * 255.0) as u8;
        let tabs = self.visible_tabs();
        let bar_y = pm.height() as f32 / s;
        let (x0, y, _, _) = tab_rect(0, tabs.len(), bar_y);
        let dx = self.tab_dx();
        let total = tabs.len() as f32 * TAB_W + (tabs.len() as f32 - 1.0) * TAB_GAP;
        // The pill.
        let (px, py, pw) = ((x0 + dx - 6.0) * s, (bar_y - TAB_BOTTOM) * s, (total + 12.0) * s);
        let ph = TAB_PILL_H * s;
        fill_rounded(pm, px, py, pw, ph, ph / 2.0, [255, 255, 255, (alpha * 13.0) as u8]);
        gfx::stroke_rounded(pm, px + 0.5 * s, py + 0.5 * s, pw - s, ph - s, ph / 2.0, s, [255, 255, 255, (alpha * 12.0) as u8]);
        if self.tab_target().is_some() {
            // The circle stretches a little in the direction it is moving, like a drop of liquid.
            let stretch = (self.tab_bar.vel.abs() * 2.4).min(18.0);
            let cx = x0 + dx + self.tab_bar.pos * (TAB_W + TAB_GAP) + TAB_W / 2.0;
            let squash = stretch * 0.22;
            let (bw, bh) = (TAB_BUTTON + stretch, TAB_BUTTON - squash);
            let by = y + squash / 2.0;
            // A faint halo trails it, then the circle itself.
            fill_rounded(pm, (cx - bw / 2.0 - 2.0) * s, (by - 2.0) * s, (bw + 4.0) * s, (bh + 4.0) * s, (bh + 4.0) * s / 2.0, [255, 255, 255, (alpha * 8.0) as u8]);
            fill_rounded(pm, (cx - bw / 2.0) * s, by * s, bw * s, bh * s, bh * s / 2.0, [255, 255, 255, (alpha * 36.0) as u8]);
        }
        for (i, (_, view)) in tabs.iter().enumerate() {
            let rect = tab_rect(i, tabs.len(), bar_y);
            let rect = (rect.0 + dx, rect.1, rect.2, rect.3);
            let near = 1.0 - (self.tab_bar.pos - i as f32).abs().min(1.0);
            // The mouse is compared in the view's coordinates, which are this much higher.
            let glow = self.hot_t((rect.0, rect.1 + TAB_SHIFT, rect.2, rect.3), s) * (1.0 - near * 2.0).clamp(0.0, 1.0);
            let cx = rect.0 + rect.2 / 2.0;
            if glow > 0.01 {
                fill_rounded(pm, (cx - TAB_BUTTON / 2.0) * s, rect.1 * s, TAB_BUTTON * s, rect.3 * s, rect.3 * s / 2.0, [255, 255, 255, (alpha * 16.0 * glow) as u8]);
            }
            // Grey icons, white on the open tab.
            let color = gfx::mix([150, 150, 158, a], [255, 255, 255, a], near);
            let hover = self.hot_t((rect.0, rect.1 + TAB_SHIFT, rect.2, rect.3), s);
            let grow = 1.0 + 0.14 * near.clamp(0.0, 1.0) + 0.06 * hover;
            draw_tab_icon(pm, *view, cx * s, (rect.1 + rect.3 / 2.0) * s, s * 0.92 * grow, color);
        }
    }

    /// A click on one of the tabs goes to that view. True when it was on a tab.
    pub(crate) fn click_tab_bar(&mut self, cx: f32, ry: f32, bar_y: f32) -> bool {
        let dx = self.tab_dx();
        let tabs = self.visible_tabs();
        for (i, (_, v)) in tabs.iter().enumerate() {
            let r = tab_rect(i, tabs.len(), bar_y);
            let r = (r.0 + dx, r.1, r.2, r.3);
            if cx >= r.0 && cx <= r.0 + r.2 && ry >= r.1 && ry <= r.1 + r.3 {
                self.go_to_view(*v);
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tab bar's pill sits at the bottom of the notch, and the shelf's Clear button beside it, clear of
    /// the pill and inside the notch.
    #[test]
    fn the_bottom_bar_and_the_clear_button_have_their_places() {
        let height = EXPANDED.1 as f32;
        let (first, last) = (tab_rect(0, TABS.len(), height), tab_rect(TABS.len() - 1, TABS.len(), height));
        assert!(first.1 + first.3 <= height - 4.0, "the tabs stay inside the bottom edge");
        let clear = shelf_clear_rect(height);
        assert!(clear.0 > last.0 + last.2 + 6.0, "Clear is clear of the last tab");
        assert!(clear.0 + clear.2 <= EXPANDED.0 as f32 - 20.0 && clear.1 + clear.3 <= height - 4.0);
        // Level with the pill: the same centre.
        assert!((clear.1 + clear.3 / 2.0 - (first.1 + first.3 / 2.0)).abs() < 0.5);
    }

    /// The whole tab bar has to fit in the notch.
    #[test]
    fn tab_bar_fits_the_notch() {
        let total = TABS.len() as f32 * TAB_W + (TABS.len() as f32 - 1.0) * TAB_GAP;
        assert!(total <= EXPANDED.0 as f32 - 8.0, "tab bar is {total}px wide");
    }
}
