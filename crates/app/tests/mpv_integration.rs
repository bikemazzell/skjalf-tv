//! Real-libmpv integration tests. Ignored by default (need the vendored
//! libmpv): run with
//! `PKG_CONFIG_PATH=... MPV_LIBRARY_DIR=... LD_LIBRARY_PATH=... cargo test --test mpv_integration -- --ignored`.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use skjalf::player::{BufferPlan, MpvEngine, PlayerEngine, PlayerError, PlayerEvent};
use skjalf_core::tracks::{selected, SubtitlePref, Track, TrackKind};

fn clip() -> PathBuf {
    generated_clip("skjalf-m8-testsrc.mp4", 3)
}

fn long_clip() -> PathBuf {
    generated_clip("skjalf-c5-testsrc6.mp4", 6)
}

fn generated_clip(name: &str, seconds: u32) -> PathBuf {
    let path = std::env::temp_dir().join(name);
    if !path.exists() {
        let spec = format!("testsrc=duration={seconds}:size=64x48:rate=10");
        let status = Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", &spec])
            .args(["-pix_fmt", "yuv420p"])
            .arg(&path)
            .status()
            .expect("ffmpeg installed on dev box");
        assert!(status.success(), "ffmpeg clip generation failed");
    }
    path
}

#[test]
#[ignore]
fn headless_loads_and_plays() {
    let mut e = MpvEngine::headless().expect("libmpv init");
    e.set_volume(50).unwrap();
    assert!(!e.is_paused());
    e.pause(true).unwrap();
    assert!(e.is_paused());
    e.pause(false).unwrap();

    e.load(clip().to_str().unwrap()).unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut saw_eos = false;
    let mut max_pos = 0.0f64;
    while Instant::now() < deadline {
        if e.take_events().contains(&PlayerEvent::Ended) {
            saw_eos = true;
        }
        max_pos = max_pos.max(e.position().unwrap_or(0.0));
        if saw_eos || max_pos >= 2.0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(max_pos > 0.5, "playback advanced (pos={max_pos})");
    assert!(
        saw_eos || max_pos >= 2.0,
        "reached EOS or 2s (pos={max_pos}, eos={saw_eos})"
    );
}

#[test]
#[ignore]
fn tls_verification_on_by_default() {
    let e = MpvEngine::new(80, false, None).expect("libmpv init");
    let verify = e
        .with_client(|c| Ok::<bool, PlayerError>(c.get_property("tls-verify").unwrap_or(true)))
        .expect("property read");
    assert!(verify, "stream credentials must be TLS-protected");

    let e = MpvEngine::new(80, true, None).expect("libmpv init");
    let verify = e
        .with_client(|c| Ok::<bool, PlayerError>(c.get_property("tls-verify").unwrap_or(true)))
        .expect("property read");
    assert!(!verify, "ignore_ssl setting disables verification");
}

#[test]
#[ignore]
fn output_drivers_match_embedded_and_headless_modes() {
    let embedded = MpvEngine::new(80, false, None).expect("libmpv init");
    let vo: String = embedded
        .with_client(|client| client.get_property("vo").map_err(PlayerError::from))
        .expect("video output property");
    assert_eq!(vo, "libmpv");

    let headless = MpvEngine::headless().expect("headless libmpv init");
    let vo: String = headless
        .with_client(|client| client.get_property("vo").map_err(PlayerError::from))
        .expect("headless video output property");
    assert_eq!(vo, "null");
}

fn property<T: rsmpv::GetProperty>(engine: &MpvEngine, name: &str) -> T {
    engine
        .with_client(|client| client.get_property(name).map_err(PlayerError::from))
        .unwrap_or_else(|error| panic!("{name}: {error}"))
}

#[test]
#[ignore]
fn a_cache_directory_moves_the_buffer_to_disk_with_a_larger_limit() {
    let dir = std::env::temp_dir().join(format!("skjalf-disk-cache-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let on_disk = MpvEngine::new(80, false, Some(&dir)).expect("libmpv init");
    assert!(property::<bool>(&on_disk, "cache-on-disk"));
    assert_eq!(
        property::<String>(&on_disk, "demuxer-cache-dir"),
        dir.to_str().unwrap()
    );
    assert_eq!(property::<i64>(&on_disk, "demuxer-max-bytes"), 4 << 30);

    let in_memory = MpvEngine::new(80, false, None).expect("libmpv init");
    assert!(!property::<bool>(&in_memory, "cache-on-disk"));
    assert_eq!(property::<i64>(&in_memory, "demuxer-max-bytes"), 150 << 20);
    std::fs::remove_dir_all(dir).ok();
}

/// Poll the engine until `done` holds or time runs out.
fn poll_until(engine: &mut MpvEngine, done: impl Fn(&MpvEngine) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        engine.take_events();
        if done(engine) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn buffered_load(start_secs: f64, stall_secs: f64) -> MpvEngine {
    let mut engine = MpvEngine::headless().expect("libmpv init");
    engine.load(long_clip().to_str().unwrap()).unwrap();
    engine.pause(false).unwrap();
    engine
        .set_buffering(BufferPlan {
            start_secs,
            stall_secs,
        })
        .unwrap();
    engine
}

#[test]
#[ignore]
fn a_start_wait_holds_the_file_paused_until_the_buffer_is_full() {
    let mut engine = buffered_load(60.0, 7.0);
    assert!(engine.is_paused(), "the file waits paused");
    let status = engine.buffer_status().expect("buffer status");
    assert!(status.waiting);
    assert_eq!(status.target_secs, 60.0);
    assert!(!property::<bool>(&engine, "cache-pause-initial"));
    assert_eq!(property::<f64>(&engine, "cache-pause-wait"), 7.0);
    assert!(
        poll_until(&mut engine, |engine| !engine.is_paused()),
        "a local file is read to its end, so playback starts"
    );
    let status = engine.buffer_status().expect("buffer status");
    assert!(!status.waiting);
    assert_eq!(status.target_secs, 7.0, "later stalls use the stall wait");
}

#[test]
#[ignore]
fn start_now_ends_the_start_wait_at_once() {
    let mut engine = buffered_load(600.0, 5.0);
    assert!(engine.is_paused());
    engine.start_now().unwrap();
    assert!(!engine.buffer_status().expect("buffer status").waiting);
    assert!(poll_until(&mut engine, |engine| !engine.is_paused()));
    assert_eq!(property::<f64>(&engine, "cache-pause-wait"), 5.0);
}

#[test]
#[ignore]
fn a_failed_load_ends_the_start_wait() {
    let mut engine = MpvEngine::headless().expect("libmpv init");
    let missing = std::env::temp_dir().join(format!("skjalf-missing-wait-{}", std::process::id()));
    engine.load(missing.to_str().unwrap()).unwrap();
    engine.pause(false).unwrap();
    engine
        .set_buffering(BufferPlan {
            start_secs: 600.0,
            stall_secs: 1.0,
        })
        .unwrap();
    assert!(engine.buffer_status().expect("buffer status").waiting);
    assert!(
        poll_until(&mut engine, |engine| !engine
            .buffer_status()
            .expect("buffer status")
            .waiting),
        "the error ends the wait, so no buffering screen covers it"
    );
}

#[test]
#[ignore]
fn start_now_during_a_stall_restores_the_stall_wait_afterward() {
    let mut engine = buffered_load(0.0, 9.0);
    assert!(!engine.is_paused());
    engine.start_now().unwrap();
    assert!(
        poll_until(&mut engine, |engine| {
            let wait = property::<f64>(engine, "cache-pause-wait");
            (wait - 0.5).abs() < 1e-6 || (wait - 9.0).abs() < 1e-6
        }),
        "start now lowers the wait"
    );
    assert!(
        poll_until(&mut engine, |engine| {
            (property::<f64>(engine, "cache-pause-wait") - 9.0).abs() < 1e-6
        }),
        "the stall wait comes back once playback runs"
    );
}

#[test]
#[ignore]
fn stop_returns_to_idle() {
    let mut e = MpvEngine::headless().unwrap();
    e.load(clip().to_str().unwrap()).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    e.stop().unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(e.position().unwrap_or(0.0), 0.0);
}

#[test]
#[ignore]
fn missing_file_reports_async_error() {
    let mut engine = MpvEngine::headless().expect("libmpv init");
    let missing = std::env::temp_dir().join(format!("skjalf-missing-{}", std::process::id()));
    assert!(!missing.exists());
    engine
        .load(missing.to_str().unwrap())
        .expect("load command accepted");
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if !engine.take_errors().is_empty() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("libmpv's missing-file error did not reach the engine");
}

/// Drain engine events until `Loaded` arrives.
fn wait_loaded(engine: &mut MpvEngine) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if engine.take_events().contains(&PlayerEvent::Loaded) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("file did not load");
}

fn wait_position(engine: &MpvEngine, at_least: f64) -> f64 {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Some(position) = engine.position() {
            if position >= at_least {
                return position;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("position never reached {at_least}: {:?}", engine.position());
}

#[test]
#[ignore]
fn load_at_starts_playback_at_the_requested_position() {
    let mut engine = MpvEngine::headless().unwrap();
    engine
        .load_at(long_clip().to_str().unwrap(), Some(1.5))
        .unwrap();
    wait_loaded(&mut engine);
    assert!(wait_position(&engine, 1.4) >= 1.4);
}

#[test]
#[ignore]
fn seek_relative_moves_the_position() {
    let mut engine = MpvEngine::headless().unwrap();
    engine.pause(true).unwrap();
    engine.load(long_clip().to_str().unwrap()).unwrap();
    wait_loaded(&mut engine);
    let before = wait_position(&engine, 0.0);
    engine.seek_relative(3.0).unwrap();
    let after = wait_position(&engine, before + 2.5);
    assert!(after >= before + 2.5, "{before} -> {after}");
    engine.seek_relative(-2.0).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if engine.position().is_some_and(|p| p <= after - 1.5) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("backward seek did not take effect");
}

#[test]
#[ignore]
fn duration_matches_the_clip_length() {
    let mut engine = MpvEngine::headless().unwrap();
    engine.load(long_clip().to_str().unwrap()).unwrap();
    wait_loaded(&mut engine);
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(duration) = engine.duration() {
            assert!((5.5..=6.5).contains(&duration), "duration {duration}");
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("duration never became known");
}

#[test]
#[ignore]
fn ended_event_arrives_once_at_end_of_file() {
    let mut engine = MpvEngine::headless().unwrap();
    engine.load(clip().to_str().unwrap()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut ended = 0;
    while Instant::now() < deadline && ended == 0 {
        ended += engine
            .take_events()
            .iter()
            .filter(|event| **event == PlayerEvent::Ended)
            .count();
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(ended, 1, "Ended arrives at end of file");
    std::thread::sleep(Duration::from_millis(1200));
    assert!(
        !engine.take_events().contains(&PlayerEvent::Ended),
        "Ended is reported once per load"
    );
}

#[test]
#[ignore]
fn a_later_load_without_start_begins_near_zero() {
    let mut engine = MpvEngine::headless().unwrap();
    engine
        .load_at(long_clip().to_str().unwrap(), Some(4.0))
        .unwrap();
    wait_loaded(&mut engine);
    wait_position(&engine, 3.9);
    engine.pause(true).unwrap();
    engine.load_at(long_clip().to_str().unwrap(), None).unwrap();
    wait_loaded(&mut engine);
    let position = engine.position().unwrap_or(0.0);
    assert!(position < 1.0, "start leaked into a later load: {position}");
}

/// 6 s clip: video, English + German audio, English subtitles.
/// Generated once per test run.
fn multitrack_clip() -> PathBuf {
    static CLIP: OnceLock<PathBuf> = OnceLock::new();
    CLIP.get_or_init(generate_multitrack_clip).clone()
}

fn generate_multitrack_clip() -> PathBuf {
    let path = std::env::temp_dir().join("skjalf-tracks-testsrc.mkv");
    if !path.exists() {
        let srt = std::env::temp_dir().join("skjalf-tracks-testsrc.srt");
        std::fs::write(&srt, "1\n00:00:00,000 --> 00:00:06,000\nHello\n").unwrap();
        let status = Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error"])
            .args(["-f", "lavfi", "-i", "testsrc=duration=6:size=64x48:rate=10"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=6"])
            .args(["-f", "lavfi", "-i", "sine=frequency=880:duration=6"])
            .arg("-i")
            .arg(&srt)
            .args(["-map", "0:v", "-map", "1:a", "-map", "2:a", "-map", "3:s"])
            .args(["-pix_fmt", "yuv420p", "-c:a", "aac", "-c:s", "srt"])
            .args(["-metadata:s:a:0", "language=eng"])
            .args([
                "-metadata:s:a:1",
                "language=ger",
                "-metadata:s:a:1",
                "title=Deutsch",
            ])
            .args(["-metadata:s:s:0", "language=eng"])
            .args(["-disposition:a:0", "default", "-disposition:s:0", "0"])
            .arg(&path)
            .status()
            .expect("ffmpeg installed on dev box");
        // mpv would auto-load a same-named .srt as an extra external track.
        std::fs::remove_file(&srt).unwrap();
        assert!(status.success(), "ffmpeg multitrack clip generation failed");
    }
    path
}

/// Poll the engine's observed track list until `done` holds (10 s budget).
fn wait_tracks(e: &MpvEngine, done: impl Fn(&[Track]) -> bool) -> Vec<Track> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let tracks = e.tracks();
        if done(&tracks) || Instant::now() >= deadline {
            return tracks;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn selected_lang(tracks: &[Track], kind: TrackKind) -> Option<String> {
    selected(tracks, kind).and_then(|t| t.lang.clone())
}

fn both_audio_tracks(tracks: &[Track]) -> bool {
    tracks.iter().filter(|t| t.kind == TrackKind::Audio).count() == 2
        && selected(tracks, TrackKind::Audio).is_some()
}

#[test]
#[ignore]
fn lists_switches_and_disables_tracks() {
    let mut e = MpvEngine::headless().unwrap();
    e.set_track_prefs(None, &SubtitlePref::Auto).unwrap();
    e.load(multitrack_clip().to_str().unwrap()).unwrap();

    let tracks = wait_tracks(&e, both_audio_tracks);
    let langs: Vec<_> = tracks
        .iter()
        .map(|t| (t.kind, t.lang.as_deref(), t.title.as_deref()))
        .collect();
    assert_eq!(
        langs,
        vec![
            (TrackKind::Audio, Some("eng"), None),
            (TrackKind::Audio, Some("ger"), Some("Deutsch")),
            (TrackKind::Sub, Some("eng"), None),
        ]
    );
    assert_eq!(
        selected_lang(&tracks, TrackKind::Audio).as_deref(),
        Some("eng")
    );

    // Switches go out asynchronously; wait for mpv's own track list to agree.
    let german = tracks[1].id;
    e.select_track(TrackKind::Audio, Some(german)).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let tracks = wait_tracks(&e, |t| {
        selected_lang(t, TrackKind::Audio).as_deref() == Some("ger")
    });
    assert_eq!(
        selected_lang(&tracks, TrackKind::Audio).as_deref(),
        Some("ger")
    );
    assert_eq!(property::<i64>(&e, "aid"), german, "mpv itself switched");

    let sub = tracks[2].id;
    e.select_track(TrackKind::Sub, Some(sub)).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let tracks = wait_tracks(&e, |t| selected(t, TrackKind::Sub).is_some());
    assert_eq!(selected(&tracks, TrackKind::Sub).map(|t| t.id), Some(sub));
    assert_eq!(property::<i64>(&e, "sid"), sub);

    e.select_track(TrackKind::Sub, None).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let tracks = wait_tracks(&e, |t| selected(t, TrackKind::Sub).is_none());
    assert!(selected(&tracks, TrackKind::Sub).is_none(), "subtitles off");
    assert_eq!(property::<String>(&e, "sid"), "no");
}

#[test]
#[ignore]
fn language_preferences_pick_tracks_on_every_load() {
    let clip = multitrack_clip();
    let clip = clip.to_str().unwrap();
    let mut e = MpvEngine::headless().unwrap();

    // Canonical "de" must match the stream's ISO 639-2 "ger" tag.
    e.set_track_prefs(Some("de"), &SubtitlePref::Lang("en".into()))
        .unwrap();
    e.load(clip).unwrap();
    let tracks = wait_tracks(&e, |t| {
        both_audio_tracks(t) && selected(t, TrackKind::Sub).is_some()
    });
    assert_eq!(
        selected_lang(&tracks, TrackKind::Audio).as_deref(),
        Some("ger")
    );
    assert_eq!(
        selected_lang(&tracks, TrackKind::Sub).as_deref(),
        Some("eng")
    );

    // mpv carries a manual switch over to the next file; re-applying the
    // preferences before each load (as the app does) must undo it.
    e.select_track(TrackKind::Audio, Some(tracks[0].id))
        .unwrap();
    e.select_track(TrackKind::Sub, None).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    e.set_track_prefs(Some("de"), &SubtitlePref::Lang("en".into()))
        .unwrap();
    e.load(clip).unwrap();
    let tracks = wait_tracks(&e, |t| {
        both_audio_tracks(t)
            && selected_lang(t, TrackKind::Audio).as_deref() == Some("ger")
            && selected(t, TrackKind::Sub).is_some()
    });
    assert_eq!(
        selected_lang(&tracks, TrackKind::Audio).as_deref(),
        Some("ger")
    );
    assert_eq!(
        selected_lang(&tracks, TrackKind::Sub).as_deref(),
        Some("eng")
    );

    e.set_track_prefs(Some("en"), &SubtitlePref::Off).unwrap();
    e.load(clip).unwrap();
    let tracks = wait_tracks(&e, |t| {
        both_audio_tracks(t) && selected_lang(t, TrackKind::Audio).as_deref() == Some("eng")
    });
    assert_eq!(
        selected_lang(&tracks, TrackKind::Audio).as_deref(),
        Some("eng")
    );
    assert!(
        selected(&tracks, TrackKind::Sub).is_none(),
        "Off wins over stream flags"
    );
}

#[test]
#[ignore]
fn a_track_switch_waits_briefly_then_restores_the_stall_wait() {
    let clip = multitrack_clip();
    let mut e = MpvEngine::headless().unwrap();
    e.set_track_prefs(None, &SubtitlePref::Auto).unwrap();
    e.load(clip.to_str().unwrap()).unwrap();
    e.pause(false).unwrap();
    e.set_buffering(BufferPlan {
        start_secs: 0.0,
        stall_secs: 10.0,
    })
    .unwrap();
    let tracks = wait_tracks(&e, both_audio_tracks);
    assert!(poll_until(&mut e, |e| e
        .position()
        .is_some_and(|p| p > 0.5)));

    e.select_track(TrackKind::Audio, Some(tracks[1].id))
        .unwrap();
    assert!(
        poll_until(&mut e, |e| property::<f64>(e, "cache-pause-wait") < 1.0),
        "a refill after the switch waits only briefly"
    );
    let switched_at = e.position().unwrap();
    assert!(
        poll_until(&mut e, |e| e
            .position()
            .is_some_and(|p| p > switched_at + 1.5)
            && property::<f64>(e, "cache-pause-wait") == 10.0),
        "the configured stall wait returns once playback runs on"
    );
}

/// 3 s clip: English audio, default-flagged English subtitles only.
fn default_subtitle_clip() -> PathBuf {
    let path = std::env::temp_dir().join("skjalf-default-subs.mkv");
    if !path.exists() {
        let srt = std::env::temp_dir().join("skjalf-default-subs-input.srt");
        std::fs::write(&srt, "1\n00:00:00,000 --> 00:00:03,000\nHello\n").unwrap();
        let status = Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error"])
            .args(["-f", "lavfi", "-i", "testsrc=duration=3:size=64x48:rate=10"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=3"])
            .arg("-i")
            .arg(&srt)
            .args(["-map", "0:v", "-map", "1:a", "-map", "2:s"])
            .args(["-pix_fmt", "yuv420p", "-c:a", "aac", "-c:s", "srt"])
            .args([
                "-metadata:s:a:0",
                "language=eng",
                "-metadata:s:s:0",
                "language=eng",
            ])
            .args(["-disposition:s:0", "default"])
            .arg(&path)
            .status()
            .expect("ffmpeg installed on dev box");
        std::fs::remove_file(&srt).unwrap();
        assert!(
            status.success(),
            "ffmpeg default-subtitle clip generation failed"
        );
    }
    path
}

#[test]
#[ignore]
fn a_subtitle_language_that_is_missing_shows_no_subtitles() {
    let clip = default_subtitle_clip();
    let clip = clip.to_str().unwrap();
    let mut e = MpvEngine::headless().unwrap();

    // Stream default: the default-flagged English subtitles appear.
    e.set_track_prefs(None, &SubtitlePref::Auto).unwrap();
    e.load(clip).unwrap();
    let tracks = wait_tracks(&e, |t| selected(t, TrackKind::Sub).is_some());
    assert_eq!(
        selected_lang(&tracks, TrackKind::Sub).as_deref(),
        Some("eng")
    );

    // German wanted, none present: no other language stands in for it.
    e.set_track_prefs(Some("en"), &SubtitlePref::Lang("de".into()))
        .unwrap();
    e.load(clip).unwrap();
    assert!(poll_until(&mut e, |e| e
        .position()
        .is_some_and(|p| p > 0.3)));
    let tracks = e.tracks();
    assert_eq!(
        selected_lang(&tracks, TrackKind::Audio).as_deref(),
        Some("eng")
    );
    assert!(selected(&tracks, TrackKind::Sub).is_none(), "{tracks:?}");
}
