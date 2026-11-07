//! What the window system tells the app: window events, user events and the wake-ups in between.

use crate::*;

const PROGRESS_TICK: Duration = Duration::from_millis(500);

/// How often the timer's ring is redrawn while it is on screen and running, so it empties smoothly.
const RING_TICK: Duration = Duration::from_millis(16);

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
            UserEvent::Notice(notice) => {
                if let sys::Notice::Volume { percent, muted } = &notice {
                    self.volume.level = Some((*percent, *muted));
                    // The open notch has its own volume slider: no popup for a change made there.
                    if self.progress > 0.5 {
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                        return;
                    }
                }
                self.show_notice(notice)
            }
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
            UserEvent::Clip(content) => {
                if self.settings.clipboard
                    && self.clips.add(content)
                    && self.view == View::Clipboard
                    && self.progress > 0.0
                    && let Some(w) = &self.window
                {
                    w.request_redraw();
                }
            }
            UserEvent::Menu(id) => {
                let now = Instant::now();
                match self.tray.entry(&id) {
                    Some(TrayEntry::Quit) => self.quit = true,
                    Some(TrayEntry::About) => {
                        self.panel_wanted = true;
                        self.panel_page = Some(panel::Page::About);
                        self.last_tick = now;
                    }
                    Some(TrayEntry::Settings) => {
                        self.panel_wanted = true;
                        self.last_tick = now;
                    }
                    None => {}
                }
            }
            UserEvent::Presence { presence, fullscreen } => {
                let now = Instant::now();
                // Coming back from idle: greet first, then back to the clock.
                if self.presence != Presence::Active && presence == Presence::Active {
                    self.eyes.greeting_until = Some(now + GREETING);
                }
                if presence == Presence::Idle {
                    self.eyes.next_blink = now + Duration::from_millis(1500);
                }
                self.presence = presence;
                self.fullscreen = fullscreen;
                self.apply_visibility();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            UserEvent::ShelfMissing(missing) => {
                self.shelf.forget(&missing);
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            UserEvent::Screens => {
                // Place the island again, on the screen it belongs on, and read the refresh rate afresh.
                self.rate_read = None;
                self.monitor_area = None;
                self.last_tick = Instant::now();
                self.refresh_edges();
                self.step();
                // The screen picker in Settings shows what is connected: draw it again for a screen that came or went.
                if let Some(p) = &mut self.panel {
                    p.redraw();
                }
            }
            UserEvent::FollowTo(screen) => self.follow_to(screen),
            UserEvent::Icon { path, icon } => {
                // Only for a file still on the shelf: it may have been removed while the icon was loading.
                if let Some(slot) = self.shelf_ui.icons.get_mut(&path) {
                    *slot = icon;
                    if self.view == View::Shelf
                        && self.progress > 0.0
                        && let Some(w) = &self.window
                    {
                        w.request_redraw();
                    }
                }
            }
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
            UserEvent::AutoUpdate(found) => {
                if let Some(status) = update::take_auto(&self.updates.status, found) {
                    if let update::Status::Available { version, .. } = &status
                        && self.updates.told.as_ref() != Some(version)
                    {
                        self.updates.told = Some(version.clone());
                        self.show_notice(sys::Notice::Update { version: version.clone() });
                    }
                    self.updates.status = status;
                    if let Some(p) = &self.panel {
                        p.redraw();
                    }
                }
            }
            UserEvent::LockKey { name, vk } => {
                self.show_notice(sys::Notice::Lock { name, on: sys::key_toggled(vk) });
            }
            UserEvent::Lyrics { key, fetched, online } => {
                // Ignore answers for a song that is no longer playing, and ones that began before the lyrics
                // switch was turned the other way.
                if key == self.lyric.key && online == self.settings.lyrics_online {
                    // A second look that finds nothing does not take away what the first one found.
                    let keep = !matches!(fetched, lyrics::Fetched::Lines(_)) && matches!(self.lyric.state, LyricsState::Found(_));
                    self.lyric.retry_at = None;
                    self.lyric.state = match fetched {
                        lyrics::Fetched::Lines(l) => LyricsState::Found(l),
                        _ if keep => std::mem::replace(&mut self.lyric.state, LyricsState::None),
                        lyrics::Fetched::NotFound => LyricsState::NotFound,
                        lyrics::Fetched::Off => LyricsState::Off,
                        lyrics::Fetched::Unreachable => {
                            self.lyric.retry_at = Some(Instant::now() + LYRICS_RETRY);
                            LyricsState::Unreachable
                        }
                    };
                    self.lyric.manual = None;
                    self.snap_lyrics();
                    if self.progress > 0.0
                        && let Some(w) = &self.window
                    {
                        w.request_redraw();
                    }
                }
            }
            UserEvent::Media(state) => {
                // An empty timeline that a player sends for a moment after a seek must not throw the lyrics to the top.
                let state = state.keep_timeline(&self.media, self.position(), self.seeked);
                // Windows is slow to be listened to the first time, so once a song plays (and the bars would follow it)
                // that is done at once, before the closed notch shows the bars.
                if state.playing && self.settings.closed_music && self.settings.closed_bars && self.settings.closed_sound {
                    loopback::warm();
                }
                let key = format!("{}|{}", state.title, state.artist);
                if !state.has_session || state.title.is_empty() {
                    self.lyric.key.clear();
                    self.lyric.state = LyricsState::None;
                    self.seeked = false;
                } else if key != self.lyric.key {
                    self.seeked = false;
                    self.lyric.key = key.clone();
                    self.lyric.state = LyricsState::Loading;
                    self.lyric.manual = None;
                    self.lyric.pos = -1.0;
                    self.lyric.v = 0.0;
                    (self.art.track_t, self.art.track_v) = (0.0, 0.0);
                    (self.lyric.duration, self.lyric.refetched) = (state.duration, false);
                    lyrics::spawn_fetch(key, state.title.clone(), state.artist.clone(), state.duration, self.settings.lyrics_online, self.proxy.clone());
                } else if !self.lyric.refetched && !state.duration.is_zero() && state.duration.abs_diff(self.lyric.duration) > Duration::from_secs(2) {
                    // The first look used no length or the old song's: look again, once, with the right one.
                    // What is shown stays until the answer comes.
                    (self.lyric.duration, self.lyric.refetched) = (state.duration, true);
                    lyrics::spawn_fetch(key, state.title.clone(), state.artist.clone(), state.duration, self.settings.lyrics_online, self.proxy.clone());
                } else if self.lyric.retry_at.is_some_and(|at| Instant::now() >= at) {
                    // The lyrics site could not be reached a while ago: ask again. The message stays until the
                    // answer comes, and the next try is only planned if that one fails too.
                    self.lyric.retry_at = None;
                    self.lyric.duration = state.duration;
                    lyrics::spawn_fetch(key, state.title.clone(), state.artist.clone(), state.duration, self.settings.lyrics_online, self.proxy.clone());
                }
                if let Some(art) = &state.art {
                    let art_key = format!("{}|{}|{}", state.title, state.artist, art.len());
                    if art_key != self.art.key {
                        self.art.tint = art_tint(art);
                        self.art.key = art_key;
                    }
                }
                self.media = state;
                self.media_at = Instant::now();
                // Only worth repainting if the player is visible.
                if self.progress > 0.0
                    && let Some(w) = &self.window
                {
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
                if self.hovered && self.scrub.is_none() && !self.lyric.settle {
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
                if self.volume.drag {
                    let s = self.ui_scale();
                    self.set_volume_at((position.x / s) as f32);
                }
                // Hover highlights follow the mouse.
                if self.progress >= 0.9
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
                self.volume.drag = false;
                if let Some(at) = self.scrub.take() {
                    if let Some(handle) = &self.media_handle {
                        handle.send(Command::Seek(at));
                    }
                    self.seeked = true;
                    // Show the jump immediately instead of waiting for the player to report it.
                    self.media.position = at;
                    self.media_at = Instant::now();
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
            WindowEvent::Focused(false) if self.shelf_ui.selecting => {
                // The user went to another app: the highlight and the keyboard are no longer ours.
                self.shelf_ui.selecting = false;
                self.edit.prev_foreground = None;
                self.shelf_ui.selected.clear();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } if self.view == View::Shelf && event.state == ElementState::Pressed => {
                match &event.logical_key {
                    Key::Named(NamedKey::Escape) => self.shelf_ui.selected.clear(),
                    _ => return,
                }
                self.sync_selection_focus();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } if self.progress >= 0.9 && self.over_volume() => {
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
            WindowEvent::MouseWheel { delta, .. } if self.progress >= 0.9 && !self.edit.on && self.timer_field.is_none() && self.wheel_changes_tab() => {
                let dy = wheel(&delta, 1.0, 1.0);
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
            WindowEvent::MouseWheel { delta, .. } if self.view == View::Timer && !self.edit.on && self.progress >= 0.9 => {
                let dy = wheel(&delta, 1.0, 1.0);
                let s = self.ui_scale() as f32;
                let (cx, cy) = (self.cursor.0 as f32 / s, self.cursor.1 as f32 / s + TAB_SHIFT);
                let column = TIMER_COLS.iter().position(|x| (cx - x).abs() <= 28.0);
                if let Some(i) = column.filter(|_| dy != 0.0 && (TIMER_PLUS_Y - 12.0..=TIMER_MINUS_Y + 12.0).contains(&cy))
                    && self.timer.adjust(Unit::ALL[i], if dy > 0.0 { 1 } else { -1 })
                    && let Some(w) = &self.window
                {
                    w.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } if self.view == View::Lyrics && !self.edit.on && self.progress >= 0.9 => {
                let s = self.ui_scale() as f32;
                let (cx, cy) = (self.cursor.0 as f32 / s, self.cursor.1 as f32 / s + TAB_SHIFT);
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
            WindowEvent::MouseWheel { delta, .. } if self.edit.on && self.progress >= 0.9 => {
                let dy = wheel(&delta, 1.0, 1.0);
                let s = self.ui_scale() as f32;
                let (cx, cy) = (self.cursor.0 as f32 / s, self.cursor.1 as f32 / s + TAB_SHIFT);
                if dy != 0.0
                    && self.edit.editor.wheel(cx, cy, if dy > 0.0 { 1 } else { -1 }, clock_now())
                    && let Some(w) = &self.window
                {
                    w.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } if matches!(self.view, View::Shelf | View::Todo | View::Clipboard) && self.progress >= 0.9 => {
                let dy = wheel(&delta, 1.0, 1.0);
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
            WindowEvent::KeyboardInput { event, .. } if self.edit.on && event.state == ElementState::Pressed => {
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
            WindowEvent::Focused(false) if self.timer_field.is_some() => {
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
                event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + self.frame));
            }
            WindowEvent::HoveredFileCancelled => {
                self.shelf_ui.drag_hover = false;
                self.last_tick = Instant::now();
            }
            WindowEvent::DroppedFile(path) => {
                self.shelf.add(path);
                self.shelf_ui.drag_hover = false;
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
        if self.quit {
            event_loop.exit();
            return;
        }
        let now = Instant::now();

        // Highlighted shelf files hold the keyboard; let go of both once the notch closes or the shelf is left.
        if self.shelf_ui.selecting && (self.view != View::Shelf || !self.hovered) {
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
        // The settings window asks for frames while its controls are moving.
        let mut panel_wake = None;
        let frame = self.frame;
        if let Some(p) = &mut self.panel {
            if p.reset_armed.is_some_and(|t| now >= t) {
                p.reset_armed = None;
                p.redraw();
            }
            if p.cleared.is_some_and(|(_, t)| now >= t) {
                p.cleared = None;
                p.redraw();
            }
            if p.busy {
                p.redraw();
                panel_wake = Some(now + frame);
            } else if let Some(t) = p.reset_armed {
                panel_wake = Some(t);
            }
            if let Some((_, t)) = p.cleared {
                panel_wake = Some(panel_wake.map_or(t, |w| w.min(t)));
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
        if self.lyric.settle && !self.animating() {
            self.lyric.settle = false;
            if self.hovered && !self.cursor_over_window() {
                self.leave_at = Some(now + LYRIC_GRACE);
            }
        }

        if self.timer.tick(now) {
            self.notices.alert_msg = TIMES_UP.into();
            self.notices.alert_until = Some(now + self.alarm_for());
            self.notices.next_chime = Some(now + self.alarm_every());
            self.view = View::Timer;
            self.last_tick = now;
            self.play_alarm();
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
        if let Some(text) = self.todo.take_due(todo::unix_now()) {
            let short: String = text.chars().take(40).collect();
            self.notices.alert_msg = format!("Reminder: {short}");
            self.notices.alert_until = Some(now + ALERT);
            self.view = View::Todo;
            self.last_tick = now;
            self.play_sound(sound::Chime::Reminder);
        }
        // Copied items are forgotten after the time the user chose.
        if let Some(keep) = self.settings.clip_keep.duration()
            && self.clips.expire(now, keep)
            && self.view == View::Clipboard
            && self.progress > 0.0
            && let Some(w) = &self.window
        {
            w.request_redraw();
        }
        self.check_prayer();
        self.water.roll(today());
        if self.settings.water && self.water.due(now) {
            self.notices.alert_msg = "Time to drink some water!".into();
            self.notices.alert_until = Some(now + ALERT);
            self.view = View::Water;
            self.last_tick = now;
            self.play_sound(sound::Chime::Reminder);
        }
        if self.notices.next_chime.is_some_and(|t| now >= t) {
            if self.notices.alert_until.is_some() && self.notices.alert_msg == TIMES_UP {
                self.play_alarm();
                self.notices.next_chime = Some(now + self.alarm_every());
            } else {
                self.notices.next_chime = None;
            }
        }
        if self.notices.alert_until.is_some_and(|t| now >= t) {
            if self.notices.alert_msg == TIMES_UP {
                sound::stop_file();
            }
            self.notices.alert_until = None;
            self.last_tick = now;
        }
        if self.notices.until.is_some_and(|t| now >= t) {
            self.notices.until = None;
            self.last_tick = now;
        }
        if self.clip_flash.as_ref().is_some_and(|(_, t)| now >= *t) {
            self.clip_flash = None;
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }

        if self.view != self.switch.shown {
            // Moving between Music and its lyrics is the same page; any other change is a transition in
            // which the old view slides out as the new one slides in from the side of its tab.
            let tab = |v: View| self.visible_tabs().iter().position(|(_, t)| *t == if v == View::Lyrics { View::Player } else { v });
            let (from, to) = (tab(self.switch.shown), tab(self.view));
            if from != to || from.is_none() {
                self.switch.prev = Some(self.switch.shown);
                self.switch.dir = match (from, to) {
                    (Some(a), Some(b)) if b < a => -1.0,
                    _ => 1.0,
                };
                self.switch.t = 0.0;
                self.switch.v = 0.0;
            }
            self.switch.shown = self.view;
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
            event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame(now)));
            return;
        }

        let mut wake: Option<Instant> = None;
        self.companion_tick(now, &mut wake);
        let mut at = |t: Instant| earliest(&mut wake, t);

        // Periodic refresh for the progress bar / countdown. Only redraw when due, since a redraw
        // itself triggers another about_to_wait.
        let expanded = self.progress >= 1.0;
        if (expanded && (self.media.playing || self.edit.on || self.timer_field.is_some() || matches!(self.view, View::Water | View::Todo))) || self.timer.running() {
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
        // The bars on the closed notch move a few times a second while a song plays.
        if self.bars_running() {
            let (tick, due) = (self.music_tick(), self.music.bars_at + self.music_tick());
            if now >= due {
                self.music.bars_at = now;
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
                at(now + tick);
            } else {
                at(due);
            }
        }
        if let Some(wait) = self.prayer_minute_tick() {
            at(now + wait);
        }
        if let Some(t) = self.prayer_wake(now) {
            at(t);
        }
        if let Some(t) = self.stats_tick(now) {
            at(t);
        }
        // Wake exactly when the next lyric line starts, so the glide begins on time.
        if expanded
            && self.view == View::Lyrics
            && self.media.playing
            && let LyricsState::Found(lines) = &self.lyric.state
        {
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
        if let Some(t) = self.notices.alert_until {
            at(t);
        }
        if let Some(t) = self.notices.next_chime {
            at(t);
        }
        if let Some(t) = panel_wake {
            at(t);
        }
        for t in [self.enter_at, self.leave_at].into_iter().flatten() {
            at(t);
        }
        if let Some(t) = self.notices.until {
            at(t);
        }
        if let Some((_, t)) = self.clip_flash {
            at(t);
        }
        if let Some(t) = self.settings.clip_keep.duration().and_then(|keep| self.clips.next_expiry(keep)) {
            at(t);
        }

        event_loop.set_control_flow(match wake {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        });
    }
}
