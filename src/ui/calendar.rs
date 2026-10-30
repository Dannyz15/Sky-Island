//! The Calendar view, in month and week form.

use crate::*;

const KEY_CAL_SELECTED: u64 = 0x4000_0000_0000_0010;

// Calendar tab layout, in logical pixels: month grid on the left, the chosen day's tasks on the right.
// The month heading stays at the top; the weekday letters and the dates sit lower, with a gap between.
pub(crate) const CAL_GRID: (f32, f32) = (24.0, 65.0);

pub(crate) const CAL_CELL: (f32, f32) = (24.0, 16.0);

pub(crate) const CAL_HEADS_Y: f32 = 62.0; // baseline of the weekday letters
pub(crate) const CAL_PREV: (f32, f32, f32, f32) = (24.0, 30.0, 24.0, 16.0);

pub(crate) const CAL_NEXT: (f32, f32, f32, f32) = (168.0, 30.0, 24.0, 16.0);

pub(crate) const CAL_TITLE: (f32, f32, f32, f32) = (52.0, 30.0, 112.0, 16.0);

/// The day panel on the right, from this x to `CAL_PANEL_X + CAL_PANEL_W`.
pub(crate) const CAL_PANEL_X: f32 = 216.0;

pub(crate) const CAL_PANEL_W: f32 = 180.0;

pub(crate) const CAL_ROWS_Y: f32 = 60.0;

pub(crate) const CAL_ROW_H: f32 = 20.0;

pub(crate) const CAL_VISIBLE: usize = 4;

pub(crate) const CAL_ADD: (f32, f32, f32, f32) = (CAL_PANEL_X, 142.0, CAL_PANEL_W, 18.0);

/// A day's row in the week view, which fills the place of the month grid.
pub(crate) fn cal_week_rect(i: usize) -> (f32, f32, f32, f32) {
    (CAL_GRID.0, CAL_GRID.1 - 14.0 + i as f32 * 15.0, 7.0 * CAL_CELL.0, 14.0)
}

pub(crate) fn cal_cell_rect(i: usize) -> (f32, f32, f32, f32) {
    let (col, row) = ((i % 7) as f32, (i / 7) as f32);
    (CAL_GRID.0 + col * CAL_CELL.0, CAL_GRID.1 + row * CAL_CELL.1, CAL_CELL.0, CAL_CELL.1)
}

