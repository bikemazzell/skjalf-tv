//! Resume rules, Continue Watching order, and persistence.

use skjalf_core::progress::{Progress, ProgressEntry, ProgressKind};
use skjalf_core::store::Store;

fn entry(id: &str, position: f64, duration: Option<f64>, updated_at: i64) -> ProgressEntry {
    ProgressEntry {
        source_id: "s1".into(),
        kind: ProgressKind::Movie,
        id: id.into(),
        title: format!("Title {id}"),
        position_secs: position,
        duration_secs: duration,
        updated_at,
        finished: false,
        series_id: None,
        series_name: None,
        season: None,
        episode: None,
    }
}

#[test]
fn resume_needs_thirty_seconds_and_less_than_ninety_five_percent() {
    assert_eq!(entry("a", 29.9, Some(5400.0), 0).resume_position(), None);
    assert_eq!(
        entry("a", 30.0, Some(5400.0), 0).resume_position(),
        Some(30.0)
    );
    assert_eq!(entry("a", 600.0, None, 0).resume_position(), Some(600.0));
    assert_eq!(
        entry("a", 5129.9, Some(5400.0), 0).resume_position(),
        Some(5129.9)
    );
    assert_eq!(entry("a", 5130.0, Some(5400.0), 0).resume_position(), None);
    let mut done = entry("a", 100.0, Some(5400.0), 0);
    done.finished = true;
    assert_eq!(done.resume_position(), None);
}

#[test]
fn saving_past_the_finish_line_marks_finished_and_a_replay_clears_it() {
    let mut progress = Progress::default();
    progress.record(entry("a", 5200.0, Some(5400.0), 1));
    assert!(
        progress
            .get("s1", ProgressKind::Movie, "a")
            .unwrap()
            .finished
    );
    progress.record(entry("a", 120.0, Some(5400.0), 2));
    let saved = progress.get("s1", ProgressKind::Movie, "a").unwrap();
    assert!(!saved.finished);
    assert_eq!(saved.resume_position(), Some(120.0));
}

#[test]
fn end_of_file_marks_finished_at_the_duration() {
    let mut progress = Progress::default();
    progress.mark_finished(entry("a", 100.0, Some(900.0), 1));
    let saved = progress.get("s1", ProgressKind::Movie, "a").unwrap();
    assert!(saved.finished);
    assert_eq!(saved.position_secs, 900.0);
    progress.mark_finished(entry("b", 100.0, None, 1));
    assert!(
        progress
            .get("s1", ProgressKind::Movie, "b")
            .unwrap()
            .finished
    );
}

#[test]
fn entries_are_keyed_by_source_kind_and_id() {
    let mut progress = Progress::default();
    progress.record(entry("1", 100.0, None, 1));
    let mut episode = entry("1", 200.0, None, 2);
    episode.kind = ProgressKind::Episode;
    progress.record(episode);
    let mut other = entry("1", 300.0, None, 3);
    other.source_id = "s2".into();
    progress.record(other);
    assert_eq!(
        progress
            .get("s1", ProgressKind::Movie, "1")
            .unwrap()
            .position_secs,
        100.0
    );
    assert_eq!(
        progress
            .get("s1", ProgressKind::Episode, "1")
            .unwrap()
            .position_secs,
        200.0
    );
    assert_eq!(
        progress
            .get("s2", ProgressKind::Movie, "1")
            .unwrap()
            .position_secs,
        300.0
    );
    progress.record(entry("1", 150.0, None, 4));
    assert_eq!(
        progress
            .get("s1", ProgressKind::Movie, "1")
            .unwrap()
            .position_secs,
        150.0,
        "same key replaces"
    );
}

#[test]
fn continue_watching_is_newest_first_unfinished_and_limited() {
    let mut progress = Progress::default();
    progress.record(entry("old", 100.0, Some(5400.0), 10));
    progress.record(entry("new", 100.0, Some(5400.0), 30));
    progress.record(entry("mid", 100.0, Some(5400.0), 20));
    progress.record(entry("short", 10.0, Some(5400.0), 40));
    progress.record(entry("done", 5300.0, Some(5400.0), 50));
    let mut other = entry("elsewhere", 100.0, None, 60);
    other.source_id = "s2".into();
    progress.record(other);
    let ids: Vec<_> = progress
        .continue_watching("s1", 12)
        .iter()
        .map(|entry| entry.id.as_str())
        .collect();
    assert_eq!(ids, ["new", "mid", "old"]);
    assert_eq!(progress.continue_watching("s1", 2).len(), 2);
}

