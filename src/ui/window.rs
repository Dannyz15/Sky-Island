//! The island's window: its size and place, the springs that move it, hover timing and visibility.

use crate::*;

/// The closed notch, about the size of a MacBook's.
pub(crate) const COLLAPSED: (f64, f64) = (148.0, 28.0);

/// Width of the curved flare on each side of the notch's top edge, where it meets the top of the screen.
/// The window is this much wider than the notch on both sides; the notch is drawn in the middle.
pub(crate) const EAR: f64 = 12.0;

pub(crate) const EXPANDED: (f64, f64) = (420.0, 176.0);

/// How much of a hidden island stays on the top edge of the screen, in logical pixels.
const TUCK_STRIP: f64 = 3.0;

impl App {
    fn target(&self) -> f64 {
        if self.hovered || self.drag_hover || self.editing || self.timer_field.is_some() || self.alert_until.is_some() { 1.0 } else { 0.0 }
    }

    /// With "Hide island" on, the island slides away whenever there is nothing to show or touch.
    pub(crate) fn tuck_target(&self) -> f64 {
        let needed = self.hovered
            || self.enter_at.is_some()
            || self.drag_hover
            || self.editing
            || self.timer_field.is_some()
            || self.alert_until.is_some()
            || self.notice_until.is_some()
            || self.progress > 0.0;
        if self.settings.hide_island && !needed { 1.0 } else { 0.0 }
    }

    /// Where the parts of the player sit right now.
    pub(crate) fn geo(&self) -> PlayerGeo {
        player_geo(self.view_t.clamp(0.0, 1.0) as f32)
    }

    pub(crate) fn animating(&self) -> bool {
        (self.progress - self.target()).abs() > f64::EPSILON
            || self.switch_t < 1.0
            || self.list_t < 1.0
            || self.cal_t < 1.0
            || self.track_t < 1.0
            || self.art_shown.iter().zip(self.art_tint).any(|(a, b)| (a - b as f32).abs() > 0.5)
            || self.tab_target().is_some_and(|t| (t - self.tab_pos).abs() > f32::EPSILON)
            || (self.tuck_t - self.tuck_target()).abs() > f64::EPSILON
            || (self.view_t - self.view_target()).abs() > f64::EPSILON
            || (self.ov_t - self.ov_target()).abs() > f64::EPSILON
            || self.glows.busy()
            || (self.closed_k - self.closed_target()).abs() > 1e-4
            || (self.target_k() - self.ui_k).abs() > 1e-3
            || (self.view == View::Lyrics
                && matches!(self.lyrics, LyricsState::Found(_))
                && (self.lyric_pos - self.lyric_target()).abs() > f64::EPSILON)
    }

    fn ov_target(&self) -> f64 {
        if self.notice_until.is_some() { 1.0 } else { 0.0 }
    }

    fn view_target(&self) -> f64 {
        if self.view == View::Lyrics { 1.0 } else { 0.0 }
    }

    /// Playback position, extrapolated since the last update from the media thread.
    pub(crate) fn position(&self) -> Duration {
        if let Some(at) = self.scrub {
            return at;
        }
        let mut p = self.media.position;
        if self.media.playing {
            p += self.media_at.elapsed();
        }
        if !self.media.duration.is_zero() {
            p = p.min(self.media.duration);
        }
        p
    }

