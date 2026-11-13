//! Tests of the settings window.

use super::paint::backdrop_is_for;
use super::*;

#[test]
fn every_page_fits_inside_the_window() {
    for page in Page::ALL {
        let lay = layout(page, frame_for(WIDTH as f32));
        for g in &lay.groups {
            assert!(g.rect.0 >= SIDEBAR_W && g.rect.0 + g.rect.2 <= WIDTH as f32, "{} is outside on {}", g.title, page.title());
            // About and Closed notch are taller than the default window and scroll; every other page fits.
            assert!(matches!(page, Page::About | Page::ClosedNotch) || g.rect.1 + g.rect.3 <= HEIGHT as f32 - 14.0, "{} runs past the bottom on {}", g.title, page.title());
        }
        for item in &lay.items {
            assert!(item.row.0 + item.row.2 <= WIDTH as f32, "{} is too wide", item.label);
            assert!(item.ctrl.0 >= item.row.0 && item.ctrl.0 + item.ctrl.2 <= item.row.0 + item.row.2, "{} control is outside its row", item.label);
        }
        if let Some(hero) = lay.hero {
            assert!(hero.1 + hero.3 <= HEIGHT as f32, "the card at the top is inside the window");
        }
    }
}

#[test]
fn the_cached_background_is_only_reused_for_the_same_size_colour_and_look() {
    let tint = [10, 20, 30];
    let cached = Some((800, 600, tint, false, Pixmap::new(1, 1).unwrap()));
    assert!(backdrop_is_for(&cached, (800, 600), tint, false));
    assert!(!backdrop_is_for(&None, (800, 600), tint, false), "nothing cached yet");
    assert!(!backdrop_is_for(&cached, (801, 600), tint, false), "another width");
    assert!(!backdrop_is_for(&cached, (800, 599), tint, false), "another height");
    assert!(!backdrop_is_for(&cached, (800, 600), [10, 20, 31], false), "another page colour");
    assert!(!backdrop_is_for(&cached, (800, 600), tint, true), "the Light theme needs its own background");
}

#[test]
fn the_content_grows_with_the_window_and_stays_in_the_middle() {
    let default = frame_for(WIDTH as f32);
    assert_eq!(default, Frame { x: SIDEBAR_W + 30.0, w: MIN_CONTENT_W });
    assert_eq!(frame_for(500.0), default, "never narrower than the default");
    for width in [900.0_f32, 1280.0, 1920.0, 3840.0] {
        let f = frame_for(width);
        assert!(f.w >= MIN_CONTENT_W && f.w <= MAX_CONTENT_W, "{width}: {f:?}");
        let (left, right) = (f.x - SIDEBAR_W, width - (f.x + f.w));
        assert!((left - right).abs() < 0.01 || f.w < MAX_CONTENT_W, "{width}: centred in the room beside the sidebar");
        assert!(f.x >= SIDEBAR_W + 30.0 && f.x + f.w <= width - 30.0, "{width}: {f:?}");
    }
}

#[test]
fn a_page_scrolls_only_as_far_as_it_is_taller_than_the_window() {
    for page in Page::ALL {
        let lay = layout(page, frame_for(WIDTH as f32));
        // Most pages fit the default window; About (a tall card) and Closed notch (the eye picker) scroll a little.
        let limit = match page {
            Page::About => 330.0,
            Page::ClosedNotch => 220.0,
            _ => 0.0,
        };
        assert!(scroll_limit(&lay, HEIGHT as f32) <= limit, "{} needs too much scrolling in the default window", page.title());
        let short = scroll_limit(&lay, MIN_HEIGHT as f32);
        assert!((short - (lay.bottom() + BOTTOM_PAD - MIN_HEIGHT as f32).max(0.0)).abs() < 0.01);
        assert!(scroll_limit(&lay, 300.0) > short || short == 0.0 && lay.bottom() + BOTTOM_PAD <= 300.0, "{}", page.title());
    }
}

