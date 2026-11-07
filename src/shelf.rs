//! File shelf: a short list of file paths parked on the notch, saved between runs.
//! Only paths are stored; the files themselves are never copied or moved.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use winit::event_loop::EventLoopProxy;

use crate::{UserEvent, store};

/// Whether a path may be on another computer: a `\\server\share` path, or a drive letter that is a network drive
/// or one that Windows cannot tell the kind of (a mapped drive whose server is gone reads as that). Such a path is
/// never looked at by the app on its own (only when the user opens or drags it): asking about it can wait a long
/// time for a server that is gone, and it makes Windows sign in to that server. (A removed USB stick reads the
/// same, so its files stay on the shelf until it is back.)
pub fn is_remote(path: &Path) -> bool {
    let text = path.as_os_str().to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        // A long path: `\\?\C:\...` is on this PC, `\\?\UNC\server\...` is a share.
        return rest.get(..4).is_some_and(|p| p.eq_ignore_ascii_case(r"UNC\")) || is_remote(Path::new(rest));
    }
    if text.starts_with(r"\\") || text.starts_with("//") {
        return true;
    }
    // 0 cannot be told, 1 no such root, 4 network.
    crate::sys::drive_type(path).is_some_and(|kind| matches!(kind, 0 | 1 | 4))
}

/// Look for files that are gone, on a thread of its own: a file on a network drive that cannot be reached can
/// take a long time to answer, and the app should not wait for it to start. The missing ones come back as
/// `UserEvent::ShelfMissing`.
pub fn check_in_background(paths: Vec<PathBuf>, proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || {
        // One at a time, each reported as soon as it is known, so a slow path does not hold back the others.
        for path in paths {
            if !is_remote(&path) && !path.exists() {
                let _ = proxy.send_event(UserEvent::ShelfMissing(vec![path]));
            }
        }
    });
}

const MAX_ITEMS: usize = 50;
/// Tiles per row and visible rows in the shelf view.
pub const COLS: usize = 5;
pub const ROWS: usize = 2;

pub struct Shelf {
    pub items: Vec<PathBuf>,
    /// First visible row.
    pub scroll: usize,
    /// Files were added and the list was not saved yet (see `add` and `flush`).
    dirty: bool,
}

impl Shelf {
    pub fn new() -> Self {
        let items =
            Self::path().and_then(|p| store::read_json(&p)).and_then(|v| v.as_array().map(|a| a.iter().filter_map(Value::as_str).map(PathBuf::from).collect())).unwrap_or_default();
        Self { items, scroll: 0, dirty: false }
    }

    #[cfg(test)]
    fn empty() -> Self {
        Self { items: Vec::new(), scroll: 0, dirty: false }
    }

    fn path() -> Option<PathBuf> {
        store::path("shelf.json")
    }

    fn save(&mut self) {
        self.dirty = false;
        // Tests must not overwrite the user's real data.
        if cfg!(test) {
            return;
        }
        if let Some(p) = Self::path() {
            let paths: Vec<String> = self.items.iter().map(|p| p.to_string_lossy().into_owned()).collect();
            store::write(&p, &json!(paths).to_string());
        }
    }

    /// Add a file, newest first. Duplicates move to the front; the oldest fall off the end. The list is not saved
    /// here: dropping several files at once delivers them one by one, and saving after each would write (and flush
    /// to disk) the same file once per file. The event loop calls `flush` when it has handled them all.
    pub fn add(&mut self, path: PathBuf) {
        self.items.retain(|p| p != &path);
        self.items.insert(0, path);
        self.items.truncate(MAX_ITEMS);
        self.scroll = 0;
        self.dirty = true;
    }

    /// Save the list if files were added since it was last saved (once, however many were added).
    pub fn flush(&mut self) {
        if self.dirty {
            self.save();
        }
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.items.len() {
            self.items.remove(index);
            self.clamp_scroll();
            self.save();
        }
    }

    /// Take every file off the shelf (the files themselves are untouched).
    pub fn clear(&mut self) {
        self.items.clear();
        self.scroll = 0;
        self.save();
    }

    /// Drop the entries a check found missing (see `check_in_background`).
    pub fn forget(&mut self, missing: &[PathBuf]) {
        let before = self.items.len();
        self.items.retain(|p| !missing.contains(p));
        if self.items.len() != before {
            self.clamp_scroll();
            self.save();
        }
    }

    /// Drop entries whose files no longer exist, e.g. after one was moved by a drag.
    pub fn prune(&mut self) {
        let before = self.items.len();
        // A file on another computer is kept: it is not asked about (see `is_remote`).
        self.items.retain(|p| is_remote(p) || p.exists());
        if self.items.len() != before {
            self.clamp_scroll();
            self.save();
        }
    }

    /// Scroll by whole rows; positive goes further down the list.
    pub fn scroll_by(&mut self, rows: isize) {
        self.scroll = (self.scroll as isize + rows).max(0) as usize;
        self.clamp_scroll();
    }

    fn clamp_scroll(&mut self) {
        let total_rows = self.items.len().div_ceil(COLS);
        self.scroll = self.scroll.min(total_rows.saturating_sub(ROWS));
    }

    /// Index into `items` for a visible slot (0 .. COLS * ROWS), if there is a file there.
    pub fn index_at(&self, slot: usize) -> Option<usize> {
        let i = self.scroll * COLS + slot;
        (slot < COLS * ROWS && i < self.items.len()).then_some(i)
    }
}

