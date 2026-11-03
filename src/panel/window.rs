//! Opening the settings window and what happens in it: keys, the mouse, the wheel and redraws.

use super::*;

/// Ask for a sound file with the standard Windows Open dialog.
pub(super) fn pick_sound(owner: &Window) -> Option<std::path::PathBuf> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::{FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH};
    use windows::core::w;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let RawWindowHandle::Win32(h) = owner.window_handle().ok()?.as_raw() else { return None };
    let hwnd = HWND(h.hwnd.get() as *mut _);
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        dialog.SetTitle(w!("Choose the timer alarm sound")).ok()?;
        let filters = [COMDLG_FILTERSPEC { pszName: w!("Sound files"), pszSpec: w!("*.wav;*.mp3;*.m4a;*.wma;*.aac") }];
        dialog.SetFileTypes(&filters).ok()?;
        // Cancelling the dialog is an error result, which is simply "nothing chosen".
        dialog.Show(Some(hwnd)).ok()?;
        let name = dialog.GetResult().ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = name.to_string().ok().map(std::path::PathBuf::from);
        CoTaskMemFree(Some(name.0 as *const _));
        path
    }
}

impl App {
    /// Open the settings window, or bring it forward if it is already open.
    pub(crate) fn open_panel(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(p) = &self.panel {
            p.window.set_minimized(false);
            p.window.focus_window();
            return;
        }
        let mut attrs = Window::default_attributes()
            .with_title("Sky Island Settings")
            .with_inner_size(LogicalSize::new(WIDTH, HEIGHT))
            .with_resizable(true)
            .with_enabled_buttons(WindowButtons::CLOSE | WindowButtons::MINIMIZE | WindowButtons::MAXIMIZE);
        if let Ok(icon) = Icon::from_rgba(appicon::rgba(32), 32, 32) {
            attrs = attrs.with_window_icon(Some(icon));
        }
        let Ok(window) = event_loop.create_window(attrs) else { return };
        let window = Rc::new(window);
        // Where the window was last time, if that is still on a screen: it goes there first, so the screen it
        // is on (and its scale) is the one the size below is worked out for.
        let spot = Spot::load();
        let screens: Vec<_> = window.available_monitors().map(|m| (m.position().x, m.position().y, m.size().width, m.size().height)).collect();
        let at = spot.filter(|s| on_a_screen(s.x, s.y, &screens));
        if let Some(s) = at {
            window.set_outer_position(PhysicalPosition::new(s.x, s.y));
        }
        // On a screen too small for the whole window (a laptop at 150 percent), the window and everything
        // in it are shrunk to fit, with room left for the title bar and the taskbar.
        let monitor = window.current_monitor().or_else(|| window.primary_monitor());
        let sf = window.scale_factor();
        let screen = monitor.as_ref().map(|m| (m.size().width as f64, m.size().height as f64));
        let fit = screen.map_or(1.0, |(w, h)| fit_to_screen(w, h, sf));
        // The size it had last time, as far as this screen has room for it.
        let (width, height) = match (spot, screen) {
            (Some(s), Some((w, h))) => size_on_screen(s.width, s.height, w, h, sf, fit),
            _ => (WIDTH, HEIGHT),
        };
        if fit < 1.0 || (width, height) != (WIDTH, HEIGHT) {
            let _ = window.request_inner_size(LogicalSize::new(width * fit, height * fit));
        }
        // It can be made bigger (or full screen) but not smaller than the sidebar and a page need.
        window.set_min_inner_size(Some(LogicalSize::new(WIDTH * fit, MIN_HEIGHT * fit)));
        // Without a place to go back to: in the middle of the screen the notch is on.
        if at.is_none()
            && let Some(monitor) = monitor
        {
            let (pos, size) = (monitor.position(), monitor.size());
            let outer = window.outer_size();
            let x = pos.x + (size.width as i32 - outer.width as i32) / 2;
            let y = pos.y + (size.height as i32 - outer.height as i32) / 2;
            window.set_outer_position(PhysicalPosition::new(x, y));
        }
        let Ok(context) = Context::new(window.clone()) else { return };
        let Ok(surface) = Surface::new(&context, window.clone()) else { return };
        window.request_redraw();
        self.panel = Some(Panel {
            window,
            surface,
            glows: Glows::default(),
            cursor: (-1.0, -1.0),
            over_window: false,
            drag: None,
            city: CityField::default(),
            capture: false,
            mods: winit::keyboard::ModifiersState::empty(),
            show_changelog: false,
            changelog_scroll: 0.0,
            changelog_max: 0.0,
            scroll: 0.0,
            scroll_max: 0.0,
            scroll_shown: 0.0,
            bar_drag: None,
            spot: None,
            page: Page::General,
            page_since: Instant::now().checked_sub(PAGE_FADE).unwrap_or_else(Instant::now),
            reset_armed: None,
            cleared: None,
            last_draw: Instant::now(),
            opened: Instant::now(),
            fit,
            busy: false,
            canvas: None,
            backdrop: None,
        });
    }

