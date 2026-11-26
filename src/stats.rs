//! System stats for the Stats tab: how busy the processor and the graphics chip are, how much memory and disk is
//! used, and how fast data moves over the network. Looked at only while the tab is open (once a second), with a few
//! cheap Windows calls; nothing runs while it is closed.

#[cfg(any(windows, test))]
use std::collections::HashMap;
use std::collections::VecDeque;
#[cfg(any(windows, target_os = "linux"))]
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, channel};
#[cfg(any(windows, target_os = "linux"))]
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

#[cfg(windows)]
use windows::Win32::Foundation::FILETIME;
#[cfg(windows)]
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
#[cfg(windows)]
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
#[cfg(windows)]
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives};
#[cfg(windows)]
use windows::Win32::System::Diagnostics::Debug::{SEM_FAILCRITICALERRORS, SetThreadErrorMode};
#[cfg(windows)]
use windows::Win32::System::Performance::{
    PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA, PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhOpenQueryW,
};
#[cfg(windows)]
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
#[cfg(windows)]
use windows::Win32::System::Threading::GetSystemTimes;
#[cfg(windows)]
use windows::core::PCWSTR;

/// How often the numbers are looked at while the tab is open.
pub const EVERY: Duration = Duration::from_secs(1);
/// The first look is quick, so the tab does not sit empty: the processor and the network need two looks to say anything.
pub const FIRST_AGAIN: Duration = Duration::from_millis(250);
/// How many seconds of the processor's history are kept for the graph.
pub const HISTORY: usize = 60;
/// The disks are looked at this often (they hardly change; a drive plugged in shows up within this time).
const DISK_EVERY: Duration = Duration::from_secs(15);
/// The network graph is never scaled to less than this many bytes a second, so a quiet link shows as quiet.
pub const QUIET_LINK: f32 = 64.0 * 1024.0;
/// The most drives that are listed, so the notch does not grow without end.
pub const MAX_DRIVES: usize = 6;

/// What `GetDriveTypeW` says for a hard disk or SSD and for a USB stick or card reader.
#[cfg(windows)]
const DRIVE_REMOVABLE: u32 = 2;
#[cfg(windows)]
const DRIVE_FIXED: u32 = 3;

/// One drive: its letter (or short mount label on Linux), and the space in use and in all, in bytes.
#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(not(target_os = "linux"), derive(Copy))]
pub struct Drive {
    pub letter: char,
    pub used: u64,
    pub total: u64,
    #[cfg(target_os = "linux")]
    pub label: String,
}

impl Drive {
    pub fn label(&self) -> std::borrow::Cow<'_, str> {
        #[cfg(target_os = "linux")]
        {
            std::borrow::Cow::Borrowed(&self.label)
        }
        #[cfg(not(target_os = "linux"))]
        {
            std::borrow::Cow::Owned(self.letter.to_string())
        }
    }

    pub fn same_id(&self, other: &Self) -> bool {
        #[cfg(target_os = "linux")]
        {
            self.label == other.label
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.letter == other.letter
        }
    }
}

/// What the last look found. `None` where it needs a second look or Windows would not say.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Sample {
    /// How busy the processor was since the look before: 0.0 to 1.0.
    pub cpu: Option<f32>,
    /// Memory in use and in all, in bytes.
    pub memory: Option<(u64, u64)>,
    /// The drives that are there (hard disks, SSDs and USB sticks with a disk in), in the order of their letters.
    pub disks: Vec<Drive>,
    /// Bytes a second received and sent, over the network adapters that are really there.
    pub network: Option<(f64, f64)>,
    /// The graphics chips, each with its name and how busy it was. Empty until they have been looked at, and on a PC
    /// where Windows has no counters for them.
    pub gpus: Vec<Chip>,
}

/// One graphics chip of this PC: its name, and how busy it is (0.0 to 1.0, `None` until a look has an answer).
#[derive(Clone, PartialEq, Debug)]
pub struct Chip {
    pub name: String,
    pub load: Option<f32>,
}

/// The most graphics chips that are listed, so the notch does not grow without end.
pub const MAX_CHIPS: usize = 3;

#[derive(Default)]
pub struct Stats {
    pub last: Sample,
    /// What the look before the last one found, which the view glides from to what the last one found.
    pub prev: Sample,
    /// The fastest the network was in the last minute (never below a quiet link), which the graph is scaled by, and
    /// the same before the last look.
    pub net_scale: f32,
    pub net_scale_prev: f32,
    /// The processor's load for each of the last seconds, oldest first.
    pub history: VecDeque<f32>,
    /// How busy the busiest graphics chip was for each of the last seconds, oldest first.
    pub gpu_history: VecDeque<f32>,
    /// The network's speed for each of the last seconds, oldest first: bytes a second received and sent.
    pub net_history: VecDeque<(f32, f32)>,
    /// Whether at least one look has been taken.
    pub started: bool,
    /// Whether the drives have been looked at yet (until then the view shows nothing for them rather than "not available").
    pub disks_known: bool,
    /// The look at the drives that is under way on its own thread: a disk that has spun down can take seconds to
    /// answer, and the notch must not wait for it.
    pending: Option<Receiver<Vec<Drive>>>,
    prev_cpu: Option<(u64, u64)>,
    prev_net: Option<(u64, u64, Instant)>,
    disk_at: Option<Instant>,
}

impl Stats {
    /// The processor's load from the counters of Windows: `idle` and `total` time so far. The first call only
    /// remembers them.
    pub fn feed_cpu(&mut self, idle: u64, total: u64) {
        if let Some((pi, pt)) = self.prev_cpu.replace((idle, total)) {
            let dt = total.saturating_sub(pt);
            if dt > 0 {
                let busy = 1.0 - (idle.saturating_sub(pi) as f32 / dt as f32);
                let busy = busy.clamp(0.0, 1.0);
                self.last.cpu = Some(busy);
                if self.history.len() == HISTORY {
                    self.history.pop_front();
                }
                self.history.push_back(busy);
            }
        }
    }

    /// The network's speed from the counters of the adapters: bytes received and sent so far. The first call only
    /// remembers them (as does one after the counters went backwards, when an adapter went away).
    pub fn feed_net(&mut self, received: u64, sent: u64, now: Instant) {
        if let Some((pr, ps, at)) = self.prev_net.replace((received, sent, now)) {
            let secs = now.saturating_duration_since(at).as_secs_f64();
            if secs > 0.05 && received >= pr && sent >= ps {
                let (down, up) = ((received - pr) as f64 / secs, (sent - ps) as f64 / secs);
                self.last.network = Some((down, up));
                if self.net_history.len() == HISTORY {
                    self.net_history.pop_front();
                }
                self.net_history.push_back((down as f32, up as f32));
            }
        }
    }

