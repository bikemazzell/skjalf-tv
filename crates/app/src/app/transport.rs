//! Player transport: focus model, seek, pause, volume steps, and the
//! controls shown in the bar. Live and on-demand playback share the bar.

use skjalf_core::catalog::{ContentKey, ContentKind};
use skjalf_core::nav::OsdKind;

use super::{App, Playback, PlayerFocus};
use crate::input::Action;
use crate::player::BufferStatus;

/// One control in the transport bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BarControl {
    Back,
    Rewind,
    PlayPause,
    Forward,
    Previous,
    Next,
    Favorite,
    Volume,
    Tracks,
}

impl BarControl {
    pub fn label(self) -> &'static str {
        match self {
            BarControl::Back => "Back",
            BarControl::Rewind => "Rewind 10 seconds",
            BarControl::PlayPause => "Play or pause",
            BarControl::Forward => "Forward 10 seconds",
            BarControl::Previous => "Previous channel",
            BarControl::Next => "Next channel",
            BarControl::Favorite => "Favorite",
            BarControl::Volume => "Volume",
            BarControl::Tracks => "Audio and subtitles",
        }
    }
}

const ON_DEMAND_CONTROLS: [BarControl; 6] = [
    BarControl::Back,
    BarControl::Rewind,
    BarControl::PlayPause,
    BarControl::Forward,
    BarControl::Volume,
    BarControl::Tracks,
];
const LIVE_CONTROLS: [BarControl; 7] = [
    BarControl::Back,
    BarControl::Previous,
    BarControl::PlayPause,
    BarControl::Next,
    BarControl::Favorite,
    BarControl::Volume,
    BarControl::Tracks,
];

/// Index of Play/Pause in both control lists; bar focus starts here.
pub const DEFAULT_CONTROL: usize = 2;
const SEEK_SECS: f64 = 10.0;
const LONG_SEEK_SECS: f64 = 60.0;
const VOLUME_STEP: i16 = 5;
/// The app reads position and duration from the engine at most this often.
const CLOCK_READ_MS: u64 = 500;
const STATUS_MS: u64 = 3000;

pub fn bar_controls(on_demand: bool) -> &'static [BarControl] {
    if on_demand {
        &ON_DEMAND_CONTROLS
    } else {
        &LIVE_CONTROLS
    }
}

/// Last position, duration, and buffer state read from the engine.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MediaClock {
    pub position: Option<f64>,
    pub duration: Option<f64>,
    pub buffer: Option<BufferStatus>,
    read_at_ms: Option<u64>,
}

impl MediaClock {
    /// True while playback waits for the buffer to fill.
    pub fn buffering(&self) -> bool {
        self.buffer.is_some_and(|buffer| buffer.waiting)
    }
}

impl App {
    /// Read the buffer state, and position and duration for the on-demand
    /// bar, rate limited.
    pub(super) fn refresh_media_clock(&mut self, now_ms: u64) {
        if self.router.current() != skjalf_core::nav::View::Player {
            return;
        }
        if self
            .media
            .read_at_ms
            .is_some_and(|at| now_ms < at + CLOCK_READ_MS)
        {
            return;
        }
        if let Some(engine) = &self.engine {
            self.media.buffer = engine.buffer_status();
            if self.is_on_demand() {
                self.media.position = engine.position();
                self.media.duration = engine.duration();
            }
        }
        self.media.read_at_ms = Some(now_ms);
    }

    pub(super) fn handle_player(&mut self, action: Action, now_ms: u64) {
        if let PlayerFocus::Tracks(row) = self.player_focus {
            return self.tracks_panel_action(row, action);
        }
        match action {
            Action::Options => return self.open_tracks_panel(),
            Action::Favorite => return self.toggle_playing_favorite(now_ms),
            Action::Epg if !self.is_on_demand() => return self.show_program_info(now_ms),
            Action::Digit(_) if self.is_on_demand() => return,
            Action::Digit(d) => {
                self.zap.push(d, now_ms);
                return self.show_zap_osd(now_ms);
            }
            Action::Epg | Action::Quit => return,
            _ => {}
        }
        match self.player_focus {
            PlayerFocus::Video => self.video_action(action, now_ms),
            PlayerFocus::Controls(index) => self.bar_action(index, action, now_ms),
            PlayerFocus::Seek => self.seek_bar_action(action, now_ms),
            PlayerFocus::Tracks(_) => {}
        }
    }

