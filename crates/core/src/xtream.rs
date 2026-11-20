//! Xtream Codes API client (standard `player_api.php`), I/O behind a trait.

use crate::catalog::{
    provider_id, Episode, Movie, MovieCatalog, MovieInfo, Season, SeriesCatalog, SeriesInfo,
    SeriesItem,
};
use crate::model::{
    Activation, Channel, EpgEntry, Group, Playlist, Source, SourceKind, StreamFormat,
};
use base64::Engine;
use chrono::{NaiveDate, NaiveDateTime, Utc};
use serde::{Deserialize, Deserializer};
use std::collections::HashSet;
use std::time::Duration;
use thiserror::Error;

/// Size and time bounds for one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchLimits {
    pub max_bytes: usize,
    pub timeout: Duration,
}

impl FetchLimits {
    pub const DEFAULT: Self = Self {
        max_bytes: 64 * 1024 * 1024,
        timeout: Duration::from_secs(90),
    };
    /// Movie and series lists reach tens of megabytes.
    pub const CATALOG: Self = Self {
        max_bytes: 256 * 1024 * 1024,
        timeout: Duration::from_secs(240),
    };
    pub const DETAILS: Self = Self {
        max_bytes: 8 * 1024 * 1024,
        timeout: Duration::from_secs(30),
    };
}

/// Network abstraction so the client is unit-testable without a socket.
pub trait HttpFetcher {
    fn get(&self, url: &str) -> Result<String, FetchError>;

    fn get_with(&self, url: &str, _limits: FetchLimits) -> Result<String, FetchError> {
        self.get(url)
    }

    /// Raw bytes (images), at most `max_bytes`.
    fn get_bytes(&self, _url: &str, _max_bytes: usize) -> Result<Vec<u8>, FetchError> {
        Err(FetchError("Downloads are not supported here.".into()))
    }
}

#[derive(Debug, Error, PartialEq)]
#[error("http error: {0}")]
pub struct FetchError(pub String);

