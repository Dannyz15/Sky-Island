//! What the window system tells the app: window events, user events and the wake-ups in between.

use crate::*;

/// How far a wheel turned: one notch of a mouse wheel is worth `per_line`, one pixel of a touchpad `per_pixel`.
pub(crate) fn wheel(delta: &MouseScrollDelta, per_line: f64, per_pixel: f64) -> f64 {
    match delta {
        MouseScrollDelta::LineDelta(_, y) => f64::from(*y) * per_line,
        MouseScrollDelta::PixelDelta(p) => p.y * per_pixel,
    }
}

/// Whether Ctrl is held right now. The notch never has the keyboard focus, so its own key events
/// cannot be trusted to say.
pub(crate) fn ctrl_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL};
    unsafe { GetAsyncKeyState(VK_CONTROL.0 as i32) < 0 }
}

/// Updates: how the last check went, the version the notch has already told about, and the changelog.
pub(crate) struct Updates {
    /// How the last "Check for updates" in the settings window went.
    pub(crate) status: update::Status,
    /// The version the notch has already told the user about, so the notice comes once per version.
    pub(crate) told: Option<String>,
    /// The changelog of the settings window, loaded from GitHub when it is first opened.
    pub(crate) changelog: update::Changelog,
}

impl Default for Updates {
    fn default() -> Self {
        Self { status: update::Status::Idle, told: None, changelog: update::Changelog::Idle }
    }
}

/// The mouse: where it is, whether it is over the notch, when entering or leaving takes effect, and when the wheel last changed the tab.
pub(crate) struct Pointer {
    pub(crate) cursor: (f64, f64),
    pub(crate) hovered: bool,
    /// When the cursor entering/leaving takes effect (see `hover_open`, `hover_close`).
    pub(crate) enter_at: Option<Instant>,
    pub(crate) leave_at: Option<Instant>,
    /// When the mouse wheel last changed the tab, so one flick moves one tab.
    pub(crate) last_wheel_tab: Instant,
}

