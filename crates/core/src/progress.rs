//! Playback progress for movies and episodes: resume positions and the
//! Continue Watching list. Pure data; persistence lives in [`crate::store`].

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// Positions under this many seconds are not worth resuming.
pub const MIN_RESUME_SECS: f64 = 30.0;
/// A position at or past this share of the duration counts as finished.
pub const FINISHED_FRACTION: f64 = 0.95;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressKind {
    Movie,
    Episode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProgressEntry {
    pub source_id: String,
    pub kind: ProgressKind,
    pub id: String,
    pub title: String,
    pub position_secs: f64,
    pub duration_secs: Option<f64>,
    pub updated_at: i64,
    pub finished: bool,
    pub series_id: Option<String>,
    pub series_name: Option<String>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
}

impl ProgressEntry {
    fn past_finish_line(&self) -> bool {
        self.duration_secs.is_some_and(|duration| {
            duration > 0.0 && self.position_secs >= duration * FINISHED_FRACTION
        })
    }

    /// Where playback should resume, if it should.
    pub fn resume_position(&self) -> Option<f64> {
        (!self.finished && !self.past_finish_line() && self.position_secs >= MIN_RESUME_SECS)
            .then_some(self.position_secs)
    }

    /// The marker a list row shows for this entry.
    pub fn marker(&self) -> Marker {
        if self.finished || self.past_finish_line() {
            return Marker::Watched;
        }
        match self.resume_position() {
            Some(position_secs) => {
                let duration = self.duration_secs.filter(|d| *d > 0.0);
                Marker::InProgress {
                    position_secs,
                    fraction: duration.map(|d| (position_secs / d).clamp(0.0, 1.0)),
                    left_secs: duration.map(|d| (d - position_secs).max(0.0)),
                }
            }
            None => Marker::None,
        }
    }

    fn key_matches(&self, source_id: &str, kind: ProgressKind, id: &str) -> bool {
        self.source_id == source_id && self.kind == kind && self.id == id
    }
}

/// Total episode count of a series, learned when its details load.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeriesTotal {
    pub source_id: String,
    pub series_id: String,
    pub episodes: u32,
}

/// A live channel the user started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelVisit {
    pub source_id: String,
    pub channel_id: String,
    pub name: String,
    pub visited_at: i64,
}

/// One item of the Recently Watched list.
#[derive(Debug, Clone, PartialEq)]
pub enum Recent {
    Entry(ProgressEntry),
    Channel(ChannelVisit),
}

/// Channel visits kept per source; the oldest are dropped past this.
pub const MAX_VISITS_PER_SOURCE: usize = 50;

/// Every saved entry. Nothing is dropped; Continue Watching limits its own list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    entries: Vec<ProgressEntry>,
    #[serde(default)]
    series_totals: Vec<SeriesTotal>,
    #[serde(default)]
    visits: Vec<ChannelVisit>,
}

/// What a list row shows for an item.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Marker {
    None,
    /// Partly watched. `fraction` and `left_secs` are known only with a duration.
    InProgress {
        position_secs: f64,
        fraction: Option<f64>,
        left_secs: Option<f64>,
    },
    Watched,
}

/// Markers for every item of one source, built once when progress changes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MarkerIndex {
    movies: HashMap<String, Marker>,
    episodes: HashMap<String, Marker>,
    series: HashMap<String, Marker>,
}

impl MarkerIndex {
    pub fn movie(&self, id: &str) -> Marker {
        self.movies.get(id).copied().unwrap_or(Marker::None)
    }

    pub fn episode(&self, id: &str) -> Marker {
        self.episodes.get(id).copied().unwrap_or(Marker::None)
    }

    pub fn series(&self, id: &str) -> Marker {
        self.series.get(id).copied().unwrap_or(Marker::None)
    }
}

/// "m:ss left", or "h:mm left" from an hour up.
pub fn format_left(secs: f64) -> String {
    let secs = secs.max(0.0) as u64;
    if secs >= 3600 {
        format!("{}:{:02} left", secs / 3600, secs % 3600 / 60)
    } else {
        format!("{}:{:02} left", secs / 60, secs % 60)
    }
}

impl Progress {
    pub fn get(&self, source_id: &str, kind: ProgressKind, id: &str) -> Option<&ProgressEntry> {
        self.entries
            .iter()
            .find(|entry| entry.key_matches(source_id, kind, id))
    }

    /// Save a position. Reaching the finish line marks the entry finished.
    /// A replay of a finished item stays finished until it passes
    /// [`MIN_RESUME_SECS`]; then it becomes an in-progress resume.
    pub fn record(&mut self, mut entry: ProgressEntry) {
        let was_finished = self
            .get(&entry.source_id, entry.kind, &entry.id)
            .is_some_and(|existing| existing.finished);
        let short_replay = was_finished && entry.position_secs < MIN_RESUME_SECS;
        entry.finished = entry.past_finish_line() || short_replay;
        self.upsert(entry);
    }

    /// End of file: finished, with the position at the end.
    pub fn mark_finished(&mut self, mut entry: ProgressEntry) {
        if let Some(duration) = entry.duration_secs {
            entry.position_secs = duration;
        }
        entry.finished = true;
        self.upsert(entry);
    }