#[test]
fn a_scrolled_layout_moves_every_rect_by_the_same_amount() {
    let frame = frame_for(WIDTH as f32);
    let (plain, moved) = (layout(Page::About, frame), layout(Page::About, frame).shifted(-40.0));
    assert_eq!(plain.groups.len(), moved.groups.len());
    for (a, b) in plain.items.iter().zip(&moved.items) {
        assert_eq!((a.row.0, a.row.2, a.row.1 - 40.0, a.ctrl.1 - 40.0), (b.row.0, b.row.2, b.row.1, b.ctrl.1));
    }
    assert_eq!(plain.hero.map(|h| h.1 - 40.0), moved.hero.map(|h| h.1));
    assert!((plain.bottom() - 40.0 - moved.bottom()).abs() < 0.01);
}

#[test]
fn the_scroll_bar_thumb_and_the_scroll_agree() {
    let bar = page_bar(WIDTH as f32, HEIGHT as f32);
    let max = 300.0;
    let thumb = bar.thumb(max);
    assert!(thumb >= 26.0 && thumb < bar.height, "a thumb that shows the share of the page");
    assert_eq!(bar.thumb_top(0.0, max), bar.top);
    assert!((bar.thumb_top(max, max) + thumb - (bar.top + bar.height)).abs() < 0.01, "at the end the thumb reaches the bottom");
    for scroll in [0.0, 40.0, 150.0, 299.0, 300.0] {
        let back = bar.scroll_at(bar.thumb_top(scroll, max), max);
        assert!((back - scroll).abs() < 0.01, "{scroll} came back as {back}");
    }
    assert_eq!(bar.scroll_at(bar.top - 500.0, max), 0.0, "dragged above the track");
    assert_eq!(bar.scroll_at(bar.top + bar.height + 500.0, max), max, "dragged below the track");
}

#[test]
fn the_scroll_bar_is_taken_hold_of_on_and_beside_it_only() {
    let bar = page_bar(WIDTH as f32, HEIGHT as f32);
    assert!(bar.grabs(bar.x + 1.0, bar.top + 10.0));
    assert!(bar.grabs(bar.x - 6.0, bar.top + bar.height), "a few pixels to the left still count");
    assert!(!bar.grabs(bar.x - 20.0, bar.top + 10.0), "the cards are not the bar");
    assert!(!bar.grabs(bar.x + 1.0, bar.top - 5.0), "above the track");
    // Nothing a page shows at its widest reaches the bar.
    let frame = frame_for(WIDTH as f32);
    assert!(frame.x + frame.w < bar.x - 8.0);
    let changelog = changelog_bar(frame, HEIGHT as f32);
    assert!(changelog.top > CHANGELOG_TOP && changelog.top + changelog.height < HEIGHT as f32, "inside the card");
}

#[test]
fn page_keys_scroll_by_a_page_and_stop_at_the_ends() {
    let (max, view) = (440.0, 600.0);
    assert_eq!(scrolled_by_key(0.0, max, view, ScrollKey::PageDown), 440.0, "a page is longer than what is left");
    assert_eq!(scrolled_by_key(100.0, 2000.0, view, ScrollKey::PageDown), 660.0, "the view less a few lines");
    assert_eq!(scrolled_by_key(100.0, 2000.0, view, ScrollKey::PageUp), 0.0);
    assert_eq!(scrolled_by_key(300.0, 2000.0, view, ScrollKey::Home), 0.0);
    assert_eq!(scrolled_by_key(0.0, max, view, ScrollKey::End), max);
    assert_eq!(scrolled_by_key(0.0, 0.0, view, ScrollKey::End), 0.0, "a page that fits does not scroll");
}

#[test]
fn the_window_comes_back_where_it_was_left() {
    let spot = Spot { width: 1000.0, height: 800.0, x: -1500, y: 40 };
    assert_eq!(Spot::from_json(&spot.to_json()), Some(spot));
    assert_eq!(Spot::from_json(&json!({})), None, "no file content");
    assert_eq!(Spot::from_json(&json!({ "width": 50.0, "height": 800.0, "x": 0, "y": 0 })), None, "far too narrow to be a window");
    let rounded = Spot::from_json(&json!({ "width": 839.9999, "height": 639.9999, "x": 5, "y": 6 }));
    assert!(rounded.is_some(), "the smallest window, rounded down, still counts");
    let spot = rounded.unwrap();
    assert_eq!(size_on_screen(spot.width, spot.height, 1920.0, 1200.0, 1.25, 1.0), (WIDTH, MIN_HEIGHT));
    assert_eq!(Spot::from_json(&json!({ "width": "wide", "height": 800.0, "x": 0, "y": 0 })), None);
    assert_eq!(Spot::from_json(&json!({ "width": 900.0, "height": 800.0, "x": 99999999999i64, "y": 0 })), None);
}

