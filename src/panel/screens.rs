//! The screen picker on the Notch page: a small map of the screens that are connected right now, drawn where they sit
//! in Windows' display layout, with a number on each. Click one to put the notch on it; the two tiles at the right are
//! All (the notch follows the mouse) and Every (a closed notch on each screen, the one the mouse touches opens). The map is made from what Windows reports each time it is drawn, so a screen
//! that is plugged in or taken out shows up (or goes) by itself, and there is no choice for a screen that is not there.

use super::*;

/// A connected screen, as the picker shows it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Detected {
    /// Its number in the picker: 1 is the first screen when they are put in order of their device numbers.
    pub order: u32,
    /// The number at the end of Windows' device name (`DISPLAY5`).
    pub device: u32,
    /// Where it is and how big, in pixels.
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    /// Whether it is the main screen.
    pub primary: bool,
}

/// A screen as Windows reports it: device number, position, size and whether it is the main one.
pub(super) type Raw = (u32, i32, i32, u32, u32, bool);

/// The picker's control: the map of the screens, then the All and Every tiles.
pub(super) const SCREEN_MAP: (f32, f32) = (300.0, 84.0);
/// The width of the two tiles at the right (All above, Every below), and the room between them and the map.
const ALL_W: f32 = 66.0;
const GAP: f32 = 8.0;
/// The room between the two tiles.
const STACK_GAP: f32 = 6.0;
/// Room left round the screens inside the map.
const PAD: f32 = 4.0;

/// Number the screens: they are put in order of their device numbers and counted from 1. (`raw` is each screen's
/// device number, position, size and whether it is the main one.) This is the order `Show on` uses for "Screen 2" and
/// the rest, so the number on a tile is the one that choosing it saves.
pub(super) fn arrange(raw: &[Raw]) -> Vec<Detected> {
    let mut sorted = raw.to_vec();
    sorted.sort_by_key(|r| r.0);
    sorted.dedup_by_key(|r| r.0);
    sorted.iter().enumerate().map(|(i, &(device, x, y, w, h, primary))| Detected { order: i as u32 + 1, device, x, y, w, h, primary }).collect()
}

/// What choosing this screen saves: the main screen is always Main, the others are counted.
pub(super) fn setting_for(d: &Detected) -> Screen {
    if d.primary { Screen::Main } else { Screen::Display(d.order.clamp(1, Screen::MAX_DISPLAY)) }
}

/// Whether the notch is on this screen according to the setting (All lights the All tile, not one screen).
pub(super) fn is_chosen(d: &Detected, setting: Screen) -> bool {
    match setting {
        Screen::Main => d.primary,
        Screen::Display(n) => d.order == n,
        Screen::All | Screen::Every => false,
    }
}

/// The area the screens are drawn in.
fn map_area(ctrl: Rect) -> Rect {
    (ctrl.0, ctrl.1, ctrl.2 - ALL_W - GAP, ctrl.3)
}

/// The All tile (the upper one of the two at the right).
pub(super) fn all_tile(ctrl: Rect) -> Rect {
    (ctrl.0 + ctrl.2 - ALL_W, ctrl.1, ALL_W, (ctrl.3 - STACK_GAP) / 2.0)
}

/// The Every tile (the lower one).
pub(super) fn every_tile(ctrl: Rect) -> Rect {
    let upper = all_tile(ctrl);
    (upper.0, upper.1 + upper.3 + STACK_GAP, ALL_W, upper.3)
}