    /// Take a look at everything.
    pub fn sample(&mut self, now: Instant) {
        self.started = true;
        self.prev = self.last.clone();
        self.net_scale_prev = self.net_scale;
        if let Some((idle, total)) = system_times() {
            self.feed_cpu(idle, total);
        }
        self.last.memory = memory();
        if let Some(rx) = &self.pending
            && let Ok(found) = rx.try_recv()
        {
            self.last.disks = found;
            self.disks_known = true;
            self.pending = None;
        }
        if self.pending.is_none() && self.disk_at.is_none_or(|at| now.saturating_duration_since(at) >= DISK_EVERY) {
            self.disk_at = Some(now);
            let (tx, rx) = channel();
            std::thread::spawn(move || {
                let _ = tx.send(drives());
            });
            self.pending = Some(rx);
        }
        if let Some((received, sent)) = network_totals() {
            self.feed_net(received, sent, now);
        }
        if let Some(chips) = gpu_look() {
            self.last.gpus = chips;
        }
        if let Some(busiest) = self.last.gpus.iter().filter_map(|chip| chip.load).reduce(f32::max) {
            if self.gpu_history.len() == HISTORY {
                self.gpu_history.pop_front();
            }
            self.gpu_history.push_back(busiest);
        }
        self.net_scale = scale_of(&self.net_history);
        if self.net_scale_prev <= 0.0 {
            self.net_scale_prev = self.net_scale;
        }
    }

    /// Whether the numbers that need two looks are there.
    pub fn ready(&self) -> bool {
        self.last.cpu.is_some()
    }

    /// Whether the graphics chips have not shown their load yet: they are not named yet, or named without a load (the
    /// first reading is on its way). A PC with no load to show stays "pending", so the caller only waits for a while.
    pub fn gpu_pending(&self) -> bool {
        self.last.gpus.is_empty() || self.last.gpus.iter().any(|c| c.load.is_none())
    }
}

/// What the look at the graphics chips has found: nothing yet (`None`), or the chips (none at all when the OS has no
/// load to report for any chip of this PC).
#[cfg(any(windows, target_os = "linux"))]
type GpuReport = Arc<Mutex<Option<Vec<Chip>>>>;

/// How long the look at the graphics chips carries on after it was last asked for. The Stats view asks once a second.
#[cfg(any(windows, target_os = "linux", test))]
const GPU_GRACE_MS: u64 = 3_000;

/// The look at the graphics chips. Naming them and opening their counters takes a couple of seconds, and reading
/// them a few milliseconds, so it is done on a thread of its own that is started the first time the Stats view
/// asks and then stays: it sleeps (without waking up) while nobody asks, and starts reading within a second of
/// the next ask. It leaves the answer in `report`.
#[cfg(any(windows, target_os = "linux"))]
struct GpuWatch {
    report: GpuReport,
    /// When it was last asked, in milliseconds since the program began.
    asked: Arc<AtomicU64>,
    thread: std::thread::Thread,
}

#[cfg(windows)]
static GPU_WATCH: OnceLock<GpuWatch> = OnceLock::new();
#[cfg(any(windows, target_os = "linux"))]
static CLOCK: OnceLock<Instant> = OnceLock::new();

/// Milliseconds since the program began (the first time anyone asks).
#[cfg(any(windows, target_os = "linux"))]
fn now_ms() -> u64 {
    CLOCK.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Whether the look is still wanted: it was asked for in the last few seconds.
#[cfg(any(windows, target_os = "linux", test))]
fn gpu_wanted(asked_ms: u64, now_ms: u64) -> bool {
    now_ms.saturating_sub(asked_ms) <= GPU_GRACE_MS
}

/// Ask for the look at the graphics chips (which starts it, or wakes it) and say what it has found so far.
#[cfg(windows)]
fn gpu_look() -> Option<Vec<Chip>> {
    let watch = GPU_WATCH.get_or_init(|| {
        let (report, asked) = (GpuReport::default(), Arc::new(AtomicU64::new(0)));
        let (r, a) = (report.clone(), asked.clone());
        // The first ask is "now"; the thread starts reading at once.
        a.store(now_ms(), Ordering::Relaxed);
        let thread = std::thread::spawn(move || watch_gpus(&r, &a)).thread().clone();
        GpuWatch { report, asked, thread }
    });
    let now = now_ms();
    let was = watch.asked.swap(now, Ordering::Relaxed);
    if !gpu_wanted(was, now) {
        // Asked again after a long while: what it found back then is old; the new loads come in about a second.
        if let Ok(mut report) = watch.report.lock()
            && let Some(chips) = report.as_mut()
        {
            chips.iter_mut().for_each(|chip| chip.load = None);
        }
    }
    watch.thread.unpark();
    watch.report.lock().ok().and_then(|report| report.clone())
}

#[cfg(target_os = "linux")]
fn gpu_look() -> Option<Vec<Chip>> {
    linux::gpu_look()
}

#[cfg(not(any(windows, target_os = "linux")))]
fn gpu_look() -> Option<Vec<Chip>> {
    None
}

/// The identity Windows gives a graphics chip: the two halves of its LUID. In the counters it is written into the
/// name of each instance, such as `pid_4568_luid_0x00000000_0x0000D2A9_phys_0_eng_0_engtype_3D`.
#[cfg(any(windows, test))]
type Luid = (u32, u32);

/// The LUID in the name of an instance of the "GPU Engine" counter.
#[cfg(any(windows, test))]
fn luid_of(instance: &str) -> Option<Luid> {
    let (_, rest) = instance.split_once("luid_")?;
    let mut halves = rest.split('_').map(|h| u32::from_str_radix(h.trim_start_matches("0x"), 16).ok());
    Some((halves.next()??, halves.next()??))
}

/// How busy each chip is: its busiest engine, from the counter "GPU Engine, Utilization Percentage" (the one Task
/// Manager uses). `readings` are the instances of the counter, each with its name and its percentage. The programs that
/// use the same engine of the same chip add up, and the engine with the most is how busy the chip is.
#[cfg(any(windows, test))]
fn loads_by_chip(readings: impl IntoIterator<Item = (String, f64)>) -> HashMap<Luid, f32> {
    let mut engines: HashMap<(Luid, String), f64> = HashMap::new();
    for (name, percent) in readings {
        let Some(luid) = luid_of(&name) else { continue };
        // What follows the LUID names the engine: its number and its kind.
        let engine = name.split_once("luid_").map_or("", |(_, rest)| rest.splitn(3, '_').nth(2).unwrap_or("")).to_ascii_lowercase();
        *engines.entry((luid, engine)).or_default() += percent.max(0.0);
    }
    let mut chips: HashMap<Luid, f64> = HashMap::new();
    for ((luid, _), sum) in engines {
        let busiest = chips.entry(luid).or_default();
        *busiest = busiest.max(sum);
    }
    chips.into_iter().map(|(luid, percent)| (luid, (percent / 100.0).clamp(0.0, 1.0) as f32)).collect()
}

/// The name of an adapter from the text Windows gives (ends at the first nul).
#[cfg(any(windows, test))]
fn adapter_name(wide: &[u16]) -> String {
    let end = wide.iter().position(|c| *c == 0).unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..end]).trim().to_string()
}

