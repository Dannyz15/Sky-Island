//! System watchers (volume, audio device, brightness, lock keys, battery, network).
//! Each pushes a `Notice` to the UI only when something actually changes.

#[cfg(windows)]
use std::sync::Mutex;
#[cfg(any(windows, target_os = "linux"))]
use std::sync::OnceLock;
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(any(windows, target_os = "linux"))]
use std::sync::mpsc::{self, Sender};
#[cfg(any(windows, target_os = "linux"))]
use std::thread;
#[cfg(any(windows, target_os = "linux", test))]
use std::time::Duration;
#[cfg(any(windows, test))]
use std::time::Instant;

#[cfg(windows)]
use windows::Networking::Connectivity::{NetworkInformation, NetworkStatusChangedEventHandler};
#[cfg(windows)]
use windows::Win32::Devices::Display::{DISPLAY_BRIGHTNESS, IOCTL_VIDEO_QUERY_DISPLAY_BRIGHTNESS};
#[cfg(windows)]
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
#[cfg(windows)]
use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl};
#[cfg(windows)]
use windows::Win32::Media::Audio::{
    AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE, EDataFlow, ERole, IMMDevice, IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl, MMDeviceEnumerator,
    PKEY_AudioEndpoint_FormFactor, eConsole, eRender,
};
#[cfg(windows)]
use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, GetDriveTypeW, OPEN_EXISTING};
#[cfg(windows)]
use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, STGM_READ};
#[cfg(windows)]
use windows::Win32::System::IO::DeviceIoControl;
#[cfg(windows)]
use windows::Win32::System::Power::{GetSystemPowerStatus, RegisterPowerSettingNotification, SYSTEM_POWER_STATUS};
#[cfg(windows)]
use windows::Win32::System::SystemServices::{GUID_ACDC_POWER_SOURCE, GUID_BATTERY_PERCENTAGE_REMAINING, GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS};
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CAPITAL, VK_NUMLOCK};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DEVICE_NOTIFY_WINDOW_HANDLE, DefWindowProcW, DispatchMessageW, GetMessageW, KBDLLHOOKSTRUCT, MSG, PBT_POWERSETTINGCHANGE, SetTimer, SetWindowsHookExW,
    TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_POWERBROADCAST, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_TIMER,
};
#[cfg(windows)]
use windows::core::{PCWSTR, Result, implement, w};
use winit::event_loop::EventLoopProxy;

use crate::UserEvent;
#[cfg(windows)]
use crate::msgwin;