    /// Switch the open settings window to a page.
    pub(crate) fn show_page(&mut self, page: Page) {
        if let Some(p) = &mut self.panel {
            p.show(page);
        }
    }

    /// Open the settings window (or bring it forward) on a page; done when the event loop is next free to.
    pub(crate) fn open_settings_at(&mut self, page: Page) {
        self.panel_wanted = true;
        self.panel_page = Some(page);
        self.last_tick = Instant::now();
    }

    /// The page the settings window shows.
    pub(super) fn page(&self) -> Page {
        self.panel.as_ref().map_or(Page::General, |p| p.page)
    }

    /// An event for the settings window.
    pub(crate) fn panel_event(&mut self, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.panel = None,
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                // While a city is being typed, the keys are its own.
                if self.city_key(&event) || self.hotkey_key(&event) {
                    return;
                }
                let page = self.page();
                let step = |delta: isize| Page::ALL[(page.index() as isize + delta).clamp(0, Page::ALL.len() as isize - 1) as usize];
                match event.logical_key {
                    Key::Named(NamedKey::F11) => {
                        if let Some(p) = &self.panel {
                            let full = p.window.fullscreen().is_none().then_some(Fullscreen::Borderless(None));
                            p.window.set_fullscreen(full);
                        }
                    }
                    Key::Named(NamedKey::Escape) => match &mut self.panel {
                        // Escape first leaves full screen, then the changelog, then closes the window.
                        Some(p) if p.window.fullscreen().is_some() => p.window.set_fullscreen(None),
                        Some(p) if p.show_changelog => {
                            p.show_changelog = false;
                            p.page_since = Instant::now();
                            p.redraw();
                        }
                        _ => self.panel = None,
                    },
                    Key::Named(NamedKey::ArrowDown) => {
                        if let Some(p) = &mut self.panel {
                            p.show(step(1));
                        }
                    }
                    Key::Named(NamedKey::ArrowUp) => {
                        if let Some(p) = &mut self.panel {
                            p.show(step(-1));
                        }
                    }
                    Key::Named(named @ (NamedKey::PageUp | NamedKey::PageDown | NamedKey::Home | NamedKey::End)) => {
                        let key = match named {
                            NamedKey::PageUp => ScrollKey::PageUp,
                            NamedKey::PageDown => ScrollKey::PageDown,
                            NamedKey::Home => ScrollKey::Home,
                            _ => ScrollKey::End,
                        };
                        if let Some(p) = &mut self.panel {
                            p.scroll_key(key);
                        }
                    }
                    _ => {}
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                if let Some(p) = &mut self.panel {
                    p.mods = m.state();
                }
            }
            WindowEvent::ThemeChanged(theme) => {
                self.system_light = theme == winit::window::Theme::Light;
                self.settings_changed();
            }
            WindowEvent::Resized(_) | WindowEvent::Moved(_) | WindowEvent::Focused(_) => {
                if let Some(p) = &mut self.panel {
                    p.note_spot();
                    p.redraw();
                }
            }
            WindowEvent::CursorEntered { .. } => {
                if let Some(p) = &mut self.panel {
                    p.over_window = true;
                }
            }
            WindowEvent::CursorLeft { .. } => {
                if let Some(p) = &mut self.panel {
                    p.over_window = false;
                    p.cursor = (-1.0, -1.0);
                    p.redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let Some(p) = &mut self.panel else { return };
                let s = p.scale();
                p.cursor = (position.x as f32 / s, position.y as f32 / s);
                p.over_window = true;
                let y = p.cursor.1;
                p.drag_bar_to(y);
                let (cx, drag, page, frame) = (p.cursor.0, p.drag, p.page, frame_for(p.logical_size().0));
                p.redraw();
                if let Some(slide) = drag {
                    let lay = layout(page, frame);
                    if let Some(item) = lay.items.iter().find(|i| matches!(i.kind, Kind::Slider(k) if k == slide)) {
                        let v = slider_value(range_of(slide), item.ctrl, cx);
                        self.slider_set(slide, v);
                    }
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                // The scroll bar is taken hold of first, and let go of wherever the mouse is when the button comes up.
                if let Some(p) = &mut self.panel {
                    if state == ElementState::Pressed && p.cursor.0 >= SIDEBAR_W && p.grab_bar() {
                        return;
                    }
                    if state == ElementState::Released && p.bar_drag.take().is_some() {
                        p.redraw();
                    }
                }
                // A click anywhere stops the typing (a click on the box starts it again below).
                if state == ElementState::Pressed
                    && let Some(p) = &mut self.panel
                    && (p.city.typing.take().is_some() | std::mem::take(&mut p.capture))
                {
                    p.redraw();
                }
                let Some(p) = &self.panel else { return };
                let (cx, cy) = p.cursor;
                let frame = frame_for(p.logical_size().0);
                if state == ElementState::Released {
                    if let Some(slide) = self.panel.as_mut().and_then(|p| p.drag.take()) {
                        self.settings_changed();
                        if slide == Slide::Volume {
                            // Let the new loudness be heard.
                            self.play_sound(crate::sound::Chime::Reminder);
                        }
                    }
                    return;
                }
                // The sidebar: a page.
                if cx < SIDEBAR_W {
                    if let Some(page) = Page::ALL.into_iter().find(|pg| inside(side_rect(*pg), cx, cy))
                        && let Some(p) = &mut self.panel {
                            p.show(page);
                        }
                    return;
                }
                if p.show_changelog {
                    // Only the Back button does anything here.
                    if inside(changelog_back(frame), cx, cy)
                        && let Some(p) = &mut self.panel {
                            p.show_changelog = false;
                            p.page_since = Instant::now();
                            p.redraw();
                        }
                    return;
                }
                // The page scrolls under its title: clicks above that line miss, and the rest are where the rows are drawn.
                if cy < SCROLL_TOP {
                    return;
                }
                let cy = cy + p.scroll_shown;
                let page = p.page;
                let lay = layout(page, frame);
                let Some(item) = lay.items.iter().find(|i| match i.kind {
                    Kind::Slider(_) => inside((i.ctrl.0 - 8.0, i.row.1, SLIDER_W + 16.0, i.row.3), cx, cy),
                    Kind::Seg(_) | Kind::Pick | Kind::Text(_) | Kind::Cycle(_) | Kind::Button(_) | Kind::EyeGrid => inside(i.ctrl, cx, cy),
                    Kind::Link(url) => !url.is_empty() && inside(i.row, cx, cy),
                    Kind::Toggle(_) => inside(i.row, cx, cy),
                }) else {
                    return;
                };
                match item.kind {
                    Kind::Link(url) => {
                        let _ = std::process::Command::new("explorer").arg(url).spawn();
                    }
                    Kind::Toggle(t) => self.toggle(t),
                    Kind::Cycle(Stepper::Method) => {
                        // The left third steps back; the rest steps forward.
                        self.step_method(if cx < item.ctrl.0 + item.ctrl.2 / 3.0 { -1 } else { 1 });
                    }
                    Kind::Text(Field::Hotkey) => {
                        if let Some(p) = &mut self.panel {
                            p.capture = true;
                            p.redraw();
                        }
                    }
                    Kind::Text(Field::City) => {
                        if let Some(p) = &mut self.panel {
                            p.city.typing = Some(String::new());
                            p.city.status = CityStatus::Idle;
                            p.redraw();
                        }
                    }
                    Kind::EyeGrid => {
                        if let Some(i) = eye_tile_at(item.ctrl, cx, cy) {
                            self.set_eye_style(i);
                        }
                    }
                    Kind::Seg(seg) => {
                        let cells = seg_cells(seg, item.ctrl);
                        let i = cells.iter().position(|c| cx < c.0 + c.2).unwrap_or(cells.len() - 1);
                        self.set_seg(seg, i);
                    }
                    Kind::Slider(slide) => {
                        let v = slider_value(range_of(slide), item.ctrl, cx);
                        self.slider_set(slide, v);
                        if let Some(p) = &mut self.panel {
                            p.drag = Some(slide);
                            p.redraw();
                        }
                    }
                    Kind::Pick => {
                        if self.settings.alarm_sound.is_some() && cx >= item.ctrl.0 + item.ctrl.2 - 30.0 {
                            // The cross on the right goes back to the built-in chime.
                            crate::sound::stop_file();
                            self.settings.alarm_sound = None;
                            self.settings_changed();
                        } else if let Some(path) = self.panel.as_ref().and_then(|p| pick_sound(&p.window)) {
                            self.settings.alarm_sound = Some(path);
                            self.settings_changed();
                            // Let the new sound be heard (a file still playing is stopped first).
                            crate::sound::stop_file();
                            self.play_alarm();
                        }
                        if let Some(p) = &self.panel {
                            p.redraw();
                        }
                    }
                    Kind::Button(Act::DetectCity) => self.detect_city(),
                    Kind::Button(Act::ClearCity) => self.clear_city(),
                    Kind::Button(Act::OpenFolder) => {
                        if let Some(dir) = std::env::var_os("APPDATA").map(|d| std::path::PathBuf::from(d).join("Sky-Island")) {
                            let _ = std::fs::create_dir_all(&dir);
                            let _ = std::process::Command::new("explorer").arg(dir).spawn();
                        }
                    }
                    Kind::Button(Act::ClearLyrics) => {
                        let removed = crate::lyrics::clear_cache();
                        if let Some(p) = &mut self.panel {
                            p.cleared = Some((removed, Instant::now() + Duration::from_secs(3)));
                            p.redraw();
                        }
                    }
                    Kind::Button(Act::Changelog) => {
                        if let Some(p) = &mut self.panel {
                            p.show_changelog = true;
                            p.changelog_scroll = 0.0;
                            p.page_since = Instant::now();
                            p.redraw();
                        }
                        if matches!(self.changelog, crate::update::Changelog::Idle | crate::update::Changelog::Failed) {
                            self.changelog = crate::update::Changelog::Loading;
                            crate::update::spawn_changelog(self.proxy.clone());
                        }
                    }
                    Kind::Button(Act::CheckUpdate) => {
                        match self.update.clone() {
                            crate::update::Status::Available { version, asset: Some(asset), .. } => {
                                self.update = crate::update::Status::Downloading { version: version.clone(), percent: 0 };
                                crate::update::spawn_download(version, asset, self.proxy.clone());
                            }
                            crate::update::Status::Available { page, .. } => {
                                // A release without an installer: show its page instead.
                                let _ = std::process::Command::new("explorer").arg(page).spawn();
                            }
                            crate::update::Status::Ready { path, asset, .. } => {
                                // The installer closes this app, installs over it and starts it again. It is checked
                                // once more first, because the file has been lying in the temporary folder.
                                if crate::update::launch(&path, &asset) {
                                    self.quit = true;
                                } else {
                                    // The file was removed, changed or cannot be run: offer the download again.
                                    self.update = crate::update::Status::DownloadFailed;
                                }
                            }
                            crate::update::Status::Checking | crate::update::Status::Downloading { .. } => {}
                            _ => {
                                self.update = crate::update::Status::Checking;
                                crate::update::spawn_check(self.proxy.clone());
                            }
                        }
                        if let Some(p) = &self.panel {
                            p.redraw();
                        }
                    }
                    Kind::Button(Act::Reset) => {
                        let armed = self.panel.as_ref().is_some_and(|p| p.reset_armed.is_some_and(|t| Instant::now() < t));
                        if armed {
                            if let Some(p) = &mut self.panel {
                                p.reset_armed = None;
                            }
                            self.reset_settings();
                        } else if let Some(p) = &mut self.panel {
                            p.reset_armed = Some(Instant::now() + RESET_WINDOW);
                            p.redraw();
                        }
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let Some(p) = &mut self.panel else { return };
                if p.show_changelog {
                    let dy = crate::wheel(&delta, 54.0, 1.0) as f32;
                    p.changelog_scroll = (p.changelog_scroll - dy).clamp(0.0, p.changelog_max);
                    p.redraw();
                } else {
                    let dy = crate::wheel(&delta, 54.0, 1.0) as f32;
                    p.scroll = (p.scroll - dy).clamp(0.0, p.scroll_max);
                    p.redraw();
                }
            }
            WindowEvent::RedrawRequested => self.draw_panel(),
            _ => {}
        }
    }

    pub(super) fn draw_panel(&mut self) {
        let Some(mut panel) = self.panel.take() else { return };
        self.paint_panel(&mut panel);
        self.panel = Some(panel);
    }
}