    fn upsert(&mut self, entry: ProgressEntry) {
        match self
            .entries
            .iter_mut()
            .find(|existing| existing.key_matches(&entry.source_id, entry.kind, &entry.id))
        {
            Some(existing) => *existing = entry,
            None => self.entries.push(entry),
        }
    }

    /// Remember how many episodes a series has.
    pub fn set_series_total(&mut self, source_id: &str, series_id: &str, episodes: u32) {
        match self
            .series_totals
            .iter_mut()
            .find(|total| total.source_id == source_id && total.series_id == series_id)
        {
            Some(total) => total.episodes = episodes,
            None => self.series_totals.push(SeriesTotal {
                source_id: source_id.to_owned(),
                series_id: series_id.to_owned(),
                episodes,
            }),
        }
    }

    pub fn series_total(&self, source_id: &str, series_id: &str) -> Option<u32> {
        self.series_totals
            .iter()
            .find(|total| total.source_id == source_id && total.series_id == series_id)
            .map(|total| total.episodes)
    }

    /// Row markers for one source. Series are Watched when every episode of
    /// the stored total is finished, and In progress after any viewing.
    pub fn marker_index(&self, source_id: &str) -> MarkerIndex {
        let mut index = MarkerIndex::default();
        let mut finished: HashMap<&str, u32> = HashMap::new();
        let mut started: HashMap<&str, bool> = HashMap::new();
        for entry in self.entries.iter().filter(|e| e.source_id == source_id) {
            let marker = entry.marker();
            match entry.kind {
                ProgressKind::Movie => {
                    index.movies.insert(entry.id.clone(), marker);
                }
                ProgressKind::Episode => {
                    index.episodes.insert(entry.id.clone(), marker);
                    if let Some(series_id) = entry.series_id.as_deref() {
                        match marker {
                            Marker::Watched => {
                                *finished.entry(series_id).or_default() += 1;
                                started.insert(series_id, true);
                            }
                            Marker::InProgress { .. } => {
                                started.insert(series_id, true);
                            }
                            Marker::None => {}
                        }
                    }
                }
            }
        }
        for series_id in started.keys() {
            let done = finished.get(series_id).copied().unwrap_or(0);
            let complete = self
                .series_total(source_id, series_id)
                .is_some_and(|total| total > 0 && done >= total);
            let marker = if complete {
                Marker::Watched
            } else {
                Marker::InProgress {
                    position_secs: 0.0,
                    fraction: None,
                    left_secs: None,
                }
            };
            index.series.insert((*series_id).to_owned(), marker);
        }
        index
    }

    /// Unfinished entries with a resume position, newest first.
    pub fn continue_watching(&self, source_id: &str, limit: usize) -> Vec<&ProgressEntry> {
        let mut items: Vec<&ProgressEntry> = self
            .entries
            .iter()
            .filter(|entry| entry.source_id == source_id && entry.resume_position().is_some())
            .collect();
        items.sort_by_key(|entry| std::cmp::Reverse(entry.updated_at));
        items.truncate(limit);
        items
    }

    /// Note that a channel started. A repeat visit only moves the timestamp.
    pub fn record_visit(&mut self, source_id: &str, channel_id: &str, name: &str, visited_at: i64) {
        self.visits
            .retain(|visit| !(visit.source_id == source_id && visit.channel_id == channel_id));
        self.visits.push(ChannelVisit {
            source_id: source_id.to_owned(),
            channel_id: channel_id.to_owned(),
            name: name.to_owned(),
            visited_at,
        });
        let excess = self
            .visits
            .iter()
            .filter(|visit| visit.source_id == source_id)
            .count()
            .saturating_sub(MAX_VISITS_PER_SOURCE);
        let mut dropped = 0;
        self.visits.retain(|visit| {
            let drop = visit.source_id == source_id && dropped < excess;
            dropped += usize::from(drop);
            !drop
        });
    }

    /// Movies, episodes, and channels of one source, newest first. A series
    /// shows once, with its latest episode.
    pub fn recently_watched(&self, source_id: &str, limit: usize) -> Vec<Recent> {
        let mut items: Vec<(i64, Recent)> = self
            .visits
            .iter()
            .rev()
            .filter(|visit| visit.source_id == source_id)
            .map(|visit| (visit.visited_at, Recent::Channel(visit.clone())))
            .chain(
                self.entries
                    .iter()
                    .filter(|entry| entry.source_id == source_id)
                    .map(|entry| (entry.updated_at, Recent::Entry(entry.clone()))),
            )
            .collect();
        items.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
        let mut seen = HashSet::new();
        items
            .into_iter()
            .map(|(_, item)| item)
            .filter(|item| seen.insert(item.tile_key()))
            .take(limit)
            .collect()
    }
}

impl Recent {
    fn tile_key(&self) -> (u8, String) {
        match self {
            Recent::Channel(visit) => (0, visit.channel_id.clone()),
            Recent::Entry(entry) => match (entry.kind, &entry.series_id) {
                (ProgressKind::Episode, Some(series_id)) => (1, series_id.clone()),
                (ProgressKind::Episode, None) => (2, entry.id.clone()),
                (ProgressKind::Movie, _) => (3, entry.id.clone()),
            },
        }
    }
}