/// The graphics chips of this PC with their names, as Windows lists them. The software one that Windows draws with
/// when there is no chip ("Microsoft Basic Render Driver") is left out.
#[cfg(windows)]
fn adapters() -> Vec<(Luid, String)> {
    const SOFTWARE: u32 = 2; // DXGI_ADAPTER_FLAG_SOFTWARE
    let mut found = Vec::new();
    // SAFETY: plain DXGI calls; each object is released when it goes out of scope.
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else { return found };
        let mut i = 0;
        while let Ok(adapter) = factory.EnumAdapters1(i) {
            i += 1;
            let Ok(desc) = adapter.GetDesc1() else { continue };
            if desc.Flags & SOFTWARE != 0 {
                continue;
            }
            found.push(((desc.AdapterLuid.HighPart as u32, desc.AdapterLuid.LowPart), adapter_name(&desc.Description)));
            if found.len() == MAX_CHIPS {
                break;
            }
        }
    }
    found
}

/// Names the chips at once, then reads their counters once a second while the look is wanted (`asked`), and sleeps
/// while it is not, for the rest of the run.
#[cfg(windows)]
fn watch_gpus(report: &Mutex<Option<Vec<Chip>>>, asked: &AtomicU64) {
    let chips = adapters();
    let say = |chips: Vec<Chip>| {
        if let Ok(mut report) = report.lock() {
            *report = Some(chips);
        }
    };
    if chips.is_empty() {
        say(Vec::new());
        return;
    }
    // The names are there at once, with the loads still to come.
    let names = || chips.iter().map(|(_, name)| Chip { name: name.clone(), load: None }).collect::<Vec<_>>();
    say(names());
    let path: Vec<u16> = r"\GPU Engine(*)\Utilization Percentage".encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: the query and the counter are used only on this thread, and the query stays open for as long as it lives.
    unsafe {
        let mut query = PDH_HQUERY::default();
        if PdhOpenQueryW(PCWSTR::null(), 0, &mut query) != 0 {
            say(Vec::new());
            return;
        }
        let mut counter = PDH_HCOUNTER::default();
        if PdhAddEnglishCounterW(query, PCWSTR(path.as_ptr()), 0, &mut counter) != 0 {
            let _ = PdhCloseQuery(query);
            say(Vec::new());
            return;
        }
        loop {
            // Asleep until the Stats view asks. A rate needs two readings, so the first one on waking is only the start.
            while !gpu_wanted(asked.load(Ordering::Relaxed), now_ms()) {
                std::thread::park();
            }
            let _ = PdhCollectQueryData(query);
            // The first reading after waking comes quickly, so the graphics chips show their load soon; then once a second.
            let mut wait = 300;
            while gpu_wanted(asked.load(Ordering::Relaxed), now_ms()) {
                std::thread::sleep(Duration::from_millis(wait));
                wait = 1000;
                if PdhCollectQueryData(query) != 0 {
                    continue;
                }
                match engine_readings(counter) {
                    // A chip that no program is using has no instance in the counter: it is idle.
                    Some(readings) if !readings.is_empty() => {
                        let loads = loads_by_chip(readings);
                        say(chips.iter().map(|(luid, name)| Chip { name: name.clone(), load: Some(loads.get(luid).copied().unwrap_or(0.0)) }).collect());
                    }
                    _ => say(Vec::new()),
                }
            }
            // Nobody asked for a while: back to the names, until the next ask.
            say(names());
        }
    }
}

/// The name and percentage of every instance of the counter, or `None` when Windows gives none.
///
/// # Safety
/// `counter` must be a counter of an open query on which data has been collected.
#[cfg(windows)]
unsafe fn engine_readings(counter: PDH_HCOUNTER) -> Option<Vec<(String, f64)>> {
    // The buffer is `u64`s so that it is lined up for the items that Windows writes into it.
    let mut buffer: Vec<u64> = Vec::new();
    let (mut bytes, mut count) = (0u32, 0u32);
    for _ in 0..3 {
        let items = (!buffer.is_empty()).then(|| buffer.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>());
        // SAFETY: with no buffer Windows only says how big one must be; otherwise `bytes` is the size of `buffer`.
        let status = unsafe { PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut bytes, &mut count, items) };
        if status == PDH_MORE_DATA {
            buffer = vec![0u64; (bytes as usize).div_ceil(8) + 1];
            bytes = (buffer.len() * 8) as u32;
            continue;
        }
        if status != 0 || buffer.is_empty() {
            return None;
        }
        // SAFETY: Windows wrote `count` items at the start of the buffer, with their names after them inside it.
        let items = unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>(), count as usize) };
        return Some(
            items
                .iter()
                .filter(|item| item.FmtValue.CStatus == 0 || item.FmtValue.CStatus == 1)
                .map(|item| {
                    // SAFETY: the name is a nul-ended string inside the buffer; the value is a double because that was asked for.
                    unsafe { (String::from_utf16_lossy(item.szName.as_wide()), item.FmtValue.Anonymous.doubleValue) }
                })
                .collect(),
        );
    }
    None
}

#[cfg(windows)]
fn filetime(t: FILETIME) -> u64 {
    (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)
}

/// Idle time and total (kernel and user) time of all the processors, in 100 ns units.
#[cfg(windows)]
fn system_times() -> Option<(u64, u64)> {
    let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
    // SAFETY: three valid pointers to FILETIMEs.
    unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)).ok()? };
    // The kernel time includes the idle time.
    Some((filetime(idle), filetime(kernel) + filetime(user)))
}

#[cfg(target_os = "linux")]
fn system_times() -> Option<(u64, u64)> {
    linux::system_times()
}

#[cfg(not(any(windows, target_os = "linux")))]
fn system_times() -> Option<(u64, u64)> {
    None
}

/// Memory in use and in all, in bytes.
#[cfg(windows)]
fn memory() -> Option<(u64, u64)> {
    let mut info = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    // SAFETY: `info` is a valid MEMORYSTATUSEX with its length set.
    unsafe { GlobalMemoryStatusEx(&mut info).ok()? };
    (info.ullTotalPhys > 0).then(|| (info.ullTotalPhys.saturating_sub(info.ullAvailPhys), info.ullTotalPhys))
}

