//! Real-libmpv integration tests. Ignored by default (need the vendored
//! libmpv): run with
//! `PKG_CONFIG_PATH=... MPV_LIBRARY_DIR=... LD_LIBRARY_PATH=... cargo test --test mpv_integration -- --ignored`.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use skjalf::player::{BufferPlan, MpvEngine, PlayerEngine, PlayerError, PlayerEvent};

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
        property::<String>(&on_disk, "cache-dir"),
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
