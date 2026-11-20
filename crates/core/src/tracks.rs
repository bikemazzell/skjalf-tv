//! Audio/subtitle tracks: display labels, track stepping, and the language
//! preferences from Settings (pure; the engine supplies and applies tracks).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Audio,
    Sub,
}

/// One selectable stream track as reported by the player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    /// Player track id (mpv `aid`/`sid` value).
    pub id: i64,
    pub kind: TrackKind,
    pub lang: Option<String>,
    pub title: Option<String>,
    pub codec: Option<String>,
    pub channels: Option<u32>,
    pub selected: bool,
    pub forced: bool,
}

/// The subtitle choice applied to every newly loaded stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubtitlePref {
    Off,
    /// On: the stream's default subtitle track, else its first one.
    Auto,
    /// On: this canonical language code (see [`canonical`]), else none.
    Lang(String),
}

pub struct Language {
    pub name: &'static str,
    /// Lowercase ISO 639-1 code first, then ISO 639-2 B/T forms.
    pub codes: &'static [&'static str],
}

/// Languages offered in Settings; codes cover the forms streams use.
/// mpv itself matches a stored code against the other ISO 639 forms.
pub const LANGUAGES: &[Language] = &[
    lang("English", &["en", "eng"]),
    lang("German", &["de", "ger", "deu"]),
    lang("French", &["fr", "fre", "fra"]),
    lang("Spanish", &["es", "spa"]),
    lang("Italian", &["it", "ita"]),
    lang("Portuguese", &["pt", "por"]),
    lang("Dutch", &["nl", "dut", "nld"]),
    lang("Polish", &["pl", "pol"]),
    lang("Czech", &["cs", "cze", "ces"]),
    lang("Hungarian", &["hu", "hun"]),
    lang("Romanian", &["ro", "rum", "ron"]),
    lang("Greek", &["el", "gre", "ell"]),
    lang("Swedish", &["sv", "swe"]),
    lang("Norwegian", &["no", "nor", "nob", "nno"]),
    lang("Danish", &["da", "dan"]),
    lang("Finnish", &["fi", "fin"]),
    lang("Russian", &["ru", "rus"]),
    lang("Ukrainian", &["uk", "ukr"]),
    lang("Serbian", &["sr", "srp"]),
    lang("Croatian", &["hr", "hrv"]),
    lang("Albanian", &["sq", "alb", "sqi"]),
    lang("Turkish", &["tr", "tur"]),
    lang("Arabic", &["ar", "ara"]),
    lang("Persian", &["fa", "per", "fas"]),
    lang("Hebrew", &["he", "heb"]),
    lang("Hindi", &["hi", "hin"]),
    lang("Chinese", &["zh", "chi", "zho"]),
    lang("Japanese", &["ja", "jpn"]),
    lang("Korean", &["ko", "kor"]),
    lang("Vietnamese", &["vi", "vie"]),
    lang("Thai", &["th", "tha"]),
    lang("Indonesian", &["id", "ind"]),
];

const fn lang(name: &'static str, codes: &'static [&'static str]) -> Language {
    Language { name, codes }
}

fn language(code: &str) -> Option<&'static Language> {
    let code = code.to_lowercase();
    LANGUAGES.iter().find(|l| l.codes.contains(&code.as_str()))
}

/// Human name for a language code (the code itself when unknown).
pub fn language_name(code: &str) -> String {
    language(code).map_or_else(|| code.to_owned(), |l| l.name.to_owned())
}

/// Stored form of a language code: the table's first code, else lowercase.
pub fn canonical(code: &str) -> String {
    language(code).map_or_else(|| code.to_lowercase(), |l| l.codes[0].to_owned())
}

/// One-line description, e.g. `English · Commentary · AC3 5.1`.
pub fn track_label(track: &Track) -> String {
    let name = track.lang.as_deref().map(language_name);
    let title = track
        .title
        .clone()
        .filter(|t| name.as_ref().is_none_or(|n| !n.eq_ignore_ascii_case(t)));
    let format = [
        track.codec.as_deref().map(str::to_uppercase),
        track.channels.map(channel_layout),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");
    let parts: Vec<String> = [
        name,
        title,
        Some(format).filter(|f| !f.is_empty()),
        track.forced.then(|| "forced".to_owned()),
    ]
    .into_iter()
    .flatten()
    .collect();
    if parts.is_empty() {
        format!("Track {}", track.id)
    } else {
        parts.join(" \u{00b7} ")
    }
}

fn channel_layout(channels: u32) -> String {
    match channels {
        1 => "mono".into(),
        2 => "stereo".into(),
        6 => "5.1".into(),
        8 => "7.1".into(),
        n => format!("{n}ch"),
    }
}

/// Currently selected track of a kind.
pub fn selected(tracks: &[Track], kind: TrackKind) -> Option<&Track> {
    tracks.iter().find(|t| t.kind == kind && t.selected)
}

/// The next selection for `kind` (`Some(None)` = subtitles off), or `None`
/// when the stream has no track of that kind. Audio has no off state.
pub fn step_track(tracks: &[Track], kind: TrackKind, forward: bool) -> Option<Option<i64>> {
    let ids = tracks.iter().filter(|t| t.kind == kind).map(|t| Some(t.id));
    let options: Vec<Option<i64>> = match kind {
        TrackKind::Audio => ids.collect(),
        TrackKind::Sub => std::iter::once(None).chain(ids).collect(),
    };
    if options.iter().all(Option::is_none) {
        return None;
    }
    let current = selected(tracks, kind).map(|t| t.id);
    let next = match options.iter().position(|o| *o == current) {
        Some(i) => step_index(i, options.len(), forward),
        None if forward => 0,
        None => options.len() - 1,
    };
    Some(options[next])
}

fn step_index(i: usize, len: usize, forward: bool) -> usize {
    if forward {
        (i + 1) % len
    } else {
        (i + len - 1) % len
    }
}

/// Codes offered for a preference: the table, plus an unknown current code.
fn pref_codes(current: Option<&str>) -> Vec<String> {
    let mut codes: Vec<String> = LANGUAGES.iter().map(|l| l.codes[0].to_owned()).collect();
    if let Some(code) = current.filter(|c| language(c).is_none()) {
        codes.push(code.to_owned());
    }
    codes
}

fn cycle<T: PartialEq + Clone>(options: &[T], current: &T, forward: bool) -> T {
    let i = options.iter().position(|o| o == current).unwrap_or(0);
    options[step_index(i, options.len(), forward)].clone()
}

/// Next language preference: stream default, then each language.
pub fn cycle_language_pref(current: Option<&str>, forward: bool) -> Option<String> {
    let options: Vec<Option<String>> = std::iter::once(None)
        .chain(pref_codes(current).into_iter().map(Some))
        .collect();
    cycle(&options, &current.map(canonical), forward)
}

pub fn language_pref_label(pref: Option<&str>) -> String {
    pref.map_or_else(|| "Stream default".into(), language_name)
}