#[cfg(target_os = "linux")]
fn memory() -> Option<(u64, u64)> {
    linux::memory()
}

#[cfg(not(any(windows, target_os = "linux")))]
fn memory() -> Option<(u64, u64)> {
    None
}

/// The drives with a disk in: fixed ones and removable ones (USB sticks, card readers), not network drives or
/// CD drives. An empty card reader is skipped, without Windows asking anyone to insert a disk.
#[cfg(windows)]
fn drives() -> Vec<Drive> {
    let mut found = Vec::new();
    let mut before = Default::default();
    // SAFETY: plain Windows calls; the error mode of this thread is put back at the end.
    unsafe {
        let _ = SetThreadErrorMode(SEM_FAILCRITICALERRORS, Some(&mut before));
        let letters = GetLogicalDrives();
        for i in 0..26u32 {
            if letters & (1 << i) == 0 {
                continue;
            }
            let letter = (b'A' + i as u8) as char;
            let root: Vec<u16> = format!("{letter}:\\").encode_utf16().chain(std::iter::once(0)).collect();
            let kind = GetDriveTypeW(PCWSTR(root.as_ptr()));
            if kind != DRIVE_FIXED && kind != DRIVE_REMOVABLE {
                continue;
            }
            let (mut free, mut total) = (0u64, 0u64);
            if GetDiskFreeSpaceExW(PCWSTR(root.as_ptr()), Some(&mut free), Some(&mut total), None).is_ok() && total > 0 {
                found.push(Drive { letter, used: total.saturating_sub(free), total });
                if found.len() == MAX_DRIVES {
                    break;
                }
            }
        }
        let _ = SetThreadErrorMode(before, None);
    }
    found
}

#[cfg(target_os = "linux")]
fn drives() -> Vec<Drive> {
    linux::drives()
}

#[cfg(not(any(windows, target_os = "linux")))]
fn drives() -> Vec<Drive> {
    Vec::new()
}

/// Bytes received and sent so far by the network adapters that are really there and up (not the loopback, and not
/// the virtual ones that only repeat what a real one carries).
#[cfg(windows)]
fn network_totals() -> Option<(u64, u64)> {
    let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    // SAFETY: Windows fills in a table that is freed below; the rows are only read inside its stated length.
    unsafe {
        if GetIfTable2(&mut table).is_err() || table.is_null() {
            return None;
        }
        let rows = std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize);
        let (mut received, mut sent) = (0u64, 0u64);
        for row in rows {
            // Bit 0 of the flags is "hardware interface"; an operational status of 1 is "up"; type 24 is the loopback.
            let hardware = row.InterfaceAndOperStatusFlags._bitfield & 1 != 0;
            if hardware && row.OperStatus.0 == 1 && row.Type != 24 {
                received += row.InOctets;
                sent += row.OutOctets;
            }
        }
        FreeMibTable(table as *const _);
        Some((received, sent))
    }
}

#[cfg(target_os = "linux")]
fn network_totals() -> Option<(u64, u64)> {
    linux::network_totals()
}