/// Where each screen is drawn: all of them scaled by the same amount to fill the map area, side by side or above one
/// another as they are in Windows, and each a little smaller than its place so there is a line between neighbours.
pub(super) fn tiles(ctrl: Rect, screens: &[Detected]) -> Vec<Rect> {
    let Some(first) = screens.first() else { return Vec::new() };
    let (mut left, mut top, mut right, mut bottom) = (first.x, first.y, first.x + first.w as i32, first.y + first.h as i32);
    for d in screens {
        left = left.min(d.x);
        top = top.min(d.y);
        right = right.max(d.x + d.w as i32);
        bottom = bottom.max(d.y + d.h as i32);
    }
    let area = map_area(ctrl);
    let (room_w, room_h) = (area.2 - 2.0 * PAD, area.3 - 2.0 * PAD);
    let (all_w, all_h) = ((right - left).max(1) as f32, (bottom - top).max(1) as f32);
    let scale = (room_w / all_w).min(room_h / all_h);
    // The whole layout sits in the middle of the area.
    let (ox, oy) = (area.0 + (area.2 - all_w * scale) / 2.0, area.1 + (area.3 - all_h * scale) / 2.0);
    screens
        .iter()
        .map(|d| {
            let (x, y) = (ox + (d.x - left) as f32 * scale, oy + (d.y - top) as f32 * scale);
            let (w, h) = (d.w as f32 * scale, d.h as f32 * scale);
            (x + 1.5, y + 1.5, (w - 3.0).max(6.0), (h - 3.0).max(6.0))
        })
        .collect()
}

/// What is under (`x`, `y`): the choice it would save.
pub(super) fn pick_at(ctrl: Rect, screens: &[Detected], x: f32, y: f32) -> Option<Screen> {
    // All and Every are only offered when there is more than one screen to go between.
    if screens.len() > 1 {
        if inside(all_tile(ctrl), x, y) {
            return Some(Screen::All);
        }
        if inside(every_tile(ctrl), x, y) {
            return Some(Screen::Every);
        }
    }
    tiles(ctrl, screens).iter().zip(screens).find(|(t, _)| inside(**t, x, y)).map(|(_, d)| setting_for(d))
}

impl App {
    /// The screens that are connected now, from the island's window (empty before there is one).
    pub(super) fn detected_screens(&self) -> Vec<Detected> {
        let Some(window) = &self.window else { return Vec::new() };
        let main = window.primary_monitor().map(|m| (m.position().x, m.position().y));
        let raw: Vec<_> = window
            .available_monitors()
            .enumerate()
            .map(|(i, m)| {
                let device = m.name().as_deref().and_then(crate::ui::display_number).unwrap_or(1000 + i as u32);
                let (p, z) = (m.position(), m.size());
                (device, p.x, p.y, z.width, z.height, main == Some((p.x, p.y)))
            })
            .collect();
        arrange(&raw)
    }

