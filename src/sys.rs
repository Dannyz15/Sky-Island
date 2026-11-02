//! System watchers (volume, audio device, brightness, lock keys, battery, network).
//! Each pushes a `Notice` to the UI only when something actually changes.

use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use windows::Networking::Connectivity::{NetworkInformation, NetworkStatusChangedEventHandler};
use windows::Win32::Devices::Display::{DISPLAY_BRIGHTNESS, IOCTL_VIDEO_QUERY_DISPLAY_BRIGHTNESS};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl};
use windows::Win32::Media::Audio::{
    AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE, EDataFlow, ERole, IMMDevice, IMMDeviceEnumerator, IMMNotificationClient,
    IMMNotificationClient_Impl, MMDeviceEnumerator, PKEY_AudioEndpoint_FormFactor, eConsole, eRender,
};
use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, STGM_READ};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Power::{
    GetSystemPowerStatus, RegisterPowerSettingNotification, SYSTEM_POWER_STATUS,
};
use windows::Win32::System::SystemServices::{
    GUID_ACDC_POWER_SOURCE, GUID_BATTERY_PERCENTAGE_REMAINING, GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CAPITAL, VK_NUMLOCK};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DEVICE_NOTIFY_WINDOW_HANDLE, DefWindowProcW, DispatchMessageW, GetMessageW, KBDLLHOOKSTRUCT, MSG,
    PBT_POWERSETTINGCHANGE, SetWindowsHookExW, TranslateMessage, WH_KEYBOARD_LL, WM_KEYDOWN, WM_POWERBROADCAST, WM_SYSKEYDOWN,
};
use windows::core::{PCWSTR, Result, implement, w};
use winit::event_loop::EventLoopProxy;

use crate::{UserEvent, msgwin};