#[cfg(not(any(windows, target_os = "linux")))]
fn network_totals() -> Option<(u64, u64)> {
    None
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    pub(super) fn parse_proc_stat(text: &str) -> Option<(u64, u64)> {
        let line = text.lines().find(|l| l.starts_with("cpu "))?;
        // Use the first 8 columns (user..=steal); columns 9–10 (guest, guest_nice) are already included in user/nice.
        let vals: Vec<u64> = line.split_whitespace().skip(1).take(8).filter_map(|s| s.parse::<u64>().ok()).collect();
        if vals.len() < 4 {
            return None;
        }
        let idle = vals[3].saturating_add(vals.get(4).copied().unwrap_or(0));
        let total: u64 = vals.iter().copied().fold(0, u64::saturating_add);
        (total > 0).then_some((idle, total))
    }

    pub(super) fn system_times() -> Option<(u64, u64)> {
        let text = std::fs::read_to_string("/proc/stat").ok()?;
        parse_proc_stat(&text)
    }

    pub(super) fn parse_proc_meminfo(text: &str) -> Option<(u64, u64)> {
        let (mut total, mut available, mut free, mut buffers, mut cached) = (0u64, None, 0u64, 0u64, 0u64);
        for line in text.lines() {
            let Some((key, rest)) = line.split_once(':') else { continue };
            let Some(kb) = rest.split_whitespace().next().and_then(|s| s.parse::<u64>().ok()) else { continue };
            let bytes = kb.saturating_mul(1024);
            match key.trim() {
                "MemTotal" => total = bytes,
                "MemAvailable" => available = Some(bytes),
                "MemFree" => free = bytes,
                "Buffers" => buffers = bytes,
                "Cached" => cached = bytes,
                _ => {}
            }
        }
        if total == 0 {
            return None;
        }
        let avail = available.unwrap_or_else(|| free.saturating_add(buffers).saturating_add(cached));
        Some((total.saturating_sub(avail), total))
    }

    pub(super) fn memory() -> Option<(u64, u64)> {
        let text = std::fs::read_to_string("/proc/meminfo").ok()?;
        parse_proc_meminfo(&text)
    }

    pub(super) fn parse_proc_net_dev(text: &str, include: impl Fn(&str) -> bool) -> (u64, u64) {
        let (mut received, mut sent) = (0u64, 0u64);
        for line in text.lines().skip(2) {
            let Some((iface, rest)) = line.split_once(':') else { continue };
            let iface = iface.trim();
            if !include(iface) {
                continue;
            }
            let mut cols = rest.split_whitespace();
            let Some(rx) = cols.next().and_then(|s| s.parse::<u64>().ok()) else { continue };
            let Some(tx) = cols.nth(7).and_then(|s| s.parse::<u64>().ok()) else { continue };
            received = received.saturating_add(rx);
            sent = sent.saturating_add(tx);
        }
        (received, sent)
    }

    fn is_hw_net_up(iface: &str) -> bool {
        if iface == "lo" {
            return false;
        }
        let sys = Path::new("/sys/class/net").join(iface);
        if !sys.join("device").exists() {
            return false;
        }
        std::fs::read_to_string(sys.join("operstate")).ok().is_some_and(|s| s.trim() == "up")
    }

    pub(super) fn network_totals() -> Option<(u64, u64)> {
        let text = std::fs::read_to_string("/proc/net/dev").ok()?;
        Some(parse_proc_net_dev(&text, is_hw_net_up))
    }

    pub(super) fn short_mount_label(mount: &str) -> String {
        if mount == "/" {
            return "/".to_string();
        }
        let seg = mount.split('/').rfind(|s| !s.is_empty()).unwrap_or("/");
        if seg.chars().count() > 8 {
            let mut out: String = seg.chars().take(7).collect();
            out.push('…');
            out
        } else {
            seg.to_string()
        }
    }

    pub(super) fn parse_df_output(text: &str, block_scale: u64) -> Vec<Drive> {
        let mut by_dev: HashMap<String, (String, u64, u64)> = HashMap::new();
        for line in text.lines().skip(1) {
            let mut tokens = [""; 5];
            let mut rest = line.trim_start();
            let mut ok = true;
            for slot in &mut tokens {
                let Some(end) = rest.find(char::is_whitespace) else {
                    ok = false;
                    break;
                };
                *slot = &rest[..end];
                rest = rest[end..].trim_start();
            }
            if !ok || rest.is_empty() {
                continue;
            }
            let (fs, blocks_s, used_s, mount) = (tokens[0], tokens[1], tokens[2], rest.trim_end());
            if !fs.starts_with("/dev/") || fs.starts_with("/dev/loop") || fs.starts_with("/dev/sr") || fs.starts_with("/dev/fd") {
                continue;
            }
            if mount == "/boot/efi" || mount == "/efi" || mount.starts_with("/snap/") {
                continue;
            }
            let (Ok(blocks), Ok(used_blocks)) = (blocks_s.parse::<u64>(), used_s.parse::<u64>()) else { continue };
            let total = blocks.saturating_mul(block_scale);
            let used = used_blocks.saturating_mul(block_scale).min(total);
            if total == 0 {
                continue;
            }
            let replace = match by_dev.get(fs) {
                None => true,
                Some((prev_mount, _, _)) => mount.len() < prev_mount.len() || (mount.len() == prev_mount.len() && mount < prev_mount.as_str()),
            };
            if replace {
                by_dev.insert(fs.to_string(), (mount.to_string(), used, total));
            }
        }
        let mut entries: Vec<(String, u64, u64)> = by_dev.into_values().collect();
        entries.sort_by(|a, b| (a.0 != "/").cmp(&(b.0 != "/")).then_with(|| a.0.cmp(&b.0)));
        entries.truncate(MAX_DRIVES);
        entries
            .into_iter()
            .map(|(mount, used, total)| {
                let label = short_mount_label(&mount);
                let letter = label.chars().next().unwrap_or('/');
                Drive { letter, used, total, label }
            })
            .collect()
    }

    pub(super) fn drives() -> Vec<Drive> {
        let run = |args: &[&str]| Command::new("df").args(args).env("LC_ALL", "C").stdin(Stdio::null()).stderr(Stdio::null()).output().ok().filter(|o| o.status.success());
        if let Some(out) = run(&["-P", "-B1"]) {
            return parse_df_output(&String::from_utf8_lossy(&out.stdout), 1);
        }
        if let Some(out) = run(&["-P", "-k"]) {
            return parse_df_output(&String::from_utf8_lossy(&out.stdout), 1024);
        }
        Vec::new()
    }

    pub(super) fn clean_pci_gpu_name(raw: &str) -> String {
        let trimmed = raw.trim();
        let inner = if let (Some(l), Some(r)) = (trimmed.find('['), trimmed.rfind(']'))
            && r > l + 1
        {
            trimmed[l + 1..r].trim()
        } else {
            trimmed
        };
        inner.split(" / ").next().and_then(|s| s.split('/').next()).unwrap_or(inner).trim().to_string()
    }

    pub(super) fn lookup_pci_name(pci_ids: &str, vendor: &str, device: &str) -> Option<String> {
        let v = vendor.trim().trim_start_matches("0x").trim_start_matches("0X").to_ascii_lowercase();
        let d = device.trim().trim_start_matches("0x").trim_start_matches("0X").to_ascii_lowercase();
        if v.is_empty() || d.is_empty() {
            return None;
        }
        let mut in_vendor = false;
        for line in pci_ids.lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            if !line.starts_with('\t') {
                if in_vendor {
                    break;
                }
                if line.strip_prefix(&v).is_some_and(|r| r.starts_with(char::is_whitespace)) {
                    in_vendor = true;
                }
                continue;
            }
            if in_vendor
                && let Some(after_tab) = line.strip_prefix('\t').filter(|s| !s.starts_with('\t'))
                && let Some(rest) = after_tab.strip_prefix(&d)
                && rest.starts_with(char::is_whitespace)
            {
                let name = rest.trim();
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
        None
    }

    pub(super) fn parse_nvidia_smi(text: &str) -> Vec<Chip> {
        let mut chips = Vec::new();
        for line in text.lines() {
            let Some((name, util_s)) = line.rsplit_once(',') else { continue };
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            let load = util_s.trim().parse::<f32>().ok().map(|p| (p / 100.0).clamp(0.0, 1.0));
            chips.push(Chip { name: name.to_string(), load });
            if chips.len() == MAX_CHIPS {
                break;
            }
        }
        chips
    }

    fn resolve_card_name(dev_dir: &Path, pci_ids: Option<&str>, vendor_hex: &str) -> String {
        if let Ok(name) = std::fs::read_to_string(dev_dir.join("product_name")) {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
        if let (Some(ids), Ok(dev)) = (pci_ids, std::fs::read_to_string(dev_dir.join("device")))
            && let Some(raw) = lookup_pci_name(ids, vendor_hex, &dev)
        {
            let cleaned = clean_pci_gpu_name(&raw);
            if !cleaned.is_empty() {
                return cleaned;
            }
        }
        match vendor_hex {
            "1002" => "AMD Radeon",
            "8086" => "Intel Graphics",
            "10de" => "NVIDIA GPU",
            _ => "GPU",
        }
        .to_string()
    }

    struct SysfsGpu {
        name: String,
        busy_path: PathBuf,
    }

    fn discover_gpus() -> (Vec<SysfsGpu>, bool) {
        let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
            return (Vec::new(), false);
        };
        let mut cards: Vec<(String, PathBuf)> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let num = name.strip_prefix("card")?;
                (!num.is_empty() && num.bytes().all(|b| b.is_ascii_digit())).then(|| (name, e.path()))
            })
            .collect();
        cards.sort_by(|a, b| a.0.cmp(&b.0));

        let pci_ids = ["/usr/share/hwdata/pci.ids", "/usr/share/misc/pci.ids", "/usr/share/pci.ids"].into_iter().find_map(|p| std::fs::read_to_string(p).ok());

        let mut sysfs = Vec::new();
        let mut has_nvidia = false;
        for (_, card_path) in cards {
            let dev_dir = card_path.join("device");
            let vendor =
                std::fs::read_to_string(dev_dir.join("vendor")).map(|s| s.trim().trim_start_matches("0x").trim_start_matches("0X").to_ascii_lowercase()).unwrap_or_default();
            let busy_path = dev_dir.join("gpu_busy_percent");
            if std::fs::read_to_string(&busy_path).ok().and_then(|s| s.trim().parse::<u64>().ok()).is_some() {
                let name = resolve_card_name(&dev_dir, pci_ids.as_deref(), &vendor);
                sysfs.push(SysfsGpu { name, busy_path });
            } else if vendor == "10de" {
                has_nvidia = true;
            }
        }
        (sysfs, has_nvidia)
    }

    fn read_nvidia_gpus() -> Vec<Chip> {
        let Ok(out) = Command::new("nvidia-smi")
            .args(["--query-gpu=name,utilization.gpu", "--format=csv,noheader,nounits"])
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
        else {
            return Vec::new();
        };
        if !out.status.success() {
            return Vec::new();
        }
        parse_nvidia_smi(&String::from_utf8_lossy(&out.stdout))
    }

    fn watch_gpus(report: &Mutex<Option<Vec<Chip>>>, asked: &AtomicU64) {
        let (sysfs, has_nvidia) = discover_gpus();
        let say = |chips: Vec<Chip>| {
            if let Ok(mut guard) = report.lock() {
                *guard = Some(chips);
            }
        };
        if sysfs.is_empty() && !has_nvidia {
            say(Vec::new());
            return;
        }
        let names = || sysfs.iter().take(MAX_CHIPS).map(|g| Chip { name: g.name.clone(), load: None }).collect::<Vec<_>>();
        say(names());

        loop {
            while !gpu_wanted(asked.load(Ordering::Relaxed), now_ms()) {
                std::thread::park();
            }
            while gpu_wanted(asked.load(Ordering::Relaxed), now_ms()) {
                let mut chips = Vec::with_capacity(MAX_CHIPS);
                for g in &sysfs {
                    let load = std::fs::read_to_string(&g.busy_path).ok().and_then(|s| s.trim().parse::<f32>().ok()).map(|p| (p / 100.0).clamp(0.0, 1.0));
                    chips.push(Chip { name: g.name.clone(), load });
                    if chips.len() == MAX_CHIPS {
                        break;
                    }
                }
                if has_nvidia && chips.len() < MAX_CHIPS {
                    for nv in read_nvidia_gpus() {
                        chips.push(nv);
                        if chips.len() == MAX_CHIPS {
                            break;
                        }
                    }
                }
                say(chips);
                std::thread::sleep(Duration::from_secs(1));
            }
            say(names());
        }
    }

    static GPU_WATCH: OnceLock<GpuWatch> = OnceLock::new();

    pub(super) fn gpu_look() -> Option<Vec<Chip>> {
        let watch = GPU_WATCH.get_or_init(|| {
            let (report, asked) = (GpuReport::default(), Arc::new(AtomicU64::new(0)));
            let (r, a) = (report.clone(), asked.clone());
            a.store(now_ms(), Ordering::Relaxed);
            let thread =
                std::thread::Builder::new().name("gpu-stats".into()).spawn(move || watch_gpus(&r, &a)).map(|h| h.thread().clone()).unwrap_or_else(|_| std::thread::current());
            GpuWatch { report, asked, thread }
        });
        let now = now_ms();
        let was = watch.asked.swap(now, Ordering::Relaxed);
        if !gpu_wanted(was, now)
            && let Ok(mut report) = watch.report.lock()
            && let Some(chips) = report.as_mut()
        {
            chips.iter_mut().for_each(|chip| chip.load = None);
        }
        watch.thread.unpark();
        watch.report.lock().ok().and_then(|report| report.clone())
    }
}