/// Something worth showing briefly in the notch.
#[derive(Clone)]
pub enum Notice {
    Volume {
        percent: u32,
        muted: bool,
    },
    Brightness {
        percent: u32,
    },
    Lock {
        name: &'static str,
        on: bool,
    },
    Battery {
        percent: u32,
        kind: BatteryKind,
    },
    Network {
        connected: bool,
        name: String,
        wifi: bool,
    },
    Audio {
        name: String,
        headphones: bool,
    },
    /// A newer version of Sky Island exists.
    Update {
        version: String,
    },
    /// A prayer time has come: its name and the minute of the day.
    Prayer {
        name: &'static str,
        minute: u32,
    },
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum BatteryKind {
    Plugged,
    Unplugged,
    Low,
}

type Proxy = EventLoopProxy<UserEvent>;

#[cfg(any(windows, target_os = "linux"))]
fn send(proxy: &Proxy, notice: Notice) {
    let _ = proxy.send_event(UserEvent::Notice(notice));
}

/// Start every watcher except the keyboard hook, which has its own start (`install_lock_hook`).
pub fn spawn_all(proxy: Proxy) {
    #[cfg(windows)]
    {
        let p = proxy.clone();
        thread::spawn(move || audio(p));
        let p = proxy.clone();
        thread::spawn(move || battery(p));
        let p = proxy.clone();
        thread::spawn(move || brightness(p));
        thread::spawn(move || network(proxy));
    }
    #[cfg(target_os = "linux")]
    {
        thread::spawn(move || linux::audio(proxy));
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    let _ = proxy;
}

// ---- Audio: volume, mute and default output device ----

#[cfg(windows)]
enum AudioMsg {
    DefaultChanged,
    /// The user moved the volume slider in the notch: 0.0 to 1.0.
    SetVolume(f32),
    ToggleMute,
}

/// How the notch's volume slider reaches the audio thread.
#[cfg(windows)]
static AUDIO_TX: OnceLock<Sender<AudioMsg>> = OnceLock::new();

/// Set the volume of the default output device (0 to 100 percent). Any volume above zero also unmutes it.
pub fn set_volume(percent: u32) {
    #[cfg(windows)]
    if let Some(tx) = AUDIO_TX.get() {
        let _ = tx.send(AudioMsg::SetVolume(percent.min(100) as f32 / 100.0));
    }
    #[cfg(target_os = "linux")]
    linux::set_volume(percent);
    #[cfg(not(any(windows, target_os = "linux")))]
    let _ = percent;
}

/// Mute the default output device, or unmute it.
pub fn toggle_mute() {
    #[cfg(windows)]
    if let Some(tx) = AUDIO_TX.get() {
        let _ = tx.send(AudioMsg::ToggleMute);
    }
    #[cfg(target_os = "linux")]
    linux::toggle_mute();
}

#[cfg(windows)]
#[implement(IAudioEndpointVolumeCallback)]
struct VolumeWatch {
    proxy: Mutex<Proxy>,
}

#[cfg(windows)]
impl IAudioEndpointVolumeCallback_Impl for VolumeWatch_Impl {
    fn OnNotify(&self, pnotify: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> Result<()> {
        if let Some(data) = unsafe { pnotify.as_ref() } {
            let notice = Notice::Volume { percent: (data.fMasterVolume * 100.0).round() as u32, muted: data.bMuted.as_bool() };
            if let Ok(p) = self.proxy.lock() {
                send(&p, notice);
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
#[implement(IMMNotificationClient)]
struct DeviceWatch {
    tx: Mutex<Sender<AudioMsg>>,
}

#[cfg(windows)]
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
        if flow == eRender
            && role == eConsole
            && let Ok(tx) = self.tx.lock()
        {
            let _ = tx.send(AudioMsg::DefaultChanged);
        }
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &windows::Win32::Foundation::PROPERTYKEY) -> Result<()> {
        Ok(())
    }
}

/// An output device's friendly name and whether it is headphones or a headset.
#[cfg(windows)]
fn describe(device: &IMMDevice) -> Option<(String, bool)> {
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ).ok()?;
        let name = store.GetValue(&PKEY_Device_FriendlyName).ok()?.to_string();
        // EndpointFormFactor: 3 = Headphones, 5 = Headset.
        let form = store.GetValue(&PKEY_AudioEndpoint_FormFactor).ok().and_then(|v| u32::try_from(&v).ok()).unwrap_or(0);
        Some((name, matches!(form, 3 | 5)))
    }
}

#[cfg(windows)]
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

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::io::{self, BufRead, BufReader};
    use std::process::{Command, Stdio};

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(super) struct SinkInfo {
        pub(super) name: String,
        pub(super) description: String,
        pub(super) active_port: String,
        pub(super) percent: u32,
        pub(super) muted: bool,
        pub(super) headphones: bool,
    }

    pub(super) fn is_headphones(form_factor: &str, icon_name: &str, active_port: &str, description: &str) -> bool {
        const NEEDLES: [&str; 5] = ["headphone", "headset", "hands-free", "earphone", "airpods"];
        [form_factor, icon_name, active_port, description].into_iter().any(|field| {
            let lower = field.to_ascii_lowercase();
            NEEDLES.iter().any(|needle| lower.contains(needle))
        })
    }

    fn collect_percents(fragment: &str, sum: &mut u64, count: &mut u64) {
        for word in fragment.split_whitespace() {
            let cleaned = word.trim_matches(|c: char| matches!(c, ',' | '/' | ';' | '(' | ')'));
            if let Some(num) = cleaned.strip_suffix('%')
                && !num.is_empty()
                && num.bytes().all(|b| b.is_ascii_digit())
                && let Ok(val) = num.parse::<u64>()
            {
                *sum += val;
                *count += 1;
            }
        }
    }

    pub(super) fn parse_sinks(text: &str) -> Vec<SinkInfo> {
        #[derive(Default)]
        struct Builder {
            started: bool,
            name: String,
            description: String,
            active_port: String,
            form_factor: String,
            icon_name: String,
            muted: bool,
            vol_sum: u64,
            vol_count: u64,
            in_volume: bool,
        }

        impl Builder {
            fn finish(self, out: &mut Vec<SinkInfo>) {
                if !self.started || self.name.is_empty() {
                    return;
                }
                let percent = (self.vol_sum + self.vol_count / 2).checked_div(self.vol_count).unwrap_or(0).min(100) as u32;
                let headphones = is_headphones(&self.form_factor, &self.icon_name, &self.active_port, &self.description);
                out.push(SinkInfo { name: self.name, description: self.description, active_port: self.active_port, percent, muted: self.muted, headphones });
            }
        }

        let mut sinks = Vec::new();
        let mut cur = Builder::default();

        for raw in text.lines() {
            let line = raw.trim_end_matches('\r');
            if line.starts_with("Sink #") {
                std::mem::take(&mut cur).finish(&mut sinks);
                cur.started = true;
                continue;
            }
            if !cur.started {
                continue;
            }
            if cur.in_volume {
                if line.starts_with("\t ") || line.starts_with("  ") {
                    collect_percents(line, &mut cur.vol_sum, &mut cur.vol_count);
                    continue;
                }
                cur.in_volume = false;
            }
            if let Some(field) = line.strip_prefix('\t').filter(|s| !s.starts_with(['\t', ' '])) {
                if let Some(val) = field.strip_prefix("Name:") {
                    cur.name = val.trim().to_string();
                } else if let Some(val) = field.strip_prefix("Description:") {
                    cur.description = val.trim().to_string();
                } else if let Some(val) = field.strip_prefix("Mute:") {
                    cur.muted = val.trim().eq_ignore_ascii_case("yes");
                } else if let Some(val) = field.strip_prefix("Volume:") {
                    cur.in_volume = true;
                    cur.vol_sum = 0;
                    cur.vol_count = 0;
                    collect_percents(val, &mut cur.vol_sum, &mut cur.vol_count);
                } else if let Some(val) = field.strip_prefix("Active Port:") {
                    cur.active_port = val.trim().to_string();
                }
            } else {
                let trimmed = line.trim();
                if let Some(val) = trimmed.strip_prefix("device.form_factor = ") {
                    cur.form_factor = val.trim().trim_matches('"').to_string();
                } else if let Some(val) = trimmed.strip_prefix("device.icon_name = ") {
                    cur.icon_name = val.trim().trim_matches('"').to_string();
                }
            }
        }
        cur.finish(&mut sinks);
        sinks
    }

    pub(super) fn is_relevant_subscribe_line(line: &str) -> bool {
        line.contains(" on sink #") || line.contains(" on server #")
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(super) enum AudioDelta {
        Initial { percent: u32, muted: bool },
        OutputChanged { name: String, headphones: bool, percent: u32, muted: bool },
        VolumeChanged { percent: u32, muted: bool },
        Unchanged,
    }

    #[derive(Default)]
    pub(super) struct AudioTracker {
        last_sink: Option<String>,
        last_port: Option<String>,
        last_desc: Option<String>,
        last_vol: Option<(u32, bool)>,
    }

    impl AudioTracker {
        pub(super) fn observe(&mut self, sink: &SinkInfo) -> AudioDelta {
            if self.last_sink.is_none() {
                self.last_sink = Some(sink.name.clone());
                self.last_port = Some(sink.active_port.clone());
                self.last_desc = Some(sink.description.clone());
                self.last_vol = Some((sink.percent, sink.muted));
                return AudioDelta::Initial { percent: sink.percent, muted: sink.muted };
            }

            let sink_changed = self.last_sink.as_deref() != Some(sink.name.as_str());
            let port_changed = self.last_port.as_deref() != Some(sink.active_port.as_str());
            let desc_changed = self.last_desc.as_deref() != Some(sink.description.as_str());

            if sink_changed || port_changed {
                let fallback = || if !sink.description.is_empty() { sink.description.clone() } else { sink.name.clone() };
                let name = if sink_changed || desc_changed {
                    fallback()
                } else {
                    let port = sink.active_port.strip_prefix("[Out] ").unwrap_or(&sink.active_port).trim();
                    if !port.is_empty() { port.to_string() } else { fallback() }
                };
                self.last_sink = Some(sink.name.clone());
                self.last_port = Some(sink.active_port.clone());
                self.last_desc = Some(sink.description.clone());
                self.last_vol = Some((sink.percent, sink.muted));
                return AudioDelta::OutputChanged { name, headphones: sink.headphones, percent: sink.percent, muted: sink.muted };
            }

            if self.last_vol != Some((sink.percent, sink.muted)) {
                self.last_vol = Some((sink.percent, sink.muted));
                return AudioDelta::VolumeChanged { percent: sink.percent, muted: sink.muted };
            }

            AudioDelta::Unchanged
        }
    }

    enum Cmd {
        SetVolume(u32),
        ToggleMute,
    }

    static AUDIO_TX: OnceLock<Sender<Cmd>> = OnceLock::new();

    pub(super) fn set_volume(percent: u32) {
        if let Some(tx) = AUDIO_TX.get() {
            let _ = tx.send(Cmd::SetVolume(percent.min(100)));
        }
    }

    pub(super) fn toggle_mute() {
        if let Some(tx) = AUDIO_TX.get() {
            let _ = tx.send(Cmd::ToggleMute);
        }
    }

    fn apply_volume(percent: u32) {
        let arg = format!("{}%", percent.min(100));
        let _ =
            Command::new("pactl").args(["set-sink-volume", "@DEFAULT_SINK@", &arg]).env("LC_ALL", "C").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
        if percent > 0 {
            let _ =
                Command::new("pactl").args(["set-sink-mute", "@DEFAULT_SINK@", "0"]).env("LC_ALL", "C").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
        }
    }

    fn apply_toggle_mute() {
        let _ =
            Command::new("pactl").args(["set-sink-mute", "@DEFAULT_SINK@", "toggle"]).env("LC_ALL", "C").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }

    fn query_default_sink() -> Option<SinkInfo> {
        let def = Command::new("pactl").arg("get-default-sink").env("LC_ALL", "C").stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        if !def.status.success() {
            return None;
        }
        let default_name = String::from_utf8_lossy(&def.stdout).trim().to_string();
        if default_name.is_empty() {
            return None;
        }
        let list = Command::new("pactl").args(["list", "sinks"]).env("LC_ALL", "C").stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        if !list.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&list.stdout);
        parse_sinks(&text).into_iter().find(|s| s.name == default_name)
    }

    fn dispatch(proxy: &Proxy, tracker: &mut AudioTracker, sink: &SinkInfo) {
        match tracker.observe(sink) {
            AudioDelta::Initial { percent, muted } => {
                let _ = proxy.send_event(UserEvent::SysVolume { percent, muted });
            }
            AudioDelta::OutputChanged { name, headphones, percent, muted } => {
                send(proxy, Notice::Audio { name, headphones });
                let _ = proxy.send_event(UserEvent::SysVolume { percent, muted });
            }
            AudioDelta::VolumeChanged { percent, muted } => {
                send(proxy, Notice::Volume { percent, muted });
            }
            AudioDelta::Unchanged => {}
        }
    }

    pub(super) fn audio(proxy: Proxy) {
        let (tx, rx) = mpsc::channel();
        if AUDIO_TX.set(tx).is_err() {
            return;
        }
        let _ = thread::Builder::new().name("audio-ctl".into()).spawn(move || {
            while let Ok(first) = rx.recv() {
                let mut pending = vec![first];
                pending.extend(rx.try_iter());
                let mut i = 0;
                while i < pending.len() {
                    match pending[i] {
                        Cmd::SetVolume(mut p) => {
                            while i + 1 < pending.len() {
                                if let Cmd::SetVolume(next) = pending[i + 1] {
                                    p = next;
                                    i += 1;
                                } else {
                                    break;
                                }
                            }
                            apply_volume(p);
                        }
                        Cmd::ToggleMute => apply_toggle_mute(),
                    }
                    i += 1;
                }
            }
        });

        let mut tracker = AudioTracker::default();
        if let Some(sink) = query_default_sink() {
            dispatch(&proxy, &mut tracker, &sink);
        }

        loop {
            let mut cmd = Command::new("pactl");
            cmd.arg("subscribe").env("LC_ALL", "C").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
            kill_on_parent_exit(&mut cmd);
            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return,
                Err(_) => {
                    thread::sleep(Duration::from_secs(2));
                    continue;
                }
            };
            if let Some(stdout) = child.stdout.take() {
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    if is_relevant_subscribe_line(&line)
                        && let Some(sink) = query_default_sink()
                    {
                        dispatch(&proxy, &mut tracker, &sink);
                    }
                }
            }
            let _ = child.wait();
            thread::sleep(Duration::from_secs(2));
        }
    }

