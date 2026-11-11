//! The list of what was copied: newest first, with pinned text on top, forgetting and scrolling.

use std::time::{Duration, Instant};

use super::{Content, MAX_CHARS};
use crate::store;

/// Entries kept, newest first (pinned ones come before them and are not counted against this).
pub const MAX_ITEMS: usize = 30;

/// How many items can be pinned.
pub const MAX_PINNED: usize = 10;
/// Rows visible at once in the clipboard view.
pub const VISIBLE: usize = 3;
/// Images are stored compressed (PNG). Once they add up to more than this, the oldest are dropped.
const MAX_IMAGE_BYTES: usize = 48 * 1024 * 1024;

pub struct History {
    pub items: Vec<Content>,
    /// When each entry was copied, and whether it is pinned, in the same order as `items`. Pinned entries (text only)
    /// are always the first ones.
    copied: Vec<Instant>,
    pinned: Vec<bool>,
    /// First visible row.
    pub scroll: usize,
}

impl History {
    pub fn new() -> Self {
        Self { items: Vec::new(), copied: Vec::new(), pinned: Vec::new(), scroll: 0 }
    }

    /// Add something copied, at the top. Blank text is ignored, and copying the same thing again moves
    /// it to the top instead of adding a second entry.
    pub fn add(&mut self, content: Content) -> bool {
        let content = match content {
            Content::Text(t) if t.trim().is_empty() => return false,
            Content::Text(t) => Content::Text(t.chars().take(MAX_CHARS).collect()),
            image => image,
        };
        if let Some(i) = self.items.iter().position(|c| c.same_as(&content)) {
            if self.pinned[i] {
                // Already pinned: it stays where it is.
                return true;
            }
            self.forget(i);
        }
        // Behind the pinned entries, at the top of the rest.
        let at = self.pinned_count();
        self.items.insert(at, content);
        self.copied.insert(at, Instant::now());
        self.pinned.insert(at, false);
        while self.items.len() - at > MAX_ITEMS {
            self.forget(self.items.len() - 1);
        }
        self.evict_old_images();
        self.scroll = 0;
        true
    }

    fn forget(&mut self, index: usize) {
        self.items.remove(index);
        self.copied.remove(index);
        self.pinned.remove(index);
    }

    pub fn pinned_count(&self) -> usize {
        self.pinned.iter().filter(|p| **p).count()
    }

    pub fn is_pinned(&self, index: usize) -> bool {
        self.pinned.get(index).copied().unwrap_or(false)
    }

    /// Pin an entry (it goes to the very top and is kept), or unpin it (it goes to the top of the rest, as if
    /// copied now). Only text can be pinned, and only `MAX_PINNED` at a time. Returns whether anything changed.
    pub fn toggle_pin(&mut self, index: usize) -> bool {
        let Some(Content::Text(_)) = self.items.get(index) else { return false };
        let was_pinned = self.pinned[index];
        if !was_pinned && self.pinned_count() >= MAX_PINNED {
            return false;
        }
        let item = self.items.remove(index);
        self.copied.remove(index);
        self.pinned.remove(index);
        let at = if was_pinned { self.pinned_count() } else { 0 };
        self.items.insert(at, item);
        self.copied.insert(at, Instant::now());
        self.pinned.insert(at, !was_pinned);
        self.clamp_scroll();
        true
    }

