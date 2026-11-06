//! The real loudness of the music, for the bars on the closed notch. Windows lets a program listen to the sound that is
//! going to the speakers or the headphones (WASAPI "loopback"); this reads it, splits it into five bands of pitch with
//! an FFT, and says how loud each band is. Nothing is recorded or sent anywhere: the samples are looked at as they come
//! and thrown away, and the microphone is not used.
//!
//! It listens on a thread of its own, and only while the bars are being drawn (`listen` is asked for every frame of
//! them). A moment after the last ask the thread closes the device and sleeps, so nothing runs while no song plays or
//! while the notch is open.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Media::Audio::{
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX, eMultimedia,
    eRender,
};
use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree};

/// How many bands (bars) there are.
pub const BANDS: usize = 5;

/// The FFT looks at this many samples at a time (about 43 ms of sound) and at a new look every `HOP` samples.
const FFT_N: usize = 2048;
const HOP: usize = 1024;

/// The edges of the bands in hertz: bass, low middle, middle, high middle and treble.
const EDGES: [f32; BANDS + 1] = [45.0, 150.0, 450.0, 1400.0, 4200.0, 12000.0];
/// Added to each band's loudness, in decibels. Music has far more energy low down than high up, and without this the
/// bars on the right would hardly ever move.
const TILT_DB: [f32; BANDS] = [-2.0, 0.0, 2.0, 4.0, 6.0];

/// How many decibels the bars span, from the loudest band (full height) down to the quietest that still shows.
const RANGE_DB: f32 = 34.0;
/// The loudest band is never taken to be quieter than this, so a hiss or a very quiet room does not fill the bars.
const TOP_FLOOR_DB: f32 = -35.0;
/// How fast what counts as loud forgets a loud passage, decibels a second.
const TOP_FALL_DB: f32 = 4.0;
/// How long a bar takes to fall to about a third of its height once the sound stops.
const FALL_SECS: f32 = 0.12;
/// How much of the way to a louder reading a bar goes at once.
const RISE: f32 = 0.75;

/// The levels of the five bands, 0 to 1, made from a stream of mono samples.
pub struct Analyser {
    rate: f32,
    /// The last `FFT_N` samples, written round and round.
    ring: Vec<f32>,
    pos: usize,
    since: usize,
    window: Vec<f32>,
    /// cos and sin of -2 pi k / `FFT_N`.
    twiddle: Vec<(f32, f32)>,
    /// The first and last bin of each band.
    bins: [(usize, usize); BANDS],
    re: Vec<f32>,
    im: Vec<f32>,
    /// What the loudest band was lately, in decibels.
    top_db: f32,
    levels: [f32; BANDS],
}