pub fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Upper-case extension for the tile badge, or "FILE" / "DIR". It is drawn every frame, so whether a path is a
/// folder is asked only once for each, and never for one on another computer.
pub fn badge(path: &Path) -> String {
    use std::time::{Duration, Instant};
    /// How long an answer is kept, and how many paths are remembered at most.
    const KEEP: Duration = Duration::from_secs(30);
    const MAX: usize = 200;
    thread_local! {
        static FOLDERS: std::cell::RefCell<std::collections::HashMap<PathBuf, (bool, Instant)>> = Default::default();
    }
    let folder = FOLDERS.with(|f| {
        let mut known = f.borrow_mut();
        // Found by reference first: nothing is allocated for a path that is already known.
        if let Some(&(folder, at)) = known.get(path)
            && at.elapsed() < KEEP
        {
            return folder;
        }
        if known.len() >= MAX {
            known.clear();
        }
        let folder = !is_remote(path) && path.is_dir();
        known.insert(path.to_path_buf(), (folder, Instant::now()));
        folder
    });
    if folder {
        return "DIR".into();
    }
    match path.extension().and_then(|e| e.to_str()) {
        Some(e) if !e.is_empty() => e.chars().take(4).collect::<String>().to_uppercase(),
        _ => "FILE".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn files_found_missing_leave_the_shelf_and_the_rest_stay() {
        let mut shelf = Shelf::empty();
        shelf.items = ["a", "b", "c"].map(PathBuf::from).to_vec();
        shelf.forget(&[PathBuf::from("b"), PathBuf::from("zzz")]);
        assert_eq!(shelf.items, ["a", "c"].map(PathBuf::from));
        shelf.forget(&[]);
        assert_eq!(shelf.items.len(), 2);
    }

    #[test]
    fn add_puts_newest_first_and_dedupes() {
        let mut s = Shelf::empty();
        s.add(p("a.txt"));
        s.add(p("b.txt"));
        s.add(p("a.txt"));
        assert_eq!(s.items, vec![p("a.txt"), p("b.txt")]);
    }

    #[test]
    fn files_added_one_by_one_are_saved_once_when_flushed() {
        let mut s = Shelf::empty();
        assert!(!s.dirty, "a fresh shelf has nothing to save");
        s.flush();
        for name in ["a.txt", "b.txt", "c.txt"] {
            s.add(p(name));
            assert!(s.dirty, "adding waits for the flush instead of saving each file");
        }
        s.flush();
        assert!(!s.dirty, "one flush saved them all");
        s.add(p("d.txt"));
        s.remove(0);
        assert!(!s.dirty, "any other change is saved at once, and that includes the files added before it");
        assert_eq!(s.items, vec![p("c.txt"), p("b.txt"), p("a.txt")]);
    }

    #[test]
    fn caps_the_list() {
        let mut s = Shelf::empty();
        for i in 0..MAX_ITEMS + 5 {
            s.add(p(&format!("f{i}.txt")));
        }
        assert_eq!(s.items.len(), MAX_ITEMS);
        assert_eq!(s.items[0], p(&format!("f{}.txt", MAX_ITEMS + 4)));
    }

    #[test]
    fn scroll_stays_in_range_and_slots_map_to_items() {
        let mut s = Shelf::empty();
        for i in 0..13 {
            s.add(p(&format!("f{i}.txt")));
        }
        // 13 items = 3 rows, 2 visible, so at most 1 row of scroll.
        s.scroll_by(5);
        assert_eq!(s.scroll, 1);
        s.scroll_by(-5);
        assert_eq!(s.scroll, 0);
        assert_eq!(s.index_at(0), Some(0));
        s.scroll_by(1);
        assert_eq!(s.index_at(0), Some(COLS));
        assert_eq!(s.index_at(COLS * ROWS - 1), None, "slot past the end of the list is empty");
    }

    #[test]
    fn clear_empties_the_shelf() {
        let mut s = Shelf::empty();
        s.add(p("a"));
        s.add(p("b"));
        s.clear();
        assert!(s.items.is_empty() && s.scroll == 0);
    }

    #[test]
    fn remove_clamps_scroll() {
        let mut s = Shelf::empty();
        for i in 0..11 {
            s.add(p(&format!("f{i}.txt")));
        }
        s.scroll_by(1);
        for _ in 0..6 {
            s.remove(0);
        }
        assert_eq!(s.scroll, 0);
    }

    #[test]
    fn a_path_on_another_computer_is_told_apart() {
        assert!(is_remote(&p(r"\\server\share\a.txt")));
        assert!(is_remote(&p("//server/share/a.txt")));
        assert!(is_remote(&p(r"\\?\UNC\server\share\a.txt")));
        assert!(!is_remote(&p(r"\\?\C:\Users\a.txt")));
        assert!(!is_remote(&p(r"C:\Windows\notepad.exe")));
        assert!(!is_remote(&p("relative.txt")));
    }

    #[test]
    fn badge_uses_extension() {
        assert_eq!(badge(&p("photo.jpeg")), "JPEG");
        assert_eq!(badge(&p("archive.tar.gz")), "GZ");
        assert_eq!(badge(&p("README")), "FILE");
    }
}
