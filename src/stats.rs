//! System stats for the Stats tab: how busy the processor is, how much memory and disk is used, and how fast data
//! moves over the network. Looked at only while the tab is open (once a second), with a few cheap Windows calls;
//! nothing runs while it is closed.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::FILETIME;
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::GetSystemTimes;
use windows::core::PCWSTR;

/// How often the numbers are looked at while the tab is open.
pub const EVERY: Duration = Duration::from_secs(1);
/// The first look is quick, so the tab does not sit empty: the processor and the network need two looks to say anything.
pub const FIRST_AGAIN: Duration = Duration::from_millis(400);
/// How many seconds of the processor's history are kept for the graph.
pub const HISTORY: usize = 60;
/// The disk is looked at this often (it hardly changes).
const DISK_EVERY: Duration = Duration::from_secs(15);

/// What the last look found. `None` where it needs a second look or Windows would not say.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Sample {
    /// How busy the processor was since the look before: 0.0 to 1.0.
    pub cpu: Option<f32>,
    /// Memory in use and in all, in bytes.
    pub memory: Option<(u64, u64)>,
    /// Disk in use and in all, in bytes (the drive Windows is on).
    pub disk: Option<(u64, u64)>,
    /// Bytes a second received and sent, over the network adapters that are really there.
    pub network: Option<(f64, f64)>,
}

#[derive(Default)]
pub struct Stats {
    pub last: Sample,
    /// The processor's load for each of the last seconds, oldest first.
    pub history: VecDeque<f32>,
    /// Whether at least one look has been taken.
    pub started: bool,
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
                self.last.network = Some(((received - pr) as f64 / secs, (sent - ps) as f64 / secs));
            }
        }
    }

    /// Take a look at everything.
    pub fn sample(&mut self, now: Instant) {
        self.started = true;
        if let Some((idle, total)) = system_times() {
            self.feed_cpu(idle, total);
        }
        self.last.memory = memory();
        if self.disk_at.is_none_or(|at| now.saturating_duration_since(at) >= DISK_EVERY) {
            self.disk_at = Some(now);
            self.last.disk = disk();
        }
        if let Some((received, sent)) = network_totals() {
            self.feed_net(received, sent, now);
        }
    }

    /// Whether the numbers that need two looks are there.
    pub fn ready(&self) -> bool {
        self.last.cpu.is_some()
    }
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

/// Space in use and in all on the drive Windows is on, in bytes.
fn disk() -> Option<(u64, u64)> {
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
    let root: Vec<u16> = format!("{drive}\\").encode_utf16().chain(std::iter::once(0)).collect();
    let (mut free, mut total) = (0u64, 0u64);
    // SAFETY: a NUL-terminated path and two valid pointers.
    unsafe { GetDiskFreeSpaceExW(PCWSTR(root.as_ptr()), Some(&mut free), Some(&mut total), None).ok()? };
    (total > 0).then(|| (total.saturating_sub(free), total))
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

/// "9.1 GB" from a number of bytes.
pub fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
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
    fn numbers_are_written_for_people() {
        assert_eq!(gigabytes(1_073_741_824 * 9 + 107_374_182), "9.1 GB");
        assert_eq!(speed(0.0), "0 B/s");
        assert_eq!(speed(900.0), "900 B/s");
        assert_eq!(speed(80.0 * 1024.0), "80 KB/s");
        assert_eq!(speed(1.2 * 1_048_576.0), "1.2 MB/s");
        assert_eq!(speed(-5.0), "0 B/s");
    }

    #[test]
    fn this_pc_answers_the_questions() {
        // Real calls, so only what is true on any Windows PC: memory and disk exist, and the processor times grow.
        let (used, total) = memory().expect("memory");
        assert!(total > used && used > 0);
        let (used, total) = disk().expect("disk");
        assert!(total >= used && total > 0);
        let (idle, all) = system_times().expect("times");
        assert!(all >= idle && all > 0);
        assert!(network_totals().is_some());
    }
}