impl Analyser {
    pub fn new(rate: u32) -> Self {
        let rate = rate.max(8000) as f32;
        let window = (0..FFT_N).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FFT_N as f32).cos()).collect();
        let twiddle = twiddles();
        let hz_per_bin = rate / FFT_N as f32;
        let mut bins = [(0, 0); BANDS];
        for (b, bin) in bins.iter_mut().enumerate() {
            let first = ((EDGES[b] / hz_per_bin).round() as usize).clamp(1, FFT_N / 2 - 1);
            let last = ((EDGES[b + 1] / hz_per_bin).round() as usize).clamp(first + 1, FFT_N / 2);
            *bin = (first, last);
        }
        Self { rate, ring: vec![0.0; FFT_N], pos: 0, since: 0, window, twiddle, bins, re: vec![0.0; FFT_N], im: vec![0.0; FFT_N], top_db: TOP_FLOOR_DB, levels: [0.0; BANDS] }
    }

    /// The levels now, 0 to 1.
    pub fn levels(&self) -> [f32; BANDS] {
        self.levels
    }

    /// Take in some mono samples (-1 to 1), looking at the sound again for every `HOP` of them.
    pub fn push(&mut self, samples: &[f32]) {
        for &s in samples {
            self.ring[self.pos] = if s.is_finite() { s.clamp(-1.0, 1.0) } else { 0.0 };
            self.pos = (self.pos + 1) % FFT_N;
            self.since += 1;
            if self.since >= HOP {
                self.since = 0;
                self.look();
            }
        }
    }

    /// `secs` went by with no sound at all: the bars fall.
    pub fn idle(&mut self, secs: f32) {
        let keep = (-secs / FALL_SECS).exp();
        self.levels.iter_mut().for_each(|l| *l *= keep);
        self.top_db = (self.top_db - TOP_FALL_DB * secs).max(TOP_FLOOR_DB);
    }

    /// One look: the last `FFT_N` samples through the FFT, the loudness of each band, and the bars' new heights.
    fn look(&mut self) {
        // The oldest sample is the one about to be written.
        for i in 0..FFT_N {
            self.re[i] = self.ring[(self.pos + i) % FFT_N] * self.window[i];
            self.im[i] = 0.0;
        }
        fft(&mut self.re, &mut self.im, &self.twiddle);
        // A full-scale sine comes out of the FFT at FFT_N / 4 with this window; that is taken as 1.
        let unit = FFT_N as f32 / 4.0;
        let mut db = [0.0f32; BANDS];
        for (b, &(first, last)) in self.bins.iter().enumerate() {
            let power: f32 = (first..last).map(|k| self.re[k] * self.re[k] + self.im[k] * self.im[k]).sum();
            db[b] = 20.0 * (power.sqrt() / unit + 1e-7).log10() + TILT_DB[b];
        }
        let dt = HOP as f32 / self.rate;
        let loudest = db.iter().cloned().fold(f32::MIN, f32::max);
        self.top_db = (self.top_db - TOP_FALL_DB * dt).max(loudest).max(TOP_FLOOR_DB);
        let keep = (-dt / FALL_SECS).exp();
        for (level, db) in self.levels.iter_mut().zip(db) {
            let now = ((db - (self.top_db - RANGE_DB)) / RANGE_DB).clamp(0.0, 1.0);
            *level = if now > *level { *level + (now - *level) * RISE } else { *level * keep };
        }
    }
}

/// cos and sin of -2 pi k / `FFT_N`, for the FFT.
fn twiddles() -> Vec<(f32, f32)> {
    (0..FFT_N / 2)
        .map(|k| {
            let a = -std::f64::consts::TAU * k as f64 / FFT_N as f64;
            (a.cos() as f32, a.sin() as f32)
        })
        .collect()
}