#[test]
fn a_remembered_size_is_kept_inside_what_the_screen_has_room_for() {
    // A roomy screen: whatever was left is kept, but never below the smallest window.
    assert_eq!(size_on_screen(1000.0, 800.0, 2560.0, 1440.0, 1.0, 1.0), (1000.0, 800.0));
    assert_eq!(size_on_screen(10.0, 10.0, 2560.0, 1440.0, 1.0, 1.0), (WIDTH, MIN_HEIGHT));
    // A smaller screen than the one it was left on.
    let (w, h) = size_on_screen(3000.0, 2000.0, 1920.0, 1200.0, 1.25, 1.0);
    assert!(w <= 1920.0 / 1.25 && h <= 1200.0 / 1.25 - 130.0 + 0.01, "{w} x {h}");
    // A tiny screen never makes it smaller than the window can be.
    let (w, h) = size_on_screen(900.0, 700.0, 800.0, 500.0, 1.0, 0.55);
    assert!(w >= WIDTH && (MIN_HEIGHT..700.0).contains(&h), "{w} x {h}");
    assert_eq!(size_on_screen(10.0, 10.0, 800.0, 400.0, 1.0, 0.55), (WIDTH, MIN_HEIGHT));
}

#[test]
fn a_remembered_place_is_used_only_while_the_title_bar_is_on_a_screen() {
    let screens = [(0, 0, 1920, 1080), (1920, 0, 1920, 1080)];
    assert!(on_a_screen(100, 100, &screens));
    assert!(on_a_screen(2000, 50, &screens), "on the second screen");
    assert!(!on_a_screen(5000, 100, &screens), "the screen it was on is gone");
    assert!(!on_a_screen(100, -400, &screens), "title bar above the top");
    assert!(!on_a_screen(100, 100, &[]), "no screens known");
}

#[test]
fn groups_on_a_page_do_not_overlap() {
    for page in Page::ALL {
        let lay = layout(page, frame_for(WIDTH as f32));
        for pair in lay.groups.windows(2) {
            assert!(pair[0].rect.1 + pair[0].rect.3 < pair[1].rect.1, "{} overlaps {} on {}", pair[0].title, pair[1].title, page.title());
        }
    }
}

#[test]
fn labels_have_room_next_to_their_controls() {
    for page in Page::ALL {
        for item in layout(page, frame_for(WIDTH as f32)).items {
            // A control under its label has the whole row above it for the text.
            if item.ctrl.2 > 0.0 && !under_label(item.kind) {
                assert!(item.ctrl.0 - item.row.0 > 230.0, "{} leaves too little room for its text", item.label);
            }
        }
    }
}

#[test]
fn every_setting_is_on_exactly_one_page() {
    let mut toggles = Vec::new();
    let (mut segs, mut slides, mut picks) = (Vec::new(), Vec::new(), 0);
    for page in Page::ALL {
        for item in layout(page, frame_for(WIDTH as f32)).items {
            match item.kind {
                Kind::Toggle(t) => toggles.push(t),
                Kind::Seg(g) => segs.push(g),
                Kind::Slider(l) => slides.push(l),
                Kind::Pick => picks += 1,
                // The tab switches are the tiles of the tab grid.
                Kind::TabGrid => toggles.extend(TAB_TILES.iter().map(|t| t.1)),
                _ => {}
            }
        }
    }
    assert_eq!(toggles.len(), 40, "all forty switches: {toggles:?}");
    for (i, a) in toggles.iter().enumerate() {
        assert!(!toggles[i + 1..].contains(a), "{a:?} is on two pages");
    }
    assert_eq!(segs.len(), 12);
    assert_eq!(slides.len(), 7);
    assert_eq!(picks, 1);
}

