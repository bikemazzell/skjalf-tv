//! Right-to-left display text.

use std::borrow::Cow;

use skjalf_core::display::{
    rtl_conversions, short_episode_title, visual_text, wrap_rtl, DisplayNames,
};

const PERSIAN: &str = "دنیای دیگر";
const PERSIAN_VISUAL: &str =
    "\u{FEAE}\u{FB95}\u{FBFE}\u{FEA9} \u{FBFC}\u{FE8E}\u{FBFF}\u{FEE7}\u{FEA9}";

#[test]
fn latin_text_is_returned_unchanged_and_unallocated() {
    for text in [
        "",
        "News 1",
        "IR - A Different World (2026)",
        "Película ñandú ü",
        "日本語",
    ] {
        assert!(matches!(visual_text(text), Cow::Borrowed(_)), "{text}");
    }
}

#[test]
fn a_persian_word_comes_back_joined_and_in_visual_order() {
    let visual = visual_text(PERSIAN);
    assert_eq!(visual, PERSIAN_VISUAL);
    assert!(
        visual
            .chars()
            .all(|c| !('\u{600}'..='\u{6FF}').contains(&c)),
        "only presentation forms remain: {visual:?}"
    );
    let chars: Vec<char> = visual.chars().collect();
    assert!(chars.contains(&'\u{FEE7}'), "noon takes its initial form");
    assert!(chars.contains(&'\u{FBFF}'), "yeh takes its medial form");
    assert!(!chars.contains(&'\u{FEE5}'), "no isolated noon");
    assert!(!chars.contains(&'\u{FBFD}'), "no isolated yeh");
    let reversed_logical = visual_text("دیگر");
    assert!(
        PERSIAN_VISUAL.starts_with(&*reversed_logical),
        "the second word is drawn first"
    );
}

#[test]
fn mixed_text_keeps_the_latin_prefix_first_and_its_own_order() {
    let visual = visual_text("IR - A Different World (2026) دنیای دیگر");
    assert_eq!(
        visual,
        format!("IR - A Different World (2026) {PERSIAN_VISUAL}")
    );
    let numbers = visual_text("دنیای 2026 دیگر");
    assert!(
        numbers.contains("2026"),
        "digits stay in natural order: {numbers:?}"
    );
}

#[test]
fn brackets_mirror_inside_right_to_left_runs() {
    let visual = visual_text("دنیای (2026) دیگر");
    assert!(visual.contains("(2026)"), "{visual:?}");
    let angle = visual_text("دیگر <x>");
    assert!(angle.contains("<x>") || angle.contains(">x<"), "{angle:?}");
}

#[test]
fn multi_line_text_is_converted_line_by_line() {
    let input = format!("{PERSIAN}\nHello world\n{PERSIAN}");
    let visual = visual_text(&input);
    let lines: Vec<&str> = visual.split('\n').collect();
    assert_eq!(lines, [PERSIAN_VISUAL, "Hello world", PERSIAN_VISUAL]);
}

#[test]
fn a_54000_name_catalog_converts_each_name_once() {
    let names: Vec<String> = (0..54_000)
        .map(|n| match n % 50 {
            0 => format!("{PERSIAN} {n}"),
            _ => format!("Channel {n}"),
        })
        .collect();
    let before = rtl_conversions();
    let display = DisplayNames::build(names.iter().map(String::as_str));
    let built = rtl_conversions() - before;
    assert!(built >= 1_080, "each right-to-left name converted: {built}");
    let after_build = rtl_conversions();
    for _ in 0..3 {
        for (index, name) in names.iter().enumerate() {
            let shown = display.get(index, name);
            if index % 50 == 0 {
                assert!(shown.contains('\u{FEA9}'), "{shown}");
            } else {
                assert_eq!(shown, name);
            }
        }
    }
    assert_eq!(
        rtl_conversions(),
        after_build,
        "reading display names never converts again"
    );
}

const SERIES: &str = "IR - A Different World (2026) (US) دنیای دیگر";

