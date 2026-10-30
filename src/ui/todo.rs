//! The To-do view and the task editor.

use crate::*;

// To-do view, in logical pixels.
const TODO_FIELD: (f32, f32, f32, f32) = (24.0, 34.0, 372.0, 24.0);

const TODO_ROWS_Y: f32 = 64.0;

const TODO_ROW_H: f32 = 24.0;

pub(crate) const TODO_AMBER: [u8; 3] = [255, 190, 70];

impl App {
    /// Open the editor for a new task, or for the task at `target`. Remember which app had focus,
    /// then take it so key presses reach us.
    pub(crate) fn start_editing(&mut self, target: Option<usize>) {
        use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
        let fg = unsafe { GetForegroundWindow() };
        self.prev_foreground = (!fg.is_invalid()).then_some(fg);
        let clock = clock_now();
        match target.and_then(|i| self.todo.tasks.get(i).map(|task| (i, task))) {
            Some((i, task)) => {
                self.editor = editor::Editor::for_task(i, task, clock);
                self.input = task.text.clone();
            }
            None => {
                // A task added on the calendar starts on the selected day.
                let date = (self.view == View::Calendar).then_some(self.cal.selected);
                self.editor = editor::Editor::new_task(date, clock);
                self.input.clear();
            }
        }
        self.editing = true;
        self.edit_started = Instant::now();
        if let Some(w) = &self.window {
            w.focus_window();
        }
    }