#[test]
fn the_window_shrinks_to_fit_a_small_screen_and_stays_whole_on_a_roomy_one() {
    // 1920 x 1080 at 100, 125 and 150 percent, and 1366 x 768 at 100 percent.
    assert_eq!(fit_to_screen(1920.0, 1080.0, 1.0), 1.0);
    assert_eq!(fit_to_screen(1920.0, 1080.0, 1.25), 1.0);
    for (w, h, sf) in [(1920.0, 1080.0, 1.5), (1366.0, 768.0, 1.0), (1280.0, 720.0, 1.0)] {
        let fit = fit_to_screen(w, h, sf);
        assert!((0.55..1.0).contains(&fit), "{w}x{h} at {sf}: {fit}");
        // The window (plus about 130 logical pixels for the title bar and taskbar) fits the screen.
        assert!(HEIGHT * fit * sf + 130.0 * sf <= h + 1.0 || fit == 0.55, "{w}x{h} at {sf}");
    }
}

#[test]
fn the_sidebar_rows_fit_above_the_version() {
    let last = side_rect(Page::About);
    assert!(last.1 + last.3 < MIN_HEIGHT as f32 - VERSION_FROM_BOTTOM - 12.0, "the pages must not run into the version, even in the smallest window");
    for (i, page) in Page::ALL.into_iter().enumerate() {
        assert_eq!(page.index(), i);
        let r = side_rect(page);
        assert!(r.0 + r.2 <= SIDEBAR_W);
    }
}

#[test]
fn sliders_land_on_their_steps_and_stay_in_range() {
    let range = settings::OPEN_DELAY;
    let track = (100.0, 0.0, 100.0, 4.0);
    assert_eq!(slider_value(range, track, 0.0), 0, "left of the track is the minimum");
    assert_eq!(slider_value(range, track, 100.0), 0);
    assert_eq!(slider_value(range, track, 200.0), 500);
    assert_eq!(slider_value(range, track, 900.0), 500, "right of the track is the maximum");
    assert_eq!(slider_value(range, track, 150.0), 250);
    assert_eq!(slider_value(range, track, 153.0) % 10, 0, "always a whole step");
    let alarm = settings::ALARM_EVERY;
    assert_eq!(slider_value(alarm, track, 100.0), 1);
    assert_eq!(slider_value(alarm, track, 200.0), 10);
}

#[test]
fn slider_text_is_readable() {
    assert_eq!(slider_text(Slide::OpenDelay, 0), "Instant");
    assert_eq!(slider_text(Slide::OpenDelay, 90), "90 ms");
    assert_eq!(slider_text(Slide::Volume, 60), "60%");
    assert_eq!(slider_text(Slide::AlarmEvery, 3), "3 s");
}

#[test]
fn the_choice_lists_match_the_settings() {
    assert_eq!(THEME_NAMES.len(), ThemeMode::ALL.len());
    assert_eq!(LOOK_NAMES.len(), NotchShape::ALL.len());
    assert_eq!(GLASS_NAMES.len(), NotchGlass::ALL.len());
    assert_eq!(seg_weights(Seg::Glass).len(), GLASS_NAMES.len());
    assert_eq!(seg_weights(Seg::Look).len(), LOOK_NAMES.len());
    assert_eq!(MOTION_NAMES.len(), Motion::ALL.len());
    assert_eq!(seg_weights(Seg::Theme).len(), THEME_NAMES.len());
    assert_eq!(seg_weights(Seg::Motion).len(), MOTION_NAMES.len());
    assert_eq!(seg_weights(Seg::Size).len(), SIZE_NAMES.len());
    assert_eq!(SIZE_NAMES.len(), NotchSize::ALL.len());
    for seg in [Seg::Open, Seg::WeekStart, Seg::CalView] {
        assert_eq!(seg_weights(seg).len(), seg_names(seg).len());
    }
}

#[test]
fn the_eye_picker_has_a_tile_for_every_style_and_they_fit_its_control() {
    let ctrl = (400.0, 20.0, EYE_GRID.0, EYE_GRID.1);
    for i in 0..EyeStyle::ALL.len() {
        let t = eye_tile(ctrl, i);
        assert!(t.0 >= ctrl.0 && t.0 + t.2 <= ctrl.0 + ctrl.2 + 0.01 && t.1 >= ctrl.1 && t.1 + t.3 <= ctrl.1 + ctrl.3 + 0.01, "tile {i} is inside");
        assert_eq!(eye_tile_at(ctrl, t.0 + t.2 / 2.0, t.1 + t.3 / 2.0), Some(i), "its middle is that tile");
        for j in 0..i {
            let o = eye_tile(ctrl, j);
            assert!(t.0 >= o.0 + o.2 || o.0 >= t.0 + t.2 || t.1 >= o.1 + o.3 || o.1 >= t.1 + t.3, "tiles {i} and {j} do not overlap");
        }
    }
    assert_eq!(eye_tile_at(ctrl, ctrl.0 - 5.0, ctrl.1 - 5.0), None);
    // The row is tall enough for the picker, and the picker is on the Closed notch page, once.
    assert!(row_height("x", Kind::EyeGrid, 1) > EYE_GRID.1);
    let on_page = layout(Page::ClosedNotch, frame_for(WIDTH as f32)).items.iter().filter(|i| matches!(i.kind, Kind::EyeGrid)).count();
    assert_eq!(on_page, 1);
}