    fn video_action(&mut self, action: Action, now_ms: u64) {
        let on_demand = self.is_on_demand();
        match action {
            Action::Select if self.buffer_waiting() => self.start_now(now_ms),
            Action::Down => self.player_focus = PlayerFocus::Controls(DEFAULT_CONTROL),
            Action::Up if on_demand => self.player.show_osd(OsdKind::Info, now_ms),
            Action::Back => self.leave_player(now_ms),
            Action::Up if !on_demand => self.step_channel(true, now_ms),
            Action::ChUp if !on_demand => self.step_channel(true, now_ms),
            Action::ChDown if !on_demand => self.step_channel(false, now_ms),
            Action::Left if !on_demand => self.adjust_volume(-5, now_ms),
            Action::Right if !on_demand => self.adjust_volume(5, now_ms),
            Action::Select if !on_demand => {
                if let Some(num) = self.zap.force() {
                    self.zap_to(num, now_ms);
                } else {
                    self.player.show_osd(OsdKind::Info, now_ms);
                }
            }
            Action::Left => self.seek_by(-SEEK_SECS, now_ms),
            Action::Right => self.seek_by(SEEK_SECS, now_ms),
            Action::ChUp => self.seek_by(-LONG_SEEK_SECS, now_ms),
            Action::ChDown => self.seek_by(LONG_SEEK_SECS, now_ms),
            Action::Select => self.toggle_pause(now_ms),
            _ => {}
        }
    }

    fn bar_action(&mut self, index: usize, action: Action, now_ms: u64) {
        let controls = bar_controls(self.is_on_demand());
        let on_volume = controls.get(index) == Some(&BarControl::Volume);
        match action {
            Action::Up if on_volume => self.adjust_volume(VOLUME_STEP, now_ms),
            Action::Down if on_volume => self.adjust_volume(-VOLUME_STEP, now_ms),
            Action::Left => {
                self.player_focus =
                    PlayerFocus::Controls((index + controls.len() - 1) % controls.len())
            }
            Action::Right => {
                self.player_focus = PlayerFocus::Controls((index + 1) % controls.len())
            }
            Action::Select => {
                if let Some(control) = controls.get(index) {
                    self.activate_bar_control(*control, now_ms);
                }
            }
            Action::Down if self.is_on_demand() => self.player_focus = PlayerFocus::Seek,
            Action::Up | Action::Down | Action::Back => self.player_focus = PlayerFocus::Video,
            _ => {}
        }
        self.player.show_osd(OsdKind::Info, now_ms);
    }

    fn seek_bar_action(&mut self, action: Action, now_ms: u64) {
        match action {
            Action::Left => self.seek_by(-SEEK_SECS, now_ms),
            Action::Right => self.seek_by(SEEK_SECS, now_ms),
            Action::ChUp => self.seek_by(-LONG_SEEK_SECS, now_ms),
            Action::ChDown => self.seek_by(LONG_SEEK_SECS, now_ms),
            Action::Select => self.toggle_pause(now_ms),
            Action::Up => self.player_focus = PlayerFocus::Controls(DEFAULT_CONTROL),
            Action::Down | Action::Back => self.player_focus = PlayerFocus::Video,
            _ => {}
        }
    }

    pub fn activate_bar_control(&mut self, control: BarControl, now_ms: u64) {
        match control {
            BarControl::Back => self.leave_player(now_ms),
            BarControl::Rewind => self.seek_by(-SEEK_SECS, now_ms),
            BarControl::Forward => self.seek_by(SEEK_SECS, now_ms),
            BarControl::PlayPause => self.toggle_pause(now_ms),
            BarControl::Previous => self.step_channel(false, now_ms),
            BarControl::Next => self.step_channel(true, now_ms),
            BarControl::Favorite => self.toggle_playing_favorite(now_ms),
            BarControl::Volume => self.toggle_mute(now_ms),
            BarControl::Tracks => self.open_tracks_panel(),
        }
    }

    /// A short status line (paused, seek, volume) with the bar and title.
    pub(super) fn show_status(&mut self, text: String, now_ms: u64) {
        self.status = Some((text, now_ms + STATUS_MS));
        self.player.show_osd(OsdKind::Info, now_ms);
    }