    /// Stop typing and give focus back to the app the user was in.
    pub(crate) fn stop_editing(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;
        if !self.editing {
            return;
        }
        self.editing = false;
        self.last_tick = Instant::now();
        if let Some(h) = self.prev_foreground.take() {
            let _ = unsafe { SetForegroundWindow(h) };
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Save the task in the editor. Blank text is refused and the editor stays open.
    pub(crate) fn commit_task(&mut self) {
        let (due, date) = (self.editor.due(clock_now()), self.editor.date);
        let saved = match self.editor.index {
            Some(i) => self.todo.update(i, &self.input, due, date),
            None => self.todo.add(&self.input, due, date),
        };
        if saved {
            self.stop_editing();
        }
    }

    pub(crate) fn draw_todo(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let Some(font) = &self.font else { return };
        let heading = self.heading_font().unwrap_or(font);
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [165, 165, 175, a]);
        let rgb = TODO_AMBER;
        let amber = [rgb[0], rgb[1], rgb[2], a];
        let width = EXPANDED.0 as f32 * s;
        let (now, today, offset) = (todo::unix_now(), today_day(), tz_offset());

        if self.editing {
            self.draw_editor(pm, s, alpha);
            return;
        }

        // "Add a task" field, which also shows a banner when a reminder fires.
        let field = TODO_FIELD;
        let (fx, fy, fw, fh) = field;
        let hot = self.hot_t(field, s);
        if self.alert_until.is_some() {
            gfx::gradient_rounded(pm, fx * s, fy * s, fw * s, fh * s, 12.0 * s, [rgb[0], rgb[1], rgb[2], (alpha * 70.0) as u8], [rgb[0], rgb[1], rgb[2], (alpha * 36.0) as u8], false);
            gfx::stroke_rounded(pm, fx * s + 0.5 * s, fy * s + 0.5 * s, fw * s - s, fh * s - s, 12.0 * s, s, [rgb[0], rgb[1], rgb[2], (alpha * 120.0) as u8]);
            draw_text(pm, heading, &self.alert_msg, (fx + 12.0) * s, (fy + 16.0) * s, 12.0 * s, white, (fw - 24.0) * s);
        } else {
            draw_card(pm, field, 12.0, s, alpha, hot, rgb);
            gfx::gradient_circle(pm, (fx + 15.0) * s, (fy + fh / 2.0) * s, 8.0 * s, gfx::mix(amber, white, 0.3), amber);
            fill_rect(pm, (fx + 15.0 - 4.0) * s, (fy + fh / 2.0 - 0.75) * s, 8.0 * s, 1.5 * s, [0, 0, 0, a]);
            fill_rect(pm, (fx + 15.0 - 0.75) * s, (fy + fh / 2.0 - 4.0) * s, 1.5 * s, 8.0 * s, [0, 0, 0, a]);
            draw_text(pm, font, "Add a task", (fx + 30.0) * s, (fy + 16.0) * s, 12.0 * s, pick(white, grey, hot), (fw - 44.0) * s);
        }

        if self.todo.tasks.is_empty() {
            gfx::glow(pm, width / 2.0, 98.0 * s, 70.0 * s, [rgb[0], rgb[1], rgb[2], (alpha * 36.0) as u8], None);
            draw_tab_icon(pm, View::Todo, width / 2.0, 90.0 * s, s * 2.4, [rgb[0], rgb[1], rgb[2], a]);
            draw_text_centered(pm, heading, "Nothing to do", width / 2.0, 126.0 * s, 15.0 * s, white, width - 80.0 * s);
            draw_text_centered(pm, font, "Click the box above to add a task", width / 2.0, 146.0 * s, 11.0 * s, grey, width - 80.0 * s);
            return;
        }

        for row in 0..todo::VISIBLE {
            self.animated_row(pm, s, row, todo::VISIBLE, |pm| {
                let Some(i) = self.todo.index_at(row) else { return };
                let task = &self.todo.tasks[i];
                let y = TODO_ROWS_Y + row as f32 * TODO_ROW_H;
                let rect = (24.0, y + 1.0, 372.0, TODO_ROW_H - 2.0);
                let hot = self.hot_t(rect, s);
                if hot > 0.01 {
                    gfx::gradient_rounded(pm, rect.0 * s, rect.1 * s, rect.2 * s, rect.3 * s, 8.0 * s, [255, 255, 255, (alpha * 40.0 * hot) as u8], [255, 255, 255, (alpha * 20.0 * hot) as u8], false);
                    gfx::stroke_rounded(pm, rect.0 * s + 0.5 * s, rect.1 * s + 0.5 * s, rect.2 * s - s, rect.3 * s - s, 8.0 * s, s, [rgb[0], rgb[1], rgb[2], (alpha * 70.0 * hot) as u8]);
                }

                // Checkbox.
                draw_check(pm, 28.0 * s, (y + 5.0) * s, 14.0 * s, task.done, amber, a);

                // When it is due (or its day) as a small tag, so the text only gets the room that is left.
                let when = task_when(task, today, now, offset, self.settings.clock_24h);
                let tag_w = when.as_ref().map_or(0.0, |(label, _)| text_width(font, label, 10.0 * s) / s + 12.0);
                let text_w = ((346.0 - 52.0 - 8.0) * s - tag_w * s).min(250.0 * s);

                // Text, struck through when done.
                let color = if task.done { [120, 120, 130, a] } else { white };
                draw_text(pm, font, &task.text, 52.0 * s, (y + 16.0) * s, 12.0 * s, color, text_w);
                if task.done {
                    let w = text_width(font, &task.text, 12.0 * s).min(text_w);
                    fill_rect(pm, 52.0 * s, (y + 12.0) * s, w, 1.2 * s, color);
                }
                if let Some((label, overdue)) = &when {
                    let (tint, ink) = if task.done {
                        ([255, 255, 255], [120, 120, 130, a])
                    } else if *overdue {
                        ([255, 96, 86], [255, 130, 120, a])
                    } else if task.due.is_some() {
                        (rgb, [255, 214, 140, a])
                    } else {
                        ([255, 255, 255], [165, 165, 175, a])
                    };
                    let (tx, tw) = (346.0 - tag_w, tag_w);
                    fill_rounded(pm, tx * s, (y + 4.5) * s, tw * s, 15.0 * s, 7.5 * s, [tint[0], tint[1], tint[2], (alpha * if tint == [255, 255, 255] { 16.0 } else { 40.0 }) as u8]);
                    draw_text_centered(pm, font, label, (tx + tw / 2.0) * s, (y + 15.5) * s, 10.0 * s, ink, tw * s);
                }

                // Edit and remove buttons.
                let over_edit = self.hot_t((346.0, y, 26.0, TODO_ROW_H), s);
                let over_remove = self.hot_t((372.0, y, 24.0, TODO_ROW_H), s);
                draw_pencil(pm, 358.0 * s, (y + 12.0) * s, s, pick(white, [120, 120, 130, a], over_edit));
                draw_text_centered(pm, font, "\u{00D7}", 384.0 * s, (y + 17.0) * s, 13.0 * s, pick([255, 120, 110, a], grey, over_remove), 14.0 * s);
            });
        }

        // Scroll indicator when there are more tasks than rows.
        let total = self.todo.tasks.len();
        if total > todo::VISIBLE {
            let (top, height) = (TODO_ROWS_Y, TODO_ROW_H * todo::VISIBLE as f32);
            fill_rounded(pm, 410.0 * s, top * s, 3.0 * s, height * s, 1.5 * s, [255, 255, 255, (alpha * 22.0) as u8]);
            let thumb = height * todo::VISIBLE as f32 / total as f32;
            let at = top + (height - thumb) * self.todo.scroll as f32 / (total - todo::VISIBLE) as f32;
            fill_rounded(pm, 410.0 * s, at * s, 3.0 * s, thumb * s, 1.5 * s, amber);
        }
    }

    /// The task editor: name, day, reminder time and quick choices.
    pub(crate) fn draw_editor(&self, pm: &mut Pixmap, s: f32, alpha: f32) {
        let Some(font) = &self.font else { return };
        let a = (alpha * 255.0) as u8;
        let (white, grey) = ([255, 255, 255, a], [165, 165, 175, a]);
        let rgb = TODO_AMBER;
        let amber = [rgb[0], rgb[1], rgb[2], a];
        let (clock, ed, clock_24h) = (clock_now(), &self.editor, self.settings.clock_24h);
        // A pill that is lit when chosen, and brighter under the mouse.
        let pill = |pm: &mut Pixmap, action: editor::Action, on: bool| {
            let r = editor::rect(action);
            let (x, y, w, h) = (r.0 * s, r.1 * s, r.2 * s, r.3 * s);
            if on {
                gfx::gradient_rounded(pm, x, y, w, h, h / 2.0, [rgb[0], rgb[1], rgb[2], (alpha * 80.0) as u8], [rgb[0], rgb[1], rgb[2], (alpha * 38.0) as u8], false);
                gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [rgb[0], rgb[1], rgb[2], (alpha * 110.0) as u8]);
            } else {
                let boost = pick(14.0, 0.0, self.hot_t(r, s));
                fill_rounded(pm, x, y, w, h, h / 2.0, [255, 255, 255, (alpha * (16.0 + boost)) as u8]);
            }
        };
        let label = |pm: &mut Pixmap, action: editor::Action, text: &str, color: [u8; 4]| {
            let r = editor::rect(action);
            draw_text_centered(pm, font, text, (r.0 + r.2 / 2.0) * s, (r.1 + 16.0) * s, 11.0 * s, color, r.2 * s);
        };

        // The task's name, in a field that glows while you type.
        let (fx, fy, fw, fh) = editor::FIELD;
        gfx::glow(pm, (fx + fw / 2.0) * s, (fy + fh / 2.0) * s, 150.0 * s, [rgb[0], rgb[1], rgb[2], (alpha * 18.0) as u8], None);
        gfx::gradient_rounded(pm, fx * s, fy * s, fw * s, fh * s, 10.0 * s, [255, 255, 255, (alpha * 36.0) as u8], [255, 255, 255, (alpha * 20.0) as u8], false);
        gfx::stroke_rounded(pm, fx * s + 0.5 * s, fy * s + 0.5 * s, fw * s - s, fh * s - s, 10.0 * s, s, [rgb[0], rgb[1], rgb[2], (alpha * 140.0) as u8]);
        let (tx, ty, max_w) = ((fx + 12.0) * s, (fy + 17.0) * s, (fw - 24.0) * s);
        let shown = tail_fit(font, &self.input, 13.0 * s, max_w - 8.0 * s);
        if shown.is_empty() {
            draw_text(pm, font, "What needs doing?", tx, ty, 13.0 * s, [135, 135, 148, a], max_w);
        } else {
            draw_text(pm, font, &shown, tx, ty, 13.0 * s, white, max_w);
        }
        if (self.edit_started.elapsed().as_millis() / 500).is_multiple_of(2) {
            let x = tx + text_width(font, &shown, 13.0 * s) + 1.0 * s;
            fill_rect(pm, x, (fy + 5.0) * s, 1.5 * s, 16.0 * s, amber);
        }

        // The day: arrows around the date, then quick choices.
        draw_text(pm, font, "Date", 24.0 * s, (editor::DATE_Y + 16.0) * s, 11.0 * s, grey, 50.0 * s);
        for (action, dir) in [(editor::Action::DatePrev, -1.0), (editor::Action::DateNext, 1.0)] {
            let r = editor::rect(action);
            let (cx, cy) = ((r.0 + r.2 / 2.0) * s, (r.1 + r.3 / 2.0) * s);
            fill_circle(pm, cx, cy, 9.0 * s, [255, 255, 255, (alpha * pick(34.0, 16.0, self.hot_t(r, s))) as u8]);
            let tip = [(cx - 2.0 * dir * s, cy - 4.0 * s), (cx + 2.0 * dir * s, cy), (cx - 2.0 * dir * s, cy + 4.0 * s)];
            stroke_poly(pm, &tip, 1.6 * s, white, false);
        }
        let dt = editor::DATE_TEXT;
        let (text, color) = match ed.date {
            Some(day) => (calendar::long_date(day), if day == clock.today { amber } else { white }),
            None => ("No date".to_string(), grey),
        };
        draw_text_centered(pm, self.heading_font().unwrap_or(font), &text, (dt.0 + dt.2 / 2.0) * s, (dt.1 + 16.0) * s, 12.0 * s, color, dt.2 * s);
        for (action, name, on) in [
            (editor::Action::DateNone, "None", ed.date.is_none()),
            (editor::Action::DateToday, "Today", ed.date == Some(clock.today)),
            (editor::Action::DateTomorrow, "Tomorrow", ed.date == Some(clock.today + 1)),
        ] {
            pill(pm, action, on);
            label(pm, action, name, if on { white } else { grey });
        }

        // The reminder: a switch, then hour and minute steppers.
        draw_text(pm, font, "Remind", 24.0 * s, (editor::REMIND_Y + 16.0) * s, 11.0 * s, grey, 54.0 * s);
        draw_switch(pm, 82.0 * s, (editor::REMIND_Y + 3.0) * s, s, self.toggle_t(100, ed.remind), a);
        if ed.remind {
            for (action, plus) in [
                (editor::Action::HourDown, false),
                (editor::Action::HourUp, true),
                (editor::Action::MinuteDown, false),
                (editor::Action::MinuteUp, true),
            ] {
                let r = editor::rect(action);
                draw_step(pm, r, plus, s, alpha, self.hot_t(r, s));
            }
            let (hour, minute) = (ed.minutes / 60, ed.minutes % 60);
            let hour_text = if clock_24h { format!("{hour:02}") } else { format!("{}", if hour % 12 == 0 { 12 } else { hour % 12 }) };
            let clock_font = self.heading_font().unwrap_or(font);
            for (r, text) in [(editor::HOUR_TEXT, hour_text), (editor::MINUTE_TEXT, format!("{minute:02}"))] {
                draw_text_centered(pm, clock_font, &text, (r.0 + r.2 / 2.0) * s, (r.1 + 17.0) * s, 15.0 * s, white, r.2 * s);
            }
            draw_text_centered(pm, font, ":", 194.0 * s, (editor::REMIND_Y + 16.0) * s, 15.0 * s, grey, 8.0 * s);
            if !clock_24h {
                pill(pm, editor::Action::Meridiem, true);
                label(pm, editor::Action::Meridiem, if hour < 12 { "AM" } else { "PM" }, white);
            }
            let (status, past) = ed.status(clock);
            let c = if past { [255, 130, 120, a] } else { [255, 214, 140, a] };
            draw_text(pm, font, &status, 396.0 * s - text_width(font, &status, 10.0 * s), (editor::REMIND_Y + 16.0) * s, 10.0 * s, c, 90.0 * s);
        } else {
            draw_text(pm, font, "No reminder", 126.0 * s, (editor::REMIND_Y + 16.0) * s, 11.0 * s, [135, 135, 148, a], 120.0 * s);
        }

        // Quick choices, then cancel and save.
        for (action, name) in [
            (editor::Action::In(15), "In 15m"),
            (editor::Action::In(60), "In 1h"),
            (editor::Action::In(180), "In 3h"),
            (editor::Action::TomorrowMorning, "Tomorrow 9:00"),
            (editor::Action::Cancel, "Cancel"),
        ] {
            pill(pm, action, false);
            label(pm, action, name, grey);
        }
        let r = editor::rect(editor::Action::Save);
        let hot = self.hot_t(r, s);
        gfx::glow(pm, (r.0 + r.2 / 2.0) * s, (r.1 + r.3 / 2.0) * s, 46.0 * s, [rgb[0], rgb[1], rgb[2], (alpha * pick(80.0, 46.0, hot)) as u8], None);
        gfx::gradient_rounded(pm, r.0 * s, r.1 * s, r.2 * s, r.3 * s, r.3 * s / 2.0, gfx::mix(amber, white, pick(0.4, 0.25, hot)), amber, false);
        label(pm, editor::Action::Save, if ed.index.is_some() { "Save" } else { "Add" }, [20, 14, 0, a]);
    }

