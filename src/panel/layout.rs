//! What is on each page of the settings window and where it sits: the pages, their rows and controls, and the layout of a page for a window of a given width.

use super::*;

/// Where a page's content sits across the window: it grows with the window up to a limit and stays in the middle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Frame {
    pub(super) x: f32,
    pub(super) w: f32,
}

/// The frame for a window `width` logical pixels wide.
pub(super) fn frame_for(width: f32) -> Frame {
    let room = (width - SIDEBAR_W - 60.0).max(MIN_CONTENT_W);
    let w = room.min(MAX_CONTENT_W);
    Frame { x: SIDEBAR_W + 30.0 + (room - w) / 2.0, w }
}

/// The on/off settings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Toggle {
    Autostart,
    HideFullscreen,
    ShowClock,
    Clock24,
    Eyes,
    IdleEyes,
    Minimal,
    ClosedMusic,
    ClosedBars,
    ClosedSpin,
    ClosedEdge,
    ClosedSound,
    LyricsOnline,
    EmptyShelf,
    HudVolume,
    HudBrightness,
    HudLock,
    HudBattery,
    HudNetwork,
    HudAudio,
    HudUpdate,
    Sound,
    Clipboard,
    HideIsland,
    Water,
    Timer,
    Weather,
    Prayer,
    HudPrayer,
    Hotkey,
    TabMusic,
    TabStats,
    TabShelf,
    TabTodo,
    TabCalendar,
    TabClipboard,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Seg {
    Theme,
    Motion,
    Size,
    Open,
    WeekStart,
    CalView,
    ClipKeep,
    Units,
    Asr,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Slide {
    OpenDelay,
    CloseDelay,
    Volume,
    AlarmEvery,
    AlarmFor,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Act {
    OpenFolder,
    ClearLyrics,
    CheckUpdate,
    Changelog,
    Reset,
    DetectCity,
    ClearCity,
}

/// The controls that step through a long list of choices.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Stepper {
    Method,
    /// The e-Solat zone (only when that is the method).
    Zone,
}

/// The boxes the user types in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Field {
    City,
    /// The key combination for the hotkey, which is taken by pressing the keys.
    Hotkey,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Kind {
    Toggle(Toggle),
    Seg(Seg),
    Slider(Slide),
    Button(Act),
    /// A button that shows the current choice, for the timer's alarm sound.
    Pick,
    /// A box to type in.
    Text(Field),
    /// The current choice of a long list, with arrows to step to the one before and the one after.
    Cycle(Stepper),
    /// A line with a value on the right; with an address it opens that page when clicked.
    Link(&'static str),
    /// The eye picker: a tile for each style, each showing a pair of eyes drawn the way the closed notch draws them.
    EyeGrid,
    /// The map of the connected screens, to choose where the notch is.
    ScreenMap,
}

/// The pages of the window, in the order of the sidebar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Page {
    General,
    Notch,
    Appearance,
    Tabs,
    Location,
    Sounds,
    Calendar,
    Shelf,
    Notices,
    About,
}

impl Page {
    pub(super) const ALL: [Page; 10] =
        [Page::General, Page::Notch, Page::Appearance, Page::Tabs, Page::Location, Page::Sounds, Page::Calendar, Page::Shelf, Page::Notices, Page::About];

    pub(super) fn index(self) -> usize {
        Self::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }

    pub(super) fn title(self) -> &'static str {
        match self {
            Page::General => "General",
            Page::Notch => "Notch",
            Page::Appearance => "Appearance",
            Page::Tabs => "Tabs",
            Page::Location => "Location",
            Page::Sounds => "Sounds",
            Page::Calendar => "Calendar",
            Page::Shelf => "Shelf",
            Page::Notices => "Notices",
            Page::About => "About",
        }
    }

    pub(super) fn subtitle(self) -> &'static str {
        match self {
            Page::General => "How Sky Island starts, and the key that opens it.",
            Page::Notch => "How the notch opens and hides, how big it is, and what the closed notch shows.",
            Page::Appearance => "The colours and the motion.",
            Page::Tabs => "Choose which tabs the notch has. It always keeps at least one.",
            Page::Location => "Where you are, for the weather and the prayer times.",
            Page::Sounds => "Chimes and the timer alarm.",
            Page::Calendar => "How the calendar counts its weeks and what it shows.",
            Page::Shelf => "What happens to the files on the shelf.",
            Page::Notices => "Choose which changes pop up on the notch.",
            Page::About => "This version, updates and your data.",
        }
    }

    /// The colour of the page's icon, and of its sliders and choices.
    pub(super) fn tint(self) -> [u8; 3] {
        match self {
            Page::General => [142, 148, 168],
            Page::Notch => [96, 110, 255],
            Page::Appearance => [170, 112, 255],
            Page::Tabs => [255, 176, 64],
            Page::Location => [64, 196, 124],
            Page::Sounds => [255, 92, 120],
            Page::Calendar => [255, 96, 86],
            Page::Shelf => [255, 140, 56],
            Page::Notices => [56, 176, 240],
            Page::About => [70, 190, 210],
        }
    }
}