    /// The pinned text, in order, for `pinned.json`.
    pub fn pinned_text(&self) -> Vec<&str> {
        self.items
            .iter()
            .zip(&self.pinned)
            .filter_map(|(c, p)| match c {
                Content::Text(t) if *p => Some(t.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Put the texts kept from last time at the top as pinned entries (the first of them first).
    pub fn load_pinned(&mut self, texts: Vec<String>) {
        for text in texts.into_iter().rev() {
            if self.pinned_count() < MAX_PINNED && !text.trim().is_empty() {
                self.items.insert(0, Content::Text(text.chars().take(MAX_CHARS).collect()));
                self.copied.insert(0, Instant::now());
                self.pinned.insert(0, true);
            }
        }
    }

    /// Forget what was copied more than `keep` ago. Returns whether anything went.
    pub fn expire(&mut self, now: Instant, keep: Duration) -> bool {
        let before = self.items.len();
        let mut i = self.items.len();
        while i > 0 {
            i -= 1;
            if !self.pinned[i] && now.saturating_duration_since(self.copied[i]) >= keep {
                self.forget(i);
            }
        }
        self.clamp_scroll();
        self.items.len() != before
    }

    /// When the oldest entry is due to be forgotten.
    pub fn next_expiry(&self, keep: Duration) -> Option<Instant> {
        self.copied.iter().zip(&self.pinned).filter(|(_, pinned)| !**pinned).map(|(at, _)| *at).min().map(|at| at + keep)
    }

    /// Drop the oldest images while the stored pictures take too much memory. The newest entry stays.
    fn evict_old_images(&mut self) {
        let mut total: usize = self.items.iter().map(Content::image_bytes).sum();
        let mut i = self.items.len();
        let newest = self.pinned_count() + 1;
        while total > MAX_IMAGE_BYTES && i > newest {
            i -= 1;
            let size = self.items[i].image_bytes();
            if size > 0 {
                total -= size;
                self.forget(i);
            }
        }
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.items.len() {
            self.forget(index);
            self.clamp_scroll();
        }
    }

    /// Forget everything but the pinned entries.
    pub fn clear(&mut self) {
        let mut i = self.items.len();
        while i > 0 {
            i -= 1;
            if !self.pinned[i] {
                self.forget(i);
            }
        }
        self.scroll = 0;
    }

    pub fn scroll_by(&mut self, rows: isize) {
        self.scroll = (self.scroll as isize + rows).max(0) as usize;
        self.clamp_scroll();
    }

    fn clamp_scroll(&mut self) {
        self.scroll = self.scroll.min(self.items.len().saturating_sub(VISIBLE));
    }

    /// Entry index shown in a visible row, if there is one.
    pub fn index_at(&self, row: usize) -> Option<usize> {
        let i = self.scroll + row;
        (row < VISIBLE && i < self.items.len()).then_some(i)
    }
}

/// The pinned texts kept from last time.
pub fn read_pinned() -> Vec<String> {
    let Some(v) = store::path("pinned.json").and_then(|p| store::read_json(&p)) else { return Vec::new() };
    let Some(list) = v.as_array() else { return Vec::new() };
    list.iter().filter_map(|t| t.as_str()).filter(|t| !t.trim().is_empty()).take(MAX_PINNED).map(str::to_string).collect()
}

/// Keep the pinned texts for the next time (or the file is removed when there are none).
pub fn save_pinned(history: &History) {
    if cfg!(test) {
        return;
    }
    let Some(path) = store::path("pinned.json") else { return };
    let texts = history.pinned_text();
    if texts.is_empty() {
        let _ = std::fs::remove_file(path);
    } else {
        store::write(&path, &serde_json::json!(texts).to_string());
    }
}

/// One line for the list: line breaks and runs of spaces become a single space.
pub fn preview(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::super::Image;
    use super::*;

    fn text(s: &str) -> Content {
        Content::Text(s.to_string())
    }

    /// An entry that only matters for its size and identity.
    fn fake_image(bytes: usize, tag: u8) -> Content {
        Content::Image(Image { png: vec![tag; bytes], width: 2, height: 2, thumb: vec![0; 16], thumb_w: 2, thumb_h: 2 })
    }

    fn texts(h: &History) -> Vec<String> {
        h.items
            .iter()
            .map(|c| match c {
                Content::Text(t) => t.clone(),
                Content::Image(_) => "<image>".to_string(),
            })
            .collect()
    }

    #[test]
    fn a_pinned_text_goes_to_the_top_and_stays_there() {
        let mut h = History::new();
        for t in ["a", "b", "c"] {
            h.add(text(t));
        }
        assert_eq!(texts(&h), ["c", "b", "a"]);
        assert!(h.toggle_pin(2), "pin a");
        assert_eq!(texts(&h), ["a", "c", "b"]);
        assert!(h.is_pinned(0) && !h.is_pinned(1));
        h.add(text("d"));
        assert_eq!(texts(&h), ["a", "d", "c", "b"], "new copies go behind the pinned ones");
        h.add(text("a"));
        assert_eq!(texts(&h), ["a", "d", "c", "b"], "copying a pinned text again changes nothing");
        assert!(h.toggle_pin(0), "unpin a");
        assert_eq!(texts(&h), ["a", "d", "c", "b"], "it joins the others at the top");
        assert_eq!(h.pinned_count(), 0);
    }

    #[test]
    fn pinned_entries_are_not_forgotten_cleared_or_pushed_out() {
        let mut h = History::new();
        h.add(text("keep"));
        h.toggle_pin(0);
        for i in 0..MAX_ITEMS + 5 {
            h.add(text(&format!("n{i}")));
        }
        assert_eq!(h.items.len(), MAX_ITEMS + 1, "30 others and the pinned one");
        assert_eq!(texts(&h)[0], "keep");
        let later = Instant::now() + Duration::from_secs(3600);
        assert!(h.expire(later, Duration::from_secs(60)));
        assert_eq!(texts(&h), ["keep"], "everything but the pinned one was forgotten");
        assert!(h.next_expiry(Duration::from_secs(60)).is_none(), "nothing left to forget");
        h.add(text("x"));
        h.clear();
        assert_eq!(texts(&h), ["keep"], "Clear keeps the pinned ones");
        h.remove(0);
        assert!(h.items.is_empty(), "the cross removes a pinned one too");
    }

    #[test]
    fn only_text_can_be_pinned_and_only_so_many() {
        let mut h = History::new();
        h.add(fake_image(10, 1));
        assert!(!h.toggle_pin(0), "a picture cannot be pinned");
        for i in 0..MAX_PINNED + 2 {
            h.add(text(&format!("t{i}")));
            // The new copy is the first one behind the pinned ones.
            let newest = h.pinned_count();
            h.toggle_pin(newest);
        }
        assert_eq!(h.pinned_count(), MAX_PINNED);
        assert!(!h.toggle_pin(MAX_PINNED), "one more than the limit");
        assert!(!h.toggle_pin(99));
    }

    #[test]
    fn pinned_texts_come_back_in_their_order() {
        let mut h = History::new();
        h.load_pinned(vec!["one".into(), " ".into(), "two".into()]);
        assert_eq!(texts(&h), ["one", "two"]);
        assert_eq!(h.pinned_count(), 2);
        assert_eq!(h.pinned_text(), ["one", "two"]);
        h.add(text("new"));
        assert_eq!(texts(&h), ["one", "two", "new"]);
    }

    #[test]
    fn old_entries_are_forgotten_and_new_ones_stay() {
        let mut h = History::new();
        h.add(text("old"));
        let start = Instant::now();
        std::thread::sleep(Duration::from_millis(30));
        h.add(text("new"));
        assert_eq!(h.items.len(), h.copied.len());
        let keep = Duration::from_millis(25);
        // Long enough after the first copy, not yet after the second.
        assert!(h.expire(start + Duration::from_millis(26), keep), "the first one is old enough");
        assert_eq!(texts(&h), ["new"]);
        assert!(!h.expire(start + Duration::from_millis(26), keep), "nothing more to forget");
        assert!(h.next_expiry(keep).is_some());
        assert!(h.expire(start + Duration::from_secs(60), keep));
        assert!(h.items.is_empty() && h.copied.is_empty());
        assert_eq!(h.next_expiry(keep), None);
        // Removing, clearing and copying the same thing again keep the two lists in step.
        h.add(text("a"));
        h.add(text("b"));
        h.add(text("a"));
        assert_eq!((texts(&h), h.copied.len()), (vec!["a".to_string(), "b".to_string()], 2));
        h.remove(0);
        assert_eq!((h.items.len(), h.copied.len()), (1, 1));
        h.clear();
        assert_eq!((h.items.len(), h.copied.len()), (0, 0));
    }

    #[test]
    fn newest_first_and_copying_again_moves_to_top() {
        let mut h = History::new();
        h.add(text("one"));
        h.add(text("two"));
        h.add(text("one"));
        assert_eq!(texts(&h), vec!["one", "two"]);
    }

    #[test]
    fn blank_text_is_ignored_and_long_text_is_cut() {
        let mut h = History::new();
        assert!(!h.add(text("   \n\t ")));
        assert!(h.items.is_empty());
        assert!(h.add(text(&"x".repeat(MAX_CHARS + 100))));
        assert_eq!(texts(&h)[0].chars().count(), MAX_CHARS);
    }

    #[test]
    fn keeps_only_the_newest_entries() {
        let mut h = History::new();
        for i in 0..MAX_ITEMS + 5 {
            h.add(text(&format!("item {i}")));
        }
        assert_eq!(h.items.len(), MAX_ITEMS);
        assert_eq!(texts(&h)[0], format!("item {}", MAX_ITEMS + 4));
    }

    #[test]
    fn clear_removes_text_and_images() {
        let mut h = History::new();
        h.add(text("a"));
        h.add(fake_image(10, 1));
        h.scroll_by(1);
        h.clear();
        assert!(h.items.is_empty());
        assert_eq!(h.scroll, 0);
        assert!(h.index_at(0).is_none());
    }

    #[test]
    fn scroll_and_remove_stay_in_range() {
        let mut h = History::new();
        for i in 0..7 {
            h.add(text(&format!("t{i}")));
        }
        h.scroll_by(10);
        assert_eq!(h.scroll, 4, "7 entries, 3 visible: at most 4 rows of scroll");
        assert_eq!(h.index_at(0), Some(4));
        assert_eq!(h.index_at(VISIBLE), None);
        h.remove(6);
        h.remove(5);
        h.remove(4);
        h.remove(3);
        assert_eq!(h.scroll, 0);
    }

    #[test]
    fn preview_collapses_whitespace() {
        assert_eq!(preview("  hello \n\n   world\t!  "), "hello world !");
        assert_eq!(preview(""), "");
    }

    #[test]
    fn the_same_picture_moves_to_the_top_instead_of_repeating() {
        let mut h = History::new();
        h.add(fake_image(100, 1));
        h.add(text("between"));
        h.add(fake_image(100, 1));
        assert_eq!(texts(&h), vec!["<image>", "between"]);
    }

    #[test]
    fn old_pictures_are_dropped_when_they_use_too_much_memory() {
        let mut h = History::new();
        let big = MAX_IMAGE_BYTES / 2 + 1;
        h.add(fake_image(big, 1));
        h.add(text("keep me"));
        h.add(fake_image(big, 2)); // now over the limit: the oldest picture goes
        assert_eq!(texts(&h), vec!["<image>", "keep me"]);
        assert!(matches!(&h.items[0], Content::Image(i) if i.png[0] == 2));
    }

    #[test]
    fn one_huge_picture_is_still_kept() {
        let mut h = History::new();
        h.add(fake_image(MAX_IMAGE_BYTES + 10, 1));
        assert_eq!(h.items.len(), 1, "the newest entry is never evicted");
    }
}