    /// Ask the Linux kernel to send `SIGTERM` to a long-lived child process if its parent thread/process exits.
    pub(crate) fn kill_on_parent_exit(cmd: &mut Command) {
        use std::os::unix::process::CommandExt;

        const PR_SET_PDEATHSIG: i32 = 1;
        const SIGTERM: usize = 15;

        unsafe extern "C" {
            fn prctl(option: i32, ...) -> i32;
            fn getppid() -> i32;
        }

        let parent_pid = std::process::id() as i32;
        // SAFETY: `pre_exec` runs between `fork` and `execve`; `prctl(PR_SET_PDEATHSIG, ...)` and `getppid` are
        // async-signal-safe syscalls, and `parent_pid` is copied as a plain `i32` before `fork`.
        unsafe {
            cmd.pre_exec(move || {
                prctl(PR_SET_PDEATHSIG, SIGTERM, 0usize, 0usize, 0usize);
                if getppid() != parent_pid {
                    return Err(io::Error::new(io::ErrorKind::BrokenPipe, "parent exited"));
                }
                Ok(())
            });
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) use linux::kill_on_parent_exit;

// ---- Battery ----

/// How often the battery is looked at if Windows cannot tell the app when its state changes.
#[cfg(windows)]
const BATTERY_POLL: Duration = Duration::from_secs(3);

/// What the battery looked like at the last look, to tell what changed.
#[cfg(any(windows, test))]
struct BatteryWatch {
    ac: Option<u8>,
    percent: Option<u32>,
}

#[cfg(any(windows, test))]
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

#[cfg(windows)]
static BATTERY: Mutex<BatteryWatch> = Mutex::new(BatteryWatch { ac: None, percent: None });
#[cfg(windows)]
static BATTERY_PROXY: OnceLock<Proxy> = OnceLock::new();

/// Whether this PC has a battery to watch.
#[cfg(windows)]
fn has_battery() -> bool {
    let mut st = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut st) }.is_ok() && st.BatteryFlag & 128 == 0
}