#[test]
fn every_record_is_kept_and_only_the_continue_list_is_limited() {
    let dir = std::env::temp_dir().join(format!("skjalf-progress-keep-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let store = Store::open(&dir).unwrap();
    let mut progress = Progress::default();
    for n in 0..600 {
        progress.record(entry(&n.to_string(), 100.0, None, n));
    }
    assert!(progress.get("s1", ProgressKind::Movie, "0").is_some());
    assert!(progress.get("s1", ProgressKind::Movie, "599").is_some());
    assert_eq!(progress.continue_watching("s1", 12).len(), 12);
    assert_eq!(progress.continue_watching("s1", 1000).len(), 600);
    store.save_progress(&progress).unwrap();
    let reloaded = store.load_progress();
    assert_eq!(reloaded, progress);
    for n in [0, 250, 599] {
        assert!(reloaded
            .get("s1", ProgressKind::Movie, &n.to_string())
            .is_some());
    }
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn progress_roundtrips_privately_and_corrupt_files_load_empty() {
    let dir = std::env::temp_dir().join(format!("skjalf-progress-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let store = Store::open(&dir).unwrap();
    assert_eq!(store.load_progress(), Progress::default());
    let mut progress = Progress::default();
    let mut episode = entry("9", 400.0, Some(2700.0), 7);
    episode.kind = ProgressKind::Episode;
    episode.series_id = Some("2001".into());
    episode.series_name = Some("Show".into());
    episode.season = Some(2);
    episode.episode = Some(3);
    progress.record(episode);
    store.save_progress(&progress).unwrap();
    assert_eq!(store.load_progress(), progress);
    let path = dir.join("progress.json");
    assert!(!std::fs::read_to_string(&path).unwrap().contains('\n'));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    std::fs::write(&path, "{broken").unwrap();
    assert_eq!(store.load_progress(), Progress::default());
    std::fs::remove_dir_all(dir).ok();
}

fn episode(series: &str, id: &str, position: f64, finished: bool) -> ProgressEntry {
    let mut entry = entry(id, position, Some(2700.0), 1);
    entry.kind = ProgressKind::Episode;
    entry.series_id = Some(series.into());
    entry.finished = finished;
    entry
}

#[test]
fn movie_and_episode_markers_follow_the_resume_rules() {
    use skjalf_core::progress::{format_left, Marker};
    let mut progress = Progress::default();
    progress.record(entry("watched", 5200.0, Some(5400.0), 1));
    progress.record(entry("partial", 1800.0, Some(5400.0), 2));
    progress.record(entry("short", 10.0, Some(5400.0), 3));
    progress.record(entry("nodur", 700.0, None, 4));
    let index = progress.marker_index("s1");
    assert_eq!(index.movie("watched"), Marker::Watched);
    assert_eq!(
        index.movie("partial"),
        Marker::InProgress {
            position_secs: 1800.0,
            fraction: Some(1.0 / 3.0),
            left_secs: Some(3600.0)
        }
    );
    assert_eq!(index.movie("short"), Marker::None);
    assert_eq!(index.movie("missing"), Marker::None);
    assert!(matches!(
        index.movie("nodur"),
        Marker::InProgress {
            fraction: None,
            left_secs: None,
            ..
        }
    ));
    assert_eq!(
        progress.marker_index("other").movie("partial"),
        Marker::None
    );
    assert_eq!(format_left(3600.0), "1:00 left");
    assert_eq!(format_left(5130.0), "1:25 left");
    assert_eq!(format_left(1800.0), "30:00 left");
    assert_eq!(format_left(65.0), "1:05 left");
}

#[test]
fn series_markers_need_a_known_total_to_read_watched() {
    use skjalf_core::progress::Marker;
    let mut progress = Progress::default();
    progress.mark_finished(episode("a", "a1", 0.0, true));
    progress.mark_finished(episode("a", "a2", 0.0, true));
    progress.record(episode("b", "b1", 600.0, false));
    progress.mark_finished(episode("c", "c1", 0.0, true));

    let unknown = progress.marker_index("s1");
    assert!(
        matches!(unknown.series("a"), Marker::InProgress { .. }),
        "no total yet"
    );
    assert!(matches!(unknown.series("b"), Marker::InProgress { .. }));
    assert_eq!(unknown.series("untouched"), Marker::None);

    progress.set_series_total("s1", "a", 2);
    progress.set_series_total("s1", "c", 3);
    progress.set_series_total("s1", "b", 8);
    let known = progress.marker_index("s1");
    assert_eq!(known.series("a"), Marker::Watched, "2 of 2 finished");
    assert!(
        matches!(known.series("c"), Marker::InProgress { .. }),
        "1 of 3"
    );
    assert!(matches!(known.series("b"), Marker::InProgress { .. }));
    assert_eq!(progress.series_total("s1", "a"), Some(2));
    progress.set_series_total("s1", "a", 3);
    assert!(matches!(
        progress.marker_index("s1").series("a"),
        Marker::InProgress { .. }
    ));
    assert_eq!(progress.series_total("other", "a"), None);
}

#[test]
fn series_totals_persist_and_old_files_without_them_load() {
    let dir = std::env::temp_dir().join(format!("skjalf-totals-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let store = Store::open(&dir).unwrap();
    let mut progress = Progress::default();
    progress.set_series_total("s1", "a", 12);
    store.save_progress(&progress).unwrap();
    assert_eq!(store.load_progress().series_total("s1", "a"), Some(12));
    std::fs::write(dir.join("progress.json"), r#"{"entries":[]}"#).unwrap();
    assert_eq!(store.load_progress(), Progress::default());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_short_replay_keeps_the_watched_mark_until_thirty_seconds() {
    let mut progress = Progress::default();
    progress.mark_finished(entry("a", 0.0, Some(5400.0), 1));
    progress.record(entry("a", 12.0, Some(5400.0), 2));
    let saved = progress.get("s1", ProgressKind::Movie, "a").unwrap();
    assert!(saved.finished, "under 30 s keeps Watched");
    assert!(progress.continue_watching("s1", 12).is_empty());
    progress.record(entry("a", 45.0, Some(5400.0), 3));
    let saved = progress.get("s1", ProgressKind::Movie, "a").unwrap();
    assert!(!saved.finished, "30 s or more becomes a resume");
    assert_eq!(saved.resume_position(), Some(45.0));
    progress.mark_finished(entry("a", 100.0, Some(5400.0), 4));
    assert!(
        progress
            .get("s1", ProgressKind::Movie, "a")
            .unwrap()
            .finished
    );
    progress.record(entry("a", 5.0, Some(5400.0), 5));
    assert!(
        progress
            .get("s1", ProgressKind::Movie, "a")
            .unwrap()
            .finished
    );
}