    /// Advance animation, then resize/reposition the window to match.
    pub(crate) fn step(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f64().min(1.0 / 20.0);
        self.last_tick = now;

        // The Animation setting makes the springs slower or quicker, and can take the bounce out.
        let (speed, bounce) = (self.settings.motion.speed(), self.settings.motion.bounce());
        let tune = |omega: f64, zeta: f64| (omega * speed, if bounce { zeta } else { 1.0 });

        // Everything is pulled towards its goal by a spring. Opening swings out a little past the goal and
        // settles; closing and hiding glide in without bouncing.
        let target = self.target();
        let (omega, zeta) = if target > 0.5 { tune(24.0, 0.74) } else { tune(28.0, 1.0) };
        spring(&mut self.progress, &mut self.progress_v, target, dt, omega, zeta);

        let tuck_goal = self.tuck_target();
        spring(&mut self.tuck_t, &mut self.tuck_v, tuck_goal, dt, 22.0 * speed, 1.0);

        spring(&mut self.switch_t, &mut self.switch_v, 1.0, dt, 24.0 * speed, 1.0);
        if self.switch_t >= 1.0 {
            self.prev_view = None;
        }

        if let Some(target) = self.tab_target() {
            let (mut x, mut v) = (self.tab_pos as f64, self.tab_vel as f64);
            let (omega, zeta) = tune(28.0, 0.7);
            spring(&mut x, &mut v, target as f64, dt, omega, zeta);
            self.tab_pos = x as f32;
            self.tab_vel = v as f32;
        }

        // A list that changed (scrolled, or an entry came or went) lets its rows ease in.
        let sig = self.list_signature();
        if sig != self.list_sig {
            if sig.0 == self.list_sig.0 && self.progress > 0.5 {
                self.list_dir = if sig.1 != self.list_sig.1 {
                    if sig.1 > self.list_sig.1 { 1.0 } else { -1.0 }
                } else if sig.3 != self.list_sig.3 {
                    if sig.3 > self.list_sig.3 { 1.0 } else { -1.0 }
                } else if sig.2 > self.list_sig.2 {
                    -1.0
                } else {
                    1.0
                };
                self.list_t = 0.0;
                self.list_v = 0.0;
            }
            self.list_sig = sig;
        }
        spring(&mut self.list_t, &mut self.list_v, 1.0, dt, 17.0 * speed, if bounce { 0.9 } else { 1.0 });
        spring(&mut self.cal_t, &mut self.cal_v, 1.0, dt, 15.0 * speed, if bounce { 0.82 } else { 1.0 });
        spring(&mut self.track_t, &mut self.track_v, 1.0, dt, 14.0 * speed, 1.0);
        let k = (1.0 - (-dt * 9.0 * speed).exp()) as f32;
        for (shown, goal) in self.art_shown.iter_mut().zip(self.art_tint) {
            *shown += (goal as f32 - *shown) * k;
            if (*shown - goal as f32).abs() < 0.5 {
                *shown = goal as f32;
            }
        }

        let ot = self.ov_target();
        let (omega, zeta) = if ot > 0.5 { tune(26.0, 0.8) } else { tune(28.0, 1.0) };
        spring(&mut self.ov_t, &mut self.ov_v, ot, dt, omega, zeta);

        let vt = self.view_target();
        let (omega, zeta) = if vt > 0.5 { tune(24.0, 0.85) } else { tune(28.0, 1.0) };
        spring(&mut self.view_t, &mut self.view_v, vt, dt, omega, zeta);

        // The lyrics glide to the line being sung, or to wherever they were scrolled.
        if self.view == View::Lyrics && matches!(self.lyrics, LyricsState::Found(_)) {
            let goal = self.lyric_target();
            spring(&mut self.lyric_pos, &mut self.lyric_v, goal, dt, 14.0 * speed, 1.0);
        }

        let closed_goal = self.closed_target();
        spring(&mut self.closed_k, &mut self.closed_v, closed_goal, dt, 24.0 * speed, 1.0);
        self.ui_k = self.target_k();
        let k = self.ui_k;

        let Some(window) = self.window.clone() else { return };
        // The size may swing a little past its goal while the notch opens.
        let t = self.progress.max(0.0);
        let wide = self.view_t.clamp(0.0, 1.0);
        let full_h = lerp(EXPANDED.1, LYRICS_H, wide);
        // The closed notch grows to the compact notice size while a notice is showing.
        let ov = self.ov_t.clamp(0.0, 1.0);
        let closed = self.closed_size();
        let base = (lerp(closed.0, OVERLAY.0, ov), lerp(closed.1, OVERLAY.1, ov));
        // Opening the lyrics widens the notch evenly on both sides, so it stays centred on the screen.
        let w = lerp(base.0, lerp(EXPANDED.0, LYRICS_W, wide), t);
        let h = lerp(base.1, full_h, t);
        // The Size setting scales the window itself; everything in it is drawn at the same scale.
        let _ = window.request_inner_size(LogicalSize::new((w + 2.0 * EAR) * k, h * k));

        if let Some(monitor) = window.current_monitor().or_else(|| window.primary_monitor()) {
            let scale = window.scale_factor() * k;
            let mpos = monitor.position();
            let msize = monitor.size();
            self.screen_w = msize.width as f64 / window.scale_factor();
            let area = (mpos.x, mpos.y, mpos.x + msize.width as i32, mpos.y + msize.height as i32);
            if self.monitor_area != Some(area) {
                self.monitor_area = Some(area);
                self.apply_visibility();
            }
            let pw = (w + 2.0 * EAR) * scale;
            let x = mpos.x as f64 + (msize.width as f64 - pw) / 2.0;
            // Tucked away, only a thin strip stays on screen, so the cursor can still find the island.
            let y = mpos.y as f64 - self.tuck_t.clamp(0.0, 1.0) * (h - TUCK_STRIP) * scale;
            window.set_outer_position(PhysicalPosition::new(x, y));
        }
        window.request_redraw();
    }