    /// A click in the To-do view. (`cx`, `cy`) is the spot in the view's own coordinates.
    pub(crate) fn click_todo(&mut self, cx: f32, cy: f32, window: &Window) {
        let inside = |r: (f32, f32, f32, f32)| cx >= r.0 && cx <= r.0 + r.2 && cy >= r.1 && cy <= r.1 + r.3;
        if inside(TODO_FIELD) {
            window.request_redraw();
            self.start_editing(None);
            return;
        }
        let row = ((cy - TODO_ROWS_Y) / TODO_ROW_H).floor();
        let Some(idx) = (row >= 0.0).then(|| self.todo.index_at(row as usize)).flatten() else { return };
        if cx <= 46.0 {
            self.todo.toggle(idx);
        } else if cx >= 372.0 {
            self.todo.remove(idx);
        } else {
            // Anywhere else on a task opens it for editing.
            self.start_editing(Some(idx));
        }
        window.request_redraw();
    }

    /// The editor panel covers the whole notch while a task is being added or changed. True when it took the click.
    pub(crate) fn click_editor(&mut self, cx: f32, cy: f32, window: &Window) -> bool {
        if !self.editing {
            return false;
        }
        if let Some(action) = editor::hit(cx, cy, self.editor.remind, self.settings.clock_24h) {
            match self.editor.apply(action, clock_now()) {
                editor::Outcome::Save => self.commit_task(),
                editor::Outcome::Cancel => self.stop_editing(),
                editor::Outcome::Continue => {}
            }
        }
        window.request_redraw();
        true
    }
}
