//! Core data model — sources, groups, channels, EPG, favorites, settings.

use crate::browse::SortMode;
use crate::tracks::SubtitlePref;
use chrono::{NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// How a source delivers live streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum StreamFormat {
    #[default]
    Ts,
    M3u8,
}

/// The provider backend a playlist comes from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceKind {
    Xtream {
        host: String,
        username: String,
        password: String,
    },
    M3u {
        url: String,
    },
}

/// A configured IPTV provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub kind: SourceKind,
    pub stream_format: StreamFormat,
}

/// A channel category (Xtream "live category" / M3U `group-title`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub name: String,
}

/// A live channel. `url` may contain `{user}`/`{pass}` placeholders (see
/// [`crate::template`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    pub id: String,
    pub name: String,
    /// Channel number used for zapping, if the source provides one.
    pub num: Option<u32>,
    pub logo: Option<String>,
    pub group_id: String,
    pub url: String,
}

/// One EPG time slot ("now/next" granularity).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpgEntry {
    pub title: String,
    pub description: String,
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
}

impl EpgEntry {
    pub fn is_now(&self, at: NaiveDateTime) -> bool {
        self.start <= at && at < self.end
    }
}

/// Everything cached locally for one source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Playlist {
    pub source: Source,
    pub groups: Vec<Group>,
    pub channels: Vec<Channel>,
}

impl Playlist {
    /// Channels of one group, in playlist order.
    pub fn channels_in_group<'a>(
        &'a self,
        group_id: &'a str,
    ) -> impl Iterator<Item = &'a Channel> + 'a {
        self.channels.iter().filter(move |c| c.group_id == group_id)
    }

    pub fn find(&self, channel_id: &str) -> Option<&Channel> {
        self.channels.iter().find(|c| c.id == channel_id)
    }

    pub fn find_by_num(&self, num: u32) -> Option<&Channel> {
        self.channels.iter().find(|c| c.num == Some(num))
    }
}

/// Favorite channels, keyed by (source id, channel id).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Favorites(BTreeSet<(String, String)>);

impl Favorites {
    pub fn toggle(&mut self, source_id: &str, channel_id: &str) {
        let key = (source_id.to_owned(), channel_id.to_owned());
        if !self.0.insert(key.clone()) {
            self.0.remove(&key);
        }
    }

    pub fn contains(&self, source_id: &str, channel_id: &str) -> bool {
        self.0
            .contains(&(source_id.to_owned(), channel_id.to_owned()))
    }

    pub fn iter(&self) -> impl Iterator<Item = &(String, String)> {
        self.0.iter()
    }
}

/// Account activation status reported by the provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Activation {
    Active,
    /// Account expired on this date.
    Expired {
        date: NaiveDate,
    },
    Banned,
    Invalid,
}

/// User-facing app settings (persisted as TOML).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// 0–100.
    pub volume: u8,
    /// (source id, channel id) to resume on startup.
    pub last_channel: Option<(String, String)>,
    /// Skip TLS certificate verification for provider hosts.
    pub ignore_ssl: bool,
    /// Last connected source; identifies the cached playlist on startup.
    pub source: Option<Source>,
    /// Browser sort modes; files without this table load as Provider order.
    #[serde(default)]
    pub sort_modes: SortModes,
    /// How often content refreshes in the background.
    #[serde(default)]
    pub refresh_interval: RefreshInterval,
    /// Last successful refresh per source id.
    #[serde(default)]
    pub last_refresh: std::collections::BTreeMap<String, RefreshTimes>,
    /// Seconds of video to buffer before a movie or episode starts.
    #[serde(default)]
    pub start_buffer_secs: u32,
    /// Seconds of video to buffer before playback resumes after a stall.
    #[serde(default = "default_stall_buffer_secs")]
    pub stall_buffer_secs: u32,
    /// Preferred audio language (canonical code); None = stream default.
    #[serde(default)]
    pub audio_lang: Option<String>,
    #[serde(default)]
    pub subtitles_on: bool,
    /// Preferred subtitle language while subtitles are on; None = stream default.
    #[serde(default)]
    pub subtitle_lang: Option<String>,
}

fn default_stall_buffer_secs() -> u32 {
    STALL_BUFFER_CHOICES[0]
}

/// Choices for [`Settings::start_buffer_secs`]; 0 starts at once.
pub const START_BUFFER_CHOICES: [u32; 7] = [0, 60, 120, 300, 600, 1200, 1800];

/// Choices for [`Settings::stall_buffer_secs`]; 1 second is mpv's own default.
pub const STALL_BUFFER_CHOICES: [u32; 8] = [1, 5, 10, 30, 60, 120, 300, 600];

/// The next or previous choice after `current`, wrapping around. A value
/// that is not a choice moves to the first choice.
pub fn cycle_choice(choices: &[u32], current: u32, forward: bool) -> u32 {
    let len = choices.len();
    match choices.iter().position(|choice| *choice == current) {
        Some(index) if forward => choices[(index + 1) % len],
        Some(index) => choices[(index + len - 1) % len],
        None => choices[0],
    }
}

