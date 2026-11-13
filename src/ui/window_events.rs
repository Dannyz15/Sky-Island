//! What the app does with the mouse, keys and files over the notch.

use crate::*;

impl App {
    /// The mouse came onto the notch.
    pub(super) fn on_cursor_entered(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        self.pointer.inside = true;
        self.pointer.leave_at = None;
        // With "Open the notch" on Click, resting on it does nothing, except to bring an auto-hidden
        // island back into view.
        let waits_for_click = self.settings.open_on_click && self.spring.tuck_t < 0.1;
        if !self.pointer.hovered && !waits_for_click {
            self.pointer.enter_at = Some(now + self.hover_open());
            event_loop.set_control_flow(ControlFlow::WaitUntil(now + self.hover_open()));
        }
    }

    /// The mouse left the notch.
    pub(super) fn on_cursor_left(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        self.pointer.inside = false;
        self.pointer.enter_at = None; // a fly-by never opened it
        if self.pointer.hovered && self.play.scrub.is_none() && !self.lyric.settle {
            self.pointer.leave_at = Some(now + self.hover_close());
            event_loop.set_control_flow(ControlFlow::WaitUntil(now + self.hover_close()));
        }
    }

    /// The mouse moved over the notch.
    pub(super) fn on_cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        // The window has a flare on each side of the notch (and the floating pill a gap above it); positions count from
        // the notch itself.
        let ear = EAR * self.ui_scale();
        let gap = crate::glass::gap(self.settings.look) * self.ui_scale();
        let position = PhysicalPosition::new(position.x - ear, position.y - gap);
        self.pointer.cursor = (position.x, position.y);
        if self.play.scrub.is_some() {
            let s = self.ui_scale();
            self.play.scrub = Some(self.scrub_at((position.x / s) as f32));
        }
        if self.volume.drag {
            let s = self.ui_scale();
            self.set_volume_at((position.x / s) as f32);
        }
        // Hover highlights follow the mouse.
        if self.spring.progress >= 0.9
            && let Some(w) = &self.window
        {
            w.request_redraw();
        }
        if let Some((idx, start)) = self.shelf_ui.press
            && (position.x - start.0).hypot(position.y - start.1) > DRAG_THRESHOLD
        {
            self.shelf_ui.press = None;
            self.drag_out(idx);
        }
    }

    /// The left button went down on the notch.
    pub(super) fn on_left_pressed(&mut self) {
        if self.settings.open_on_click && !self.pointer.hovered && self.spring.progress < 0.9 {
            // A click on the closed notch opens it.
            self.pointer.hovered = true;
            self.pointer.enter_at = None;
            self.pointer.leave_at = None;
            self.last_tick = Instant::now();
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        } else {
            self.click();
        }
    }

    /// The left button came up.
    pub(super) fn on_left_released(&mut self) {
        self.volume.drag = false;
        if let Some(at) = self.play.scrub.take() {
            if let Some(handle) = &self.play.handle {
                handle.send(Command::Seek(at));
            }
            self.play.seeked = true;
            // Show the jump immediately instead of waiting for the player to report it.
            self.media.position = at;
            self.play.at = Instant::now();
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
        // A press that never turned into a drag is a click: open the file.
        if let Some((idx, _)) = self.shelf_ui.press.take()
            && let Some(path) = self.shelf.items.get(idx).cloned()
        {
            crate::open::open(&path);
            self.shelf_ui.selected.clear();
            self.sync_selection_focus();
        }
    }

    /// The user went to another app while shelf files were highlighted.
    pub(super) fn on_shelf_focus_lost(&mut self) {
        // The user went to another app: the highlight and the keyboard are no longer ours.
        self.shelf_ui.selecting = false;
        self.edit.prev_foreground = None;
        self.shelf_ui.selected.clear();
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// A key pressed while shelf files are highlighted.
    pub(super) fn on_shelf_key(&mut self, event: winit::event::KeyEvent) {
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.shelf_ui.selected.clear(),
            _ => return,
        }
        self.sync_selection_focus();
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// The wheel over the volume control.
    pub(super) fn wheel_volume(&mut self, delta: MouseScrollDelta) {
        // The wheel over the volume control changes the volume, five percent a notch.
        let dy = wheel(&delta, 1.0, 1.0);
        if dy != 0.0 {
            let now = self.volume.level.map_or(0, |(p, _)| p) as i32;
            let next = (now + if dy > 0.0 { 5 } else { -5 }).clamp(0, 100) as u32;
            self.volume.level = Some((next, false));
            sys::set_volume(next);
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
    }

    /// The wheel changes the tab.
    pub(super) fn wheel_tabs(&mut self, delta: MouseScrollDelta) {
        let dy = wheel(&delta, 1.0, 1.0);
        // Scrolling down goes to the next tab, up to the one before; one step per flick.
        if dy != 0.0 && self.pointer.last_wheel_tab.elapsed() > Duration::from_millis(220) {
            let tabs = self.visible_tabs();
            let here = if self.view == View::Lyrics { View::Player } else { self.view };
            if let Some(i) = tabs.iter().position(|(_, v)| *v == here) {
                let next = if dy < 0.0 { (i + 1).min(tabs.len() - 1) } else { i.saturating_sub(1) };
                if next != i {
                    self.pointer.last_wheel_tab = Instant::now();
                    self.go_to_view(tabs[next].1);
                }
            }
        }
    }

    /// The wheel over a column of the timer.
    pub(super) fn wheel_timer(&mut self, delta: MouseScrollDelta) {
        let dy = wheel(&delta, 1.0, 1.0);
        let s = self.ui_scale() as f32;
        let (cx, cy) = (self.pointer.cursor.0 as f32 / s, self.pointer.cursor.1 as f32 / s + TAB_SHIFT);
        let column = TIMER_COLS.iter().position(|x| (cx - x).abs() <= 28.0);
        if let Some(i) = column.filter(|_| dy != 0.0 && (TIMER_PLUS_Y - 12.0..=TIMER_MINUS_Y + 12.0).contains(&cy))
            && self.timer.adjust(Unit::ALL[i], if dy > 0.0 { 1 } else { -1 })
            && let Some(w) = &self.window
        {
            w.request_redraw();
        }
    }

    /// The wheel scrolls the lyrics.
    pub(super) fn wheel_lyrics(&mut self, delta: MouseScrollDelta) {
        let s = self.ui_scale() as f32;
        let (cx, cy) = (self.pointer.cursor.0 as f32 / s, self.pointer.cursor.1 as f32 / s + TAB_SHIFT);
        let lines = match &self.lyric.state {
            LyricsState::Found(l) => l.len(),
            _ => 0,
        };
        // Scroll the lyrics by hand: wheel up goes to earlier lines.
        let step = wheel(&delta, 1.0, 1.0 / 24.0);
        if lines > 0 && step != 0.0 && cx >= LYRIC_X - 8.0 && (LYRIC_AREA.0..=LYRIC_AREA.1).contains(&cy) {
            let from = self.lyric.manual.unwrap_or(self.lyric.pos.max(0.0));
            let to = (from - step).clamp(0.0, lines as f64 - 1.0);
            let sung = self.current_line();
            // Back on the line being sung: carry on following the music.
            self.lyric.manual = if sung.is_some_and(|c| c == to.round() as usize) { None } else { Some(to) };
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
    }

    /// The wheel in the editor.
    pub(super) fn wheel_editor(&mut self, delta: MouseScrollDelta) {
        let dy = wheel(&delta, 1.0, 1.0);
        let s = self.ui_scale() as f32;
        let (cx, cy) = (self.pointer.cursor.0 as f32 / s, self.pointer.cursor.1 as f32 / s + TAB_SHIFT);
        if dy != 0.0
            && self.edit.editor.wheel(cx, cy, if dy > 0.0 { 1 } else { -1 }, clock_now())
            && let Some(w) = &self.window
        {
            w.request_redraw();
        }
    }

    /// The wheel in a list (shelf, to-do, clipboard).
    pub(super) fn wheel_list(&mut self, delta: MouseScrollDelta) {
        let dy = wheel(&delta, 1.0, 1.0);
        if dy != 0.0 {
            // Wheel up shows earlier rows.
            let rows = if dy > 0.0 { -1 } else { 1 };
            match self.view {
                View::Todo => self.todo.scroll_by(rows),
                View::Clipboard => self.clip.history.scroll_by(rows),
                _ => self.shelf.scroll_by(rows),
            }
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
    }

    /// A key pressed while the timer is being typed.
    pub(super) fn on_timer_key(&mut self, event: winit::event::KeyEvent) {
        self.timer_key(&event);
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// A key pressed while a task is being typed.
    pub(super) fn on_edit_key(&mut self, event: winit::event::KeyEvent) {
        match &event.logical_key {
            Key::Named(NamedKey::Enter) => self.commit_task(),
            Key::Named(NamedKey::Escape) => self.stop_editing(),
            Key::Named(NamedKey::Backspace) => {
                self.edit.text.pop();
            }
            _ => {
                if let Some(text) = &event.text {
                    for c in text.chars().filter(|c| !c.is_control()) {
                        if self.edit.text.chars().count() < 80 {
                            self.edit.text.push(c);
                        }
                    }
                }
            }
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}
