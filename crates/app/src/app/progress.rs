//! Saving and reading playback progress for movies and episodes.

use skjalf_core::catalog::ContentKind;
use skjalf_core::display::{short_episode_title, visual_text};
use skjalf_core::nav::View;
use skjalf_core::progress::{ProgressEntry, ProgressKind};

use super::{App, OnDemand, Playback};

/// Save the position this often while on-demand media plays.
pub const SAVE_EVERY_MS: u64 = 10_000;
/// Continue Watching shows at most this many items.
pub const CONTINUE_LIMIT: usize = 12;

fn progress_kind(kind: ContentKind) -> Option<ProgressKind> {
    match kind {
        ContentKind::Movie => Some(ProgressKind::Movie),
        ContentKind::Episode => Some(ProgressKind::Episode),
        ContentKind::Live | ContentKind::Series => None,
    }
}

impl App {
    fn progress_entry(
        &self,
        item: &OnDemand,
        position: f64,
        duration: Option<f64>,
    ) -> Option<ProgressEntry> {
        let kind = progress_kind(item.key.kind)?;
        let episode = item.episode.as_ref();
        Some(ProgressEntry {
            source_id: self.source_id()?.to_owned(),
            kind,
            id: item.key.id.clone(),
            title: item.record_title.clone(),
            position_secs: position,
            duration_secs: duration.filter(|d| *d > 0.0),
            updated_at: i64::try_from((self.clock)()).unwrap_or(i64::MAX),
            finished: false,
            series_id: episode.map(|e| e.series_id.clone()),
            series_name: episode.map(|e| e.series_name.clone()),
            season: episode.map(|e| e.season),
            episode: episode.map(|e| e.number),
        })
    }

    pub fn resume_position(&self, kind: ProgressKind, id: &str) -> Option<f64> {
        self.progress
            .get(self.source_id()?, kind, id)?
            .resume_position()
    }

    pub fn progress_entry_for(&self, kind: ProgressKind, id: &str) -> Option<&ProgressEntry> {
        self.progress.get(self.source_id()?, kind, id)
    }

    /// Save the current position of on-demand playback (live writes nothing).
    pub(super) fn save_progress(&mut self, now_ms: u64) {
        let Playback::OnDemand(item) = self.playback.clone() else {
            return;
        };
        if self.router.current() != View::Player || self.media_ended {
            return;
        }
        let Some(engine) = &self.engine else {
            return;
        };
        let (Some(position), duration) = (engine.position(), engine.duration()) else {
            return;
        };
        if let Some(entry) = self.progress_entry(&item, position, duration) {
            self.progress.record(entry);
            self.persist_progress();
        }
        self.last_progress_save_ms = now_ms;
    }

    /// End of file: the item is finished.
    pub(super) fn finish_progress(&mut self, now_ms: u64) {
        let Playback::OnDemand(item) = self.playback.clone() else {
            return;
        };
        self.media_ended = true;
        let duration = self.engine.as_ref().and_then(|engine| engine.duration());
        let position = duration.unwrap_or_default();
        if let Some(entry) = self.progress_entry(&item, position, duration) {
            self.progress.mark_finished(entry);
            self.persist_progress();
        }
        self.last_progress_save_ms = now_ms;
    }

    pub(super) fn save_progress_periodically(&mut self, now_ms: u64) {
        if self.is_on_demand()
            && self.router.current() == View::Player
            && now_ms >= self.last_progress_save_ms + SAVE_EVERY_MS
        {
            self.save_progress(now_ms);
        }
    }

    fn persist_progress(&mut self) {
        self.refresh_continue_items();
        let failed = self
            .store
            .as_ref()
            .is_none_or(|store| store.save_progress(&self.progress).is_err());
        if failed && !self.progress_save_failed {
            self.progress_save_failed = true;
            self.notice = Some("Could not save playback progress.".into());
        }
    }

    /// Remember a series' episode count so its row can read Watched.
    pub(super) fn record_series_total(&mut self, series_id: &str, episodes: u32) {
        let Some(source_id) = self.source_id().map(str::to_owned) else {
            return;
        };
        if self.progress.series_total(&source_id, series_id) != Some(episodes) {
            self.progress
                .set_series_total(&source_id, series_id, episodes);
            self.persist_progress();
        }
    }

    pub(super) fn refresh_continue_items(&mut self) {
        self.rebuild_continue_items();
        self.continue_titles = self.continue_items.iter().map(continue_title).collect();
        // Built here, not per frame: a lookup by id scans the whole catalog.
        self.continue_art = self
            .continue_items
            .iter()
            .map(|entry| match (entry.kind, entry.series_id.as_deref()) {
                (ProgressKind::Episode, Some(series_id)) => (
                    self.series_slot.loaded.as_ref().and_then(|loaded| {
                        loaded
                            .data
                            .series
                            .iter()
                            .find(|series| series.id == series_id)?
                            .cover
                            .clone()
                    }),
                    super::frames::frame_key("series", series_id),
                ),
                _ => (
                    self.movie_slot.loaded.as_ref().and_then(|loaded| {
                        loaded
                            .data
                            .movies
                            .iter()
                            .find(|movie| movie.id == entry.id)?
                            .icon
                            .clone()
                    }),
                    super::frames::frame_key("movie", &entry.id),
                ),
            })
            .collect();
    }

    fn rebuild_continue_items(&mut self) {
        self.markers = self
            .source_id()
            .map(|source_id| self.progress.marker_index(source_id))
            .unwrap_or_default();
        self.continue_items = self
            .source_id()
            .map(|source_id| {
                self.progress
                    .continue_watching(source_id, CONTINUE_LIMIT)
                    .into_iter()
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
    }
}

/// How a Continue Watching tile names an entry: episodes read
/// "Series  E3  Title" with the repeated series name dropped.
fn continue_title(entry: &ProgressEntry) -> String {
    let text = match (&entry.series_name, entry.episode) {
        (Some(series), Some(number)) => {
            let short = short_episode_title(series, &entry.title, number);
            if short.starts_with(&format!("E{number}  ")) {
                // Records from older builds already hold "Series  E3  Title".
                format!("{series}  {short}")
            } else {
                format!("{series}  E{number}  {short}")
            }
        }
        _ => entry.title.clone(),
    };
    visual_text(&text).into_owned()
}
