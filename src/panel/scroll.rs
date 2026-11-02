//! Scrolling in the settings window: how far a page can scroll, the scroll bar (drawing geometry and dragging) and the keys.

use super::*;

/// How far a page can scroll in a window `height` logical pixels tall: 0 when it all fits.
pub(super) fn scroll_limit(lay: &Layout, height: f32) -> f32 {
    (lay.bottom() + BOTTOM_PAD - height).max(0.0)
}

/// A scroll bar: its track is `height` logical pixels tall, from `top`, at `x`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Bar {
    pub(super) x: f32,
    pub(super) top: f32,
    pub(super) height: f32,
}

impl Bar {
    /// How long the thumb is for a page that scrolls `max`: the share of the page that shows, but never tiny.
    pub(super) fn thumb(&self, max: f32) -> f32 {
        (self.height * self.height / (self.height + max)).clamp(26.0_f32.min(self.height), self.height)
    }

    pub(super) fn thumb_top(&self, scroll: f32, max: f32) -> f32 {
        self.top + (self.height - self.thumb(max)) * (scroll / max).clamp(0.0, 1.0)
    }

    /// How far the page is scrolled when the thumb's top is at `thumb_top`.
    pub(super) fn scroll_at(&self, thumb_top: f32, max: f32) -> f32 {
        let room = self.height - self.thumb(max);
        if room <= 0.0 { 0.0 } else { ((thumb_top - self.top) / room).clamp(0.0, 1.0) * max }
    }

    /// Whether the mouse is on the bar or close enough to take hold of it.
    pub(super) fn grabs(&self, x: f32, y: f32) -> bool {
        x >= self.x - 8.0 && x <= self.x + 11.0 && y >= self.top && y <= self.top + self.height
    }
}

/// The bar of a page in a window `width` x `height` logical pixels.
pub(super) fn page_bar(width: f32, height: f32) -> Bar {
    Bar { x: width - 12.0, top: SCROLL_TOP + 6.0, height: height - SCROLL_TOP - 18.0 }
}

/// The bar inside the changelog's card.
pub(super) fn changelog_bar(frame: Frame, height: f32) -> Bar {
    let card_h = height - CHANGELOG_TOP - 18.0;
    Bar { x: frame.x + frame.w - 9.0, top: CHANGELOG_TOP + 14.0, height: card_h - 28.0 }
}

/// Where a scroll bar is being dragged: which one, and how far below the top of the thumb the mouse took hold.
#[derive(Clone, Copy)]
pub(super) struct BarDrag {
    pub(super) changelog: bool,
    pub(super) grab: f32,
}

/// Which keys scroll a page.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum ScrollKey {
    PageUp,
    PageDown,
    Home,
    End,
}

/// Where a page is scrolled to after a key, when `view` is the height that shows and `max` the furthest it goes.
/// A page key moves by the view less a few lines, so the line the eye was on stays in sight.
pub(super) fn scrolled_by_key(pos: f32, max: f32, view: f32, key: ScrollKey) -> f32 {
    let page = (view - 40.0).max(40.0);
    let to = match key {
        ScrollKey::PageUp => pos - page,
        ScrollKey::PageDown => pos + page,
        ScrollKey::Home => 0.0,
        ScrollKey::End => max,
    };
    to.clamp(0.0, max)
}

/// A white shape on the tile, drawn the same in the dark and the light look.
/// A thin scroll bar: a track and a thumb for how far `scroll` is of `max`. It is wider while the mouse is
/// on it (`hot`) so it is easy to see that it can be taken hold of.
pub(super) fn paint_scrollbar(pm: &mut Pixmap, bar: Bar, scroll: f32, max: f32, accent: [u8; 4], s: f32, hot: bool) {
    if max <= 0.0 || bar.height <= 0.0 {
        return;
    }
    let (width, track_alpha) = if hot { (6.0, 30) } else { (3.0, 18) };
    let x = bar.x + 3.0 - width;
    crate::fill_rounded(pm, x * s, bar.top * s, width * s, bar.height * s, width / 2.0 * s, [255, 255, 255, track_alpha]);
    crate::fill_rounded(pm, x * s, bar.thumb_top(scroll, max) * s, width * s, bar.thumb(max) * s, width / 2.0 * s, accent);
}