/// Look at the power status now and tell the UI what changed since the last look.
#[cfg(windows)]
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
#[cfg(windows)]
fn battery(proxy: Proxy) {
    if !has_battery() || BATTERY_PROXY.set(proxy).is_err() {
        return; // no battery (desktop), nothing to watch
    }
    look_at_battery(); // the starting point; nothing is announced for it
    // SAFETY: the class name is a `w!` literal, which lives for the whole run.
    let listening = unsafe {
        msgwin::spawn(w!("SkyIslandPower"), Some(power_proc), |window| {
            let target = HANDLE(window.0);
            let source = RegisterPowerSettingNotification(target, &GUID_ACDC_POWER_SOURCE, DEVICE_NOTIFY_WINDOW_HANDLE);
            let charge = RegisterPowerSettingNotification(target, &GUID_BATTERY_PERCENTAGE_REMAINING, DEVICE_NOTIFY_WINDOW_HANDLE);
            source.is_ok() && charge.is_ok()
        })
    };
    if !listening {
        loop {
            thread::sleep(BATTERY_POLL);
            look_at_battery();
        }
    }
}

#[cfg(windows)]
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
#[cfg(any(windows, test))]
const BRIGHTNESS_IDLE: Duration = Duration::from_millis(300);
#[cfg(any(windows, test))]
const BRIGHTNESS_QUICK: Duration = Duration::from_millis(50);
#[cfg(windows)]
const BRIGHTNESS_QUICK_FOR: Duration = Duration::from_secs(3);
/// While a slider is dragged, Windows can report the previous step again for a moment (up to about 0.2 s once
/// the drag ends). A reading that goes against the way the brightness was just moving is only believed after it
/// has stayed this long. A change after the brightness has been still for `BRIGHTNESS_STILL` is believed at once.
#[cfg(any(windows, test))]
const BRIGHTNESS_TURN_HOLD: Duration = Duration::from_millis(250);
#[cfg(any(windows, test))]
const BRIGHTNESS_STILL: Duration = Duration::from_secs(1);

