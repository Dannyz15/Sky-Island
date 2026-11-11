//! What the app does with the messages other threads send it.

use crate::*;

impl App {
    /// A notice from the system.
    pub(super) fn on_notice(&mut self, notice: sys::Notice) {
        if let sys::Notice::Volume { percent, muted } = &notice {
            self.volume.level = Some((*percent, *muted));
            // The open notch has its own volume slider: no popup for a change made there.
            if self.spring.progress > 0.5 {
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
                return;
            }
        }
        self.show_notice(notice)
    }

    /// Text or a picture was copied.
    pub(super) fn on_clip(&mut self, content: clipboard::Content) {
        if self.settings.clipboard
            && self.clip.history.add(content)
            && self.view == View::Clipboard
            && self.spring.progress > 0.0
            && let Some(w) = &self.window
        {
            w.request_redraw();
        }
    }

    /// An entry of the tray menu was chosen.
    pub(super) fn on_menu(&mut self, id: MenuId) {
        let now = Instant::now();
        match self.tray.entry(&id) {
            Some(TrayEntry::Quit) => self.quit = true,
            Some(TrayEntry::About) => {
                self.panel_wanted = true;
                self.panel_page = Some(panel::Page::About);
                self.last_tick = now;
            }
            Some(TrayEntry::Settings) => {
                self.panel_wanted = true;
                self.last_tick = now;
            }
            None => {}
        }
    }

    /// The user came back, went idle, or a full-screen app came or went.
    pub(super) fn on_presence(&mut self, presence: Presence, fullscreen: presence::Fullscreen) {
        let now = Instant::now();
        // Coming back from idle: greet first, then back to the clock.
        if self.vis.presence != Presence::Active && presence == Presence::Active {
            self.eyes.greeting_until = Some(now + GREETING);
        }
        if presence == Presence::Idle {
            self.eyes.next_blink = now + Duration::from_millis(1500);
        }
        self.vis.presence = presence;
        self.vis.fullscreen = fullscreen;
        self.apply_visibility();
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// A screen was plugged in or out or changed.
    pub(super) fn on_screens(&mut self) {
        // Place the island again, on the screen it belongs on, and read the refresh rate afresh.
        self.pacing.rate_read = None;
        self.vis.monitor_area = None;
        self.last_tick = Instant::now();
        self.refresh_edges();
        self.step();
        // The screen picker in Settings shows what is connected: draw it again for a screen that came or went.
        if let Some(p) = &mut self.panel {
            p.redraw();
        }
    }

    /// A shelf file's icon was loaded.
    pub(super) fn on_icon(&mut self, path: PathBuf, icon: Option<Pixmap>) {
        if let Some(slot) = self.shelf_ui.icons.get_mut(&path) {
            *slot = icon;
            if self.view == View::Shelf
                && self.spring.progress > 0.0
                && let Some(w) = &self.window
            {
                w.request_redraw();
            }
        }
    }

    /// The answer to a check for updates the app made by itself.
    pub(super) fn on_auto_update(&mut self, found: update::Status) {
        if let Some(status) = update::take_auto(&self.updates.status, found) {
            if let update::Status::Available { version, .. } = &status
                && self.updates.told.as_ref() != Some(version)
            {
                self.updates.told = Some(version.clone());
                self.show_notice(sys::Notice::Update { version: version.clone() });
            }
            self.updates.status = status;
            if let Some(p) = &self.panel {
                p.redraw();
            }
        }
    }

    /// The lyrics of a song were fetched.
    pub(super) fn on_lyrics(&mut self, key: String, fetched: lyrics::Fetched, online: bool) {
        if key == self.lyric.key && online == self.settings.lyrics_online {
            // A second look that finds nothing does not take away what the first one found.
            let keep = !matches!(fetched, lyrics::Fetched::Lines(_)) && matches!(self.lyric.state, LyricsState::Found(_));
            self.lyric.retry_at = None;
            self.lyric.state = match fetched {
                lyrics::Fetched::Lines(l) => LyricsState::Found(l),
                _ if keep => std::mem::replace(&mut self.lyric.state, LyricsState::None),
                lyrics::Fetched::NotFound => LyricsState::NotFound,
                lyrics::Fetched::Off => LyricsState::Off,
                lyrics::Fetched::Unreachable => {
                    self.lyric.retry_at = Some(Instant::now() + LYRICS_RETRY);
                    LyricsState::Unreachable
                }
            };
            self.lyric.manual = None;
            self.snap_lyrics();
            if self.spring.progress > 0.0
                && let Some(w) = &self.window
            {
                w.request_redraw();
            }
        }
    }

    /// The player reported a new state.
    pub(super) fn on_media(&mut self, state: MediaState) {
        let state = state.keep_timeline(&self.media, self.position(), self.play.seeked);
        // Windows is slow to be listened to the first time, so once a song plays (and the bars would follow it)
        // that is done at once, before the closed notch shows the bars.
        if state.playing && self.settings.closed_music && self.settings.closed_bars && self.settings.closed_sound {
            loopback::warm();
        }
        let key = format!("{}|{}", state.title, state.artist);
        if !state.has_session || state.title.is_empty() {
            self.lyric.key.clear();
            self.lyric.state = LyricsState::None;
            self.play.seeked = false;
        } else if key != self.lyric.key {
            self.play.seeked = false;
            self.lyric.key = key.clone();
            self.lyric.state = LyricsState::Loading;
            self.lyric.manual = None;
            self.lyric.pos = -1.0;
            self.lyric.v = 0.0;
            (self.art.track_t, self.art.track_v) = (0.0, 0.0);
            (self.lyric.duration, self.lyric.refetched) = (state.duration, false);
            lyrics::spawn_fetch(key, state.title.clone(), state.artist.clone(), state.duration, self.settings.lyrics_online, self.proxy.clone());
        } else if !self.lyric.refetched && !state.duration.is_zero() && state.duration.abs_diff(self.lyric.duration) > Duration::from_secs(2) {
            // The first look used no length or the old song's: look again, once, with the right one.
            // What is shown stays until the answer comes.
            (self.lyric.duration, self.lyric.refetched) = (state.duration, true);
            lyrics::spawn_fetch(key, state.title.clone(), state.artist.clone(), state.duration, self.settings.lyrics_online, self.proxy.clone());
        } else if self.lyric.retry_at.is_some_and(|at| Instant::now() >= at) {
            // The lyrics site could not be reached a while ago: ask again. The message stays until the
            // answer comes, and the next try is only planned if that one fails too.
            self.lyric.retry_at = None;
            self.lyric.duration = state.duration;
            lyrics::spawn_fetch(key, state.title.clone(), state.artist.clone(), state.duration, self.settings.lyrics_online, self.proxy.clone());
        }
        if let Some(art) = &state.art {
            let art_key = format!("{}|{}|{}", state.title, state.artist, art.len());
            if art_key != self.art.key {
                self.art.tint = art_tint(art);
                self.art.key = art_key;
            }
        }
        self.media = state;
        self.play.at = Instant::now();
        // Only worth repainting if the player is visible.
        if self.spring.progress > 0.0
            && let Some(w) = &self.window
        {
            w.request_redraw();
        }
    }
}
