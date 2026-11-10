//! The app's small JSON files in %APPDATA%\Sky-Island, read and written so that one bad moment (a crash,
//! a power cut, a file damaged by hand) cannot cost the user their data.

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

/// A file in the app's data folder, which is created when it is missing.
pub fn path(name: &str) -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("APPDATA")?).join("Sky-Island");
    fs::create_dir_all(&dir).ok()?;
    Some(dir.join(name))
}

/// A file another program holds open (Windows error 32, a sharing violation, or 33, a lock violation) is tried
/// again this many times, this far apart: a virus scanner or a backup tool lets go of it in a moment. Any other
/// error (no right to read it, a bad disk) will not clear, so it is not waited for.
const READ_TRIES: u32 = 5;
const READ_WAIT: std::time::Duration = std::time::Duration::from_millis(150);

fn is_locked(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(32 | 33))
}

/// Set when a file stayed locked through every try: the files after it are tried once, so a locked folder costs
/// the start of the app about a second and not a second for each file.
static GAVE_UP: AtomicBool = AtomicBool::new(false);

/// Files that could not be read at all. They are not overwritten blindly: `write` first keeps a copy of what is in
/// them.
static UNREAD: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// The JSON in a file. `None` when there is no file. A file that is damaged (it is read, but it is not
/// JSON) is not left in place, where the next save would overwrite it: it is renamed to `<name>.corrupt`
/// (the previous copy of that is replaced), so the user can still get to what was in it. A file that
/// cannot be read at all (an antivirus or a backup tool holds it at sign-in) is left alone and `None` is given,
/// after a few tries if it is locked; `write` keeps a copy of it before it is first replaced.
pub fn read_json(path: &Path) -> Option<Value> {
    let tries = if GAVE_UP.load(Ordering::Relaxed) { 1 } else { READ_TRIES };
    let mut tried = 0;
    let text = loop {
        tried += 1;
        match fs::read_to_string(path) {
            Ok(text) => break text,
            Err(e) if e.kind() == ErrorKind::NotFound => return None,
            // Not text at all: as damaged as a file can be.
            Err(e) if e.kind() == ErrorKind::InvalidData => break String::new(),
            Err(e) if is_locked(&e) && tried < tries => std::thread::sleep(READ_WAIT),
            Err(e) => {
                if is_locked(&e) {
                    GAVE_UP.store(true, Ordering::Relaxed);
                }
                if let Ok(mut unread) = UNREAD.lock()
                    && !unread.iter().any(|p| p == path)
                {
                    unread.push(path.to_path_buf());
                }
                return None;
            }
        }
    };
    let parsed = serde_json::from_str::<Value>(&text).ok();
    if parsed.is_none() {
        let mut kept = path.as_os_str().to_owned();
        kept.push(".corrupt");
        let _ = fs::rename(path, kept);
    }
    parsed
}

/// How many copies of one file are kept at most (`.unread`, `.unread2`, ...).
const UNREAD_COPIES: u32 = 5;

/// For a file that could not be read when the app started: keep a copy of it as `<name>.unread` before it is
/// replaced. A copy that is already there is never written over (it may be the only one of what the user had, while
/// the file now holds the defaults an earlier start saved): the next free name is used, and when all are taken the
/// oldest copies stay. Gives false when the copy cannot be made, so the file is not touched yet.
fn keep_unread(path: &Path) -> bool {
    let Ok(mut unread) = UNREAD.lock() else { return true };
    let Some(i) = unread.iter().position(|p| p == path) else { return true };
    if !path.exists() {
        unread.remove(i);
        return true;
    }
    for n in 1..=UNREAD_COPIES {
        let mut name = path.as_os_str().to_owned();
        name.push(if n == 1 { ".unread".to_string() } else { format!(".unread{n}") });
        let copy = PathBuf::from(name);
        if copy.exists() {
            continue;
        }
        if fs::copy(path, &copy).is_ok() {
            unread.remove(i);
            return true;
        }
        return false;
    }
    // Every name is taken: the oldest copies are the ones to keep.
    unread.remove(i);
    true
}

/// Writes that are in progress, one at a time, and the texts whose file was held by another program (the latest
/// text for each path). One lock for all of it means two threads never write the same file (or the same `.tmp`)
/// at once: the weather and prayer threads, the lyrics threads and the main thread all save through here.
static WRITING: Mutex<Vec<(PathBuf, String)>> = Mutex::new(Vec::new());
static RETRYING: AtomicBool = AtomicBool::new(false);

/// How often, and how many times, a text whose file was held is written again, on a thread of its own.
const RETRY_EVERY: std::time::Duration = if cfg!(test) { std::time::Duration::from_millis(50) } else { std::time::Duration::from_millis(500) };
const RETRY_TIMES: u32 = 40;