    /// Choose where the notch is from the picker.
    pub(super) fn set_screen(&mut self, screen: Screen) {
        if self.settings.screen == screen {
            return;
        }
        self.settings.screen = screen;
        // The screen the mouse had last touched (for All) does not carry over, and the notch goes to its new screen
        // now, not at the next frame that happens to come: the place and the refresh rate are read afresh.
        self.vis.follow = None;
        self.pacing.rate_read = None;
        self.vis.monitor_area = None;
        self.settings_changed();
        self.step();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two() -> Vec<Detected> {
        // The main screen at 125 percent, and a 1080p screen to its left and 50 pixels lower (named DISPLAY5).
        arrange(&[(5, -1920, 50, 1920, 1080, false), (1, 0, 0, 1920, 1200, true)])
    }

    #[test]
    fn the_screens_are_numbered_in_order_of_their_device_numbers() {
        let s = two();
        assert_eq!((s[0].device, s[0].order, s[0].primary), (1, 1, true));
        assert_eq!((s[1].device, s[1].order, s[1].primary), (5, 2, false));
        assert!(arrange(&[]).is_empty());
        assert_eq!(arrange(&[(3, 0, 0, 10, 10, true), (3, 0, 0, 10, 10, true)]).len(), 1, "a screen listed twice is one");
    }

    #[test]
    fn choosing_a_screen_saves_main_or_its_number() {
        let s = two();
        assert_eq!(setting_for(&s[0]), Screen::Main);
        assert_eq!(setting_for(&s[1]), Screen::Display(2));
        // A screen that comes before the main one in the order is the first, not the second.
        let before = arrange(&[(3, 0, 0, 1920, 1080, true), (1, -1920, 0, 1920, 1080, false)]);
        assert_eq!(setting_for(&before[0]), Screen::Display(1));
        assert!(is_chosen(&before[0], Screen::Display(1)) && !is_chosen(&before[1], Screen::Display(1)));
        assert!(is_chosen(&s[0], Screen::Main) && !is_chosen(&s[1], Screen::Main));
        assert!(is_chosen(&s[1], Screen::Display(2)) && !is_chosen(&s[0], Screen::Display(2)));
        assert!(!is_chosen(&s[0], Screen::All) && !is_chosen(&s[1], Screen::All));
        assert!(!is_chosen(&s[0], Screen::Every) && !is_chosen(&s[1], Screen::Every));
    }

    #[test]
    fn the_map_keeps_the_screens_where_they_are_and_inside_the_control() {
        let ctrl = (100.0, 20.0, SCREEN_MAP.0, SCREEN_MAP.1);
        let s = two();
        let t = tiles(ctrl, &s);
        assert_eq!(t.len(), 2);
        // The second screen is to the left of the main one, and starts lower.
        assert!(t[1].0 + t[1].2 <= t[0].0, "left of the main screen");
        assert!(t[1].1 > t[0].1 || t[0].1 > t[1].1 - 1.0, "not on the same line: they are 50 pixels apart");
        let area = map_area(ctrl);
        for r in &t {
            assert!(r.0 >= area.0 && r.0 + r.2 <= area.0 + area.2 && r.1 >= area.1 && r.1 + r.3 <= area.1 + area.3, "{r:?} is inside {area:?}");
        }
        // Nothing overlaps the All and Every tiles, and the two do not overlap each other or leave the control.
        let (all, every) = (all_tile(ctrl), every_tile(ctrl));
        assert!(t.iter().all(|r| r.0 + r.2 <= all.0));
        assert!(all.1 + all.3 <= every.1, "All is above Every");
        assert!(every.1 + every.3 <= ctrl.1 + ctrl.3 + 0.01 && all.0 + all.2 <= ctrl.0 + ctrl.2 + 0.01);
        assert!(all.3 >= 30.0, "tall enough for two lines of text: {all:?}");
    }

    #[test]
    fn a_click_picks_the_screen_or_all() {
        let ctrl = (100.0, 20.0, SCREEN_MAP.0, SCREEN_MAP.1);
        let s = two();
        let t = tiles(ctrl, &s);
        let mid = |r: Rect| (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0);
        let (x, y) = mid(t[0]);
        assert_eq!(pick_at(ctrl, &s, x, y), Some(Screen::Main));
        let (x, y) = mid(t[1]);
        assert_eq!(pick_at(ctrl, &s, x, y), Some(Screen::Display(2)));
        let (x, y) = mid(all_tile(ctrl));
        assert_eq!(pick_at(ctrl, &s, x, y), Some(Screen::All));
        let (x, y) = mid(every_tile(ctrl));
        assert_eq!(pick_at(ctrl, &s, x, y), Some(Screen::Every));
        assert_eq!(pick_at(ctrl, &s, ctrl.0 - 5.0, ctrl.1 - 5.0), None);
        // With one screen there is nothing to follow between, so the All tile is not there to click.
        let (x, y) = mid(all_tile(ctrl));
        assert_eq!(pick_at(ctrl, &s[..1], x, y), None);
        let (x, y) = mid(every_tile(ctrl));
        assert_eq!(pick_at(ctrl, &s[..1], x, y), None);
    }

    #[test]
    fn four_screens_in_a_row_and_a_single_screen_both_fit() {
        let ctrl = (0.0, 0.0, SCREEN_MAP.0, SCREEN_MAP.1);
        let row: Vec<_> = (0..4).map(|i| (i as u32 + 1, i * 1920, 0, 1920, 1080, i == 0)).collect();
        let t = tiles(ctrl, &arrange(&row));
        let area = map_area(ctrl);
        assert!(t.iter().all(|r| r.0 >= area.0 && r.0 + r.2 <= area.0 + area.2 && r.2 >= 6.0));
        let one = tiles(ctrl, &arrange(&[(1, 0, 0, 1920, 1200, true)]));
        assert_eq!(one.len(), 1);
        assert!(one[0].2 > 40.0, "a lone screen is big enough to show its number: {:?}", one[0]);
    }

    /// Three or four screens as people arrange them: in a row, in a grid, one above two, mixed sizes.
    fn layouts() -> Vec<(&'static str, Vec<Raw>)> {
        vec![
            ("three in a row", vec![(1, 0, 0, 1920, 1080, true), (2, 1920, 0, 1920, 1080, false), (3, 3840, 0, 1920, 1080, false)]),
            ("four in a row", (0..4).map(|i| (i as u32 + 1, i * 1920, 0, 1920, 1080, i == 0)).collect()),
            ("two by two", vec![(1, 0, 0, 1920, 1080, true), (2, 1920, 0, 1920, 1080, false), (3, 0, 1080, 1920, 1080, false), (4, 1920, 1080, 1920, 1080, false)]),
            ("one above two", vec![(1, 960, 0, 1920, 1080, true), (2, 0, 1080, 1920, 1080, false), (3, 1920, 1080, 1920, 1080, false)]),
            ("laptop with two tall ones", vec![(1, 0, 0, 1920, 1200, true), (2, -1080, -400, 1080, 1920, false), (3, 1920, -400, 1080, 1920, false)]),
            (
                "names that are not in order",
                vec![(7, 0, 0, 2560, 1440, true), (2, -1920, 100, 1920, 1080, false), (12, 2560, 0, 1920, 1080, false), (4, 0, -1080, 1920, 1080, false)],
            ),
        ]
    }

    #[test]
    fn three_or_four_screens_fit_the_map_and_do_not_overlap() {
        let ctrl = (50.0, 10.0, SCREEN_MAP.0, SCREEN_MAP.1);
        let area = map_area(ctrl);
        for (name, raw) in layouts() {
            let screens = arrange(&raw);
            assert_eq!(screens.len(), raw.len(), "{name}");
            let t = tiles(ctrl, &screens);
            for (i, r) in t.iter().enumerate() {
                assert!(r.0 >= area.0 && r.0 + r.2 <= area.0 + area.2 + 0.01 && r.1 >= area.1 && r.1 + r.3 <= area.1 + area.3 + 0.01, "{name}: {r:?} is inside {area:?}");
                assert!(r.2 >= 24.0 && r.3 >= 12.0, "{name}: tile {} is big enough to click: {r:?}", i + 1);
                for (j, o) in t.iter().enumerate().skip(i + 1) {
                    let apart = r.0 + r.2 <= o.0 || o.0 + o.2 <= r.0 || r.1 + r.3 <= o.1 || o.1 + o.3 <= r.1;
                    assert!(apart, "{name}: tiles {} and {} overlap: {r:?} {o:?}", i + 1, j + 1);
                }
            }
            // Every tile is found by a click in its middle, and saves its own number (the main one saves Main).
            for (r, d) in t.iter().zip(&screens) {
                assert_eq!(pick_at(ctrl, &screens, r.0 + r.2 / 2.0, r.1 + r.3 / 2.0), Some(setting_for(d)), "{name}");
            }
            let saved: Vec<_> = screens.iter().map(setting_for).collect();
            assert_eq!(saved.iter().filter(|s| **s == Screen::Main).count(), 1, "{name}: one main screen");
            assert!(saved.iter().all(|s| s.display_number().is_none_or(|n| (1..=Screen::MAX_DISPLAY).contains(&n))), "{name}");
        }
    }

    #[test]
    fn every_screen_of_a_big_set_can_be_chosen_by_its_number() {
        for (name, raw) in layouts() {
            let screens = arrange(&raw);
            let devices: Vec<u32> = raw.iter().map(|r| r.0).collect();
            for d in screens.iter().filter(|d| !d.primary) {
                let n = setting_for(d).display_number().unwrap();
                assert_eq!(crate::ui::screen_by_order(&devices, n), Some(d.device), "{name}: screen {n}");
            }
        }
    }

    #[test]
    fn the_number_in_a_choice_is_the_number_the_notch_uses() {
        // `Show on` finds "Screen 2" by the same rule, so what a tile saves is where the notch goes.
        let devices = [5u32, 1];
        let s = arrange(&[(5, -1920, 50, 1920, 1080, false), (1, 0, 0, 1920, 1200, true)]);
        let saved = setting_for(s.iter().find(|d| d.device == 5).unwrap());
        assert_eq!(crate::ui::screen_by_order(&devices, saved.display_number().unwrap()), Some(5));
    }
}