/// Something worth showing briefly in the notch.
#[derive(Clone)]
pub enum Notice {
    Volume { percent: u32, muted: bool },
    Brightness { percent: u32 },
    Lock { name: &'static str, on: bool },
    Battery { percent: u32, kind: BatteryKind },
    Network { connected: bool, name: String, wifi: bool },
    Audio { name: String, headphones: bool },
    /// A newer version of Sky Island exists.
    Update { version: String },
    /// A prayer time has come: its name and the minute of the day.
    Prayer { name: &'static str, minute: u32 },
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum BatteryKind {
    Plugged,
    Unplugged,
    Low,
}

type Proxy = EventLoopProxy<UserEvent>;

fn send(proxy: &Proxy, notice: Notice) {
    let _ = proxy.send_event(UserEvent::Notice(notice));
}

/// Start every watcher except the keyboard hook, which has its own start (`install_lock_hook`).
pub fn spawn_all(proxy: Proxy) {
    let p = proxy.clone();
    thread::spawn(move || audio(p));
    let p = proxy.clone();
    thread::spawn(move || battery(p));
    let p = proxy.clone();
    thread::spawn(move || brightness(p));
    thread::spawn(move || network(proxy));
}

// ---- Audio: volume, mute and default output device ----

enum AudioMsg {
    DefaultChanged,
    /// The user moved the volume slider in the notch: 0.0 to 1.0.
    SetVolume(f32),
    ToggleMute,
}

/// How the notch's volume slider reaches the audio thread.
static AUDIO_TX: OnceLock<Sender<AudioMsg>> = OnceLock::new();

/// Set the volume of the default output device (0 to 100 percent). Any volume above zero also unmutes it.
pub fn set_volume(percent: u32) {
    if let Some(tx) = AUDIO_TX.get() {
        let _ = tx.send(AudioMsg::SetVolume(percent.min(100) as f32 / 100.0));
    }
}

/// Mute the default output device, or unmute it.
pub fn toggle_mute() {
    if let Some(tx) = AUDIO_TX.get() {
        let _ = tx.send(AudioMsg::ToggleMute);
    }
}

#[implement(IAudioEndpointVolumeCallback)]
struct VolumeWatch {
    proxy: Mutex<Proxy>,
}

impl IAudioEndpointVolumeCallback_Impl for VolumeWatch_Impl {
    fn OnNotify(&self, pnotify: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> Result<()> {
        if let Some(data) = unsafe { pnotify.as_ref() } {
            let notice = Notice::Volume {
                percent: (data.fMasterVolume * 100.0).round() as u32,
                muted: data.bMuted.as_bool(),
            };
            if let Ok(p) = self.proxy.lock() {
                send(&p, notice);
            }
        }
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct DeviceWatch {
    tx: Mutex<Sender<AudioMsg>>,
}

impl IMMNotificationClient_Impl for DeviceWatch_Impl {
    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> Result<()> {
        Ok(())
    }
    fn OnDeviceAdded(&self, _: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn OnDeviceRemoved(&self, _: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _: &PCWSTR) -> Result<()> {
        if flow == eRender && role == eConsole
            && let Ok(tx) = self.tx.lock() {
                let _ = tx.send(AudioMsg::DefaultChanged);
            }
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &windows::Win32::Foundation::PROPERTYKEY) -> Result<()> {
        Ok(())
    }
}

/// An output device's friendly name and whether it is headphones or a headset.
fn describe(device: &IMMDevice) -> Option<(String, bool)> {
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ).ok()?;
        let name = store.GetValue(&PKEY_Device_FriendlyName).ok()?.to_string();
        // EndpointFormFactor: 3 = Headphones, 5 = Headset.
        let form = store
            .GetValue(&PKEY_AudioEndpoint_FormFactor)
            .ok()
            .and_then(|v| u32::try_from(&v).ok())
            .unwrap_or(0);
        Some((name, matches!(form, 3 | 5)))
    }
}

fn audio(proxy: Proxy) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let Ok(enumerator) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else {
            return;
        };

        let (tx, rx) = mpsc::channel();
        let _ = AUDIO_TX.set(tx.clone());
        let client: IMMNotificationClient = DeviceWatch { tx: Mutex::new(tx) }.into();
        let _ = enumerator.RegisterEndpointNotificationCallback(&client);

        // Volume callback registration for the current default output device.
        let mut attached: Option<(IAudioEndpointVolume, IAudioEndpointVolumeCallback)> = None;
        let attach = |attached: &mut Option<(IAudioEndpointVolume, IAudioEndpointVolumeCallback)>| -> Option<IMMDevice> {
            if let Some((vol, cb)) = attached.take() {
                let _ = vol.UnregisterControlChangeNotify(&cb);
            }
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
            let vol: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None).ok()?;
            let cb: IAudioEndpointVolumeCallback = VolumeWatch { proxy: Mutex::new(proxy.clone()) }.into();
            vol.RegisterControlChangeNotify(&cb).ok()?;
            *attached = Some((vol, cb));
            Some(device)
        };

        // Tell the notch how loud the device is, for its volume slider (this is not a notice).
        let announce = |attached: &Option<(IAudioEndpointVolume, IAudioEndpointVolumeCallback)>| {
            if let Some((vol, _)) = attached
                && let (Ok(level), Ok(mute)) = (vol.GetMasterVolumeLevelScalar(), vol.GetMute())
            {
                let _ = proxy.send_event(UserEvent::SysVolume { percent: (level * 100.0).round() as u32, muted: mute.as_bool() });
            }
        };
        attach(&mut attached);
        announce(&attached);
        while let Ok(msg) = rx.recv() {
            match msg {
                AudioMsg::DefaultChanged => {
                    if let Some((name, headphones)) = attach(&mut attached).as_ref().and_then(describe) {
                        send(&proxy, Notice::Audio { name, headphones });
                    }
                    announce(&attached);
                }
                AudioMsg::SetVolume(level) => {
                    if let Some((vol, _)) = &attached {
                        let _ = vol.SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), std::ptr::null());
                        if level > 0.0 {
                            let _ = vol.SetMute(false, std::ptr::null());
                        }
                    }
                }
                AudioMsg::ToggleMute => {
                    if let Some((vol, _)) = &attached
                        && let Ok(mute) = vol.GetMute()
                    {
                        let _ = vol.SetMute(!mute.as_bool(), std::ptr::null());
                    }
                }
            }
        }
    }
}

// ---- Battery ----

/// How often the battery is looked at if Windows cannot tell the app when its state changes.
const BATTERY_POLL: Duration = Duration::from_secs(3);