/// How long to wait before reading the brightness again, given how much of the "quick" period is left.
#[cfg(any(windows, test))]
fn brightness_interval(quick_left: Duration) -> Duration {
    if quick_left.is_zero() { BRIGHTNESS_IDLE } else { BRIGHTNESS_QUICK }
}

/// What the brightness has been doing, to tell a real change from a step-old reading.
#[cfg(any(windows, test))]
struct Seen {
    last: u32,
    /// 1 when the last change went up, -1 when it went down, 0 before any.
    heading: i8,
    moved: Instant,
    /// A reading that turned back, and since when it has been seen.
    held: Option<(u32, Instant)>,
}

#[cfg(any(windows, test))]
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
#[cfg(windows)]
struct Panel {
    lcd: usize,
    seen: Seen,
}

#[cfg(windows)]
static PANEL: Mutex<Option<Panel>> = Mutex::new(None);
#[cfg(windows)]
static BRIGHTNESS_PROXY: OnceLock<Proxy> = OnceLock::new();
/// The thread that reads the brightness; Windows' notifications wake it.
#[cfg(windows)]
static BRIGHTNESS_READER: OnceLock<thread::Thread> = OnceLock::new();

/// The panel's brightness in percent for the power source in use, or `None` if the driver does not answer.
#[cfg(windows)]
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
#[cfg(windows)]
fn look_at_brightness() -> Option<bool> {
    let mut guard = PANEL.lock().ok()?;
    let panel = guard.as_mut()?;
    let now = read_brightness(HANDLE(panel.lcd as *mut _))?;
    let announce = panel.seen.look(now, Instant::now());
    if announce && let Some(proxy) = BRIGHTNESS_PROXY.get() {
        send(proxy, Notice::Brightness { percent: now });
    }
    Some(announce || panel.seen.held.is_some())
}