impl Default for Pointer {
    fn default() -> Self {
        Self { cursor: (0.0, 0.0), hovered: false, enter_at: None, leave_at: None, last_wheel_tab: Instant::now() }
    }
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
        self.play.handle = Some(media::spawn(self.proxy.clone()));
        sys::spawn_all(self.proxy.clone());
        self.make_non_activating();
        self.tray = Tray::build(&self.proxy);
        sys::install_lock_hook(self.proxy.clone());
        presence::spawn(self.proxy.clone(), self.native_handle().unwrap_or(0));
        shelf::check_in_background(self.shelf.items.clone(), self.proxy.clone());
        clipboard::set_enabled(self.settings.clipboard);
        clipboard::spawn_watcher(self.proxy.clone());
        update::spawn_auto(self.settings.hud_update, self.proxy.clone());
        self.weather.tx = Some(weather::spawn(self.proxy.clone()));
        self.sync_weather();
        self.prayer.tx = Some(prayer::spawn(self.proxy.clone()));
        self.sync_prayer();
        self.refresh_edges();
        hotkey::start(self.proxy.clone(), self.settings.hotkey.then_some((self.settings.hotkey_mods, self.settings.hotkey_vk)));
        self.step();
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Notice(notice) => self.on_notice(notice),
            UserEvent::Hotkey => self.toggle_by_key(),
            UserEvent::HotkeyStatus(ok) => {
                self.hotkey_ok = ok;
                if let Some(p) = &self.panel {
                    p.redraw();
                }
            }
            UserEvent::SysVolume { percent, muted } => {
                self.volume.level = Some((percent, muted));
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            UserEvent::Clip(content) => self.on_clip(content),
            UserEvent::Menu(id) => self.on_menu(id),
            UserEvent::Presence { presence, fullscreen } => self.on_presence(presence, fullscreen),
            UserEvent::ShelfMissing(missing) => {
                self.shelf.forget(&missing);
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            UserEvent::Screens => self.on_screens(),
            UserEvent::FollowTo(screen) => self.follow_to(screen),
            UserEvent::Icon { path, icon } => self.on_icon(path, icon),
            UserEvent::Weather(report) => self.weather_arrived(report),
            UserEvent::Place(found) => self.place_found(found, crate::location::How::Exact),
            UserEvent::Located(found) => match found {
                Ok((place, how)) => self.place_found(Ok(place), how),
                Err(e) => self.place_found(Err(e), crate::location::How::Exact),
            },
            UserEvent::Prayer(table) => self.prayer_arrived(table),
            UserEvent::Changelog(releases) => {
                self.updates.changelog = match releases {
                    Some(list) => update::Changelog::Ready(list),
                    None => update::Changelog::Failed,
                };
                if let Some(p) = &self.panel {
                    p.redraw();
                }
            }
            UserEvent::Update(status) => {
                self.updates.status = status;
                if let Some(p) = &self.panel {
                    p.redraw();
                }
            }
            UserEvent::AutoUpdate(found) => self.on_auto_update(found),
            UserEvent::LockKey { name, vk } => {
                self.show_notice(sys::Notice::Lock { name, on: sys::key_toggled(vk) });
            }
            UserEvent::Lyrics { key, fetched, online } => self.on_lyrics(key, fetched, online),
            UserEvent::Media(state) => self.on_media(state),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.panel.as_ref().is_some_and(|p| p.id() == id) {
            self.panel_event(event);
            return;
        }
        // The copies of the notch on the other screens take no part in anything: they are only drawn again if asked.
        if self.is_mirror(id) {
            match event {
                WindowEvent::RedrawRequested => self.draw_mirrors(true),
                WindowEvent::CursorEntered { .. } | WindowEvent::CursorMoved { .. } => self.mirror_touched(id),
                _ => {}
            }
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::CursorEntered { .. } => self.on_cursor_entered(event_loop),
            WindowEvent::CursorLeft { .. } => self.on_cursor_left(event_loop),
            WindowEvent::CursorMoved { position, .. } => self.on_cursor_moved(position),
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => self.on_left_pressed(),
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. } => self.on_left_released(),
            WindowEvent::Focused(false) if self.shelf_ui.selecting => self.on_shelf_focus_lost(),
            WindowEvent::KeyboardInput { event, .. } if self.view == View::Shelf && event.state == ElementState::Pressed => self.on_shelf_key(event),
            WindowEvent::MouseWheel { delta, .. } if self.spring.progress >= 0.9 && self.over_volume() => self.wheel_volume(delta),
            WindowEvent::MouseWheel { delta, .. } if self.spring.progress >= 0.9 && !self.edit.on && self.timer_entry.field.is_none() && self.wheel_changes_tab() => {
                self.wheel_tabs(delta)
            }
            WindowEvent::MouseWheel { delta, .. } if self.view == View::Timer && !self.edit.on && self.spring.progress >= 0.9 => self.wheel_timer(delta),
            WindowEvent::MouseWheel { delta, .. } if self.view == View::Lyrics && !self.edit.on && self.spring.progress >= 0.9 => self.wheel_lyrics(delta),
            WindowEvent::MouseWheel { delta, .. } if self.edit.on && self.spring.progress >= 0.9 => self.wheel_editor(delta),
            WindowEvent::MouseWheel { delta, .. } if matches!(self.view, View::Shelf | View::Todo | View::Clipboard) && self.spring.progress >= 0.9 => self.wheel_list(delta),
            WindowEvent::KeyboardInput { event, .. } if self.timer_entry.field.is_some() && event.state == ElementState::Pressed => self.on_timer_key(event),
            WindowEvent::KeyboardInput { event, .. } if self.edit.on && event.state == ElementState::Pressed => self.on_edit_key(event),
            WindowEvent::Focused(false) if self.timer_entry.field.is_some() => {
                // The user clicked into another app: keep what was typed and don't steal focus back.
                self.edit.prev_foreground = None;
                self.end_timer_edit(true);
            }
            WindowEvent::Focused(false) if self.edit.on => {
                // The user clicked into another app: stop typing without stealing focus back.
                self.edit.prev_foreground = None;
                self.stop_editing();
            }
            WindowEvent::HoveredFile(_) => {
                self.shelf_ui.drag_hover = true;
                self.view = View::Shelf;
                self.last_tick = Instant::now();
                event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + self.pacing.frame));
            }
            WindowEvent::HoveredFileCancelled => {
                self.shelf_ui.drag_hover = false;
                self.last_tick = Instant::now();
            }
            WindowEvent::DroppedFile(path) => {
                self.shelf.add(path);
                self.shelf_ui.drag_hover = false;
                // The drop happened over the notch, so keep it open until the cursor leaves.
                self.pointer.hovered = true;
                self.pointer.leave_at = None;
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
            // The island moved to a screen with another scale: size and place it for that one.
            WindowEvent::ScaleFactorChanged { .. } => {
                self.last_tick = Instant::now();
                self.step();
            }
            WindowEvent::RedrawRequested => self.draw(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Files dropped on the shelf are saved once, after all of the drop's events were handled (and before quitting).
        self.shelf.flush();
        if self.quit {
            event_loop.exit();
            return;
        }
        let now = Instant::now();

        // Highlighted shelf files hold the keyboard; let go of both once the notch closes or the shelf is left.
        if self.shelf_ui.selecting && (self.view != View::Shelf || !self.pointer.hovered) {
            self.shelf_ui.selected.clear();
            self.sync_selection_focus();
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }

        if self.mirrors.wanted {
            self.mirrors.wanted = false;
            self.sync_mirrors(event_loop);
        }
        if self.panel_wanted {
            self.panel_wanted = false;
            self.open_panel(event_loop);
            if let Some(page) = self.panel_page.take() {
                self.show_page(page);
            }
        }
        let panel_wake = self.panel_tick(now);

        self.hover_due(now);
        self.alerts_due(now);
        self.sync_view_switch(now);

        // Only tick while something is changing; otherwise sleep until the next event (0% idle CPU).
        // A step draws a frame, and drawing wakes the loop again at once: stepping at every wake-up made the notch
        // redraw thousands of times a second while a switch of view settled, and the settings window, on the same
        // thread, waited for it. So a step is taken only when it is due, one for each frame of the screen.
        if self.animating() {
            if now >= self.pacing.due {
                self.step();
                self.pacing.due = self.next_frame(now);
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(self.pacing.due));
            return;
        }

        let wake = self.next_wake(now, panel_wake);

        event_loop.set_control_flow(match wake {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        });
    }
}