/// Whether a failed replace is because another program has the file open (a sharing violation, a lock violation, or
/// access denied, which is what Windows says when a file is renamed over while it is open).
fn is_busy(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(5 | 32 | 33))
}

/// Replace a file with `text` all at once: the text is written to a temporary file next to it and then
/// renamed over it, so the file is always either the old version or the new one, never half of one. If another
/// program holds the file at that moment (a virus scanner, a backup tool), the text is not dropped: it is written
/// again a moment later, for about twenty seconds, and a newer text for the same file replaces it.
pub fn write(path: &Path, text: &str) {
    let Ok(mut waiting) = WRITING.lock() else { return };
    if !keep_unread(path) {
        return;
    }
    match write_now(path, text) {
        Ok(()) => waiting.retain(|(p, _)| p != path),
        Err(e) if is_busy(&e) => {
            match waiting.iter_mut().find(|(p, _)| p == path) {
                Some(entry) => entry.1 = text.to_string(),
                None => waiting.push((path.to_path_buf(), text.to_string())),
            }
            drop(waiting);
            retry_later();
        }
        Err(_) => {}
    }
}

/// What the saving thread is told: a text to write, or a wish to know when everything told so far is on disk.
enum Later {
    Save(PathBuf, String),
    Flush(std::sync::mpsc::Sender<()>),
}

static LATER: std::sync::OnceLock<Option<std::sync::mpsc::Sender<Later>>> = std::sync::OnceLock::new();

/// The thread that writes what `write_later` is given. Texts that arrive while it writes are gathered and only the
/// newest text for each file is written, so a quick run of clicks costs a few writes and not one for each click.
fn later_sender() -> Option<&'static std::sync::mpsc::Sender<Later>> {
    LATER
        .get_or_init(|| {
            let (tx, rx) = std::sync::mpsc::channel::<Later>();
            std::thread::Builder::new()
                .name("store-later".into())
                .spawn(move || {
                    while let Ok(first) = rx.recv() {
                        let mut saves: Vec<(PathBuf, String)> = Vec::new();
                        let mut acks = Vec::new();
                        for msg in std::iter::once(first).chain(std::iter::from_fn(|| rx.try_recv().ok())) {
                            match msg {
                                Later::Save(path, text) => match saves.iter_mut().find(|(p, _)| *p == path) {
                                    Some(entry) => entry.1 = text,
                                    None => saves.push((path, text)),
                                },
                                Later::Flush(ack) => acks.push(ack),
                            }
                        }
                        for (path, text) in saves {
                            write(&path, &text);
                        }
                        for ack in acks {
                            let _ = ack.send(());
                        }
                    }
                })
                .ok()
                .map(|_| tx)
        })
        .as_ref()
}

/// Like `write`, but the file is written by another thread, so the caller (the settings window, on the thread that
/// draws) does not wait for the disk (a write that is flushed to disk takes 5 to 15 ms). `flush` waits for it.
pub fn write_later(path: &Path, text: &str) {
    match later_sender() {
        Some(tx) if tx.send(Later::Save(path.to_path_buf(), text.to_string())).is_ok() => {}
        _ => write(path, text),
    }
}

/// Wait (a few seconds at most) until every text given to `write_later` is on disk: done when the app quits.
pub fn flush() {
    let Some(Some(tx)) = LATER.get() else { return };
    let (ack, done) = std::sync::mpsc::channel();
    if tx.send(Later::Flush(ack)).is_ok() {
        let _ = done.recv_timeout(std::time::Duration::from_secs(3));
    }
}

fn write_now(path: &Path, text: &str) -> std::io::Result<()> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    let done = fs::File::create(&temp).and_then(|mut f| f.write_all(text.as_bytes()).and_then(|()| f.sync_all())).and_then(|()| fs::rename(&temp, path));
    if done.is_err() {
        let _ = fs::remove_file(&temp);
    }
    done
}

