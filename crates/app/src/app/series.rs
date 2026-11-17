//! Series details: seasons on the left, episodes on the right.

use skjalf_core::catalog::{ContentKey, ContentKind, Episode, SeriesInfo, SeriesItem};
use skjalf_core::display::{short_episode_title, visual_text};
use skjalf_core::nav::View;
use skjalf_core::progress::ProgressKind;
use skjalf_core::xtream;

use super::catalog_jobs::JobTag;
use super::movies::DetailInfo;
use super::playback::{EpisodeRef, OnDemand};
use super::App;
use crate::input::Action;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SeriesFocus {
    Seasons,
    #[default]
    Episodes,
    Retry,
    Favorite,
}

#[derive(Debug, Clone)]
pub struct SeriesDetails {
    pub series: SeriesItem,
    /// The title as drawn, converted once when the details open.
    pub display_name: String,
    pub info: DetailInfo<SeriesInfo>,
    pub focus: SeriesFocus,
    pub season: usize,
    pub episode: usize,
    /// Season and episode numbers to select once the info arrives.
    pending_select: Option<(u32, u32)>,
    /// Inline choice shown when an episode has a resume position.
    pub choice: Option<ResumeChoice>,
    /// Episode rows as drawn, built once when the info arrives.
    pub episode_titles: Vec<Vec<String>>,
    request: u64,
}

/// "Resume" or "Play from Start" for one episode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResumeChoice {
    pub season: usize,
    pub episode: usize,
    /// Focused button: `true` is Resume.
    pub resume_focused: bool,
}

impl SeriesDetails {
    pub fn episodes(&self) -> &[Episode] {
        match &self.info {
            DetailInfo::Ready(info) => info
                .seasons
                .get(self.season)
                .map_or(&[], |season| season.episodes.as_slice()),
            _ => &[],
        }
    }

    pub fn episode_at(&self, season: usize, episode: usize) -> Option<&Episode> {
        match &self.info {
            DetailInfo::Ready(info) => info.seasons.get(season)?.episodes.get(episode),
            _ => None,
        }
    }

    fn season_number(&self, season: usize) -> u32 {
        match &self.info {
            DetailInfo::Ready(info) => info.seasons.get(season).map_or(0, |s| s.number),
            _ => 0,
        }
    }

    fn season_count(&self) -> usize {
        match &self.info {
            DetailInfo::Ready(info) => info.seasons.len(),
            _ => 0,
        }
    }
}

impl App {
    pub(super) fn open_series_details(&mut self, item: usize) {
        let Some(series) = self
            .series_slot
            .loaded
            .as_ref()
            .and_then(|loaded| loaded.data.series.get(item))
            .cloned()
        else {
            return;
        };
        self.open_series(series, None);
    }

    /// Open details for `series`; `select` is a (season, episode) number pair.
    pub(super) fn open_series(&mut self, series: SeriesItem, select: Option<(u32, u32)>) {
        self.details_request += 1;
        let request = self.details_request;
        self.request_series_info(&series.id, request);
        self.series_details = Some(SeriesDetails {
            display_name: visual_text(&series.name).into_owned(),
            series,
            info: DetailInfo::Loading,
            focus: SeriesFocus::default(),
            season: 0,
            episode: 0,
            pending_select: select,
            choice: None,
            episode_titles: Vec::new(),
            request,
        });
        self.router.push(View::SeriesDetails);
    }

    pub(super) fn apply_series_info(
        &mut self,
        tag: JobTag,
        series_id: &str,
        result: Result<SeriesInfo, String>,
    ) {
        let current_source = self.source_id().map(str::to_owned);
        let Some(details) = &mut self.series_details else {
            return;
        };
        if current_source.as_deref() != Some(tag.source_id.as_str())
            || details.request != tag.request
            || details.series.id != series_id
        {
            return;
        }
        match result {
            Ok(info) => {
                let total: usize = info
                    .seasons
                    .iter()
                    .map(|season| season.episodes.len())
                    .sum();
                let series_id = details.series.id.clone();
                if let Some((season, episode)) = details.pending_select.take() {
                    if let Some(position) = info.seasons.iter().position(|s| s.number == season) {
                        details.season = position;
                        details.episode = info.seasons[position]
                            .episodes
                            .iter()
                            .position(|e| e.number == episode)
                            .unwrap_or(0);
                    }
                }
                details.episode_titles = info
                    .seasons
                    .iter()
                    .map(|season| {
                        season
                            .episodes
                            .iter()
                            .map(|episode| {
                                let short = short_episode_title(
                                    &details.series.name,
                                    &episode.title,
                                    episode.number,
                                );
                                visual_text(&short).replace(['\n', '\r'], " ")
                            })
                            .collect()
                    })
                    .collect();
                details.info = DetailInfo::Ready(display_series_info(info));
                details.focus = SeriesFocus::Episodes;
                self.record_series_total(&series_id, u32::try_from(total).unwrap_or(u32::MAX));
            }
            Err(_) => {
                details.info = DetailInfo::Unavailable;
                details.focus = SeriesFocus::Retry;
            }
        }
    }