/// A plain radix-2 FFT of `re` and `im` (their length a power of two, `twiddle` made for it).
fn fft(re: &mut [f32], im: &mut [f32], twiddle: &[(f32, f32)]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let (half, step) = (len / 2, n / len);
        for start in (0..n).step_by(len) {
            for k in 0..half {
                let (cr, ci) = twiddle[k * step];
                let (a, b) = (start + k, start + k + half);
                let (tr, ti) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

/// How the device writes its samples.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Sample {
    F32,
    I16,
    I32,
}

/// What the format Windows mixes in is, if it is one that can be read: (sample rate, channels, sample kind, bytes a frame).
fn describe(format: &WAVEFORMATEX) -> Option<(u32, usize, Sample, usize)> {
    // The format is packed, so each field is copied out before it is looked at.
    let (tag, channels, rate, align, bits) = (format.wFormatTag, format.nChannels, format.nSamplesPerSec, format.nBlockAlign, format.wBitsPerSample);
    const IEEE_FLOAT: u16 = 3;
    const EXTENSIBLE: u16 = 0xFFFE;
    const PCM: u16 = 1;
    // The shared mix is float, but an extensible format says what its samples are in the first part of a GUID that
    // follows the usual fields: 3 is float, 1 is whole numbers, and anything else cannot be read.
    let float = match tag {
        IEEE_FLOAT => true,
        PCM => false,
        EXTENSIBLE => {
            // The extra part has to be all there (22 bytes) before the GUID in it is looked at.
            if { format.cbSize } < 22 {
                return None;
            }
            // SAFETY: an extensible format is a WAVEFORMATEXTENSIBLE, which starts with this structure, and `cbSize`
            // says Windows gave us the rest of it.
            let sub = unsafe { std::ptr::read_unaligned((format as *const WAVEFORMATEX as *const u8).add(24) as *const u32) };
            match sub {
                3 => true,
                1 => false,
                _ => return None,
            }
        }
        _ => return None,
    };
    let kind = match (bits, float) {
        (32, true) => Sample::F32,
        (16, false) => Sample::I16,
        (32, false) => Sample::I32,
        _ => return None,
    };
    (channels >= 1 && rate >= 8000 && align as usize >= channels as usize * (bits as usize / 8)).then_some((rate, channels as usize, kind, align as usize))
}

/// One mono sample from each frame of `bytes`: the channels added and divided.
fn to_mono(bytes: &[u8], channels: usize, kind: Sample, align: usize, out: &mut Vec<f32>) {
    let width = match kind {
        Sample::I16 => 2,
        _ => 4,
    };
    for frame in bytes.chunks_exact(align) {
        let mut sum = 0.0;
        for c in 0..channels {
            let b = &frame[c * width..(c + 1) * width];
            sum += match kind {
                Sample::F32 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                Sample::I16 => i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0,
                Sample::I32 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
            };
        }
        out.push(sum / channels as f32);
    }
}

/// The name Windows gives a device, which stays the same while it is the same device.
fn device_id(device: &IMMDevice) -> Option<String> {
    // SAFETY: the text Windows hands back is read once and then freed.
    unsafe {
        let id = device.GetId().ok()?;
        let text = id.to_string().ok();
        CoTaskMemFree(Some(id.0 as *const _));
        text
    }
}

/// The open device: the default one the PC plays on, listened to.
struct Capture {
    enumerator: IMMDeviceEnumerator,
    /// Which device it is, to notice when another one becomes the default.
    id: Option<String>,
    client: IAudioClient,
    reader: IAudioCaptureClient,
    rate: u32,
    channels: usize,
    kind: Sample,
    align: usize,
}

impl Capture {
    fn open() -> Option<Capture> {
        // SAFETY: plain COM and WASAPI calls on this thread; the format Windows hands back is freed after use.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia).ok()?;
            let id = device_id(&device);
            let client: IAudioClient = device.Activate(CLSCTX_ALL, None).ok()?;
            let format = client.GetMixFormat().ok()?;
            let described = describe(&*format);
            // 200 ms of room, in units of 100 ns.
            let started = described.and_then(|_| client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, 2_000_000, 0, format, None).ok());
            CoTaskMemFree(Some(format as *const _));
            let (rate, channels, kind, align) = described?;
            started?;
            let reader: IAudioCaptureClient = client.GetService().ok()?;
            client.Start().ok()?;
            Some(Capture { enumerator, id, client, reader, rate, channels, kind, align })
        }
    }

    /// Whether this is still the device the PC plays on. When another one becomes the default (headphones plugged in,
    /// another output picked) the old one keeps answering, only with nothing, so this has to be asked.
    fn still_default(&self) -> bool {
        let Some(id) = &self.id else { return true };
        // SAFETY: a plain COM call on the thread that made the enumerator.
        let now = unsafe { self.enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia).ok() };
        now.and_then(|d| device_id(&d)).is_none_or(|now| &now == id)
    }

    /// Take what has come in since the last time and add it, as mono samples, to `out`. `None` when the device went
    /// away (it was unplugged or switched off); a device that is only no longer the default is noticed by
    /// `still_default`.
    fn pump(&self, out: &mut Vec<f32>) -> Option<()> {
        // SAFETY: each buffer is read before it is given back, and only for as many frames as Windows says it holds.
        unsafe {
            loop {
                let frames = self.reader.GetNextPacketSize().ok()?;
                if frames == 0 {
                    return Some(());
                }
                let (mut data, mut got, mut flags) = (std::ptr::null_mut(), 0u32, 0u32);
                self.reader.GetBuffer(&mut data, &mut got, &mut flags, None, None).ok()?;
                const SILENT: u32 = 2;
                if flags & SILENT != 0 || data.is_null() {
                    out.extend(std::iter::repeat_n(0.0, got as usize));
                } else {
                    to_mono(std::slice::from_raw_parts(data, got as usize * self.align), self.channels, self.kind, self.align, out);
                }
                self.reader.ReleaseBuffer(got).ok()?;
            }
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        // SAFETY: stopping a stream that is running (or already stopped) is harmless.
        unsafe {
            let _ = self.client.Stop();
        }
    }
}

