//! Playback kinds: live channels and on-demand items (movies, episodes).

use skjalf_core::catalog::ContentKey;
use skjalf_core::nav::{OsdKind, View, ZapBuffer};

use super::{App, PlaybackStatus, PlayerFocus};

/// An on-demand item. `url` keeps `{user}`/`{pass}` placeholders until play.
#[derive(Debug, Clone, PartialEq)]
pub struct OnDemand {
    pub key: ContentKey,
    /// The title as shown while playing.
    pub title: String,
    /// The provider's own title, stored in progress records.
    pub record_title: String,
    pub url: String,
    /// What the Favorite action toggles (the movie or the series).
    pub favorite: ContentKey,
    /// Resume position in seconds; `None` plays from the start.
    pub start: Option<f64>,
    /// Series context for episodes.
    pub episode: Option<EpisodeRef>,
}

/// Which episode of which series an on-demand item is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodeRef {
    pub series_id: String,
    pub series_name: String,
    pub season: u32,
    pub number: u32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum Playback {
    #[default]
    Live,
    OnDemand(Box<OnDemand>),
}

impl App {
    pub fn is_on_demand(&self) -> bool {
        matches!(self.playback, Playback::OnDemand(_))
    }

    /// Name shown in the player overlay.
    pub fn now_playing_title(&self) -> String {
        match &self.playback {
            Playback::OnDemand(item) => item.title.clone(),
            Playback::Live => self
                .player
                .channel_id
                .as_ref()
                .and_then(|id| self.playlist.as_ref().and_then(|p| p.find(id)))
                .map_or_else(|| "Idle".to_owned(), |channel| channel.name.clone()),
        }
    }

    /// Load `url` into the engine; on failure report it and stay put.
    /// `start_wait_secs` of video buffer before playback starts (0: none).
    pub(super) fn load_media(
        &mut self,
        url: &str,
        start: Option<f64>,
        start_wait_secs: u32,
        now_ms: u64,
    ) -> bool {
        let plan = crate::player::BufferPlan {
            start_secs: f64::from(start_wait_secs),
            stall_secs: f64::from(self.settings.stall_buffer_secs),
        };
        if let Some(engine) = &mut self.engine {
            let started = engine
                .load_at(url, start)
                .and_then(|_| engine.pause(false))
                .and_then(|_| engine.set_buffering(plan))
                .and_then(|_| engine.set_volume(if self.muted { 0 } else { self.settings.volume }));
            if let Err(e) = started {
                let message = format!("Playback error: {e}");
                self.playback_status = PlaybackStatus::Failed(message.clone());
                self.notice = Some(message.clone());
                self.player.show_osd(OsdKind::Zap(message), now_ms);
                return false;
            }
        } else {
            let message = "Player engine is unavailable.".to_owned();
            self.playback_status = PlaybackStatus::Failed(message.clone());
            self.notice = Some(message);
            return false;
        }
        self.paused = false;
        true
    }

    pub(super) fn enter_player(&mut self, now_ms: u64) {
        if self.router.current() != View::Player {
            self.router.push(View::Player);
        }
        self.player_focus = PlayerFocus::Video;
        self.player.show_osd(OsdKind::Info, now_ms);
        self.zap = ZapBuffer::default();
        self.epg.clear();
        self.epg_fetched = None;
        self.epg_rx = None;
    }

    /// Play a movie or episode. Never touches the saved last channel.
    pub fn play_on_demand(&mut self, item: OnDemand, now_ms: u64) {
        if self.editing.is_some() {
            self.finish_text_edit();
        }
        let url = self.render_template(&item.url);
        if !self.load_media(&url, item.start, self.settings.start_buffer_secs, now_ms) {
            return;
        }
        self.media = Default::default();
        self.media_ended = false;
        self.last_progress_save_ms = now_ms;
        self.playback_status = PlaybackStatus::Loading;
        self.notice = Some(format!("Loading {}…", item.title));
        self.player.channel_id = None;
        self.playback = Playback::OnDemand(Box::new(item));
        self.enter_player(now_ms);
    }
}