/// "Off", "5 seconds", "1 minute", or "10 minutes".
pub fn wait_label(secs: u32) -> String {
    match secs {
        0 => "Off".into(),
        1 => "1 second".into(),
        60 => "1 minute".into(),
        secs if secs % 60 == 0 => format!("{} minutes", secs / 60),
        secs => format!("{secs} seconds"),
    }
}

/// Background refresh interval; Hours24 is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefreshInterval {
    Off,
    Hours6,
    Hours12,
    #[default]
    Hours24,
    Hours48,
    Days7,
}

impl RefreshInterval {
    pub const ALL: [RefreshInterval; 6] = [
        RefreshInterval::Off,
        RefreshInterval::Hours6,
        RefreshInterval::Hours12,
        RefreshInterval::Hours24,
        RefreshInterval::Hours48,
        RefreshInterval::Days7,
    ];

    pub fn label(self) -> &'static str {
        match self {
            RefreshInterval::Off => "Off",
            RefreshInterval::Hours6 => "6 hours",
            RefreshInterval::Hours12 => "12 hours",
            RefreshInterval::Hours24 => "24 hours",
            RefreshInterval::Hours48 => "48 hours",
            RefreshInterval::Days7 => "7 days",
        }
    }

    pub fn seconds(self) -> Option<u64> {
        let hours = match self {
            RefreshInterval::Off => return None,
            RefreshInterval::Hours6 => 6,
            RefreshInterval::Hours12 => 12,
            RefreshInterval::Hours24 => 24,
            RefreshInterval::Hours48 => 48,
            RefreshInterval::Days7 => 24 * 7,
        };
        Some(hours * 3600)
    }

    fn position(self) -> usize {
        Self::ALL.iter().position(|v| *v == self).unwrap_or(0)
    }

    pub fn next(self) -> Self {
        Self::ALL[(self.position() + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        Self::ALL[(self.position() + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    /// True when `last` (unix seconds) is older than the interval at `now`.
    /// A kind that never refreshed is due; Off is never due.
    pub fn is_due(self, last: Option<u64>, now: u64) -> bool {
        match (self.seconds(), last) {
            (None, _) => false,
            (Some(_), None) => true,
            (Some(interval), Some(last)) => now.saturating_sub(last) >= interval,
        }
    }
}

/// Unix seconds of the last successful refresh per content kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RefreshTimes {
    #[serde(default)]
    pub live: Option<u64>,
    #[serde(default)]
    pub movies: Option<u64>,
    #[serde(default)]
    pub series: Option<u64>,
    /// The user opened Movies at least once; refresh retries it until loaded.
    #[serde(default)]
    pub movies_opened: bool,
    #[serde(default)]
    pub series_opened: bool,
}

/// Sort mode per browser kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SortModes {
    #[serde(default)]
    pub live: SortMode,
    #[serde(default)]
    pub movies: SortMode,
    #[serde(default)]
    pub series: SortMode,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            volume: 80,
            last_channel: None,
            ignore_ssl: false,
            source: None,
            sort_modes: SortModes::default(),
            refresh_interval: RefreshInterval::default(),
            last_refresh: Default::default(),
            start_buffer_secs: 0,
            stall_buffer_secs: default_stall_buffer_secs(),
            audio_lang: None,
            subtitles_on: false,
            subtitle_lang: None,
        }
    }
}

impl Settings {
    /// The subtitle choice that every new stream starts with.
    pub fn subtitle_pref(&self) -> SubtitlePref {
        match (&self.subtitle_lang, self.subtitles_on) {
            (_, false) => SubtitlePref::Off,
            (None, true) => SubtitlePref::Auto,
            (Some(code), true) => SubtitlePref::Lang(code.clone()),
        }
    }

