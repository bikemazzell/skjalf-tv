//! Real-libmpv integration tests. Ignored by default (need the vendored
//! libmpv): run with
//! `PKG_CONFIG_PATH=... MPV_LIBRARY_DIR=... LD_LIBRARY_PATH=... cargo test --test mpv_integration -- --ignored`.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use skjalf::player::{MpvEngine, PlayerEngine, PlayerError, PlayerEvent};

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
        if let Some(ev) = e.poll_event() {
            if matches!(ev, rsmpv::Event::EndFile { .. }) {
                saw_eos = true;
            }
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
    let e = MpvEngine::new(80, false).expect("libmpv init");
    let verify = e
        .with_client(|c| Ok::<bool, PlayerError>(c.get_property("tls-verify").unwrap_or(true)))
        .expect("property read");
    assert!(verify, "stream credentials must be TLS-protected");

    let e = MpvEngine::new(80, true).expect("libmpv init");
    let verify = e
        .with_client(|c| Ok::<bool, PlayerError>(c.get_property("tls-verify").unwrap_or(true)))
        .expect("property read");
    assert!(!verify, "ignore_ssl setting disables verification");
}

#[test]
#[ignore]
fn output_drivers_match_embedded_and_headless_modes() {
    let embedded = MpvEngine::new(80, false).expect("libmpv init");
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
