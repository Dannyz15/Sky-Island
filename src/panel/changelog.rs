//! The changelog that takes the place of a page: its lines (releases from GitHub) and how they are drawn.

use super::*;

/// The Back button of the changelog, and where its card starts.
pub(super) fn changelog_back(frame: Frame) -> Rect {
    (frame.x + frame.w - 96.0, 34.0, 96.0, 30.0)
}

pub(super) const CHANGELOG_TOP: f32 = 100.0;

/// What kind of line a changelog row is.
#[derive(Clone, PartialEq, Debug)]
pub(super) enum ChangeKind {
    /// The name of a release, with its date.
    Version(String),
    Heading,
    Bullet,
    Plain,
}

pub(super) struct ChangeLine {
    pub(super) kind: ChangeKind,
    pub(super) text: String,
    /// Where the line sits, from the top of the list.
    pub(super) y: f32,
}

/// How far above the bottom of the window the version number sits in the sidebar (the baseline).
pub(super) const VERSION_FROM_BOTTOM: f32 = 24.0;

impl App {
    /// The changelog as lines at their place (relative to the top of the list), and the height of all of it.
    /// `width` is how wide the text may be, in logical pixels.
    pub(super) fn changelog_lines(&self, font: &fontdue::Font, heading: &fontdue::Font, width: f32) -> (Vec<ChangeLine>, f32) {
        use crate::update::{Changelog, Note};
        // Only letters the font has are drawn, so symbols and emoji in the notes do not show as boxes.
        let usable = |text: &str| -> String { text.chars().filter(|c| c.is_whitespace() || font.lookup_glyph_index(*c) != 0).collect::<String>().trim().to_string() };
        let mut lines = Vec::new();
        let mut y = 0.0;
        let push = |lines: &mut Vec<ChangeLine>, y: &mut f32, kind: ChangeKind, text: String, step: f32| {
            lines.push(ChangeLine { kind, text, y: *y });
            *y += step;
        };
        match &self.updates.changelog {
            Changelog::Idle | Changelog::Loading => push(&mut lines, &mut y, ChangeKind::Plain, "Loading the changelog...".to_string(), 20.0),
            Changelog::Failed => push(&mut lines, &mut y, ChangeKind::Plain, "Could not load the changelog. Are you online?".to_string(), 20.0),
            Changelog::Ready(releases) => {
                for (i, release) in releases.iter().enumerate() {
                    if i > 0 {
                        y += 14.0;
                    }
                    push(&mut lines, &mut y, ChangeKind::Version(release.date.clone()), format!("Version {}", release.version), 30.0);
                    let notes = crate::update::notes_of(&release.body);
                    if notes.is_empty() {
                        push(&mut lines, &mut y, ChangeKind::Plain, "No notes for this version.".to_string(), 20.0);
                    }
                    for note in notes {
                        match note {
                            Note::Heading(h) => {
                                y += 4.0;
                                push(&mut lines, &mut y, ChangeKind::Heading, usable(&h), 22.0);
                            }
                            Note::Bullet(b) => {
                                for (k, row) in crate::wrap(font, &usable(&b), 12.0, width - 18.0).into_iter().enumerate() {
                                    push(&mut lines, &mut y, if k == 0 { ChangeKind::Bullet } else { ChangeKind::Plain }, row, 17.0);
                                }
                                y += 3.0;
                            }
                            Note::Text(t) => {
                                for row in crate::wrap(font, &usable(&t), 12.0, width - 18.0) {
                                    push(&mut lines, &mut y, ChangeKind::Plain, row, 17.0);
                                }
                                y += 3.0;
                            }
                        }
                    }
                }
            }
        }
        let _ = heading;
        lines.retain(|l| !l.text.is_empty());
        (lines, y)
    }