    pub fn set_volume(&mut self, volume: u8) {
        self.volume = volume.min(100);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_choices_cycle_both_ways_and_wrap() {
        assert_eq!(cycle_choice(&START_BUFFER_CHOICES, 0, true), 60);
        assert_eq!(cycle_choice(&START_BUFFER_CHOICES, 1800, true), 0);
        assert_eq!(cycle_choice(&START_BUFFER_CHOICES, 0, false), 1800);
        assert_eq!(cycle_choice(&STALL_BUFFER_CHOICES, 30, false), 10);
    }

    #[test]
    fn an_unknown_buffer_value_moves_to_the_first_choice() {
        assert_eq!(cycle_choice(&STALL_BUFFER_CHOICES, 45, true), 1);
        assert_eq!(cycle_choice(&START_BUFFER_CHOICES, 7, false), 0);
    }

    #[test]
    fn wait_labels_read_naturally() {
        let labels: Vec<String> = [0, 1, 5, 60, 120, 1800]
            .into_iter()
            .map(wait_label)
            .collect();
        assert_eq!(
            labels,
            [
                "Off",
                "1 second",
                "5 seconds",
                "1 minute",
                "2 minutes",
                "30 minutes"
            ]
        );
    }
    use chrono::{NaiveDate, NaiveDateTime};

    fn xtream_source() -> Source {
        Source {
            id: "s1".into(),
            name: "My IPTV".into(),
            kind: SourceKind::Xtream {
                host: "http://example.tv:8080".into(),
                username: "u".into(),
                password: "p".into(),
            },
            stream_format: StreamFormat::Ts,
        }
    }

    #[test]
    fn source_xtream_roundtrip() {
        let json = serde_json::to_string(&xtream_source()).unwrap();
        assert!(json.contains(r#""kind":"xtream""#), "{json}");
        let back: Source = serde_json::from_str(&json).unwrap();
        assert_eq!(back, xtream_source());
    }

    #[test]
    fn source_m3u_roundtrip() {
        let s = Source {
            id: "s2".into(),
            name: "List".into(),
            kind: SourceKind::M3u {
                url: "http://x/y.m3u".into(),
            },
            stream_format: StreamFormat::M3u8,
        };
        let back: Source = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn playlist_roundtrip_and_queries() {
        let pl = Playlist {
            source: xtream_source(),
            groups: vec![
                Group {
                    id: "g1".into(),
                    name: "News".into(),
                },
                Group {
                    id: "g2".into(),
                    name: "Sports".into(),
                },
            ],
            channels: vec![
                Channel {
                    id: "c1".into(),
                    name: "CNN".into(),
                    num: Some(1),
                    logo: None,
                    group_id: "g1".into(),
                    url: "http://a/{user}/{pass}/1.ts".into(),
                },
                Channel {
                    id: "c2".into(),
                    name: "Sky".into(),
                    num: Some(2),
                    logo: Some("http://l/2.png".into()),
                    group_id: "g2".into(),
                    url: "http://a/{user}/{pass}/2.ts".into(),
                },
                Channel {
                    id: "c3".into(),
                    name: "BBC".into(),
                    num: None,
                    logo: None,
                    group_id: "g1".into(),
                    url: "http://a/{user}/{pass}/3.ts".into(),
                },
            ],
        };
        let back: Playlist = serde_json::from_str(&serde_json::to_string(&pl).unwrap()).unwrap();
        assert_eq!(back, pl);

        let g1: Vec<&str> = pl.channels_in_group("g1").map(|c| c.id.as_str()).collect();
        assert_eq!(g1, ["c1", "c3"]);
        assert_eq!(pl.find_by_num(2).unwrap().id, "c2");
        assert!(pl.find_by_num(9).is_none());
        assert_eq!(pl.find("c3").unwrap().name, "BBC");
    }

    #[test]
    fn favorites_toggle() {
        let mut f = Favorites::default();
        assert!(!f.contains("s1", "c1"));
        f.toggle("s1", "c1");
        assert!(f.contains("s1", "c1"));
        f.toggle("s1", "c1"); // toggle again removes
        assert!(!f.contains("s1", "c1"));
        assert!(!f.contains("s1", "cX"), "unknown channel is not favorite");
    }

    #[test]
    fn settings_default_and_volume_clamp() {
        let s = Settings::default();
        assert!((0..=100).contains(&s.volume));
        let mut s = Settings::default();
        s.set_volume(150);
        assert_eq!(s.volume, 100);
        s.set_volume(200);
        assert_eq!(s.volume, 100);
        s.set_volume(50);
        assert_eq!(s.volume, 50);
    }

    #[test]
    fn settings_last_channel_roundtrip() {
        let s = Settings {
            last_channel: Some(("s1".into(), "c2".into())),
            ..Default::default()
        };
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        let mapped = back
            .last_channel
            .as_ref()
            .map(|(s, c)| (s.as_str(), c.as_str()));
        assert_eq!(mapped, Some(("s1", "c2")));
    }

    #[test]
    fn activation_roundtrip() {
        let a = Activation::Expired {
            date: NaiveDate::from_ymd_opt(2027, 1, 2).unwrap(),
        };
        let back: Activation = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(back, a);
        assert_eq!(
            serde_json::to_string(&Activation::Banned).unwrap(),
            r#""banned""#
        );
    }

    #[test]
    fn epg_is_now() {
        let e = EpgEntry {
            title: "News".into(),
            description: "desc".into(),
            start: NaiveDateTime::parse_from_str("2026-11-15 20:00", "%Y-%m-%d %H:%M").unwrap(),
            end: NaiveDateTime::parse_from_str("2026-11-15 21:00", "%Y-%m-%d %H:%M").unwrap(),
        };
        let mid = NaiveDateTime::parse_from_str("2026-11-15 20:30", "%Y-%m-%d %H:%M").unwrap();
        let before = NaiveDateTime::parse_from_str("2026-11-15 19:59", "%Y-%m-%d %H:%M").unwrap();
        assert!(e.is_now(mid));
        assert!(!e.is_now(before));
    }
}