const STARTING: u8 = 0;
const LISTENING: u8 = 1;
const FAILED: u8 = 2;

/// How long the listening carries on after it was last asked for.
const GRACE_MS: u64 = 1_500;
/// How long it waits before trying again after the device could not be opened.
const RETRY: Duration = Duration::from_secs(10);
/// How often the device is looked at.
const POLL: Duration = Duration::from_millis(20);
/// How often it checks that the device is still the one the PC plays on.
const CHECK_DEFAULT: Duration = Duration::from_secs(1);

/// What the thread and the UI share.
struct Shared {
    /// The five levels, as the bits of f32 numbers.
    levels: [AtomicU32; BANDS],
    /// When it was last asked, in milliseconds since the program began (0: never).
    asked: AtomicU64,
    state: AtomicU8,
    /// Whether the thread is asleep because nobody asked (so it needs waking).
    asleep: AtomicBool,
    /// Whether it has listened at least once. After that, while it starts again, the bars stay flat (they are about to
    /// follow the sound) instead of jumping to the made-up beat for a moment.
    worked: AtomicBool,
}

static SHARED: Shared = Shared {
    levels: [const { AtomicU32::new(0) }; BANDS],
    asked: AtomicU64::new(0),
    state: AtomicU8::new(STARTING),
    asleep: AtomicBool::new(false),
    worked: AtomicBool::new(false),
};
static THREAD: OnceLock<thread::Thread> = OnceLock::new();
static CLOCK: OnceLock<Instant> = OnceLock::new();