/// One row of a page: its label, a line under it, what it controls, and the text of a button.
pub(super) type RowSpec = (&'static str, &'static str, Kind, &'static str);

/// The groups (title and rows) on a page.
pub(super) fn page_spec(page: Page) -> Vec<(&'static str, Vec<RowSpec>)> {
    use Kind::{Button, Cycle, EyeGrid, Link, Pick, ScreenMap, Seg as S, Slider, Text, Toggle as T};
    match page {
        Page::General => vec![
            ("Startup", vec![("Start with Windows", "Open Sky Island when you sign in", T(Toggle::Autostart), "")]),
            (
                "Hotkey",
                vec![
                    ("Open with a key", "Press it anywhere to open the notch, and again to close it", T(Toggle::Hotkey), ""),
                    ("Key combination", "Click, then press the keys you want", Text(Field::Hotkey), ""),
                ],
            ),
        ],
        Page::Notch => vec![
            (
                "Opening",
                vec![
                    ("Open the notch", "Hover over it, or click it", S(Seg::Open), ""),
                    ("Open delay", "How long the mouse rests on the notch before it opens", Slider(Slide::OpenDelay), ""),
                    ("Close delay", "How long the mouse is away before it closes", Slider(Slide::CloseDelay), ""),
                ],
            ),
            (
                "Behaviour",
                vec![
                    ("Hide in full screen", "Games, videos and slides that fill the screen", T(Toggle::HideFullscreen), ""),
                    ("Auto-hide notch", "Slide out of view when you are not using it", T(Toggle::HideIsland), ""),
                ],
            ),
            (
                "Size",
                vec![
                    ("Notch size", "Make the whole notch bigger or smaller", S(Seg::Size), ""),
                    ("Minimal notch", "A smaller pill when nothing is playing", T(Toggle::Minimal), ""),
                ],
            ),
            ("Screen", vec![("Show on", "Click a screen. All follows your mouse; Every keeps a closed notch on each screen", ScreenMap, "")]),
            (
                "Closed notch",
                vec![
                    ("Show clock", "The time on the closed notch", T(Toggle::ShowClock), ""),
                    ("24-hour clock", "13:00 instead of 1:00 PM", T(Toggle::Clock24), ""),
                    ("Show music", "The album cover and moving bars on the closed notch while a song plays", T(Toggle::ClosedMusic), ""),
                    ("Moving bars", "Five bars that rise and fall beside the clock (with Show music)", T(Toggle::ClosedBars), ""),
                    ("Spinning record", "The cover turns like a record; off keeps it still (with Show music)", T(Toggle::ClosedSpin), ""),
                    ("Edge glow", "A thin line along the notch in the cover's colour that breathes (with Show music)", T(Toggle::ClosedEdge), ""),
                    (
                        "Bars follow the sound",
                        "Listens to the sound this PC plays, only while the bars show and only on this PC, so they rise with the music. Off, they move to a made-up beat",
                        T(Toggle::ClosedSound),
                        "",
                    ),
                ],
            ),
            (
                "Eyes",
                vec![
                    ("Enable eyes", "Blinking eyes on the closed notch while you work", T(Toggle::Eyes), ""),
                    ("Idle eyes", "They appear, and doze off, when you stop using the PC", T(Toggle::IdleEyes), ""),
                    ("Eye style", "Pick one to see it on the closed notch", EyeGrid, ""),
                ],
            ),
        ],
        Page::Appearance => {
            vec![("Look", vec![("Theme", "Dark, light, or the same as Windows", S(Seg::Theme), ""), ("Animation", "How lively the movement is", S(Seg::Motion), "")])]
        }
        Page::Tabs => vec![
            (
                "Tabs",
                vec![
                    ("Music", "What is playing, its controls and the lyrics", T(Toggle::TabMusic), ""),
                    (
                        "Find lyrics online",
                        "Sends the title and artist of what plays to LRCLIB to find its lyrics. Off, only lyrics saved on this PC show",
                        T(Toggle::LyricsOnline),
                        "",
                    ),
                    ("Timer", "A countdown timer with an alarm", T(Toggle::Timer), ""),
                    ("Water tracker", "A daily glass counter with reminders", T(Toggle::Water), ""),
                    ("Weather", "The weather now and the days ahead for your city", T(Toggle::Weather), ""),
                    ("Prayer times", "Today's prayers and the next one for your city", T(Toggle::Prayer), ""),
                    ("System stats", "Processor, memory, graphics, drives and network, with live graphs, while the notch is open", T(Toggle::TabStats), ""),
                    ("Shelf", "A place to park files", T(Toggle::TabShelf), ""),
                    ("To-do", "Tasks with reminders", T(Toggle::TabTodo), ""),
                    ("Calendar", "A month or a week with your tasks", T(Toggle::TabCalendar), ""),
                    ("Clipboard", "The history of what you copy", T(Toggle::TabClipboard), ""),
                ],
            ),
            (
                "Clipboard",
                vec![
                    ("Clipboard history", "Keep what you copy, text and pictures", T(Toggle::Clipboard), ""),
                    ("Forget copied items", "Remove them from the history after a while", S(Seg::ClipKeep), ""),
                ],
            ),
        ],
        Page::Location => vec![
            (
                "Your city",
                vec![
                    ("City", "Type a city and press Enter", Text(Field::City), ""),
                    ("Detect my location", "Uses Windows Location; if it is off, your internet connection", Button(Act::DetectCity), "Detect"),
                    ("Clear location", "Forget the city; Weather and Prayer wait for a new one", Button(Act::ClearCity), "Clear"),
                ],
            ),
            ("Weather", vec![("Temperature", "Celsius or Fahrenheit", S(Seg::Units), "")]),
            (
                "Prayer times",
                vec![
                    ("Method", "How the times are worked out", Cycle(Stepper::Method), ""),
                    ("Zone", "For JAKIM e-Solat only; Automatic is the zone nearest to your city", Cycle(Stepper::Zone), ""),
                    ("Asr", "How the length of the shadow is counted", S(Seg::Asr), ""),
                ],
            ),
        ],
        Page::Sounds => vec![
            ("Chimes", vec![("Sounds", "Chimes for the timer, to-do and water reminders", T(Toggle::Sound), ""), ("Volume", "How loud they are", Slider(Slide::Volume), "")]),
            (
                "Timer alarm",
                vec![
                    ("Repeat every", "How often the alarm sounds again", Slider(Slide::AlarmEvery), ""),
                    ("Ring for", "How long it rings if nobody stops it", Slider(Slide::AlarmFor), ""),
                    ("Alarm sound", "Use a sound file of your own", Pick, ""),
                ],
            ),
        ],
        Page::Calendar => {
            vec![("Options", vec![("Week starts on", "The first column of the month", S(Seg::WeekStart), ""), ("Default view", "A month, or just one week", S(Seg::CalView), "")])]
        }
        Page::Shelf => vec![("Shelf", vec![("Start with an empty shelf", "Files from your last session are cleared when Sky Island starts", T(Toggle::EmptyShelf), "")])],
        Page::Notices => vec![
            (
                "Indicators",
                vec![
                    ("Volume", "When you change the volume or mute", T(Toggle::HudVolume), ""),
                    ("Brightness", "On laptop screens", T(Toggle::HudBrightness), ""),
                    ("Caps Lock and Num Lock", "When you press the lock keys", T(Toggle::HudLock), ""),
                ],
            ),
            (
                "Popups",
                vec![
                    ("Battery", "Charging, unplugged and low battery", T(Toggle::HudBattery), ""),
                    ("Wi-Fi and internet", "When the connection changes", T(Toggle::HudNetwork), ""),
                    ("Headphones and speakers", "When an audio device is connected", T(Toggle::HudAudio), ""),
                    ("Prayer time", "When a prayer time comes, with the Prayer tab on", T(Toggle::HudPrayer), ""),
                ],
            ),
            ("Updates", vec![("New version", "Looks on GitHub at start and every 6 hours, and tells you when there is one", T(Toggle::HudUpdate), "")]),
        ],
        Page::About => vec![
            (
                "Updates and data",
                vec![
                    ("Check for updates", "", Button(Act::CheckUpdate), "Check for updates"),
                    ("Changelog", "What changed in each version", Button(Act::Changelog), "View"),
                    ("Open data folder", "Your settings, tasks and shelf live here", Button(Act::OpenFolder), "Open"),
                    ("Lyrics cache", "Lyrics saved on this PC for the songs you played", Button(Act::ClearLyrics), "Clear"),
                    ("Reset to defaults", "Put every setting back to what a fresh install has", Button(Act::Reset), "Reset"),
                ],
            ),
            ("Info", vec![("Website", "", Link("https://github.com/Dannyz15/Sky-Island"), "github.com/Dannyz15/Sky-Island"), ("Made by", "", Link(""), "Dannyz15")]),
            (
                "Thanks",
                vec![
                    ("Synced lyrics", "", Link("https://lrclib.net"), "LRCLIB"),
                    ("Weather", "", Link("https://open-meteo.com"), "Open-Meteo"),
                    ("Places and city search", "", Link("https://www.openstreetmap.org/copyright"), "OpenStreetMap"),
                    ("Prayer times", "", Link("https://aladhan.com"), "Aladhan"),
                    ("Prayer times in Malaysia", "", Link("https://www.e-solat.gov.my"), "JAKIM e-Solat"),
                    ("Built with", "", Link("https://github.com/Dannyz15/Sky-Island/blob/main/Cargo.toml"), "Rust and open-source crates"),
                ],
            ),
        ],
    }
}

pub(super) struct Item {
    pub(super) label: &'static str,
    pub(super) desc: &'static str,
    pub(super) kind: Kind,
    /// The text of a button.
    pub(super) btn: &'static str,
    pub(super) row: Rect,
    /// The part of the row that holds the control.
    pub(super) ctrl: Rect,
    /// Whether another row of the same card is above it, for the line between them.
    pub(super) after_row: bool,
}

pub(super) struct Group {
    pub(super) title: &'static str,
    pub(super) rect: Rect,
}

pub(super) struct Layout {
    pub(super) groups: Vec<Group>,
    pub(super) items: Vec<Item>,
    /// The card at the top of the About page.
    pub(super) hero: Option<Rect>,
}

impl Layout {
    /// Where the last card ends.
    pub(super) fn bottom(&self) -> f32 {
        let groups = self.groups.iter().map(|g| g.rect.1 + g.rect.3);
        groups.chain(self.hero.map(|h| h.1 + h.3)).fold(CONTENT_TOP, f32::max)
    }

    /// The same layout moved down by `dy` (up when negative), for a scrolled page.
    pub(super) fn shifted(mut self, dy: f32) -> Layout {
        for g in &mut self.groups {
            g.rect.1 += dy;
        }
        for item in &mut self.items {
            item.row.1 += dy;
            item.ctrl.1 += dy;
        }
        if let Some(h) = &mut self.hero {
            h.1 += dy;
        }
        self
    }
}

/// Everything on a page, laid out in logical pixels, every line of text on one line.
#[cfg(test)]
pub(super) fn layout(page: Page, frame: Frame) -> Layout {
    layout_with(page, frame, &|_, _, _| 1)
}

/// The control of a row, inside the row `row` (its left, top, width and height).
fn ctrl_rect(kind: Kind, btn: &str, row: Rect) -> Rect {
    let right = row.0 + row.2 - 18.0;
    let mid = row.1 + row.3 / 2.0;
    match kind {
        Kind::Toggle(_) => (right - 40.0, mid - 11.25, 40.0, 22.5),
        Kind::Seg(seg) => (right - seg_width(seg), mid - 14.0, seg_width(seg), 28.0),
        Kind::Slider(_) => (right - SLIDER_TEXT_W - 12.0 - SLIDER_W, mid - 2.0, SLIDER_W, 4.0),
        Kind::Pick => (right - 200.0, mid - 14.0, 200.0, 28.0),
        // The zone's name is longer than the others, so its control is wider.
        Kind::Cycle(Stepper::Zone) => (right - 300.0, mid - 14.0, 300.0, 28.0),
        Kind::Text(_) | Kind::Cycle(_) => (right - 260.0, mid - 14.0, 260.0, 28.0),
        Kind::Button(_) => {
            let w = if btn.len() > 12 { 168.0 } else { 104.0 };
            (right - w, mid - 14.0, w, 28.0)
        }
        Kind::Link(_) => (right - 260.0, mid - 14.0, 260.0, 28.0),
        Kind::EyeGrid => (right - EYE_GRID.0, mid - EYE_GRID.1 / 2.0, EYE_GRID.0, EYE_GRID.1),
        Kind::ScreenMap => (right - SCREEN_MAP.0, mid - SCREEN_MAP.1 / 2.0, SCREEN_MAP.0, SCREEN_MAP.1),
    }
}

/// The width a row's text has before it reaches the control, in logical pixels.
pub(super) fn text_room(kind: Kind, btn: &str, frame: Frame) -> f32 {
    ctrl_rect(kind, btn, (frame.x, 0.0, frame.w, 0.0)).0 - frame.x - 36.0
}

/// Everything on a page, laid out in logical pixels. `lines` says how many lines the text under a row's label
/// takes in a given width (the text is what the row shows now, which can differ from the one in the page's
/// list), and a row grows by a line of text for each one past the first.
pub(super) fn layout_with(page: Page, frame: Frame, lines: &dyn Fn(Kind, &'static str, f32) -> usize) -> Layout {
    let (mut groups, mut items) = (Vec::new(), Vec::new());
    let mut y = CONTENT_TOP;
    let mut hero = None;
    if page == Page::About {
        hero = Some((frame.x, y, frame.w, ABOUT_HERO_H));
        y += ABOUT_HERO_H + GROUP_GAP;
    }
    if page == Page::Calendar {
        // A preview of the month (or week) with a label above it.
        y += GROUP_LABEL_H;
        hero = Some((frame.x, y, frame.w, CAL_PREVIEW_H));
        y += CAL_PREVIEW_H + GROUP_GAP;
    }
    for (title, rows) in page_spec(page) {
        y += GROUP_LABEL_H;
        let heights: Vec<f32> = rows.iter().map(|(_, desc, kind, btn)| row_height(desc, *kind, lines(*kind, desc, text_room(*kind, btn, frame)))).collect();
        let height: f32 = heights.iter().sum();
        groups.push(Group { title, rect: (frame.x, y, frame.w, height) });
        let mut row_y = y;
        for (i, (label, desc, kind, btn)) in rows.into_iter().enumerate() {
            let row_h = heights[i];
            let row = (frame.x, row_y, frame.w, row_h);
            row_y += row_h;
            let ctrl = ctrl_rect(kind, btn, row);
            items.push(Item { label, desc, kind, btn, row, ctrl, after_row: i > 0 });
        }
        y += height + GROUP_GAP;
    }
    Layout { groups, items, hero }
}

/// The height of a row: taller when it has text under its label, and taller still for each line of that text
/// past the first (`lines` is how many it takes).
pub(super) fn row_height(desc: &str, kind: Kind, lines: usize) -> f32 {
    if matches!(kind, Kind::EyeGrid) {
        return EYE_GRID.1 + 28.0;
    }
    if matches!(kind, Kind::ScreenMap) {
        return SCREEN_MAP.1 + 28.0;
    }
    if desc.is_empty() && !matches!(kind, Kind::Button(Act::CheckUpdate)) {
        return 40.0;
    }
    ROW_H + lines.saturating_sub(1) as f32 * DESC_LINE_H
}

/// The eye picker: five tiles across and two down, each showing a pair of eyes.
const EYE_TILE: (f32, f32) = (50.0, 34.0);
const EYE_GAP: f32 = 6.0;
const EYE_COLS: usize = 5;
pub(super) const EYE_GRID: (f32, f32) = (EYE_COLS as f32 * EYE_TILE.0 + (EYE_COLS - 1) as f32 * EYE_GAP, 2.0 * EYE_TILE.1 + EYE_GAP);

/// Tile `i` of the eye picker whose control is `ctrl`.
pub(super) fn eye_tile(ctrl: Rect, i: usize) -> Rect {
    let (col, row) = ((i % EYE_COLS) as f32, (i / EYE_COLS) as f32);
    (ctrl.0 + col * (EYE_TILE.0 + EYE_GAP), ctrl.1 + row * (EYE_TILE.1 + EYE_GAP), EYE_TILE.0, EYE_TILE.1)
}

/// The tile under (`x`, `y`), if any.
pub(super) fn eye_tile_at(ctrl: Rect, x: f32, y: f32) -> Option<usize> {
    (0..EyeStyle::ALL.len()).find(|&i| inside(eye_tile(ctrl, i), x, y))
}

/// The height of the calendar preview card.
pub(super) const CAL_PREVIEW_H: f32 = 232.0;

/// The height of the card at the top of the About page: the logo with three centred lines under it.
pub(super) const ABOUT_HERO_H: f32 = 152.0;

/// The sidebar row of a page.
pub(super) fn side_rect(page: Page) -> Rect {
    (12.0, SIDE_TOP + page.index() as f32 * SIDE_ITEM_H, SIDEBAR_W - 24.0, SIDE_ITEM_H - 4.0)
}

pub(super) fn inside(r: Rect, x: f32, y: f32) -> bool {
    x >= r.0 && x <= r.0 + r.2 && y >= r.1 && y <= r.1 + r.3
}