/// What the battery looked like at the last look, to tell what changed.
struct BatteryWatch {
    ac: Option<u8>,
    percent: Option<u32>,
}

impl BatteryWatch {
    /// The notices that a new look calls for: one when the charger is plugged in or pulled out, and one as
    /// the charge falls through 20, 10 and 5 percent while unplugged. `ac` and `percent` are Windows' own
    /// numbers, 255 meaning unknown.
    fn look(&mut self, ac: u8, percent: u8) -> Vec<BatteryKind> {
        let mut notices = Vec::new();
        if percent == 255 || ac == 255 {
            return notices;
        }
        let percent = percent as u32;
        if let Some(prev) = self.ac
            && prev != ac
        {
            notices.push(if ac == 1 { BatteryKind::Plugged } else { BatteryKind::Unplugged });
        }
        if let (Some(prev), 0) = (self.percent, ac)
            && [20, 10, 5].iter().any(|&t| prev > t && percent <= t)
        {
            notices.push(BatteryKind::Low);
        }
        self.ac = Some(ac);
        self.percent = Some(percent);
        notices
    }
}

static BATTERY: Mutex<BatteryWatch> = Mutex::new(BatteryWatch { ac: None, percent: None });
static BATTERY_PROXY: OnceLock<Proxy> = OnceLock::new();

/// Whether this PC has a battery to watch.
fn has_battery() -> bool {
    let mut st = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut st) }.is_ok() && st.BatteryFlag & 128 == 0
}

/// Look at the power status now and tell the UI what changed since the last look.
fn look_at_battery() {
    let mut st = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut st) }.is_err() {
        return;
    }
    let (Ok(mut watch), Some(proxy)) = (BATTERY.lock(), BATTERY_PROXY.get()) else { return };
    for kind in watch.look(st.ACLineStatus, st.BatteryLifePercent) {
        send(proxy, Notice::Battery { percent: st.BatteryLifePercent as u32, kind });
    }
}

/// Windows says when the power source or the charge changes (to a hidden window), so nothing runs in
/// between. If that cannot be set up, the battery is looked at every few seconds instead.
fn battery(proxy: Proxy) {
    if !has_battery() || BATTERY_PROXY.set(proxy).is_err() {
        return; // no battery (desktop), nothing to watch
    }
    look_at_battery(); // the starting point; nothing is announced for it
    let listening = msgwin::spawn(w!("SkyIslandPower"), Some(power_proc), |window| unsafe {
        let target = HANDLE(window.0);
        let source = RegisterPowerSettingNotification(target, &GUID_ACDC_POWER_SOURCE, DEVICE_NOTIFY_WINDOW_HANDLE);
        let charge = RegisterPowerSettingNotification(target, &GUID_BATTERY_PERCENTAGE_REMAINING, DEVICE_NOTIFY_WINDOW_HANDLE);
        source.is_ok() && charge.is_ok()
    });
    if !listening {
        loop {
            thread::sleep(BATTERY_POLL);
            look_at_battery();
        }
    }
}

unsafe extern "system" fn power_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if message == WM_POWERBROADCAST && wparam.0 as u32 == PBT_POWERSETTINGCHANGE {
        look_at_battery();
        return LRESULT(1);
    }
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

// ---- Brightness (laptop panels) ----

/// How often the brightness is read: quickly for a few seconds after a change so a dragged slider is followed,
/// slowly otherwise (only when Windows cannot tell the app about changes; see `brightness`).
const BRIGHTNESS_IDLE: Duration = Duration::from_millis(300);
const BRIGHTNESS_QUICK: Duration = Duration::from_millis(50);
const BRIGHTNESS_QUICK_FOR: Duration = Duration::from_secs(3);
/// While a slider is dragged, Windows can report the previous step again for a moment (up to about 0.2 s once
/// the drag ends). A reading that goes against the way the brightness was just moving is only believed after it
/// has stayed this long. A change after the brightness has been still for `BRIGHTNESS_STILL` is believed at once.
const BRIGHTNESS_TURN_HOLD: Duration = Duration::from_millis(250);
const BRIGHTNESS_STILL: Duration = Duration::from_secs(1);