fn now_ms() -> u64 {
    // One more than the time, so that "never asked" (0) is never a time.
    CLOCK.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

// `asked` and `asleep` are a handshake: each side writes one and then reads the other, and with anything weaker than
// SeqCst both could read the old value, so the thread would sleep with nobody to wake it.
fn wanted() -> bool {
    let asked = SHARED.asked.load(Ordering::SeqCst);
    asked != 0 && now_ms().saturating_sub(asked) <= GRACE_MS
}

/// Ask for the levels of the music (which starts the listening, or wakes it) and say what they are, 0 to 1 for each
/// band, or `None` when the sound of this PC cannot be listened to (and while it starts the very first time).
pub fn listen() -> Option<[f32; BANDS]> {
    SHARED.asked.store(now_ms(), Ordering::SeqCst);
    let thread = THREAD.get_or_init(|| thread::Builder::new().name("loopback".into()).spawn(run).map(|h| h.thread().clone()).unwrap_or_else(|_| thread::current()));
    if SHARED.asleep.swap(false, Ordering::SeqCst) {
        thread.unpark();
    }
    match SHARED.state.load(Ordering::Relaxed) {
        LISTENING => {}
        // Starting again: the levels are flat until the sound comes in.
        STARTING if SHARED.worked.load(Ordering::Relaxed) => {}
        _ => return None,
    }
    Some(std::array::from_fn(|i| f32::from_bits(SHARED.levels[i].load(Ordering::Relaxed))))
}

/// Open the device once and let it go at once, on a thread of its own. The first time Windows is asked to be listened to
/// it takes most of a second, and every time after that a few milliseconds, so this is done as soon as a song plays,
/// so that the bars do not begin with the made-up ones.
pub fn warm() {
    static DONE: AtomicBool = AtomicBool::new(false);
    if DONE.swap(true, Ordering::Relaxed) {
        return;
    }
    let _ = thread::Builder::new().name("loopback-warm".into()).spawn(|| drop(Capture::open()));
}

fn publish(levels: [f32; BANDS]) {
    levels.iter().zip(&SHARED.levels).for_each(|(l, slot)| slot.store(l.to_bits(), Ordering::Relaxed));
}

/// The thread: waits until the levels are wanted, listens while they are, and lets go of the device when they are not.
fn run() {
    loop {
        while !wanted() {
            SHARED.state.store(STARTING, Ordering::Relaxed);
            publish([0.0; BANDS]);
            SHARED.asleep.store(true, Ordering::SeqCst);
            // Asked just now? Then do not sleep after all.
            if wanted() {
                SHARED.asleep.store(false, Ordering::SeqCst);
                break;
            }
            thread::park();
        }
        let Some(capture) = Capture::open() else {
            SHARED.state.store(FAILED, Ordering::Relaxed);
            thread::park_timeout(RETRY);
            continue;
        };
        let mut analyser = Analyser::new(capture.rate);
        let mut samples = Vec::with_capacity(4096);
        let mut last = Instant::now();
        let mut checked = last;
        SHARED.state.store(LISTENING, Ordering::Relaxed);
        SHARED.worked.store(true, Ordering::Relaxed);
        while wanted() {
            samples.clear();
            if capture.pump(&mut samples).is_none() {
                break;
            }
            let now = Instant::now();
            // Another device became the default: listen to that one instead.
            if now.duration_since(checked) >= CHECK_DEFAULT {
                checked = now;
                if !capture.still_default() {
                    break;
                }
            }
            if samples.is_empty() {
                // Nothing is playing (the device sends nothing at all then).
                analyser.idle(now.duration_since(last).as_secs_f32());
            } else {
                analyser.push(&samples);
            }
            last = now;
            publish(analyser.levels());
            thread::sleep(POLL);
        }
        // Either nobody asks any more or the device went away: close it, and start again if it is still wanted.
        drop(capture);
        SHARED.state.store(STARTING, Ordering::Relaxed);
        publish([0.0; BANDS]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn sine(hz: f32, amplitude: f32, secs: f32) -> Vec<f32> {
        (0..(RATE as f32 * secs) as usize).map(|i| amplitude * (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin()).collect()
    }

    fn levels_of(samples: &[f32]) -> [f32; BANDS] {
        let mut a = Analyser::new(RATE);
        a.push(samples);
        a.levels()
    }

    fn loudest(levels: [f32; BANDS]) -> usize {
        (0..BANDS).max_by(|&a, &b| levels[a].total_cmp(&levels[b])).unwrap()
    }

    #[test]
    fn the_fft_puts_a_sine_in_its_own_bin() {
        let twiddle = twiddles();
        // Bin 64 is 1500 Hz at 48 kHz: a sine on exactly that comes out at FFT_N / 2 in that bin and nowhere else.
        let mut re: Vec<f32> = (0..FFT_N).map(|i| (std::f32::consts::TAU * 64.0 * i as f32 / FFT_N as f32).sin()).collect();
        let mut im = vec![0.0; FFT_N];
        fft(&mut re, &mut im, &twiddle);
        let mag = |k: usize| (re[k] * re[k] + im[k] * im[k]).sqrt();
        assert!((mag(64) - FFT_N as f32 / 2.0).abs() < 0.5, "{}", mag(64));
        assert!((0..FFT_N / 2).filter(|&k| k != 64).all(|k| mag(k) < 0.5));
    }

    #[test]
    fn each_pitch_lights_its_own_bar() {
        // A low note, a middle one and a high one: the bar for each is the highest.
        assert_eq!(loudest(levels_of(&sine(90.0, 0.5, 1.0))), 0, "bass");
        assert_eq!(loudest(levels_of(&sine(250.0, 0.5, 1.0))), 1);
        assert_eq!(loudest(levels_of(&sine(800.0, 0.5, 1.0))), 2);
        assert_eq!(loudest(levels_of(&sine(2500.0, 0.5, 1.0))), 3);
        assert_eq!(loudest(levels_of(&sine(8000.0, 0.5, 1.0))), 4, "treble");
        // A pure note leaves the far bars low.
        let bass = levels_of(&sine(90.0, 0.5, 1.0));
        assert!(bass[0] > 0.9 && bass[4] < 0.1, "{bass:?}");
    }

    #[test]
    fn a_quiet_song_moves_the_bars_as_much_as_a_loud_one() {
        let (loud, quiet) = (levels_of(&sine(250.0, 0.8, 1.0)), levels_of(&sine(250.0, 0.04, 1.0)));
        assert!(loud[1] > 0.9 && quiet[1] > 0.9, "{loud:?} {quiet:?}");
    }

    #[test]
    fn silence_and_nothing_at_all_leave_the_bars_flat() {
        assert_eq!(levels_of(&vec![0.0; 24_000]), [0.0; BANDS]);
        // A very quiet hiss is below what shows.
        let hiss: Vec<f32> = (0..24_000).map(|i| 0.0004 * ((i * 7919 % 2003) as f32 / 1001.0 - 1.0)).collect();
        assert!(levels_of(&hiss).iter().all(|&l| l < 0.2), "{:?}", levels_of(&hiss));
        // Not a number, and out of range, are taken as nothing and as full scale.
        let odd = levels_of(&[f32::NAN, f32::INFINITY, -9.0, 9.0].repeat(5000));
        assert!(odd.iter().all(|l| (0.0..=1.0).contains(l)), "{odd:?}");
    }

    #[test]
    fn the_bars_fall_when_the_sound_stops() {
        let mut a = Analyser::new(RATE);
        a.push(&sine(250.0, 0.5, 1.0));
        let before = a.levels()[1];
        assert!(before > 0.9);
        a.idle(0.12);
        assert!(a.levels()[1] < before * 0.45 && a.levels()[1] > 0.0);
        a.idle(1.5);
        assert!(a.levels()[1] < 0.001);
        // Sound that has stopped in the middle of the data falls too, as the zeros come in.
        let mut b = Analyser::new(RATE);
        b.push(&sine(250.0, 0.5, 1.0));
        b.push(&vec![0.0; 24_000]);
        assert!(b.levels()[1] < 0.05, "{:?}", b.levels());
    }

    #[test]
    fn the_bands_follow_the_sample_rate() {
        for rate in [44_100, 48_000, 96_000, 192_000] {
            let a = Analyser::new(rate);
            let mut last = 0;
            for &(first, last_bin) in &a.bins {
                assert!(first >= 1 && first < last_bin && last_bin <= FFT_N / 2, "{rate}: {:?}", a.bins);
                assert!(first >= last, "{rate}: the bands do not go back");
                last = first;
            }
        }
    }

    #[test]
    fn a_format_is_read_for_what_it_is() {
        let format = |tag: u16, channels: u16, bits: u16| WAVEFORMATEX {
            wFormatTag: tag,
            nChannels: channels,
            nSamplesPerSec: 48_000,
            nAvgBytesPerSec: 48_000 * channels as u32 * bits as u32 / 8,
            nBlockAlign: channels * bits / 8,
            wBitsPerSample: bits,
            cbSize: 0,
        };
        assert_eq!(describe(&format(3, 2, 32)), Some((48_000, 2, Sample::F32, 8)));
        assert_eq!(describe(&format(1, 2, 16)), Some((48_000, 2, Sample::I16, 4)));
        assert_eq!(describe(&format(1, 2, 32)), Some((48_000, 2, Sample::I32, 8)));
        assert_eq!(describe(&format(1, 2, 24)), None, "packed 24 bit is not read");
        assert_eq!(describe(&format(3, 0, 32)), None, "no channels");
        assert_eq!(describe(&format(2, 2, 16)), None, "a packed kind (ADPCM) is not read");
        assert_eq!(describe(&format(0xFFFE, 2, 32)), None, "an extensible format without its extra part is not looked into");
    }

    #[test]
    fn frames_become_one_mono_sample_each() {
        let mut out = Vec::new();
        let bytes: Vec<u8> = [0.5f32, -0.25, 1.0, 1.0].iter().flat_map(|f| f.to_le_bytes()).collect();
        to_mono(&bytes, 2, Sample::F32, 8, &mut out);
        assert_eq!(out, vec![0.125, 1.0]);
        out.clear();
        let bytes: Vec<u8> = [16384i16, 0, -32768, -32768].iter().flat_map(|v| v.to_le_bytes()).collect();
        to_mono(&bytes, 2, Sample::I16, 4, &mut out);
        assert_eq!(out, vec![0.25, -1.0]);
        // Half a frame at the end is left out.
        out.clear();
        to_mono(&bytes[..6], 2, Sample::I16, 4, &mut out);
        assert_eq!(out.len(), 1);
    }
}