#[test]
fn the_tab_grid_has_a_tile_for_every_tab_inside_its_row() {
    let lay = layout(Page::Tabs, frame_for(WIDTH as f32));
    let rows: Vec<_> = lay.items.iter().filter(|i| matches!(i.kind, Kind::TabGrid)).collect();
    assert_eq!(rows.len(), 1, "one grid, on the Tabs page");
    let (row, ctrl) = (rows[0].row, rows[0].ctrl);
    assert!(ctrl.1 >= row.1 + 40.0 && ctrl.1 + ctrl.3 <= row.1 + row.3, "under the label, inside the row");
    assert!(ctrl.0 >= row.0 && ctrl.0 + ctrl.2 <= row.0 + row.2);
    // Every tab of the notch has a tile, and each one only once.
    let views: Vec<View> = TAB_TILES.iter().map(|t| t.0).collect();
    for view in [View::Player, View::Timer, View::Water, View::Weather, View::Prayer, View::Stats, View::Shelf, View::Todo, View::Calendar, View::Clipboard] {
        assert_eq!(views.iter().filter(|v| **v == view).count(), 1, "{:?}", view as u8);
    }
    for i in 0..TAB_TILES.len() {
        let t = tab_tile(ctrl, i);
        assert!(t.0 >= ctrl.0 - 0.01 && t.0 + t.2 <= ctrl.0 + ctrl.2 + 0.01 && t.1 >= ctrl.1 && t.1 + t.3 <= ctrl.1 + ctrl.3 + 0.01, "tile {i} is inside");
        assert_eq!(tab_tile_at(ctrl, t.0 + t.2 / 2.0, t.1 + t.3 / 2.0), Some(i));
        for j in 0..i {
            let o = tab_tile(ctrl, j);
            assert!(t.0 >= o.0 + o.2 || o.0 >= t.0 + t.2 || t.1 >= o.1 + o.3 || o.1 >= t.1 + t.3, "tiles {i} and {j} do not overlap");
        }
    }
    assert_eq!(tab_tile_at(ctrl, ctrl.0 - 5.0, ctrl.1 - 5.0), None, "the gap around the grid is no tile");
}

#[test]
fn the_sidebar_has_its_section_headings_in_the_gaps_between_rows() {
    for pair in Page::ALL.windows(2) {
        let (a, b) = (side_rect(pair[0]), side_rect(pair[1]));
        let gap = b.1 - (a.1 + a.3);
        if pair[1].section().is_some() {
            assert!(gap >= SIDE_HEAD_H, "room for the heading above {}", pair[1].title());
        } else if pair[1] == Page::About {
            assert!(gap > SIDE_ITEM_H - a.3, "About stands a little apart");
        } else {
            assert!((gap - (SIDE_ITEM_H - a.3)).abs() < 0.01, "{} follows {} closely", pair[1].title(), pair[0].title());
        }
    }
}

#[test]
fn the_screen_picker_is_on_the_notch_page_once_and_its_row_is_tall_enough() {
    let lay = layout(Page::Notch, frame_for(WIDTH as f32));
    let rows: Vec<_> = lay.items.iter().filter(|i| matches!(i.kind, Kind::ScreenMap)).collect();
    assert_eq!(rows.len(), 1);
    assert!(row_height("x", Kind::ScreenMap, 1) > SCREEN_MAP.1);
    let ctrl = rows[0].ctrl;
    assert_eq!((ctrl.2, ctrl.3), SCREEN_MAP);
    // The picker sits inside its row and does not run into the text of the row.
    assert!(ctrl.1 >= rows[0].row.1 && ctrl.1 + ctrl.3 <= rows[0].row.1 + rows[0].row.3);
}