    pub fn toggle_mute(&mut self, now_ms: u64) {
        let muted = !self.muted;
        if let Some(engine) = &mut self.engine {
            let volume = if muted { 0 } else { self.settings.volume };
            if let Err(error) = engine.set_volume(volume) {
                self.notice = Some(format!("Volume error: {error}"));
                return;
            }
        }
        self.muted = muted;
        let text = if muted {
            "Muted".to_owned()
        } else {
            format!("Volume {}%", self.settings.volume)
        };
        self.show_status(text, now_ms);
    }

    /// Play what is already buffered instead of waiting for the full buffer.
    pub fn start_now(&mut self, now_ms: u64) {
        if let Some(engine) = &mut self.engine {
            if let Err(error) = engine.start_now() {
                self.notice = Some(format!("Playback error: {error}"));
                return;
            }
        }
        if let Some(buffer) = &mut self.media.buffer {
            buffer.waiting = false;
        }
        self.show_status("Starting playback".into(), now_ms);
    }

    /// True while playback waits for the buffer, read from the engine now.
    pub fn buffer_waiting(&self) -> bool {
        self.engine
            .as_ref()
            .and_then(|engine| engine.buffer_status())
            .is_some_and(|buffer| buffer.waiting)
    }

    pub fn toggle_pause(&mut self, now_ms: u64) {
        if self.buffer_waiting() {
            return self.start_now(now_ms);
        }
        let paused = !self.paused;
        if let Some(engine) = &mut self.engine {
            if let Err(error) = engine.pause(paused) {
                self.notice = Some(format!("Playback error: {error}"));
                return;
            }
        }
        self.paused = paused;
        if !paused {
            self.media_ended = false;
        }
        let text = if paused { "Paused" } else { "Playing" };
        self.show_status(text.into(), now_ms);
    }

    pub fn seek_by(&mut self, secs: f64, now_ms: u64) {
        if !self.is_on_demand() {
            return;
        }
        if let Some(engine) = &mut self.engine {
            if let Err(error) = engine.seek_relative(secs) {
                self.notice = Some(format!("Playback error: {error}"));
                return;
            }
        }
        self.media_ended = false;
        let target = self.media.position.map(|position| {
            let limit = self.media.duration.unwrap_or(f64::MAX);
            (position + secs).clamp(0.0, limit)
        });
        self.media.position = target;
        self.show_status(format!("{secs:+.0} s"), now_ms);
    }

    /// Touch on the seek bar: jump to a fraction of the duration.
    pub fn seek_to_fraction(&mut self, fraction: f64, now_ms: u64) {
        let (Some(position), Some(duration)) = (self.media.position, self.media.duration) else {
            return;
        };
        self.player_focus = PlayerFocus::Seek;
        self.seek_by(fraction.clamp(0.0, 1.0) * duration - position, now_ms);
    }

    /// True when the playing channel or on-demand item is a favorite.
    pub fn playing_is_favorite(&self) -> bool {
        let Some(source_id) = self.source_id() else {
            return false;
        };
        let key = match &self.playback {
            Playback::OnDemand(item) => item.favorite.clone(),
            Playback::Live => match &self.player.channel_id {
                Some(id) => ContentKey::new(ContentKind::Live, id),
                None => return false,
            },
        };
        self.favorites.contains(source_id, &key.favorite_key())
    }

    fn toggle_playing_favorite(&mut self, now_ms: u64) {
        match self.playback.clone() {
            Playback::OnDemand(item) => self.toggle_favorite(&item.favorite, &item.title, now_ms),
            Playback::Live => {
                if let Some(id) = self.player.channel_id.clone() {
                    let name = self
                        .playlist
                        .as_ref()
                        .and_then(|p| p.find(&id))
                        .map(|c| c.name.clone())
                        .unwrap_or_default();
                    let key = ContentKey::new(ContentKind::Live, &id);
                    self.toggle_favorite(&key, &name, now_ms);
                }
            }
        }
    }

    fn show_program_info(&mut self, now_ms: u64) {
        self.player.show_osd(OsdKind::Epg, now_ms);
        self.epg_fetched = None; // force a fresh fetch
        self.fetch_epg(now_ms);
    }

    pub(super) fn leave_player(&mut self, now_ms: u64) {
        self.save_progress(now_ms);
        if let Some(engine) = &mut self.engine {
            if engine.stop().is_err() {
                self.notice = Some("Could not stop playback.".into());
            }
        }
        self.playback_status = super::PlaybackStatus::Idle;
        self.player_focus = PlayerFocus::Video;
        self.zap = Default::default();
        self.epg_rx = None;
        self.player.hide_osd();
        self.router.back();
    }
}