#[test]
fn episode_titles_that_repeat_the_series_name_keep_only_the_useful_part() {
    let title = format!("{SERIES} - S01E03 - Center de Stage");
    assert_eq!(short_episode_title(SERIES, &title, 3), "Center de Stage");
    let lower = format!("{} - s01e03 - Center de Stage", SERIES.to_lowercase());
    assert_eq!(short_episode_title(SERIES, &lower, 3), "Center de Stage");
    assert_eq!(
        short_episode_title("Show", "  SHOW  -  S10E12: The Finale  ", 12),
        "The Finale"
    );
}

#[test]
fn the_marker_alone_is_enough_without_the_series_name() {
    assert_eq!(
        short_episode_title("Show", "Other Name - S02E05 - Real Title", 5),
        "Real Title"
    );
    assert_eq!(
        short_episode_title("Show", "S2E5 - Real Title", 5),
        "Real Title"
    );
}

#[test]
fn a_title_equal_to_the_series_name_or_only_a_marker_becomes_episode_n() {
    assert_eq!(short_episode_title("Show", "Show", 4), "Episode 4");
    assert_eq!(short_episode_title("Show", "  show ", 4), "Episode 4");
    assert_eq!(short_episode_title("Show", "Show - S01E04", 4), "Episode 4");
    assert_eq!(short_episode_title("Show", "S01E04 - ", 4), "Episode 4");
    assert_eq!(short_episode_title("Show", "", 7), "Episode 7");
}

#[test]
fn titles_without_the_series_name_or_a_marker_stay_as_they_are() {
    assert_eq!(short_episode_title("Show", "Pilot", 1), "Pilot");
    assert_eq!(
        short_episode_title("Show", "The Show Must Go On", 2),
        "The Show Must Go On",
        "a name inside the title is not a prefix"
    );
    assert_eq!(short_episode_title("", "Pilot", 1), "Pilot");
    assert_eq!(
        short_episode_title("Show", "Showdown", 3),
        "Showdown",
        "the name must end at a separator"
    );
}

const PARAGRAPH: &str = "داستان دختری به‌نام مریم که در شهری کوچک زندگی می‌کند";

#[test]
fn rtl_paragraphs_are_wrapped_in_logical_order_then_reordered_per_line() {
    let measure = |text: &str| text.chars().count() as f32 * 10.0;
    let lines = wrap_rtl(PARAGRAPH, 160.0, &measure);
    assert!(lines.len() >= 3, "{lines:?}");
    assert!(lines.iter().all(|line| line.rtl));
    assert!(
        lines.iter().all(|line| measure(&line.text) <= 160.0),
        "{lines:?}"
    );
    let first_words: Vec<&str> = PARAGRAPH.split(' ').take(2).collect();
    assert!(
        lines[0]
            .text
            .contains(&*visual_text(&first_words.join(" "))),
        "the first visual line holds the first logical words: {:?}",
        lines[0].text
    );
    let rejoined: Vec<String> = lines.iter().map(|line| line.text.clone()).collect();
    let all_words: usize = PARAGRAPH.split(' ').count();
    assert!(rejoined.iter().map(|l| l.split(' ').count()).sum::<usize>() >= all_words);
    let wide = wrap_rtl(PARAGRAPH, 10_000.0, &measure);
    assert_eq!(wide.len(), 1);
    assert_eq!(wide[0].text, visual_text(PARAGRAPH));
}

#[test]
fn latin_and_mixed_paragraphs_are_kept_apart() {
    let measure = |text: &str| text.chars().count() as f32 * 10.0;
    let text = format!("A plain English paragraph.\n{PARAGRAPH}");
    let lines = wrap_rtl(&text, 160.0, &measure);
    assert_eq!(lines[0].text, "A plain English paragraph.");
    assert!(!lines[0].rtl, "Latin paragraphs are left to the renderer");
    assert!(lines[1..].iter().all(|line| line.rtl));
    assert!(wrap_rtl("", 100.0, &measure)
        .iter()
        .all(|line| line.text.is_empty()));
}