#[test]
fn segmented_choices_fill_the_control_without_gaps() {
    let ctrl = (100.0, 10.0, 204.0, 24.0);
    let cells = seg_cells(Seg::Theme, ctrl);
    assert_eq!(cells.len(), 3);
    assert_eq!(cells[0].0, 100.0);
    assert!((cells[2].0 + cells[2].2 - 304.0).abs() < 0.01, "the last choice ends where the control ends");
    for pair in cells.windows(2) {
        assert!((pair[0].0 + pair[0].2 - pair[1].0).abs() < 0.01, "no gap between choices");
    }
    assert!(cells[2].2 > cells[0].2 * 1.9, "Follow System gets the most room");
}

#[test]
fn a_row_grows_by_a_line_for_each_extra_line_of_its_text() {
    assert_eq!(row_height("text", Kind::Link(""), 1), ROW_H);
    assert_eq!(row_height("text", Kind::Link(""), 2), ROW_H + DESC_LINE_H);
    assert_eq!(row_height("text", Kind::Link(""), 3), ROW_H + 2.0 * DESC_LINE_H);
    assert_eq!(row_height("", Kind::Link(""), 1), 40.0, "a row with no text under its label stays short");
}

#[test]
fn rows_with_long_text_get_taller_and_everything_below_moves_down() {
    let frame = frame_for(WIDTH as f32);
    // Say every second line is long enough to need two lines.
    let long = |_: Kind, desc: &'static str, _: f32| if desc.len().is_multiple_of(2) { 2 } else { 1 };
    for page in Page::ALL {
        let (plain, tall) = (layout(page, frame), layout_with(page, frame, &long));
        assert!(tall.bottom() >= plain.bottom(), "{} did not grow", page.title());
        for pair in tall.groups.windows(2) {
            assert!(pair[0].rect.1 + pair[0].rect.3 < pair[1].rect.1, "{} overlaps {} on {}", pair[0].title, pair[1].title, page.title());
        }
        for (a, b) in plain.items.iter().zip(&tall.items) {
            assert_eq!(a.ctrl.2, b.ctrl.2, "{}: the controls keep their size", a.label);
            assert!(b.ctrl.1 >= b.row.1 && b.ctrl.1 + b.ctrl.3 <= b.row.1 + b.row.3 + 0.01, "{}: its control is inside its row", a.label);
        }
    }
}

#[test]
fn every_zone_fits_the_zone_control_in_the_real_font() {
    let Some(font) = crate::load_font() else { return };
    let frame = frame_for(WIDTH as f32);
    let row = layout(Page::Location, frame).items.into_iter().find(|i| matches!(i.kind, Kind::Cycle(Stepper::Zone))).expect("the Zone row");
    // The text may use the control's width less the arrows, and a little of that is kept for the heading's heavier letters.
    let room = row.ctrl.2 - 56.0 - 12.0;
    let too_wide: Vec<String> = crate::zones::ZONES
        .iter()
        .map(|zone| format!("{}{}", crate::zones::AUTOMATIC, crate::zones::describe(zone)))
        .filter(|text| crate::text_width(&font, text, 12.0) > room)
        .collect();
    assert!(too_wide.is_empty(), "wider than {room}: {too_wide:?}");
}

#[test]
fn every_line_under_a_label_fits_its_row_in_the_real_font() {
    let Some(font) = crate::load_font() else { return };
    let frame = frame_for(WIDTH as f32);
    for page in Page::ALL {
        for (_, rows) in page_spec(page) {
            for (label, desc, kind, btn) in rows {
                let room = text_room(kind, btn, frame);
                let lines = crate::wrap(&font, desc, DESC_PX, room);
                // Every line of the wrapped text is inside the room, and no row needs more than three lines.
                assert!(lines.len() <= 3, "{label} needs {} lines", lines.len());
                for line in &lines {
                    assert!(desc.is_empty() || crate::text_width(&font, line, DESC_PX) <= room, "{label}: a line is wider than its room");
                }
            }
        }
    }
    // The note that was once cut off: short enough now for one line.
    let room = text_room(Kind::Text(Field::City), "", frame);
    let note = CityField { status: CityStatus::Approximate, ..Default::default() }.note().0;
    assert_eq!(crate::wrap(&font, &note, DESC_PX, room).len(), 1, "{note}");
}
