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
use windows::Win32::System::SystemServices::{GUID_ACDC_POWER_SOURCE, GUID_BATTERY_PERCENTAGE_REMAINING};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CAPITAL, VK_NUMLOCK};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DEVICE_NOTIFY_WINDOW_HANDLE, DefWindowProcW, KBDLLHOOKSTRUCT, PBT_POWERSETTINGCHANGE, SetWindowsHookExW, WH_KEYBOARD_LL, WM_KEYDOWN,
    WM_POWERBROADCAST, WM_SYSKEYDOWN,
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

/// Start every watcher. The keyboard hook must be installed from a thread that pumps messages,
/// which the UI thread does, so it is separate.
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

        attach(&mut attached);
        while let Ok(AudioMsg::DefaultChanged) = rx.recv() {
            if let Some((name, headphones)) = attach(&mut attached).as_ref().and_then(describe) {
                send(&proxy, Notice::Audio { name, headphones });
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

const BRIGHTNESS_IDLE: Duration = Duration::from_millis(300);
const BRIGHTNESS_QUICK: Duration = Duration::from_millis(100);
const BRIGHTNESS_QUICK_FOR: Duration = Duration::from_secs(3);

/// How long to wait before reading the brightness again, given how much of the "quick" period is left.
fn brightness_interval(quick_left: Duration) -> Duration {
    if quick_left.is_zero() { BRIGHTNESS_IDLE } else { BRIGHTNESS_QUICK }
}

fn brightness(proxy: Proxy) {
    unsafe {
        let Ok(lcd) = CreateFileW(w!("\\\\.\\LCD"), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, Default::default(), None) else {
            return; // no built-in panel, e.g. a desktop
        };
        let read = |lcd: HANDLE| -> Option<u32> {
            let mut out = DISPLAY_BRIGHTNESS::default();
            let mut returned = 0u32;
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
        };

        // If the driver doesn't answer the query, give up rather than polling for nothing.
        let Some(mut last) = read(lcd) else {
            let _ = CloseHandle(lcd);
            return;
        };
        // Slow while nothing happens, quick for a few seconds after a change so a dragged slider is followed.
        let mut quick_until = Instant::now();
        loop {
            thread::sleep(brightness_interval(quick_until.saturating_duration_since(Instant::now())));
            match read(lcd) {
                Some(now) if now != last => {
                    last = now;
                    quick_until = Instant::now() + BRIGHTNESS_QUICK_FOR;
                    send(&proxy, Notice::Brightness { percent: now });
                }
                Some(_) => {}
                None => break,
            }
        }
        let _ = CloseHandle(lcd);
    }
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

/// Install the low-level keyboard hook on the calling thread, which must keep pumping messages.
/// The hook only says a lock key was pressed. The real on/off state is read a moment later on
/// the UI thread, because at hook time the key hasn't taken effect yet, and counting presses
/// drifts out of sync if a key-up is ever missed.
pub fn install_lock_hook(proxy: Proxy) {
    if PROXY.set(proxy).is_err() {
        return;
    }
    unsafe {
        let _ = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), None, 0);
    }
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

    #[test]
    fn brightness_is_read_slowly_until_it_changes() {
        assert_eq!(brightness_interval(Duration::ZERO), BRIGHTNESS_IDLE);
        assert_eq!(brightness_interval(Duration::from_secs(1)), BRIGHTNESS_QUICK);
        assert!(BRIGHTNESS_IDLE >= Duration::from_millis(250), "far fewer wake-ups than the old 100 ms");
    }
}