    /// The changelog in place of a page: its title, a Back button, and the releases in a card that scrolls.
    pub(super) fn paint_changelog(&self, pm: &mut Pixmap, panel: &Panel, font: &fontdue::Font, heading: &fontdue::Font, tint: [u8; 3], s: f32, frame: Frame, lh: f32) {
        let (white, grey) = ([255, 255, 255, 255], [160, 160, 172, 255]);
        crate::draw_text(pm, heading, "Changelog", frame.x * s, 52.0 * s, 24.0 * s, white, 400.0 * s);
        crate::draw_text(pm, font, "What changed in each version, from the project's releases on GitHub.", frame.x * s, 74.0 * s, 11.5 * s, [150, 150, 164, 255], frame.w * s);

        // The Back button.
        let (cx, cy) = panel.cursor;
        let back = changelog_back(frame);
        let hot = panel.glows.fade(KEY_SIDE + 30, inside(back, cx, cy));
        let (x, y, w, h) = (back.0 * s, back.1 * s, back.2 * s, back.3 * s);
        gfx::gradient_rounded(
            pm,
            x,
            y,
            w,
            h,
            h / 2.0,
            [tint[0], tint[1], tint[2], crate::pick(110.0, 62.0, hot) as u8],
            [tint[0], tint[1], tint[2], crate::pick(66.0, 30.0, hot) as u8],
            false,
        );
        gfx::stroke_rounded(pm, x + 0.5 * s, y + 0.5 * s, w - s, h - s, h / 2.0, s, [tint[0], tint[1], tint[2], 130]);
        let tip = [(x + 17.0 * s, y + h / 2.0 - 4.0 * s), (x + 13.0 * s, y + h / 2.0), (x + 17.0 * s, y + h / 2.0 + 4.0 * s)];
        stroke_points(pm, &tip, false, 1.6 * s, white);
        crate::draw_text_centered(pm, heading, "Back", x + w / 2.0 + 6.0 * s, y + 19.0 * s, 12.0 * s, white, w - 30.0 * s);

        // The card with the releases.
        let card = (frame.x, CHANGELOG_TOP, frame.w, lh - CHANGELOG_TOP - 18.0);
        crate::draw_card(pm, card, 14.0, s, 1.0, 0.0, tint);
        let (lines, _) = self.changelog_lines(font, heading, frame.w - 44.0);
        let (top, bottom) = (card.1 + 8.0, card.1 + card.3 - 8.0);
        let accent = [tint[0], tint[1], tint[2], 255];
        for line in &lines {
            let ly = top + 6.0 + line.y - panel.changelog_scroll;
            // Only lines that fit completely inside the card are drawn, so none is cut in half.
            if ly < top || ly + 18.0 > bottom {
                continue;
            }
            let x = card.0 + 22.0;
            match &line.kind {
                ChangeKind::Version(date) => {
                    crate::draw_text(pm, heading, &line.text, x * s, (ly + 15.0) * s, 15.0 * s, white, (frame.w - 150.0) * s);
                    if !date.is_empty() {
                        let tw = crate::text_width(font, date, 11.0 * s);
                        crate::draw_text(pm, font, date, (card.0 + card.2 - 22.0) * s - tw, (ly + 14.0) * s, 11.0 * s, grey, 100.0 * s);
                    }
                    crate::fill_rect(pm, x * s, (ly + 22.0) * s, (card.2 - 44.0) * s, s, [255, 255, 255, 22]);
                }
                ChangeKind::Heading => crate::draw_text(pm, heading, &line.text, x * s, (ly + 13.0) * s, 12.5 * s, accent, (frame.w - 44.0) * s),
                ChangeKind::Bullet => {
                    crate::fill_circle(pm, (x + 4.0) * s, (ly + 8.0) * s, 2.0 * s, accent);
                    crate::draw_text(pm, font, &line.text, (x + 18.0) * s, (ly + 12.0) * s, 12.0 * s, [220, 220, 230, 255], (frame.w - 62.0) * s);
                }
                ChangeKind::Plain => crate::draw_text(pm, font, &line.text, (x + 18.0) * s, (ly + 12.0) * s, 12.0 * s, [200, 200, 212, 255], (frame.w - 62.0) * s),
            }
        }
        // A scroll bar when there is more than fits.
        if panel.changelog_max > 0.0 {
            let bar = changelog_bar(frame, lh);
            let hot = panel.bar_drag.is_some() || bar.grabs(cx, cy);
            paint_scrollbar(pm, bar, panel.changelog_scroll, panel.changelog_max, accent, s, hot);
        }
    }
}