/// How long to wait before reading the brightness again, given how much of the "quick" period is left.
fn brightness_interval(quick_left: Duration) -> Duration {
    if quick_left.is_zero() { BRIGHTNESS_IDLE } else { BRIGHTNESS_QUICK }
}

/// What the brightness has been doing, to tell a real change from a step-old reading.
struct Seen {
    last: u32,
    /// 1 when the last change went up, -1 when it went down, 0 before any.
    heading: i8,
    moved: Instant,
    /// A reading that turned back, and since when it has been seen.
    held: Option<(u32, Instant)>,
}

impl Seen {
    fn new(first: u32, at: Instant) -> Self {
        Seen { last: first, heading: 0, moved: at, held: None }
    }

    /// Whether the reading `now`, taken at `at`, is a change worth announcing.
    fn look(&mut self, now: u32, at: Instant) -> bool {
        if now == self.last {
            self.held = None; // the step-old reading went away again
            return false;
        }
        let heading = if now > self.last { 1 } else { -1 };
        if self.heading != 0 && heading != self.heading && at.duration_since(self.moved) < BRIGHTNESS_STILL {
            match self.held {
                Some((value, since)) if value == now => {
                    if at.duration_since(since) < BRIGHTNESS_TURN_HOLD {
                        return false;
                    }
                }
                _ => {
                    self.held = Some((now, at));
                    return false;
                }
            }
        }
        self.held = None;
        self.heading = heading;
        self.moved = at;
        self.last = now;
        true
    }
}

/// The built-in panel (its handle as a number, so it can live in a static) and what its brightness has done.
struct Panel {
    lcd: usize,
    seen: Seen,
}

static PANEL: Mutex<Option<Panel>> = Mutex::new(None);
static BRIGHTNESS_PROXY: OnceLock<Proxy> = OnceLock::new();
/// The thread that reads the brightness; Windows' notifications wake it.
static BRIGHTNESS_READER: OnceLock<thread::Thread> = OnceLock::new();

/// The panel's brightness in percent for the power source in use, or `None` if the driver does not answer.
fn read_brightness(lcd: HANDLE) -> Option<u32> {
    let mut out = DISPLAY_BRIGHTNESS::default();
    let mut returned = 0u32;
    unsafe {
        DeviceIoControl(
            lcd,
            IOCTL_VIDEO_QUERY_DISPLAY_BRIGHTNESS,
            None,
            0,
            Some(&mut out as *mut _ as *mut _),
            std::mem::size_of::<DISPLAY_BRIGHTNESS>() as u32,
            Some(&mut returned),
            None,
        )
        .ok()?;
        let mut st = SYSTEM_POWER_STATUS::default();
        let _ = GetSystemPowerStatus(&mut st);
        Some(if st.ACLineStatus == 1 { out.ucACBrightness } else { out.ucDCBrightness } as u32)
    }
}

/// Read the brightness now and tell the UI if it changed. `Some(true)` while the brightness is on the move
/// (changed, or a reading that turned back is being checked), `None` when the panel cannot be read.
fn look_at_brightness() -> Option<bool> {
    let mut guard = PANEL.lock().ok()?;
    let panel = guard.as_mut()?;
    let now = read_brightness(HANDLE(panel.lcd as *mut _))?;
    let announce = panel.seen.look(now, Instant::now());
    if announce
        && let Some(proxy) = BRIGHTNESS_PROXY.get()
    {
        send(proxy, Notice::Brightness { percent: now });
    }
    Some(announce || panel.seen.held.is_some())
}

