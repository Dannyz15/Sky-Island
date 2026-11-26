//! Clipboard history for text and images (screenshots). Kept in memory only, so it is gone when the app closes,
//! except for the text items the user pins: those stay at the top, are never forgotten, and are kept in
//! `pinned.json`. Also holds the Windows clipboard calls the app needs.

mod history;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(windows, target_os = "linux", test))]
mod pictures;
#[cfg(windows)]
mod win;

pub use history::{History, VISIBLE, preview, read_pinned, save_pinned};
#[cfg(target_os = "linux")]
pub use linux::{empty, set_content, set_enabled, spawn_watcher};
#[cfg(windows)]
pub use win::{empty, set_content, set_enabled, spawn_watcher};

#[cfg(not(any(windows, target_os = "linux")))]
pub fn spawn_watcher(_proxy: winit::event_loop::EventLoopProxy<crate::UserEvent>) {}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn set_enabled(_on: bool) {}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn set_content(_content: &Content) -> bool {
    false
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn empty() {}

/// Longer text is cut to this many characters.
const MAX_CHARS: usize = 5000;

#[derive(Clone)]
pub struct Image {
    /// The whole picture, PNG-compressed.
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Small preview, straight RGBA.
    pub thumb: Vec<u8>,
    pub thumb_w: u32,
    pub thumb_h: u32,
}

#[derive(Clone)]
pub enum Content {
    Text(String),
    Image(Image),
}

impl Content {
    fn same_as(&self, other: &Content) -> bool {
        match (self, other) {
            (Content::Text(a), Content::Text(b)) => a == b,
            (Content::Image(a), Content::Image(b)) => a.png == b.png,
            _ => false,
        }
    }

    fn image_bytes(&self) -> usize {
        match self {
            Content::Image(i) => i.png.len(),
            Content::Text(_) => 0,
        }
    }
}
