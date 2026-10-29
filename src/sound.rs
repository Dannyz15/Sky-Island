//! Short chime sounds, synthesized in code and played straight from memory, so the app needs no
//! sound files. They replace the harsh system beep.

use std::f32::consts::TAU;
use std::collections::HashMap;
use std::sync::Mutex;

use std::path::Path;

use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
use windows::Win32::Media::Multimedia::mciSendStringW;
use windows::core::PCWSTR;

const RATE: u32 = 44_100;

#[derive(Clone, Copy)]
pub enum Chime {
    /// The timer ran out: two notes going up.
    TimerDone,
    /// A water or to-do reminder: one soft note.
    Reminder,
}

/// Play a chime without blocking, at `volume` percent. Does nothing if it can't be played.
pub fn play(chime: Chime, volume: u32) {
    // Each chime is made once per volume and kept for good, because it must stay valid while it plays
    // in the background. The volume moves in steps of 10, so there are at most twenty of them.
    type Cache = HashMap<(u8, u32), &'static [u8]>;
    static CACHE: Mutex<Option<Cache>> = Mutex::new(None);
    let volume = volume.clamp(10, 100) / 10 * 10;
    let key = (chime as u8, volume);
    #[allow(clippy::explicit_auto_deref)]
    let wav: &'static [u8] = {
        let Ok(mut cache) = CACHE.lock() else { return };
        let map = cache.get_or_insert_with(HashMap::new);
        *map.entry(key).or_insert_with(|| {
            // E5 and B5 for the pair, G5 for the reminder. Each is (frequency Hz, start s, length s).
            let data = match chime {
                Chime::TimerDone => wav(&[(659.3, 0.0, 0.9), (987.8, 0.16, 1.0)], 1.3, volume as f32 / 100.0),
                Chime::Reminder => wav(&[(784.0, 0.0, 0.9)], 1.0, volume as f32 / 100.0),
            };
            Box::leak(data.into_boxed_slice())
        })
    };
    // The buffer lives in a static, so it stays valid while the sound plays in the background.
    unsafe {
        let _ = PlaySoundW(PCWSTR(wav.as_ptr() as *const u16), None, SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
    }
}

/// Send one command to the system's media player (MCI), which plays WAV, MP3, WMA, AAC and more.
/// Returns whether it worked, and the text it answered with.
fn mci(command: &str) -> (bool, String) {
    let wide: Vec<u16> = command.encode_utf16().chain(Some(0)).collect();
    let mut answer = [0u16; 64];
    let code = unsafe { mciSendStringW(PCWSTR(wide.as_ptr()), Some(&mut answer), None) };
    let len = answer.iter().position(|&c| c == 0).unwrap_or(answer.len());
    (code == 0, String::from_utf16_lossy(&answer[..len]))
}

/// Play a sound file the user picked, without blocking, at `volume` percent. A file that is still
/// playing is left alone, so a long song is not restarted by every repeat of the alarm. Returns
/// whether it is playing; false means the file could not be played.
pub fn play_file(path: &Path, volume: u32) -> bool {
    const ALIAS: &str = "skyalarm";
    if mci(&format!("status {ALIAS} mode")).1 == "playing" {
        return true;
    }
    mci(&format!("close {ALIAS}"));
    // Quotes are not allowed in Windows file names, so the path can be quoted as it is.
    if !mci(&format!("open \"{}\" type mpegvideo alias {ALIAS}", path.display())).0 {
        return false;
    }
    // MCI's volume runs from 0 to 1000; not every kind of file supports it, which is fine.
    mci(&format!("setaudio {ALIAS} volume to {}", volume.clamp(0, 100) * 10));
    if mci(&format!("play {ALIAS} from 0")).0 {
        true
    } else {
        mci(&format!("close {ALIAS}"));
        false
    }
}

/// Stop the user's sound file if it is playing.
pub fn stop_file() {
    mci("close skyalarm");
}

/// A 16-bit mono WAV file of bell-like notes: a sine with two quieter overtones that fades out.
/// `loudness` is 1.0 for the normal level.
fn wav(notes: &[(f32, f32, f32)], total: f32, loudness: f32) -> Vec<u8> {
    let count = (total * RATE as f32) as usize;
    let mut samples = vec![0f32; count];
    for &(hz, start, len) in notes {
        for (i, out) in samples.iter_mut().enumerate() {
            let t = i as f32 / RATE as f32 - start;
            if t < 0.0 || t > len {
                continue;
            }
            // Quick attack so it doesn't click, then an exponential fade.
            let envelope = (1.0 - (-t * 600.0).exp()) * (-t * 5.0 / len).exp();
            let tone = (TAU * hz * t).sin() + 0.30 * (TAU * 2.0 * hz * t).sin() + 0.10 * (TAU * 3.0 * hz * t).sin();
            *out += tone * envelope;
        }
    }

    // Keep the loudest point comfortably below clipping.
    let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs())).max(f32::EPSILON);
    let gain = 0.45 * loudness / peak;

    let data_len = (count * 2) as u32;
    let mut out = Vec::with_capacity(44 + count * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes()); // bytes per second
    out.extend_from_slice(&2u16.to_le_bytes()); // bytes per frame
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s * gain * i16::MAX as f32) as i16).to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_has_a_valid_header_and_the_right_length() {
        let data = wav(&[(440.0, 0.0, 0.5)], 0.6, 1.0);
        let samples = (0.6 * RATE as f32) as usize;
        assert_eq!(&data[0..4], b"RIFF");
        assert_eq!(&data[8..16], b"WAVEfmt ");
        assert_eq!(&data[36..40], b"data");
        assert_eq!(data.len(), 44 + samples * 2);
        let riff_len = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
        assert_eq!(riff_len, data.len() - 8);
    }

    #[test]
    fn a_lower_volume_makes_a_quieter_wav() {
        let peak = |loudness: f32| {
            wav(&[(659.3, 0.0, 0.9)], 1.0, loudness)[44..]
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]).unsigned_abs())
                .max()
                .unwrap()
        };
        assert!((peak(0.5) as u32) < peak(1.0) as u32 * 6 / 10, "half the volume should be about half as loud");
        assert!(peak(0.1) > 500, "the quietest setting can still be heard");
    }

    #[test]
    fn a_missing_sound_file_is_refused_so_the_chime_is_used() {
        assert!(!play_file(Path::new(r"C:\no\such\sound.mp3"), 50));
    }

    /// Plays a real file, so it needs a sound card: run it by hand with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn a_real_sound_file_plays_and_stops() {
        let path = Path::new(r"C:\Windows\Media\tada.wav");
        assert!(play_file(path, 10));
        assert_eq!(mci("status skyalarm mode").1, "playing");
        assert!(play_file(path, 10), "a file that is still playing is left alone");
        stop_file();
        assert!(!mci("status skyalarm mode").0, "stopping closes it");
    }

    #[test]
    fn wav_is_audible_but_never_clips() {
        let data = wav(&[(659.3, 0.0, 0.9), (987.8, 0.16, 1.0)], 1.3, 1.0);
        let peak = data[44..]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]).unsigned_abs())
            .max()
            .unwrap();
        assert!(peak > 5_000, "too quiet: {peak}");
        assert!(peak < 20_000, "too loud: {peak}");
    }
}