/// Windows says when the panel's brightness changes (to a hidden window), so nothing runs while it stays
/// the same. The notification is only a wake-up: Windows sends it a few times a second while a slider is
/// dragged, with a value that can be a step old, so the brightness is read directly, every 100 ms until it
/// has been still for a few seconds. The power source is watched too, because the brightness can differ
/// between the charger and the battery. If that cannot be set up, the brightness is read on a timer instead.
fn brightness(proxy: Proxy) {
    let lcd = unsafe {
        let Ok(lcd) = CreateFileW(w!("\\\\.\\LCD"), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, Default::default(), None) else {
            return; // no built-in panel, e.g. a desktop
        };
        lcd
    };
    // If the driver doesn't answer the query, give up rather than watching for nothing.
    let Some(first) = read_brightness(lcd) else {
        let _ = unsafe { CloseHandle(lcd) };
        return;
    };
    if BRIGHTNESS_PROXY.set(proxy).is_err() || BRIGHTNESS_READER.set(thread::current()).is_err() {
        return;
    }
    if let Ok(mut panel) = PANEL.lock() {
        *panel = Some(Panel { lcd: lcd.0 as usize, seen: Seen::new(first, Instant::now()) });
    }
    let listening = msgwin::spawn(w!("SkyIslandBrightness"), Some(brightness_proc), |window| unsafe {
        let target = HANDLE(window.0);
        let level = RegisterPowerSettingNotification(target, &GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS, DEVICE_NOTIFY_WINDOW_HANDLE);
        let source = RegisterPowerSettingNotification(target, &GUID_ACDC_POWER_SOURCE, DEVICE_NOTIFY_WINDOW_HANDLE);
        level.is_ok() && source.is_ok()
    });
    let mut quick_until = Instant::now();
    loop {
        let quick_left = quick_until.saturating_duration_since(Instant::now());
        if listening && quick_left.is_zero() {
            thread::park(); // until Windows reports a change
            quick_until = Instant::now() + BRIGHTNESS_QUICK_FOR;
        } else {
            thread::sleep(brightness_interval(quick_left));
        }
        match look_at_brightness() {
            Some(true) => quick_until = Instant::now() + BRIGHTNESS_QUICK_FOR,
            Some(false) => {}
            None => break,
        }
    }
    if let Ok(mut panel) = PANEL.lock() {
        *panel = None;
    }
    let _ = unsafe { CloseHandle(lcd) };
}

unsafe extern "system" fn brightness_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if message == WM_POWERBROADCAST && wparam.0 as u32 == PBT_POWERSETTINGCHANGE {
        if let Some(reader) = BRIGHTNESS_READER.get() {
            reader.unpark();
        }
        return LRESULT(1);
    }
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

// ---- Network ----

fn network(proxy: Proxy) {
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }

    fn current() -> (bool, String, bool) {
        match NetworkInformation::GetInternetConnectionProfile() {
            Ok(profile) => {
                let name = profile.ProfileName().map(|n| n.to_string()).unwrap_or_default();
                let wifi = profile.IsWlanConnectionProfile().unwrap_or(false);
                (true, name, wifi)
            }
            Err(_) => (false, String::new(), false),
        }
    }

    let last = Mutex::new(current());
    let handler = NetworkStatusChangedEventHandler::new(move |_| {
        let now = current();
        if let Ok(mut prev) = last.lock()
            && (prev.0, &prev.1) != (now.0, &now.1) {
                send(&proxy, Notice::Network { connected: now.0, name: now.1.clone(), wifi: now.2 });
                *prev = now;
            }
        Ok(())
    });
    if NetworkInformation::NetworkStatusChanged(&handler).is_err() {
        return;
    }
    // Keep the thread (and so the registration) alive.
    loop {
        thread::park();
    }
}

// ---- Caps Lock / Num Lock ----

static PROXY: OnceLock<Proxy> = OnceLock::new();

/// Install the low-level keyboard hook. Windows calls the hook on the thread that installed it and waits for
/// the answer before the key goes on to any app, and drops a hook that is too slow: so it gets a thread of its
/// own that does nothing else, and a busy notch cannot make typing wait.
/// The hook only says a lock key was pressed. The real on/off state is read a moment later on
/// the UI thread, because at hook time the key hasn't taken effect yet, and counting presses
/// drifts out of sync if a key-up is ever missed.
pub fn install_lock_hook(proxy: Proxy) {
    if PROXY.set(proxy).is_err() {
        return;
    }
    thread::spawn(|| unsafe {
        let _ = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), None, 0);
        // A hook needs its thread to keep taking messages.
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}