/// What the network graph is scaled by: the fastest second in `history`, in either direction, but never less than a
/// quiet link.
pub fn scale_of(history: &VecDeque<(f32, f32)>) -> f32 {
    history.iter().map(|(down, up)| down.max(*up)).fold(0.0_f32, f32::max).max(QUIET_LINK)
}

/// "9.1 GB" from a number of bytes.
pub fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
}

/// How much of a drive is used, in words: "312 / 476 GB", or "1.2 / 1.8 TB" for a drive of a terabyte or more.
pub fn used_of(used: u64, total: u64) -> String {
    const GB: f64 = 1_073_741_824.0;
    let (used, total) = (used as f64 / GB, total as f64 / GB);
    if total >= 1024.0 { format!("{:.1} / {:.1} TB", used / 1024.0, total / 1024.0) } else { format!("{:.0} / {:.0} GB", used, total) }
}

/// A graphics chip's name made short for a small label: "NVIDIA GeForce RTX 4050 Laptop GPU" is "RTX 4050" and
/// "Intel(R) UHD Graphics" is "Intel UHD".
pub fn short_gpu_name(name: &str) -> String {
    let mut short = name.replace("(R)", "").replace("(TM)", "").replace("(tm)", "").replace("(r)", "");
    for word in ["NVIDIA ", "GeForce ", "AMD "] {
        short = short.replace(word, "");
    }
    for tail in [" Laptop GPU", " Graphics", " GPU"] {
        let stripped = short.trim().strip_suffix(tail).map(str::to_string);
        if let Some(stripped) = stripped {
            short = stripped;
        }
    }
    let short = short.split_whitespace().collect::<Vec<_>>().join(" ");
    if short.is_empty() { name.trim().to_string() } else { short }
}

/// How many threads the processor runs at once (what Task Manager calls logical processors).
pub fn threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

