//! The light look: used when the theme is Light, or Follow System while Windows uses its light theme.
//!
//! The notch is drawn with a dark palette: white text and white see-through highlights on a black
//! body. In the light theme every colour that is drawn is passed through [`map`], which turns the
//! whites and greys into dark inks (so white text becomes dark text and a white highlight becomes a
//! dark one) and only darkens the coloured ones a little, so the accents stay recognisable on a light
//! body. Pictures such as album covers and file icons are not colours, so they stay as they are.

use std::cell::Cell;

thread_local! {
    static LIGHT: Cell<bool> = const { Cell::new(false) };
    static GLASS: Cell<bool> = const { Cell::new(false) };
}

/// Draw the controls on this thread as pieces of glass (the glass looks of the notch), or as before.
pub fn set_glass(on: bool) {
    GLASS.with(|g| g.set(on));
}

pub fn is_glass() -> bool {
    GLASS.with(Cell::get)
}

/// Switch the drawing on this thread to the light look (or back to dark).
pub fn set_light(on: bool) {
    LIGHT.with(|l| l.set(on));
}

pub fn is_light() -> bool {
    LIGHT.with(Cell::get)
}

/// The colour to really draw for a colour of the dark palette. Unchanged in the dark look.
pub fn map(c: [u8; 4]) -> [u8; 4] {
    if !is_light() {
        return c;
    }
    let [r, g, b, a] = c;
    let (hi, lo) = (r.max(g).max(b) as f32, r.min(g).min(b) as f32);
    // 1 for greys and whites, fading to 0 for strong colours.
    let neutral = (1.0 - (hi - lo) / 80.0).clamp(0.0, 1.0);
    let channel = |v: u8| {
        let flipped = (255.0 - v as f32) * 0.92 + 8.0;
        let kept = v as f32 * 0.8;
        (kept + (flipped - kept) * neutral).round().clamp(0.0, 255.0) as u8
    };
    [channel(r), channel(g), channel(b), a]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dark_look_changes_nothing() {
        set_light(false);
        assert_eq!(map([255, 255, 255, 200]), [255, 255, 255, 200]);
        assert_eq!(map([12, 200, 99, 7]), [12, 200, 99, 7]);
    }

    #[test]
    fn whites_and_greys_become_dark_inks_with_the_same_see_through() {
        set_light(true);
        let white = map([255, 255, 255, 90]);
        assert!(white[0] < 30 && white[3] == 90, "white text turns dark: {white:?}");
        let grey = map([165, 165, 175, 255]);
        assert!(grey[0] > 60 && grey[0] < 130, "grey stays a readable mid grey: {grey:?}");
        let dark = map([0, 0, 0, 255]);
        assert!(dark[0] > 230, "black turns light: {dark:?}");
        set_light(false);
    }

    #[test]
    fn strong_colours_stay_recognisable_and_only_darken() {
        set_light(true);
        let amber = map([255, 200, 90, 255]);
        assert!(amber[0] > amber[2] && amber[1] > amber[2], "still amber: {amber:?}");
        assert!(amber[0] < 255 && amber[0] > 150, "a little darker: {amber:?}");
        set_light(false);
    }
}