/// Windows says when the panel's brightness changes (to a hidden window), so nothing runs while it stays
/// the same. The notification is only a wake-up: Windows sends it a few times a second while a slider is
/// dragged, with a value that can be a step old, so the brightness is read directly, every 100 ms until it
/// has been still for a few seconds. The power source is watched too, because the brightness can differ
/// between the charger and the battery. If that cannot be set up, the brightness is read on a timer instead.
#[cfg(windows)]
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
    // SAFETY: the class name is a `w!` literal, which lives for the whole run.
    let listening = unsafe {
        msgwin::spawn(w!("SkyIslandBrightness"), Some(brightness_proc), |window| {
            let target = HANDLE(window.0);
            let level = RegisterPowerSettingNotification(target, &GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS, DEVICE_NOTIFY_WINDOW_HANDLE);
            let source = RegisterPowerSettingNotification(target, &GUID_ACDC_POWER_SOURCE, DEVICE_NOTIFY_WINDOW_HANDLE);
            level.is_ok() && source.is_ok()
        })
    };
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

#[cfg(windows)]
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

#[cfg(windows)]
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
            && (prev.0, &prev.1) != (now.0, &now.1)
        {
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

#[cfg(windows)]
static PROXY: OnceLock<Proxy> = OnceLock::new();

/// How often the keyboard hook is put in again, in milliseconds.
#[cfg(windows)]
const HOOK_RENEWED_EVERY_MS: u32 = 60_000;

/// Install the low-level keyboard hook. Windows calls the hook on the thread that installed it and waits for
/// the answer before the key goes on to any app, and drops a hook that is too slow: so it gets a thread of its
/// own that does nothing else, and a busy notch cannot make typing wait.
/// The hook only says a lock key was pressed. The real on/off state is read a moment later on
/// the UI thread, because at hook time the key hasn't taken effect yet, and counting presses
/// drifts out of sync if a key-up is ever missed.
pub fn install_lock_hook(proxy: Proxy) {
    #[cfg(windows)]
    {
        if PROXY.set(proxy).is_err() {
            return;
        }
        thread::spawn(|| unsafe {
            let mut hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), None, 0).ok();
            // Windows takes away a hook that once answered too slowly, and says nothing about it: the lock notices would
            // stop for good. A timer on this thread puts the hook in again now and then.
            let _ = SetTimer(None, 0, HOOK_RENEWED_EVERY_MS, None);
            // A hook needs its thread to keep taking messages.
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                if msg.message == WM_TIMER {
                    // The new hook is in before the old one goes, so no key falls between the two (a key seen by both
                    // is one press: see `LOCK_HELD`).
                    if let Ok(new) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), None, 0)
                        && let Some(old) = hook.replace(new)
                    {
                        let _ = UnhookWindowsHookEx(old);
                    }
                    continue;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
    }
    #[cfg(not(windows))]
    let _ = proxy;
}