#[derive(Debug, Error)]
pub enum XtreamError {
    #[error("network: {0}")]
    Network(#[from] FetchError),
    #[error("bad response: {0}")]
    BadResponse(String),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not an Xtream source")]
    NotXtream,
}

fn base_url(source: &Source) -> Result<&str, XtreamError> {
    match &source.kind {
        SourceKind::Xtream { host, .. } => Ok(host.trim_end_matches('/')),
        _ => Err(XtreamError::NotXtream),
    }
}

fn creds(source: &Source) -> Result<(&str, &str), XtreamError> {
    match &source.kind {
        SourceKind::Xtream {
            username, password, ..
        } => Ok((username, password)),
        _ => Err(XtreamError::NotXtream),
    }
}

use crate::template::encode_component as enc;

fn api_url(
    source: &Source,
    action: Option<&str>,
    extra: &[(&str, &str)],
) -> Result<String, XtreamError> {
    let host = base_url(source)?;
    let (user, pass) = creds(source)?;
    let mut url = format!(
        "{host}/player_api.php?username={}&password={}",
        enc(user),
        enc(pass)
    );
    if let Some(action) = action {
        url.push_str(&format!("&action={action}"));
    }
    for (k, v) in extra {
        url.push_str(&format!("&{k}={}", enc(v)));
    }
    Ok(url)
}

fn fetch_action(
    f: &dyn HttpFetcher,
    source: &Source,
    action: &str,
    extra: &[(&str, &str)],
) -> Result<serde_json::Value, XtreamError> {
    let body = f.get(&api_url(source, Some(action), extra)?)?;
    serde_json::from_str(&body).map_err(|e| XtreamError::BadResponse(format!("{action}: {e}")))
}

/// Fetch and parse straight into a typed value, without a `Value` tree.
fn fetch_typed<T: serde::de::DeserializeOwned>(
    f: &dyn HttpFetcher,
    source: &Source,
    action: &str,
    extra: &[(&str, &str)],
    limits: FetchLimits,
) -> Result<T, XtreamError> {
    let body = f.get_with(&api_url(source, Some(action), extra)?, limits)?;
    serde_json::from_str(&body).map_err(|e| XtreamError::BadResponse(format!("{action}: {e}")))
}

// -- login ------------------------------------------------------------------

#[derive(Deserialize)]
struct LoginResponse {
    #[serde(default)]
    user_info: UserInfo,
}

#[derive(Default, Deserialize)]
struct UserInfo {
    #[serde(default)]
    status: String,
    #[serde(default)]
    exp_date: Option<serde_json::Value>,
    #[serde(default)]
    active_cons: Option<serde_json::Value>,
    #[serde(default)]
    max_connections: Option<serde_json::Value>,
}

fn count(value: &Option<serde_json::Value>) -> Option<u64> {
    match value {
        Some(serde_json::Value::Number(n)) => n.as_u64(),
        Some(serde_json::Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    }
}

/// True when the account has a connection to spare. Unknown limits allow it.
pub fn free_connection(f: &dyn HttpFetcher, source: &Source) -> Result<bool, XtreamError> {
    let body = f.get(&api_url(source, None, &[])?)?;
    let resp: LoginResponse =
        serde_json::from_str(&body).map_err(|e| XtreamError::BadResponse(format!("login: {e}")))?;
    let info = &resp.user_info;
    Ok(match count(&info.max_connections) {
        Some(max) if max > 0 => count(&info.active_cons).unwrap_or(0) < max,
        _ => true,
    })
}

fn exp_date(info: &UserInfo) -> Option<NaiveDate> {
    let raw = match &info.exp_date {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => return None,
    };
    let secs: i64 = raw.parse().ok()?;
    chrono::DateTime::from_timestamp(secs, 0).map(|t| t.naive_utc().date())
}

/// What the login reports about the account.
#[derive(Debug, Clone, PartialEq)]
pub struct Account {
    pub activation: Activation,
    /// The day the account expires; `None` when the provider sends none.
    pub expires: Option<NaiveDate>,
}

/// Check credentials → [`Activation`].
pub fn login(f: &dyn HttpFetcher, source: &Source) -> Result<Activation, XtreamError> {
    account(f, source).map(|account| account.activation)
}

/// Check credentials and read the expiry date.
pub fn account(f: &dyn HttpFetcher, source: &Source) -> Result<Account, XtreamError> {
    let body = f.get(&api_url(source, None, &[])?)?;
    let resp: LoginResponse =
        serde_json::from_str(&body).map_err(|e| XtreamError::BadResponse(format!("login: {e}")))?;
    let info = &resp.user_info;
    Ok(Account {
        activation: activation(info),
        expires: exp_date(info),
    })
}

/// Footer text for the account's expiry, and whether it needs attention
/// (7 days or less, or already expired).
pub fn expiry_notice(expires: Option<NaiveDate>, today: NaiveDate) -> (String, bool) {
    let Some(date) = expires else {
        return ("No expiry date".into(), false);
    };
    let days = (date - today).num_days();
    match days {
        ..0 => (format!("Account expired on {date}"), true),
        0 => ("Account expires today".into(), true),
        1 => (format!("Account expires {date} (1 day)"), true),
        _ => (format!("Account expires {date} ({days} days)"), days <= 7),
    }
}

/// [`expiry_notice`] for today's local date.
pub fn expiry_notice_now(expires: Option<NaiveDate>) -> (String, bool) {
    expiry_notice(expires, chrono::Local::now().date_naive())
}

fn activation(info: &UserInfo) -> Activation {
    let expired = || exp_date(info).unwrap_or_else(|| NaiveDate::from_ymd_opt(1970, 1, 1).unwrap());
    match info.status.as_str() {
        "Active" => match exp_date(info) {
            Some(date) if date < Utc::now().date_naive() => Activation::Expired { date },
            _ => Activation::Active,
        },
        "Expired" => Activation::Expired { date: expired() },
        "Banned" => Activation::Banned,
        _ => Activation::Invalid,
    }
}

// -- playlist ---------------------------------------------------------------

#[derive(Deserialize)]
struct Category {
    #[serde(default)]
    category_id: serde_json::Value,
    #[serde(default)]
    category_name: String,
}

#[derive(Deserialize)]
struct Stream {
    #[serde(default)]
    name: String,
    #[serde(default)]
    stream_id: serde_json::Value,
    #[serde(default)]
    stream_icon: Option<String>,
    #[serde(default)]
    num: serde_json::Value,
    #[serde(default)]
    category_id: serde_json::Value,
}

fn as_str(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// Fetch categories + live streams and build a [`Playlist`] whose channel
/// URLs carry `{user}`/`{pass}` placeholders (filled at play time).
pub fn playlist(f: &dyn HttpFetcher, source: &Source) -> Result<Playlist, XtreamError> {
    let host = base_url(source)?.to_owned();
    let (user, pass) = creds(source)?;
    let fmt = match source.stream_format {
        StreamFormat::Ts => "ts",
        StreamFormat::M3u8 => "m3u8",
    };

    let cats: Vec<Category> =
        serde_json::from_value(fetch_action(f, source, "get_live_categories", &[])?)?;
    let streams: Vec<Stream> =
        serde_json::from_value(fetch_action(f, source, "get_live_streams", &[])?)?;

    let mut groups: Vec<Group> = cats
        .into_iter()
        .map(|c| Group {
            id: as_str(&c.category_id),
            name: c.category_name,
        })
        .collect();

    let mut seen_ids = std::collections::HashSet::new();
    let mut ungrouped_id = "__skjalf_ungrouped__".to_owned();
    while groups.iter().any(|group| group.id == ungrouped_id) {
        ungrouped_id.push('_');
    }
    let mut has_ungrouped = false;
    let channels = streams
        .into_iter()
        .filter_map(|s| {
            let stream_id = as_str(&s.stream_id);
            if stream_id.is_empty() {
                return None;
            }
            let mut id = stream_id.clone();
            let mut copy = 1;
            while !seen_ids.insert(id.clone()) {
                copy += 1;
                id = format!("{stream_id}#{copy}");
            }
            let num = match &s.num {
                serde_json::Value::Number(n) => n
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .filter(|n| *n > 0),
                serde_json::Value::String(st) => st.parse::<u32>().ok().filter(|n| *n > 0),
                _ => None,
            };
            let category_id = as_str(&s.category_id);
            let group_id =
                if !category_id.is_empty() && groups.iter().any(|group| group.id == category_id) {
                    category_id
                } else {
                    has_ungrouped = true;
                    ungrouped_id.clone()
                };
            Some(Channel {
                url: format!("{host}/live/{{user}}/{{pass}}/{stream_id}.{fmt}"),
                id,
                name: s.name,
                num,
                logo: s.stream_icon.filter(|l| !l.is_empty()),
                group_id,
            })
        })
        .collect();
    if has_ungrouped {
        groups.push(Group {
            id: ungrouped_id,
            name: "Ungrouped".into(),
        });
    }

    // Credentials are templated, not interpolated, so playlists never persist them.
    let _ = (user, pass);
    Ok(Playlist {
        source: source.clone(),
        groups,
        channels,
    })
}

// -- EPG --------------------------------------------------------------------

#[derive(Deserialize)]
struct ShortEpgResponse {
    #[serde(default)]
    epg_listings: Vec<RawListing>,
}

#[derive(Deserialize)]
struct RawListing {
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    start: String,
    #[serde(default)]
    end: String,
}

fn decode_b64(s: &str) -> String {
    base64::engine::general_purpose::STANDARD
        .decode(s.trim().as_bytes())
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_default()
}

/// `get_short_epg` now/next entries (base64 title/description decoded).
pub fn short_epg(
    f: &dyn HttpFetcher,
    source: &Source,
    stream_id: &str,
    limit: u32,
) -> Result<Vec<EpgEntry>, XtreamError> {
    let body = fetch_action(
        f,
        source,
        "get_short_epg",
        &[("stream_id", stream_id), ("limit", &limit.to_string())],
    )?;
    let resp: ShortEpgResponse = serde_json::from_value(body)?;
    Ok(resp
        .epg_listings
        .into_iter()
        .map(|l| EpgEntry {
            title: decode_b64(&l.title),
            description: decode_b64(&l.description),
            start: NaiveDateTime::parse_from_str(&l.start, "%Y-%m-%d %H:%M:%S").unwrap_or_default(),
            end: NaiveDateTime::parse_from_str(&l.end, "%Y-%m-%d %H:%M:%S").unwrap_or_default(),
        })
        .collect())
}

// -- typed on-demand parsing --------------------------------------------------

/// A JSON scalar that servers send as a string, an integer, or a float.
#[derive(Deserialize)]
#[serde(untagged)]
enum Flex {
    Str(String),
    Int(i64),
    Float(f64),
    Other(serde::de::IgnoredAny),
}

impl Flex {
    /// Trimmed text; empty strings and non-scalars are missing.
    fn text(self) -> Option<String> {
        match self {
            Flex::Str(text) => {
                let trimmed = text.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_owned())
            }
            Flex::Int(n) => Some(n.to_string()),
            Flex::Float(n) => Some(n.to_string()),
            Flex::Other(_) => None,
        }
    }

    fn number(self) -> Option<i64> {
        self.text().and_then(|text| text.parse().ok())
    }
}

fn text(value: Option<Flex>) -> Option<String> {
    value.and_then(Flex::text)
}

/// Accept the expected shape, or take the default for anything else (many
/// servers send `[]` where an object belongs).
fn lenient<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Lenient<T> {
        Value(T),
        Other(serde::de::IgnoredAny),
    }
    Ok(match Lenient::<T>::deserialize(deserializer)? {
        Lenient::Value(value) => value,
        Lenient::Other(_) => T::default(),
    })
}

#[derive(Deserialize)]
struct RawCategory {
    #[serde(default)]
    category_id: Option<Flex>,
    #[serde(default)]
    category_name: Option<Flex>,
}

/// Resolves item categories to groups, with an Ungrouped group for orphans
/// and the `#n` suffix rule for duplicate ids.
struct GroupResolver {
    groups: Vec<Group>,
    known: HashSet<String>,
    ungrouped_id: String,
    has_ungrouped: bool,
    seen_ids: HashSet<String>,
}

impl GroupResolver {
    fn new(categories: Vec<RawCategory>) -> Self {
        let groups: Vec<Group> = categories
            .into_iter()
            .map(|category| Group {
                id: text(category.category_id).unwrap_or_default(),
                name: text(category.category_name).unwrap_or_default(),
            })
            .collect();
        let known: HashSet<String> = groups.iter().map(|group| group.id.clone()).collect();
        let mut ungrouped_id = "__skjalf_ungrouped__".to_owned();
        while known.contains(&ungrouped_id) {
            ungrouped_id.push('_');
        }
        Self {
            groups,
            known,
            ungrouped_id,
            has_ungrouped: false,
            seen_ids: HashSet::new(),
        }
    }

