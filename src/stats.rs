//! System stats for the Stats tab: how busy the processor and the graphics chip are, how much memory and disk is
//! used, and how fast data moves over the network. Looked at only while the tab is open (once a second), with a few
//! cheap Windows calls; nothing runs while it is closed.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::FILETIME;
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives};
use windows::Win32::System::Diagnostics::Debug::{SEM_FAILCRITICALERRORS, SetThreadErrorMode};
use windows::Win32::System::Performance::{
    PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA, PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhOpenQueryW,
};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::GetSystemTimes;
use windows::core::PCWSTR;

/// How often the numbers are looked at while the tab is open.
pub const EVERY: Duration = Duration::from_secs(1);
/// The first look is quick, so the tab does not sit empty: the processor and the network need two looks to say anything.
pub const FIRST_AGAIN: Duration = Duration::from_millis(400);
/// How many seconds of the processor's history are kept for the graph.
pub const HISTORY: usize = 60;
/// The disks are looked at this often (they hardly change; a drive plugged in shows up within this time).
const DISK_EVERY: Duration = Duration::from_secs(15);
/// The network graph is never scaled to less than this many bytes a second, so a quiet link shows as quiet.
pub const QUIET_LINK: f32 = 64.0 * 1024.0;
/// The most drives that are listed, so the notch does not grow without end.
pub const MAX_DRIVES: usize = 6;

/// What `GetDriveTypeW` says for a hard disk or SSD and for a USB stick or card reader.
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;

/// One drive: its letter, and the space in use and in all, in bytes.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Drive {
    pub letter: char,
    pub used: u64,
    pub total: u64,
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
}

/// What the look at the graphics chips has found: nothing yet (`None`), or the chips (none at all when Windows has no
/// load to report for any chip of this PC).
type GpuReport = Arc<Mutex<Option<Vec<Chip>>>>;

/// How long the look at the graphics chips carries on after it was last asked for. The Stats view asks once a second.
const GPU_GRACE_MS: u64 = 3_000;

/// The look at the graphics chips. Naming them and opening their counters takes a couple of seconds, and reading
/// them a few milliseconds, so it is done on a thread of its own that is started the first time the Stats view
/// asks and then stays: it sleeps (without waking up) while nobody asks, and starts reading within a second of
/// the next ask. It leaves the answer in `report`.
struct GpuWatch {
    report: GpuReport,
    /// When it was last asked, in milliseconds since the program began.
    asked: Arc<AtomicU64>,
    thread: std::thread::Thread,
}

static GPU_WATCH: OnceLock<GpuWatch> = OnceLock::new();
static CLOCK: OnceLock<Instant> = OnceLock::new();

/// Milliseconds since the program began (the first time anyone asks).
fn now_ms() -> u64 {
    CLOCK.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Whether the look is still wanted: it was asked for in the last few seconds.
fn gpu_wanted(asked_ms: u64, now_ms: u64) -> bool {
    now_ms.saturating_sub(asked_ms) <= GPU_GRACE_MS
}

/// Ask for the look at the graphics chips (which starts it, or wakes it) and say what it has found so far.
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

/// The identity Windows gives a graphics chip: the two halves of its LUID. In the counters it is written into the
/// name of each instance, such as `pid_4568_luid_0x00000000_0x0000D2A9_phys_0_eng_0_engtype_3D`.
type Luid = (u32, u32);

/// The LUID in the name of an instance of the "GPU Engine" counter.
fn luid_of(instance: &str) -> Option<Luid> {
    let (_, rest) = instance.split_once("luid_")?;
    let mut halves = rest.split('_').map(|h| u32::from_str_radix(h.trim_start_matches("0x"), 16).ok());
    Some((halves.next()??, halves.next()??))
}

/// How busy each chip is: its busiest engine, from the counter "GPU Engine, Utilization Percentage" (the one Task
/// Manager uses). `readings` are the instances of the counter, each with its name and its percentage. The programs that
/// use the same engine of the same chip add up, and the engine with the most is how busy the chip is.
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
fn adapter_name(wide: &[u16]) -> String {
    let end = wide.iter().position(|c| *c == 0).unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..end]).trim().to_string()
}

/// The graphics chips of this PC with their names, as Windows lists them. The software one that Windows draws with
/// when there is no chip ("Microsoft Basic Render Driver") is left out.
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
            while gpu_wanted(asked.load(Ordering::Relaxed), now_ms()) {
                std::thread::sleep(Duration::from_millis(1000));
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

fn filetime(t: FILETIME) -> u64 {
    (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)
}

/// Idle time and total (kernel and user) time of all the processors, in 100 ns units.
fn system_times() -> Option<(u64, u64)> {
    let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
    // SAFETY: three valid pointers to FILETIMEs.
    unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)).ok()? };
    // The kernel time includes the idle time.
    Some((filetime(idle), filetime(kernel) + filetime(user)))
}

/// Memory in use and in all, in bytes.
fn memory() -> Option<(u64, u64)> {
    let mut info = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    // SAFETY: `info` is a valid MEMORYSTATUSEX with its length set.
    unsafe { GlobalMemoryStatusEx(&mut info).ok()? };
    (info.ullTotalPhys > 0).then(|| (info.ullTotalPhys.saturating_sub(info.ullAvailPhys), info.ullTotalPhys))
}

/// The drives with a disk in: fixed ones and removable ones (USB sticks, card readers), not network drives or
/// CD drives. An empty card reader is skipped, without Windows asking anyone to insert a disk.
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

/// Bytes received and sent so far by the network adapters that are really there and up (not the loopback, and not
/// the virtual ones that only repeat what a real one carries).
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
}