/// Start the thread that writes the waiting texts again (one thread at a time).
fn retry_later() {
    if RETRYING.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = std::thread::Builder::new().name("store-retry".into()).spawn(|| {
        for _ in 0..RETRY_TIMES {
            std::thread::sleep(RETRY_EVERY);
            let Ok(mut waiting) = WRITING.lock() else { break };
            waiting.retain(|(path, text)| write_now(path, text).is_err_and(|e| is_busy(&e)));
            if waiting.is_empty() {
                break;
            }
        }
        // Still held after all that: given up. The flag goes down under the lock, so a text that arrives now
        // starts a new thread.
        if let Ok(mut waiting) = WRITING.lock() {
            waiting.clear();
            RETRYING.store(false, Ordering::SeqCst);
        } else {
            RETRYING.store(false, Ordering::SeqCst);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sky-island-store-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_write_replaces_the_file_and_leaves_no_temporary_one() {
        let dir = scratch("write");
        let file = dir.join("a.json");
        write(&file, "[1]");
        write(&file, "[1,2]");
        assert_eq!(fs::read_to_string(&file).unwrap(), "[1,2]");
        assert!(!dir.join("a.json.tmp").exists());
        assert_eq!(read_json(&file).unwrap().as_array().unwrap().len(), 2);
    }

    #[test]
    fn a_later_write_is_on_disk_after_the_flush_and_only_the_newest_text_counts() {
        let dir = scratch("later");
        let file = dir.join("settings.json");
        for n in 0..50 {
            write_later(&file, &format!("[{n}]"));
        }
        flush();
        assert_eq!(fs::read_to_string(&file).unwrap(), "[49]");
        assert!(!dir.join("settings.json.tmp").exists());
    }

    #[test]
    fn a_missing_file_is_just_nothing() {
        let dir = scratch("missing");
        assert!(read_json(&dir.join("none.json")).is_none());
        assert!(fs::read_dir(&dir).unwrap().next().is_none(), "nothing is created or renamed");
    }

    #[test]
    fn a_damaged_file_is_kept_aside_instead_of_being_overwritten() {
        let dir = scratch("damaged");
        let file = dir.join("todo.json");
        fs::write(&file, "[{\"text\": \"buy milk\"").unwrap();
        assert!(read_json(&file).is_none());
        assert!(!file.exists(), "the next save writes a fresh file");
        assert_eq!(fs::read_to_string(dir.join("todo.json.corrupt")).unwrap(), "[{\"text\": \"buy milk\"", "what was in it is still there");
    }

    #[test]
    fn a_file_that_could_not_be_read_is_copied_before_it_is_replaced() {
        let dir = scratch("unread");
        let file = dir.join("settings.json");
        fs::write(&file, "{\"theme\": 1}").unwrap();
        UNREAD.lock().unwrap().push(file.clone());
        write(&file, "{}");
        assert_eq!(fs::read_to_string(dir.join("settings.json.unread")).unwrap(), "{\"theme\": 1}", "what the user had is kept");
        assert_eq!(fs::read_to_string(&file).unwrap(), "{}");
        write(&file, "[]");
        assert_eq!(fs::read_to_string(dir.join("settings.json.unread")).unwrap(), "{\"theme\": 1}", "the copy is made only once");
    }

    #[test]
    fn a_copy_that_is_already_there_is_never_written_over() {
        let dir = scratch("unread-twice");
        let file = dir.join("todo.json");
        fs::write(dir.join("todo.json.unread"), "the user's own").unwrap();
        fs::write(&file, "the defaults of the first start").unwrap();
        UNREAD.lock().unwrap().push(file.clone());
        write(&file, "[]");
        assert_eq!(fs::read_to_string(dir.join("todo.json.unread")).unwrap(), "the user's own");
        assert_eq!(fs::read_to_string(dir.join("todo.json.unread2")).unwrap(), "the defaults of the first start");
    }

    #[test]
    fn a_save_while_the_file_is_held_is_made_when_it_is_let_go() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = scratch("held");
        let file = dir.join("todo.json");
        fs::write(&file, "[1]").unwrap();
        // Another program has the file open and shares nothing, as a scanner can.
        let hold = fs::OpenOptions::new().read(true).share_mode(0).open(&file).unwrap();
        write(&file, "[1,2]");
        assert_eq!(fs::read_to_string(dir.join("todo.json.tmp")).ok(), None, "no temporary file is left behind");
        write(&file, "[1,2,3]");
        drop(hold);
        for _ in 0..100 {
            if fs::read_to_string(&file).unwrap() == "[1,2,3]" {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!("the newest text was never written: {}", fs::read_to_string(&file).unwrap());
    }

    #[test]
    fn many_threads_saving_one_file_leave_a_whole_file() {
        let dir = scratch("threads");
        let file = dir.join("prayer.json");
        let threads: Vec<_> = (0..8)
            .map(|n| {
                let file = file.clone();
                std::thread::spawn(move || {
                    for i in 0..20 {
                        write(&file, &format!("{{\"n\": {n}, \"i\": {i}, \"pad\": \"{}\"}}", "x".repeat(2000)));
                    }
                })
            })
            .collect();
        threads.into_iter().for_each(|t| t.join().unwrap());
        assert!(read_json(&file).is_some(), "the file is whole JSON");
        assert!(!dir.join("prayer.json.tmp").exists());
    }

    #[test]
    fn only_a_held_file_is_waited_for() {
        assert!(is_locked(&std::io::Error::from_raw_os_error(32)));
        assert!(is_locked(&std::io::Error::from_raw_os_error(33)));
        assert!(!is_locked(&std::io::Error::from_raw_os_error(5)), "access denied will not clear");
    }
}
