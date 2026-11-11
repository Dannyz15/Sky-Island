//! Opening the settings window and sending what happens in it on.

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
            last_switch: Instant::now().checked_sub(PAGE_FADE).unwrap_or_else(Instant::now),
            reset_armed: None,
            cleared: None,
            last_draw: Instant::now(),
            opened: Instant::now(),
            fit,
            busy: false,
            canvas: None,
            backdrop: None,
            page_back: None,
            glow_mask: None,
            tinted: None,
            layer_back: None,
            layer: None,
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
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => self.panel_key(event),
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
            WindowEvent::CursorMoved { position, .. } => self.panel_cursor_moved(position),
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => self.panel_mouse_button(state),
            WindowEvent::MouseWheel { delta, .. } => self.panel_wheel(delta),
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
