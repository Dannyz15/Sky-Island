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

/// Lists and the calendar easing in after they changed: how far along they are, the way they come from, and what the list was.
pub(crate) struct Slides {
    /// Rows of a list ease in after the list changed (it scrolled, an entry came or went): 0 right after,
    /// 1 once they have settled. `list_dir` is the way they come from, and `list_sig` what the list was.
    pub(crate) list_t: f64,
    /// How fast `list_t` is changing (the spring's speed).
    pub(crate) list_v: f64,
    pub(crate) list_dir: f32,
    pub(crate) list_sig: (View, usize, usize, i64),
    /// The calendar's month (or week) slides in the same way when the arrows are used.
    pub(crate) cal_t: f64,
    /// How fast `cal_t` is changing (the spring's speed).
    pub(crate) cal_v: f64,
    /// The way the new month (or week) comes from.
    pub(crate) cal_dir: f32,
}

/// The notch's springs: how far each view is from the player's height, the extra height for what is in the view, the closed notch's size, how open the notch is, and how far it is tucked away.
pub(crate) struct Springs {
    /// 0.0 = player height, 1.0 = lyrics height
    pub(crate) view_t: f64,
    pub(crate) view_v: f64,
    /// How much taller than normal the open notch is for what is in its view (the Stats view with many drives), in
    /// logical pixels, and how fast that is changing.
    pub(crate) grow_t: f64,
    pub(crate) grow_v: f64,
    /// How much of its normal size the closed notch has (smaller for the minimal notch), with its speed.
    pub(crate) closed_k: f64,
    pub(crate) closed_v: f64,
    /// 0.0 = collapsed, 1.0 = expanded. A spring, so it can swing a little past 1.0 when it opens.
    pub(crate) progress: f64,
    pub(crate) progress_v: f64,
    /// 0.0 = in place, 1.0 = slid up out of view (the "Auto-hide notch" setting).
    pub(crate) tuck_t: f64,
    pub(crate) tuck_v: f64,
    /// The `tuck_target` the animation is heading for, to restart its clock when that changes.
    pub(crate) tuck_goal: f64,
}

impl Springs {
    /// The springs at rest: the notch closed, and tucked away when `hidden` (the Auto-hide notch setting).
    pub(crate) fn new(hidden: bool) -> Self {
        Self {
            view_t: 0.0,
            view_v: 0.0,
            grow_t: 0.0,
            grow_v: 0.0,
            closed_k: 1.0,
            closed_v: 0.0,
            progress: 0.0,
            progress_v: 0.0,
            tuck_t: if hidden { 1.0 } else { 0.0 },
            tuck_v: 0.0,
            tuck_goal: if hidden { 1.0 } else { 0.0 },
        }
    }
}

impl Default for Slides {
    fn default() -> Self {
        Self { list_t: 1.0, list_v: 0.0, list_dir: 1.0, list_sig: (View::Player, 0, 0, 0), cal_t: 1.0, cal_v: 0.0, cal_dir: 1.0 }
    }
}

impl App {
    fn target(&self) -> f64 {
        if self.hovered || self.shelf_ui.drag_hover || self.edit.on || self.timer_entry.field.is_some() || self.notices.alert_until.is_some() { 1.0 } else { 0.0 }
    }

    /// With "Auto-hide notch" on, the notch slides away whenever there is nothing to show or touch.
    pub(crate) fn tuck_target(&self) -> f64 {
        let needed = self.hovered
            || self.enter_at.is_some()
            || self.shelf_ui.drag_hover
            || self.edit.on
            || self.timer_entry.field.is_some()
            || self.notices.alert_until.is_some()
            || self.notices.until.is_some()
            || self.previewing()
            || self.spring.progress > 0.0;
        if self.settings.hide_island && !needed { 1.0 } else { 0.0 }
    }

    /// Where the parts of the player sit right now.
    pub(crate) fn geo(&self) -> PlayerGeo {
        player_geo(self.spring.view_t.clamp(0.0, 1.0) as f32)
    }