    fn group_for(&mut self, category_id: Option<String>) -> String {
        match category_id {
            Some(id) if self.known.contains(&id) => id,
            _ => {
                self.has_ungrouped = true;
                self.ungrouped_id.clone()
            }
        }
    }

    fn unique_id(&mut self, provider_id: &str) -> String {
        let mut id = provider_id.to_owned();
        let mut copy = 1;
        while !self.seen_ids.insert(id.clone()) {
            copy += 1;
            id = format!("{provider_id}#{copy}");
        }
        id
    }

    fn finish(mut self) -> Vec<Group> {
        if self.has_ungrouped {
            self.groups.push(Group {
                id: self.ungrouped_id,
                name: "Ungrouped".into(),
            });
        }
        self.groups
    }
}

const DEFAULT_EXTENSION: &str = "mp4";

#[derive(Deserialize)]
struct RawMovie {
    #[serde(default)]
    name: Option<Flex>,
    #[serde(default)]
    stream_id: Option<Flex>,
    #[serde(default)]
    stream_icon: Option<Flex>,
    #[serde(default)]
    rating: Option<Flex>,
    #[serde(default)]
    added: Option<Flex>,
    #[serde(default)]
    category_id: Option<Flex>,
    #[serde(default)]
    container_extension: Option<Flex>,
}

/// Fetch movie categories and the movie list as a typed catalog.
pub fn movie_catalog(f: &dyn HttpFetcher, source: &Source) -> Result<MovieCatalog, XtreamError> {
    let categories: Vec<RawCategory> =
        fetch_typed(f, source, "get_vod_categories", &[], FetchLimits::DEFAULT)?;
    let raw: Vec<RawMovie> = fetch_typed(f, source, "get_vod_streams", &[], FetchLimits::CATALOG)?;
    let mut resolver = GroupResolver::new(categories);
    let mut movies = Vec::with_capacity(raw.len());
    for movie in raw {
        let Some(stream_id) = text(movie.stream_id) else {
            continue;
        };
        movies.push(Movie {
            id: resolver.unique_id(&stream_id),
            name: text(movie.name).unwrap_or_else(|| "Untitled".into()),
            group_id: resolver.group_for(text(movie.category_id)),
            icon: text(movie.stream_icon),
            added: movie.added.and_then(Flex::number),
            rating: text(movie.rating),
            extension: text(movie.container_extension).unwrap_or_else(|| DEFAULT_EXTENSION.into()),
        });
    }
    Ok(MovieCatalog {
        source_id: source.id.clone(),
        groups: resolver.finish(),
        movies,
    })
}

#[derive(Deserialize, Default)]
struct RawMovieMeta {
    #[serde(default)]
    description: Option<Flex>,
    #[serde(default)]
    plot: Option<Flex>,
    #[serde(default)]
    genre: Option<Flex>,
    #[serde(default)]
    director: Option<Flex>,
    #[serde(default)]
    cast: Option<Flex>,
    #[serde(default)]
    actors: Option<Flex>,
    #[serde(default)]
    releasedate: Option<Flex>,
    #[serde(default)]
    duration_secs: Option<Flex>,
    #[serde(default)]
    rating: Option<Flex>,
    #[serde(default)]
    movie_image: Option<Flex>,
    #[serde(default)]
    cover_big: Option<Flex>,
    #[serde(default)]
    youtube_trailer: Option<Flex>,
}

#[derive(Deserialize, Default)]
struct RawMovieInfo {
    #[serde(default, deserialize_with = "lenient")]
    info: RawMovieMeta,
}

/// Fetch one movie's details. Missing or `[]` metadata yields empty fields.
pub fn movie_info(
    f: &dyn HttpFetcher,
    source: &Source,
    movie_id: &str,
) -> Result<MovieInfo, XtreamError> {
    let response: RawMovieInfo = fetch_typed(
        f,
        source,
        "get_vod_info",
        &[("vod_id", provider_id(movie_id))],
        FetchLimits::DETAILS,
    )?;
    let meta = response.info;
    Ok(MovieInfo {
        description: text(meta.description).or_else(|| text(meta.plot)),
        genre: text(meta.genre),
        director: text(meta.director),
        cast: text(meta.cast).or_else(|| text(meta.actors)),
        release_date: text(meta.releasedate),
        duration_secs: meta
            .duration_secs
            .and_then(Flex::number)
            .and_then(|secs| u64::try_from(secs).ok())
            .filter(|secs| *secs > 0),
        rating: text(meta.rating),
        poster: image_url(meta.movie_image).or_else(|| image_url(meta.cover_big)),
        trailer: text(meta.youtube_trailer).and_then(|raw| youtube_id(&raw)),
    })
}

/// A YouTube video id from a bare id or a youtube.com or youtu.be link. The
/// id goes into a URL, so it must be the id characters only.
fn youtube_id(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let lower = raw.to_ascii_lowercase();
    let after = |marker: &str| {
        lower
            .find(marker)
            .map(|at| &raw[at + marker.len()..])
            .map(|rest| rest.split(['&', '?', '#', '/']).next().unwrap_or(""))
    };
    let id = if ["https://youtu.be/", "http://youtu.be/"]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
    {
        after("youtu.be/")?
    } else if [
        "https://www.youtube.com/watch?",
        "https://youtube.com/watch?",
        "http://www.youtube.com/watch?",
        "http://youtube.com/watch?",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
    {
        after("v=")?
    } else {
        raw
    };
    let valid = (6..=20).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    valid.then(|| id.to_owned())
}

/// An art URL the app may download: `http` or `https` only.
fn image_url(value: Option<Flex>) -> Option<String> {
    text(value).filter(|url| {
        let lower = url.to_ascii_lowercase();
        lower.starts_with("http://") || lower.starts_with("https://")
    })
}

/// Movie stream URL with `{user}`/`{pass}` placeholders for play time.
pub fn movie_url(source: &Source, movie: &Movie) -> Result<String, XtreamError> {
    let host = base_url(source)?;
    Ok(format!(
        "{host}/movie/{{user}}/{{pass}}/{}.{}",
        provider_id(&movie.id),
        movie.extension
    ))
}

// -- series -------------------------------------------------------------------

#[derive(Deserialize)]
struct RawSeries {
    #[serde(default)]
    name: Option<Flex>,
    #[serde(default)]
    series_id: Option<Flex>,
    #[serde(default)]
    cover: Option<Flex>,
    #[serde(default)]
    rating: Option<Flex>,
    #[serde(default)]
    last_modified: Option<Flex>,
    #[serde(default)]
    category_id: Option<Flex>,
}

/// Fetch series categories and the series list as a typed catalog. Episode
/// lists are never fetched here.
pub fn series_catalog(f: &dyn HttpFetcher, source: &Source) -> Result<SeriesCatalog, XtreamError> {
    let categories: Vec<RawCategory> = fetch_typed(
        f,
        source,
        "get_series_categories",
        &[],
        FetchLimits::DEFAULT,
    )?;
    let raw: Vec<RawSeries> = fetch_typed(f, source, "get_series", &[], FetchLimits::CATALOG)?;
    let mut resolver = GroupResolver::new(categories);
    let mut series = Vec::with_capacity(raw.len());
    for item in raw {
        let Some(series_id) = text(item.series_id) else {
            continue;
        };
        series.push(SeriesItem {
            id: resolver.unique_id(&series_id),
            name: text(item.name).unwrap_or_else(|| "Untitled".into()),
            group_id: resolver.group_for(text(item.category_id)),
            cover: text(item.cover),
            last_modified: item.last_modified.and_then(Flex::number),
            rating: text(item.rating),
        });
    }
    Ok(SeriesCatalog {
        source_id: source.id.clone(),
        groups: resolver.finish(),
        series,
    })
}

#[derive(Deserialize, Default)]
struct RawSeriesMeta {
    #[serde(default)]
    plot: Option<Flex>,
    #[serde(default)]
    genre: Option<Flex>,
    #[serde(default)]
    cast: Option<Flex>,
    #[serde(default)]
    rating: Option<Flex>,
    #[serde(default, rename = "releaseDate")]
    release_date_camel: Option<Flex>,
    #[serde(default)]
    release_date: Option<Flex>,
}

#[derive(Deserialize)]
struct RawSeason {
    #[serde(default)]
    season_number: Option<Flex>,
    #[serde(default)]
    name: Option<Flex>,
    #[serde(default)]
    cover_big: Option<Flex>,
    #[serde(default)]
    cover: Option<Flex>,
    #[serde(default)]
    cover_tmdb: Option<Flex>,
}

#[derive(Deserialize, Default)]
struct RawEpisodeMeta {
    #[serde(default)]
    duration_secs: Option<Flex>,
    #[serde(default)]
    plot: Option<Flex>,
    #[serde(default)]
    movie_image: Option<Flex>,
}

#[derive(Deserialize)]
struct RawEpisode {
    #[serde(default)]
    id: Option<Flex>,
    #[serde(default)]
    episode_num: Option<Flex>,
    #[serde(default)]
    title: Option<Flex>,
    #[serde(default)]
    container_extension: Option<Flex>,
    #[serde(default)]
    season: Option<Flex>,
    #[serde(default, deserialize_with = "lenient")]
    info: RawEpisodeMeta,
}

#[derive(Deserialize, Default)]
struct RawSeriesInfo {
    #[serde(default, deserialize_with = "lenient")]
    seasons: Vec<RawSeason>,
    #[serde(default, deserialize_with = "lenient")]
    info: RawSeriesMeta,
    #[serde(default, deserialize_with = "lenient")]
    episodes: std::collections::HashMap<String, Vec<RawEpisode>>,
}

fn season_number(value: Option<Flex>) -> Option<u32> {
    value
        .and_then(Flex::number)
        .and_then(|n| u32::try_from(n).ok())
}

/// Fetch one series' seasons and episodes. Seasons come from the episode
/// map in numeric order; episodes sort by number.
pub fn series_info(
    f: &dyn HttpFetcher,
    source: &Source,
    series_id: &str,
) -> Result<SeriesInfo, XtreamError> {
    let response: RawSeriesInfo = fetch_typed(
        f,
        source,
        "get_series_info",
        &[("series_id", provider_id(series_id))],
        FetchLimits::DETAILS,
    )?;
    let mut names: std::collections::HashMap<u32, String> = Default::default();
    let mut covers: std::collections::HashMap<u32, String> = Default::default();
    for season in response.seasons {
        let Some(number) = season_number(season.season_number) else {
            continue;
        };
        if let Some(name) = text(season.name) {
            names.insert(number, name);
        }
        let cover = image_url(season.cover_big)
            .or_else(|| image_url(season.cover))
            .or_else(|| image_url(season.cover_tmdb));
        if let Some(cover) = cover {
            covers.insert(number, cover);
        }
    }
    let mut by_season: std::collections::BTreeMap<u32, Vec<Episode>> = Default::default();
    for (key, episodes) in response.episodes {
        let key_number = key.trim().parse::<u32>().ok();
        for episode in episodes {
            let Some(id) = text(episode.id) else {
                continue;
            };
            let Some(season) = key_number.or_else(|| season_number(episode.season)) else {
                continue;
            };
            let number = season_number(episode.episode_num).unwrap_or(0);
            by_season.entry(season).or_default().push(Episode {
                title: text(episode.title).unwrap_or_else(|| format!("Episode {number}")),
                extension: text(episode.container_extension)
                    .unwrap_or_else(|| DEFAULT_EXTENSION.into()),
                duration_secs: episode
                    .info
                    .duration_secs
                    .and_then(Flex::number)
                    .and_then(|secs| u64::try_from(secs).ok())
                    .filter(|secs| *secs > 0),
                plot: text(episode.info.plot),
                image: image_url(episode.info.movie_image),
                id,
                number,
            });
        }
    }
    let seasons = by_season
        .into_iter()
        .map(|(number, mut episodes)| {
            episodes.sort_by(|a, b| a.number.cmp(&b.number).then_with(|| a.id.cmp(&b.id)));
            Season {
                name: names
                    .get(&number)
                    .cloned()
                    .unwrap_or_else(|| format!("Season {number}")),
                cover: covers.remove(&number),
                number,
                episodes,
            }
        })
        .collect();
    let meta = response.info;
    Ok(SeriesInfo {
        plot: text(meta.plot),
        genre: text(meta.genre),
        cast: text(meta.cast),
        rating: text(meta.rating),
        release_date: text(meta.release_date_camel).or_else(|| text(meta.release_date)),
        seasons,
    })
}

/// Episode stream URL with `{user}`/`{pass}` placeholders for play time.
pub fn episode_url(source: &Source, episode: &Episode) -> Result<String, XtreamError> {
    let host = base_url(source)?;
    Ok(format!(
        "{host}/series/{{user}}/{{pass}}/{}.{}",
        episode.id, episode.extension
    ))
}
