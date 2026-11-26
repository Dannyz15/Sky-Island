//! Mouse clicks: the common part (the alarm, the tab bar, the editor), then the view that is showing.
//! What a click does inside each view is in that view's file (`click_todo` in todo.rs and so on).

use crate::*;

impl App {
    pub(crate) fn click(&mut self) {
        if self.spring.progress < 0.9 || !self.has_notch() {
            return;
        }
        let s = self.ui_scale() as f32;
        // `ry` is the mouse in the window; `cy` is the same spot in the view's own coordinates.
        let (cx, ry) = (self.pointer.cursor.0 as f32 / s, self.pointer.cursor.1 as f32 / s);
        let cy = ry + TAB_SHIFT;
        let bar_y = self.window_h();

        if self.click_alarm_guard(cx, cy, ry, bar_y) {
            return;
        }

        // A click anywhere finishes the number being typed; it is then handled as usual.
        if self.timer_entry.field.is_some() {
            self.end_timer_edit(true);
        }

        // The tab bar, the shelf's Clear button beside it, and the editor that covers the whole notch.
        if self.click_tab_bar(cx, ry, bar_y) || self.click_shelf_clear(cx, ry, bar_y) || self.click_editor(cx, cy) {
            return;
        }

        match self.view {
            View::Todo => self.click_todo(cx, cy),
            View::Calendar => self.click_calendar(cx, cy),
            View::Clipboard => self.click_clipboard(cx, cy),
            View::Shelf => self.click_shelf(cx, cy),
            View::Water => self.click_water(cx, cy),
            View::Weather => self.click_weather(cx, cy),
            View::Prayer => self.click_prayer(cx, cy),
            View::Stats => {}
            View::Timer => self.click_timer(cx, cy),
            View::Player | View::Lyrics => self.click_player(cx, cy),
        }
    }
}
