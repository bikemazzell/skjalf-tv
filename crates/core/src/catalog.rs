//! On-demand catalog models: movies, series, seasons, and episodes.
//!
//! Models hold no stream URLs or credentials; URLs are built at play time
//! (see [`crate::xtream::movie_url`]).

use serde::{Deserialize, Serialize};

use crate::model::Group;

/// What a browsable or playable item is. Keys never collide across kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContentKind {
    Live,
    Movie,
    Series,
    Episode,
}

impl ContentKind {
    fn prefix(self) -> &'static str {
        match self {
            ContentKind::Live => "",
            ContentKind::Movie => "movie:",
            ContentKind::Series => "series:",
            ContentKind::Episode => "episode:",
        }
    }

    fn group_prefix(self) -> &'static str {
        match self {
            ContentKind::Live => "live:",
            ContentKind::Movie => "movie:",
            ContentKind::Series => "series:",
            ContentKind::Episode => "episode:",
        }
    }
}

const GROUP_PREFIX: &str = "group:";

/// Typed item identity. Live keys stay the plain channel id so saved
/// favorites keep working; other kinds carry a prefix.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContentKey {
    pub kind: ContentKind,
    pub id: String,
}

impl ContentKey {
    pub fn new(kind: ContentKind, id: &str) -> Self {
        Self {
            kind,
            id: id.to_owned(),
        }
    }

    /// The string stored in the favorites set.
    pub fn favorite_key(&self) -> String {
        format!("{}{}", self.kind.prefix(), self.id)
    }

    /// The item id of a stored favorite key of `kind`, if it is one.
    pub fn id_of_favorite(kind: ContentKind, stored: &str) -> Option<&str> {
        match kind {
            ContentKind::Live => {
                let other = [
                    ContentKind::Movie,
                    ContentKind::Series,
                    ContentKind::Episode,
                ]
                .iter()
                .any(|kind| stored.starts_with(kind.prefix()))
                    || stored.starts_with(GROUP_PREFIX);
                (!other).then_some(stored)
            }
            _ => stored.strip_prefix(kind.prefix()),
        }
    }

    /// The string stored in the favorites set for a provider group.
    pub fn group_favorite_key(kind: ContentKind, group_id: &str) -> String {
        format!("{GROUP_PREFIX}{}{group_id}", kind.group_prefix())
    }

    /// The group id of a stored group favorite of `kind`, if it is one.
    pub fn group_id_of_favorite(kind: ContentKind, stored: &str) -> Option<&str> {
        stored
            .strip_prefix(GROUP_PREFIX)?
            .strip_prefix(kind.group_prefix())
    }
}

/// Group positions sorted by name, case-insensitive; ties keep provider order.
pub fn group_name_order(groups: &[Group]) -> Vec<usize> {
    let lower: Vec<String> = groups
        .iter()
        .map(|group| group.name.to_lowercase())
        .collect();
    let mut order: Vec<usize> = (0..groups.len()).collect();
    order.sort_by(|&a, &b| lower[a].cmp(&lower[b]));
    order
}

/// The provider's own id for a local id (drops the `#n` duplicate suffix).
pub fn provider_id(id: &str) -> &str {
    id.split('#').next().unwrap_or(id)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Movie {
    pub id: String,
    pub name: String,
    pub group_id: String,
    pub icon: Option<String>,
    pub added: Option<i64>,
    pub rating: Option<String>,
    pub extension: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MovieCatalog {
    /// Identifies the source in memory; never written to the cache file.
    #[serde(skip)]
    pub source_id: String,
    pub groups: Vec<Group>,
    pub movies: Vec<Movie>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MovieInfo {
    pub description: Option<String>,
    pub genre: Option<String>,
    pub director: Option<String>,
    pub cast: Option<String>,
    pub release_date: Option<String>,
    pub duration_secs: Option<u64>,
    pub rating: Option<String>,
}

/// A provider rating with one decimal ("7.296" is "7.3", "7" is "7.0").
/// Ratings that are empty, zero, negative, or not numbers show nothing.
pub fn format_rating(rating: Option<&str>) -> Option<String> {
    let value = rating?.trim().parse::<f64>().ok()?;
    (value.is_finite() && value > 0.0).then(|| format!("{value:.1}"))
}

/// `H:MM:SS` or `M:SS` for a duration in seconds.
pub fn format_duration(secs: u64) -> String {
    let (hours, minutes, seconds) = (secs / 3600, secs % 3600 / 60, secs % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// Local date and time for a unix timestamp, for status lines.
pub fn format_unix_local(secs: u64) -> String {
    i64::try_from(secs)
        .ok()
        .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeriesItem {
    pub id: String,
    pub name: String,
    pub group_id: String,
    pub cover: Option<String>,
    pub last_modified: Option<i64>,
    pub rating: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeriesCatalog {
    /// Identifies the source in memory; never written to the cache file.
    #[serde(skip)]
    pub source_id: String,
    pub groups: Vec<Group>,
    pub series: Vec<SeriesItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Episode {
    pub id: String,
    pub number: u32,
    pub title: String,
    pub extension: String,
    pub duration_secs: Option<u64>,
    pub plot: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Season {
    pub number: u32,
    pub name: String,
    pub episodes: Vec<Episode>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SeriesInfo {
    pub plot: Option<String>,
    pub genre: Option<String>,
    pub cast: Option<String>,
    pub rating: Option<String>,
    pub release_date: Option<String>,
    pub seasons: Vec<Season>,
}