/// Whether a toggle key (Caps Lock, Num Lock) is currently on.
pub fn key_toggled(vk: i32) -> bool {
    #[cfg(windows)]
    unsafe {
        GetKeyState(vk) & 1 != 0
    }
    #[cfg(not(windows))]
    {
        let _ = vk;
        false
    }
}

/// Whether Caps Lock and Num Lock are held down at the moment (set on the key going down, cleared on the key coming
/// up). A key that is held sends its down message again and again; only the first one is a press.
#[cfg(windows)]
static LOCK_HELD: [AtomicBool; 2] = [AtomicBool::new(false), AtomicBool::new(false)];

#[cfg(windows)]
unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let message = wparam.0 as u32;
    if code >= 0 && matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP) {
        let key = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let lock = match key.vkCode {
            v if v == VK_CAPITAL.0 as u32 => Some(("Caps Lock", VK_CAPITAL.0 as i32, 0)),
            v if v == VK_NUMLOCK.0 as u32 => Some(("Num Lock", VK_NUMLOCK.0 as i32, 1)),
            _ => None,
        };
        let down = matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN);
        // The first message of a press only; a repeat of a held key, and the key coming up, are not a press.
        let pressed = lock.is_some_and(|(_, _, slot)| LOCK_HELD[slot].swap(down, Ordering::Relaxed) != down) && down;
        if let (true, Some((name, vk, _)), Some(proxy)) = (pressed, lock, PROXY.get()) {
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

/// What kind of drive a path with a drive letter ("C:\...") is on, as `GetDriveTypeW` says it: 2 removable, 3 fixed
/// (hard disk or SSD), 4 network, 5 CD, 6 RAM disk; 0 and 1 mean it cannot be told or nothing is there. `None` for a
/// path with no drive letter (a share such as `\\server\share`, or a relative path). Answered by this PC, so it
/// never waits for a server.
pub fn drive_type(path: &std::path::Path) -> Option<u32> {
    #[cfg(windows)]
    {
        let text = path.as_os_str().to_string_lossy();
        let bytes = text.as_bytes();
        if bytes.len() < 2 || bytes[1] != b':' || !bytes[0].is_ascii_alphabetic() {
            return None;
        }
        let root: Vec<u16> = format!(r"{}:\", bytes[0] as char).encode_utf16().chain([0]).collect();
        // SAFETY: a NUL-terminated root such as "C:\"; the call only reads it.
        Some(unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) })
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        None
    }
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

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_sinks_reads_multiple_sinks_and_surround_volumes() {
        let sample = "\
Sink #10
\tState: RUNNING
\tName: alsa_output.pci.speaker
\tDescription: Built-in Speaker
\tMute: no
\tVolume: front-left: 55706 /  85% / -4.23 dB,   front-right: 55706 /  85% / -4.23 dB
\t        balance 0.00
\tBase Volume: 65536 / 100% / 0.00 dB
\tProperties:
\t\tdevice.icon_name = \"audio-speakers\"
\tActive Port: [Out] Speaker
Sink #11
\tState: IDLE
\tName: bluez_output.headphones
\tDescription: Sony WH-1000XM5
\tMute: yes
\tVolume: front-left: 72090 / 110% / 2.5 dB,   front-right: 72090 / 110% / 2.5 dB,
\t        rear-left: 58982 /  90% / -2.75 dB,   rear-right: 58982 /  90% / -2.75 dB
\tBase Volume: 65536 / 100% / 0.00 dB
\tProperties:
\t\tdevice.form_factor = \"headset\"
\t\tdevice.icon_name = \"audio-headset-bluetooth\"
\tActive Port: headset-output
";
        let sinks = linux::parse_sinks(sample);
        assert_eq!(sinks.len(), 2);
        assert_eq!(sinks[0].name, "alsa_output.pci.speaker");
        assert_eq!(sinks[0].description, "Built-in Speaker");
        assert_eq!(sinks[0].percent, 85);
        assert!(!sinks[0].muted);
        assert!(!sinks[0].headphones);
        assert_eq!(sinks[0].active_port, "[Out] Speaker");

        assert_eq!(sinks[1].name, "bluez_output.headphones");
        assert_eq!(sinks[1].percent, 100, "average clamped to 100, Base Volume ignored");
        assert!(sinks[1].muted);
        assert!(sinks[1].headphones);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn headphones_detection_and_subscribe_filter() {
        assert!(linux::is_headphones("headphone", "", "", ""));
        assert!(linux::is_headphones("", "audio-headset", "", ""));
        assert!(linux::is_headphones("", "", "[Out] Headphones", ""));
        assert!(linux::is_headphones("", "", "", "AirPods Pro Hands-Free"));
        assert!(!linux::is_headphones("speaker", "audio-speakers", "[Out] Speaker", "Built-in Audio"));

        assert!(linux::is_relevant_subscribe_line("Event 'change' on sink #26911"));
        assert!(linux::is_relevant_subscribe_line("Event 'change' on server #0"));
        assert!(!linux::is_relevant_subscribe_line("Event 'change' on client #30856"));
        assert!(!linux::is_relevant_subscribe_line("Event 'new' on sink-input #42"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn audio_tracker_emits_initial_volume_and_output_deltas() {
        let mut tracker = linux::AudioTracker::default();
        let speaker = linux::SinkInfo {
            name: "sink.speaker".into(),
            description: "Built-in Audio".into(),
            active_port: "[Out] Speaker".into(),
            percent: 50,
            muted: false,
            headphones: false,
        };
        assert_eq!(tracker.observe(&speaker), linux::AudioDelta::Initial { percent: 50, muted: false });
        assert_eq!(tracker.observe(&speaker), linux::AudioDelta::Unchanged);

        let louder = linux::SinkInfo { percent: 65, ..speaker.clone() };
        assert_eq!(tracker.observe(&louder), linux::AudioDelta::VolumeChanged { percent: 65, muted: false });

        let jack = linux::SinkInfo { active_port: "[Out] Headphones".into(), headphones: true, percent: 35, ..louder.clone() };
        assert_eq!(tracker.observe(&jack), linux::AudioDelta::OutputChanged { name: "Headphones".into(), headphones: true, percent: 35, muted: false });

        let usb = linux::SinkInfo { name: "sink.usb".into(), description: "USB DAC".into(), active_port: "analog-output".into(), percent: 80, muted: true, headphones: false };
        assert_eq!(tracker.observe(&usb), linux::AudioDelta::OutputChanged { name: "USB DAC".into(), headphones: false, percent: 80, muted: true });
    }
}
