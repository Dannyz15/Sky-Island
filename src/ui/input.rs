//! Mouse clicks: the common part (the alarm, the tab bar, the editor), then the view that is showing.
//! What a click does inside each view is in that view's file (`click_todo` in todo.rs and so on).

use crate::*;

impl App {
    pub(crate) fn click(&mut self) {
        if self.progress < 0.9 {
            return;
        }
        let Some(window) = self.window.clone() else { return };
        let s = self.ui_scale() as f32;
        // `ry` is the mouse in the window; `cy` is the same spot in the view's own coordinates.
        let (cx, ry) = (self.cursor.0 as f32 / s, self.cursor.1 as f32 / s);
        let cy = ry + TAB_SHIFT;
        let bar_y = self.window_h();

        if self.click_alarm_guard(cx, cy, ry, bar_y, &window) {
            return;
        }

        // A click anywhere finishes the number being typed; it is then handled as usual.
        if self.timer_field.is_some() {
            self.end_timer_edit(true);
        }

        // The tab bar, the shelf's Clear button beside it, and the editor that covers the whole notch.
        if self.click_tab_bar(cx, ry, bar_y) || self.click_shelf_clear(cx, ry, bar_y, &window) || self.click_editor(cx, cy, &window) {
            return;
        }

        match self.view {
            View::Todo => self.click_todo(cx, cy, &window),
            View::Calendar => self.click_calendar(cx, cy, &window),
            View::Clipboard => self.click_clipboard(cx, cy, &window),
            View::Shelf => self.click_shelf(cx, cy, &window),
            View::Water => self.click_water(cx, cy, &window),
            View::Timer => self.click_timer(cx, cy, &window),
            View::Player | View::Lyrics => self.click_player(cx, cy, &window),
        }
    }
}
