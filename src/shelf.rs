//! File shelf: a short list of file paths parked on the notch, saved between runs.
//! Only paths are stored; the files themselves are never copied or moved.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

const MAX_ITEMS: usize = 50;
/// Tiles per row and visible rows in the shelf view.
pub const COLS: usize = 5;
pub const ROWS: usize = 2;

pub struct Shelf {
    pub items: Vec<PathBuf>,
    /// First visible row.
    pub scroll: usize,
}

impl Shelf {
    pub fn new() -> Self {
        let items = Self::path()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| {
                v.as_array().map(|a| {
                    a.iter().filter_map(Value::as_str).map(PathBuf::from).filter(|p| p.exists()).collect()
                })
            })
            .unwrap_or_default();
        Self { items, scroll: 0 }
    }

    #[cfg(test)]
    fn empty() -> Self {
        Self { items: Vec::new(), scroll: 0 }
    }

    fn path() -> Option<PathBuf> {
        let dir = PathBuf::from(std::env::var_os("APPDATA")?).join("Sky-Island");
        fs::create_dir_all(&dir).ok()?;
        Some(dir.join("shelf.json"))
    }

    fn save(&self) {
        // Tests must not overwrite the user's real data.
        if cfg!(test) {
            return;
        }
        if let Some(p) = Self::path() {
            let paths: Vec<String> = self.items.iter().map(|p| p.to_string_lossy().into_owned()).collect();
            let _ = fs::write(p, json!(paths).to_string());
        }
    }

    /// Add a file, newest first. Duplicates move to the front; the oldest fall off the end.
    pub fn add(&mut self, path: PathBuf) {
        self.items.retain(|p| p != &path);
        self.items.insert(0, path);
        self.items.truncate(MAX_ITEMS);
        self.scroll = 0;
        self.save();
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

    /// Drop entries whose files no longer exist, e.g. after one was moved by a drag.
    pub fn prune(&mut self) {
        let before = self.items.len();
        self.items.retain(|p| p.exists());
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

/// Upper-case extension for the tile badge, or "FILE" / "DIR".
pub fn badge(path: &Path) -> String {
    if path.is_dir() {
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
    fn add_puts_newest_first_and_dedupes() {
        let mut s = Shelf::empty();
        s.add(p("a.txt"));
        s.add(p("b.txt"));
        s.add(p("a.txt"));
        assert_eq!(s.items, vec![p("a.txt"), p("b.txt")]);
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
    fn badge_uses_extension() {
        assert_eq!(badge(&p("photo.jpeg")), "JPEG");
        assert_eq!(badge(&p("archive.tar.gz")), "GZ");
        assert_eq!(badge(&p("README")), "FILE");
    }
}