    pub(crate) fn animating(&self) -> bool {
        (self.spring.progress - self.target()).abs() > f64::EPSILON
            || self.switch.t < 1.0
            || self.slides.list_t < 1.0
            || self.slides.cal_t < 1.0
            || self.art.track_t < 1.0
            || self.art.shown.iter().zip(self.art.tint).any(|(a, b)| (a - b as f32).abs() > 0.5)
            || self.tab_target().is_some_and(|t| (t - self.tab_bar.pos).abs() > f32::EPSILON)
            || (self.spring.tuck_t - self.tuck_target()).abs() > f64::EPSILON
            || (self.spring.view_t - self.view_target()).abs() > f64::EPSILON
            || (self.spring.grow_t - self.stats_extra()).abs() > f64::EPSILON
            || (self.notices.t - self.ov_target()).abs() > f64::EPSILON
            || self.glows.busy()
            || self.eyes_moving()
            || (self.music.k - self.music_target()).abs() > 1e-4
            || (self.music.w - self.music_extra_goal()).abs() > 1e-3
            || (self.spring.closed_k - self.closed_target()).abs() > 1e-4
            || (self.target_k() - self.ui_k).abs() > 1e-3
            || (self.view == View::Lyrics && matches!(self.lyric.state, LyricsState::Found(_)) && (self.lyric.pos - self.lyric_target()).abs() > f64::EPSILON)
    }

