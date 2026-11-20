//! Track labels, track stepping, and language preference cycling.

use skjalf_core::model::Settings;
use skjalf_core::tracks::{
    canonical, cycle_language_pref, language_name, language_pref_label, step_track, track_label,
    SubtitlePref, Track, TrackKind, LANGUAGES,
};

fn track(id: i64, kind: TrackKind, lang: Option<&str>, selected: bool) -> Track {
    Track {
        id,
        kind,
        lang: lang.map(Into::into),
        title: None,
        codec: None,
        channels: None,
        selected,
        forced: false,
    }
}

fn movie_tracks(audio_sel: i64, sub_sel: Option<i64>) -> Vec<Track> {
    vec![
        track(1, TrackKind::Audio, Some("eng"), audio_sel == 1),
        track(2, TrackKind::Audio, Some("ger"), audio_sel == 2),
        track(3, TrackKind::Audio, None, audio_sel == 3),
        track(1, TrackKind::Sub, Some("eng"), sub_sel == Some(1)),
        track(2, TrackKind::Sub, Some("fre"), sub_sel == Some(2)),
    ]
}

// -- languages ------------------------------------------------------------------

#[test]
fn language_codes_resolve_across_iso_forms() {
    assert_eq!(language_name("ger"), "German");
    assert_eq!(language_name("DEU"), "German");
    assert_eq!(language_name("de"), "German");
    assert_eq!(language_name("xyz"), "xyz", "unknown codes show as-is");
    assert_eq!(canonical("GER"), "de");
    assert_eq!(canonical("eng"), "en");
    assert_eq!(canonical("Xyz"), "xyz");
}

#[test]
fn language_table_codes_are_unique_and_lowercase() {
    let mut seen = std::collections::HashSet::new();
    for lang in LANGUAGES {
        assert!(!lang.codes.is_empty(), "{} has codes", lang.name);
        for code in lang.codes {
            assert_eq!(*code, code.to_lowercase());
            assert!(seen.insert(*code), "duplicate code {code}");
        }
    }
}

// -- labels ------------------------------------------------------------------------

#[test]
fn audio_label_names_language_title_codec_and_layout() {
    let t = Track {
        title: Some("Director's commentary".into()),
        codec: Some("ac3".into()),
        channels: Some(6),
        ..track(2, TrackKind::Audio, Some("eng"), false)
    };
    assert_eq!(
        track_label(&t),
        "English \u{00b7} Director's commentary \u{00b7} AC3 5.1"
    );
    let stereo = Track {
        codec: Some("aac".into()),
        channels: Some(2),
        ..track(1, TrackKind::Audio, None, false)
    };
    assert_eq!(track_label(&stereo), "AAC stereo");
}

#[test]
fn label_skips_title_that_repeats_the_language_and_marks_forced() {
    let t = Track {
        title: Some("english".into()),
        forced: true,
        ..track(4, TrackKind::Sub, Some("en"), false)
    };
    assert_eq!(track_label(&t), "English \u{00b7} forced");
}

#[test]
fn label_falls_back_to_track_number() {
    assert_eq!(
        track_label(&track(7, TrackKind::Audio, None, false)),
        "Track 7"
    );
}

// -- stepping ------------------------------------------------------------------------

#[test]
fn audio_steps_wrap_without_an_off_state() {
    let tracks = movie_tracks(1, None);
    assert_eq!(step_track(&tracks, TrackKind::Audio, true), Some(Some(2)));
    assert_eq!(step_track(&tracks, TrackKind::Audio, false), Some(Some(3)));
    let tracks = movie_tracks(3, None);
    assert_eq!(step_track(&tracks, TrackKind::Audio, true), Some(Some(1)));
}

#[test]
fn subtitle_steps_pass_through_off() {
    let tracks = movie_tracks(1, None);
    assert_eq!(step_track(&tracks, TrackKind::Sub, true), Some(Some(1)));
    assert_eq!(step_track(&tracks, TrackKind::Sub, false), Some(Some(2)));
    let tracks = movie_tracks(1, Some(2));
    assert_eq!(step_track(&tracks, TrackKind::Sub, true), Some(None));
    assert_eq!(step_track(&tracks, TrackKind::Sub, false), Some(Some(1)));
}

#[test]
fn stepping_a_kind_without_tracks_does_nothing() {
    let tracks = vec![track(1, TrackKind::Audio, Some("eng"), true)];
    assert_eq!(step_track(&tracks, TrackKind::Sub, true), None);
    assert_eq!(step_track(&[], TrackKind::Audio, true), None);
}

#[test]
fn audio_step_with_nothing_selected_starts_at_first_track() {
    let tracks = movie_tracks(0, None);
    assert_eq!(step_track(&tracks, TrackKind::Audio, true), Some(Some(1)));
}

// -- preferences ------------------------------------------------------------------

#[test]
fn language_pref_cycles_stream_default_then_table_and_wraps() {
    assert_eq!(
        cycle_language_pref(None, true).as_deref(),
        Some(LANGUAGES[0].codes[0])
    );
    let last = LANGUAGES.last().unwrap().codes[0];
    assert_eq!(cycle_language_pref(None, false).as_deref(), Some(last));
    assert_eq!(cycle_language_pref(Some(last), true), None);
    assert_eq!(cycle_language_pref(Some("en"), false), None);
    assert_eq!(
        cycle_language_pref(Some("GER"), true).as_deref(),
        Some("fr")
    );
}

#[test]
fn language_pref_keeps_an_unknown_language_in_the_cycle() {
    let last = LANGUAGES.last().unwrap().codes[0];
    assert_eq!(
        cycle_language_pref(Some("xyz"), false).as_deref(),
        Some(last)
    );
    assert_eq!(cycle_language_pref(Some("xyz"), true), None);
}

#[test]
fn preference_labels() {
    assert_eq!(language_pref_label(None), "Stream default");
    assert_eq!(language_pref_label(Some("de")), "German");
}

#[test]
fn settings_map_to_the_subtitle_choice_the_engine_applies() {
    let settings = |on: bool, lang: Option<&str>| Settings {
        subtitles_on: on,
        subtitle_lang: lang.map(Into::into),
        ..Default::default()
    };
    assert_eq!(Settings::default().subtitle_pref(), SubtitlePref::Off);
    assert_eq!(
        settings(false, Some("de")).subtitle_pref(),
        SubtitlePref::Off
    );
    assert_eq!(settings(true, None).subtitle_pref(), SubtitlePref::Auto);
    assert_eq!(
        settings(true, Some("de")).subtitle_pref(),
        SubtitlePref::Lang("de".into())
    );
}