    pub fn retry_series_info(&mut self) {
        let Some(details) = &mut self.series_details else {
            return;
        };
        self.details_request += 1;
        details.request = self.details_request;
        details.info = DetailInfo::Loading;
        details.focus = SeriesFocus::Episodes;
        let (id, request) = (details.series.id.clone(), details.request);
        self.request_series_info(&id, request);
    }

    pub(super) fn handle_series_details(&mut self, action: Action, now_ms: u64) {
        if let Some(choice) = self.series_details.as_ref().and_then(|d| d.choice) {
            return self.handle_resume_choice(choice, action, now_ms);
        }
        match action {
            Action::Up | Action::Down => self.series_details_step(action, false),
            Action::Left | Action::Right => self.series_details_horizontal(action),
            Action::Select => self.series_details_select(now_ms),
            Action::Favorite => self.toggle_series_favorite(now_ms),
            Action::Back => {
                self.router.back();
            }
            _ => {}
        }
    }

    /// Held-direction repeat inside the season or episode list.
    pub(super) fn series_details_step(&mut self, action: Action, repeated: bool) {
        let forward = action == Action::Down;
        let Some(details) = &mut self.series_details else {
            return;
        };
        let wrap = |value: usize, len: usize| {
            if forward {
                (value + 1) % len
            } else {
                (value + len - 1) % len
            }
        };
        let at_top = |value: usize| !forward && value == 0;
        match details.focus {
            SeriesFocus::Seasons if at_top(details.season) => {
                if !repeated {
                    details.focus = SeriesFocus::Favorite;
                }
            }
            SeriesFocus::Seasons => {
                let len = details.season_count();
                if len > 0 {
                    details.season = wrap(details.season, len);
                    details.episode = 0;
                }
            }
            SeriesFocus::Episodes if at_top(details.episode) => {
                if !repeated {
                    details.focus = SeriesFocus::Favorite;
                }
            }
            SeriesFocus::Episodes => {
                let len = details.episodes().len();
                if len > 0 {
                    details.episode = wrap(details.episode, len);
                }
            }
            SeriesFocus::Retry => {
                if !forward {
                    details.focus = SeriesFocus::Favorite;
                }
            }
            SeriesFocus::Favorite if forward && !repeated => {
                details.focus = match details.info {
                    DetailInfo::Unavailable => SeriesFocus::Retry,
                    DetailInfo::Ready(_) if !details.episodes().is_empty() => SeriesFocus::Episodes,
                    DetailInfo::Ready(_) => SeriesFocus::Seasons,
                    DetailInfo::Loading => SeriesFocus::Favorite,
                };
            }
            SeriesFocus::Favorite => {}
        }
    }

    fn series_details_horizontal(&mut self, action: Action) {
        let Some(details) = &mut self.series_details else {
            return;
        };
        let has_episodes = !details.episodes().is_empty();
        details.focus = match (details.focus, action) {
            (SeriesFocus::Seasons, Action::Right) if has_episodes => SeriesFocus::Episodes,
            (SeriesFocus::Episodes, Action::Left) if details.season_count() > 0 => {
                SeriesFocus::Seasons
            }
            (focus, _) => focus,
        };
    }

    fn handle_resume_choice(&mut self, choice: ResumeChoice, action: Action, now_ms: u64) {
        match action {
            Action::Left | Action::Right | Action::Up | Action::Down => {
                if let Some(details) = &mut self.series_details {
                    details.choice = Some(ResumeChoice {
                        resume_focused: !choice.resume_focused,
                        ..choice
                    });
                }
            }
            Action::Select => self.confirm_resume_choice(choice.resume_focused, now_ms),
            Action::Back => {
                if let Some(details) = &mut self.series_details {
                    details.choice = None;
                }
            }
            _ => {}
        }
    }

