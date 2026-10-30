//! What the window system tells the app: window events, user events and the wake-ups in between.

use crate::*;

const FRAME: Duration = Duration::from_millis(8);

const PROGRESS_TICK: Duration = Duration::from_millis(500);

/// How often the timer's ring is redrawn while it is on screen and running, so it empties smoothly.
const RING_TICK: Duration = Duration::from_millis(16);

/// Whether Ctrl is held right now. The notch never has the keyboard focus, so its own key events
/// cannot be trusted to say.
pub(crate) fn ctrl_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL};
    unsafe { GetAsyncKeyState(VK_CONTROL.0 as i32) < 0 }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        // A setup downloaded by "Check for updates" has done its job once this version is running.
        update::clean_up_downloads();
        let attrs = Window::default_attributes()
            .with_title("Sky Island")
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_skip_taskbar(true)
            .with_inner_size(LogicalSize::new(COLLAPSED.0 + 2.0 * EAR, COLLAPSED.1));
        let window = Rc::new(event_loop.create_window(attrs).expect("create window"));
        let context = Context::new(window.clone()).expect("softbuffer context");
        self.surface = Some(Surface::new(&context, window.clone()).expect("softbuffer surface"));
        self.system_light = window.theme() == Some(Theme::Light);
        self.window = Some(window);
        self.media_handle = Some(media::spawn(self.proxy.clone()));
        sys::spawn_all(self.proxy.clone());
        self.make_non_activating();
        self.build_tray();
        sys::install_lock_hook(self.proxy.clone());
        presence::spawn(self.proxy.clone());
        clipboard::spawn_watcher(self.proxy.clone());
        self.step();
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Notice(notice) => self.show_notice(notice),
            UserEvent::Clip(content) => {
                if self.settings.clipboard
                    && self.clips.add(content)
                    && self.view == View::Clipboard
                    && self.progress > 0.0
                    && let Some(w) = &self.window {
                        w.request_redraw();
                    }
            }
            UserEvent::Menu(id) => {
                let now = Instant::now();
                if Some(&id) == self.tray_quit_id.as_ref() {
                    self.quit = true;
                } else if Some(&id) == self.tray_about_id.as_ref() {
                    self.panel_wanted = true;
                    self.panel_about = true;
                    self.last_tick = now;
                } else if Some(&id) == self.tray_settings_id.as_ref() {
                    self.panel_wanted = true;
                    self.last_tick = now;
                }
            }
            UserEvent::Presence { presence, fullscreen } => {
                let now = Instant::now();
                // Coming back from idle: greet first, then back to the clock.
                if self.presence != Presence::Active && presence == Presence::Active {
                    self.greeting_until = Some(now + GREETING);
                }
                if presence == Presence::Idle {
                    self.next_blink = now + Duration::from_millis(1500);
                }
                self.presence = presence;
                self.fullscreen = fullscreen;
                self.apply_visibility();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            UserEvent::Changelog(releases) => {
                self.changelog = match releases {
                    Some(list) => update::Changelog::Ready(list),
                    None => update::Changelog::Failed,
                };
                if let Some(p) = &self.panel {
                    p.redraw();
                }
            }
            UserEvent::Update(status) => {
                self.update = status;
                if let Some(p) = &self.panel {
                    p.redraw();
                }
            }
            UserEvent::LockKey { name, vk } => {
                self.show_notice(sys::Notice::Lock { name, on: sys::key_toggled(vk) });
            }
            UserEvent::Lyrics { key, fetched } => {
                // Ignore answers for a song that is no longer playing.
                if key == self.lyrics_key {
                    // A second look that finds nothing does not take away what the first one found.
                    let keep = !matches!(fetched, lyrics::Fetched::Lines(_)) && matches!(self.lyrics, LyricsState::Found(_));
                    self.lyrics_retry_at = None;
                    self.lyrics = match fetched {
                        lyrics::Fetched::Lines(l) => LyricsState::Found(l),
                        _ if keep => std::mem::replace(&mut self.lyrics, LyricsState::None),
                        lyrics::Fetched::NotFound => LyricsState::NotFound,
                        lyrics::Fetched::Unreachable => {
                            self.lyrics_retry_at = Some(Instant::now() + LYRICS_RETRY);
                            LyricsState::Unreachable
                        }
                    };
                    self.lyric_manual = None;
                    self.snap_lyrics();
                    if self.progress > 0.0
                        && let Some(w) = &self.window {
                            w.request_redraw();
                        }
                }
            }
            UserEvent::Media(state) => {
                let key = format!("{}|{}", state.title, state.artist);
                if !state.has_session || state.title.is_empty() {
                    self.lyrics_key.clear();
                    self.lyrics = LyricsState::None;
                } else if key != self.lyrics_key {
                    self.lyrics_key = key.clone();
                    self.lyrics = LyricsState::Loading;
                    self.lyric_manual = None;
                    self.lyric_pos = -1.0;
                    self.lyric_v = 0.0;
                    (self.track_t, self.track_v) = (0.0, 0.0);
                    (self.lyrics_duration, self.lyrics_refetched) = (state.duration, false);
                    lyrics::spawn_fetch(key, state.title.clone(), state.artist.clone(), state.duration, self.proxy.clone());
                } else if !self.lyrics_refetched
                    && !state.duration.is_zero()
                    && state.duration.abs_diff(self.lyrics_duration) > Duration::from_secs(2)
                {
                    // The first look used no length or the old song's: look again, once, with the right one.
                    // What is shown stays until the answer comes.
                    (self.lyrics_duration, self.lyrics_refetched) = (state.duration, true);
                    lyrics::spawn_fetch(key, state.title.clone(), state.artist.clone(), state.duration, self.proxy.clone());
                } else if self.lyrics_retry_at.is_some_and(|at| Instant::now() >= at) {
                    // The lyrics site could not be reached a while ago: ask again. The message stays until the
                    // answer comes, and the next try is only planned if that one fails too.
                    self.lyrics_retry_at = None;
                    self.lyrics_duration = state.duration;
                    lyrics::spawn_fetch(key, state.title.clone(), state.artist.clone(), state.duration, self.proxy.clone());
                }
                if let Some(art) = &state.art {
                    let art_key = format!("{}|{}|{}", state.title, state.artist, art.len());
                    if art_key != self.art_key {
                        self.art_tint = art_tint(art);
                        self.art_key = art_key;
                    }
                }
                self.media = state;
                self.media_at = Instant::now();
                // Only worth repainting if the player is visible.
                if self.progress > 0.0
                    && let Some(w) = &self.window {
                        w.request_redraw();
                    }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.panel.as_ref().is_some_and(|p| p.id() == id) {
            self.panel_event(event);
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::CursorEntered { .. } => {
                let now = Instant::now();
                self.leave_at = None;
                // With "Open the notch" on Click, resting on it does nothing, except to bring an auto-hidden
                // island back into view.
                let waits_for_click = self.settings.open_on_click && self.tuck_t < 0.1;
                if !self.hovered && !waits_for_click {
                    self.enter_at = Some(now + self.hover_open());
                    event_loop.set_control_flow(ControlFlow::WaitUntil(now + self.hover_open()));
                }
            }
            WindowEvent::CursorLeft { .. } => {
                let now = Instant::now();
                self.enter_at = None; // a fly-by never opened it
                if self.hovered && self.scrub.is_none() && !self.lyric_settle {
                    self.leave_at = Some(now + self.hover_close());
                    event_loop.set_control_flow(ControlFlow::WaitUntil(now + self.hover_close()));
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                // The window has a flare on each side of the notch; positions count from the notch itself.
                let ear = EAR * self.ui_scale();
                let position = PhysicalPosition::new(position.x - ear, position.y);
                self.cursor = (position.x, position.y);
                if self.scrub.is_some() {
                    let s = self.ui_scale();
                    self.scrub = Some(self.scrub_at((position.x / s) as f32));
                }
                // Hover highlights follow the mouse.
                if self.progress >= 0.9
                    && let Some(w) = &self.window {
                        w.request_redraw();
                    }
                if let Some((idx, start)) = self.press
                    && (position.x - start.0).hypot(position.y - start.1) > DRAG_THRESHOLD {
                        self.press = None;
                        self.drag_out(idx);
                    }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                if self.settings.open_on_click && !self.hovered && self.progress < 0.9 {
                    // A click on the closed notch opens it.
                    self.hovered = true;
                    self.enter_at = None;
                    self.leave_at = None;
                    self.last_tick = Instant::now();
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                } else {
                    self.click();
                }
            }
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. } => {
                if let Some(at) = self.scrub.take() {
                    if let Some(handle) = &self.media_handle {
                        handle.send(Command::Seek(at));
                    }
                    // Show the jump immediately instead of waiting for the player to report it.
                    self.media.position = at;
                    self.media_at = Instant::now();
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
                // A press that never turned into a drag is a click: open the file.
                if let Some((idx, _)) = self.press.take()
                    && let Some(path) = self.shelf.items.get(idx).cloned() {
                        let _ = std::process::Command::new("explorer").arg(&path).spawn();
                        self.selected.clear();
                        self.sync_selection_focus();
                    }
            }
            WindowEvent::Focused(false) if self.selecting => {
                // The user went to another app: the highlight and the keyboard are no longer ours.
                self.selecting = false;
                self.prev_foreground = None;
                self.selected.clear();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } if self.view == View::Shelf && event.state == ElementState::Pressed => {
                match &event.logical_key {
                    Key::Named(NamedKey::Escape) => self.selected.clear(),
                    _ => return,
                }
                self.sync_selection_focus();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. }
                if self.progress >= 0.9 && !self.editing && self.timer_field.is_none() && self.wheel_changes_tab() =>
            {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y,
                };
                // Scrolling down goes to the next tab, up to the one before; one step per flick.
                if dy != 0.0 && self.last_wheel_tab.elapsed() > Duration::from_millis(220) {
                    let tabs = self.visible_tabs();
                    let here = if self.view == View::Lyrics { View::Player } else { self.view };
                    if let Some(i) = tabs.iter().position(|(_, v)| *v == here) {
                        let next = if dy < 0.0 { (i + 1).min(tabs.len() - 1) } else { i.saturating_sub(1) };
                        if next != i {
                            self.last_wheel_tab = Instant::now();
                            self.go_to_view(tabs[next].1);
                        }
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } if self.view == View::Timer && !self.editing && self.progress >= 0.9 => {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y,
                };
                let s = self.ui_scale() as f32;
                let (cx, cy) = (self.cursor.0 as f32 / s, self.cursor.1 as f32 / s + TAB_SHIFT);
                let column = TIMER_COLS.iter().position(|x| (cx - x).abs() <= 28.0);
                if let Some(i) = column.filter(|_| dy != 0.0 && (TIMER_PLUS_Y - 12.0..=TIMER_MINUS_Y + 12.0).contains(&cy))
                    && self.timer.adjust(Unit::ALL[i], if dy > 0.0 { 1 } else { -1 })
                    && let Some(w) = &self.window {
                        w.request_redraw();
                    }
            }
            WindowEvent::MouseWheel { delta, .. } if self.view == View::Lyrics && !self.editing && self.progress >= 0.9 => {
                let s = self.ui_scale() as f32;
                let (cx, cy) = (self.cursor.0 as f32 / s, self.cursor.1 as f32 / s + TAB_SHIFT);
                let lines = match &self.lyrics {
                    LyricsState::Found(l) => l.len(),
                    _ => 0,
                };
                // Scroll the lyrics by hand: wheel up goes to earlier lines.
                let step = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y / 24.0,
                };
                if lines > 0 && step != 0.0 && cx >= LYRIC_X - 8.0 && (LYRIC_AREA.0..=LYRIC_AREA.1).contains(&cy) {
                    let from = self.lyric_manual.unwrap_or(self.lyric_pos.max(0.0));
                    let to = (from - step).clamp(0.0, lines as f64 - 1.0);
                    let sung = self.current_line();
                    // Back on the line being sung: carry on following the music.
                    self.lyric_manual = if sung.is_some_and(|c| c == to.round() as usize) { None } else { Some(to) };
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } if self.editing && self.progress >= 0.9 => {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y,
                };
                let s = self.ui_scale() as f32;
                let (cx, cy) = (self.cursor.0 as f32 / s, self.cursor.1 as f32 / s + TAB_SHIFT);
                if dy != 0.0
                    && self.editor.wheel(cx, cy, if dy > 0.0 { 1 } else { -1 }, clock_now())
                    && let Some(w) = &self.window {
                        w.request_redraw();
                    }
            }
            WindowEvent::MouseWheel { delta, .. }
                if matches!(self.view, View::Shelf | View::Todo | View::Clipboard) && self.progress >= 0.9 =>
            {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y,
                };
                if dy != 0.0 {
                    // Wheel up shows earlier rows.
                    let rows = if dy > 0.0 { -1 } else { 1 };
                    match self.view {
                        View::Todo => self.todo.scroll_by(rows),
                        View::Clipboard => self.clips.scroll_by(rows),
                        _ => self.shelf.scroll_by(rows),
                    }
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } if self.timer_field.is_some() && event.state == ElementState::Pressed => {
                self.timer_key(&event);
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } if self.editing && event.state == ElementState::Pressed => {
                match &event.logical_key {
                    Key::Named(NamedKey::Enter) => self.commit_task(),
                    Key::Named(NamedKey::Escape) => self.stop_editing(),
                    Key::Named(NamedKey::Backspace) => {
                        self.input.pop();
                    }
                    _ => {
                        if let Some(text) = &event.text {
                            for c in text.chars().filter(|c| !c.is_control()) {
                                if self.input.chars().count() < 80 {
                                    self.input.push(c);
                                }
                            }
                        }
                    }
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::Focused(false) if self.timer_field.is_some() => {
                // The user clicked into another app: keep what was typed and don't steal focus back.
                self.prev_foreground = None;
                self.end_timer_edit(true);
            }
            WindowEvent::Focused(false) if self.editing => {
                // The user clicked into another app: stop typing without stealing focus back.
                self.prev_foreground = None;
                self.stop_editing();
            }
            WindowEvent::HoveredFile(_) => {
                self.drag_hover = true;
                self.view = View::Shelf;
                self.last_tick = Instant::now();
                event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + FRAME));
            }
            WindowEvent::HoveredFileCancelled => {
                self.drag_hover = false;
                self.last_tick = Instant::now();
            }
            WindowEvent::DroppedFile(path) => {
                self.shelf.add(path);
                self.drag_hover = false;
                // The drop happened over the notch, so keep it open until the cursor leaves.
                self.hovered = true;
                self.leave_at = None;
                self.view = View::Shelf;
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::ThemeChanged(theme) => {
                self.system_light = theme == Theme::Light;
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => self.draw(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.quit {
            event_loop.exit();
            return;
        }
        let now = Instant::now();

        // Highlighted shelf files hold the keyboard; let go of both once the notch closes or the shelf is left.
        if self.selecting && (self.view != View::Shelf || !self.hovered) {
            self.selected.clear();
            self.sync_selection_focus();
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }

        if self.panel_wanted {
            self.panel_wanted = false;
            self.open_panel(event_loop);
            if std::mem::take(&mut self.panel_about) {
                self.show_about_page();
            }
        }
        // The settings window asks for frames while its controls are moving.
        let mut panel_wake = None;
        if let Some(p) = &mut self.panel {
            if p.reset_armed.is_some_and(|t| now >= t) {
                p.reset_armed = None;
                p.redraw();
            }
            if p.busy {
                p.redraw();
                panel_wake = Some(now + FRAME);
            } else if let Some(t) = p.reset_armed {
                panel_wake = Some(t);
            }
        }

        if self.enter_at.is_some_and(|t| now >= t) {
            self.enter_at = None;
            self.hovered = true;
            self.last_tick = now;
        }
        if self.leave_at.is_some_and(|t| now >= t) {
            self.leave_at = None;
            self.hovered = false;
            self.last_tick = now;
        }

        // The notch has finished changing width. If the mouse is now outside it (closing the lyrics pulls
        // the sides in), give it a moment to come back before the notch closes.
        if self.lyric_settle && !self.animating() {
            self.lyric_settle = false;
            if self.hovered && !self.cursor_over_window() {
                self.leave_at = Some(now + LYRIC_GRACE);
            }
        }

        if self.timer.tick(now) {
            self.alert_msg = TIMES_UP.into();
            self.alert_until = Some(now + self.alarm_for());
            self.next_chime = Some(now + self.alarm_every());
            self.view = View::Timer;
            self.last_tick = now;
            self.play_alarm();
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
        if let Some(text) = self.todo.take_due(todo::unix_now()) {
            let short: String = text.chars().take(40).collect();
            self.alert_msg = format!("Reminder: {short}");
            self.alert_until = Some(now + ALERT);
            self.view = View::Todo;
            self.last_tick = now;
            self.play_sound(sound::Chime::Reminder);
        }
        self.water.roll(today());
        if self.settings.water && self.water.due(now) {
            self.alert_msg = "Time to drink some water!".into();
            self.alert_until = Some(now + ALERT);
            self.view = View::Water;
            self.last_tick = now;
            self.play_sound(sound::Chime::Reminder);
        }
        if self.next_chime.is_some_and(|t| now >= t) {
            if self.alert_until.is_some() && self.alert_msg == TIMES_UP {
                self.play_alarm();
                self.next_chime = Some(now + self.alarm_every());
            } else {
                self.next_chime = None;
            }
        }
        if self.alert_until.is_some_and(|t| now >= t) {
            if self.alert_msg == TIMES_UP {
                sound::stop_file();
            }
            self.alert_until = None;
            self.last_tick = now;
        }
        if self.notice_until.is_some_and(|t| now >= t) {
            self.notice_until = None;
            self.last_tick = now;
        }
        if self.clip_flash.as_ref().is_some_and(|(_, t)| now >= *t) {
            self.clip_flash = None;
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }

        if self.view != self.shown_view {
            // Moving between Music and its lyrics is the same page; any other change is a transition in
            // which the old view slides out as the new one slides in from the side of its tab.
            let tab = |v: View| self.visible_tabs().iter().position(|(_, t)| *t == if v == View::Lyrics { View::Player } else { v });
            let (from, to) = (tab(self.shown_view), tab(self.view));
            if from != to || from.is_none() {
                self.prev_view = Some(self.shown_view);
                self.switch_dir = match (from, to) {
                    (Some(a), Some(b)) if b < a => -1.0,
                    _ => 1.0,
                };
                self.switch_t = 0.0;
                self.switch_v = 0.0;
            }
            self.shown_view = self.view;
            self.last_tick = now;
        }
        let goal = self.tuck_target();
        if goal != self.tuck_goal {
            self.tuck_goal = goal;
            self.last_tick = now;
        }

        // Only tick while something is changing; otherwise sleep until the next event (0% idle CPU).
        if self.animating() {
            self.step();
            event_loop.set_control_flow(ControlFlow::WaitUntil(now + FRAME));
            return;
        }

        let mut wake: Option<Instant> = None;
        self.companion_tick(now, &mut wake);
        let mut at = |t: Instant| earliest(&mut wake, t);

        // Periodic refresh for the progress bar / countdown. Only redraw when due, since a redraw
        // itself triggers another about_to_wait.
        let expanded = self.progress >= 1.0;
        if (expanded && (self.media.playing || self.editing || self.timer_field.is_some() || matches!(self.view, View::Water | View::Todo)))
            || self.timer.running()
        {
            let tick = if expanded && self.view == View::Timer && self.timer.running() { RING_TICK } else { PROGRESS_TICK };
            let due = self.last_ui_tick + tick;
            if now >= due {
                self.last_ui_tick = now;
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
                at(now + tick);
            } else {
                at(due);
            }
        }
        // Wake exactly when the next lyric line starts, so the glide begins on time.
        if expanded && self.view == View::Lyrics && self.media.playing
            && let LyricsState::Found(lines) = &self.lyrics {
                let next = self.current_line().map_or(0, |c| c + 1);
                if let Some(line) = lines.get(next) {
                    at(now + line.at.saturating_sub(self.position()));
                }
            }
        if let Some(d) = self.timer.deadline() {
            at(d);
        }
        if let Some(d) = self.water.deadline().filter(|_| self.settings.water) {
            at(d);
        }
        if let Some(due) = self.todo.next_due() {
            at(now + Duration::from_secs(due.saturating_sub(todo::unix_now())));
        }
        if let Some(t) = self.alert_until {
            at(t);
        }
        if let Some(t) = self.next_chime {
            at(t);
        }
        if let Some(t) = panel_wake {
            at(t);
        }
        for t in [self.enter_at, self.leave_at].into_iter().flatten() {
            at(t);
        }
        if let Some(t) = self.notice_until {
            at(t);
        }
        if let Some((_, t)) = self.clip_flash {
            at(t);
        }

        event_loop.set_control_flow(match wake {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        });
    }
}