/// Whether a toggle key (Caps Lock, Num Lock) is currently on.
pub fn key_toggled(vk: i32) -> bool {
    unsafe { GetKeyState(vk) & 1 != 0 }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN) {
        let key = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let lock = match key.vkCode {
            v if v == VK_CAPITAL.0 as u32 => Some(("Caps Lock", VK_CAPITAL.0 as i32)),
            v if v == VK_NUMLOCK.0 as u32 => Some(("Num Lock", VK_NUMLOCK.0 as i32)),
            _ => None,
        };
        if let (Some((name, vk)), Some(proxy)) = (lock, PROXY.get()) {
            // Keep the hook fast (Windows drops slow hooks): wait for the key to take effect elsewhere.
            let proxy = proxy.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(40));
                let _ = proxy.send_event(UserEvent::LockKey { name, vk });
            });
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watch() -> BatteryWatch {
        BatteryWatch { ac: None, percent: None }
    }

    #[test]
    fn the_first_look_announces_nothing() {
        assert!(watch().look(0, 15).is_empty(), "already low when the app starts: no warning");
        assert!(watch().look(255, 255).is_empty(), "unknown values");
    }

    #[test]
    fn plugging_in_and_pulling_out_are_announced() {
        let mut w = watch();
        w.look(0, 80);
        assert_eq!(w.look(1, 80), vec![BatteryKind::Plugged]);
        assert!(w.look(1, 81).is_empty());
        assert_eq!(w.look(0, 81), vec![BatteryKind::Unplugged]);
    }

    #[test]
    fn the_low_battery_warning_comes_once_per_threshold_and_only_unplugged() {
        let mut w = watch();
        w.look(0, 22);
        assert!(w.look(0, 21).is_empty());
        assert_eq!(w.look(0, 20), vec![BatteryKind::Low], "falls through 20");
        assert!(w.look(0, 19).is_empty(), "no repeat below it");
        assert_eq!(w.look(0, 10), vec![BatteryKind::Low]);
        assert_eq!(w.look(0, 4), vec![BatteryKind::Low], "5 percent");
        let mut charging = watch();
        charging.look(1, 21);
        assert!(charging.look(1, 19).is_empty(), "no warning while the charger is in");
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_dragged_slider_is_followed_step_by_step() {
        let t = Instant::now();
        let mut seen = Seen::new(50, t);
        assert!(seen.look(48, t + ms(60)));
        assert!(seen.look(44, t + ms(120)));
        assert!(seen.look(40, t + ms(180)));
        assert!(!seen.look(40, t + ms(240)), "no change, nothing to announce");
    }

    #[test]
    fn a_step_old_reading_that_goes_away_again_is_not_shown() {
        let t = Instant::now();
        let mut seen = Seen::new(50, t);
        assert!(seen.look(40, t + ms(100)));
        assert!(!seen.look(44, t + ms(150)), "turned back for a moment");
        assert!(seen.held.is_some(), "still being watched");
        assert!(!seen.look(40, t + ms(200)), "back to the real value: nothing changed");
        assert!(seen.held.is_none());
        assert_eq!(seen.last, 40);
    }

    #[test]
    fn turning_back_for_real_is_shown_once_it_stays() {
        let t = Instant::now();
        let mut seen = Seen::new(50, t);
        assert!(seen.look(40, t + ms(100)));
        assert!(!seen.look(46, t + ms(150)));
        assert!(!seen.look(46, t + ms(300)), "not long enough yet");
        assert!(seen.look(46, t + ms(410)), "stayed for 250 ms");
        assert_eq!(seen.last, 46);
    }

    #[test]
    fn a_change_after_the_brightness_was_still_is_shown_at_once() {
        let t = Instant::now();
        let mut seen = Seen::new(50, t);
        assert!(seen.look(40, t + ms(100)));
        assert!(seen.look(70, t + ms(100) + BRIGHTNESS_STILL + ms(1)), "the opposite way, but a key press after a pause");
    }

    #[test]
    fn brightness_is_read_slowly_until_it_changes() {
        assert_eq!(brightness_interval(Duration::ZERO), BRIGHTNESS_IDLE);
        assert_eq!(brightness_interval(Duration::from_secs(1)), BRIGHTNESS_QUICK);
        assert!(BRIGHTNESS_IDLE >= Duration::from_millis(250), "far fewer wake-ups than the old 100 ms");
    }
}