/// "1.2 MB/s", "80 KB/s" or "0 B/s" from bytes a second.
pub fn speed(bytes_per_sec: f64) -> String {
    let b = bytes_per_sec.max(0.0);
    if b >= 1_048_576.0 {
        format!("{:.1} MB/s", b / 1_048_576.0)
    } else if b >= 1024.0 {
        format!("{:.0} KB/s", b / 1024.0)
    } else {
        format!("{:.0} B/s", b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_processor_load_is_the_share_of_time_not_idle() {
        let mut s = Stats::default();
        s.feed_cpu(1_000, 2_000);
        assert_eq!(s.last.cpu, None, "the first look has nothing to compare with");
        s.feed_cpu(1_750, 3_000); // 1000 passed, 750 of it idle
        assert_eq!(s.last.cpu, Some(0.25));
        s.feed_cpu(1_750, 4_000); // all of it busy
        assert_eq!(s.last.cpu, Some(1.0));
        s.feed_cpu(1_750, 4_000); // no time passed: the old value stays
        assert_eq!(s.last.cpu, Some(1.0));
        assert_eq!(s.history.len(), 2);
    }

    #[test]
    fn the_history_keeps_the_last_minute() {
        let mut s = Stats::default();
        for i in 0..(HISTORY as u64 + 20) {
            s.feed_cpu(i * 10, i * 100);
        }
        assert_eq!(s.history.len(), HISTORY);
    }

    #[test]
    fn the_network_speed_is_bytes_over_seconds() {
        let mut s = Stats::default();
        let t = Instant::now();
        s.feed_net(1_000, 500, t);
        assert_eq!(s.last.network, None);
        s.feed_net(3_000, 1_500, t + Duration::from_secs(2));
        assert_eq!(s.last.network, Some((1_000.0, 500.0)));
        // Counters that went backwards (an adapter went away) are skipped, not shown as a huge number.
        s.feed_net(10, 10, t + Duration::from_secs(3));
        assert_eq!(s.last.network, Some((1_000.0, 500.0)));
        s.feed_net(110, 60, t + Duration::from_secs(4));
        assert_eq!(s.last.network, Some((100.0, 50.0)));
    }

    #[test]
    fn each_graphics_chip_is_as_busy_as_its_busiest_engine() {
        let reading = |name: &str, percent: f64| (name.to_string(), percent);
        let (a, b) = ("luid_0x00000000_0x0000D2A9_phys_0_eng_0_engtype_3D", "luid_0x00000000_0x0000D2A9_phys_0_eng_1_engtype_Copy");
        let chip = (0, 0xD2A9);
        // Two programs on the same engine add up; another engine of the same chip does not add to them.
        let loads = loads_by_chip([reading(&format!("pid_10_{a}"), 20.0), reading(&format!("pid_20_{a}"), 15.0), reading(&format!("pid_10_{b}"), 30.0)]);
        assert_eq!(loads.get(&chip), Some(&0.35));
        // A load over 100 in all is shown as full.
        assert_eq!(loads_by_chip([reading(&format!("pid_1_{a}"), 80.0), reading(&format!("pid_2_{a}"), 70.0)]).get(&chip), Some(&1.0));
        assert!(loads_by_chip(Vec::new()).is_empty(), "no instances at all is no loads");
        // Two chips are told apart by their LUID, whatever the case of the letters.
        let other = "pid_3_luid_0x00000000_0x0000ffff_phys_0_eng_0_engtype_3d";
        let loads = loads_by_chip([reading(&format!("pid_1_{a}"), 10.0), reading(other, 50.0)]);
        assert_eq!((loads.get(&chip), loads.get(&(0, 0xFFFF))), (Some(&0.1), Some(&0.5)));
        // A name with no LUID is not a chip.
        assert!(loads_by_chip([reading("_Total", 99.0)]).is_empty());
    }

    #[test]
    fn the_luid_is_read_from_the_name_of_an_instance() {
        assert_eq!(luid_of("pid_4568_luid_0x00000000_0x0000D2A9_phys_0_eng_0_engtype_3D"), Some((0, 0xD2A9)));
        assert_eq!(luid_of("pid_4_luid_0x00000001_0x00012d4d_phys_0"), Some((1, 0x12D4D)));
        assert_eq!(luid_of("_Total"), None);
        assert_eq!(luid_of("pid_4_luid_zzz"), None);
    }

    #[test]
    fn an_adapter_is_named_by_its_text_up_to_the_nul() {
        let mut wide = [0u16; 128];
        for (slot, c) in wide.iter_mut().zip("NVIDIA GeForce RTX 3050 ".encode_utf16()) {
            *slot = c;
        }
        assert_eq!(adapter_name(&wide), "NVIDIA GeForce RTX 3050");
        assert_eq!(adapter_name(&[]), "");
    }

    #[test]
    fn the_look_at_the_graphics_chips_is_wanted_for_a_few_seconds_after_it_was_asked_for() {
        assert!(gpu_wanted(10_000, 10_000));
        assert!(gpu_wanted(10_000, 10_000 + GPU_GRACE_MS));
        assert!(!gpu_wanted(10_000, 10_000 + GPU_GRACE_MS + 1));
        assert!(gpu_wanted(10_000, 9_000), "a clock that went back is not a reason to stop");
    }

    #[cfg(windows)]
    #[test]
    fn the_graphics_chips_are_named_on_their_own_thread() {
        let mut tries = 0;
        let mut chips = gpu_look();
        while chips.is_none() && tries < 100 {
            std::thread::sleep(Duration::from_millis(50));
            chips = gpu_look();
            tries += 1;
        }
        // On a PC with graphics chips they have names (and, once a second has passed, loads); on one without (a build
        // server) the list is empty; it is never still unanswered.
        let chips = chips.expect("an answer within five seconds");
        assert!(chips.len() <= MAX_CHIPS);
        for chip in &chips {
            assert!(!chip.name.is_empty());
            assert!(chip.load.is_none_or(|load| (0.0..=1.0).contains(&load)));
        }
    }

    #[test]
    fn the_network_and_the_graphics_chip_keep_a_minute_of_history() {
        let mut s = Stats::default();
        let t = Instant::now();
        for i in 0..(HISTORY as u64 + 10) {
            s.feed_net(i * 1_000, i * 100, t + Duration::from_secs(i));
        }
        assert_eq!(s.net_history.len(), HISTORY);
        assert_eq!(s.net_history.back(), Some(&(1_000.0, 100.0)));
    }

    #[test]
    fn the_network_graph_is_scaled_to_the_fastest_second_but_never_below_a_quiet_link() {
        let mut history: VecDeque<(f32, f32)> = VecDeque::new();
        assert_eq!(scale_of(&history), QUIET_LINK, "no history yet");
        history.push_back((1_000.0, 500.0));
        assert_eq!(scale_of(&history), QUIET_LINK, "a few hundred bytes a second is a quiet link");
        history.push_back((100.0, 4_000_000.0));
        assert_eq!(scale_of(&history), 4_000_000.0, "a fast second, either way, sets the scale");
    }

    #[test]
    fn drives_and_chips_are_written_for_people() {
        const GB: u64 = 1_073_741_824;
        assert_eq!(used_of(312 * GB, 476 * GB), "312 / 476 GB");
        assert_eq!(used_of(1229 * GB, 1863 * GB), "1.2 / 1.8 TB");
        assert_eq!(short_gpu_name("NVIDIA GeForce RTX 4050 Laptop GPU"), "RTX 4050");
        assert_eq!(short_gpu_name("Intel(R) UHD Graphics"), "Intel UHD");
        assert_eq!(short_gpu_name("AMD Radeon(TM) Graphics"), "Radeon");
        assert_eq!(short_gpu_name("AMD Radeon RX 7600"), "Radeon RX 7600");
        assert_eq!(short_gpu_name("Some Card"), "Some Card");
        assert_eq!(short_gpu_name("GPU"), "GPU", "never empty");
        assert!(threads() >= 1);
    }

    #[test]
    fn numbers_are_written_for_people() {
        assert_eq!(gigabytes(1_073_741_824 * 9 + 107_374_182), "9.1 GB");
        assert_eq!(speed(0.0), "0 B/s");
        assert_eq!(speed(900.0), "900 B/s");
        assert_eq!(speed(80.0 * 1024.0), "80 KB/s");
        assert_eq!(speed(1.2 * 1_048_576.0), "1.2 MB/s");
        assert_eq!(speed(-5.0), "0 B/s");
    }

    #[cfg(windows)]
    #[test]
    fn the_drives_arrive_from_their_own_thread_without_the_look_waiting() {
        let mut s = Stats::default();
        let start = Instant::now();
        s.sample(start);
        assert!(!s.disks_known || !s.last.disks.is_empty());
        // The answer comes a moment later; look again until it is there (at most a few seconds).
        let mut tries = 0;
        while !s.disks_known && tries < 200 {
            std::thread::sleep(Duration::from_millis(20));
            s.sample(start + Duration::from_millis(20 * tries));
            tries += 1;
        }
        assert!(s.disks_known && !s.last.disks.is_empty(), "the drives were found");
    }

    #[cfg(windows)]
    #[test]
    fn this_pc_answers_the_questions() {
        // Real calls, so only what is true on any Windows PC: memory and disk exist, and the processor times grow.
        let (used, total) = memory().expect("memory");
        assert!(total > used && used > 0);
        let disks = drives();
        assert!(!disks.is_empty() && disks.len() <= MAX_DRIVES, "at least the drive Windows is on");
        assert!(disks.iter().all(|d| d.total >= d.used && d.total > 0));
        assert!(disks.windows(2).all(|w| w[0].letter < w[1].letter), "in the order of their letters");
        let (idle, all) = system_times().expect("times");
        assert!(all >= idle && all > 0);
        assert!(network_totals().is_some());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn proc_stat_parser_counts_idle_and_skips_guest_double_count() {
        let sample = "\
cpu  100 10 40 800 50 5 5 0 99 11
cpu0 50 5 20 400 25 2 3 0 50 5
";
        // idle + iowait = 800 + 50 = 850; total (first 8 fields only) = 1010.
        assert_eq!(linux::parse_proc_stat(sample), Some((850, 1010)));
        assert_eq!(linux::parse_proc_stat("cpu  1 2 3\n"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn proc_meminfo_parser_uses_available_with_legacy_fallback() {
        let modern = "\
MemTotal:       16384000 kB
MemFree:         2000000 kB
MemAvailable:   10000000 kB
Buffers:          500000 kB
Cached:          4000000 kB
";
        assert_eq!(linux::parse_proc_meminfo(modern), Some((6_384_000 * 1024, 16_384_000 * 1024)));

        let legacy = "\
MemTotal:       1000 kB
MemFree:         200 kB
Buffers:         100 kB
Cached:          300 kB
";
        assert_eq!(linux::parse_proc_meminfo(legacy), Some((400 * 1024, 1000 * 1024)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn proc_net_dev_parser_sums_selected_interfaces() {
        let sample = "\
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo: 5000 10 0 0 0 0 0 0 5000 10 0 0 0 0 0 0
wlp3s0: 12000 100 0 0 0 0 0 0 3400 50 0 0 0 0 0 0
docker0: 99999 10 0 0 0 0 0 0 88888 10 0 0 0 0 0 0
eth0: 800 5 0 0 0 0 0 0 600 4 0 0 0 0 0 0
";
        assert_eq!(linux::parse_proc_net_dev(sample, |n| matches!(n, "wlp3s0" | "eth0")), (12_800, 4_000));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn df_parser_deduplicates_btrfs_subvolumes_and_handles_spaces() {
        let df_out = "\
Filesystem     1-blocks      Used Available Capacity Mounted on
tmpfs        1000000000  10000000 990000000       1% /tmp
/dev/loop0    100000000 100000000         0     100% /snap/core/1
/dev/mapper/root 500000000000 300000000000 200000000000 60% /home
/dev/mapper/root 500000000000 300000000000 200000000000 60% /
/dev/nvme0n1p1  2000000000   500000000  1500000000 25% /boot/efi
/dev/sda1     64000000000  16000000000 48000000000 25% /run/media/user/My Backup Drive
";
        let found = linux::parse_df_output(df_out, 1);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].label, "/");
        assert_eq!(found[0].total, 500_000_000_000);
        assert_eq!(found[1].label, "My Back…");
        assert_eq!(found[1].used, 16_000_000_000);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn pci_id_lookup_and_nvidia_smi_parser() {
        let ids = "\
# comment
1002  Advanced Micro Devices, Inc. [AMD/ATI]
\t1636  Renoir [Radeon Vega Series / Radeon Vega Mobile Series]
\t\t17aa 5081  Lenovo Device
10de  NVIDIA Corporation
\t28e0  AD107M [GeForce RTX 4050 Max-Q / Mobile]
";
        let raw = linux::lookup_pci_name(ids, "0x1002", "0x1636").unwrap();
        assert_eq!(linux::clean_pci_gpu_name(&raw), "Radeon Vega Series");
        assert_eq!(linux::clean_pci_gpu_name("Intel UHD Graphics 630"), "Intel UHD Graphics 630");

        let nv = linux::parse_nvidia_smi("NVIDIA GeForce RTX 4050 Laptop GPU, 37\nNVIDIA GeForce RTX 3090, 105\n");
        assert_eq!(nv.len(), 2);
        assert_eq!(nv[0].name, "NVIDIA GeForce RTX 4050 Laptop GPU");
        assert_eq!(nv[0].load, Some(0.37));
        assert_eq!(nv[1].load, Some(1.0));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_this_pc_answers_the_questions() {
        let (used, total) = memory().expect("memory");
        assert!(total > used && used > 0);
        let disks = drives();
        assert!(!disks.is_empty() && disks.len() <= MAX_DRIVES);
        assert!(disks.iter().all(|d| d.total >= d.used && d.total > 0 && !d.label.is_empty()));
        let (idle, all) = system_times().expect("times");
        assert!(all >= idle && all > 0);
        assert!(network_totals().is_some());

        let mut tries = 0;
        let mut chips = gpu_look();
        while chips.is_none() && tries < 100 {
            std::thread::sleep(Duration::from_millis(50));
            chips = gpu_look();
            tries += 1;
        }
        let chips = chips.expect("gpu_look answers within five seconds");
        assert!(chips.len() <= MAX_CHIPS);
        for chip in &chips {
            assert!(!chip.name.is_empty());
            assert!(chip.load.is_none_or(|load| (0.0..=1.0).contains(&load)));
        }
    }
}
