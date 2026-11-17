//! Display text for provider strings: right-to-left scripts in visual order
//! and shorter episode titles. Logical text stays the source of truth for
//! search and records; these strings are for drawing only.

use std::borrow::Cow;
use std::cell::Cell;
use std::sync::OnceLock;

use ar_reshaper::ArabicReshaper;
use unicode_bidi::{bidi_class, BidiClass, BidiInfo};

thread_local! {
    static RTL_CONVERSIONS: Cell<usize> = const { Cell::new(0) };
}

/// Right-to-left conversions run on the calling thread so far. Tests use it
/// to assert that painting never repeats a conversion.
pub fn rtl_conversions() -> usize {
    RTL_CONVERSIONS.with(Cell::get)
}

fn is_rtl(c: char) -> bool {
    c > '\u{58f}' && matches!(bidi_class(c), BidiClass::R | BidiClass::AL)
}

fn reshaper() -> &'static ArabicReshaper {
    static RESHAPER: OnceLock<ArabicReshaper> = OnceLock::new();
    RESHAPER.get_or_init(ArabicReshaper::default)
}

/// Whether the text contains right-to-left letters.
pub fn has_rtl(text: &str) -> bool {
    text.chars().any(is_rtl)
}

/// One drawn line of a wrapped paragraph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedLine {
    /// Right-to-left lines are already in visual order; others are as given.
    pub text: String,
    pub rtl: bool,
}

/// Break each paragraph of `text` at spaces into lines that fit `max_width`
/// (measured on the drawn string with `measure`), then convert every
/// right-to-left line to visual order. Wrapping a converted paragraph would
/// put its first words on the last line. Paragraphs without right-to-left
/// letters come back whole for the renderer's own wrapping.
pub fn wrap_rtl(text: &str, max_width: f32, measure: &dyn Fn(&str) -> f32) -> Vec<WrappedLine> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if !has_rtl(paragraph) {
            lines.push(WrappedLine {
                text: paragraph.to_owned(),
                rtl: false,
            });
            continue;
        }
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if current.is_empty() {
                word.to_owned()
            } else {
                format!("{current} {word}")
            };
            if !current.is_empty() && measure(&visual_text(&candidate)) > max_width {
                lines.push(rtl_line(&current));
                current = word.to_owned();
            } else {
                current = candidate;
            }
        }
        if !current.is_empty() {
            lines.push(rtl_line(&current));
        }
    }
    lines
}

fn rtl_line(logical: &str) -> WrappedLine {
    WrappedLine {
        text: visual_text(logical).into_owned(),
        rtl: true,
    }
}

/// The text as a left-to-right renderer must draw it. Text without
/// right-to-left letters is returned unchanged and unallocated.
pub fn visual_text(text: &str) -> Cow<'_, str> {
    if !text.chars().any(is_rtl) {
        return Cow::Borrowed(text);
    }
    RTL_CONVERSIONS.with(|count| count.set(count.get() + 1));
    let lines: Vec<String> = text.split('\n').map(visual_line).collect();
    Cow::Owned(lines.join("\n"))
}

fn visual_line(line: &str) -> String {
    if !line.chars().any(is_rtl) {
        return line.to_owned();
    }
    let reshaped = reshaper().reshape(line);
    let info = BidiInfo::new(&reshaped, None);
    let mut visual = String::with_capacity(reshaped.len());
    for paragraph in &info.paragraphs {
        let (levels, runs) = info.visual_runs(paragraph, paragraph.range.clone());
        for run in runs {
            let text = &reshaped[run.clone()];
            if levels[run.start].is_rtl() {
                visual.extend(text.chars().rev().map(mirror));
            } else {
                visual.push_str(text);
            }
        }
    }
    visual
}

/// Brackets swap shape inside right-to-left runs.
fn mirror(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '\u{ab}' => '\u{bb}',
        '\u{bb}' => '\u{ab}',
        other => other,
    }
}

/// Display strings for a list of names, built once per catalog load. Names
/// that need no conversion cost one empty slot.
#[derive(Debug, Default)]
pub struct DisplayNames {
    converted: Vec<Option<Box<str>>>,
}

impl DisplayNames {
    pub fn push(&mut self, name: &str) {
        self.converted.push(match visual_text(name) {
            Cow::Borrowed(_) => None,
            Cow::Owned(text) => Some(text.into_boxed_str()),
        });
    }

    pub fn build<'a>(names: impl IntoIterator<Item = &'a str>) -> Self {
        Self {
            converted: names
                .into_iter()
                .map(|name| match visual_text(name) {
                    Cow::Borrowed(_) => None,
                    Cow::Owned(text) => Some(text.into_boxed_str()),
                })
                .collect(),
        }
    }

    /// The display string for position `item`, or `logical` when unchanged.
    pub fn get<'a>(&'a self, item: usize, logical: &'a str) -> &'a str {
        self.converted
            .get(item)
            .and_then(|text| text.as_deref())
            .unwrap_or(logical)
    }
}

/// Strip a leading series name and an "S01E03 - " marker from an episode
/// title. Falls back to "Episode N" when nothing is left.
pub fn short_episode_title(series_name: &str, title: &str, number: u32) -> String {
    let mut rest = title.trim();
    let series = series_name.trim();
    if !series.is_empty() {
        if let Some(stripped) = strip_prefix_ignore_case(rest, series)
            .filter(|stripped| stripped.chars().next().is_none_or(is_separator))
        {
            rest = stripped;
        }
    }
    rest = rest.trim_start_matches(is_separator).trim();
    if let Some(after) = after_episode_marker(rest) {
        rest = after;
    }
    if rest.is_empty() {
        format!("Episode {number}")
    } else {
        rest.to_owned()
    }
}

fn is_separator(c: char) -> bool {
    c.is_whitespace() || matches!(c, '-' | '\u{2013}' | '\u{2014}' | ':' | '|')
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let mut text_chars = text.char_indices();
    for expected in prefix.chars() {
        let (_, actual) = text_chars.next()?;
        if !actual.to_lowercase().eq(expected.to_lowercase()) {
            return None;
        }
    }
    Some(text_chars.next().map_or("", |(index, _)| &text[index..]))
}

/// The text after the first `S<digits>E<digits>` marker and its separator,
/// or the empty text when the marker ends the title.
fn after_episode_marker(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let mut start = 0;
    while start < bytes.len() {
        if matches!(bytes[start], b'S' | b's') {
            let mut end = start + 1;
            let digits = |end: &mut usize| {
                let from = *end;
                while bytes.get(*end).is_some_and(u8::is_ascii_digit) {
                    *end += 1;
                }
                *end > from
            };
            if digits(&mut end) && matches!(bytes.get(end), Some(b'E' | b'e')) {
                end += 1;
                if digits(&mut end) {
                    return Some(text[end..].trim_start_matches(is_separator).trim());
                }
            }
        }
        start += 1;
    }
    None
}