    /// Play the episode behind the inline choice, resuming or from the start.
    pub fn confirm_resume_choice(&mut self, resume: bool, now_ms: u64) {
        let Some(choice) = self.series_details.as_mut().and_then(|d| d.choice.take()) else {
            return;
        };
        let start = resume
            .then(|| self.episode_resume(choice.season, choice.episode))
            .flatten();
        self.play_episode(choice.season, choice.episode, start, now_ms);
    }

    /// Resume position of an episode of the open series.
    pub fn episode_resume(&self, season: usize, episode: usize) -> Option<f64> {
        let chosen = self.series_details.as_ref()?.episode_at(season, episode)?;
        self.resume_position(ProgressKind::Episode, &chosen.id)
    }

    /// Play the episode, or ask first when it has a resume position.
    fn start_episode(&mut self, season: usize, episode: usize, now_ms: u64) {
        if self.episode_resume(season, episode).is_some() {
            if let Some(details) = &mut self.series_details {
                details.choice = Some(ResumeChoice {
                    season,
                    episode,
                    resume_focused: true,
                });
            }
        } else {
            self.play_episode(season, episode, None, now_ms);
        }
    }

    fn series_details_select(&mut self, now_ms: u64) {
        let Some(details) = &self.series_details else {
            return;
        };
        match details.focus {
            SeriesFocus::Seasons => self.series_details_horizontal(Action::Right),
            SeriesFocus::Retry => self.retry_series_info(),
            SeriesFocus::Favorite => self.toggle_series_favorite(now_ms),
            SeriesFocus::Episodes => {
                let (season, episode) = (details.season, details.episode);
                self.start_episode(season, episode, now_ms);
            }
        }
    }

    /// Touch selection of a season row.
    pub fn select_season(&mut self, season: usize) {
        if let Some(details) = &mut self.series_details {
            if season < details.season_count() {
                details.choice = None;
                details.season = season;
                details.episode = 0;
                details.focus = SeriesFocus::Seasons;
            }
        }
    }

    /// Touch selection of an episode row; plays it.
    pub fn click_episode(&mut self, episode: usize, now_ms: u64) {
        let Some(details) = &mut self.series_details else {
            return;
        };
        if episode >= details.episodes().len() {
            return;
        }
        details.choice = None;
        details.episode = episode;
        details.focus = SeriesFocus::Episodes;
        let season = details.season;
        self.start_episode(season, episode, now_ms);
    }

    pub fn series_is_favorite(&self) -> bool {
        let (Some(details), Some(source_id)) = (&self.series_details, self.source_id()) else {
            return false;
        };
        let key = ContentKey::new(ContentKind::Series, &details.series.id);
        self.favorites.contains(source_id, &key.favorite_key())
    }

    pub fn toggle_series_favorite(&mut self, now_ms: u64) {
        if let Some(details) = &self.series_details {
            let key = ContentKey::new(ContentKind::Series, &details.series.id);
            let name = details.series.name.clone();
            self.toggle_favorite(&key, &name, now_ms);
        }
    }

    fn play_episode(&mut self, season: usize, episode: usize, start: Option<f64>, now_ms: u64) {
        if let Some(details) = &mut self.series_details {
            details.choice = None;
        }
        let (Some(details), Some(source)) = (&self.series_details, &self.settings.source) else {
            return;
        };
        let Some(chosen) = details.episode_at(season, episode) else {
            return;
        };
        let season_number = details.season_number(season);
        let Ok(url) = xtream::episode_url(source, chosen) else {
            return;
        };
        let item = OnDemand {
            key: ContentKey::new(ContentKind::Episode, &chosen.id),
            title: format!(
                "{}  E{}  {}",
                details.series.name,
                chosen.number,
                short_episode_title(&details.series.name, &chosen.title, chosen.number)
            ),
            record_title: chosen.title.clone(),
            url,
            favorite: ContentKey::new(ContentKind::Series, &details.series.id),
            start,
            episode: Some(EpisodeRef {
                series_id: details.series.id.clone(),
                series_name: details.series.name.clone(),
                season: season_number,
                number: chosen.number,
            }),
        };
        self.play_on_demand(item, now_ms);
    }
}

/// Convert the free-text fields once, when the info arrives. Episode titles
/// stay logical: they feed play titles and are converted when drawn.
fn display_series_info(mut info: SeriesInfo) -> SeriesInfo {
    // The plot stays logical: the view wraps it before reordering.
    for field in [&mut info.genre, &mut info.cast, &mut info.release_date]
        .into_iter()
        .flatten()
    {
        *field = visual_text(field).into_owned();
    }
    for season in &mut info.seasons {
        season.name = visual_text(&season.name).into_owned();
    }
    info
}