impl App {
    /// One week as seven rows, one per day: its name, its date, and a dot for each task.
    pub(crate) fn draw_calendar_week(&self, pm: &mut Pixmap, s: f32, alpha: f32, monday: bool, today: i64) {
        let Some(font) = &self.font else { return };
        let heading = self.heading_font().unwrap_or(font);
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [165, 165, 175, a]);
        let amber = [TODO_AMBER[0], TODO_AMBER[1], TODO_AMBER[2], a];
        let rgb = self.accent(View::Calendar);
        let violet = [rgb[0], rgb[1], rgb[2], a];
        const NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        let start = calendar::week_start(self.cal.selected, monday);
        for i in 0..7 {
            let day = start + i as i64;
            let rect = cal_week_rect(i);
            let (x, y, w, h) = rect;
            let is_today = day == today;
            if day == self.cal.selected {
                gfx::gradient_rounded(pm, x * s, y * s, w * s, h * s, 6.0 * s, [255, 255, 255, (alpha * 44.0) as u8], [255, 255, 255, (alpha * 22.0) as u8], false);
                gfx::stroke_rounded(pm, x * s + 0.5 * s, y * s + 0.5 * s, w * s - s, h * s - s, 6.0 * s, s, [255, 255, 255, (alpha * 90.0) as u8]);
            } else {
                let glow = self.hot_t(rect, s);
                if glow > 0.01 {
                    fill_rounded(pm, x * s, y * s, w * s, h * s, 6.0 * s, [255, 255, 255, (alpha * 16.0 * glow) as u8]);
                }
            }
            if is_today {
                gfx::gradient_rounded(pm, x * s + 2.0 * s, y * s + 1.0 * s, 32.0 * s, (h - 2.0) * s, 5.0 * s, gfx::mix(violet, white, 0.25), violet, false);
            }
            let (_, _, dom) = calendar::civil_from_days(day);
            let name = NAMES[calendar::weekday(day)];
            let weekend = calendar::weekday(day) >= 5;
            let ink = if is_today { white } else if weekend { [190, 175, 225, a] } else { grey };
            draw_text_centered(pm, font, name, (x + 17.0) * s, (y + 10.5) * s, 9.0 * s, ink, 30.0 * s);
            draw_text(pm, if is_today { heading } else { font }, &dom.to_string(), (x + 42.0) * s, (y + 10.5) * s, 10.0 * s, if weekend && !is_today { [190, 175, 225, a] } else { white }, 24.0 * s);
            let (open, total) = self.todo.day_counts(day);
            for k in 0..total.min(8) {
                let colour = if k < open { amber } else { [110, 110, 120, a] };
                fill_circle(pm, (x + 70.0 + k as f32 * 6.0) * s, (y + h / 2.0) * s, 1.8 * s, colour);
            }
        }
    }

    pub(crate) fn draw_calendar(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let Some(font) = &self.font else { return };
        let heading = self.heading_font().unwrap_or(font);
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [165, 165, 175, a]);
        let amber = [TODO_AMBER[0], TODO_AMBER[1], TODO_AMBER[2], a];
        let rgb = self.accent(View::Calendar);
        let violet = [rgb[0], rgb[1], rgb[2], a];
        let (today, now, offset) = (today_day(), todo::unix_now(), tz_offset());
        let selected = self.cal.selected;

        if self.editing {
            self.draw_editor(pm, s, alpha);
            return;
        }

        // Month heading with previous / next arrows.
        for (rect, dir) in [(CAL_PREV, -1.0), (CAL_NEXT, 1.0)] {
            let (cx, cy) = ((rect.0 + rect.2 / 2.0) * s, (rect.1 + rect.3 / 2.0) * s);
            let glow = self.hot_t(rect, s);
            if glow > 0.01 {
                fill_circle(pm, cx, cy, 9.0 * s, [255, 255, 255, (alpha * 22.0 * glow) as u8]);
            }
            let tip = [(cx - 2.0 * dir * s, cy - 4.0 * s), (cx + 2.0 * dir * s, cy), (cx - 2.0 * dir * s, cy + 4.0 * s)];
            stroke_poly(pm, &tip, 1.7 * s, pick(white, grey, glow), false);
        }
        let (monday, week) = (self.settings.week_monday, self.settings.calendar_week);
        let title = if week { calendar::week_title(calendar::week_start(selected, monday)) } else { self.cal.title() };
        draw_text_centered(pm, heading, &title, (CAL_TITLE.0 + CAL_TITLE.2 / 2.0) * s, (CAL_TITLE.1 + 13.0) * s, 12.5 * s, white, CAL_TITLE.2 * s);
        let grid = |pm: &mut Pixmap| {
        if week {
            self.draw_calendar_week(pm, s, alpha, monday, today);
            return;
        }
        for (i, head) in calendar::week_heads(monday).iter().enumerate() {
            let x = CAL_GRID.0 + (i as f32 + 0.5) * CAL_CELL.0;
            let ink = if calendar::is_weekend(i, monday) { gfx::mix(grey, violet, 0.45) } else { [130, 130, 142, a] };
            draw_text_centered(pm, font, head, x * s, CAL_HEADS_Y * s, 9.0 * s, ink, CAL_CELL.0 * s);
        }

        // Month grid: the selected day is outlined (the outline glides from day to day), today is filled, and
        // a dot marks days with tasks.
        if let Some(i) = (0..calendar::CELLS).find(|&i| self.cal.cell(i, monday) == Some(selected)) {
            let (x, y, w, h) = cal_cell_rect(i);
            let gx = self.glows.follow(KEY_CAL_SELECTED, x as f64, 30.0, 0.78) as f32;
            let gy = self.glows.follow(KEY_CAL_SELECTED + 1, y as f64, 30.0, 0.78) as f32;
            gfx::gradient_rounded(pm, (gx + 1.0) * s, gy * s, (w - 2.0) * s, h * s, 6.0 * s, [255, 255, 255, (alpha * 44.0) as u8], [255, 255, 255, (alpha * 22.0) as u8], false);
            gfx::stroke_rounded(pm, (gx + 1.0) * s + 0.5 * s, gy * s + 0.5 * s, (w - 2.0) * s - s, h * s - s, 6.0 * s, s, [255, 255, 255, (alpha * 90.0) as u8]);
        }
        for i in 0..calendar::CELLS {
            let Some(day) = self.cal.cell(i, monday) else { continue };
            let rect = cal_cell_rect(i);
            let (x, y, w, h) = rect;
            let is_today = day == today;
            if day != selected {
                let glow = self.hot_t(rect, s);
                if glow > 0.01 {
                    fill_rounded(pm, (x + 1.0) * s, y * s, (w - 2.0) * s, h * s, 6.0 * s, [255, 255, 255, (alpha * 16.0 * glow) as u8]);
                }
            }
            if is_today {
                gfx::glow(pm, (x + w / 2.0) * s, (y + h / 2.0) * s, 16.0 * s, [rgb[0], rgb[1], rgb[2], (alpha * 70.0) as u8], None);
                gfx::gradient_rounded(pm, (x + 3.0) * s, (y + 1.0) * s, (w - 6.0) * s, (h - 2.0) * s, 5.0 * s, gfx::mix(violet, white, 0.25), violet, false);
            }
            let (_, _, dom) = calendar::civil_from_days(day);
            let weekend = calendar::is_weekend(i % 7, monday);
            let ink = if is_today { white } else if weekend { [190, 175, 225, a] } else { white };
            draw_text_centered(pm, if is_today { heading } else { font }, &dom.to_string(), (x + w / 2.0) * s, (y + 11.5) * s, 10.0 * s, ink, w * s);
            let (open, total) = self.todo.day_counts(day);
            if total > 0 {
                let dot = if open > 0 { amber } else { [110, 110, 120, a] };
                fill_circle(pm, (x + w / 2.0) * s, (y + 14.3) * s, 1.2 * s, if is_today { white } else { dot });
            }
        }
        };
        self.slide_in(pm, s, grid);

        // The selected day sits in a card: its heading and its tasks.
        draw_card(pm, (CAL_PANEL_X - 8.0, 34.0, CAL_PANEL_W + 16.0, 126.0), 12.0, s, alpha, 0.0, rgb);
        draw_text(pm, heading, &calendar::long_date(selected), CAL_PANEL_X * s, 48.0 * s, 13.0 * s, white, 120.0 * s);
        let ids = self.todo.on_day(selected);
        let note = if ids.len() > CAL_VISIBLE {
            format!("+{} more", ids.len() - CAL_VISIBLE)
        } else if selected == today {
            "Today".to_string()
        } else {
            String::new()
        };
        if !note.is_empty() {
            let nw = text_width(font, &note, 10.0 * s) / s + 12.0;
            let nx = CAL_PANEL_X + CAL_PANEL_W - nw;
            fill_rounded(pm, nx * s, 38.0 * s, nw * s, 15.0 * s, 7.5 * s, [rgb[0], rgb[1], rgb[2], (alpha * 44.0) as u8]);
            draw_text_centered(pm, font, &note, (nx + nw / 2.0) * s, 49.0 * s, 10.0 * s, gfx::mix(violet, white, 0.5), nw * s);
        }
        if ids.is_empty() {
            draw_tab_icon(pm, View::Calendar, (CAL_PANEL_X + CAL_PANEL_W / 2.0) * s, 88.0 * s, s * 1.6, [rgb[0], rgb[1], rgb[2], (alpha * 150.0) as u8]);
            draw_text_centered(pm, font, "No tasks", (CAL_PANEL_X + CAL_PANEL_W / 2.0) * s, 114.0 * s, 12.0 * s, grey, CAL_PANEL_W * s);
        }
        for (row, &i) in ids.iter().take(CAL_VISIBLE).enumerate() {
            self.animated_row(pm, s, row, CAL_VISIBLE, |pm| {
                let task = &self.todo.tasks[i];
                let y = CAL_ROWS_Y + row as f32 * CAL_ROW_H;
                let rect = (CAL_PANEL_X - 4.0, y + 1.0, CAL_PANEL_W + 8.0, CAL_ROW_H - 2.0);
                let hot = self.hot_t(rect, s);
                if hot > 0.01 {
                    fill_rounded(pm, rect.0 * s, rect.1 * s, rect.2 * s, rect.3 * s, 6.0 * s, [255, 255, 255, (alpha * 30.0 * hot) as u8]);
                }
                draw_check(pm, CAL_PANEL_X * s, (y + 4.0) * s, 12.0 * s, task.done, amber, a);
                let color = if task.done { [120, 120, 130, a] } else { white };
                // The time of a reminder, so the text only gets the room that is left.
                let time = task.due.map(|due| split_clock(due, now, offset, self.settings.clock_24h, task.done));
                let label_w = time.as_ref().map_or(0.0, |(label, _)| text_width(font, label, 9.0 * s));
                let text_w = (128.0 * s - label_w - if label_w > 0.0 { 6.0 * s } else { 0.0 }).max(30.0 * s);
                draw_text(pm, font, &task.text, (CAL_PANEL_X + 18.0) * s, (y + 14.0) * s, 11.0 * s, color, text_w);
                if task.done {
                    let w = text_width(font, &task.text, 11.0 * s).min(text_w);
                    fill_rect(pm, (CAL_PANEL_X + 18.0) * s, (y + 10.5) * s, w, 1.2 * s, color);
                }
                if let Some((label, overdue)) = &time {
                    let c = if *overdue { [255, 130, 120, a] } else { [255, 214, 140, a] };
                    draw_text(pm, font, label, (CAL_PANEL_X + CAL_PANEL_W - 32.0) * s - label_w, (y + 14.0) * s, 9.0 * s, c, 80.0 * s);
                }
                let over_remove = self.hot_t((CAL_PANEL_X + CAL_PANEL_W - 22.0, y, 30.0, CAL_ROW_H), s);
                let over_edit = (hot - over_remove).max(0.0);
                draw_pencil(pm, (CAL_PANEL_X + CAL_PANEL_W - 24.0) * s, (y + 10.0) * s, s * 0.85, pick(white, [120, 120, 130, a], over_edit));
                draw_text_centered(pm, font, "\u{00D7}", (CAL_PANEL_X + CAL_PANEL_W - 6.0) * s, (y + 15.0) * s, 13.0 * s, pick([255, 120, 110, a], grey, over_remove), 14.0 * s);
            });
        }


        // Add a task for this day.
        let add = CAL_ADD;
        let hot = self.hot_t(add, s);
        let (x, y, w, h) = add;
        gfx::gradient_rounded(pm, x * s, y * s, w * s, h * s, h * s / 2.0, [rgb[0], rgb[1], rgb[2], (alpha * pick(90.0, 56.0, hot)) as u8], [rgb[0], rgb[1], rgb[2], (alpha * pick(56.0, 28.0, hot)) as u8], false);
        gfx::stroke_rounded(pm, x * s + 0.5 * s, y * s + 0.5 * s, w * s - s, h * s - s, h * s / 2.0, s, [rgb[0], rgb[1], rgb[2], (alpha * 100.0) as u8]);
        draw_text_centered(pm, font, "+  Add task", (x + w / 2.0) * s, (y + 13.0) * s, 11.0 * s, white, (w - 16.0) * s);
    }

    /// A click in the Calendar view. (`cx`, `cy`) is the spot in the view's own coordinates.
    pub(crate) fn click_calendar(&mut self, cx: f32, cy: f32, window: &Window) {
        let inside = |r: (f32, f32, f32, f32)| cx >= r.0 && cx <= r.0 + r.2 && cy >= r.1 && cy <= r.1 + r.3;
        let (monday, week) = (self.settings.week_monday, self.settings.calendar_week);
        if inside(CAL_PREV) {
            if week { self.cal.shift_week(-1) } else { self.cal.shift_month(-1) }
            (self.cal_t, self.cal_v, self.cal_dir) = (0.0, 0.0, -1.0);
        } else if inside(CAL_NEXT) {
            if week { self.cal.shift_week(1) } else { self.cal.shift_month(1) }
            (self.cal_t, self.cal_v, self.cal_dir) = (0.0, 0.0, 1.0);
        } else if inside(CAL_TITLE) {
            self.cal = calendar::Calendar::new(today_day());
        } else if inside(CAL_ADD) {
            window.request_redraw();
            self.start_editing(None);
            return;
        } else if week && let Some(i) = (0..7).find(|&i| inside(cal_week_rect(i))) {
            self.cal.selected = calendar::week_start(self.cal.selected, monday) + i as i64;
        } else if !week && let Some(day) = (0..calendar::CELLS).find(|&i| inside(cal_cell_rect(i))).and_then(|i| self.cal.cell(i, monday)) {
            self.cal.selected = day;
        } else {
            let row = ((cy - CAL_ROWS_Y) / CAL_ROW_H).floor();
            if !(CAL_PANEL_X - 4.0..=CAL_PANEL_X + CAL_PANEL_W + 4.0).contains(&cx) || row < 0.0 || row >= CAL_VISIBLE as f32 {
                return;
            }
            let Some(&idx) = self.todo.on_day(self.cal.selected).get(row as usize) else { return };
            if cx <= CAL_PANEL_X + 18.0 {
                self.todo.toggle(idx);
            } else if cx >= CAL_PANEL_X + CAL_PANEL_W - 22.0 {
                self.todo.remove(idx);
            } else {
                self.start_editing(Some(idx));
            }
        }
        window.request_redraw();
    }
}