    fn ov_target(&self) -> f64 {
        if self.notices.until.is_some() { 1.0 } else { 0.0 }
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

    /// The screen the island is on: the one the Screen setting names, if it is connected, otherwise the main
    /// screen.
    pub(crate) fn island_monitor(&self, window: &Window) -> Option<winit::monitor::MonitorHandle> {
        // With All, the notch is on the screen the mouse last touched the top of (the main one until then).
        let monitors: Vec<_> = window.available_monitors().collect();
        let wanted = if self.settings.screen.follows() {
            self.follow
        } else {
            // "Display 2" is the second screen in order, not the one whose device name ends in 2: the names count up
            // every time a screen is plugged in (a second screen is often DISPLAY5), Windows' own Display 2 does not.
            let devices: Vec<u32> = monitors.iter().filter_map(|m| m.name().and_then(|name| display_number(&name))).collect();
            self.settings.screen.display_number().and_then(|n| screen_by_order(&devices, n))
        };
        let chosen = wanted.and_then(|n| monitors.into_iter().find(|m| m.name().and_then(|name| display_number(&name)) == Some(n)));
        chosen.or_else(|| window.primary_monitor()).or_else(|| window.current_monitor())
    }

    /// The hotkey: open the notch, or close it if it is open. (Opened this way it stays open until the key is pressed
    /// again or the mouse has been over it and left.) Nothing happens while a full-screen app hides the notch.
    pub(crate) fn toggle_by_key(&mut self) {
        if self.hidden || self.window.is_none() {
            return;
        }
        let open = self.hovered || self.spring.progress > 0.5;
        self.hovered = !open;
        self.enter_at = None;
        self.leave_at = None;
        self.last_tick = Instant::now();
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Tell the hotkey thread what combination the settings want.
    pub(crate) fn apply_hotkey(&mut self) {
        hotkey::set(self.settings.hotkey.then_some((self.settings.hotkey_mods, self.settings.hotkey_vk)));
    }

    /// Set up what "All" needs: while it is the Show on setting and there is another screen, the mouse is watched for
    /// touching that screen's top edge (`edge.rs`); otherwise nothing is watched. Run when the setting or the screens
    /// change and when the notch has moved.
    pub(crate) fn refresh_edges(&mut self) {
        // The closed notches on the other screens (for Every) are made or taken away to match, when the loop is next free.
        self.mirrors.wanted = true;
        let Some(window) = self.window.clone() else { return };
        if !self.settings.screen.follows() {
            self.follow = None;
            edge::disable();
            return;
        }
        let screens: Vec<edge::Screen> = window
            .available_monitors()
            .enumerate()
            .map(|(i, m)| edge::Screen {
                number: m.name().and_then(|n| display_number(&n)).unwrap_or(i as u32 + 1),
                x: m.position().x,
                y: m.position().y,
                width: m.size().width as i32,
                bar: top_inset(&m),
                scale: m.scale_factor() * self.ui_k,
            })
            .collect();
        let here = self.island_monitor(&window).map(|m| m.position());
        let current = here.and_then(|p| screens.iter().find(|s| (s.x, s.y) == (p.x, p.y))).map_or(0, |s| s.number);
        let zones = edge::zones_for(&screens, current, COLLAPSED.0 + 2.0 * EAR);
        if zones.is_empty() {
            edge::disable();
        } else {
            edge::enable(self.proxy.clone());
            edge::set_zones(zones);
        }
    }

    /// The notch goes to a screen (given by its device number): the one the mouse touched the top of, or came onto a
    /// copy of the notch on. Only with a Show on setting that follows the mouse.
    pub(crate) fn follow_to(&mut self, screen: u32) {
        if self.settings.screen.follows() && self.follow != Some(screen) {
            // The notch moves there, and the other screens are watched instead.
            self.follow = Some(screen);
            self.rate_read = None;
            self.monitor_area = None;
            self.last_tick = Instant::now();
            self.refresh_edges();
            self.step();
        }
    }

    /// When the next frame of a running animation is due: the next step of the screen's rhythm after `now`,
    /// so frames come at its refresh rate however long drawing one takes (as long as it fits in one).
    pub(crate) fn next_frame(&mut self, now: Instant) -> Instant {
        let (beat, next) = next_beat(self.beat, self.frame, now);
        self.beat = beat;
        next
    }

    /// Advance animation, then resize/reposition the window to match.
    pub(crate) fn step(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f64().min(1.0 / 20.0);
        self.last_tick = now;

        // The Animation setting makes the springs slower or quicker, and can take the bounce out.
        let (speed, bounce) = (self.settings.motion.speed(), self.settings.motion.bounce());
        let tune = |omega: f64, zeta: f64| (omega * speed, if bounce { zeta } else { 1.0 });
        self.eyes_step(dt, speed, bounce);

        // Everything is pulled towards its goal by a spring. Opening swings out a little past the goal and
        // settles; closing and hiding glide in without bouncing.
        let target = self.target();
        let (omega, zeta) = if target > 0.5 { tune(24.0, 0.74) } else { tune(28.0, 1.0) };
        spring(&mut self.spring.progress, &mut self.spring.progress_v, target, dt, omega, zeta);

        let tuck_goal = self.tuck_target();
        spring(&mut self.spring.tuck_t, &mut self.spring.tuck_v, tuck_goal, dt, 22.0 * speed, 1.0);

        spring(&mut self.switch.t, &mut self.switch.v, 1.0, dt, 24.0 * speed, 1.0);
        if self.switch.t >= 1.0 {
            self.switch.prev = None;
        }

        if let Some(target) = self.tab_target() {
            let (mut x, mut v) = (self.tab_bar.pos as f64, self.tab_bar.vel as f64);
            let (omega, zeta) = tune(28.0, 0.7);
            spring(&mut x, &mut v, target as f64, dt, omega, zeta);
            self.tab_bar.pos = x as f32;
            self.tab_bar.vel = v as f32;
        }

        // A list that changed (scrolled, or an entry came or went) lets its rows ease in.
        let sig = self.list_signature();
        if sig != self.slides.list_sig {
            if sig.0 == self.slides.list_sig.0 && self.spring.progress > 0.5 {
                self.slides.list_dir = if sig.1 != self.slides.list_sig.1 {
                    if sig.1 > self.slides.list_sig.1 { 1.0 } else { -1.0 }
                } else if sig.3 != self.slides.list_sig.3 {
                    if sig.3 > self.slides.list_sig.3 { 1.0 } else { -1.0 }
                } else if sig.2 > self.slides.list_sig.2 {
                    -1.0
                } else {
                    1.0
                };
                self.slides.list_t = 0.0;
                self.slides.list_v = 0.0;
            }
            self.slides.list_sig = sig;
        }
        spring(&mut self.slides.list_t, &mut self.slides.list_v, 1.0, dt, 17.0 * speed, if bounce { 0.9 } else { 1.0 });
        spring(&mut self.slides.cal_t, &mut self.slides.cal_v, 1.0, dt, 15.0 * speed, if bounce { 0.82 } else { 1.0 });
        spring(&mut self.art.track_t, &mut self.art.track_v, 1.0, dt, 14.0 * speed, 1.0);
        let k = (1.0 - (-dt * 9.0 * speed).exp()) as f32;
        for (shown, goal) in self.art.shown.iter_mut().zip(self.art.tint) {
            *shown += (goal as f32 - *shown) * k;
            if (*shown - goal as f32).abs() < 0.5 {
                *shown = goal as f32;
            }
        }

        let ot = self.ov_target();
        let (omega, zeta) = if ot > 0.5 { tune(26.0, 0.8) } else { tune(28.0, 1.0) };
        spring(&mut self.notices.t, &mut self.notices.v, ot, dt, omega, zeta);

        let vt = self.view_target();
        let (omega, zeta) = if vt > 0.5 { tune(24.0, 0.85) } else { tune(28.0, 1.0) };
        spring(&mut self.spring.view_t, &mut self.spring.view_v, vt, dt, omega, zeta);

        // The notch grows taller when what is in the open view needs more room (many drives in the Stats view).
        let grow = self.stats_extra();
        let (omega, zeta) = if grow > self.spring.grow_t { tune(24.0, 0.85) } else { tune(28.0, 1.0) };
        spring(&mut self.spring.grow_t, &mut self.spring.grow_v, grow, dt, omega, zeta);

        // The lyrics glide to the line being sung, or to wherever they were scrolled.
        if self.view == View::Lyrics && matches!(self.lyric.state, LyricsState::Found(_)) {
            let goal = self.lyric_target();
            spring(&mut self.lyric.pos, &mut self.lyric.v, goal, dt, 14.0 * speed, 1.0);
        }

        let closed_goal = self.closed_target();
        spring(&mut self.spring.closed_k, &mut self.spring.closed_v, closed_goal, dt, 24.0 * speed, 1.0);
        // The closed notch widens for the album cover and the bars while a song plays, and goes back when it stops.
        let (omega, zeta) = tune(22.0, 0.85);
        let music_goal = self.music_target();
        spring(&mut self.music.k, &mut self.music.v, music_goal, dt, omega, zeta);
        let extra_goal = self.music_extra_goal();
        spring(&mut self.music.w, &mut self.music.wv, extra_goal, dt, omega, zeta);
        self.ui_k = self.target_k();
        let k = self.ui_k;

        let Some(window) = self.window.clone() else { return };
        // The size may swing a little past its goal while the notch opens.
        let t = self.spring.progress.max(0.0);
        let wide = self.spring.view_t.clamp(0.0, 1.0);
        let full_h = lerp(EXPANDED.1, LYRICS_H, wide) + self.spring.grow_t.max(0.0);
        // The closed notch grows to the compact notice size while a notice is showing.
        let ov = self.notices.t.clamp(0.0, 1.0);
        let closed = self.closed_size();
        let base = (lerp(closed.0, OVERLAY.0, ov), lerp(closed.1, OVERLAY.1, ov));
        // Opening the lyrics widens the notch evenly on both sides, so it stays centred on the screen.
        let w = lerp(base.0, lerp(EXPANDED.0, LYRICS_W, wide), t);
        let h = lerp(base.1, full_h, t);
        // The Size setting scales the window itself; everything in it is drawn at the same scale.
        let _ = window.request_inner_size(LogicalSize::new((w + 2.0 * EAR) * k, h * k));

        if let Some(monitor) = self.island_monitor(&window) {
            let scale = window.scale_factor() * k;
            let mpos = monitor.position();
            let msize = monitor.size();
            self.screen_w = msize.width as f64 / window.scale_factor();
            if self.rate_read.is_none_or(|at| at.elapsed() > RATE_RECHECK) {
                self.rate_read = Some(Instant::now());
                if let Some(millihertz) = monitor.refresh_rate_millihertz() {
                    self.frame = frame_length(millihertz);
                }
            }
            let area = (mpos.x, mpos.y, mpos.x + msize.width as i32, mpos.y + msize.height as i32);
            if self.monitor_area != Some(area) {
                self.monitor_area = Some(area);
                self.apply_visibility();
            }
            let pw = (w + 2.0 * EAR) * scale;
            let x = mpos.x as f64 + (msize.width as f64 - pw) / 2.0;
            // Tucked away, only a thin strip stays on screen, so the cursor can still find the island. A taskbar
            // along the top of the screen keeps it below, instead of covering the middle of the bar.
            let top = mpos.y as f64 + top_inset(&monitor) as f64;
            let y = top - self.spring.tuck_t.clamp(0.0, 1.0) * (h - TUCK_STRIP) * scale;
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
        self.k_for(self.screen_w)
    }

    /// The Size setting's scale on a screen that is `screen_w` logical pixels wide (Auto depends on the width).
    pub(crate) fn k_for(&self, screen_w: f64) -> f64 {
        match self.settings.size.factor() {
            Some(k) => k,
            None => ((screen_w / 1536.0).clamp(0.9, 1.3) * 20.0).round() / 20.0,
        }
    }

    /// How much of its normal size the closed notch should have: smaller when the minimal notch is on
    /// and nothing is playing.
    fn closed_target(&self) -> f64 {
        if self.settings.minimal && !self.media.playing { 0.8 } else { 1.0 }
    }

    /// The closed notch, in logical pixels.
    pub(crate) fn closed_size(&self) -> (f64, f64) {
        (COLLAPSED.0 * self.spring.closed_k + self.music.w.max(0.0), COLLAPSED.1 * self.spring.closed_k)
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

    /// The notch window's Win32 handle, as a number so it can go to another thread.
    pub(crate) fn native_handle(&self) -> Option<isize> {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let handle = self.window.as_ref()?.window_handle().ok()?;
        match handle.as_raw() {
            RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
            _ => None,
        }
    }

    /// Remember the window the user is in, so the keyboard can be handed back to it after typing in the notch.
    pub(crate) fn remember_foreground(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
        let front = unsafe { GetForegroundWindow() };
        self.edit.prev_foreground = (!front.is_invalid()).then_some(front);
    }

    /// Give the keyboard back to the window remembered, if there is one.
    pub(crate) fn restore_foreground(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;
        if let Some(window) = self.edit.prev_foreground.take() {
            let _ = unsafe { SetForegroundWindow(window) };
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

/// The number Windows gives a screen, from the device name winit reports (`\\.\DISPLAY2` is 2).
pub(crate) fn display_number(device: &str) -> Option<u32> {
    let digits: String = device.chars().rev().take_while(char::is_ascii_digit).collect::<Vec<_>>().into_iter().rev().collect();
    digits.parse().ok()
}

/// The device number of the screen that is number `n` (1 is the first) when the connected screens are put in order of
/// their device numbers; `None` if there are fewer than `n` screens.
pub(crate) fn screen_by_order(devices: &[u32], n: u32) -> Option<u32> {
    let mut sorted = devices.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    sorted.get((n as usize).checked_sub(1)?).copied()
}

/// How far down from the top of the screen the usable area starts: more than zero when a taskbar (or any
/// other bar) is docked along the top.
pub(crate) fn top_inset(monitor: &winit::monitor::MonitorHandle) -> i32 {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO};
    use winit::platform::windows::MonitorHandleExtWindows;
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    let handle = HMONITOR(HANDLE(monitor.hmonitor() as *mut _).0);
    if unsafe { GetMonitorInfoW(handle, &mut info) }.as_bool() { (info.rcWork.top - info.rcMonitor.top).max(0) } else { 0 }
}

/// How often the screen's refresh rate is read again while the notch animates.
const RATE_RECHECK: Duration = Duration::from_secs(2);

/// How long one refresh of a screen takes, given its rate in thousandths of a hertz, kept to something a
/// timer can keep up with. Windows reports 0 or 1 Hz for "whatever the hardware does" (some virtual and
/// remote screens): that counts as 60.
pub(crate) fn frame_length(millihertz: u32) -> Duration {
    if millihertz < 24_000 {
        return Duration::from_micros(16_667);
    }
    Duration::from_micros(1_000_000_000 / u64::from(millihertz)).clamp(Duration::from_millis(3), Duration::from_millis(33))
}

/// The rhythm of frames: given when the beat started, how long a frame is and the time now, the beat to
/// keep and the first step of it after now. A pause of a few frames starts a new beat at `now`.
fn next_beat(beat: Instant, frame: Duration, now: Instant) -> (Instant, Instant) {
    let late = now.saturating_duration_since(beat);
    if late > frame * 4 {
        return (now, now + frame);
    }
    let steps = late.as_nanos() / frame.as_nanos().max(1) + 1;
    (beat, beat + frame * steps as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_two_is_the_second_screen_whatever_its_device_name() {
        // A second screen that Windows named DISPLAY5, because others came and went.
        assert_eq!(screen_by_order(&[1, 5], 2), Some(5));
        assert_eq!(screen_by_order(&[5, 1], 2), Some(5), "the order they were listed in does not matter");
        assert_eq!(screen_by_order(&[1, 2, 7], 3), Some(7));
        assert_eq!(screen_by_order(&[1, 5], 3), None, "no third screen");
        assert_eq!(screen_by_order(&[1], 2), None, "no second screen: the main one is used");
        assert_eq!(screen_by_order(&[1, 5], 0), None);
    }

    #[test]
    fn a_screen_is_known_by_the_number_in_its_device_name() {
        assert_eq!(display_number(r"\\.\DISPLAY1"), Some(1));
        assert_eq!(display_number(r"\\.\DISPLAY12"), Some(12));
        assert_eq!(display_number(r"\\.\DISPLAY"), None);
        assert_eq!(display_number(""), None);
    }

    #[test]
    fn a_frame_lasts_one_refresh_of_the_screen() {
        assert_eq!(frame_length(60_000), Duration::from_micros(16_666));
        assert_eq!(frame_length(144_000), Duration::from_micros(6_944));
        assert_eq!(frame_length(59_940), Duration::from_micros(16_683));
        assert_eq!(frame_length(1_000_000), Duration::from_millis(3), "an absurdly fast screen is held to what a timer can do");
        assert_eq!(frame_length(0), Duration::from_micros(16_667), "Windows says 0 Hz for the hardware default");
        assert_eq!(frame_length(1_000), Duration::from_micros(16_667), "and 1 Hz");
    }

    #[test]
    fn frames_follow_one_steady_rhythm() {
        let (start, frame) = (Instant::now(), Duration::from_millis(10));
        // Right on the beat, or a little late: the next step of the rhythm, whatever the drawing took.
        assert_eq!(next_beat(start, frame, start), (start, start + frame));
        assert_eq!(next_beat(start, frame, start + Duration::from_millis(3)), (start, start + frame));
        assert_eq!(next_beat(start, frame, start + Duration::from_millis(10)), (start, start + frame * 2));
        assert_eq!(next_beat(start, frame, start + Duration::from_millis(27)), (start, start + frame * 3), "two frames missed: skip to the next step");
        // After a pause the rhythm starts again from now.
        let later = start + Duration::from_secs(5);
        assert_eq!(next_beat(start, frame, later), (later, later + frame));
    }
}
