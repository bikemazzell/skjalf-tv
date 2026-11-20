//! Audio and subtitle tracks: the player's tracks panel. A switch lasts for
//! the playing stream only; Settings holds the languages for every stream.

use skjalf_core::tracks::{step_track, Track, TrackKind};

use super::{App, PlayerFocus};
use crate::input::Action;

impl App {
    pub fn open_tracks_panel(&mut self) {
        self.player_focus = PlayerFocus::Tracks(TrackKind::Audio);
    }

    /// The panel is modal: it takes every player action until it closes.
    pub(super) fn tracks_panel_action(&mut self, row: TrackKind, action: Action) {
        match action {
            Action::Up | Action::Down => self.player_focus = PlayerFocus::Tracks(other_row(row)),
            Action::Left | Action::Right => self.switch_track(row, action == Action::Right),
            Action::Back | Action::Select | Action::Options => {
                self.player_focus = PlayerFocus::Video
            }
            _ => {}
        }
    }

    /// Keep subtitles above the highest bottom overlay (`overlay_top`, in
    /// points from the top), or at the bottom when no overlay shows.
    pub fn keep_subtitles_above(&mut self, overlay_top: Option<f32>, screen_height: f32) {
        let position = match overlay_top {
            Some(top) if screen_height > 0.0 => {
                ((top / screen_height * 100.0).floor() as i32 - SUBTITLE_GAP).clamp(50, 100) as u8
            }
            _ => 100,
        };
        if position == self.sub_position {
            return;
        }
        self.sub_position = position;
        if let Some(engine) = &mut self.engine {
            if let Err(error) = engine.set_subtitle_position(position) {
                self.notice = Some(format!("Subtitle position error: {error}"));
            }
        }
    }

    /// Tracks of the playing file, as last reported by the engine.
    pub fn tracks(&self) -> Vec<Track> {
        self.engine
            .as_ref()
            .map(|engine| engine.tracks())
            .unwrap_or_default()
    }

    /// Step the playing file's track.
    pub fn switch_track(&mut self, kind: TrackKind, forward: bool) {
        self.player_focus = PlayerFocus::Tracks(kind);
        let Some(engine) = &mut self.engine else {
            return;
        };
        let tracks = engine.tracks();
        let Some(choice) = step_track(&tracks, kind, forward) else {
            return;
        };
        if let Err(error) = engine.select_track(kind, choice) {
            self.notice = Some(format!("Track error: {error}"));
        }
    }
}

/// Space between subtitles and the overlay above them, in percent.
const SUBTITLE_GAP: i32 = 2;

fn other_row(row: TrackKind) -> TrackKind {
    match row {
        TrackKind::Audio => TrackKind::Sub,
        TrackKind::Sub => TrackKind::Audio,
    }
}
