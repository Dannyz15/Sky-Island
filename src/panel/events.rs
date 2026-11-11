//! What the settings window does with keys, the mouse and the wheel.

use super::window::pick_sound;
use super::*;

impl App {
    /// A key pressed in the settings window.
    pub(super) fn panel_key(&mut self, event: winit::event::KeyEvent) {
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

    /// The mouse moved over the settings window.
    pub(super) fn panel_cursor_moved(&mut self, position: winit::dpi::PhysicalPosition<f64>) {
        let Some(p) = &mut self.panel else { return };
        let s = p.scale();
        p.cursor = (position.x as f32 / s, position.y as f32 / s);
        p.over_window = true;
        let y = p.cursor.1;
        p.drag_bar_to(y);
        let (cx, drag, page, frame) = (p.cursor.0, p.drag, p.page, frame_for(p.logical_size().0));
        p.redraw();
        if let Some(slide) = drag
            && let Some(p) = &self.panel
        {
            let lay = self.layout_of(p, page, frame);
            if let Some(item) = lay.items.iter().find(|i| matches!(i.kind, Kind::Slider(k) if k == slide)) {
                let v = slider_value(range_of(slide), item.ctrl, cx);
                self.slider_set(slide, v);
            }
        }
    }

    /// The left button went down or came up in the settings window.
    pub(super) fn panel_mouse_button(&mut self, state: ElementState) {
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
                && let Some(p) = &mut self.panel
            {
                p.show(page);
            }
            return;
        }
        if p.show_changelog {
            // Only the Back button does anything here.
            if inside(changelog_back(frame), cx, cy)
                && let Some(p) = &mut self.panel
            {
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
        let lay = self.layout_of(p, page, frame);
        let Some(item) = lay.items.iter().find(|i| match i.kind {
            Kind::Slider(_) => inside((i.ctrl.0 - 8.0, i.row.1, SLIDER_W + 16.0, i.row.3), cx, cy),
            Kind::Seg(_) | Kind::Pick | Kind::Text(_) | Kind::Cycle(_) | Kind::Button(_) | Kind::EyeGrid | Kind::ScreenMap | Kind::TabGrid => inside(i.ctrl, cx, cy),
            Kind::Link(url) => !url.is_empty() && inside(i.row, cx, cy),
            Kind::Toggle(_) => inside(i.row, cx, cy),
        }) else {
            return;
        };
        self.click_item(item, cx, cy);
    }

    /// A click on a row or control of the page.
    pub(super) fn click_item(&mut self, item: &Item, cx: f32, cy: f32) {
        match item.kind {
            Kind::Link(url) => {
                crate::open::open(url);
            }
            Kind::Toggle(t) => self.toggle(t),
            Kind::Cycle(stepper) => {
                // The left third steps back; the rest steps forward.
                let delta = if cx < item.ctrl.0 + item.ctrl.2 / 3.0 { -1 } else { 1 };
                match stepper {
                    Stepper::Method => self.step_method(delta),
                    Stepper::Zone => self.step_zone(delta),
                }
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
            Kind::TabGrid => {
                if let Some(i) = tab_tile_at(item.ctrl, cx, cy) {
                    self.toggle(TAB_TILES[i].1);
                }
            }
            Kind::ScreenMap => {
                if let Some(screen) = pick_at(item.ctrl, &self.detected_screens(), cx, cy) {
                    self.set_screen(screen);
                }
            }
            Kind::Seg(seg) => {
                let cells = seg_cells(seg, item.ctrl);
                let i = cells.iter().position(|c| cx < c.0 + c.2).unwrap_or(cells.len().saturating_sub(1));
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
            Kind::Pick => self.click_pick(item, cx),
            Kind::Button(act) => self.click_button(act),
        }
    }

    /// A click on the button that chooses the alarm sound.
    pub(super) fn click_pick(&mut self, item: &Item, cx: f32) {
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

    /// A click on a button.
    pub(super) fn click_button(&mut self, act: Act) {
        match act {
            Act::DetectCity => self.detect_city(),
            Act::ClearCity => self.clear_city(),
            Act::OpenFolder => {
                if let Some(dir) = std::env::var_os("APPDATA").map(|d| std::path::PathBuf::from(d).join("Sky-Island")) {
                    let _ = std::fs::create_dir_all(&dir);
                    crate::open::open(dir);
                }
            }
            Act::ClearLyrics => {
                let removed = crate::lyrics::clear_cache();
                if let Some(p) = &mut self.panel {
                    p.cleared = Some((removed, Instant::now() + Duration::from_secs(3)));
                    p.redraw();
                }
            }
            Act::Changelog => {
                if let Some(p) = &mut self.panel {
                    p.show_changelog = true;
                    p.changelog_scroll = 0.0;
                    p.page_since = Instant::now();
                    p.redraw();
                }
                if matches!(self.updates.changelog, crate::update::Changelog::Idle | crate::update::Changelog::Failed) {
                    self.updates.changelog = crate::update::Changelog::Loading;
                    crate::update::spawn_changelog(self.proxy.clone());
                }
            }
            Act::CheckUpdate => self.click_check_update(),
            Act::Reset => {
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

    /// A click on the update button: check, download, or install, by where it stands.
    pub(super) fn click_check_update(&mut self) {
        match self.updates.status.clone() {
            crate::update::Status::Available { version, asset: Some(asset), .. } => {
                self.updates.status = crate::update::Status::Downloading { version: version.clone(), percent: 0 };
                crate::update::spawn_download(version, asset, self.proxy.clone());
            }
            crate::update::Status::Available { page, .. } => {
                // A release without an installer: show its page instead.
                crate::open::open(page);
            }
            crate::update::Status::Ready { path, asset, .. } => {
                // The installer closes this app, installs over it and starts it again. It is checked
                // once more first, because the file has been lying in the temporary folder.
                if crate::update::launch(&path, &asset) {
                    self.quit = true;
                } else {
                    // The file was removed, changed or cannot be run: offer the download again.
                    self.updates.status = crate::update::Status::DownloadFailed;
                }
            }
            crate::update::Status::Checking | crate::update::Status::Downloading { .. } => {}
            _ => {
                self.updates.status = crate::update::Status::Checking;
                crate::update::spawn_check(self.proxy.clone());
            }
        }
        if let Some(p) = &self.panel {
            p.redraw();
        }
    }

    /// The wheel over the settings window.
    pub(super) fn panel_wheel(&mut self, delta: winit::event::MouseScrollDelta) {
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
}