    pub(crate) fn cursor_over_window(&self) -> bool {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let Some(w) = &self.window else { return false };
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt) }.is_err() {
            return false;
        }
        let (pos, size) = (w.outer_position().unwrap_or_default(), w.outer_size());
        pt.x >= pos.x && pt.x < pos.x + size.width as i32 && pt.y >= pos.y && pt.y < pos.y + size.height as i32
    }

    /// The scale of everything drawn on the notch: the screen's scale times the Size setting.
    pub(crate) fn ui_scale(&self) -> f64 {
        self.window.as_ref().map_or(1.0, |w| w.scale_factor()) * self.ui_k
    }

    /// The scale the Size setting asks for. Auto keeps the island the same share of the screen's width
    /// (normal at 1536 logical pixels, which is a 1920 pixel screen at 125 percent).
    fn target_k(&self) -> f64 {
        match self.settings.size.factor() {
            Some(k) => k,
            None => ((self.screen_w / 1536.0).clamp(0.9, 1.3) * 20.0).round() / 20.0,
        }
    }

    /// How much of its normal size the closed notch should have: smaller when the minimal notch is on
    /// and nothing is playing.
    fn closed_target(&self) -> f64 {
        if self.settings.minimal && !self.media.playing { 0.8 } else { 1.0 }
    }

    /// The closed notch, in logical pixels.
    pub(crate) fn closed_size(&self) -> (f64, f64) {
        (COLLAPSED.0 * self.closed_k, COLLAPSED.1 * self.closed_k)
    }

    /// The height of the window in logical pixels, which the tab bar is measured from.
    pub(crate) fn window_h(&self) -> f32 {
        let s = self.ui_scale() as f32;
        self.window.as_ref().map_or(EXPANDED.1 as f32, |w| w.inner_size().height as f32 / s)
    }

    /// How long the mouse rests on the notch before it opens, and how long it is away before it closes.
    pub(crate) fn hover_open(&self) -> Duration {
        Duration::from_millis(self.settings.open_delay as u64)
    }

    pub(crate) fn hover_close(&self) -> Duration {
        Duration::from_millis(self.settings.close_delay as u64)
    }

    /// Tray icon with a small menu, so the app can be closed without opening the notch.
    pub(crate) fn build_tray(&mut self) {
        let menu = Menu::new();
        let open = MenuItem::new("Settings\u{2026}", true, None);
        let about = MenuItem::new("About Sky Island", true, None);
        let quit = MenuItem::new("Exit", true, None);
        if menu.append_items(&[&open, &PredefinedMenuItem::separator(), &about, &quit]).is_err() {
            return;
        }
        let Ok(icon) = Icon::from_rgba(appicon::rgba(32), 32, 32) else { return };
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Sky Island")
            .with_icon(icon)
            .build();

        let proxy = std::sync::Mutex::new(self.proxy.clone());
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Ok(p) = proxy.lock() {
                let _ = p.send_event(UserEvent::Menu(event.id));
            }
        }));
        self.tray_about_id = Some(about.id().clone());
        self.tray_settings_id = Some(open.id().clone());
        self.tray_quit_id = Some(quit.id().clone());
        self.tray = tray.ok();
    }

    /// The notch window's Win32 handle, as a number so it can go to another thread.
    pub(crate) fn native_handle(&self) -> Option<isize> {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let handle = self.window.as_ref()?.window_handle().ok()?;
        match handle.as_raw() {
            RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
            _ => None,
        }
    }

    /// Make clicks on the notch not steal focus from the user's current app.
    pub(crate) fn make_non_activating(&self) {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_EX_NOACTIVATE};
        if let Some(h) = self.native_handle() {
            let hwnd = HWND(h as *mut _);
            unsafe {
                let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_NOACTIVATE.0 as isize);
            }
        }
    }

    /// Hide the window while a full-screen app is in front, if that setting is on.
    pub(crate) fn apply_visibility(&mut self) {
        let hide = self.fullscreen.hides(self.monitor_area) && self.settings.hide_fullscreen;
        if hide != self.hidden {
            self.hidden = hide;
            if hide {
                // A hidden window gets no cursor events, so don't leave it looking hovered.
                self.hovered = false;
                self.enter_at = None;
                self.leave_at = None;
            }
            if let Some(w) = &self.window {
                w.set_visible(!hide);
            }
        }
    }
}
