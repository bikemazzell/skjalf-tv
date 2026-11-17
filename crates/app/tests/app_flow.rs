//! Headless App flow tests: navigation, playback dispatch, zap, favorites,
//! persistence — with a mock engine and injected fetcher (no
//! display/network/libmpv).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use skjalf::app::{
    App, BarControl, BrowserId, DetailInfo, LoadOrigin, LoadStatus, MainButton, MovieButton,
    PlaybackStatus, PlayerFocus, RefreshKind, SeriesFocus, SettingsFocus,
};
use skjalf::input::Action;
use skjalf::player::{PlayerEngine, PlayerError, PlayerEvent};
use skjalf::steam_keyboard::{KeyboardLauncher, CLOSE_URI, OPEN_URI};
use skjalf_core::browse::{BrowserFocus, CategorySel, SortMode};
use skjalf_core::catalog::{ContentKey, ContentKind, Movie, MovieCatalog};
use skjalf_core::model::{Channel, Group, Playlist, Settings, Source, SourceKind, StreamFormat};
use skjalf_core::nav::{OsdKind, View};
use skjalf_core::progress::ProgressKind;
use skjalf_core::store::Store;
use skjalf_core::xtream::{FetchError, HttpFetcher};

#[derive(Default, Clone)]
struct MockEngine {
    starts: Arc<Mutex<Vec<Option<f64>>>>,
    pauses: Arc<Mutex<Vec<bool>>>,
    seeks: Arc<Mutex<Vec<f64>>>,
    position: Arc<Mutex<Option<f64>>>,
    duration: Arc<Mutex<Option<f64>>>,
    loads: Arc<Mutex<Vec<String>>>,
    volumes: Arc<Mutex<Vec<u8>>>,
    stopped: Arc<Mutex<usize>>,
    events: Arc<Mutex<Vec<PlayerEvent>>>,
}

impl PlayerEngine for MockEngine {
    fn load_at(&mut self, url: &str, start: Option<f64>) -> Result<(), PlayerError> {
        self.loads.lock().unwrap().push(url.to_owned());
        self.starts.lock().unwrap().push(start);
        Ok(())
    }
    fn seek_relative(&mut self, secs: f64) -> Result<(), PlayerError> {
        self.seeks.lock().unwrap().push(secs);
        Ok(())
    }
    fn position(&self) -> Option<f64> {
        *self.position.lock().unwrap()
    }
    fn duration(&self) -> Option<f64> {
        *self.duration.lock().unwrap()
    }
    fn stop(&mut self) -> Result<(), PlayerError> {
        *self.stopped.lock().unwrap() += 1;
        Ok(())
    }
    fn set_volume(&mut self, volume: u8) -> Result<(), PlayerError> {
        self.volumes.lock().unwrap().push(volume);
        Ok(())
    }
    fn pause(&mut self, paused: bool) -> Result<(), PlayerError> {
        self.pauses.lock().unwrap().push(paused);
        Ok(())
    }
    fn is_paused(&self) -> bool {
        false
    }
    fn take_events(&mut self) -> Vec<PlayerEvent> {
        std::mem::take(&mut *self.events.lock().unwrap())
    }
}

/// Test default: never reach the desktop Steam client or its X display.
struct InertKeyboard;

impl KeyboardLauncher for InertKeyboard {
    fn send(&self, _uri: &'static str) -> mpsc::Receiver<Result<(), String>> {
        let (tx, rx) = mpsc::channel();
        let _ = tx.send(Ok(()));
        rx
    }
}

/// Fails every request unless a canned body is set (EPG tests).
struct MockFetcher {
    body: Option<String>,
}

impl HttpFetcher for MockFetcher {
    fn get(&self, _url: &str) -> Result<String, FetchError> {
        self.body
            .clone()
            .ok_or_else(|| FetchError("mock: offline".into()))
    }
}

fn source() -> Source {
    Source {
        id: "xtream:http://tv.example.com/u".into(),
        name: "tv.example.com".into(),
        kind: SourceKind::Xtream {
            host: "http://tv.example.com".into(),
            username: "u".into(),
            password: "p".into(),
        },
        stream_format: StreamFormat::Ts,
    }
}

fn playlist() -> Playlist {
    let ch = |id: &str, name: &str, num: u32, group: &str| Channel {
        id: id.into(),
        name: name.into(),
        num: Some(num),
        logo: None,
        group_id: group.into(),
        url: format!("http://tv.example.com/live/{{user}}/{{pass}}/{id}.ts"),
    };
    Playlist {
        source: source(),
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
            ch("1", "News 1", 101, "g1"),
            ch("2", "News 2", 102, "g1"),
            ch("4", "News 3", 103, "g1"),
            ch("3", "Sport 1", 201, "g2"),
            ch("5", "Sport 2", 202, "g2"),
        ],
    }
}

fn group(id: &str) -> CategorySel {
    CategorySel::Group(id.into())
}

fn seeded_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("skjalf-apptest-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let store = Store::open(&dir).unwrap();
    let settings = Settings {
        source: Some(source()),
        ..Default::default()
    };
    store.save_settings(&settings).unwrap();
    store.save_playlist(&playlist()).unwrap();
    dir
}

fn main_app_at(dir: &std::path::Path) -> App {
    let mut a = App::with_store(dir);
    a.fetcher = Arc::new(MockFetcher { body: None });
    a.keyboard_launcher = Arc::new(InertKeyboard);
    a
}

fn main_app(tag: &str) -> App {
    main_app_at(&seeded_dir(tag))
}

fn app_at(dir: &std::path::Path) -> App {
    let mut a = main_app_at(dir);
    a.activate_main_button(MainButton::LiveTv);
    a
}

fn app(tag: &str) -> App {
    app_at(&seeded_dir(tag))
}

fn settings_app(tag: &str) -> App {
    let mut a = main_app(tag);
    a.activate_main_button(MainButton::Settings);
    a
}

fn open_editor(a: &mut App) {
    a.router.reset(View::Main);
    a.activate_main_button(MainButton::Settings);
    a.activate_settings_button(SettingsFocus::EditSource);
}

#[test]
fn startup_with_cached_playlist_lands_on_main_with_live_selected() {
    let a = main_app("startup");
    assert_eq!(a.router.current(), View::Main);
    assert_eq!(a.main_focus, MainButton::LiveTv);
    // Favorites pseudo-group is pinned at 0 but empty → first real group.
    assert_eq!(a.live.category, group("g1"));
    assert_eq!(
        a.category_label(BrowserId::Live, a.category_row(BrowserId::Live))
            .as_deref(),
        Some("News")
    );
}

#[test]
fn startup_restores_saved_channel_without_playing() {
    let dir = seeded_dir("last_channel");
    let store = Store::open(&dir).unwrap();
    let mut settings = store.load_settings().unwrap();
    settings.last_channel = Some((source().id, "2".into()));
    store.save_settings(&settings).unwrap();
    let a = main_app_at(&dir);
    assert_eq!(a.router.current(), View::Main);
    assert_eq!(a.live.selected, 1);
    assert!(a.player.channel_id.is_none());
}

#[test]
fn connection_failure_keeps_working_playlist_and_reports_it() {
    let mut a = app("refresh_failure");
    let original = a.playlist.clone();
    let (tx, rx) = mpsc::channel();
    tx.send(skjalf::app::ConnectOutcome::Failed("offline".into()))
        .unwrap();
    a.connect_rx = Some(rx);
    a.poll_jobs(0);
    assert_eq!(a.playlist, original);
    assert_eq!(a.router.current(), View::Live);
    assert!(a
        .refresh_status
        .as_deref()
        .unwrap_or_default()
        .contains("offline"));
    assert_eq!(a.notice, None, "failures stay out of other screens");
}

#[test]
fn successful_connection_selects_first_nonempty_group() {
    let mut a = app("connection_group");
    let (tx, rx) = mpsc::channel();
    tx.send(skjalf::app::ConnectOutcome::Ready(playlist()))
        .unwrap();
    a.connect_rx = Some(rx);
    a.poll_jobs(0);
    assert_eq!(a.live.category, group("g1"));
    assert!(!a.group_channel_ids().is_empty());
}

#[test]
fn source_edit_prefills_and_back_cancels() {
    let mut a = app("edit_cancel");
    let original = a.settings.source.clone();
    open_editor(&mut a);
    assert_eq!(a.router.current(), View::Setup);
    assert_eq!(a.setup.host, "http://tv.example.com");
    assert_eq!(a.setup.username, "u");
    assert_eq!(a.setup.password, "p");
    a.setup.password = "replacement".into();
    a.handle_action(Action::Back, 1);
    assert_eq!(a.router.current(), View::Settings);
    assert_eq!(a.settings.source, original);
    assert!(a.playlist.is_some());
}

#[test]
fn setup_physical_edit_keys_do_not_run_controller_actions() {
    let mut a = app("setup_raw_keys");
    open_editor(&mut a);
    a.activate_setup_field(skjalf::setup_model::Field::Username);
    let key = |key| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    };
    a.pump_event(&key(egui::Key::Backspace), 0);
    a.pump_event(&key(egui::Key::ArrowRight), 0);
    a.pump_event(&egui::Event::Text("x".into()), 0);
    assert_eq!(a.router.current(), View::Setup);
    assert_eq!(a.setup.username, "u");
    assert_eq!(a.setup.active, skjalf::setup_model::Field::Username);
    a.pump_event(&key(egui::Key::Escape), 0);
    assert_eq!(a.router.current(), View::Setup);
    a.pump_event(&key(egui::Key::Escape), 0);
    assert_eq!(a.router.current(), View::Settings);
}

#[test]
fn raw_remote_keys_navigate_setup_then_open_and_leave_field() {
    let mut a = app("raw_remote_setup");
    open_editor(&mut a);
    let key = |key| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    };
    assert!(a.pump_event(&key(egui::Key::ArrowDown), 0));
    assert_eq!(a.setup.row, skjalf::setup_model::Row::Field(0));
    assert!(a.pump_event(&key(egui::Key::Enter), 0));
    assert!(a.setup_field_editing());
    assert!(!a.pump_event(&key(egui::Key::ArrowRight), 0));
    assert_eq!(a.setup.row, skjalf::setup_model::Row::Field(0));
    assert!(a.pump_event(&key(egui::Key::BrowserBack), 0));
    assert!(!a.setup_field_editing());
    assert_eq!(a.router.current(), View::Setup);
    assert!(a.pump_event(&key(egui::Key::BrowserBack), 0));
    assert_eq!(a.router.current(), View::Settings);
}

#[test]
fn steam_keyboard_opens_once_and_blocks_navigation_until_done() {
    struct Recorder(Arc<Mutex<Vec<&'static str>>>);
    impl KeyboardLauncher for Recorder {
        fn send(&self, uri: &'static str) -> mpsc::Receiver<Result<(), String>> {
            self.0.lock().unwrap().push(uri);
            let (tx, rx) = mpsc::channel();
            tx.send(Ok(())).unwrap();
            rx
        }
    }
    let mut a = app("steam_keyboard");
    let requests = Arc::new(Mutex::new(Vec::new()));
    a.keyboard_launcher = Arc::new(Recorder(requests.clone()));
    a.game_mode = true;
    open_editor(&mut a);
    a.setup.focus_field(skjalf::setup_model::Field::Host);
    a.handle_action(Action::Select, 0);
    a.poll_jobs(0);
    assert!(a.steam_keyboard_active());
    assert_eq!(
        a.active_steam_keyboard_field(),
        Some(skjalf::setup_model::Field::Host)
    );
    a.handle_action(Action::Down, 0);
    assert_eq!(a.setup.active, skjalf::setup_model::Field::Host);
    a.handle_action(Action::Back, 0); // leave field and close keyboard
    assert!(!a.steam_keyboard_active());
    assert_eq!(a.active_steam_keyboard_field(), None);
    assert!(!a.setup_field_editing());
    a.handle_action(Action::Back, 0); // now cancel source edit
    assert_eq!(a.router.current(), View::Settings);
    assert_eq!(*requests.lock().unwrap(), [OPEN_URI, CLOSE_URI]);
}

#[test]
fn native_keyboard_dismissal_restores_field_navigation_without_extra_back() {
    struct Recorder {
        requests: Arc<Mutex<Vec<&'static str>>>,
        dismissal: Mutex<Option<mpsc::Sender<()>>>,
    }
    impl KeyboardLauncher for Recorder {
        fn send(&self, uri: &'static str) -> mpsc::Receiver<Result<(), String>> {
            self.requests.lock().unwrap().push(uri);
            let (tx, rx) = mpsc::channel();
            tx.send(Ok(())).unwrap();
            rx
        }
        fn watch_dismissal(&self, _cancel: Arc<AtomicBool>) -> Option<mpsc::Receiver<()>> {
            let (tx, rx) = mpsc::channel();
            *self.dismissal.lock().unwrap() = Some(tx);
            Some(rx)
        }
    }
    let mut a = app("native_keyboard_dismissal");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::new(Recorder {
        requests: requests.clone(),
        dismissal: Mutex::new(None),
    });
    a.keyboard_launcher = recorder.clone();
    a.game_mode = true;
    open_editor(&mut a);
    a.activate_setup_field(skjalf::setup_model::Field::Host);
    a.poll_jobs(0);
    recorder
        .dismissal
        .lock()
        .unwrap()
        .take()
        .unwrap()
        .send(())
        .unwrap();
    a.poll_keyboard(); // UI update processes this before the frame's input.
    assert!(!a.setup_field_editing());
    assert!(!a.steam_keyboard_active());
    assert_eq!(a.active_steam_keyboard_field(), None);
    assert_eq!(a.setup.row, skjalf::setup_model::Row::Field(0));
    let key = |key| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    };
    assert!(a.pump_event(&key(egui::Key::ArrowDown), 2));
    assert_eq!(a.setup.row, skjalf::setup_model::Row::Field(1));
    assert!(a.pump_event(&key(egui::Key::BrowserBack), 3));
    assert_eq!(a.router.current(), View::Settings);
    assert_eq!(*requests.lock().unwrap(), [OPEN_URI]);
}

#[test]
fn stale_keyboard_monitor_cannot_close_a_new_edit_session() {
    struct Recorder(Mutex<Vec<(mpsc::Sender<()>, Arc<AtomicBool>)>>);
    impl KeyboardLauncher for Recorder {
        fn send(&self, _uri: &'static str) -> mpsc::Receiver<Result<(), String>> {
            let (tx, rx) = mpsc::channel();
            tx.send(Ok(())).unwrap();
            rx
        }
        fn watch_dismissal(&self, cancel: Arc<AtomicBool>) -> Option<mpsc::Receiver<()>> {
            let (tx, rx) = mpsc::channel();
            self.0.lock().unwrap().push((tx, cancel));
            Some(rx)
        }
    }
    let mut a = app("stale_keyboard_monitor");
    let recorder = Arc::new(Recorder(Mutex::new(Vec::new())));
    a.keyboard_launcher = recorder.clone();
    a.game_mode = true;
    open_editor(&mut a);
    a.activate_setup_field(skjalf::setup_model::Field::Host);
    a.handle_action(Action::Back, 0);
    let old = recorder.0.lock().unwrap().remove(0);
    assert!(old.1.load(Ordering::Relaxed));
    a.activate_setup_field(skjalf::setup_model::Field::Username);
    old.0.send(()).ok();
    a.poll_jobs(1);
    assert!(a.setup_field_editing());
    assert_eq!(a.setup.active, skjalf::setup_model::Field::Username);
    let current_cancel = recorder.0.lock().unwrap()[0].1.clone();
    drop(a);
    assert!(current_cancel.load(Ordering::Relaxed));
}

#[test]
fn unavailable_monitor_keeps_manual_back_and_desktop_editing() {
    struct Recorder(Arc<Mutex<Vec<&'static str>>>);
    impl KeyboardLauncher for Recorder {
        fn send(&self, uri: &'static str) -> mpsc::Receiver<Result<(), String>> {
            self.0.lock().unwrap().push(uri);
            let (tx, rx) = mpsc::channel();
            tx.send(Ok(())).unwrap();
            rx
        }
    }
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut a = app("keyboard_monitor_unavailable");
    a.keyboard_launcher = Arc::new(Recorder(requests.clone()));
    open_editor(&mut a);
    a.activate_setup_field(skjalf::setup_model::Field::Host);
    a.poll_keyboard();
    assert!(a.setup_field_editing());
    assert!(requests.lock().unwrap().is_empty()); // desktop physical editing
    a.finish_setup_edit();

    a.game_mode = true;
    a.activate_setup_field(skjalf::setup_model::Field::Host);
    a.poll_keyboard();
    assert!(a.setup_field_editing());
    a.handle_action(Action::Back, 1);
    assert!(!a.setup_field_editing());
    assert_eq!(*requests.lock().unwrap(), [OPEN_URI, CLOSE_URI]);
}

#[test]
fn failed_keyboard_open_cancels_monitor_without_close_request() {
    struct Failing {
        requests: Arc<Mutex<Vec<&'static str>>>,
        cancel: Arc<Mutex<Option<Arc<AtomicBool>>>>,
    }
    impl KeyboardLauncher for Failing {
        fn send(&self, uri: &'static str) -> mpsc::Receiver<Result<(), String>> {
            self.requests.lock().unwrap().push(uri);
            let (tx, rx) = mpsc::channel();
            tx.send(Err("could not open Steam keyboard".into()))
                .unwrap();
            rx
        }
        fn watch_dismissal(&self, cancel: Arc<AtomicBool>) -> Option<mpsc::Receiver<()>> {
            *self.cancel.lock().unwrap() = Some(cancel);
            let (_tx, rx) = mpsc::channel();
            Some(rx)
        }
    }
    let requests = Arc::new(Mutex::new(Vec::new()));
    let cancel = Arc::new(Mutex::new(None));
    let mut a = app("keyboard_open_failure");
    a.keyboard_launcher = Arc::new(Failing {
        requests: requests.clone(),
        cancel: cancel.clone(),
    });
    a.game_mode = true;
    open_editor(&mut a);
    a.activate_setup_field(skjalf::setup_model::Field::Host);
    a.poll_keyboard();
    assert!(!a.steam_keyboard_active());
    assert!(cancel
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .load(Ordering::Relaxed));
    assert!(a.setup_field_editing());
    a.handle_action(Action::Back, 1);
    assert!(!a.setup_field_editing());
    assert_eq!(*requests.lock().unwrap(), [OPEN_URI]);
}

#[test]
fn refresh_failure_keeps_saved_source_and_channels() {
    let mut a = settings_app("refresh_action");
    let old_source = a.settings.source.clone();
    let old_playlist = a.playlist.clone();
    a.handle_action(Action::Down, 0);
    a.handle_action(Action::Select, 0);
    assert!(a.connect_rx.is_some());
    for _ in 0..200 {
        a.poll_jobs(0);
        if a.connect_rx.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(a.router.current(), View::Settings);
    assert_eq!(a.settings.source, old_source);
    assert_eq!(a.playlist, old_playlist);
    assert!(a.notice.as_deref().unwrap_or_default().contains("offline"));
}

#[test]
fn failed_source_replacement_keeps_prior_working_source() {
    let mut a = app("edit_failure");
    let old_settings = a.settings.clone();
    let old_playlist = a.playlist.clone();
    open_editor(&mut a);
    a.setup.password = "new-password".into();
    let replacement = a.setup.connect().unwrap().0;
    a.start_connect(replacement);
    for _ in 0..200 {
        a.poll_jobs(0);
        if a.connect_rx.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(a.router.current(), View::Setup);
    assert_eq!(a.settings, old_settings);
    assert_eq!(a.playlist, old_playlist);
    a.handle_action(Action::Back, 0);
    assert_eq!(a.router.current(), View::Settings);
}

#[test]
fn second_connection_does_not_replace_active_worker() {
    let mut a = app("connection_guard");
    let (tx, rx) = mpsc::channel();
    a.connect_rx = Some(rx);
    a.start_connect(source());
    tx.send(skjalf::app::ConnectOutcome::Ready(playlist()))
        .unwrap();
    a.poll_jobs(0);
    assert_eq!(a.router.current(), View::Live);
    assert!(a
        .refresh_status
        .as_deref()
        .unwrap_or_default()
        .contains("Last refresh"));
}

#[test]
fn refresh_completion_does_not_hide_active_player() {
    let mut a = app("refresh_during_playback");
    let engine = MockEngine::default();
    a.engine = Some(Box::new(engine.clone()));
    let (tx, rx) = mpsc::channel();
    a.connect_rx = Some(rx);
    a.handle_action(Action::Select, 0);
    assert_eq!(a.router.current(), View::Player);
    tx.send(skjalf::app::ConnectOutcome::Ready(playlist()))
        .unwrap();
    a.poll_jobs(1);
    assert_eq!(a.router.current(), View::Player);
    assert_eq!(a.player.channel_id.as_deref(), Some("1"));
    assert_eq!(*engine.stopped.lock().unwrap(), 0);
}

#[test]
fn source_edit_is_blocked_during_refresh() {
    let mut a = settings_app("edit_during_refresh");
    let (_tx, rx) = mpsc::channel();
    a.connect_rx = Some(rx);
    a.handle_action(Action::Select, 0);
    assert_eq!(a.router.current(), View::Settings);
    assert!(!a.editing_source);
    assert!(a.notice.as_deref().unwrap_or_default().contains("refresh"));
}

#[test]
fn player_loading_and_error_feedback_follow_engine_events() {
    let mut a = app("player_events");
    let engine = MockEngine::default();
    a.engine = Some(Box::new(engine.clone()));
    a.handle_action(Action::Select, 0);
    assert_eq!(a.playback_status, PlaybackStatus::Loading);
    engine.events.lock().unwrap().push(PlayerEvent::Loaded);
    a.poll_jobs(1);
    assert_eq!(a.playback_status, PlaybackStatus::Playing);
    assert!(!a.notice.as_deref().unwrap_or_default().contains("Loading"));
    engine
        .events
        .lock()
        .unwrap()
        .push(PlayerEvent::Failed("missing".into()));
    a.poll_jobs(2);
    assert!(matches!(a.playback_status, PlaybackStatus::Failed(_)));
    assert!(a.notice.as_deref().unwrap_or_default().contains("missing"));
    a.handle_action(Action::Back, 3);
    assert_eq!(a.playback_status, PlaybackStatus::Idle);
}

#[test]
fn favorite_save_error_is_visible() {
    let dir = seeded_dir("favorite_save_error");
    std::fs::create_dir(dir.join("favorites.json.tmp")).unwrap();
    let mut a = app_at(&dir);
    a.handle_action(Action::Favorite, 0);
    assert!(a
        .notice
        .as_deref()
        .unwrap_or_default()
        .contains("save favorites"));
}

#[test]
fn unavailable_store_does_not_hide_favorite_save_failure() {
    let path = std::env::temp_dir().join(format!("skjalf-not-directory-{}", std::process::id()));
    std::fs::write(&path, b"file").unwrap();
    let mut a = App::with_store(&path);
    a.keyboard_launcher = Arc::new(InertKeyboard);
    a.settings.source = Some(source());
    a.set_playlist(playlist());
    a.router.reset(View::Live);
    a.set_category(BrowserId::Live, group("g1"));
    a.handle_action(Action::Favorite, 0);
    assert!(a
        .notice
        .as_deref()
        .unwrap_or_default()
        .contains("not saved"));
}

#[test]
fn duplicate_local_channel_id_uses_provider_id_for_epg() {
    struct Recorder(Arc<Mutex<Vec<String>>>);
    impl HttpFetcher for Recorder {
        fn get(&self, url: &str) -> Result<String, FetchError> {
            self.0.lock().unwrap().push(url.into());
            Ok(r#"{"epg_listings":[]}"#.into())
        }
    }
    let mut a = app("duplicate_epg");
    a.playlist.as_mut().unwrap().channels[0].id = "1#2".into();
    let calls = Arc::new(Mutex::new(Vec::new()));
    a.fetcher = Arc::new(Recorder(calls.clone()));
    a.engine = Some(Box::new(MockEngine::default()));
    a.handle_action(Action::Select, 0);
    for _ in 0..100 {
        a.poll_jobs(0);
        if a.epg_rx.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let urls = calls.lock().unwrap();
    assert!(urls.iter().any(|url| url.contains("stream_id=1&")));
    assert!(!urls.iter().any(|url| url.contains("1%232")));
}

#[test]
fn live_navigation_moves_channels_and_groups() {
    let mut a = app("homenav");
    a.handle_action(Action::Down, 0); // News 1 → News 2
    assert_eq!(a.live.selected, 1);
    a.handle_action(Action::Down, 0); // → News 3
    a.handle_action(Action::Down, 0); // wraps back to News 1
    assert_eq!(a.live.selected, 0);
    a.handle_action(Action::Left, 0); // channel list → group list
    a.handle_action(Action::Down, 0); // News → Sports
    assert_eq!(a.live.category, group("g2"));
    assert_eq!(a.live.selected, 0); // cursor reset
    assert_eq!(a.group_channel_ids().len(), 2);
    a.handle_action(Action::Up, 0);
    assert_eq!(a.live.category, group("g1"));
}

fn shape_texts(shape: &egui::Shape, out: &mut Vec<String>) {
    match shape {
        egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                shape_texts(shape, out);
            }
        }
        _ => {}
    }
}

#[test]
fn long_last_channel_is_rendered_on_screen() {
    let mut a = app("scroll_last");
    let mut playlist = a.playlist.clone().unwrap();
    playlist.channels = (0..5000)
        .map(|n| Channel {
            id: n.to_string(),
            name: format!("CHANNEL_{n:04} {}\nnext line", "W".repeat(270)),
            num: Some(n),
            logo: None,
            group_id: "g1".into(),
            url: "/tmp/no-video".into(),
        })
        .collect();
    a.set_playlist(playlist);
    a.set_category(BrowserId::Live, group("g1"));
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    for frame in 0..10 {
        if frame == 2 {
            while a.live.selected != 4999 {
                a.handle_action(Action::Down, 100);
            }
            assert_eq!(a.live.selected, 4999);
        }
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                time: Some(frame as f64 * 0.1),
                ..Default::default()
            },
            |ctx| skjalf::views::live::show(ctx, &mut a),
        );
        if frame == 9 {
            let mut labels = Vec::new();
            for shape in &output.shapes {
                shape_texts(&shape.shape, &mut labels);
            }
            assert!(
                labels.iter().any(|text| text.contains("CHANNEL_4999")),
                "last channel stayed outside rendered rows"
            );
        }
    }
}

#[test]
fn select_plays_channel_with_templated_url_and_volume() {
    let mut a = app("select");
    let engine = MockEngine::default();
    a.engine = Some(Box::new(engine.clone()));
    a.handle_action(Action::Down, 0); // cursor → News 2
    a.handle_action(Action::Select, 100);

    assert_eq!(a.router.current(), View::Player);
    assert_eq!(a.player.channel_id.as_deref(), Some("2"));
    let loads = engine.loads.lock().unwrap();
    assert_eq!(
        loads.last().unwrap(),
        "http://tv.example.com/live/u/p/2.ts",
        "{{user}}/{{pass}} placeholders must be filled"
    );
    let volumes = engine.volumes.lock().unwrap();
    assert!(volumes.last().is_some_and(|v| *v == a.settings.volume));
}

#[test]
fn missing_engine_keeps_live_and_reports_failure() {
    let mut a = app("missing_engine");
    a.handle_action(Action::Select, 100);
    assert_eq!(a.router.current(), View::Live);
    assert!(a.player.channel_id.is_none());
}

#[test]
fn player_zap_by_number_and_commit_with_select() {
    let mut a = app("zap");
    let engine = MockEngine::default();
    a.engine = Some(Box::new(engine.clone()));
    a.handle_action(Action::Select, 0); // play News 1 (channel 0)
    assert_eq!(a.router.current(), View::Player);

    a.handle_action(Action::Digit(2), 1000);
    a.handle_action(Action::Digit(0), 1100);
    a.handle_action(Action::Digit(1), 1200);
    assert_eq!(a.zap.digits(), "201");
    a.handle_action(Action::Select, 1300); // commit

    assert_eq!(a.player.channel_id.as_deref(), Some("3"));
    let loads = engine.loads.lock().unwrap();
    assert_eq!(loads.last().unwrap(), "http://tv.example.com/live/u/p/3.ts");
}

#[test]
fn live_digits_buffer_until_commit() {
    let mut a = app("homedigits");
    a.engine = Some(Box::new(MockEngine::default()));
    a.handle_action(Action::Digit(1), 1000);
    a.handle_action(Action::Digit(0), 1100);
    a.handle_action(Action::Digit(2), 1200);
    // Still browsing Live; nothing plays until the buffer commits.
    assert_eq!(a.router.current(), View::Live);
    assert!(a.player.channel_id.is_none());
    a.handle_action(Action::Select, 1300); // commit "102" → News 2
    assert_eq!(a.router.current(), View::Player);
    assert_eq!(a.player.channel_id.as_deref(), Some("2"));
}

#[test]
fn zap_timeout_auto_commits_via_poll_jobs() {
    let mut a = app("zaptimeout");
    let engine = MockEngine::default();
    a.engine = Some(Box::new(engine.clone()));
    a.handle_action(Action::Select, 0); // play News 1 (id "1")
    a.handle_action(Action::Digit(1), 1000);
    a.handle_action(Action::Digit(0), 1100);
    a.handle_action(Action::Digit(2), 1200); // "102" → News 2
    a.poll_jobs(12_600); // past the 2.5 s deadline
    assert_eq!(
        a.player.channel_id.as_deref(),
        Some("2"),
        "zap buffer commits to a different channel on timeout"
    );
    assert!(engine.loads.lock().unwrap().len() >= 2);
}

#[test]
fn player_page_steps_within_group_with_direction() {
    let mut a = app("step");
    let engine = MockEngine::default();
    a.engine = Some(Box::new(engine.clone()));
    a.handle_action(Action::Select, 0); // News 1 (id "1"), group News
    a.handle_action(Action::ChUp, 0); // → News 2
    assert_eq!(a.player.channel_id.as_deref(), Some("2"));
    a.handle_action(Action::ChUp, 0); // → News 3
    assert_eq!(a.player.channel_id.as_deref(), Some("4"));
    a.handle_action(Action::ChDown, 0); // back → News 2
    assert_eq!(a.player.channel_id.as_deref(), Some("2"));
    a.handle_action(Action::ChDown, 0); // back → News 1
    assert_eq!(a.player.channel_id.as_deref(), Some("1"));
    assert_eq!(engine.loads.lock().unwrap().len(), 5); // 1 play + 4 steps
}

#[test]
fn player_bar_opens_with_down_and_up_or_back_returns_to_video() {
    let mut a = app("player_controls");
    a.engine = Some(Box::new(MockEngine::default()));
    a.handle_action(Action::Select, 0);
    assert_eq!(a.router.current(), View::Player);
    a.handle_action(Action::Down, 0);
    assert_eq!(a.player_focus, PlayerFocus::Controls(2));
    a.handle_action(Action::Up, 0);
    assert_eq!(a.player_focus, PlayerFocus::Video);
    a.handle_action(Action::Down, 0);
    a.handle_action(Action::Back, 0);
    assert_eq!(a.player_focus, PlayerFocus::Video);
    assert_eq!(a.router.current(), View::Player);
    a.handle_action(Action::Favorite, 0);
    assert!(
        a.favorites.contains(&source().id, "1"),
        "Favorite stays on Y"
    );
    a.handle_action(Action::Back, 0);
    assert_eq!(a.router.current(), View::Live);
}

#[test]
fn zap_cross_group_updates_live_group_and_stepping_follows() {
    let mut a = app("crossgroup");
    let engine = MockEngine::default();
    a.engine = Some(Box::new(engine.clone()));
    a.handle_action(Action::Select, 0); // News 1; live.group == 1 (News)

    a.handle_action(Action::Digit(2), 1000);
    a.handle_action(Action::Digit(0), 1100);
    a.handle_action(Action::Digit(1), 1200); // "201" → Sport 1 (g2)
    a.handle_action(Action::Select, 1300);

    assert_eq!(a.player.channel_id.as_deref(), Some("3"));
    assert_eq!(
        a.live.category,
        group("g2"),
        "live group follows the playing channel"
    );
    // Stepping uses the playing channel's group, not the old one.
    a.handle_action(Action::ChUp, 0); // Sport 1 → Sport 2
    assert_eq!(a.player.channel_id.as_deref(), Some("5"));
}

#[test]
fn engine_load_failure_surfaces_error_and_stays_in_live() {
    struct FailingEngine;
    impl PlayerEngine for FailingEngine {
        fn load_at(&mut self, _url: &str, _start: Option<f64>) -> Result<(), PlayerError> {
            Err(PlayerError("boom".into()))
        }
        fn stop(&mut self) -> Result<(), PlayerError> {
            Ok(())
        }
        fn set_volume(&mut self, _volume: u8) -> Result<(), PlayerError> {
            Ok(())
        }
        fn pause(&mut self, _paused: bool) -> Result<(), PlayerError> {
            Ok(())
        }
        fn is_paused(&self) -> bool {
            false
        }
    }

    let mut a = app("failload");
    a.engine = Some(Box::new(FailingEngine));
    a.handle_action(Action::Select, 100);

    assert_eq!(
        a.router.current(),
        View::Live,
        "failed load must not enter Player"
    );
    assert!(a.player.channel_id.is_none());
    match a.player.osd().kind {
        OsdKind::Zap(ref text) => assert!(text.contains("boom"), "OSD shows the error"),
        ref other => panic!("expected Zap OSD, got {other:?}"),
    }
}

#[test]
fn epg_via_injected_fetcher_populates_now_next() {
    let body = r#"{"epg_listings":[{"title":"TmV3cyBOb3c=","description":"RGVzY3JpcHRpb24=","start":"2026-01-01 10:00:00","end":"2026-01-01 11:00:00"}]}"#;
    let mut a = app("epginject");
    a.engine = Some(Box::new(MockEngine::default()));
    a.fetcher = Arc::new(MockFetcher {
        body: Some(body.into()),
    });
    a.handle_action(Action::Select, 0); // play News 1 → EPG worker spawns

    for _ in 0..200 {
        a.poll_jobs(0);
        if !a.epg.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(a.epg.len(), 1);
    assert_eq!(a.epg[0].title, "News Now");
}

#[test]
fn disconnected_connect_worker_unblocks_setup() {
    let dir = std::env::temp_dir().join(format!("skjalf-apptest-disc-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let mut a = main_app_at(&dir); // no playlist → Setup view
    assert_eq!(a.router.current(), View::Setup);

    let (tx, rx) = std::sync::mpsc::channel();
    drop(tx); // worker dies without a verdict
    a.connect_rx = Some(rx);
    a.setup.message = Some("Connecting\u{2026}".into());

    a.handle_action(Action::Down, 0); // busy — input ignored
    assert!(a.connect_rx.is_some());
    a.poll_jobs(0);
    assert!(
        a.connect_rx.is_none(),
        "disconnected worker clears the slot"
    );
    assert_eq!(a.setup.message.as_deref(), Some("Connection failed."));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn player_volume_adjusts_and_persists() {
    let dir = seeded_dir("volume");
    let mut a = app_at(&dir);
    let engine = MockEngine::default();
    a.engine = Some(Box::new(engine.clone()));
    a.handle_action(Action::Select, 0);
    a.handle_action(Action::Right, 0); // +5
    a.handle_action(Action::Right, 0);
    a.handle_action(Action::Left, 0); // −5 → 85
    assert_eq!(a.settings.volume, 85);

    let reloaded = Store::open(&dir).unwrap().load_settings().unwrap();
    assert_eq!(reloaded.volume, 85);
    assert!(engine.volumes.lock().unwrap().contains(&85));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn volume_engine_failure_is_visible() {
    struct VolumeFailEngine;
    impl PlayerEngine for VolumeFailEngine {
        fn load_at(&mut self, _: &str, _: Option<f64>) -> Result<(), PlayerError> {
            Ok(())
        }
        fn stop(&mut self) -> Result<(), PlayerError> {
            Ok(())
        }
        fn set_volume(&mut self, _: u8) -> Result<(), PlayerError> {
            Err(PlayerError("volume device error".into()))
        }
        fn pause(&mut self, _: bool) -> Result<(), PlayerError> {
            Ok(())
        }
        fn is_paused(&self) -> bool {
            false
        }
    }
    let mut a = app("volume_failure");
    a.engine = Some(Box::new(VolumeFailEngine));
    a.router.push(View::Player);
    a.handle_action(Action::Right, 0);
    assert!(a
        .notice
        .as_deref()
        .unwrap_or_default()
        .contains("volume device error"));
}

#[test]
fn favorite_toggle_shows_in_live_and_persists() {
    let dir = seeded_dir("fav");
    let mut a = app_at(&dir);
    a.handle_action(Action::Select, 0); // play News 1
    a.handle_action(Action::Favorite, 0); // add
    assert!(a.favorites.contains(a.source_id().unwrap(), "1"));
    a.handle_action(Action::Favorite, 0); // remove
    assert!(!a.favorites.contains(a.source_id().unwrap(), "1"));
    a.handle_action(Action::Favorite, 0); // add again for persistence check
    let favs = Store::open(&dir).unwrap().load_favorites().unwrap();
    assert!(favs.contains(a.source_id().unwrap(), "1"));

    // Favorites pseudo-group lists it.
    a.set_category(BrowserId::Live, CategorySel::Favorites);
    let ids = a.group_channel_ids();
    assert_eq!(ids, vec!["1".to_string()]);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn favorite_indices_keep_saved_order_and_skip_missing_channels() {
    let mut a = app("favorite_indices");
    let sid = a.source_id().unwrap().to_owned();
    a.favorites.toggle(&sid, "missing");
    a.set_category(BrowserId::Live, CategorySel::All);
    a.handle_action(Action::Favorite, 0); // "1"
    press(&mut a, &[Action::Down; 4]);
    a.handle_action(Action::Favorite, 0); // "5"
    a.set_category(BrowserId::Live, CategorySel::Favorites);
    let playlist = a.playlist.as_ref().unwrap();
    let ids: Vec<_> = a
        .group_channel_indices()
        .iter()
        .map(|&i| playlist.channels[i].id.as_str())
        .collect();
    assert_eq!(ids, ["1", "5"]);
    assert_eq!(a.group_channel_ids(), ["1", "5"]);
}

#[test]
fn favorite_in_live_targets_the_highlighted_channel() {
    let mut a = app("favhome");
    // Nothing played yet: Favorite targets the Live highlight (News 1).
    a.handle_action(Action::Favorite, 0);
    assert!(a.favorites.contains(a.source_id().unwrap(), "1"));
    assert!(a.player.channel_id.is_none());

    a.handle_action(Action::Left, 0); // group list
    a.handle_action(Action::Down, 0); // → Sports
    a.handle_action(Action::Right, 0); // channel list
    a.handle_action(Action::Down, 0); // highlight Sport 2 (id "5")
    a.handle_action(Action::Favorite, 0);
    assert!(a.favorites.contains(a.source_id().unwrap(), "5"));
    assert!(!a.favorites.contains(a.source_id().unwrap(), "3"));
}

#[test]
fn back_from_player_stops_engine_and_returns_to_live_then_main() {
    let mut a = app("back");
    let engine = MockEngine::default();
    a.engine = Some(Box::new(engine.clone()));
    a.handle_action(Action::Select, 0);
    a.handle_action(Action::Back, 0);
    assert_eq!(a.router.current(), View::Live);
    assert_eq!(*engine.stopped.lock().unwrap(), 1);
    a.handle_action(Action::Back, 0);
    assert_eq!(a.router.current(), View::Main);
    a.handle_action(Action::Back, 0);
    assert_eq!(a.router.current(), View::Main);
    assert!(!a.wants_exit());
}

#[test]
fn quit_action_requests_exit_from_any_view() {
    let mut a = app("quit");
    a.handle_action(Action::Quit, 0); // from home
    assert!(a.wants_exit());
    let mut b = app("quit2");
    b.handle_action(Action::Select, 0); // enter player
    b.handle_action(Action::Quit, 0); // from player
    assert!(b.wants_exit());
}

#[test]
fn larger_game_mode_displays_render_more_channel_rows() {
    let mut counts = Vec::new();
    for (width, height) in [(1280.0, 800.0), (1920.0, 1080.0), (3840.0, 2160.0)] {
        let mut a = app("display_density");
        a.notice = None;
        let mut playlist = a.playlist.clone().unwrap();
        playlist.channels = (0..100)
            .map(|n| Channel {
                id: n.to_string(),
                name: format!("DENSITY_ROW_{n:03}"),
                num: Some(n),
                logo: None,
                group_id: "g1".into(),
                url: "/tmp/no-video".into(),
            })
            .collect();
        a.set_playlist(playlist);
        a.set_category(BrowserId::Live, group("g1"));
        let ctx = egui::Context::default();
        skjalf::theme::apply(&ctx);
        for frame in 0..6 {
            let ppp = ctx.zoom_factor() * 2.0;
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width / ppp, height / ppp),
                )),
                time: Some(frame as f64 * 0.1),
                ..Default::default()
            };
            input
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .native_pixels_per_point = Some(2.0);
            let output = ctx.run(input, |ctx| {
                skjalf::theme::adapt_display(ctx, true);
                skjalf::views::live::show(ctx, &mut a);
            });
            if frame == 5 {
                let mut labels = Vec::new();
                for shape in &output.shapes {
                    shape_texts(&shape.shape, &mut labels);
                }
                assert!(!labels.iter().any(|text| text.contains("D-pad")));
                counts.push(
                    labels
                        .iter()
                        .filter(|text| text.contains("DENSITY_ROW_"))
                        .count(),
                );
            }
        }
    }
    println!("Rendered channel rows at handheld, 1080p, 4K: {counts:?}");
    assert!(counts[0] >= 10, "handheld rows: {counts:?}");
    assert!(
        counts[0] < counts[1] && counts[1] < counts[2],
        "visible rows: {counts:?}"
    );
}

fn press(a: &mut App, actions: &[Action]) {
    for action in actions {
        a.handle_action(*action, 0);
    }
}

#[test]
fn main_buttons_open_destinations_and_back_returns_to_main() {
    for (moves, destination) in [
        (vec![], View::Live),
        (vec![Action::Right], View::Movies),
        (vec![Action::Right, Action::Right], View::Series),
        (
            vec![Action::Right, Action::Right, Action::Right],
            View::Settings,
        ),
    ] {
        let mut a = main_app("main_routes");
        press(&mut a, &moves);
        a.handle_action(Action::Select, 0);
        assert_eq!(a.router.current(), destination);
        a.handle_action(Action::Back, 0);
        assert_eq!(a.router.current(), View::Main);
        assert_eq!(
            a.main_focus,
            MainButton::ALL[moves.len()],
            "Main keeps its selection after {destination:?}"
        );
        a.handle_action(Action::Back, 0);
        assert_eq!(a.router.current(), View::Main, "Back never leaves Main");
        assert!(!a.wants_exit());
    }
}

#[test]
fn main_exit_button_requests_exit() {
    let mut a = main_app("main_exit");
    press(&mut a, &[Action::Right; 4]);
    assert_eq!(a.main_focus, MainButton::Exit);
    press(&mut a, &[Action::Right]);
    assert_eq!(a.main_focus, MainButton::Exit);
    assert!(!a.wants_exit());
    a.handle_action(Action::Select, 0);
    assert!(a.wants_exit());
}

#[test]
fn main_up_down_follow_rendered_rows() {
    let mut a = main_app("main_geometry");
    press(&mut a, &[Action::Down, Action::Up]);
    assert_eq!(
        a.main_focus,
        MainButton::LiveTv,
        "single row: no vertical moves"
    );

    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 400.0),
            )),
            ..Default::default()
        },
        |ctx| skjalf::views::main_menu::show(ctx, &mut a),
    );
    assert_eq!(a.main_columns, 4, "compact screen wraps onto two rows");
    a.handle_action(Action::Down, 0);
    assert_eq!(a.main_focus, MainButton::Exit);
    a.handle_action(Action::Up, 0);
    assert_eq!(a.main_focus, MainButton::LiveTv);
    press(&mut a, &[Action::Right, Action::Down]);
    assert_eq!(a.main_focus, MainButton::Movies, "no button below Movies");
}

#[test]
fn main_buttons_widen_with_the_viewport() {
    let width_of = |logical_width: f32| {
        let mut a = main_app("main_width");
        let ctx = egui::Context::default();
        skjalf::theme::apply(&ctx);
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(logical_width, 800.0),
                )),
                ..Default::default()
            },
            |ctx| skjalf::views::main_menu::show(ctx, &mut a),
        );
        let mut labels = Vec::new();
        for shape in &output.shapes {
            shape_texts(&shape.shape, &mut labels);
        }
        assert!(labels.iter().any(|text| text == "Live TV"));
        (a.main_columns, output.shapes.len())
    };
    assert_eq!(width_of(1024.0).0, 5);
    assert_eq!(width_of(1920.0).0, 5);
    assert!(width_of(400.0).0 < 5);
}

#[test]
fn live_selection_survives_main_round_trip() {
    let mut a = app("live_round_trip");
    press(&mut a, &[Action::Down, Action::Down]);
    assert_eq!(a.live.selected, 2);
    a.handle_action(Action::Back, 0);
    assert_eq!(a.router.current(), View::Main);
    a.handle_action(Action::Select, 0);
    assert_eq!(a.router.current(), View::Live);
    assert_eq!((a.live.category.clone(), a.live.selected), (group("g1"), 2));
}

#[test]
fn live_back_returns_focus_to_channels_before_leaving() {
    let mut a = app("live_back_focus");
    a.handle_action(Action::Left, 0);
    a.handle_action(Action::Back, 0);
    assert_eq!(a.router.current(), View::Live);
    a.handle_action(Action::Back, 0);
    assert_eq!(a.router.current(), View::Main);
}

#[test]
fn settings_buttons_move_with_up_down_and_back_returns_to_main() {
    let mut a = settings_app("settings_nav");
    assert_eq!(a.settings_focus, SettingsFocus::EditSource);
    a.handle_action(Action::Down, 0);
    assert_eq!(a.settings_focus, SettingsFocus::RefreshContent);
    a.handle_action(Action::Up, 0);
    assert_eq!(a.settings_focus, SettingsFocus::EditSource);
    a.handle_action(Action::Back, 0);
    assert_eq!(a.router.current(), View::Main);
    assert_eq!(a.main_focus, MainButton::Settings);
}

#[test]
fn settings_status_hides_credentials_and_reports_counts() {
    let mut a = settings_app("settings_status");
    let lines = a.settings_status();
    assert_eq!(lines[0], "Source: Xtream account");
    assert_eq!(lines[1], "Channels: 5  Last refresh: never");
    assert!(lines[2].starts_with("Movies: 0"), "{lines:?}");
    let (_tx, rx) = mpsc::channel();
    a.connect_rx = Some(rx);
    assert!(a
        .settings_status()
        .iter()
        .any(|line| line == "Refreshing\u{2026}"));
    a.connect_rx = None;
    a.refresh_status = Some("Last refresh: 5 channels.".into());
    let text = a.settings_status().join("\n");
    for secret in ["tv.example.com", "http", "\"u\"", "p@"] {
        assert!(!text.contains(secret), "{text}");
    }
}

#[test]
fn refresh_in_settings_keeps_the_user_in_settings_and_restores_live_selection() {
    let mut a = app("refresh_restore");
    press(&mut a, &[Action::Down, Action::Down]); // News 3 (id "4")
    a.handle_action(Action::Back, 0);
    a.activate_main_button(MainButton::Settings);
    let mut refreshed = playlist();
    refreshed.channels.insert(
        0,
        Channel {
            id: "0".into(),
            name: "News 0".into(),
            num: Some(100),
            logo: None,
            group_id: "g1".into(),
            url: "http://tv.example.com/live/{user}/{pass}/0.ts".into(),
        },
    );
    let (tx, rx) = mpsc::channel();
    a.connect_rx = Some(rx);
    tx.send(skjalf::app::ConnectOutcome::Ready(refreshed))
        .unwrap();
    a.poll_jobs(0);
    assert_eq!(a.router.current(), View::Settings);
    assert_eq!(a.group_channel_ids()[a.live.selected], "4");
}

#[test]
fn refresh_restores_nearest_index_when_the_channel_is_gone() {
    let mut a = app("refresh_nearest");
    press(&mut a, &[Action::Down, Action::Down]); // id "4", last of News
    let mut refreshed = playlist();
    refreshed.channels.retain(|channel| channel.id != "4");
    let (tx, rx) = mpsc::channel();
    a.connect_rx = Some(rx);
    tx.send(skjalf::app::ConnectOutcome::Ready(refreshed))
        .unwrap();
    a.poll_jobs(0);
    assert_eq!(a.router.current(), View::Live);
    assert_eq!((a.live.category.clone(), a.live.selected), (group("g1"), 1));
}

#[test]
fn successful_first_connection_resets_to_main() {
    let dir = std::env::temp_dir().join(format!("skjalf-apptest-first-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let mut a = main_app_at(&dir);
    assert_eq!(a.router.current(), View::Setup);
    a.handle_action(Action::Back, 0);
    assert_eq!(
        a.router.current(),
        View::Setup,
        "Back on first-run Setup stays"
    );
    let (tx, rx) = mpsc::channel();
    a.connect_rx = Some(rx);
    tx.send(skjalf::app::ConnectOutcome::Ready(playlist()))
        .unwrap();
    a.poll_jobs(0);
    assert_eq!(a.router.current(), View::Main);
    a.handle_action(Action::Back, 0);
    assert_eq!(a.router.current(), View::Main);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn successful_source_edit_returns_to_main_with_the_new_source() {
    let mut a = settings_app("edit_success");
    a.activate_settings_button(SettingsFocus::EditSource);
    assert_eq!(a.router.current(), View::Setup);
    let (tx, rx) = mpsc::channel();
    a.connect_rx = Some(rx);
    let mut replacement = playlist();
    replacement.source.id = "xtream:http://other.example.com/u".into();
    tx.send(skjalf::app::ConnectOutcome::Ready(replacement))
        .unwrap();
    a.poll_jobs(0);
    assert_eq!(a.router.current(), View::Main);
    assert_eq!(
        a.settings.source.as_ref().map(|s| s.id.as_str()),
        Some("xtream:http://other.example.com/u")
    );
    a.handle_action(Action::Back, 0);
    assert_eq!(a.router.current(), View::Main);
}

#[test]
fn playing_again_inside_player_does_not_stack_a_second_player() {
    let mut a = app("single_player");
    a.engine = Some(Box::new(MockEngine::default()));
    a.handle_action(Action::Select, 0);
    a.handle_action(Action::ChUp, 0);
    a.handle_action(Action::ChUp, 0);
    assert_eq!(a.router.current(), View::Player);
    a.handle_action(Action::Back, 0);
    assert_eq!(a.router.current(), View::Live);
}

fn key_event(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }
}

fn compact_input(events: Vec<egui::Event>) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(640.0, 400.0),
        )),
        events,
        ..Default::default()
    }
}

/// Mirrors the per-frame routing in `lib.rs`: raw events pass through
/// `pump_event`, unconsumed ones reach the egui widgets.
fn live_frame(ctx: &egui::Context, a: &mut App, events: Vec<egui::Event>) -> egui::FullOutput {
    let remaining: Vec<_> = events
        .into_iter()
        .filter(|event| !a.pump_event(event, 0))
        .collect();
    if a.text_focus_release {
        ctx.memory_mut(|memory| {
            if let Some(id) = memory.focused() {
                memory.surrender_focus(id);
            }
        });
        a.text_focus_release = false;
    }
    a.live_refresh();
    ctx.run(compact_input(remaining), |ctx| {
        skjalf::views::live::show(ctx, a)
    })
}

fn start_search(a: &mut App, ctx: &egui::Context) {
    a.live.focus = BrowserFocus::Search;
    a.handle_action(Action::Select, 0);
    assert!(a.search_editing());
    live_frame(ctx, a, vec![]);
}

fn type_text(a: &mut App, ctx: &egui::Context, text: &str) {
    live_frame(ctx, a, vec![egui::Event::Text(text.into())]);
}

fn live_names(a: &App) -> Vec<String> {
    let playlist = a.playlist.as_ref().unwrap();
    a.group_channel_indices()
        .iter()
        .map(|&i| playlist.channels[i].name.clone())
        .collect()
}

#[test]
fn typing_a_query_through_raw_events_narrows_the_list() {
    let mut a = app("search_typing");
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    a.set_category(BrowserId::Live, CategorySel::All);
    assert_eq!(a.group_channel_ids().len(), 5);
    start_search(&mut a, &ctx);
    type_text(&mut a, &ctx, "SPO");
    assert_eq!(a.live.query, "SPO");
    assert_eq!(live_names(&a), ["Sport 1", "Sport 2"]);
    type_text(&mut a, &ctx, "rt 2");
    assert_eq!(live_names(&a), ["Sport 2"]);
}

#[test]
fn shortcut_letters_and_digits_type_into_the_search_field() {
    let mut a = app("search_shortcuts");
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    a.set_category(BrowserId::Live, CategorySel::All);
    a.engine = Some(Box::new(MockEngine::default()));
    start_search(&mut a, &ctx);
    for text in ["f", "e", "q", "1", "0"] {
        type_text(&mut a, &ctx, text);
    }
    assert_eq!(a.live.query, "feq10");
    assert!(!a.wants_exit());
    assert!(a.zap.digits().is_empty());
    assert_eq!(a.favorites.iter().count(), 0);
    assert_eq!(a.router.current(), View::Live);
}

#[test]
fn enter_and_escape_end_search_editing_and_focus_the_results() {
    for key in [egui::Key::Enter, egui::Key::Escape, egui::Key::BrowserBack] {
        let mut a = app("search_end");
        let ctx = egui::Context::default();
        skjalf::theme::apply(&ctx);
        a.set_category(BrowserId::Live, CategorySel::All);
        start_search(&mut a, &ctx);
        type_text(&mut a, &ctx, "news");
        live_frame(&ctx, &mut a, vec![key_event(key)]);
        assert!(!a.search_editing(), "{key:?}");
        assert_eq!(a.live.focus, BrowserFocus::Items);
        assert_eq!(a.live.query, "news");
        assert_eq!(a.router.current(), View::Live);
        assert_eq!(ctx.memory(|m| m.focused()), None, "egui focus released");
    }
}

#[test]
fn back_action_ends_search_editing_without_leaving_live() {
    let mut a = app("search_back");
    a.set_category(BrowserId::Live, CategorySel::All);
    a.live.focus = BrowserFocus::Search;
    a.handle_action(Action::Select, 0);
    a.handle_action(Action::Down, 0);
    a.handle_action(Action::Digit(4), 0);
    assert!(a.search_editing(), "actions are ignored while editing");
    a.handle_action(Action::Back, 0);
    assert!(!a.search_editing());
    assert_eq!(a.router.current(), View::Live);
}

#[test]
fn native_keyboard_dismissal_ends_search_without_an_extra_back() {
    struct Recorder {
        requests: Arc<Mutex<Vec<&'static str>>>,
        dismissal: Mutex<Option<mpsc::Sender<()>>>,
    }
    impl KeyboardLauncher for Recorder {
        fn send(&self, uri: &'static str) -> mpsc::Receiver<Result<(), String>> {
            self.requests.lock().unwrap().push(uri);
            let (tx, rx) = mpsc::channel();
            tx.send(Ok(())).unwrap();
            rx
        }
        fn watch_dismissal(&self, _cancel: Arc<AtomicBool>) -> Option<mpsc::Receiver<()>> {
            let (tx, rx) = mpsc::channel();
            *self.dismissal.lock().unwrap() = Some(tx);
            Some(rx)
        }
    }
    let mut a = app("search_native_dismissal");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::new(Recorder {
        requests: requests.clone(),
        dismissal: Mutex::new(None),
    });
    a.keyboard_launcher = recorder.clone();
    a.game_mode = true;
    a.set_category(BrowserId::Live, CategorySel::All);
    a.live.focus = BrowserFocus::Search;
    a.handle_action(Action::Select, 0);
    a.poll_jobs(0);
    assert!(a.steam_keyboard_active());
    recorder
        .dismissal
        .lock()
        .unwrap()
        .take()
        .unwrap()
        .send(())
        .unwrap();
    a.poll_keyboard();
    assert!(!a.search_editing());
    assert!(!a.steam_keyboard_active());
    assert_eq!(a.live.focus, BrowserFocus::Items);
    a.handle_action(Action::Back, 1);
    assert_eq!(a.router.current(), View::Main, "one Back leaves Live");
    assert_eq!(*requests.lock().unwrap(), [OPEN_URI]);
}

#[test]
fn clear_restores_the_category_list_and_selection() {
    let mut a = app("search_clear");
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    a.set_category(BrowserId::Live, CategorySel::All);
    press(&mut a, &[Action::Down, Action::Down, Action::Down]); // Sport 1
    start_search(&mut a, &ctx);
    type_text(&mut a, &ctx, "sport");
    assert_eq!(a.live.filtered.len(), 2);
    live_frame(&ctx, &mut a, vec![key_event(egui::Key::Enter)]);
    a.live.focus = BrowserFocus::Search;
    press(&mut a, &[Action::Right, Action::Select]);
    assert_eq!(a.live.focus, BrowserFocus::Clear);
    assert!(a.live.query.is_empty());
    assert_eq!(a.live.filtered.len(), 5);
    assert_eq!(a.group_channel_ids()[a.live.selected], "3");
}

#[test]
fn empty_results_show_a_message_and_keep_controls_reachable() {
    let mut a = app("search_empty");
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    a.set_category(BrowserId::Live, CategorySel::All);
    start_search(&mut a, &ctx);
    type_text(&mut a, &ctx, "zzz");
    let output = live_frame(&ctx, &mut a, vec![]);
    let mut labels = Vec::new();
    for shape in &output.shapes {
        shape_texts(&shape.shape, &mut labels);
    }
    assert!(
        labels.iter().any(|text| text == "No matches."),
        "{labels:?}"
    );
    live_frame(&ctx, &mut a, vec![key_event(egui::Key::Enter)]);
    assert_eq!(a.live.focus, BrowserFocus::Search, "no results to focus");
    a.handle_action(Action::Right, 0);
    assert_eq!(a.live.focus, BrowserFocus::Clear);
    a.handle_action(Action::Down, 0);
    assert_eq!(a.live.focus, BrowserFocus::Items);
    a.handle_action(Action::Up, 0);
    a.handle_action(Action::Left, 0);
    assert_eq!(a.live.focus, BrowserFocus::Categories);
    a.handle_action(Action::Down, 0);
    assert_eq!(a.live.category, group("g1"), "categories still change");
    assert!(a.group_channel_ids().is_empty());
}

#[test]
fn live_focus_order_follows_the_search_layout() {
    let mut a = app("search_focus_order");
    a.set_category(BrowserId::Live, CategorySel::All);
    assert_eq!(a.live.focus, BrowserFocus::Items);
    a.handle_action(Action::Up, 0);
    assert_eq!(a.live.focus, BrowserFocus::Search, "Up from first item");
    a.handle_action(Action::Right, 0);
    assert_eq!(a.live.focus, BrowserFocus::Clear);
    a.handle_action(Action::Down, 0);
    assert_eq!(a.live.focus, BrowserFocus::Items);
    a.handle_action(Action::Left, 0);
    assert_eq!(a.live.focus, BrowserFocus::Categories);
    a.handle_action(Action::Right, 0);
    assert_eq!(a.live.focus, BrowserFocus::Items);
    a.live.focus = BrowserFocus::Search;
    a.handle_action(Action::Left, 0);
    assert_eq!(a.live.focus, BrowserFocus::Categories);
    a.live.focus = BrowserFocus::Search;
    a.handle_action(Action::Down, 0);
    assert_eq!(a.live.focus, BrowserFocus::Items);
    a.live.focus = BrowserFocus::Clear;
    a.handle_action(Action::Left, 0);
    assert_eq!(a.live.focus, BrowserFocus::Search);
}

#[test]
fn categories_list_favorites_all_then_groups() {
    let a = app("categories");
    assert_eq!(a.categories_len(BrowserId::Live), 4);
    let labels: Vec<_> = (0..4)
        .map(|row| a.category_label(BrowserId::Live, row).unwrap())
        .collect();
    assert_eq!(labels[1..], ["All", "News", "Sports"]);
    assert!(labels[0].contains("Favorites"));
    assert_eq!(a.category_at(BrowserId::Live, 1), Some(CategorySel::All));
    assert_eq!(a.category_at(BrowserId::Live, 3), Some(group("g2")));
}

#[test]
fn all_searches_every_group_and_group_search_stays_inside_it() {
    let mut a = app("search_all");
    a.set_category(BrowserId::Live, CategorySel::All);
    a.set_query(BrowserId::Live, "1".into());
    assert_eq!(live_names(&a), ["News 1", "Sport 1"]);
    a.set_category(BrowserId::Live, group("g1"));
    assert_eq!(live_names(&a), ["News 1"]);
}

#[test]
fn search_combines_with_favorites() {
    let mut a = app("search_favorites");
    a.set_category(BrowserId::Live, CategorySel::All);
    a.handle_action(Action::Down, 0);
    a.handle_action(Action::Favorite, 0); // "2"
    press(&mut a, &[Action::Down, Action::Down]);
    a.handle_action(Action::Favorite, 0); // "3"
    a.set_category(BrowserId::Live, CategorySel::Favorites);
    assert_eq!(live_names(&a), ["News 2", "Sport 1"]);
    a.set_query(BrowserId::Live, "news".into());
    assert_eq!(live_names(&a), ["News 2"]);
    a.handle_action(Action::Favorite, 0);
    assert!(a.group_channel_ids().is_empty(), "removed from favorites");
}

#[test]
fn query_and_selection_survive_player_and_main_round_trips() {
    let mut a = app("search_round_trip");
    a.engine = Some(Box::new(MockEngine::default()));
    a.set_category(BrowserId::Live, CategorySel::All);
    a.set_query(BrowserId::Live, "news".into());
    a.handle_action(Action::Down, 0);
    assert_eq!(a.group_channel_ids()[a.live.selected], "2");
    a.handle_action(Action::Select, 0);
    assert_eq!(a.router.current(), View::Player);
    a.handle_action(Action::Back, 0);
    assert_eq!(a.router.current(), View::Live);
    assert_eq!(a.live.query, "news");
    assert_eq!(a.group_channel_ids()[a.live.selected], "2");
    a.handle_action(Action::Back, 0);
    assert_eq!(a.router.current(), View::Main);
    a.handle_action(Action::Select, 0);
    assert_eq!(a.live.query, "news");
    assert_eq!(a.live.category, CategorySel::All);
    assert_eq!(a.group_channel_ids()[a.live.selected], "2");
}

#[test]
fn refresh_restores_the_selection_by_channel_id_under_a_query() {
    let mut a = app("search_refresh");
    a.set_category(BrowserId::Live, CategorySel::All);
    a.set_query(BrowserId::Live, "news".into());
    press(&mut a, &[Action::Down, Action::Down]);
    assert_eq!(a.group_channel_ids()[a.live.selected], "4");
    let mut refreshed = playlist();
    refreshed.channels.reverse();
    let (tx, rx) = mpsc::channel();
    a.connect_rx = Some(rx);
    tx.send(skjalf::app::ConnectOutcome::Ready(refreshed))
        .unwrap();
    a.poll_jobs(0);
    assert_eq!(a.live.query, "news");
    assert_eq!(a.group_channel_ids()[a.live.selected], "4");
    assert_eq!(a.live.selected, 0, "list order changed with the refresh");
}

#[test]
fn playing_a_hidden_channel_reveals_it_in_the_list() {
    let mut a = app("search_reveal");
    a.engine = Some(Box::new(MockEngine::default()));
    a.set_category(BrowserId::Live, CategorySel::All);
    a.set_query(BrowserId::Live, "news".into());
    a.handle_action(Action::Select, 0);
    a.handle_action(Action::Digit(2), 1000);
    a.handle_action(Action::Digit(0), 1100);
    a.handle_action(Action::Digit(1), 1200);
    a.handle_action(Action::Select, 1300); // Sport 1 is hidden by "news"
    assert_eq!(a.player.channel_id.as_deref(), Some("3"));
    assert!(a.live.query.is_empty());
    assert_eq!(a.group_channel_ids()[a.live.selected], "3");
    a.handle_action(Action::ChUp, 0);
    assert_eq!(
        a.player.channel_id.as_deref(),
        Some("5"),
        "All order: 1,2,4,3,5"
    );
}

#[test]
fn held_repeat_moves_only_inside_the_category_and_channel_lists() {
    let mut a = app("repeat_scope");
    a.set_category(BrowserId::Live, CategorySel::All);
    a.handle_repeat(Action::Down);
    assert_eq!(a.live.selected, 1);
    a.handle_repeat(Action::Right);
    a.handle_repeat(Action::Left);
    assert_eq!(
        a.live.focus,
        BrowserFocus::Items,
        "horizontal repeats are ignored"
    );
    a.handle_repeat(Action::Up);
    a.handle_repeat(Action::Up);
    assert_eq!(a.live.selected, 0);
    assert_eq!(
        a.live.focus,
        BrowserFocus::Items,
        "a repeat never leaves the list for Search"
    );
    a.handle_action(Action::Up, 0);
    assert_eq!(a.live.focus, BrowserFocus::Search);
    a.handle_repeat(Action::Down);
    assert_eq!(a.live.focus, BrowserFocus::Search);

    a.live.focus = BrowserFocus::Categories;
    a.handle_repeat(Action::Down);
    assert_eq!(a.live.category, group("g1"));
    a.handle_repeat(Action::Down);
    assert_eq!(a.live.category, group("g2"));
    assert_eq!(a.live.focus, BrowserFocus::Categories);
}

#[test]
fn held_repeat_is_ignored_outside_live_and_while_editing() {
    let mut a = main_app("repeat_main");
    a.handle_repeat(Action::Down);
    assert_eq!(a.main_focus, MainButton::LiveTv);
    let mut b = app("repeat_editing");
    b.live.focus = BrowserFocus::Search;
    b.handle_action(Action::Select, 0);
    b.handle_repeat(Action::Down);
    assert_eq!(b.live.focus, BrowserFocus::Search);
}

#[test]
fn startup_lands_on_favorites_when_the_source_has_some() {
    let dir = seeded_dir("startup_favorites");
    let store = Store::open(&dir).unwrap();
    let mut favorites = skjalf_core::model::Favorites::default();
    favorites.toggle(&source().id, "3");
    store.save_favorites(&favorites).unwrap();
    let a = main_app_at(&dir);
    assert_eq!(a.live.category, CategorySel::Favorites);
    assert_eq!(a.group_channel_ids(), ["3"]);
}

#[test]
fn search_field_stays_above_the_steam_keyboard_at_deck_scale() {
    let mut a = app("search_compact");
    a.game_mode = true;
    a.set_category(BrowserId::Live, CategorySel::All);
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    start_search(&mut a, &ctx);
    live_frame(&ctx, &mut a, vec![]);
    let rect = ctx
        .read_response(skjalf::views::live::search_field_id())
        .unwrap()
        .rect;
    assert!(rect.max.y < 200.0, "search field at {rect:?}");
    assert!(rect.width() > 200.0, "search field too narrow: {rect:?}");
    assert_eq!(
        ctx.memory(|m| m.focused()),
        Some(skjalf::views::live::search_field_id())
    );
}

#[test]
fn search_on_a_54000_channel_playlist_returns_the_matching_channels() {
    let mut a = app("search_large");
    let mut large = playlist();
    large.groups = (0..900)
        .map(|n| Group {
            id: format!("g{n}"),
            name: format!("Group {n}"),
        })
        .collect();
    large.channels = (0..54_000)
        .map(|n| Channel {
            id: n.to_string(),
            name: format!(
                "Channel {n:05} {}",
                if n % 97 == 0 { "Prime" } else { "Plain" }
            ),
            num: Some(n),
            logo: None,
            group_id: format!("g{}", n % 900),
            url: "/tmp/no-video".into(),
        })
        .collect();
    a.set_playlist(large);
    a.set_category(BrowserId::Live, CategorySel::All);
    assert_eq!(a.live.filtered.len(), 54_000);
    a.set_query(BrowserId::Live, " PRIME ".into());
    let expected: Vec<usize> = (0..54_000).filter(|n| n % 97 == 0).collect();
    assert_eq!(a.group_channel_indices(), expected);
    a.set_category(BrowserId::Live, group("g97"));
    assert_eq!(
        a.group_channel_indices(),
        (0..54_000usize)
            .filter(|n| n % 97 == 0 && n % 900 == 97)
            .collect::<Vec<_>>()
    );
    a.set_query(BrowserId::Live, "channel 12345".into());
    a.set_category(BrowserId::Live, CategorySel::All);
    assert_eq!(a.group_channel_ids(), ["12345"]);
}

#[test]
fn source_replacement_builds_favorites_for_the_new_source() {
    let mut a = settings_app("favorites_new_source");
    let old_id = source().id;
    a.favorites.toggle(&old_id, "1");
    a.favorites.toggle("xtream:http://other.example.com/u", "5");
    a.activate_settings_button(SettingsFocus::EditSource);
    let mut replacement = playlist();
    replacement.source.id = "xtream:http://other.example.com/u".into();
    let (tx, rx) = mpsc::channel();
    a.connect_rx = Some(rx);
    tx.send(skjalf::app::ConnectOutcome::Ready(replacement))
        .unwrap();
    a.poll_jobs(0);
    assert_eq!(a.live.category, CategorySel::Favorites);
    assert_eq!(
        a.group_channel_ids(),
        ["5"],
        "only the new source's favorites"
    );
}

#[test]
fn touch_playback_while_searching_ends_the_edit() {
    let mut a = app("touch_play_editing");
    a.engine = Some(Box::new(MockEngine::default()));
    a.live.focus = BrowserFocus::Search;
    a.handle_action(Action::Select, 0);
    assert!(a.search_editing());
    let channel = a.playlist.as_ref().unwrap().channels[0].clone();
    a.play(&channel, 0);
    assert_eq!(a.router.current(), View::Player);
    assert!(!a.search_editing());
    a.handle_action(Action::Back, 1);
    assert_eq!(a.router.current(), View::Live);
}

#[test]
fn pending_zap_digits_do_not_start_playback_after_leaving_live() {
    let mut a = app("zap_after_back");
    a.engine = Some(Box::new(MockEngine::default()));
    for (digit, at) in [(1, 1000), (0, 1100), (2, 1200)] {
        a.handle_action(Action::Digit(digit), at);
    }
    a.handle_action(Action::Back, 1300);
    assert_eq!(a.router.current(), View::Main);
    a.poll_jobs(20_000);
    assert_eq!(a.router.current(), View::Main);
    assert!(a.player.channel_id.is_none());
}

fn reversed_playlist_app(tag: &str) -> App {
    let mut a = app(tag);
    let mut reversed = playlist();
    reversed.channels.reverse();
    a.set_playlist(reversed);
    a.set_category(BrowserId::Live, CategorySel::All);
    a
}

#[test]
fn sort_button_toggles_name_order_and_keeps_the_selected_channel() {
    let mut a = reversed_playlist_app("sort_toggle");
    assert_eq!(a.group_channel_ids(), ["5", "3", "4", "2", "1"]);
    press(&mut a, &[Action::Down, Action::Down]);
    assert_eq!(a.group_channel_ids()[a.live.selected], "4");
    a.live.focus = BrowserFocus::Clear;
    press(&mut a, &[Action::Right, Action::Select]);
    assert_eq!(a.live.focus, BrowserFocus::Sort);
    assert_eq!(a.live.sort, SortMode::Name);
    assert_eq!(a.group_channel_ids(), ["1", "2", "4", "3", "5"]);
    assert_eq!(a.group_channel_ids()[a.live.selected], "4");
    a.handle_action(Action::Select, 0);
    assert_eq!(a.live.sort, SortMode::Provider);
    assert_eq!(a.group_channel_ids()[a.live.selected], "4");
    press(&mut a, &[Action::Left, Action::Left]);
    assert_eq!(a.live.focus, BrowserFocus::Search);
    a.live.focus = BrowserFocus::Sort;
    a.handle_action(Action::Down, 0);
    assert_eq!(a.live.focus, BrowserFocus::Items);
}

#[test]
fn name_sort_orders_search_results_and_groups() {
    let mut a = reversed_playlist_app("sort_search");
    a.toggle_sort(BrowserId::Live);
    a.set_query(BrowserId::Live, "s".into());
    assert_eq!(
        live_names(&a),
        ["News 1", "News 2", "News 3", "Sport 1", "Sport 2"]
    );
    a.set_query(BrowserId::Live, "sport".into());
    assert_eq!(live_names(&a), ["Sport 1", "Sport 2"]);
    a.set_query(BrowserId::Live, String::new());
    a.set_category(BrowserId::Live, group("g2"));
    assert_eq!(live_names(&a), ["Sport 1", "Sport 2"]);
}

#[test]
fn favorites_keep_their_order_in_name_mode() {
    let mut a = reversed_playlist_app("sort_favorites");
    a.toggle_sort(BrowserId::Live);
    a.handle_action(Action::Down, 0); // wraps to News 1 ("1")
    a.handle_action(Action::Favorite, 0);
    a.handle_action(Action::Down, 0);
    a.handle_action(Action::Favorite, 0); // News 2 ("2")
    a.set_category(BrowserId::Live, CategorySel::Favorites);
    assert_eq!(a.group_channel_ids(), ["1", "2"]);
}

#[test]
fn sort_mode_persists_across_restart() {
    let dir = seeded_dir("sort_persist");
    let mut a = app_at(&dir);
    a.toggle_sort(BrowserId::Live);
    drop(a);
    let reloaded = Store::open(&dir).unwrap().load_settings().unwrap();
    assert_eq!(reloaded.sort_modes.live, SortMode::Name);
    let b = main_app_at(&dir);
    assert_eq!(b.live.sort, SortMode::Name);
    let mut c = main_app_at(&dir);
    c.set_category(BrowserId::Live, CategorySel::All);
    assert_eq!(c.group_channel_ids().len(), 5);
}

#[test]
fn sort_mode_survives_a_source_change() {
    let mut a = app("sort_source_change");
    a.toggle_sort(BrowserId::Live);
    let mut replacement = playlist();
    replacement.source.id = "xtream:http://other.example.com/u".into();
    let (tx, rx) = mpsc::channel();
    a.connect_rx = Some(rx);
    tx.send(skjalf::app::ConnectOutcome::Ready(replacement))
        .unwrap();
    a.poll_jobs(0);
    assert_eq!(a.live.sort, SortMode::Name);
}

#[test]
fn name_sort_on_a_54000_channel_playlist_is_correct() {
    let mut a = app("sort_large");
    let mut large = playlist();
    large.channels = (0..54_000)
        .map(|n| Channel {
            id: n.to_string(),
            name: format!("Channel {:05}", (n * 7919) % 54_000),
            num: Some(n),
            logo: None,
            group_id: "g1".into(),
            url: "/tmp/no-video".into(),
        })
        .collect();
    a.set_playlist(large);
    a.set_category(BrowserId::Live, CategorySel::All);
    a.toggle_sort(BrowserId::Live);
    let names = live_names(&a);
    assert_eq!(names.len(), 54_000);
    assert!(names.windows(2).all(|pair| pair[0] <= pair[1]));
    assert_eq!(names[0], "Channel 00000");
    assert_eq!(names[53_999], "Channel 53999");
}

fn click_text(ctx: &egui::Context, a: &mut App, label: &str, show: fn(&egui::Context, &mut App)) {
    let input = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1024.0, 640.0),
        )),
        ..Default::default()
    };
    let output = ctx.run(input(), |ctx| show(ctx, a));
    let mut at = None;
    for clipped in &output.shapes {
        find_text_pos(&clipped.shape, label, &mut at);
    }
    let pos = at.unwrap_or_else(|| panic!("no {label} on screen")) + egui::vec2(6.0, 6.0);
    let mut press = input();
    press.events = vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        },
    ];
    let _ = ctx.run(press, |ctx| show(ctx, a));
    let mut release = input();
    release.events = vec![egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    }];
    let _ = ctx.run(release, |ctx| show(ctx, a));
}

fn find_text_pos(shape: &egui::Shape, label: &str, out: &mut Option<egui::Pos2>) {
    match shape {
        egui::Shape::Text(text) if text.galley.text() == label => *out = Some(text.pos),
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                find_text_pos(shape, label, out);
            }
        }
        _ => {}
    }
}

#[test]
fn touch_back_button_acts_like_b_in_settings_live_and_placeholders() {
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);

    let mut a = settings_app("touch_back_settings");
    click_text(&ctx, &mut a, "Back", skjalf::views::settings::show);
    assert_eq!(a.router.current(), View::Main);

    let mut b = app("touch_back_live");
    click_text(&ctx, &mut b, "Back", skjalf::views::live::show);
    assert_eq!(b.router.current(), View::Main);

    let mut c = main_app("touch_back_placeholder");
    press(&mut c, &[Action::Right, Action::Right, Action::Select]);
    click_text(&ctx, &mut c, "Back", skjalf::views::series::show);
    assert_eq!(c.router.current(), View::Main);
}

#[test]
fn touch_sort_button_toggles_the_mode() {
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    let mut a = reversed_playlist_app("touch_sort");
    click_text(&ctx, &mut a, "Sort: Provider", skjalf::views::live::show);
    assert_eq!(a.live.sort, SortMode::Name);
}

// -- Movies -------------------------------------------------------------------

const VOD_CATEGORIES: &str = r#"[{"category_id":"12","category_name":"Action","parent_id":0},{"category_id":"13","category_name":"Drama","parent_id":0}]"#;
const VOD_STREAMS: &str = r#"[
    {"name":"Movie A","stream_id":1001,"rating":"7.2","added":"100","category_id":"12","container_extension":"mkv"},
    {"name":"Movie B","stream_id":"1002","rating":"","added":300,"category_id":"13","container_extension":"mp4"},
    {"name":"Movie C","stream_id":1003,"category_id":"12"},
    {"name":"Other D","stream_id":1004,"added":"200","category_id":"13","container_extension":"avi"}
]"#;

const SERIES_CATEGORIES: &str = r#"[{"category_id":"30","category_name":"Drama"},{"category_id":"31","category_name":"Comedy"}]"#;
const SERIES_LIST: &str = r#"[
    {"name":"Show A","series_id":2001,"rating":"8","last_modified":"50","category_id":"30"},
    {"name":"Show B","series_id":"2002","last_modified":70,"category_id":"31"},
    {"name":"Other C","series_id":2003,"category_id":"30"}
]"#;

fn series_info_body(id: &str) -> String {
    format!(
        r#"{{"seasons":[{{"season_number":1,"name":"Opening"}}],"info":{{"plot":"Plot of {id}","genre":"Drama","rating":"8"}},"episodes":{{
            "10":[{{"id":"{id}10","episode_num":1,"title":"Ten","container_extension":"mkv","season":10}}],
            "2":[{{"id":"{id}21","episode_num":2,"title":"Two B"}},{{"id":"{id}20","episode_num":1,"title":"Two A","info":{{"duration_secs":1800}}}}],
            "1":[{{"id":"{id}11","episode_num":1,"title":"One"}}]}}}}"#
    )
}

#[derive(Default)]
struct VodFetcher {
    long_persian: AtomicBool,
    repeat_series_name: AtomicBool,
    persian_info: AtomicBool,
    long_text: AtomicBool,
    calls: Mutex<Vec<String>>,
    fail_catalog: AtomicBool,
    fail_info: AtomicBool,
    gates: Mutex<std::collections::HashMap<String, mpsc::Receiver<()>>>,
}

impl VodFetcher {
    fn actions(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn gate(&self, movie_id: &str) -> mpsc::Sender<()> {
        let (tx, rx) = mpsc::channel();
        self.gates.lock().unwrap().insert(movie_id.into(), rx);
        tx
    }
}

impl HttpFetcher for VodFetcher {
    fn get(&self, url: &str) -> Result<String, FetchError> {
        let action = url
            .split("action=")
            .nth(1)
            .and_then(|rest| rest.split('&').next())
            .unwrap_or("")
            .to_owned();
        self.calls.lock().unwrap().push(action.clone());
        match action.as_str() {
            "get_short_epg" => {
                let gate = self.gates.lock().unwrap().remove("epg");
                if let Some(gate) = gate {
                    gate.recv().ok();
                }
                Ok(r#"{"epg_listings":[{"title":"TGl2ZSBTaG93","description":"","start":"2026-01-01 10:00:00","end":"2026-01-01 11:00:00"}]}"#.into())
            }
            "get_series_categories" => Ok(SERIES_CATEGORIES.into()),
            "get_series" if self.fail_catalog.load(Ordering::Relaxed) => {
                Err(FetchError("Network connection failed.".into()))
            }
            "get_series" => Ok(SERIES_LIST.into()),
            "get_series_info" => {
                let id = url.split("series_id=").nth(1).unwrap_or("").to_owned();
                let gate = self.gates.lock().unwrap().remove(&id);
                if let Some(gate) = gate {
                    gate.recv().ok();
                }
                if self.fail_info.load(Ordering::Relaxed) {
                    return Err(FetchError("Server response timed out.".into()));
                }
                let mut body = series_info_body(&id);
                if self.repeat_series_name.load(Ordering::Relaxed) {
                    body = body.replace("\"One\"", "\"Show A - S01E01 - Center de Stage\"");
                }
                if self.persian_info.load(Ordering::Relaxed) {
                    body = body.replace("\"One\"", "\"دنیای دیگر\"");
                }
                if self.persian_info.load(Ordering::Relaxed) {
                    body = body
                        .replace(&format!("Plot of {id}"), "دنیای دیگر")
                        .replace("Opening", "دنیای دیگر");
                }
                Ok(if self.long_text.load(Ordering::Relaxed) {
                    body.replace(&format!("Plot of {id}"), &"long words ".repeat(800))
                } else {
                    body
                })
            }
            "get_vod_categories" => Ok(VOD_CATEGORIES.into()),
            "get_vod_streams" if self.fail_catalog.load(Ordering::Relaxed) => {
                Err(FetchError("Network connection failed.".into()))
            }
            "get_vod_streams" => Ok(VOD_STREAMS.into()),
            "get_vod_info" => {
                let id = url.split("vod_id=").nth(1).unwrap_or("").to_owned();
                let gate = self.gates.lock().unwrap().remove(&id);
                if let Some(gate) = gate {
                    gate.recv().ok();
                }
                if self.fail_info.load(Ordering::Relaxed) {
                    return Err(FetchError("Server response timed out.".into()));
                }
                let description = if self.long_persian.load(Ordering::Relaxed) {
                    LONG_PERSIAN.to_owned()
                } else if self.persian_info.load(Ordering::Relaxed) {
                    "دنیای دیگر".to_owned()
                } else if self.long_text.load(Ordering::Relaxed) {
                    "long words ".repeat(800)
                } else {
                    format!("Plot of {id}")
                };
                let cast = if self.long_persian.load(Ordering::Relaxed) {
                    "Actor".to_owned()
                } else {
                    description.clone()
                };
                Ok(format!(
                    r#"{{"info":{{"description":"{description}","cast":"{cast}","genre":"Genre {id}","duration_secs":"5400","rating":"7.5"}}}}"#
                ))
            }
            _ => Err(FetchError("mock: offline".into())),
        }
    }
}

fn wait_for(a: &mut App, condition: impl Fn(&App) -> bool) {
    for _ in 0..600 {
        a.poll_jobs(0);
        if condition(a) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("condition not reached");
}

fn movies_app(tag: &str) -> (App, Arc<VodFetcher>, Arc<MockEngine>) {
    let mut a = main_app(tag);
    let fetcher = Arc::new(VodFetcher::default());
    a.fetcher = fetcher.clone();
    let engine = Arc::new(MockEngine::default());
    a.engine = Some(Box::new((*engine).clone()));
    (a, fetcher, engine)
}

fn open_movies(a: &mut App) {
    a.activate_main_button(MainButton::Movies);
    wait_for(a, |a| a.movie_slot.loaded.is_some());
}

fn selected_movie_id(a: &App) -> String {
    let item = a.movies.selected_item().unwrap();
    a.movie_slot.loaded.as_ref().unwrap().data.movies[item]
        .id
        .clone()
}

fn all_movies(a: &mut App) {
    a.set_category(BrowserId::Movies, CategorySel::All);
}

#[test]
fn opening_movies_loads_the_catalog_and_shows_a_loading_message_first() {
    let (mut a, fetcher, _) = movies_app("movies_open");
    a.activate_main_button(MainButton::Movies);
    assert_eq!(a.router.current(), View::Movies);
    assert_eq!(
        a.browser_notice(BrowserId::Movies).as_deref(),
        Some("Loading movies...")
    );
    a.handle_action(Action::Back, 0);
    assert_eq!(a.router.current(), View::Main, "Back works while loading");
    open_movies(&mut a);
    assert_eq!(a.movie_slot.status, LoadStatus::Ready);
    assert_eq!(a.browser_notice(BrowserId::Movies), None);
    assert_eq!(
        fetcher
            .actions()
            .iter()
            .filter(|action| *action == "get_vod_streams")
            .count(),
        1,
        "re-entering does not start a second worker"
    );
}

#[test]
fn movies_browse_search_sort_and_label_rows() {
    let (mut a, _, _) = movies_app("movies_browse");
    open_movies(&mut a);
    all_movies(&mut a);
    assert_eq!(a.movies.filtered.len(), 4);
    let first = a
        .browser_row(BrowserId::Movies, a.movies.filtered[0])
        .unwrap();
    assert_eq!(first.label, "Movie A   7.2");
    let second = a
        .browser_row(BrowserId::Movies, a.movies.filtered[1])
        .unwrap();
    assert_eq!(second.label, "Movie B", "empty rating adds nothing");
    a.set_query(BrowserId::Movies, "movie".into());
    assert_eq!(a.movies.filtered.len(), 3);
    a.set_category(BrowserId::Movies, group_sel("12"));
    assert_eq!(a.movies.filtered.len(), 2);
    a.toggle_sort(BrowserId::Movies);
    assert_eq!(a.movies.sort, skjalf_core::browse::SortMode::Name);
    let saved = a.store.as_ref().unwrap().load_settings().unwrap();
    assert_eq!(saved.sort_modes.movies, skjalf_core::browse::SortMode::Name);
    assert_eq!(
        saved.sort_modes.live,
        skjalf_core::browse::SortMode::Provider
    );
}

fn group_sel(id: &str) -> CategorySel {
    CategorySel::Group(id.into())
}

#[test]
fn movie_flow_restores_selection_at_every_back_step() {
    let (mut a, _, engine) = movies_app("movies_flow");
    open_movies(&mut a);
    all_movies(&mut a);
    press(&mut a, &[Action::Down, Action::Down]);
    assert_eq!(selected_movie_id(&a), "1003");
    a.handle_action(Action::Select, 0);
    assert_eq!(a.router.current(), View::MovieDetails);
    assert_eq!(a.movie_details.as_ref().unwrap().movie.id, "1003");
    a.handle_action(Action::Select, 0); // Play
    assert_eq!(a.router.current(), View::Player);
    assert_eq!(
        engine.loads.lock().unwrap().last().unwrap(),
        "http://tv.example.com/movie/u/p/1003.mp4"
    );
    a.handle_action(Action::Back, 1);
    assert_eq!(a.router.current(), View::MovieDetails);
    assert_eq!(*engine.stopped.lock().unwrap(), 1);
    a.handle_action(Action::Back, 2);
    assert_eq!(a.router.current(), View::Movies);
    assert_eq!(selected_movie_id(&a), "1003");
    assert_eq!(a.movies.category, CategorySel::All);
    a.handle_action(Action::Back, 3);
    assert_eq!(a.router.current(), View::Main);
    assert_eq!(a.main_focus, MainButton::Movies);
    a.handle_action(Action::Select, 4);
    assert_eq!(selected_movie_id(&a), "1003");
}

#[test]
fn movie_details_show_catalog_data_then_provider_metadata() {
    let (mut a, fetcher, _) = movies_app("movie_details_pending");
    open_movies(&mut a);
    all_movies(&mut a);
    let gate = fetcher.gate("1001");
    a.handle_action(Action::Select, 0);
    let details = a.movie_details.as_ref().unwrap();
    assert_eq!(details.movie.name, "Movie A");
    assert_eq!(details.movie.rating.as_deref(), Some("7.2"));
    assert_eq!(details.info, DetailInfo::Loading);
    gate.send(()).unwrap();
    wait_for(&mut a, |a| {
        matches!(a.movie_details.as_ref().unwrap().info, DetailInfo::Ready(_))
    });
    let DetailInfo::Ready(info) = &a.movie_details.as_ref().unwrap().info else {
        unreachable!()
    };
    assert_eq!(info.description.as_deref(), Some("Plot of 1001"));
    assert_eq!(info.duration_secs, Some(5400));
}

#[test]
fn stale_movie_info_is_ignored_after_opening_another_movie() {
    let (mut a, fetcher, _) = movies_app("movie_info_stale");
    open_movies(&mut a);
    all_movies(&mut a);
    let gate = fetcher.gate("1001");
    a.handle_action(Action::Select, 0); // Movie A, blocked
    a.handle_action(Action::Back, 0);
    a.handle_action(Action::Down, 0);
    a.handle_action(Action::Select, 0); // Movie B
    wait_for(&mut a, |a| {
        matches!(a.movie_details.as_ref().unwrap().info, DetailInfo::Ready(_))
    });
    gate.send(()).unwrap();
    for _ in 0..40 {
        a.poll_jobs(0);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let details = a.movie_details.as_ref().unwrap();
    assert_eq!(details.movie.id, "1002");
    let DetailInfo::Ready(info) = &details.info else {
        panic!("info lost");
    };
    assert_eq!(info.description.as_deref(), Some("Plot of 1002"));
}

#[test]
fn failed_movie_info_shows_unavailable_and_still_plays() {
    let (mut a, fetcher, engine) = movies_app("movie_info_failure");
    fetcher.fail_info.store(true, Ordering::Relaxed);
    open_movies(&mut a);
    all_movies(&mut a);
    a.handle_action(Action::Select, 0);
    wait_for(&mut a, |a| {
        a.movie_details.as_ref().unwrap().info == DetailInfo::Unavailable
    });
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    let output = ctx.run(compact_input(vec![]), |ctx| {
        skjalf::views::movie_details::show(ctx, &mut a, 0)
    });
    let mut labels = Vec::new();
    for shape in &output.shapes {
        shape_texts(&shape.shape, &mut labels);
    }
    assert!(labels.iter().any(|text| text == "Details unavailable."));
    a.handle_action(Action::Select, 0);
    assert_eq!(a.router.current(), View::Player);
    assert_eq!(engine.loads.lock().unwrap().len(), 1);
}

#[test]
fn movie_url_encodes_credentials_at_play_time_only() {
    let (mut a, _, engine) = movies_app("movie_url");
    if let Some(Source {
        kind: SourceKind::Xtream {
            username, password, ..
        },
        ..
    }) = &mut a.settings.source
    {
        *username = "us er".into();
        *password = "p/w&".into();
    }
    open_movies(&mut a);
    all_movies(&mut a);
    a.handle_action(Action::Select, 0);
    a.handle_action(Action::Select, 0);
    assert_eq!(
        engine.loads.lock().unwrap().last().unwrap(),
        "http://tv.example.com/movie/us%20er/p%2Fw%26/1001.mkv"
    );
    let cached = std::fs::read_dir(seeded_cache_dir(&a))
        .unwrap()
        .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
        .collect::<String>();
    assert!(!cached.contains("us%20er") && !cached.contains("p%2Fw"));
    assert!(!cached.contains("movie/"), "no URLs in caches");
}

fn seeded_cache_dir(a: &App) -> std::path::PathBuf {
    let _ = a;
    std::env::temp_dir()
        .join(format!("skjalf-apptest-movie_url-{}", std::process::id()))
        .join("cache")
}

#[test]
fn on_demand_playback_disables_live_controls_and_keeps_last_channel() {
    let (mut a, _, engine) = movies_app("movie_playback");
    a.settings.last_channel = Some((source().id, "2".into()));
    open_movies(&mut a);
    all_movies(&mut a);
    a.handle_action(Action::Select, 0);
    a.handle_action(Action::Select, 0);
    assert!(a.is_on_demand());
    let loads = engine.loads.lock().unwrap().len();
    for action in [
        Action::Up,
        Action::ChUp,
        Action::ChDown,
        Action::Digit(1),
        Action::Epg,
    ] {
        a.handle_action(action, 5);
    }
    assert_eq!(
        engine.loads.lock().unwrap().len(),
        loads,
        "no stepping or zapping"
    );
    assert!(a.zap.digits().is_empty());
    assert_eq!(a.settings.last_channel, Some((source().id, "2".into())));
    a.handle_action(Action::Favorite, 6);
    assert!(a.favorites.contains(
        &source().id,
        &ContentKey::new(ContentKind::Movie, "1001").favorite_key()
    ));
    a.handle_action(Action::Back, 7);
    assert_eq!(a.router.current(), View::MovieDetails);
    assert!(a.movie_is_favorite());
}

#[test]
fn movie_favorites_are_separate_from_live_favorites() {
    let (mut a, _, _) = movies_app("movie_favorites");
    open_movies(&mut a);
    all_movies(&mut a);
    a.handle_action(Action::Favorite, 0);
    press(&mut a, &[Action::Down, Action::Down]);
    a.handle_action(Action::Favorite, 0);
    a.set_category(BrowserId::Movies, CategorySel::Favorites);
    assert_eq!(a.movies.filtered.len(), 2);
    a.set_query(BrowserId::Movies, "movie c".into());
    assert_eq!(a.movies.filtered.len(), 1);
    a.router.reset(View::Main);
    a.activate_main_button(MainButton::LiveTv);
    a.set_category(BrowserId::Live, CategorySel::Favorites);
    assert!(
        a.group_channel_ids().is_empty(),
        "movie keys never reach Live"
    );
    let stored: Vec<_> = a.favorites.iter().map(|(_, key)| key.clone()).collect();
    assert_eq!(stored, ["movie:1001", "movie:1003"]);
}

#[test]
fn startup_loads_the_cached_catalog_without_any_network_call() {
    let dir = seeded_dir("movies_cache");
    let store = Store::open(&dir).unwrap();
    store
        .save_movies(&MovieCatalog {
            source_id: source().id,
            groups: vec![Group {
                id: "12".into(),
                name: "Action".into(),
            }],
            movies: vec![Movie {
                id: "7".into(),
                name: "Cached".into(),
                group_id: "12".into(),
                icon: None,
                added: None,
                rating: None,
                extension: "mkv".into(),
            }],
        })
        .unwrap();
    let mut a = main_app_at(&dir);
    let fetcher = Arc::new(VodFetcher::default());
    a.fetcher = fetcher.clone();
    wait_for(&mut a, |a| a.movie_slot.loaded.is_some());
    a.activate_main_button(MainButton::Movies);
    a.poll_jobs(0);
    assert_eq!(a.movie_slot.status, LoadStatus::Ready);
    assert_eq!(a.movie_groups().len(), 1);
    assert!(fetcher.actions().is_empty(), "{:?}", fetcher.actions());
}

#[test]
fn network_failure_keeps_the_previous_catalog() {
    let (mut a, fetcher, _) = movies_app("movies_failure");
    open_movies(&mut a);
    all_movies(&mut a);
    fetcher.fail_catalog.store(true, Ordering::Relaxed);
    a.start_movie_load(LoadOrigin::Network);
    wait_for(&mut a, |a| {
        matches!(a.movie_slot.status, LoadStatus::Failed(_))
    });
    assert_eq!(a.movies.filtered.len(), 4);
    assert_eq!(a.browser_notice(BrowserId::Movies), None);
    assert!(a
        .settings_status()
        .iter()
        .any(|line| line.contains("refresh failed")));
}

#[test]
fn failed_first_load_offers_retry_on_select() {
    let (mut a, fetcher, _) = movies_app("movies_retry");
    fetcher.fail_catalog.store(true, Ordering::Relaxed);
    a.activate_main_button(MainButton::Movies);
    wait_for(&mut a, |a| {
        matches!(a.movie_slot.status, LoadStatus::Failed(_))
    });
    assert!(a
        .browser_notice(BrowserId::Movies)
        .unwrap()
        .contains("Select to retry"));
    fetcher.fail_catalog.store(false, Ordering::Relaxed);
    a.handle_action(Action::Select, 0);
    wait_for(&mut a, |a| a.movie_slot.loaded.is_some());
}

#[test]
fn stale_catalog_results_are_dropped_after_a_source_change() {
    let (mut a, _, _) = movies_app("movies_stale");
    a.activate_main_button(MainButton::Movies);
    a.router.reset(View::Main);
    let mut replacement = playlist();
    replacement.source.id = "xtream:http://other.example.com/u".into();
    let (tx, rx) = mpsc::channel();
    a.connect_rx = Some(rx);
    tx.send(skjalf::app::ConnectOutcome::Ready(replacement))
        .unwrap();
    a.poll_jobs(0);
    for _ in 0..60 {
        a.poll_jobs(0);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(a.source_id(), Some("xtream:http://other.example.com/u"));
    assert!(
        a.movie_slot.loaded.is_none(),
        "the old source's catalog must not install"
    );
}

#[test]
fn m3u_sources_explain_that_movies_need_xtream() {
    let mut a = main_app("movies_m3u");
    a.settings.source = Some(Source {
        id: "m3u:x".into(),
        name: "m3u".into(),
        kind: SourceKind::M3u {
            url: "http://x/list.m3u".into(),
        },
        stream_format: StreamFormat::M3u8,
    });
    a.activate_main_button(MainButton::Movies);
    let notice = a.browser_notice(BrowserId::Movies).unwrap();
    assert_eq!(
        notice,
        "Movies and series need an Xtream account. This source provides live channels only."
    );
    assert!(a.movie_slot.loaded.is_none());
    a.handle_action(Action::Back, 0);
    assert_eq!(a.router.current(), View::Main);
}

#[test]
fn search_and_favorites_on_a_186000_movie_catalog() {
    let dir = seeded_dir("movies_large");
    let store = Store::open(&dir).unwrap();
    store
        .save_movies(&MovieCatalog {
            source_id: source().id,
            groups: (0..434)
                .map(|n| Group {
                    id: n.to_string(),
                    name: format!("Category {n}"),
                })
                .collect(),
            movies: (0..186_000)
                .map(|n| Movie {
                    id: n.to_string(),
                    name: format!(
                        "Film {n:06} {}",
                        if n % 1000 == 3 { "Special" } else { "Plain" }
                    ),
                    group_id: (n % 434).to_string(),
                    icon: None,
                    added: None,
                    rating: None,
                    extension: "mkv".into(),
                })
                .collect(),
        })
        .unwrap();
    let mut a = main_app_at(&dir);
    wait_for(&mut a, |a| a.movie_slot.loaded.is_some());
    a.set_category(BrowserId::Movies, CategorySel::All);
    assert_eq!(a.movies.filtered.len(), 186_000);
    a.set_query(BrowserId::Movies, "SPECIAL".into());
    let expected: Vec<usize> = (0..186_000).filter(|n| n % 1000 == 3).collect();
    assert_eq!(a.movies.filtered, expected);
    a.set_category(BrowserId::Movies, group_sel("3"));
    assert_eq!(
        a.movies.filtered,
        (0..186_000usize)
            .filter(|n| n % 1000 == 3 && n % 434 == 3)
            .collect::<Vec<_>>()
    );
    a.set_category(BrowserId::Movies, CategorySel::All);
    a.activate_main_button(MainButton::Movies);
    a.handle_action(Action::Favorite, 0);
    a.set_query(BrowserId::Movies, String::new());
    a.set_category(BrowserId::Movies, CategorySel::Favorites);
    assert_eq!(a.movies.filtered, [3]);
    a.toggle_sort(BrowserId::Movies);
    a.set_category(BrowserId::Movies, CategorySel::All);
    let first = a.movies.filtered[0];
    assert_eq!(first, 0);
}

#[test]
fn touch_back_works_on_movies_and_movie_details() {
    let (mut a, _, _) = movies_app("movies_touch_back");
    open_movies(&mut a);
    all_movies(&mut a);
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    a.handle_action(Action::Select, 0);
    click_text(&ctx, &mut a, "Back", |ctx, app| {
        skjalf::views::movie_details::show(ctx, app, 0)
    });
    assert_eq!(a.router.current(), View::Movies);
    click_text(&ctx, &mut a, "Back", skjalf::views::movies::show);
    assert_eq!(a.router.current(), View::Main);
}

#[test]
fn movie_search_field_stays_above_the_steam_keyboard_at_deck_scale() {
    let (mut a, _, _) = movies_app("movies_search_compact");
    open_movies(&mut a);
    all_movies(&mut a);
    a.game_mode = true;
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    a.start_search_edit(BrowserId::Movies);
    let _ = ctx.run(compact_input(vec![]), |ctx| {
        skjalf::views::movies::show(ctx, &mut a)
    });
    let _ = ctx.run(compact_input(vec![]), |ctx| {
        skjalf::views::movies::show(ctx, &mut a)
    });
    let rect = ctx
        .read_response(skjalf::views::movies::search_field_id())
        .unwrap()
        .rect;
    assert!(rect.max.y < 200.0, "{rect:?}");
    assert!(a.search_editing());
}

// -- Refresh interval -----------------------------------------------------------

use std::sync::atomic::AtomicU64;

const DAY: u64 = 86_400;
const NOW: u64 = 1_800_000_000;

fn scheduled_app(tag: &str, last_live: Option<u64>) -> (App, Arc<VodFetcher>, Arc<AtomicU64>) {
    let (mut a, fetcher, _) = movies_app(tag);
    let clock = Arc::new(AtomicU64::new(NOW));
    let reader = clock.clone();
    a.clock = Arc::new(move || reader.load(Ordering::Relaxed));
    a.auto_refresh = true;
    if let Some(last) = last_live {
        a.settings.last_refresh.insert(
            source().id,
            skjalf_core::model::RefreshTimes {
                live: Some(last),
                ..Default::default()
            },
        );
    }
    (a, fetcher, clock)
}

fn live_attempts(fetcher: &VodFetcher) -> usize {
    fetcher.actions().iter().filter(|a| a.is_empty()).count()
}

fn settle(a: &mut App) {
    for _ in 0..60 {
        a.poll_jobs(0);
        std::thread::sleep(std::time::Duration::from_millis(3));
    }
}

#[test]
fn live_refreshes_only_when_older_than_the_interval() {
    let (mut due, due_fetcher, _) = scheduled_app("interval_due", Some(NOW - DAY - 1));
    settle(&mut due);
    assert_eq!(live_attempts(&due_fetcher), 1);

    let (mut fresh, fresh_fetcher, _) = scheduled_app("interval_fresh", Some(NOW - DAY + 60));
    settle(&mut fresh);
    assert_eq!(live_attempts(&fresh_fetcher), 0);

    let (mut off, off_fetcher, _) = scheduled_app("interval_off", Some(NOW - 30 * DAY));
    off.settings.refresh_interval = skjalf_core::model::RefreshInterval::Off;
    settle(&mut off);
    assert_eq!(live_attempts(&off_fetcher), 0);
}

#[test]
fn background_refresh_failure_keeps_content_and_stays_out_of_other_screens() {
    let (mut a, fetcher, _) = scheduled_app("interval_failure", None);
    let before = a.playlist.clone();
    a.activate_main_button(MainButton::LiveTv);
    settle(&mut a);
    assert_eq!(live_attempts(&fetcher), 1);
    assert_eq!(a.playlist, before);
    assert_eq!(a.notice, None);
    assert!(a.refresh_status.as_deref().unwrap().contains("failed"));
    assert_eq!(
        a.last_refresh(RefreshKind::Live),
        None,
        "failure records no time"
    );
    assert_eq!(a.router.current(), View::Live);
}

#[test]
fn schedule_checks_every_ten_minutes_and_never_starts_a_second_worker() {
    let (mut a, fetcher, clock) = scheduled_app("interval_throttle", Some(NOW - 2 * DAY));
    let (_tx, rx) = mpsc::channel();
    a.connect_rx = Some(rx); // a refresh is already running
    settle(&mut a);
    assert_eq!(live_attempts(&fetcher), 0, "no duplicate worker");
    a.connect_rx = None;
    clock.store(NOW + 100, Ordering::Relaxed);
    settle(&mut a);
    assert_eq!(live_attempts(&fetcher), 0, "next check is 10 minutes away");
    clock.store(NOW + 601, Ordering::Relaxed);
    settle(&mut a);
    assert_eq!(live_attempts(&fetcher), 1);
}

#[test]
fn successful_refresh_records_the_clock_time_per_kind() {
    let (mut a, _, clock) = scheduled_app("interval_record", None);
    clock.store(NOW + 5, Ordering::Relaxed);
    let (tx, rx) = mpsc::channel();
    a.connect_rx = Some(rx);
    tx.send(skjalf::app::ConnectOutcome::Ready(playlist()))
        .unwrap();
    a.poll_jobs(0);
    assert_eq!(a.last_refresh(RefreshKind::Live), Some(NOW + 5));
    let saved = a.store.as_ref().unwrap().load_settings().unwrap();
    assert_eq!(saved.last_refresh[&source().id].live, Some(NOW + 5));
    clock.store(NOW + 9, Ordering::Relaxed);
    a.activate_main_button(MainButton::Movies);
    wait_for(&mut a, |a| a.last_refresh(RefreshKind::Movies).is_some());
    assert_eq!(a.last_refresh(RefreshKind::Movies), Some(NOW + 9));
    assert_eq!(a.last_refresh(RefreshKind::Live), Some(NOW + 5));
}

#[test]
fn due_movie_catalog_refreshes_once_it_is_loaded_but_not_before() {
    let (mut a, fetcher, clock) = scheduled_app("interval_movies", Some(NOW));
    settle(&mut a);
    let streams = |f: &VodFetcher| {
        f.actions()
            .iter()
            .filter(|action| *action == "get_vod_streams")
            .count()
    };
    assert_eq!(streams(&fetcher), 0, "never loaded: nothing to refresh");
    open_movies(&mut a);
    assert_eq!(streams(&fetcher), 1);
    clock.store(NOW + 2 * DAY, Ordering::Relaxed);
    a.settings.last_refresh.get_mut(&source().id).unwrap().live = Some(NOW + 2 * DAY);
    settle(&mut a);
    assert_eq!(streams(&fetcher), 2, "due movies refresh in the background");
    assert_eq!(a.movie_slot.status, LoadStatus::Ready);
}

#[test]
fn settings_cycle_the_refresh_interval_and_persist_it() {
    use skjalf_core::model::RefreshInterval;
    let mut a = settings_app("interval_setting");
    assert_eq!(a.settings.refresh_interval, RefreshInterval::Hours24);
    press(&mut a, &[Action::Down, Action::Down]);
    assert_eq!(a.settings_focus, SettingsFocus::RefreshInterval);
    a.handle_action(Action::Right, 0);
    assert_eq!(a.settings.refresh_interval, RefreshInterval::Hours48);
    a.handle_action(Action::Select, 0);
    assert_eq!(a.settings.refresh_interval, RefreshInterval::Days7);
    press(
        &mut a,
        &[Action::Left, Action::Left, Action::Left, Action::Left],
    );
    assert_eq!(a.settings.refresh_interval, RefreshInterval::Hours6);
    let saved = a.store.as_ref().unwrap().load_settings().unwrap();
    assert_eq!(saved.refresh_interval, RefreshInterval::Hours6);
    a.handle_action(Action::Up, 0);
    assert_eq!(a.settings_focus, SettingsFocus::RefreshContent);
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    let output = ctx.run(compact_input(vec![]), |ctx| {
        skjalf::views::settings::show(ctx, &mut a)
    });
    let mut labels = Vec::new();
    for shape in &output.shapes {
        shape_texts(&shape.shape, &mut labels);
    }
    assert!(labels
        .iter()
        .any(|text| text == "Refresh Interval: 6 hours"));
}

#[test]
fn live_refresh_during_on_demand_playback_keeps_the_player() {
    let (mut a, _, engine) = movies_app("interval_playback");
    open_movies(&mut a);
    all_movies(&mut a);
    a.handle_action(Action::Select, 0);
    a.handle_action(Action::Select, 0);
    assert_eq!(a.router.current(), View::Player);
    let (tx, rx) = mpsc::channel();
    a.connect_rx = Some(rx);
    tx.send(skjalf::app::ConnectOutcome::Ready(playlist()))
        .unwrap();
    a.poll_jobs(1);
    assert_eq!(a.router.current(), View::Player);
    assert!(a.is_on_demand());
    assert_eq!(*engine.stopped.lock().unwrap(), 0);
}

// -- Series ---------------------------------------------------------------------

fn open_series(a: &mut App) {
    a.activate_main_button(MainButton::Series);
    wait_for(a, |a| a.series_slot.loaded.is_some());
    a.set_category(BrowserId::Series, CategorySel::All);
}

fn open_series_details(a: &mut App) {
    a.handle_action(Action::Select, 0);
    assert_eq!(a.router.current(), View::SeriesDetails);
    wait_for(a, |a| {
        a.series_details.as_ref().unwrap().info != DetailInfo::Loading
    });
}

fn series_selection(a: &App) -> (usize, usize, SeriesFocus) {
    let details = a.series_details.as_ref().unwrap();
    (details.season, details.episode, details.focus)
}

#[test]
fn opening_series_loads_the_catalog_and_m3u_sources_get_the_message() {
    let (mut a, fetcher, _) = movies_app("series_open");
    a.activate_main_button(MainButton::Series);
    assert_eq!(a.router.current(), View::Series);
    assert_eq!(
        a.browser_notice(BrowserId::Series).as_deref(),
        Some("Loading series...")
    );
    a.handle_action(Action::Back, 0);
    assert_eq!(a.router.current(), View::Main);
    open_series(&mut a);
    assert_eq!(a.series.filtered.len(), 3);
    let row = a
        .browser_row(BrowserId::Series, a.series.filtered[0])
        .unwrap();
    assert_eq!(row.label, "Show A   8.0");
    assert!(!fetcher
        .actions()
        .iter()
        .any(|action| action == "get_series_info"));

    let mut m3u = main_app("series_m3u");
    m3u.settings.source = Some(Source {
        id: "m3u:x".into(),
        name: "m3u".into(),
        kind: SourceKind::M3u {
            url: "http://x/list.m3u".into(),
        },
        stream_format: StreamFormat::M3u8,
    });
    m3u.activate_main_button(MainButton::Series);
    assert_eq!(
        m3u.browser_notice(BrowserId::Series).as_deref(),
        Some("Movies and series need an Xtream account. This source provides live channels only.")
    );
}

#[test]
fn series_details_order_seasons_numerically_and_select_the_first_episode() {
    let (mut a, _, _) = movies_app("series_details");
    open_series(&mut a);
    a.handle_action(Action::Select, 0);
    let details = a.series_details.as_ref().unwrap();
    assert_eq!(details.info, DetailInfo::Loading);
    wait_for(&mut a, |a| {
        a.series_details.as_ref().unwrap().info != DetailInfo::Loading
    });
    let details = a.series_details.as_ref().unwrap();
    let DetailInfo::Ready(info) = &details.info else {
        panic!("not ready");
    };
    let names: Vec<_> = info.seasons.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Opening", "Season 2", "Season 10"]);
    assert_eq!(series_selection(&a), (0, 0, SeriesFocus::Episodes));
    a.handle_action(Action::Left, 0);
    assert_eq!(series_selection(&a).2, SeriesFocus::Seasons);
    a.handle_action(Action::Down, 0);
    a.handle_action(Action::Down, 0);
    assert_eq!(series_selection(&a), (2, 0, SeriesFocus::Seasons));
    a.handle_action(Action::Down, 0);
    assert_eq!(series_selection(&a).0, 0, "seasons wrap");
    a.handle_action(Action::Down, 0);
    a.handle_action(Action::Right, 0);
    assert_eq!(series_selection(&a), (1, 0, SeriesFocus::Episodes));
    let episodes = a.series_details.as_ref().unwrap().episodes();
    let titles: Vec<_> = episodes.iter().map(|e| e.title.as_str()).collect();
    assert_eq!(titles, ["Two A", "Two B"], "episodes sort by number");
}

#[test]
fn series_flow_restores_season_and_episode_at_every_back_step() {
    let (mut a, _, engine) = movies_app("series_flow");
    open_series(&mut a);
    press(&mut a, &[Action::Down]); // Show B
    open_series_details(&mut a);
    a.handle_action(Action::Left, 0);
    a.handle_action(Action::Down, 0); // Season 2
    a.handle_action(Action::Right, 0);
    a.handle_action(Action::Down, 0); // Two B
    assert_eq!(series_selection(&a), (1, 1, SeriesFocus::Episodes));
    a.handle_action(Action::Select, 1);
    assert_eq!(a.router.current(), View::Player);
    assert_eq!(
        engine.loads.lock().unwrap().last().unwrap(),
        "http://tv.example.com/series/u/p/200221.mp4"
    );
    assert!(a.is_on_demand());
    a.handle_action(Action::Back, 2);
    assert_eq!(a.router.current(), View::SeriesDetails);
    assert_eq!(series_selection(&a), (1, 1, SeriesFocus::Episodes));
    a.handle_action(Action::Back, 3);
    assert_eq!(a.router.current(), View::Series);
    assert_eq!(a.series.selected, 1);
    a.handle_action(Action::Back, 4);
    assert_eq!(a.router.current(), View::Main);
    assert_eq!(a.main_focus, MainButton::Series);
    a.handle_action(Action::Select, 5);
    assert_eq!(a.series.selected, 1);
}

#[test]
fn stale_series_info_is_dropped_after_opening_another_series() {
    let (mut a, fetcher, _) = movies_app("series_stale");
    open_series(&mut a);
    let gate = fetcher.gate("2001");
    a.handle_action(Action::Select, 0);
    a.handle_action(Action::Back, 0);
    a.handle_action(Action::Down, 0);
    a.handle_action(Action::Select, 0);
    wait_for(&mut a, |a| {
        matches!(
            a.series_details.as_ref().unwrap().info,
            DetailInfo::Ready(_)
        )
    });
    gate.send(()).unwrap();
    settle(&mut a);
    let details = a.series_details.as_ref().unwrap();
    assert_eq!(details.series.id, "2002");
    let DetailInfo::Ready(info) = &details.info else {
        panic!("lost");
    };
    assert_eq!(info.plot.as_deref(), Some("Plot of 2002"));
}

#[test]
fn failed_series_info_offers_retry_and_retry_recovers() {
    let (mut a, fetcher, _) = movies_app("series_retry");
    fetcher.fail_info.store(true, Ordering::Relaxed);
    open_series(&mut a);
    a.handle_action(Action::Select, 0);
    wait_for(&mut a, |a| {
        a.series_details.as_ref().unwrap().info == DetailInfo::Unavailable
    });
    assert_eq!(series_selection(&a).2, SeriesFocus::Retry);
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    let output = ctx.run(compact_input(vec![]), |ctx| {
        skjalf::views::series_details::show(ctx, &mut a, 0)
    });
    let mut labels = Vec::new();
    for shape in &output.shapes {
        shape_texts(&shape.shape, &mut labels);
    }
    assert!(labels.iter().any(|text| text == "Retry"));
    assert!(labels.iter().any(|text| text == "Could not load episodes."));
    fetcher.fail_info.store(false, Ordering::Relaxed);
    a.handle_action(Action::Select, 0);
    assert_eq!(a.series_details.as_ref().unwrap().info, DetailInfo::Loading);
    wait_for(&mut a, |a| {
        matches!(
            a.series_details.as_ref().unwrap().info,
            DetailInfo::Ready(_)
        )
    });
    assert_eq!(series_selection(&a).2, SeriesFocus::Episodes);
}

#[test]
fn series_details_show_loading_and_empty_states() {
    let (mut a, fetcher, _) = movies_app("series_states");
    open_series(&mut a);
    let gate = fetcher.gate("2001");
    a.handle_action(Action::Select, 0);
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    let texts = |a: &mut App| {
        let output = ctx.run(compact_input(vec![]), |ctx| {
            skjalf::views::series_details::show(ctx, a, 0)
        });
        let mut labels = Vec::new();
        for shape in &output.shapes {
            shape_texts(&shape.shape, &mut labels);
        }
        labels
    };
    assert!(texts(&mut a)
        .iter()
        .any(|text| text == "Loading episodes..."));
    gate.send(()).unwrap();
    wait_for(&mut a, |a| {
        matches!(
            a.series_details.as_ref().unwrap().info,
            DetailInfo::Ready(_)
        )
    });
    let labels = texts(&mut a);
    assert!(labels.iter().any(|text| text == "E1  Ten") || labels.iter().any(|t| t == "E1  One"));
    assert!(labels.iter().any(|text| text == "Opening"));
}

#[test]
fn series_favorites_use_the_series_key_and_stay_separate_from_movies() {
    let (mut a, _, _) = movies_app("series_favorites");
    open_series(&mut a);
    a.handle_action(Action::Favorite, 0); // Show A from the browser
    open_series_details(&mut a);
    assert!(a.series_is_favorite());
    a.handle_action(Action::Favorite, 1); // toggled off from details
    assert!(!a.series_is_favorite());
    a.handle_action(Action::Favorite, 2);
    a.handle_action(Action::Select, 3); // episode plays
    a.handle_action(Action::Favorite, 4); // player toggles the series
    assert!(!a.series_is_favorite());
    a.handle_action(Action::Favorite, 5);
    assert!(a.series_is_favorite());
    let stored: Vec<_> = a.favorites.iter().map(|(_, key)| key.clone()).collect();
    assert_eq!(stored, ["series:2001"]);
    a.router.reset(View::Main);
    a.activate_main_button(MainButton::Series);
    a.set_category(BrowserId::Series, CategorySel::Favorites);
    assert_eq!(a.series.filtered, [0]);
    a.router.reset(View::Main);
    a.activate_main_button(MainButton::Movies);
    wait_for(&mut a, |a| a.movie_slot.loaded.is_some());
    a.set_category(BrowserId::Movies, CategorySel::Favorites);
    assert!(a.movies.filtered.is_empty());
}

#[test]
fn held_repeat_moves_inside_series_lists_without_wrapping_off_the_top() {
    let (mut a, _, _) = movies_app("series_repeat");
    open_series(&mut a);
    open_series_details(&mut a);
    a.handle_repeat(Action::Down);
    assert_eq!(series_selection(&a).1, 0, "season one has one episode");
    a.handle_action(Action::Left, 0);
    a.handle_repeat(Action::Down);
    a.handle_repeat(Action::Down);
    assert_eq!(series_selection(&a), (2, 0, SeriesFocus::Seasons));
    a.handle_repeat(Action::Right);
    assert_eq!(
        series_selection(&a).2,
        SeriesFocus::Seasons,
        "no sideways repeat"
    );
    a.handle_repeat(Action::Down);
    a.handle_repeat(Action::Up);
    assert_eq!(
        series_selection(&a).0,
        0,
        "repeat moved back to the top season"
    );
    a.handle_repeat(Action::Up);
    assert_eq!(series_selection(&a).0, 0, "no wrap off the top while held");
}

#[test]
fn episode_touch_click_plays_and_seasons_select_by_touch() {
    let (mut a, _, engine) = movies_app("series_touch");
    open_series(&mut a);
    open_series_details(&mut a);
    a.select_season(2);
    assert_eq!(series_selection(&a), (2, 0, SeriesFocus::Seasons));
    a.click_episode(0, 0);
    assert_eq!(a.router.current(), View::Player);
    assert_eq!(
        engine.loads.lock().unwrap().last().unwrap(),
        "http://tv.example.com/series/u/p/200110.mkv"
    );
    a.handle_action(Action::Back, 1);
    a.handle_action(Action::Back, 2);
    assert_eq!(a.router.current(), View::Series);
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    a.handle_action(Action::Select, 3);
    wait_for(&mut a, |a| {
        matches!(
            a.series_details.as_ref().unwrap().info,
            DetailInfo::Ready(_)
        )
    });
    click_text(&ctx, &mut a, "Back", |ctx, app| {
        skjalf::views::series_details::show(ctx, app, 0)
    });
    assert_eq!(a.router.current(), View::Series);
}

#[test]
fn series_cache_loads_at_startup_without_network_and_failures_keep_it() {
    let dir = seeded_dir("series_cache");
    let store = Store::open(&dir).unwrap();
    store
        .save_series(&skjalf_core::catalog::SeriesCatalog {
            source_id: source().id,
            groups: vec![],
            series: vec![skjalf_core::catalog::SeriesItem {
                id: "9".into(),
                name: "Cached Show".into(),
                group_id: "x".into(),
                cover: None,
                last_modified: None,
                rating: None,
            }],
        })
        .unwrap();
    let mut a = main_app_at(&dir);
    let fetcher = Arc::new(VodFetcher::default());
    a.fetcher = fetcher.clone();
    wait_for(&mut a, |a| a.series_slot.loaded.is_some());
    a.activate_main_button(MainButton::Series);
    a.poll_jobs(0);
    assert!(fetcher.actions().is_empty());
    a.set_category(BrowserId::Series, CategorySel::All);
    assert_eq!(a.series.filtered.len(), 1);
    fetcher.fail_catalog.store(true, Ordering::Relaxed);
    a.start_series_load(LoadOrigin::Network);
    wait_for(&mut a, |a| {
        matches!(a.series_slot.status, LoadStatus::Failed(_))
    });
    assert_eq!(a.series.filtered.len(), 1, "failure keeps the catalog");
    assert_eq!(a.browser_notice(BrowserId::Series), None);
}

#[test]
fn refresh_content_reloads_every_catalog_and_settings_lists_series() {
    let (mut a, fetcher, _) = movies_app("series_refresh_content");
    a.activate_main_button(MainButton::Settings);
    a.activate_settings_button(SettingsFocus::RefreshContent);
    wait_for(&mut a, |a| {
        a.movie_slot.loaded.is_some() && a.series_slot.loaded.is_some()
    });
    let actions = fetcher.actions();
    assert!(actions.contains(&"get_vod_streams".to_owned()));
    assert!(actions.contains(&"get_series".to_owned()));
    let lines = a.settings_status();
    assert!(
        lines.iter().any(|line| line.starts_with("Series: 3")),
        "{lines:?}"
    );
    assert!(a.last_refresh(RefreshKind::Series).is_some());
    assert!(
        !actions.iter().any(|action| action == "get_series_info"),
        "never fetch episode lists in bulk"
    );
}

#[test]
fn due_series_catalog_refreshes_in_the_background() {
    let (mut a, fetcher, clock) = scheduled_app("series_interval", Some(NOW));
    open_series(&mut a);
    clock.store(NOW + 2 * DAY, Ordering::Relaxed);
    a.settings.last_refresh.get_mut(&source().id).unwrap().live = Some(NOW + 2 * DAY);
    settle(&mut a);
    let loads = fetcher
        .actions()
        .iter()
        .filter(|action| *action == "get_series")
        .count();
    assert_eq!(loads, 2);
}

// -- Review fixes: details layout, stale program data, opened kinds ----------------

fn text_positions(output: &egui::FullOutput, label: &str) -> Vec<egui::Pos2> {
    let mut found = None;
    for clipped in &output.shapes {
        find_text_pos(&clipped.shape, label, &mut found);
    }
    found.into_iter().collect()
}

#[test]
fn long_movie_metadata_keeps_play_and_favorite_on_screen() {
    let (mut a, fetcher, _) = movies_app("movie_long_metadata");
    fetcher.long_text.store(true, Ordering::Relaxed);
    open_movies(&mut a);
    all_movies(&mut a);
    a.handle_action(Action::Select, 0);
    wait_for(&mut a, |a| {
        matches!(a.movie_details.as_ref().unwrap().info, DetailInfo::Ready(_))
    });
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    let mut output = None;
    for _ in 0..2 {
        output = Some(ctx.run(compact_input(vec![]), |ctx| {
            skjalf::views::movie_details::show(ctx, &mut a, 0)
        }));
    }
    let output = output.unwrap();
    for label in ["Play", "Favorite"] {
        let positions = text_positions(&output, label);
        assert_eq!(positions.len(), 1, "{label} is drawn");
        assert!(positions[0].y < 200.0, "{label} at {:?}", positions[0]);
    }
}

#[test]
fn long_series_plot_keeps_the_episode_list_and_favorite_on_screen() {
    let (mut a, fetcher, _) = movies_app("series_long_plot");
    fetcher.long_text.store(true, Ordering::Relaxed);
    open_series(&mut a);
    open_series_details(&mut a);
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    let mut output = None;
    for _ in 0..2 {
        output = Some(ctx.run(compact_input(vec![]), |ctx| {
            skjalf::views::series_details::show(ctx, &mut a, 0)
        }));
    }
    let output = output.unwrap();
    let favorite = text_positions(&output, "Favorite");
    assert_eq!(favorite.len(), 1);
    assert!(favorite[0].y < 250.0, "{favorite:?}");
    let episode = text_positions(&output, "E1  One");
    assert_eq!(episode.len(), 1, "episodes remain visible");
    assert!(episode[0].y < 400.0, "{episode:?}");
}

#[test]
fn series_favorite_button_is_a_controller_focus_stop() {
    let (mut a, _, _) = movies_app("series_favorite_focus");
    open_series(&mut a);
    open_series_details(&mut a);
    assert_eq!(series_selection(&a).2, SeriesFocus::Episodes);
    a.handle_action(Action::Up, 0);
    assert_eq!(series_selection(&a).2, SeriesFocus::Favorite);
    a.handle_action(Action::Select, 0);
    assert!(a.series_is_favorite());
    a.handle_action(Action::Down, 0);
    assert_eq!(series_selection(&a).2, SeriesFocus::Episodes);
    a.handle_action(Action::Left, 0);
    a.handle_action(Action::Up, 0);
    assert_eq!(series_selection(&a).2, SeriesFocus::Favorite);
    a.handle_repeat(Action::Down);
    assert_eq!(
        series_selection(&a).2,
        SeriesFocus::Favorite,
        "repeats stay in lists"
    );
    a.handle_action(Action::Back, 0);
    assert_eq!(a.router.current(), View::Series);
}

#[test]
fn failed_series_details_reach_favorite_and_retry_with_up_and_down() {
    let (mut a, fetcher, _) = movies_app("series_favorite_retry");
    fetcher.fail_info.store(true, Ordering::Relaxed);
    open_series(&mut a);
    a.handle_action(Action::Select, 0);
    wait_for(&mut a, |a| {
        a.series_details.as_ref().unwrap().info == DetailInfo::Unavailable
    });
    a.handle_action(Action::Up, 0);
    assert_eq!(series_selection(&a).2, SeriesFocus::Favorite);
    a.handle_action(Action::Down, 0);
    assert_eq!(series_selection(&a).2, SeriesFocus::Retry);
}

#[test]
fn program_data_from_a_left_live_channel_never_reaches_a_movie_overlay() {
    let (mut a, fetcher, _) = movies_app("stale_epg");
    let gate = fetcher.gate("epg");
    a.activate_main_button(MainButton::LiveTv);
    a.handle_action(Action::Select, 0);
    assert_eq!(a.router.current(), View::Player);
    a.handle_action(Action::Back, 1);
    a.handle_action(Action::Back, 2);
    open_movies(&mut a);
    all_movies(&mut a);
    a.handle_action(Action::Select, 3);
    a.handle_action(Action::Select, 4);
    assert!(a.is_on_demand());
    gate.send(()).unwrap();
    settle(&mut a);
    assert!(a.epg.is_empty(), "{:?}", a.epg);
    assert!(a.epg_rx.is_none());
}

#[test]
fn epg_result_for_another_channel_is_ignored_while_live_plays() {
    let (mut a, fetcher, _) = movies_app("epg_channel_tag");
    let gate = fetcher.gate("epg");
    a.activate_main_button(MainButton::LiveTv);
    a.handle_action(Action::Select, 0); // channel "1", request gated
    a.player.channel_id = Some("2".into()); // playback moved on; same receiver
    a.epg_fetched = Some(("2".into(), 0)); // no fresh request for the new channel
    gate.send(()).unwrap();
    settle(&mut a);
    assert!(a.epg.is_empty());
}

#[test]
fn opened_but_never_loaded_catalogs_are_retried_by_the_schedule_and_persist() {
    let (mut a, fetcher, clock) = scheduled_app("opened_retry", Some(NOW));
    fetcher.fail_catalog.store(true, Ordering::Relaxed);
    a.activate_main_button(MainButton::Movies);
    wait_for(&mut a, |a| {
        matches!(a.movie_slot.status, LoadStatus::Failed(_))
    });
    a.router.reset(View::Main);
    let saved = a.store.as_ref().unwrap().load_settings().unwrap();
    assert!(saved.last_refresh[&source().id].movies_opened);
    assert!(!saved.last_refresh[&source().id].series_opened);
    assert!(a.movie_slot.loaded.is_none());
    fetcher.fail_catalog.store(false, Ordering::Relaxed);
    clock.store(NOW + 601, Ordering::Relaxed);
    wait_for(&mut a, |a| a.movie_slot.loaded.is_some());
    assert_eq!(a.last_refresh(RefreshKind::Movies), Some(NOW + 601));
    let streams = fetcher
        .actions()
        .iter()
        .filter(|action| *action == "get_series")
        .count();
    assert_eq!(streams, 0, "kinds never opened stay untouched");
}

#[test]
fn never_opened_kinds_without_a_catalog_are_not_refreshed() {
    let (mut a, fetcher, clock) = scheduled_app("never_opened", Some(NOW));
    clock.store(NOW + 3 * DAY, Ordering::Relaxed);
    settle(&mut a);
    assert!(!fetcher
        .actions()
        .iter()
        .any(|action| action.starts_with("get_vod")));
    assert!(!fetcher
        .actions()
        .iter()
        .any(|action| action.starts_with("get_series")));
}

// -- C5: transport bar, seek, resume --------------------------------------------

fn dir_of(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("skjalf-apptest-{tag}-{}", std::process::id()))
}

fn set_clock(engine: &MockEngine, position: f64, duration: f64) {
    *engine.position.lock().unwrap() = Some(position);
    *engine.duration.lock().unwrap() = Some(duration);
}

/// A movie playing through the fake engine, ready for transport tests.
fn playing_movie(tag: &str) -> (App, Arc<VodFetcher>, Arc<MockEngine>) {
    let (mut a, fetcher, engine) = movies_app(tag);
    open_movies(&mut a);
    all_movies(&mut a);
    a.handle_action(Action::Select, 0); // details
    a.handle_action(Action::Select, 0); // Play
    assert_eq!(a.router.current(), View::Player);
    set_clock(&engine, 100.0, 5400.0);
    a.poll_jobs(1_000);
    (a, fetcher, engine)
}

fn saved_progress(a: &App, id: &str) -> Option<skjalf_core::progress::ProgressEntry> {
    let store = a.store.as_ref().unwrap();
    store
        .load_progress()
        .get(&source().id, ProgressKind::Movie, id)
        .cloned()
}

#[test]
fn select_toggles_pause_in_on_demand_playback() {
    let (mut a, _, engine) = playing_movie("c5_pause");
    assert!(!a.paused);
    a.handle_action(Action::Select, 2_000);
    assert!(a.paused);
    a.handle_action(Action::Select, 3_000);
    assert!(!a.paused);
    assert_eq!(*engine.pauses.lock().unwrap(), [false, true, false]);
}

#[test]
fn directions_and_shoulders_seek_in_on_demand_playback() {
    let (mut a, _, engine) = playing_movie("c5_seek");
    a.handle_action(Action::Left, 2_000);
    a.handle_action(Action::Right, 2_000);
    a.handle_action(Action::ChUp, 2_000); // L1
    a.handle_action(Action::ChDown, 2_000); // R1
    assert_eq!(*engine.seeks.lock().unwrap(), [-10.0, 10.0, -60.0, 60.0]);
    assert_eq!(a.settings.volume, 80, "directions no longer change volume");
    assert_eq!(
        a.media.position,
        Some(100.0),
        "optimistic position nets out"
    );
}

#[test]
fn seek_bar_focus_and_bar_buttons_seek() {
    let (mut a, _, engine) = playing_movie("c5_seek_bar");
    a.handle_action(Action::Down, 2_000);
    assert_eq!(a.player_focus, PlayerFocus::Controls(2));
    a.handle_action(Action::Down, 2_000);
    assert_eq!(a.player_focus, PlayerFocus::Seek);
    a.handle_action(Action::Right, 2_000);
    a.handle_action(Action::Left, 2_000);
    a.handle_action(Action::Up, 2_000);
    assert_eq!(a.player_focus, PlayerFocus::Controls(2));
    a.handle_action(Action::Left, 2_000); // Rewind
    a.handle_action(Action::Select, 2_000);
    a.handle_action(Action::Right, 2_000);
    a.handle_action(Action::Right, 2_000); // Forward
    a.handle_action(Action::Select, 2_000);
    assert_eq!(*engine.seeks.lock().unwrap(), [10.0, -10.0, -10.0, 10.0]);
    a.handle_action(Action::Right, 2_000);
    assert_eq!(a.player_focus, PlayerFocus::Controls(4), "Volume");
    a.handle_action(Action::Select, 2_000);
    assert!(a.muted, "Select toggles mute");
    assert_eq!(a.settings.volume, 80);
    a.handle_action(Action::Back, 2_000);
    assert_eq!(a.player_focus, PlayerFocus::Video);
    assert_eq!(a.router.current(), View::Player);
}

#[test]
fn touch_on_the_seek_bar_seeks_to_the_touched_position() {
    let (mut a, _, engine) = playing_movie("c5_seek_touch");
    a.seek_to_fraction(0.5, 2_000);
    assert_eq!(*engine.seeks.lock().unwrap(), [2700.0 - 100.0]);
    assert_eq!(a.media.position, Some(2700.0));
    a.seek_to_fraction(2.0, 2_000);
    assert_eq!(a.media.position, Some(5400.0), "clamped to the duration");
}

#[test]
fn bar_controls_differ_by_mode_and_leave_no_text_buttons() {
    use skjalf::app::bar_controls;
    assert_eq!(
        bar_controls(true),
        [
            BarControl::Back,
            BarControl::Rewind,
            BarControl::PlayPause,
            BarControl::Forward,
            BarControl::Volume
        ]
    );
    assert_eq!(
        bar_controls(false),
        [
            BarControl::Back,
            BarControl::Previous,
            BarControl::PlayPause,
            BarControl::Next,
            BarControl::Favorite,
            BarControl::Volume
        ]
    );
}

#[test]
fn live_bar_steps_channels_pauses_and_keeps_direction_behavior() {
    let mut a = app("c5_live_bar");
    let engine = MockEngine::default();
    a.engine = Some(Box::new(engine.clone()));
    a.handle_action(Action::Select, 0);
    assert_eq!(a.player.channel_id.as_deref(), Some("1"));
    a.handle_action(Action::Up, 1);
    assert_eq!(a.player.channel_id.as_deref(), Some("2"), "Up still steps");
    a.handle_action(Action::Right, 1);
    assert_eq!(a.settings.volume, 85, "Right still changes volume");
    a.handle_action(Action::Left, 1);
    a.handle_action(Action::ChDown, 1);
    assert_eq!(a.player.channel_id.as_deref(), Some("1"));
    a.handle_action(Action::Down, 1);
    assert_eq!(a.player_focus, PlayerFocus::Controls(2));
    a.handle_action(Action::Right, 1);
    a.handle_action(Action::Select, 1); // Next
    assert_eq!(a.player.channel_id.as_deref(), Some("2"));
    a.handle_action(Action::Left, 1);
    a.handle_action(Action::Left, 1);
    a.handle_action(Action::Select, 1); // Previous
    assert_eq!(a.player.channel_id.as_deref(), Some("1"));
    a.handle_action(Action::Right, 1);
    a.handle_action(Action::Select, 1); // Play/Pause
    assert!(a.paused);
    assert_eq!(engine.pauses.lock().unwrap().last(), Some(&true));
    a.handle_action(Action::Down, 1);
    assert_eq!(a.player_focus, PlayerFocus::Video, "no seek bar in live");
    assert!(engine.seeks.lock().unwrap().is_empty());
    a.handle_action(Action::Left, 1);
    a.handle_action(Action::Select, 1);
}

#[test]
fn new_playback_never_starts_paused() {
    let mut a = app("c5_unpause");
    let engine = MockEngine::default();
    a.engine = Some(Box::new(engine.clone()));
    a.handle_action(Action::Select, 0);
    a.handle_action(Action::Down, 0);
    a.handle_action(Action::Select, 0); // Play/Pause is selected
    assert!(a.paused);
    a.handle_action(Action::Up, 0);
    a.handle_action(Action::ChUp, 0);
    assert!(!a.paused);
    assert_eq!(engine.pauses.lock().unwrap().last(), Some(&false));
}

#[test]
fn back_saves_the_position_before_stopping() {
    let (mut a, _, engine) = playing_movie("c5_back_saves");
    set_clock(&engine, 612.5, 5400.0);
    a.handle_action(Action::Back, 2_000);
    assert_eq!(a.router.current(), View::MovieDetails);
    let saved = saved_progress(&a, "1001").unwrap();
    assert_eq!(saved.position_secs, 612.5);
    assert_eq!(saved.duration_secs, Some(5400.0));
    assert!(!saved.finished);
    assert_eq!(saved.title, "Movie A");
    assert_eq!(a.continue_items.len(), 1);
}

#[test]
fn position_is_saved_every_ten_seconds_during_playback() {
    let (mut a, _, engine) = playing_movie("c5_periodic");
    set_clock(&engine, 200.0, 5400.0);
    a.poll_jobs(5_000);
    assert!(saved_progress(&a, "1001").is_none(), "not yet");
    a.poll_jobs(11_000);
    assert_eq!(saved_progress(&a, "1001").unwrap().position_secs, 200.0);
    set_clock(&engine, 300.0, 5400.0);
    a.poll_jobs(15_000);
    assert_eq!(saved_progress(&a, "1001").unwrap().position_secs, 200.0);
    a.poll_jobs(21_000);
    assert_eq!(saved_progress(&a, "1001").unwrap().position_secs, 300.0);
}

#[test]
fn position_is_read_from_the_engine_at_most_every_500_ms() {
    let (mut a, _, engine) = playing_movie("c5_rate");
    set_clock(&engine, 150.0, 5400.0);
    a.poll_jobs(1_200);
    assert_eq!(a.media.position, Some(100.0), "read 200 ms ago");
    a.poll_jobs(1_600);
    assert_eq!(a.media.position, Some(150.0));
}

#[test]
fn ended_marks_the_item_finished_and_back_keeps_it_that_way() {
    let (mut a, _, engine) = playing_movie("c5_ended");
    set_clock(&engine, 5399.0, 5400.0);
    engine.events.lock().unwrap().push(PlayerEvent::Ended);
    a.poll_jobs(2_000);
    let saved = saved_progress(&a, "1001").unwrap();
    assert!(saved.finished);
    assert_eq!(saved.position_secs, 5400.0);
    assert!(a.continue_items.is_empty());
    set_clock(&engine, 10.0, 5400.0);
    a.handle_action(Action::Back, 3_000);
    assert!(saved_progress(&a, "1001").unwrap().finished);
}

#[test]
fn live_playback_stores_no_progress() {
    let mut a = app("c5_live_none");
    let engine = MockEngine::default();
    set_clock(&engine, 900.0, 3600.0);
    a.engine = Some(Box::new(engine.clone()));
    a.handle_action(Action::Select, 0);
    a.poll_jobs(30_000);
    a.handle_action(Action::Back, 31_000);
    assert!(a.progress.continue_watching(&source().id, 12).is_empty());
    assert!(!a.store.as_ref().is_some_and(|store| {
        !store
            .load_progress()
            .continue_watching(&source().id, 12)
            .is_empty()
    }));
}

#[test]
fn quit_saves_the_position() {
    let (mut a, _, engine) = playing_movie("c5_quit");
    set_clock(&engine, 700.0, 5400.0);
    a.handle_action(Action::Quit, 2_000);
    assert!(a.wants_exit());
    assert_eq!(saved_progress(&a, "1001").unwrap().position_secs, 700.0);
}

#[test]
fn dropping_the_app_saves_the_position() {
    let (a, _, engine) = playing_movie("c5_drop");
    set_clock(&engine, 710.0, 5400.0);
    let dir = dir_of("c5_drop");
    drop(a);
    let progress = Store::open(&dir).unwrap().load_progress();
    assert_eq!(
        progress
            .get(&source().id, ProgressKind::Movie, "1001")
            .unwrap()
            .position_secs,
        710.0
    );
}

#[test]
fn resume_survives_a_restart_and_play_from_start_passes_no_start() {
    let tag = "c5_restart";
    let (mut a, _, engine) = playing_movie(tag);
    set_clock(&engine, 600.0, 5400.0);
    a.handle_action(Action::Back, 2_000);
    drop(a);

    let mut b = main_app_at(&dir_of(tag));
    b.fetcher = Arc::new(VodFetcher::default());
    let engine = MockEngine::default();
    b.engine = Some(Box::new(engine.clone()));
    wait_for(&mut b, |b| b.movie_slot.loaded.is_some());
    b.activate_main_button(MainButton::Movies);
    b.set_category(BrowserId::Movies, CategorySel::All);
    b.handle_action(Action::Select, 0);
    assert_eq!(b.movie_button_label(MovieButton::Resume), "Resume 10:00");
    assert_eq!(b.movie_button_label(MovieButton::Play), "Play from Start");
    assert_eq!(b.movie_buttons().len(), 3);
    b.handle_action(Action::Select, 0); // Resume is selected first
    assert_eq!(*engine.starts.lock().unwrap(), [Some(600.0)]);
    b.handle_action(Action::Back, 1);
    b.handle_action(Action::Down, 0);
    b.handle_action(Action::Select, 0); // Play from Start
    assert_eq!(engine.starts.lock().unwrap().last(), Some(&None));
}

#[test]
fn movie_without_progress_shows_a_plain_play_button() {
    let (mut a, _, _) = movies_app("c5_plain_play");
    open_movies(&mut a);
    all_movies(&mut a);
    a.handle_action(Action::Select, 0);
    assert_eq!(
        a.movie_buttons(),
        [MovieButton::Play, MovieButton::Favorite]
    );
    assert_eq!(a.movie_button_label(MovieButton::Play), "Play");
}

fn watch_episode(a: &mut App, engine: &MockEngine, position: f64, now: u64) {
    a.handle_action(Action::Select, now); // first episode of season 1
    set_clock(engine, position, 2700.0);
    a.poll_jobs(now + 1_000);
    a.handle_action(Action::Back, now + 2_000);
}

#[test]
fn episode_with_progress_offers_resume_or_start_and_back_cancels() {
    let (mut a, _, engine) = movies_app("c5_episode_resume");
    open_series(&mut a);
    open_series_details(&mut a);
    watch_episode(&mut a, &engine, 900.0, 0);
    assert_eq!(a.router.current(), View::SeriesDetails);
    let entry = a
        .progress_entry_for(ProgressKind::Episode, "200111")
        .unwrap();
    assert_eq!(entry.series_id.as_deref(), Some("2001"));
    assert_eq!((entry.season, entry.episode), (Some(1), Some(1)));

    a.handle_action(Action::Select, 5_000);
    let choice = a.series_details.as_ref().unwrap().choice.unwrap();
    assert!(choice.resume_focused);
    assert_eq!(a.router.current(), View::SeriesDetails, "asks first");
    a.handle_action(Action::Back, 5_000);
    assert!(a.series_details.as_ref().unwrap().choice.is_none());
    assert_eq!(a.router.current(), View::SeriesDetails, "Back only cancels");

    a.handle_action(Action::Select, 6_000);
    a.handle_action(Action::Select, 6_000); // Resume
    assert_eq!(engine.starts.lock().unwrap().last(), Some(&Some(900.0)));
    a.handle_action(Action::Back, 7_000);
    a.handle_action(Action::Select, 8_000);
    a.handle_action(Action::Right, 8_000);
    a.handle_action(Action::Select, 8_000); // Play from Start
    assert_eq!(engine.starts.lock().unwrap().last(), Some(&None));
}

#[test]
fn other_episodes_play_without_a_choice_and_rows_show_watch_status() {
    let (mut a, _, engine) = movies_app("c5_episode_status");
    open_series(&mut a);
    open_series_details(&mut a);
    watch_episode(&mut a, &engine, 900.0, 0);
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    let output = ctx.run(compact_input(vec![]), |ctx| {
        skjalf::views::series_details::show(ctx, &mut a, 0)
    });
    let mut labels = Vec::new();
    for shape in &output.shapes {
        shape_texts(&shape.shape, &mut labels);
    }
    assert!(labels.iter().any(|text| text == "E1  One"), "{labels:?}");
    assert!(labels.iter().any(|text| text == "30:00 left"), "{labels:?}");
    a.select_season(1);
    a.handle_action(Action::Right, 0);
    a.handle_action(Action::Select, 9_000);
    assert_eq!(a.router.current(), View::Player);
    assert_eq!(engine.starts.lock().unwrap().last(), Some(&None));
}

#[test]
fn finished_episodes_read_watched() {
    let (mut a, _, engine) = movies_app("c5_episode_watched");
    open_series(&mut a);
    open_series_details(&mut a);
    a.handle_action(Action::Select, 0);
    set_clock(&engine, 2699.0, 2700.0);
    engine.events.lock().unwrap().push(PlayerEvent::Ended);
    a.poll_jobs(1_000);
    a.handle_action(Action::Back, 2_000);
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    let output = ctx.run(compact_input(vec![]), |ctx| {
        skjalf::views::series_details::show(ctx, &mut a, 0)
    });
    let mut labels = Vec::new();
    for shape in &output.shapes {
        shape_texts(&shape.shape, &mut labels);
    }
    assert!(labels.iter().any(|text| text == "E1  One"));
    assert!(labels.iter().any(|text| text == "Watched"));
    a.handle_action(Action::Select, 3_000);
    assert_eq!(
        a.router.current(),
        View::Player,
        "finished plays from the start"
    );
}

#[test]
fn transport_bar_stays_on_screen_without_missing_glyphs() {
    for (width, height) in [(1280.0_f32, 800.0_f32), (640.0, 400.0)] {
        for on_demand in [true, false] {
            let (mut a, _, engine) = if on_demand {
                playing_movie("c5_bar_render")
            } else {
                let mut live = app("c5_bar_render_live");
                let engine = Arc::new(MockEngine::default());
                live.engine = Some(Box::new((*engine).clone()));
                live.handle_action(Action::Select, 0);
                (live, Arc::new(VodFetcher::default()), engine)
            };
            set_clock(&engine, 1234.0, 7300.0);
            a.poll_jobs(2_000);
            a.player_focus = PlayerFocus::Controls(2);
            let ctx = egui::Context::default();
            skjalf::theme::apply(&ctx);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height));
            let mut output = None;
            for frame in 0..6 {
                output = Some(ctx.run(
                    egui::RawInput {
                        screen_rect: Some(screen),
                        time: Some(f64::from(frame) * 0.1),
                        ..Default::default()
                    },
                    |ctx| skjalf::views::player::show(ctx, &mut a, 2_000),
                ));
            }
            for control in skjalf::app::bar_controls(on_demand) {
                let rect = ctx
                    .read_response(skjalf::views::transport_bar::control_id(*control))
                    .unwrap_or_else(|| panic!("{control:?} not drawn"))
                    .rect;
                assert!(
                    screen.contains_rect(rect),
                    "{control:?} at {rect:?} in {screen:?}"
                );
                assert!(rect.height() >= 44.0 - 0.5);
            }
            if on_demand {
                let rect = ctx
                    .read_response(skjalf::views::transport_bar::seek_bar_id())
                    .unwrap()
                    .rect;
                assert!(screen.contains_rect(rect), "seek bar {rect:?}");
            }
            let mut labels = Vec::new();
            for shape in &output.unwrap().shapes {
                shape_texts(&shape.shape, &mut labels);
            }
            assert!(
                labels.iter().all(|text| !text.contains('\u{fffd}')),
                "{labels:?}"
            );
            if on_demand {
                assert!(
                    labels.iter().any(|text| text == "20:34 / 2:01:40"),
                    "{labels:?}"
                );
            }
        }
    }
}

#[test]
fn clicking_a_bar_button_does_what_select_does() {
    let (mut a, _, engine) = playing_movie("c5_bar_click");
    a.player_focus = PlayerFocus::Controls(2);
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    let input = |events: Vec<egui::Event>| egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280.0, 800.0),
        )),
        events,
        ..Default::default()
    };
    let draw = |ctx: &egui::Context, a: &mut App, events| {
        ctx.run(input(events), |ctx| {
            skjalf::views::player::show(ctx, a, 2_000)
        })
    };
    for _ in 0..5 {
        draw(&ctx, &mut a, vec![]);
    }
    let rect = ctx
        .read_response(skjalf::views::transport_bar::control_id(
            BarControl::Forward,
        ))
        .unwrap()
        .rect;
    let pos = rect.center();
    let button = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    draw(
        &ctx,
        &mut a,
        vec![egui::Event::PointerMoved(pos), button(true)],
    );
    draw(&ctx, &mut a, vec![button(false)]);
    assert_eq!(*engine.seeks.lock().unwrap(), [10.0]);
}

// -- C5 part 2: Main content rows --------------------------------------------------

use skjalf::app::MainRow;
use skjalf_core::progress::{Progress, ProgressEntry};

fn progress_entry(
    id: &str,
    kind: ProgressKind,
    title: &str,
    updated_at: i64,
    position: f64,
) -> ProgressEntry {
    ProgressEntry {
        source_id: source().id,
        kind,
        id: id.into(),
        title: title.into(),
        position_secs: position,
        duration_secs: Some(5400.0),
        updated_at,
        finished: false,
        series_id: None,
        series_name: None,
        season: None,
        episode: None,
    }
}

fn episode_progress(id: &str, updated_at: i64) -> ProgressEntry {
    ProgressEntry {
        series_id: Some("2002".into()),
        series_name: Some("Show B".into()),
        season: Some(2),
        episode: Some(2),
        ..progress_entry(
            id,
            ProgressKind::Episode,
            "Show B  E2  Two B",
            updated_at,
            600.0,
        )
    }
}

/// An app whose store already holds progress and cached catalogs.
fn main_with_content(tag: &str) -> (App, Arc<VodFetcher>) {
    let dir = seeded_dir(tag);
    let store = Store::open(&dir).unwrap();
    let mut progress = Progress::default();
    progress.record(progress_entry(
        "1001",
        ProgressKind::Movie,
        "Movie A",
        10,
        100.0,
    ));
    progress.record(episode_progress("200221", 30));
    progress.record(progress_entry(
        "1004",
        ProgressKind::Movie,
        "Other D",
        20,
        200.0,
    ));
    progress.record(progress_entry(
        "1002",
        ProgressKind::Movie,
        "Movie B",
        40,
        5300.0,
    ));
    progress.record(progress_entry(
        "1003",
        ProgressKind::Movie,
        "Movie C",
        50,
        5.0,
    ));
    store.save_progress(&progress).unwrap();
    let fetcher = Arc::new(VodFetcher::default());
    let mut a = main_app_at(&dir);
    a.fetcher = fetcher.clone();
    a.engine = Some(Box::new(MockEngine::default()));
    (a, fetcher)
}

fn load_catalogs(a: &mut App) {
    a.start_movie_load(LoadOrigin::Network);
    a.start_series_load(LoadOrigin::Network);
    wait_for(a, |a| {
        a.movie_slot.loaded.is_some() && a.series_slot.loaded.is_some()
    });
}

fn tile_titles(a: &App, row: MainRow) -> Vec<String> {
    a.main_row_tiles(row).into_iter().map(|t| t.title).collect()
}

#[test]
fn main_rows_are_hidden_while_empty() {
    let a = main_app("main_rows_empty");
    assert!(a.main_rows().is_empty());
    let mut a = a;
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    let output = ctx.run(compact_input(vec![]), |ctx| {
        skjalf::views::main_menu::show(ctx, &mut a)
    });
    let mut labels = Vec::new();
    for shape in &output.shapes {
        shape_texts(&shape.shape, &mut labels);
    }
    assert!(!labels
        .iter()
        .any(|text| text.contains("Watching") || text.contains("Latest")));
    a.handle_action(Action::Down, 0);
    assert_eq!(a.main_row, None);
}

#[test]
fn rows_appear_when_catalogs_arrive_without_moving_the_selection() {
    let (mut a, _, _) = movies_app("main_rows_arrive");
    press(&mut a, &[Action::Right, Action::Right, Action::Right]);
    assert_eq!(a.main_focus, MainButton::Settings);
    assert!(a.main_rows().is_empty());
    load_catalogs(&mut a);
    assert_eq!(
        a.main_rows(),
        [MainRow::LatestMovies, MainRow::LatestSeries]
    );
    assert_eq!(a.main_focus, MainButton::Settings, "selection stays put");
    assert_eq!(a.main_row, None);
    assert_eq!(a.router.current(), View::Main);
}

#[test]
fn latest_rows_are_newest_first_and_skip_undated_items() {
    let (mut a, _, _) = movies_app("main_latest_order");
    load_catalogs(&mut a);
    assert_eq!(
        tile_titles(&a, MainRow::LatestMovies),
        ["Movie B", "Other D", "Movie A"]
    );
    assert_eq!(tile_titles(&a, MainRow::LatestSeries), ["Show B", "Show A"]);
}

#[test]
fn latest_rows_hold_at_most_twelve_items() {
    let dir = seeded_dir("main_latest_cap");
    Store::open(&dir)
        .unwrap()
        .save_movies(&MovieCatalog {
            source_id: source().id,
            groups: vec![],
            movies: (0..50)
                .map(|n| Movie {
                    id: n.to_string(),
                    name: format!("Film {n}"),
                    group_id: "x".into(),
                    icon: None,
                    added: Some(n),
                    rating: None,
                    extension: "mkv".into(),
                })
                .collect(),
        })
        .unwrap();
    let mut a = main_app_at(&dir);
    wait_for(&mut a, |a| a.movie_slot.loaded.is_some());
    let titles = tile_titles(&a, MainRow::LatestMovies);
    assert_eq!(titles.len(), 12);
    assert_eq!(titles[0], "Film 49");
    assert_eq!(titles[11], "Film 38");
}

#[test]
fn continue_watching_is_newest_first_and_skips_finished_and_short_items() {
    let (a, _) = main_with_content("main_continue_order");
    assert_eq!(a.main_rows(), [MainRow::Continue]);
    assert_eq!(
        tile_titles(&a, MainRow::Continue),
        ["Show B  E2  Two B", "Other D", "Movie A"]
    );
    let tiles = a.main_row_tiles(MainRow::Continue);
    assert_eq!(
        tiles[1].marker,
        skjalf_core::progress::Marker::InProgress {
            position_secs: 200.0,
            fraction: Some(200.0 / 5400.0),
            left_secs: Some(5200.0),
        }
    );
}

#[test]
fn main_row_navigation_and_selection_restore() {
    let (mut a, _) = main_with_content("main_row_nav");
    load_catalogs(&mut a);
    assert_eq!(
        a.main_rows(),
        [
            MainRow::Continue,
            MainRow::LatestMovies,
            MainRow::LatestSeries
        ]
    );
    a.handle_action(Action::Down, 0);
    assert_eq!(a.main_row, Some(MainRow::Continue));
    a.handle_action(Action::Right, 0);
    a.handle_action(Action::Right, 0);
    a.handle_action(Action::Right, 0);
    assert_eq!(
        a.main_row_selection(MainRow::Continue),
        2,
        "clamped at the end"
    );
    a.handle_action(Action::Down, 0);
    assert_eq!(a.main_row, Some(MainRow::LatestMovies));
    a.handle_action(Action::Right, 0);
    a.handle_action(Action::Down, 0);
    a.handle_action(Action::Down, 0);
    assert_eq!(
        a.main_row,
        Some(MainRow::LatestSeries),
        "no wrap past the last row"
    );
    a.handle_action(Action::Up, 0);
    a.handle_action(Action::Up, 0);
    a.handle_action(Action::Up, 0);
    assert_eq!(
        a.main_row, None,
        "Up from the first row returns to the buttons"
    );
    a.handle_action(Action::Down, 0);
    assert_eq!(a.main_row, Some(MainRow::Continue));
    assert_eq!(
        a.main_row_selection(MainRow::Continue),
        2,
        "row selection kept"
    );
    assert_eq!(a.main_row_selection(MainRow::LatestMovies), 1);
}

#[test]
fn continue_watching_movie_opens_details_and_back_returns_to_main() {
    let (mut a, _) = main_with_content("main_open_movie");
    load_catalogs(&mut a);
    a.handle_action(Action::Down, 0);
    a.handle_action(Action::Right, 0);
    a.handle_action(Action::Select, 0); // "Other D"
    assert_eq!(a.router.current(), View::MovieDetails);
    assert_eq!(a.movie_details.as_ref().unwrap().movie.id, "1004");
    assert_eq!(a.movie_focus(), MovieButton::Resume);
    a.handle_action(Action::Back, 0);
    assert_eq!(a.router.current(), View::Main);
    assert_eq!(a.main_row, Some(MainRow::Continue));
    assert_eq!(a.main_row_selection(MainRow::Continue), 1);
}

#[test]
fn continue_watching_episode_opens_its_series_at_that_episode() {
    let (mut a, _) = main_with_content("main_open_episode");
    load_catalogs(&mut a);
    a.handle_action(Action::Down, 0);
    a.handle_action(Action::Select, 0); // newest: the episode
    assert_eq!(a.router.current(), View::SeriesDetails);
    assert_eq!(a.series_details.as_ref().unwrap().series.id, "2002");
    wait_for(&mut a, |a| {
        a.series_details.as_ref().unwrap().info != DetailInfo::Loading
    });
    assert_eq!(series_selection(&a), (1, 1, SeriesFocus::Episodes));
    assert_eq!(
        a.series_details.as_ref().unwrap().episodes()[1].title,
        "Two B"
    );
    a.handle_action(Action::Back, 0);
    assert_eq!(a.router.current(), View::Main);
}

#[test]
fn continue_watching_episode_works_before_the_series_catalog_loads() {
    let (mut a, _) = main_with_content("main_open_episode_early");
    a.handle_action(Action::Down, 0);
    a.handle_action(Action::Select, 0);
    assert_eq!(a.router.current(), View::SeriesDetails);
    assert_eq!(a.series_details.as_ref().unwrap().series.name, "Show B");
    wait_for(&mut a, |a| {
        a.series_details.as_ref().unwrap().info != DetailInfo::Loading
    });
    assert_eq!(series_selection(&a).0, 1);
}

#[test]
fn latest_tiles_open_movie_and_series_details() {
    let (mut a, _, _) = movies_app("main_open_latest");
    load_catalogs(&mut a);
    a.handle_action(Action::Down, 0);
    assert_eq!(a.main_row, Some(MainRow::LatestMovies));
    a.handle_action(Action::Right, 0);
    a.handle_action(Action::Select, 0);
    assert_eq!(a.movie_details.as_ref().unwrap().movie.id, "1004");
    a.handle_action(Action::Back, 0);
    assert_eq!(a.router.current(), View::Main);
    a.handle_action(Action::Down, 0);
    a.handle_action(Action::Select, 0);
    assert_eq!(a.router.current(), View::SeriesDetails);
    assert_eq!(a.series_details.as_ref().unwrap().series.id, "2002");
    a.handle_action(Action::Back, 0);
    assert_eq!(a.router.current(), View::Main);
    assert_eq!(a.main_row, Some(MainRow::LatestSeries));
}

#[test]
fn touching_a_row_tile_opens_it_and_rows_render() {
    let (mut a, _) = main_with_content("main_row_touch");
    load_catalogs(&mut a);
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    let draw = |ctx: &egui::Context, a: &mut App, events: Vec<egui::Event>| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1024.0, 640.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| skjalf::views::main_menu::show(ctx, a),
        )
    };
    let mut output = draw(&ctx, &mut a, vec![]);
    for _ in 0..3 {
        output = draw(&ctx, &mut a, vec![]);
    }
    let mut labels = Vec::new();
    for shape in &output.shapes {
        shape_texts(&shape.shape, &mut labels);
    }
    for expected in [
        "Continue Watching",
        "Latest Movies",
        "Latest Series",
        "Live TV",
    ] {
        assert!(
            labels.iter().any(|text| text == expected),
            "{expected}: {labels:?}"
        );
    }
    let mut at = None;
    for clipped in &output.shapes {
        find_text_pos(&clipped.shape, "Other D", &mut at);
    }
    let pos = at.expect("tile drawn") + egui::vec2(8.0, 8.0);
    let button = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    draw(
        &ctx,
        &mut a,
        vec![egui::Event::PointerMoved(pos), button(true)],
    );
    draw(&ctx, &mut a, vec![button(false)]);
    assert_eq!(a.router.current(), View::MovieDetails);
    assert_eq!(a.movie_details.as_ref().unwrap().movie.id, "1004");
}

// -- Watched and in-progress markers ------------------------------------------------

use skjalf_core::progress::Marker;

fn render_labels(
    a: &mut App,
    show: impl Fn(&egui::Context, &mut App),
) -> (Vec<String>, egui::FullOutput) {
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    let mut output = None;
    for _ in 0..3 {
        output = Some(ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1024.0, 640.0),
                )),
                ..Default::default()
            },
            |ctx| show(ctx, a),
        ));
    }
    let output = output.unwrap();
    let mut labels = Vec::new();
    for shape in &output.shapes {
        shape_texts(&shape.shape, &mut labels);
    }
    (labels, output)
}

fn count_rects_with_height(shape: &egui::Shape, height: f32, found: &mut usize) {
    match shape {
        egui::Shape::Rect(rect) if (rect.rect.height() - height).abs() < 0.01 => *found += 1,
        egui::Shape::Vec(shapes) => shapes
            .iter()
            .for_each(|shape| count_rects_with_height(shape, height, found)),
        _ => {}
    }
}

fn text_color(shape: &egui::Shape, label: &str, out: &mut Option<egui::Color32>) {
    match shape {
        egui::Shape::Text(text) if text.galley.text() == label => {
            *out = text.galley.job.sections.first().map(|s| s.format.color)
        }
        egui::Shape::Vec(shapes) => shapes
            .iter()
            .for_each(|shape| text_color(shape, label, out)),
        _ => {}
    }
}

#[test]
fn movie_rows_mark_watched_and_in_progress_items() {
    let (mut a, _) = main_with_content("marker_rows");
    load_catalogs(&mut a);
    a.activate_main_button(MainButton::Movies);
    a.set_category(BrowserId::Movies, CategorySel::All);
    let (labels, output) = render_labels(&mut a, skjalf::views::movies::show);
    assert!(labels.iter().any(|text| text == "Watched"), "{labels:?}");
    assert!(
        labels.iter().any(|text| text.ends_with(" left")),
        "{labels:?}"
    );
    assert!(labels
        .iter()
        .any(|text| text == "1:26 left" || text == "28:20 left" || text.contains("left")));
    let mut dim = None;
    let mut normal = None;
    for clipped in &output.shapes {
        text_color(&clipped.shape, "Movie B", &mut dim);
        text_color(&clipped.shape, "Movie C", &mut normal);
    }
    let dim = dim.expect("watched row text");
    let normal = normal.expect("plain row text");
    assert!(
        dim.a() < normal.a() || dim.r() < normal.r(),
        "{dim:?} vs {normal:?}"
    );
    let mut bars = 0;
    for clipped in &output.shapes {
        count_rects_with_height(&clipped.shape, 3.0, &mut bars);
    }
    assert!(bars >= 2, "a thin progress bar per in-progress row: {bars}");
}

#[test]
fn selected_marked_rows_keep_dark_text_and_a_visible_pill() {
    let (mut a, _) = main_with_content("marker_selected");
    load_catalogs(&mut a);
    a.activate_main_button(MainButton::Movies);
    a.set_category(BrowserId::Movies, CategorySel::All);
    a.handle_action(Action::Down, 0); // Movie B, finished
    let (labels, output) = render_labels(&mut a, skjalf::views::movies::show);
    assert!(labels.iter().any(|text| text == "Watched"));
    let mut color = None;
    for clipped in &output.shapes {
        text_color(&clipped.shape, "Movie B", &mut color);
    }
    assert_eq!(color, Some(egui::Color32::BLACK));
}

#[test]
fn series_rows_mark_in_progress_then_watched_once_the_total_is_known() {
    let (mut a, _, engine) = movies_app("marker_series");
    open_series(&mut a);
    let (labels, _) = render_labels(&mut a, skjalf::views::series::show);
    assert!(!labels.iter().any(|t| t == "Watched" || t == "In progress"));
    open_series_details(&mut a);
    assert_eq!(
        a.progress.series_total(&source().id, "2001"),
        Some(4),
        "details store the episode total"
    );
    for (round, (season, episode)) in [(0, 0), (1, 0), (1, 1), (2, 0)].into_iter().enumerate() {
        let now = round as u64 * 10_000;
        a.select_season(season);
        a.click_episode(episode, now);
        engine.events.lock().unwrap().push(PlayerEvent::Ended);
        set_clock(&engine, 2699.0, 2700.0);
        a.poll_jobs(now + 1);
        a.handle_action(Action::Back, now + 2);
    }
    a.handle_action(Action::Back, 99_000);
    assert_eq!(a.markers.series("2001"), Marker::Watched);
    let (labels, _) = render_labels(&mut a, skjalf::views::series::show);
    assert!(labels.iter().any(|text| text == "Watched"), "{labels:?}");
}

#[test]
fn series_with_some_viewing_reads_in_progress_and_without_a_total_never_watched() {
    let (mut a, _, engine) = movies_app("marker_series_partial");
    open_series(&mut a);
    open_series_details(&mut a);
    a.handle_action(Action::Select, 0);
    engine.events.lock().unwrap().push(PlayerEvent::Ended);
    set_clock(&engine, 2699.0, 2700.0);
    a.poll_jobs(1);
    a.handle_action(Action::Back, 2);
    a.handle_action(Action::Back, 3);
    assert!(matches!(
        a.markers.series("2001"),
        Marker::InProgress { .. }
    ));
    let (labels, _) = render_labels(&mut a, skjalf::views::series::show);
    assert!(
        labels.iter().any(|text| text == "In progress"),
        "{labels:?}"
    );

    let (mut b, _, engine) = movies_app("marker_series_unknown");
    open_series(&mut b);
    open_series_details(&mut b);
    b.progress = Default::default();
    b.handle_action(Action::Select, 0);
    set_clock(&engine, 900.0, 2700.0);
    b.poll_jobs(1);
    b.handle_action(Action::Back, 2);
    assert!(matches!(
        b.markers.series("2001"),
        Marker::InProgress { .. }
    ));
}

#[test]
fn marker_index_is_rebuilt_after_a_save_and_survives_a_restart() {
    let tag = "marker_restart";
    let (mut a, _, engine) = playing_movie(tag);
    assert_eq!(a.markers.movie("1001"), Marker::None);
    set_clock(&engine, 1800.0, 5400.0);
    a.handle_action(Action::Back, 2_000);
    assert!(matches!(a.markers.movie("1001"), Marker::InProgress { .. }));
    a.handle_action(Action::Select, 3_000);
    engine.events.lock().unwrap().push(PlayerEvent::Ended);
    set_clock(&engine, 5399.0, 5400.0);
    a.poll_jobs(4_000);
    assert_eq!(a.markers.movie("1001"), Marker::Watched);
    drop(a);
    let b = main_app_at(&dir_of(tag));
    assert_eq!(b.markers.movie("1001"), Marker::Watched);
}

#[test]
fn movie_rows_keep_their_height_with_markers() {
    let (mut a, _) = main_with_content("marker_height");
    load_catalogs(&mut a);
    a.activate_main_button(MainButton::Movies);
    a.set_category(BrowserId::Movies, CategorySel::All);
    let (_, output) = render_labels(&mut a, skjalf::views::movies::show);
    let mut rows = 0;
    for clipped in &output.shapes {
        count_rects_with_height(&clipped.shape, 44.0, &mut rows);
    }
    assert!(rows >= 4, "every row is 44 points tall: {rows}");
}

#[test]
fn volume_icon_changes_volume_with_up_and_down_and_left_right_keep_moving() {
    let (mut a, _, engine) = playing_movie("c5_volume_control");
    a.handle_action(Action::Down, 2_000);
    press(&mut a, &[Action::Right, Action::Right]);
    assert_eq!(a.player_focus, PlayerFocus::Controls(4));
    a.handle_action(Action::Up, 2_000);
    assert_eq!(a.settings.volume, 85);
    assert_eq!(
        a.player_focus,
        PlayerFocus::Controls(4),
        "Up adjusts, not leaves"
    );
    a.handle_action(Action::Down, 2_000);
    a.handle_action(Action::Down, 2_000);
    assert_eq!(a.settings.volume, 75);
    for _ in 0..8 {
        a.handle_action(Action::Up, 2_000);
    }
    assert_eq!(a.settings.volume, 100, "clamped at 100");
    for _ in 0..30 {
        a.handle_action(Action::Down, 2_000);
    }
    assert_eq!(a.settings.volume, 0, "clamped at 0");
    for _ in 0..16 {
        a.handle_action(Action::Up, 2_000);
    }
    assert_eq!(a.settings.volume, 80);
    assert_eq!(a.player_focus, PlayerFocus::Controls(4));

    a.handle_action(Action::Left, 2_000);
    assert_eq!(
        a.player_focus,
        PlayerFocus::Controls(3),
        "Left moves between controls"
    );
    assert_eq!(a.settings.volume, 80);
    a.handle_action(Action::Right, 2_000);
    a.handle_action(Action::Right, 2_000);
    assert_eq!(
        a.player_focus,
        PlayerFocus::Controls(0),
        "Right wraps to Back"
    );
    assert_eq!(a.settings.volume, 80);
    a.player_focus = PlayerFocus::Controls(4);

    a.handle_action(Action::Select, 2_000);
    assert!(a.muted);
    assert_eq!(engine.volumes.lock().unwrap().last(), Some(&0));
    assert_eq!(a.settings.volume, 80, "mute keeps the saved volume");
    a.handle_action(Action::Select, 2_000);
    assert!(!a.muted);
    assert_eq!(engine.volumes.lock().unwrap().last(), Some(&80));
    a.handle_action(Action::Select, 2_000);
    a.handle_action(Action::Up, 2_000);
    assert!(!a.muted, "changing the volume unmutes");
    assert_eq!(a.settings.volume, 85);
    a.handle_action(Action::Back, 2_000);
    assert_eq!(a.player_focus, PlayerFocus::Video, "Back returns to video");
    assert_eq!(a.router.current(), View::Player);
}

#[test]
fn up_on_other_bar_controls_still_returns_to_video() {
    let (mut a, _, _) = playing_movie("c5_up_returns");
    a.handle_action(Action::Down, 2_000);
    a.handle_action(Action::Right, 2_000); // Forward
    a.handle_action(Action::Up, 2_000);
    assert_eq!(a.player_focus, PlayerFocus::Video);
    assert_eq!(a.settings.volume, 80);
    a.handle_action(Action::Down, 2_000);
    a.handle_action(Action::Down, 2_000);
    assert_eq!(a.player_focus, PlayerFocus::Seek);
}

#[test]
fn mute_applies_to_the_next_item_and_draws_a_muted_icon() {
    let (mut a, _, engine) = playing_movie("c5_mute_persist");
    a.toggle_mute(2_000);
    a.handle_action(Action::Back, 3_000);
    a.handle_action(Action::Select, 4_000);
    assert_eq!(engine.volumes.lock().unwrap().last(), Some(&0));
    let (labels, _) = render_labels(&mut a, |ctx, app| {
        skjalf::views::player::show(ctx, app, 4_000)
    });
    assert!(labels.iter().any(|text| text == "Muted"), "{labels:?}");
    assert!(labels.iter().all(|text| !text.contains('\u{fffd}')));
}

#[test]
fn touching_the_volume_icon_toggles_mute() {
    let (mut a, _, _) = playing_movie("c5_volume_touch");
    a.player_focus = PlayerFocus::Controls(2);
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    let draw = |ctx: &egui::Context, a: &mut App, events: Vec<egui::Event>| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| skjalf::views::player::show(ctx, a, 2_000),
        )
    };
    for _ in 0..5 {
        draw(&ctx, &mut a, vec![]);
    }
    let pos = ctx
        .read_response(skjalf::views::transport_bar::control_id(BarControl::Volume))
        .unwrap()
        .rect
        .center();
    let button = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    draw(
        &ctx,
        &mut a,
        vec![egui::Event::PointerMoved(pos), button(true)],
    );
    draw(&ctx, &mut a, vec![button(false)]);
    assert!(a.muted);
}

#[test]
fn replaying_a_watched_movie_briefly_keeps_it_watched() {
    let (mut a, _, engine) = playing_movie("c5_short_replay");
    engine.events.lock().unwrap().push(PlayerEvent::Ended);
    set_clock(&engine, 5399.0, 5400.0);
    a.poll_jobs(2_000);
    a.handle_action(Action::Back, 3_000);
    assert_eq!(a.markers.movie("1001"), Marker::Watched);
    a.handle_action(Action::Select, 4_000); // Play again from the start
    set_clock(&engine, 12.0, 5400.0);
    a.handle_action(Action::Back, 5_000);
    assert_eq!(
        a.markers.movie("1001"),
        Marker::Watched,
        "12 s keeps Watched"
    );
    assert!(a.continue_items.is_empty());
    a.handle_action(Action::Select, 6_000);
    set_clock(&engine, 45.0, 5400.0);
    a.handle_action(Action::Back, 7_000);
    assert!(matches!(a.markers.movie("1001"), Marker::InProgress { .. }));
    assert_eq!(a.continue_items.len(), 1);
    a.handle_action(Action::Select, 8_000);
    engine.events.lock().unwrap().push(PlayerEvent::Ended);
    set_clock(&engine, 5399.0, 5400.0);
    a.poll_jobs(9_000);
    assert_eq!(a.markers.movie("1001"), Marker::Watched, "finishing again");
}

// -- Sorted category sidebar ---------------------------------------------------------

fn named_groups(names: &[&str]) -> Vec<Group> {
    names
        .iter()
        .enumerate()
        .map(|(n, name)| Group {
            id: format!("g{n}"),
            name: (*name).to_owned(),
        })
        .collect()
}

fn sidebar(a: &App, id: BrowserId) -> Vec<String> {
    (0..a.categories_len(id))
        .map(|row| a.category_label(id, row).unwrap())
        .collect()
}

#[test]
fn live_sort_orders_the_sidebar_and_keeps_pinned_entries_and_selection() {
    let mut a = app("sidebar_live");
    let mut pl = playlist();
    pl.groups = named_groups(&["Zeta", "alpha", "Mid", "beta"]);
    for (channel, group) in pl.channels.iter_mut().zip(["g0", "g1", "g2", "g3", "g0"]) {
        channel.group_id = group.into();
    }
    a.set_playlist(pl);
    a.set_category(BrowserId::Live, group("g0"));
    let provider = sidebar(&a, BrowserId::Live);
    assert!(provider[0].contains("Favorites"));
    assert_eq!(provider[1..], ["All", "Zeta", "alpha", "Mid", "beta"]);
    assert_eq!(a.category_row(BrowserId::Live), 2);

    a.toggle_sort(BrowserId::Live);
    let sorted = sidebar(&a, BrowserId::Live);
    assert!(sorted[0].contains("Favorites"), "Favorites stays first");
    assert_eq!(sorted[1..], ["All", "alpha", "beta", "Mid", "Zeta"]);
    assert_eq!(a.live.category, group("g0"), "selected category kept");
    assert_eq!(a.category_row(BrowserId::Live), 5);
    assert_eq!(
        a.category_at(BrowserId::Live, 5),
        Some(group("g0")),
        "rows map back to the same group"
    );

    let (_, _) = render_labels(&mut a, skjalf::views::live::show);
    assert_eq!(
        a.last_browser_sel[0].map(|sel| sel.0),
        Some(5),
        "scrolls to the moved row"
    );

    a.live.focus = BrowserFocus::Categories;
    a.handle_action(Action::Down, 0);
    assert_eq!(
        a.category_row(BrowserId::Live),
        6 % a.categories_len(BrowserId::Live)
    );
    a.toggle_sort(BrowserId::Live);
    assert_eq!(
        sidebar(&a, BrowserId::Live)[2..],
        ["Zeta", "alpha", "Mid", "beta"]
    );
}

#[test]
fn group_ties_keep_provider_order() {
    let mut a = app("sidebar_ties");
    let mut pl = playlist();
    pl.groups = named_groups(&["b", "B", "a"]);
    a.set_playlist(pl);
    a.toggle_sort(BrowserId::Live);
    assert_eq!(sidebar(&a, BrowserId::Live)[2..], ["a", "b", "B"]);
}

#[test]
fn sorted_sidebar_persists_across_restart() {
    let dir = seeded_dir("sidebar_restart");
    {
        let mut a = app_at(&dir);
        let mut pl = playlist();
        pl.groups = named_groups(&["Zeta", "Alpha"]);
        a.set_playlist(pl.clone());
        a.toggle_sort(BrowserId::Live);
        Store::open(&dir).unwrap().save_playlist(&pl).unwrap();
    }
    let b = main_app_at(&dir);
    assert_eq!(b.live.sort, SortMode::Name);
    assert_eq!(sidebar(&b, BrowserId::Live)[2..], ["Alpha", "Zeta"]);
}

#[test]
fn movies_and_series_sidebars_sort_by_name_too() {
    let dir = seeded_dir("sidebar_catalogs");
    let store = Store::open(&dir).unwrap();
    store
        .save_movies(&MovieCatalog {
            source_id: source().id,
            groups: named_groups(&["Zulu", "alpha", "Mike"]),
            movies: vec![],
        })
        .unwrap();
    store
        .save_series(&skjalf_core::catalog::SeriesCatalog {
            source_id: source().id,
            groups: named_groups(&["Yankee", "Bravo"]),
            series: vec![],
        })
        .unwrap();
    let mut a = main_app_at(&dir);
    wait_for(&mut a, |a| {
        a.movie_slot.loaded.is_some() && a.series_slot.loaded.is_some()
    });
    assert_eq!(
        sidebar(&a, BrowserId::Movies)[2..],
        ["Zulu", "alpha", "Mike"]
    );
    a.set_category(BrowserId::Movies, group_sel("g2"));
    a.toggle_sort(BrowserId::Movies);
    assert_eq!(
        sidebar(&a, BrowserId::Movies)[2..],
        ["alpha", "Mike", "Zulu"]
    );
    assert_eq!(a.movies.category, group_sel("g2"), "selection kept");
    assert_eq!(a.category_row(BrowserId::Movies), 3);
    assert_eq!(
        sidebar(&a, BrowserId::Series)[2..],
        ["Yankee", "Bravo"],
        "per browser"
    );
    a.toggle_sort(BrowserId::Series);
    assert_eq!(sidebar(&a, BrowserId::Series)[2..], ["Bravo", "Yankee"]);
}

// -- Group filter and page jumps ------------------------------------------------------

fn filter_app(tag: &str) -> App {
    let mut a = app(tag);
    let mut pl = playlist();
    pl.groups = named_groups(&[
        "SI| Sport",
        "si| News",
        "SI| Kids",
        "UK| BBC",
        "DE| ARD",
        "Si| Film",
        "Other",
    ]);
    a.set_playlist(pl);
    a.live.focus = BrowserFocus::Categories;
    a
}

fn group_names(a: &App) -> Vec<String> {
    sidebar(a, BrowserId::Live)[2..].to_vec()
}

fn type_group_filter(a: &mut App, ctx: &egui::Context, text: &str) {
    type_text(a, ctx, text);
}

#[test]
fn group_filter_matches_case_insensitively_and_keeps_pinned_entries() {
    let mut a = filter_app("gf_match");
    assert_eq!(group_names(&a).len(), 7);
    a.set_group_filter(BrowserId::Live, "SI|".into());
    assert_eq!(
        group_names(&a),
        ["SI| Sport", "si| News", "SI| Kids", "Si| Film"]
    );
    let labels = sidebar(&a, BrowserId::Live);
    assert!(labels[0].contains("Favorites"));
    assert_eq!(labels[1], "All", "pinned entries stay above the matches");
    a.set_group_filter(BrowserId::Live, "  si| k ".into());
    assert_eq!(group_names(&a), ["SI| Kids"]);
    a.set_group_filter(BrowserId::Live, "zzz".into());
    assert!(group_names(&a).is_empty());
    assert_eq!(a.categories_len(BrowserId::Live), 2);
    a.set_group_filter(BrowserId::Live, String::new());
    assert_eq!(group_names(&a).len(), 7, "clearing restores the list");
}

#[test]
fn group_filter_combines_with_the_sort_mode_and_keeps_the_selection() {
    let mut a = filter_app("gf_sort");
    a.set_category(BrowserId::Live, group("g2")); // SI| Kids
    a.set_group_filter(BrowserId::Live, "si|".into());
    assert_eq!(a.category_title(BrowserId::Live), "SI| Kids");
    a.toggle_sort(BrowserId::Live);
    assert_eq!(
        group_names(&a),
        ["Si| Film", "SI| Kids", "si| News", "SI| Sport"]
    );
    assert_eq!(a.live.category, group("g2"));
    assert_eq!(a.category_row(BrowserId::Live), 3);
    a.set_group_filter(BrowserId::Live, "uk".into());
    assert_eq!(group_names(&a), ["UK| BBC"]);
    assert_eq!(
        a.live.category,
        group("g2"),
        "hidden selection stays active"
    );
    assert_eq!(a.category_title(BrowserId::Live), "SI| Kids");
    assert_eq!(
        a.category_row(BrowserId::Live),
        a.categories_len(BrowserId::Live)
    );
    a.live.focus = BrowserFocus::Categories;
    a.handle_action(Action::Down, 0);
    assert_eq!(
        a.live.category,
        group("g3"),
        "navigation resumes at the first match"
    );
}

#[test]
fn group_list_is_cached_until_the_filter_catalog_or_sort_changes() {
    let mut a = filter_app("gf_cache");
    let builds = a.group_list_builds(BrowserId::Live);
    for _ in 0..5 {
        a.live_refresh();
    }
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    live_frame(&ctx, &mut a, vec![]);
    live_frame(&ctx, &mut a, vec![]);
    assert_eq!(
        a.group_list_builds(BrowserId::Live),
        builds,
        "no rebuild per paint"
    );
    a.set_group_filter(BrowserId::Live, "si".into());
    assert_eq!(a.group_list_builds(BrowserId::Live), builds + 1);
    a.set_group_filter(BrowserId::Live, "SI ".into());
    assert_eq!(
        a.group_list_builds(BrowserId::Live),
        builds + 1,
        "same normalized text"
    );
    a.toggle_sort(BrowserId::Live);
    assert_eq!(a.group_list_builds(BrowserId::Live), builds + 2);
}

#[test]
fn group_filter_focus_path_types_and_lands_on_the_first_match() {
    let mut a = filter_app("gf_focus");
    a.set_category(BrowserId::Live, CategorySel::Favorites);
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    a.handle_action(Action::Up, 0);
    assert_eq!(a.live.focus, BrowserFocus::GroupFilter);
    a.handle_action(Action::Select, 0);
    assert!(a.group_filter_editing());
    live_frame(&ctx, &mut a, vec![]);
    type_group_filter(&mut a, &ctx, "ui");
    assert_eq!(a.live.group_filter, "ui");
    a.live.group_filter.clear();
    a.set_group_filter(BrowserId::Live, String::new());
    type_group_filter(&mut a, &ctx, "f");
    type_group_filter(&mut a, &ctx, "1");
    type_group_filter(&mut a, &ctx, "q");
    assert_eq!(
        a.live.group_filter, "f1q",
        "shortcut keys type into the field"
    );
    assert_eq!(a.favorites.iter().count(), 0);
    assert!(a.zap.digits().is_empty());
    assert!(!a.wants_exit());
    a.set_group_filter(BrowserId::Live, String::new());
    type_group_filter(&mut a, &ctx, "si|");
    assert_eq!(a.live.group_filter, "si|");
    live_frame(&ctx, &mut a, vec![key_event(egui::Key::Enter)]);
    assert!(!a.group_filter_editing());
    assert_eq!(a.live.focus, BrowserFocus::Categories);
    assert_eq!(a.live.category, group("g0"), "first match is selected");
    assert_eq!(ctx.memory(|m| m.focused()), None);
    assert_eq!(a.router.current(), View::Live);
}

#[test]
fn group_filter_edit_ends_with_escape_and_back_and_focus_moves_right_to_search() {
    for end in [egui::Key::Escape, egui::Key::BrowserBack] {
        let mut a = filter_app("gf_end");
        let ctx = egui::Context::default();
        skjalf::theme::apply(&ctx);
        a.live.focus = BrowserFocus::GroupFilter;
        a.handle_action(Action::Select, 0);
        live_frame(&ctx, &mut a, vec![]);
        type_group_filter(&mut a, &ctx, "uk");
        live_frame(&ctx, &mut a, vec![key_event(end)]);
        assert!(!a.group_filter_editing(), "{end:?}");
        assert_eq!(a.live.category, group("g3"));
    }
    let mut a = filter_app("gf_right");
    a.live.focus = BrowserFocus::GroupFilter;
    a.handle_action(Action::Right, 0);
    assert_eq!(a.live.focus, BrowserFocus::GroupFilterClear);
    a.handle_action(Action::Right, 0);
    assert_eq!(a.live.focus, BrowserFocus::Search);
    a.live.focus = BrowserFocus::GroupFilter;
    a.handle_action(Action::Down, 0);
    assert_eq!(a.live.focus, BrowserFocus::Categories);
    a.live.focus = BrowserFocus::GroupFilter;
    a.handle_action(Action::Back, 0);
    assert_eq!(a.live.focus, BrowserFocus::Items);
}

#[test]
fn native_keyboard_dismissal_ends_the_group_filter_edit() {
    struct Recorder(Mutex<Option<mpsc::Sender<()>>>);
    impl KeyboardLauncher for Recorder {
        fn send(&self, _uri: &'static str) -> mpsc::Receiver<Result<(), String>> {
            let (tx, rx) = mpsc::channel();
            tx.send(Ok(())).unwrap();
            rx
        }
        fn watch_dismissal(&self, _cancel: Arc<AtomicBool>) -> Option<mpsc::Receiver<()>> {
            let (tx, rx) = mpsc::channel();
            *self.0.lock().unwrap() = Some(tx);
            Some(rx)
        }
    }
    let mut a = filter_app("gf_native");
    let recorder = Arc::new(Recorder(Mutex::new(None)));
    a.keyboard_launcher = recorder.clone();
    a.game_mode = true;
    a.live.focus = BrowserFocus::GroupFilter;
    a.handle_action(Action::Select, 0);
    a.poll_jobs(0);
    a.set_group_filter(BrowserId::Live, "uk".into());
    recorder.0.lock().unwrap().take().unwrap().send(()).unwrap();
    a.poll_keyboard();
    assert!(!a.group_filter_editing());
    assert_eq!(a.live.focus, BrowserFocus::Categories);
    assert_eq!(a.live.category, group("g3"));
    a.handle_action(Action::Back, 1);
    assert_eq!(
        a.live.focus,
        BrowserFocus::Items,
        "no extra Back to leave the field"
    );
}

#[test]
fn group_filter_and_selection_survive_player_and_main_round_trips() {
    let mut a = filter_app("gf_round_trip");
    a.engine = Some(Box::new(MockEngine::default()));
    a.set_group_filter(BrowserId::Live, "si|".into());
    a.set_category(BrowserId::Live, group("g0"));
    a.live.focus = BrowserFocus::Items;
    a.handle_action(Action::Select, 0);
    if a.router.current() == View::Player {
        a.handle_action(Action::Back, 1);
    }
    a.handle_action(Action::Back, 2);
    a.handle_action(Action::Select, 3);
    assert_eq!(a.live.group_filter, "si|");
    assert_eq!(a.live.category, group("g0"));
    assert_eq!(group_names(&a).len(), 4);
}

#[test]
fn group_filter_field_stays_above_the_steam_keyboard_at_deck_scale() {
    let id = BrowserId::Live;
    let mut a = filter_app("gf_compact");
    a.game_mode = true;
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    a.live.focus = BrowserFocus::GroupFilter;
    a.handle_action(Action::Select, 0);
    live_frame(&ctx, &mut a, vec![]);
    live_frame(&ctx, &mut a, vec![]);
    let rect = ctx
        .read_response(skjalf::views::browser::group_filter_id(id))
        .unwrap()
        .rect;
    assert!(rect.max.y < 200.0, "{rect:?}");
    assert!(rect.width() > 100.0, "{rect:?}");
    assert_eq!(
        ctx.memory(|m| m.focused()),
        Some(skjalf::views::browser::group_filter_id(id))
    );
}

#[test]
fn touch_clear_button_restores_the_group_list() {
    let mut a = filter_app("gf_touch_clear");
    a.set_group_filter(BrowserId::Live, "uk".into());
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    click_text(&ctx, &mut a, "X", skjalf::views::live::show);
    assert!(a.live.group_filter.is_empty());
    assert_eq!(group_names(&a).len(), 7);
}

#[test]
fn page_jumps_move_a_visible_page_without_wrapping() {
    let mut a = app("page_items");
    let mut pl = playlist();
    pl.channels = (0..100)
        .map(|n| Channel {
            id: n.to_string(),
            name: format!("Channel {n:03}"),
            num: Some(n),
            logo: None,
            group_id: "g1".into(),
            url: "/tmp/none".into(),
        })
        .collect();
    a.set_playlist(pl);
    a.set_category(BrowserId::Live, CategorySel::All);
    a.page_rows[BrowserId::Live as usize][1] = 12;
    a.handle_action(Action::ChUp, 0);
    assert_eq!(a.live.selected, 0, "no wrap at the top");
    a.handle_action(Action::ChDown, 0);
    assert_eq!(a.live.selected, 12);
    a.handle_action(Action::ChDown, 0);
    assert_eq!(a.live.selected, 24);
    a.handle_action(Action::ChUp, 0);
    assert_eq!(a.live.selected, 12);
    for _ in 0..20 {
        a.handle_action(Action::ChDown, 0);
    }
    assert_eq!(a.live.selected, 99, "clamped at the bottom");
    a.handle_action(Action::ChDown, 0);
    assert_eq!(a.live.selected, 99);
    a.handle_repeat(Action::ChDown);
    assert_eq!(a.live.selected, 99, "held repeat only drives Up and Down");
}

#[test]
fn page_jumps_move_through_the_sidebar_and_clamp() {
    let mut a = filter_app("page_sidebar");
    a.page_rows[BrowserId::Live as usize][0] = 3;
    a.set_category(BrowserId::Live, CategorySel::Favorites);
    a.live.focus = BrowserFocus::Categories;
    a.handle_action(Action::ChDown, 0);
    assert_eq!(a.category_row(BrowserId::Live), 3);
    a.handle_action(Action::ChDown, 0);
    a.handle_action(Action::ChDown, 0);
    a.handle_action(Action::ChDown, 0);
    assert_eq!(a.category_row(BrowserId::Live), 8, "last of 9 rows");
    a.handle_action(Action::ChUp, 0);
    assert_eq!(a.category_row(BrowserId::Live), 5);
    for _ in 0..5 {
        a.handle_action(Action::ChUp, 0);
    }
    assert_eq!(a.category_row(BrowserId::Live), 0, "clamped, never wraps");
}

#[test]
fn player_keeps_shoulder_buttons_for_channel_stepping() {
    let mut a = app("page_player");
    a.engine = Some(Box::new(MockEngine::default()));
    a.handle_action(Action::Select, 0);
    a.handle_action(Action::ChUp, 0);
    assert_eq!(a.player.channel_id.as_deref(), Some("2"));
}

#[test]
fn group_filter_works_for_movies_and_series_sidebars() {
    let dir = seeded_dir("gf_catalogs");
    let store = Store::open(&dir).unwrap();
    store
        .save_movies(&MovieCatalog {
            source_id: source().id,
            groups: named_groups(&["SI| Films", "UK| Films", "si| Docs"]),
            movies: vec![],
        })
        .unwrap();
    store
        .save_series(&skjalf_core::catalog::SeriesCatalog {
            source_id: source().id,
            groups: named_groups(&["DE| Serien", "SI| Serije"]),
            series: vec![],
        })
        .unwrap();
    let mut a = main_app_at(&dir);
    wait_for(&mut a, |a| {
        a.movie_slot.loaded.is_some() && a.series_slot.loaded.is_some()
    });
    a.set_group_filter(BrowserId::Movies, "si|".into());
    assert_eq!(
        sidebar(&a, BrowserId::Movies)[2..],
        ["SI| Films", "si| Docs"]
    );
    a.set_group_filter(BrowserId::Series, "serije".into());
    assert_eq!(sidebar(&a, BrowserId::Series)[2..], ["SI| Serije"]);
    assert_eq!(
        sidebar(&a, BrowserId::Movies)[2..].len(),
        2,
        "filters are per browser"
    );
}

// -- C5 review fixes -----------------------------------------------------------------

#[test]
fn seeking_back_after_the_end_saves_the_new_position() {
    let (mut a, _, engine) = playing_movie("c5_end_then_seek");
    engine.events.lock().unwrap().push(PlayerEvent::Ended);
    set_clock(&engine, 5399.0, 5400.0);
    a.poll_jobs(2_000);
    assert!(saved_progress(&a, "1001").unwrap().finished);

    a.seek_to_fraction(0.5, 3_000);
    set_clock(&engine, 2700.0, 5400.0);
    a.poll_jobs(14_000);
    let saved = saved_progress(&a, "1001").unwrap();
    assert_eq!(
        saved.position_secs, 2700.0,
        "periodic save resumes after the seek"
    );
    assert!(!saved.finished, "95 percent rule decides");
    set_clock(&engine, 2900.0, 5400.0);
    a.handle_action(Action::Back, 15_000); // seek bar focus back to video
    a.handle_action(Action::Back, 15_001);
    let saved = saved_progress(&a, "1001").unwrap();
    assert_eq!(saved.position_secs, 2900.0);
    assert_eq!(saved.resume_position(), Some(2900.0));
    assert_eq!(a.continue_items.len(), 1);
}

#[test]
fn unpausing_after_the_end_also_allows_saves() {
    let (mut a, _, engine) = playing_movie("c5_end_then_unpause");
    engine.events.lock().unwrap().push(PlayerEvent::Ended);
    set_clock(&engine, 5399.0, 5400.0);
    a.poll_jobs(2_000);
    a.handle_action(Action::Select, 3_000); // pause
    a.handle_action(Action::Select, 4_000); // play again
    set_clock(&engine, 400.0, 5400.0);
    a.handle_action(Action::Back, 5_000);
    assert_eq!(saved_progress(&a, "1001").unwrap().position_secs, 400.0);
}

#[test]
fn selected_primary_button_scrolls_back_into_view_at_deck_scale() {
    let (mut a, _) = main_with_content("main_scroll_back");
    load_catalogs(&mut a);
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 400.0));
    let clock = std::cell::Cell::new(0.0_f64);
    let frames = |ctx: &egui::Context, a: &mut App| {
        let mut output = None;
        for _ in 0..12 {
            clock.set(clock.get() + 0.1);
            output = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(clock.get()),
                    ..Default::default()
                },
                |ctx| skjalf::views::main_menu::show(ctx, a),
            ));
        }
        output.unwrap()
    };
    frames(&ctx, &mut a);
    for _ in 0..3 {
        a.handle_action(Action::Down, 0);
    }
    assert_eq!(a.main_row, Some(MainRow::LatestMovies));
    a.handle_action(Action::Down, 0);
    assert_eq!(a.main_row, Some(MainRow::LatestSeries));
    let output = frames(&ctx, &mut a);
    eprintln!(
        "DBG live {:?} series {:?}",
        text_positions(&output, "Live TV"),
        text_positions(&output, "Show B")
    );
    assert!(
        text_positions(&output, "Live TV").iter().all(|p| p.y < 0.0),
        "the buttons scrolled out of view while the last row is selected"
    );
    for _ in 0..3 {
        a.handle_action(Action::Up, 0);
    }
    assert_eq!(a.main_row, None);
    let output = frames(&ctx, &mut a);
    let live_tv = text_positions(&output, "Live TV");
    assert_eq!(live_tv.len(), 1);
    assert!(
        screen.contains(live_tv[0]),
        "selected button is on screen: {live_tv:?}"
    );
    a.handle_action(Action::Right, 0);
    a.handle_action(Action::Select, 0); // Movies browser, then back to Main
    a.handle_action(Action::Back, 0);
    let output = frames(&ctx, &mut a);
    assert!(screen.contains(text_positions(&output, "Movies")[0]));
}

// -- Overlay on Up, one-line status labels, speaker icon -----------------------------

fn render_player(
    ctx: &egui::Context,
    a: &mut App,
    now_ms: u64,
    size: egui::Vec2,
) -> egui::FullOutput {
    let mut output = None;
    for frame in 0..6 {
        output = Some(ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                time: Some(now_ms as f64 / 1000.0 + f64::from(frame) * 0.01),
                ..Default::default()
            },
            |ctx| skjalf::views::player::show(ctx, a, now_ms),
        ));
    }
    output.unwrap()
}

fn text_rows(shape: &egui::Shape, label: &str, out: &mut Option<usize>) {
    match shape {
        egui::Shape::Text(text) if text.galley.text() == label => {
            *out = Some(text.galley.rows.len())
        }
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| text_rows(shape, label, out)),
        _ => {}
    }
}

fn count_paths(shape: &egui::Shape, count: &mut usize) {
    match shape {
        egui::Shape::Path(_) => *count += 1,
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| count_paths(shape, count)),
        _ => {}
    }
}

#[test]
fn up_on_video_shows_the_bar_and_title_for_on_demand_playback() {
    let (mut a, _, _) = playing_movie("c5_up_overlay");
    a.player.hide_osd();
    a.player_focus = PlayerFocus::Video;
    a.paused = false;
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    let hidden = render_player(&ctx, &mut a, 20_000, egui::vec2(1280.0, 800.0));
    assert!(
        text_positions(&hidden, "Movie A").is_empty(),
        "nothing before Up"
    );
    let fresh = egui::Context::default();
    skjalf::theme::apply(&fresh);
    a.handle_action(Action::Up, 30_000);
    let output = render_player(&fresh, &mut a, 30_500, egui::vec2(1280.0, 800.0));
    for control in skjalf::app::bar_controls(true) {
        assert!(
            fresh
                .read_response(skjalf::views::transport_bar::control_id(*control))
                .is_some(),
            "{control:?} painted half a second after Up"
        );
    }
    assert_eq!(text_positions(&output, "Movie A").len(), 1, "title shown");
    let expired = egui::Context::default();
    skjalf::theme::apply(&expired);
    let output = render_player(&expired, &mut a, 34_500, egui::vec2(1280.0, 800.0));
    assert!(
        text_positions(&output, "Movie A").is_empty(),
        "hides after the timeout"
    );
}

#[test]
fn pause_shows_bar_title_and_a_single_line_label() {
    let (mut a, _, _) = playing_movie("c5_pause_label");
    a.handle_action(Action::Select, 30_000);
    assert!(a.paused);
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    let output = render_player(&ctx, &mut a, 30_400, egui::vec2(1280.0, 800.0));
    let mut rows = None;
    for clipped in &output.shapes {
        text_rows(&clipped.shape, "Paused", &mut rows);
    }
    assert_eq!(rows, Some(1), "Paused stays on one line");
    assert_eq!(text_positions(&output, "Movie A").len(), 1);
    assert!(ctx
        .read_response(skjalf::views::transport_bar::control_id(
            BarControl::PlayPause
        ))
        .is_some());
    let pos = text_positions(&output, "Paused")[0];
    assert!(pos.x > 900.0 && pos.y < 100.0, "top right: {pos:?}");
}

#[test]
fn status_labels_never_wrap_even_on_a_narrow_screen() {
    let (mut a, _, _) = playing_movie("c5_status_narrow");
    for (action, label) in [(Action::Right, "+10 s"), (Action::Select, "Paused")] {
        a.handle_action(action, 30_000);
        let ctx = egui::Context::default();
        skjalf::theme::apply(&ctx);
        let output = render_player(&ctx, &mut a, 30_100, egui::vec2(640.0, 400.0));
        let mut rows = None;
        for clipped in &output.shapes {
            text_rows(&clipped.shape, label, &mut rows);
        }
        assert_eq!(rows, Some(1), "{label}");
    }
}

#[test]
fn speaker_icon_has_sound_arcs_unless_muted() {
    let (mut a, _, _) = playing_movie("c5_speaker");
    a.handle_action(Action::Up, 30_000);
    let count = |a: &mut App| {
        let ctx = egui::Context::default();
        skjalf::theme::apply(&ctx);
        let output = render_player(&ctx, a, 30_100, egui::vec2(1280.0, 800.0));
        let mut paths = 0;
        for clipped in &output.shapes {
            count_paths(&clipped.shape, &mut paths);
        }
        paths
    };
    let sound = count(&mut a);
    a.toggle_mute(30_050);
    a.handle_action(Action::Up, 30_060);
    let muted = count(&mut a);
    assert_eq!(sound - muted, 2, "two sound arcs vanish when muted");
}

// -- Live overlay: one channel name, program in the bar, one-line notice ---------------

fn labels_containing(output: &egui::FullOutput, needle: &str) -> Vec<String> {
    let mut labels = Vec::new();
    for clipped in &output.shapes {
        shape_texts(&clipped.shape, &mut labels);
    }
    labels.retain(|text| text.contains(needle));
    labels
}

fn live_with_epg(tag: &str) -> (App, MockEngine) {
    let body = r#"{"epg_listings":[{"title":"TmV3cyBOb3c=","description":"","start":"2026-01-01 10:00:00","end":"2026-01-01 11:00:00"},{"title":"TmV4dCBTaG93","description":"","start":"2026-01-01 11:00:00","end":"2026-01-01 12:00:00"}]}"#;
    let mut a = app(tag);
    let engine = MockEngine::default();
    a.engine = Some(Box::new(engine.clone()));
    a.fetcher = Arc::new(MockFetcher {
        body: Some(body.into()),
    });
    a.handle_action(Action::Select, 1_000);
    wait_for(&mut a, |a| a.epg.len() == 2);
    engine.events.lock().unwrap().push(PlayerEvent::Loaded);
    a.poll_jobs(1_100);
    assert!(a.notice.is_none(), "loading notice cleared once playing");
    (a, engine)
}

#[test]
fn live_playback_draws_the_channel_name_once_and_no_separate_info_strip() {
    for size in [egui::vec2(1280.0, 800.0), egui::vec2(640.0, 400.0)] {
        let (mut a, _) = live_with_epg("live_overlay_once");
        a.player.show_osd(OsdKind::Info, 1_000);
        let ctx = egui::Context::default();
        skjalf::theme::apply(&ctx);
        let output = render_player(&ctx, &mut a, 1_200, size);
        assert!(ctx
            .read_response(skjalf::views::transport_bar::control_id(
                BarControl::PlayPause
            ))
            .is_some());
        assert_eq!(labels_containing(&output, "News 1").len(), 1, "{size:?}");
        assert!(
            labels_containing(&output, "Volume").is_empty(),
            "the old strip's volume text is gone"
        );
        let program = labels_containing(&output, "Now: News Now (10:00)");
        assert_eq!(program.len(), 1, "program sits in the bar title area");
    }
}

#[test]
fn x_button_expands_the_program_list_above_the_bar() {
    let (mut a, _) = live_with_epg("live_program_list");
    a.handle_action(Action::Epg, 2_000);
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    let output = render_player(&ctx, &mut a, 2_100, egui::vec2(1280.0, 800.0));
    assert_eq!(labels_containing(&output, "10:00-11:00  News Now").len(), 1);
    assert_eq!(
        labels_containing(&output, "11:00-12:00  Next Show").len(),
        1
    );
    assert_eq!(
        labels_containing(&output, "News 1").len(),
        1,
        "name still once"
    );
    let list = text_positions(&output, "10:00-11:00  News Now")[0];
    let bar = ctx
        .read_response(skjalf::views::transport_bar::control_id(
            BarControl::PlayPause,
        ))
        .unwrap()
        .rect;
    assert!(
        list.y < bar.min.y,
        "list above the bar: {list:?} vs {bar:?}"
    );
}

#[test]
fn loading_notice_for_a_long_channel_name_stays_on_one_line() {
    let mut a = app("loading_notice");
    let mut pl = playlist();
    pl.channels[0].name = "VERY LONG CHANNEL NAME ".repeat(20);
    a.set_playlist(pl);
    a.engine = Some(Box::new(MockEngine::default()));
    a.handle_action(Action::Select, 0);
    assert!(a.notice.as_deref().unwrap().starts_with("Loading "));
    for size in [egui::vec2(1280.0, 800.0), egui::vec2(640.0, 400.0)] {
        let ctx = egui::Context::default();
        skjalf::theme::apply(&ctx);
        let output = render_player(&ctx, &mut a, 100, size);
        let notice = a.notice.clone().unwrap();
        let mut rows = None;
        for clipped in &output.shapes {
            let mut found = None;
            text_rows_prefix(&clipped.shape, "Loading ", &mut found);
            rows = rows.or(found);
        }
        let _ = notice;
        assert_eq!(rows, Some(1), "{size:?}");
    }
}

fn text_rows_prefix(shape: &egui::Shape, prefix: &str, out: &mut Option<usize>) {
    match shape {
        egui::Shape::Text(text) if text.galley.text().starts_with(prefix) => {
            *out = Some(text.galley.rows.len())
        }
        egui::Shape::Vec(shapes) => shapes
            .iter()
            .for_each(|shape| text_rows_prefix(shape, prefix, out)),
        _ => {}
    }
}

// -- Shortcuts to the filter and search fields ------------------------------------------

#[test]
fn left_on_the_sidebar_focuses_the_group_filter_without_editing() {
    let mut a = filter_app("shortcut_left");
    a.set_category(BrowserId::Live, group("g4"));
    a.live.focus = BrowserFocus::Categories;
    a.handle_action(Action::Left, 0);
    assert_eq!(a.live.focus, BrowserFocus::GroupFilter);
    assert!(!a.group_filter_editing(), "Select starts editing");
    assert_eq!(a.live.category, group("g4"), "selection untouched");
    a.handle_action(Action::Select, 0);
    assert!(a.group_filter_editing());
}

#[test]
fn left_from_the_sidebar_returns_to_the_filter_after_a_match_was_picked() {
    let mut a = filter_app("shortcut_return");
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    a.live.focus = BrowserFocus::GroupFilter;
    a.handle_action(Action::Select, 0);
    live_frame(&ctx, &mut a, vec![]);
    type_text(&mut a, &ctx, "si|");
    live_frame(&ctx, &mut a, vec![key_event(egui::Key::Enter)]);
    assert_eq!(a.live.focus, BrowserFocus::Categories);
    assert_eq!(a.live.category, group("g0"));
    a.handle_action(Action::Left, 0);
    assert_eq!(a.live.focus, BrowserFocus::GroupFilter);
    assert_eq!(a.live.group_filter, "si|", "text is kept for changing");
    a.handle_action(Action::Select, 0);
    assert!(a.group_filter_editing());
    live_frame(&ctx, &mut a, vec![]);
    live_frame(&ctx, &mut a, vec![key_event(egui::Key::Escape)]);
    assert_eq!(a.live.focus, BrowserFocus::Categories);
}

#[test]
fn epg_button_focuses_search_and_starts_editing_in_every_browser() {
    let mut a = filter_app("shortcut_search_live");
    a.live.focus = BrowserFocus::Categories;
    a.handle_action(Action::Epg, 0);
    assert!(a.search_editing());
    assert_eq!(a.live.focus, BrowserFocus::Search);
    a.handle_action(Action::Back, 0);
    assert!(!a.search_editing());

    let (mut m, _, _) = movies_app("shortcut_search_movies");
    open_movies(&mut m);
    m.handle_action(Action::Epg, 0);
    assert!(m.search_editing());
    assert_eq!(m.movies.focus, BrowserFocus::Search);

    let (mut s, _, _) = movies_app("shortcut_search_series");
    open_series(&mut s);
    s.handle_action(Action::Epg, 0);
    assert!(s.search_editing());
    assert_eq!(s.series.focus, BrowserFocus::Search);
}

#[test]
fn e_key_starts_search_and_then_types_a_literal_e() {
    let mut a = filter_app("shortcut_e_key");
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    live_frame(&ctx, &mut a, vec![egui::Event::Text("e".into())]);
    assert!(a.search_editing(), "the e key opens the search field");
    live_frame(&ctx, &mut a, vec![]);
    type_text(&mut a, &ctx, "e");
    assert_eq!(a.live.query, "e", "typing continues into the field");
}

#[test]
fn epg_button_opens_the_steam_keyboard_in_game_mode() {
    struct Recorder(Arc<Mutex<Vec<&'static str>>>);
    impl KeyboardLauncher for Recorder {
        fn send(&self, uri: &'static str) -> mpsc::Receiver<Result<(), String>> {
            self.0.lock().unwrap().push(uri);
            let (tx, rx) = mpsc::channel();
            tx.send(Ok(())).unwrap();
            rx
        }
    }
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut a = filter_app("shortcut_keyboard");
    a.keyboard_launcher = Arc::new(Recorder(requests.clone()));
    a.game_mode = true;
    a.handle_action(Action::Epg, 0);
    a.poll_jobs(0);
    assert!(a.steam_keyboard_active());
    assert_eq!(*requests.lock().unwrap(), [OPEN_URI]);
}

#[test]
fn favorite_stays_on_y_while_epg_does_not_toggle_it() {
    let mut a = filter_app("shortcut_favorite");
    a.set_category(BrowserId::Live, CategorySel::All);
    a.live.focus = BrowserFocus::Items;
    a.handle_action(Action::Favorite, 0);
    assert_eq!(a.favorites.iter().count(), 1);
    a.handle_action(Action::Epg, 0);
    assert_eq!(a.favorites.iter().count(), 1);
}

// -- Main tiles ---------------------------------------------------------------------------

#[test]
fn main_tiles_keep_titles_on_one_line_and_hide_empty_ratings() {
    let long = "PL - Nędznicy (2019) Extended Director's Cut Special Edition";
    let dir = seeded_dir("tiles_tidy");
    Store::open(&dir)
        .unwrap()
        .save_movies(&MovieCatalog {
            source_id: source().id,
            groups: vec![],
            movies: [
                (long, Some("0"), 50),
                ("Rated Eight", Some("8"), 40),
                ("Empty Rating", Some(""), 30),
                ("No Rating", None, 20),
                ("Zero Point", Some("0.0"), 10),
            ]
            .into_iter()
            .enumerate()
            .map(|(n, (name, rating, added))| Movie {
                id: n.to_string(),
                name: name.into(),
                group_id: "x".into(),
                icon: None,
                added: Some(added),
                rating: rating.map(str::to_owned),
                extension: "mkv".into(),
            })
            .collect(),
        })
        .unwrap();
    let mut a = main_app_at(&dir);
    wait_for(&mut a, |a| a.movie_slot.loaded.is_some());
    let tiles = a.main_row_tiles(MainRow::LatestMovies);
    let details: Vec<_> = tiles.iter().map(|t| t.detail.clone()).collect();
    assert_eq!(
        details,
        [None, Some("Rating 8.0".to_owned()), None, None, None]
    );
    for size in [egui::vec2(1024.0, 640.0), egui::vec2(640.0, 400.0)] {
        let ctx = egui::Context::default();
        skjalf::theme::apply(&ctx);
        let mut output = None;
        for _ in 0..4 {
            output = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ctx| skjalf::views::main_menu::show(ctx, &mut a),
            ));
        }
        let output = output.unwrap();
        let mut labels = Vec::new();
        for shape in &output.shapes {
            shape_texts(&shape.shape, &mut labels);
        }
        let mut elided = None;
        for clipped in &output.shapes {
            galley_elided(&clipped.shape, "PL - ", &mut elided);
        }
        assert_eq!(elided, Some(true), "truncated with an ellipsis at {size:?}");
        assert!(!labels.iter().any(|text| text == ")"), "no orphan fragment");
        let mut rows = None;
        for clipped in &output.shapes {
            text_rows_prefix(&clipped.shape, "PL - ", &mut rows);
        }
        assert_eq!(rows, Some(1));
        assert!(labels.iter().any(|text| text == "Rating 8.0"));
        assert!(
            !labels
                .iter()
                .any(|text| text == "0" || text == "Rating 0" || text == "8"),
            "bare or zero ratings never show: {labels:?}"
        );
    }
}

#[test]
fn zero_ratings_add_nothing_to_browser_rows() {
    let (mut a, _, _) = movies_app("zero_rating_rows");
    open_movies(&mut a);
    all_movies(&mut a);
    let first = a
        .browser_row(BrowserId::Movies, a.movies.filtered[0])
        .unwrap();
    assert_eq!(first.label, "Movie A   7.2");
    assert_eq!(skjalf::app::rating_detail(Some("0")), None);
    assert_eq!(skjalf::app::rating_detail(Some(" ")), None);
    assert_eq!(skjalf::app::rating_detail(Some("0.00")), None);
    assert_eq!(
        skjalf::app::rating_detail(Some("5.5")).as_deref(),
        Some("Rating 5.5")
    );
    assert_eq!(skjalf::app::rating_detail(Some("abc")), None);
}

fn galley_elided(shape: &egui::Shape, prefix: &str, out: &mut Option<bool>) {
    match shape {
        egui::Shape::Text(text) if text.galley.text().starts_with(prefix) => {
            *out = Some(text.galley.elided)
        }
        egui::Shape::Vec(shapes) => shapes
            .iter()
            .for_each(|shape| galley_elided(shape, prefix, out)),
        _ => {}
    }
}

#[test]
fn ratings_show_one_decimal_in_rows_tiles_and_details() {
    let dir = seeded_dir("ratings_one_decimal");
    let movie = |id: &str, name: &str, rating: &str, added: i64| Movie {
        id: id.into(),
        name: name.into(),
        group_id: "x".into(),
        icon: None,
        added: Some(added),
        rating: Some(rating.into()),
        extension: "mkv".into(),
    };
    Store::open(&dir)
        .unwrap()
        .save_movies(&MovieCatalog {
            source_id: source().id,
            groups: vec![],
            movies: vec![
                movie("1", "Alpha", "7.296", 50),
                movie("2", "Bravo", "8.987", 40),
                movie("3", "Charlie", "7", 30),
                movie("4", "Delta", "abc", 20),
                movie("5", "Echo", "0", 10),
            ],
        })
        .unwrap();
    let mut a = main_app_at(&dir);
    wait_for(&mut a, |a| a.movie_slot.loaded.is_some());
    a.set_category(BrowserId::Movies, CategorySel::All);
    let labels: Vec<_> = a
        .movies
        .filtered
        .iter()
        .map(|&item| a.browser_row(BrowserId::Movies, item).unwrap().label)
        .collect();
    assert_eq!(
        labels,
        [
            "Alpha   7.3",
            "Bravo   9.0",
            "Charlie   7.0",
            "Delta",
            "Echo"
        ]
    );
    let details: Vec<_> = a
        .main_row_tiles(MainRow::LatestMovies)
        .into_iter()
        .map(|tile| tile.detail)
        .collect();
    assert_eq!(
        details,
        [
            Some("Rating 7.3".to_owned()),
            Some("Rating 9.0".to_owned()),
            Some("Rating 7.0".to_owned()),
            None,
            None
        ]
    );
    a.activate_main_button(MainButton::Movies);
    a.handle_action(Action::Select, 0);
    let (labels, _) = render_labels(&mut a, |ctx, app| {
        skjalf::views::movie_details::show(ctx, app, 0)
    });
    assert!(labels.iter().any(|text| text == "Rating 7.3"), "{labels:?}");
}

#[test]
fn series_ratings_show_one_decimal_in_details() {
    let (mut a, _, _) = movies_app("series_rating_decimal");
    open_series(&mut a);
    open_series_details(&mut a);
    let (labels, _) = render_labels(&mut a, |ctx, app| {
        skjalf::views::series_details::show(ctx, app, 0)
    });
    assert!(
        labels.iter().any(|text| text.starts_with("Rating 8.0")),
        "{labels:?}"
    );
}

// -- Right-to-left display text ---------------------------------------------------------------

const FA_LOGICAL: &str = "دنیای دیگر";
const FA_VISUAL: &str = "\u{FEAE}\u{FB95}\u{FBFE}\u{FEA9} \u{FBFC}\u{FE8E}\u{FBFF}\u{FEE7}\u{FEA9}";

#[test]
fn live_rows_and_groups_draw_right_to_left_names_in_visual_order_and_search_stays_logical() {
    use skjalf_core::display::rtl_conversions;
    let mut a = app("rtl_live");
    let mut pl = playlist();
    pl.groups = named_groups(&[FA_LOGICAL, "News"]);
    pl.channels[0].name = format!("{FA_LOGICAL} HD");
    pl.channels[0].group_id = "g0".into();
    a.set_playlist(pl);
    a.set_category(BrowserId::Live, CategorySel::All);
    let row = a.browser_row(BrowserId::Live, 0).unwrap();
    assert!(row.label.contains(FA_VISUAL), "{:?}", row.label);
    assert_eq!(
        a.category_label(BrowserId::Live, 2).as_deref(),
        Some(FA_VISUAL)
    );
    a.set_category(BrowserId::Live, group("g0"));
    assert_eq!(a.category_title(BrowserId::Live), FA_VISUAL);

    a.set_query(BrowserId::Live, "دنیای".into());
    assert_eq!(
        a.group_channel_ids(),
        ["1"],
        "search matches the logical text"
    );
    a.set_group_filter(BrowserId::Live, "دیگر".into());
    assert_eq!(
        group_names(&a),
        [FA_VISUAL],
        "the group filter matches the logical text"
    );

    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    live_frame(&ctx, &mut a, vec![]);
    let before = rtl_conversions();
    for _ in 0..5 {
        live_frame(&ctx, &mut a, vec![]);
    }
    assert_eq!(rtl_conversions(), before, "painting converts nothing");
    let output = live_frame(&ctx, &mut a, vec![]);
    let mut labels = Vec::new();
    for shape in &output.shapes {
        shape_texts(&shape.shape, &mut labels);
    }
    assert!(labels.iter().any(|text| text.contains(FA_VISUAL)));
    assert!(!labels.iter().any(|text| text.contains(FA_LOGICAL)));
}

#[test]
fn movie_catalog_display_names_are_built_once_in_the_worker() {
    use skjalf_core::display::rtl_conversions;
    let dir = seeded_dir("rtl_movies");
    Store::open(&dir)
        .unwrap()
        .save_movies(&MovieCatalog {
            source_id: source().id,
            groups: named_groups(&[FA_LOGICAL]),
            movies: (0..2_000)
                .map(|n| Movie {
                    id: n.to_string(),
                    name: if n % 2 == 0 {
                        format!("{FA_LOGICAL} {n}")
                    } else {
                        format!("Film {n}")
                    },
                    group_id: "g0".into(),
                    icon: None,
                    added: Some(n),
                    rating: None,
                    extension: "mkv".into(),
                })
                .collect(),
        })
        .unwrap();
    let mut a = main_app_at(&dir);
    wait_for(&mut a, |a| a.movie_slot.loaded.is_some());
    a.set_category(BrowserId::Movies, CategorySel::All);
    let before = rtl_conversions();
    let first = a.browser_row(BrowserId::Movies, 0).unwrap().label;
    assert!(first.contains(FA_VISUAL), "{first:?}");
    assert_eq!(a.browser_row(BrowserId::Movies, 1).unwrap().label, "Film 1");
    for _ in 0..50 {
        for item in 0..40 {
            a.browser_row(BrowserId::Movies, item).unwrap();
        }
    }
    assert_eq!(rtl_conversions(), before, "rows read stored display names");
    a.set_query(BrowserId::Movies, "دنیای دیگر 1000".into());
    assert_eq!(a.movies.filtered.len(), 1, "search uses the logical names");
    let tiles = a.main_row_tiles(MainRow::LatestMovies);
    assert!(tiles[0].title.contains("Film") || tiles[0].title.contains(FA_VISUAL));
}

#[test]
fn details_show_right_to_left_text_in_visual_order() {
    let (mut a, fetcher, _) = movies_app("rtl_details");
    fetcher.persian_info.store(true, Ordering::Relaxed);
    open_movies(&mut a);
    all_movies(&mut a);
    a.handle_action(Action::Select, 0);
    wait_for(&mut a, |a| {
        matches!(a.movie_details.as_ref().unwrap().info, DetailInfo::Ready(_))
    });
    let DetailInfo::Ready(info) = &a.movie_details.as_ref().unwrap().info else {
        unreachable!()
    };
    assert_eq!(
        info.description.as_deref(),
        Some(FA_LOGICAL),
        "the description stays logical for wrapping"
    );
    let (labels, _) = render_labels(&mut a, |ctx, app| {
        skjalf::views::movie_details::show(ctx, app, 0)
    });
    assert!(
        labels.iter().any(|text| text.contains(FA_VISUAL)),
        "{labels:?}"
    );

    let (mut s, fetcher, _) = movies_app("rtl_series_details");
    fetcher.persian_info.store(true, Ordering::Relaxed);
    open_series(&mut s);
    open_series_details(&mut s);
    let DetailInfo::Ready(info) = &s.series_details.as_ref().unwrap().info else {
        unreachable!()
    };
    assert_eq!(info.plot.as_deref(), Some(FA_LOGICAL));
    assert_eq!(info.seasons[0].name, FA_VISUAL);
}

#[test]
fn episode_rows_and_play_titles_drop_the_repeated_series_name() {
    let (mut a, fetcher, engine) = movies_app("short_episode_titles");
    fetcher.repeat_series_name.store(true, Ordering::Relaxed);
    open_series(&mut a);
    open_series_details(&mut a);
    let series_name = a.series_details.as_ref().unwrap().series.name.clone();
    let (labels, _) = render_labels(&mut a, |ctx, app| {
        skjalf::views::series_details::show(ctx, app, 0)
    });
    assert!(
        labels.iter().any(|text| text == "E1  Center de Stage"),
        "{labels:?}"
    );
    assert!(!labels.iter().any(|text| text.contains("S01E01")));
    a.handle_action(Action::Select, 0);
    assert!(a.is_on_demand());
    assert_eq!(
        a.now_playing_title(),
        format!("{series_name}  E1  Center de Stage")
    );
    assert_eq!(engine.loads.lock().unwrap().len(), 1);
}

#[test]
fn progress_records_keep_the_providers_episode_title() {
    let (mut a, fetcher, engine) = movies_app("record_original_title");
    fetcher.repeat_series_name.store(true, Ordering::Relaxed);
    open_series(&mut a);
    open_series_details(&mut a);
    a.handle_action(Action::Select, 0);
    set_clock(&engine, 900.0, 2700.0);
    a.handle_action(Action::Back, 2_000);
    let saved = a
        .store
        .as_ref()
        .unwrap()
        .load_progress()
        .get(&source().id, ProgressKind::Episode, "200111")
        .cloned()
        .unwrap();
    assert_eq!(saved.title, "Show A - S01E01 - Center de Stage");
    assert_eq!(a.continue_titles, ["Show A  E1  Center de Stage"]);
}

#[test]
fn episode_display_titles_are_built_once_per_details_load() {
    use skjalf_core::display::rtl_conversions;
    let (mut a, fetcher, _) = movies_app("episode_titles_once");
    fetcher.persian_info.store(true, Ordering::Relaxed);
    open_series(&mut a);
    open_series_details(&mut a);
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    let draw = |ctx: &egui::Context, a: &mut App| {
        ctx.run(compact_input(vec![]), |ctx| {
            skjalf::views::series_details::show(ctx, a, 0)
        })
    };
    let first = draw(&ctx, &mut a);
    let mut labels = Vec::new();
    for shape in &first.shapes {
        shape_texts(&shape.shape, &mut labels);
    }
    assert!(
        labels.iter().any(|text| text.contains(FA_VISUAL)),
        "{labels:?}"
    );
    let warm = rtl_conversions();
    for _ in 0..5 {
        draw(&ctx, &mut a);
    }
    assert_eq!(
        rtl_conversions(),
        warm,
        "repaints convert nothing: episode rows come from the stored strings and the paragraph is cached"
    );
}

#[test]
fn program_list_draws_right_to_left_titles_in_visual_order() {
    let (mut a, _) = live_with_epg("program_list_rtl");
    a.epg[0].title = FA_LOGICAL.into();
    a.handle_action(Action::Epg, 2_000);
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    let output = render_player(&ctx, &mut a, 2_100, egui::vec2(1280.0, 800.0));
    let lists = labels_containing(&output, "10:00-11:00");
    assert_eq!(lists.len(), 1);
    assert!(lists[0].ends_with(FA_VISUAL), "{:?}", lists[0]);
}

// -- Wrapping right-to-left paragraphs ------------------------------------------------------

const LONG_PERSIAN: &str = "داستان دختری به‌نام مریم که در شهری کوچک زندگی می‌کند و هر روز صبح به کنار دریا می‌رود تا غروب خورشید را تماشا کند و با ماهیگیران پیر درباره‌ی گذشته‌های دور گفتگو کند. روزی نامه‌ای از دوستی قدیمی می‌رسد که زندگی او را برای همیشه تغییر می‌دهد و او را به سفری طولانی می‌فرستد.";

fn arabic_lines(output: &egui::FullOutput) -> Vec<(egui::Pos2, f32, String)> {
    fn walk(shape: &egui::Shape, out: &mut Vec<(egui::Pos2, f32, String)>) {
        match shape {
            egui::Shape::Text(text)
                if text
                    .galley
                    .text()
                    .chars()
                    .any(|c| ('\u{FB50}'..='\u{FEFF}').contains(&c)) =>
            {
                let rect = text.galley.rect.translate(text.pos.to_vec2());
                out.push((rect.min, rect.width(), text.galley.text().to_owned()));
            }
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, out)),
            _ => {}
        }
    }
    let mut lines = Vec::new();
    for clipped in &output.shapes {
        walk(&clipped.shape, &mut lines);
    }
    lines.sort_by(|a, b| a.0.y.total_cmp(&b.0.y));
    lines
}

#[test]
fn long_persian_paragraphs_wrap_before_reordering_and_right_align() {
    use skjalf_core::display::visual_text;
    let first_words = "داستان دختری به‌نام";
    let first_visual = visual_text(first_words).into_owned();
    for width in [640.0_f32, 1280.0] {
        let (mut a, fetcher, _) = movies_app("rtl_wrap");
        fetcher.long_persian.store(true, Ordering::Relaxed);
        open_movies(&mut a);
        all_movies(&mut a);
        a.handle_action(Action::Select, 0);
        wait_for(&mut a, |a| {
            matches!(a.movie_details.as_ref().unwrap().info, DetailInfo::Ready(_))
        });
        let ctx = egui::Context::default();
        skjalf::theme::apply(&ctx);
        let mut output = None;
        for frame in 0..4 {
            output = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 800.0),
                    )),
                    time: Some(f64::from(frame) * 0.1),
                    ..Default::default()
                },
                |ctx| skjalf::views::movie_details::show(ctx, &mut a, 0),
            ));
        }
        let lines: Vec<_> = arabic_lines(&output.unwrap())
            .into_iter()
            .filter(|line| !line.2.starts_with("Cast"))
            .collect();
        assert!(
            lines.len() >= 2,
            "{width}: wrapped into several lines: {}",
            lines.len()
        );
        assert!(
            lines[0].2.contains(&first_visual),
            "{width}: the first visual line holds the first logical words: {:?}",
            lines[0].2
        );
        assert!(
            !lines[1..].iter().any(|line| line.2.contains(&first_visual)),
            "the first words appear once, on the first line"
        );
        for (pos, line_width, text) in &lines {
            assert!(
                pos.x + line_width <= width + 0.5,
                "{width}: line fits: {text:?}"
            );
        }
        let right_edges: Vec<f32> = lines.iter().map(|(pos, w, _)| pos.x + w).collect();
        let first_edge = right_edges[0];
        assert!(
            right_edges
                .iter()
                .all(|edge| (edge - first_edge).abs() < 1.5),
            "{width}: lines are right-aligned: {right_edges:?}"
        );
        assert!(
            first_edge > width * 0.8,
            "{width}: against the right side: {first_edge}"
        );
    }
}

#[test]
fn latin_paragraphs_keep_egui_wrapping() {
    let (mut a, fetcher, _) = movies_app("latin_wrap");
    fetcher.long_text.store(true, Ordering::Relaxed);
    open_movies(&mut a);
    all_movies(&mut a);
    a.handle_action(Action::Select, 0);
    wait_for(&mut a, |a| {
        matches!(a.movie_details.as_ref().unwrap().info, DetailInfo::Ready(_))
    });
    let (labels, _) = render_labels(&mut a, |ctx, app| {
        skjalf::views::movie_details::show(ctx, app, 0)
    });
    assert!(labels
        .iter()
        .any(|text| text.starts_with("long words long words")));
}

#[test]
fn wrapped_paragraphs_are_cached_per_width() {
    use skjalf_core::display::rtl_conversions;
    let (mut a, fetcher, _) = movies_app("rtl_wrap_cache");
    fetcher.long_persian.store(true, Ordering::Relaxed);
    open_movies(&mut a);
    all_movies(&mut a);
    a.handle_action(Action::Select, 0);
    wait_for(&mut a, |a| {
        matches!(a.movie_details.as_ref().unwrap().info, DetailInfo::Ready(_))
    });
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    let draw = |width: f32, a: &mut App| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 800.0),
                )),
                ..Default::default()
            },
            |ctx| skjalf::views::movie_details::show(ctx, a, 0),
        )
    };
    for _ in 0..3 {
        draw(1280.0, &mut a);
    }
    let warm = rtl_conversions();
    for _ in 0..10 {
        draw(1280.0, &mut a);
    }
    assert_eq!(
        rtl_conversions(),
        warm,
        "same width reuses the wrapped lines"
    );
    for _ in 0..3 {
        draw(640.0, &mut a);
    }
    assert!(rtl_conversions() > warm, "a new width wraps again");
    let rewarm = rtl_conversions();
    for _ in 0..5 {
        draw(640.0, &mut a);
    }
    assert_eq!(rtl_conversions(), rewarm);
}

// -- Group filter clear control and empty message ------------------------------------------

#[test]
fn clear_control_is_a_focus_stop_that_empties_the_filter() {
    let mut a = filter_app("gf_clear_focus");
    a.set_group_filter(BrowserId::Live, "uk".into());
    assert_eq!(group_names(&a), ["UK| BBC"]);
    a.live.focus = BrowserFocus::GroupFilter;
    a.handle_action(Action::Right, 0);
    assert_eq!(a.live.focus, BrowserFocus::GroupFilterClear);
    a.handle_action(Action::Left, 0);
    assert_eq!(a.live.focus, BrowserFocus::GroupFilter);
    a.handle_action(Action::Right, 0);
    a.handle_action(Action::Select, 0);
    assert!(a.live.group_filter.is_empty());
    assert_eq!(group_names(&a).len(), 7, "the full list is back");
    assert_eq!(
        a.live.focus,
        BrowserFocus::GroupFilter,
        "focus returns to the field"
    );
    assert!(!a.group_filter_editing());
    a.handle_action(Action::Right, 0);
    a.handle_action(Action::Right, 0);
    assert_eq!(
        a.live.focus,
        BrowserFocus::Search,
        "Right from X reaches the item search"
    );
}

#[test]
fn down_from_the_filter_keeps_the_category_or_moves_to_the_first_match() {
    for start in [BrowserFocus::GroupFilter, BrowserFocus::GroupFilterClear] {
        // No filter text: focus the selected category without changing it.
        let mut a = filter_app("gf_down_keep");
        a.set_category(BrowserId::Live, group("g3"));
        a.live.focus = start;
        a.handle_action(Action::Down, 0);
        assert_eq!(a.live.focus, BrowserFocus::Categories, "{start:?}");
        assert_eq!(
            a.live.category,
            group("g3"),
            "{start:?}: not switched to Favorites"
        );

        // Filter text with matches: the first match.
        let mut a = filter_app("gf_down_match");
        a.set_category(BrowserId::Live, group("g3"));
        a.set_group_filter(BrowserId::Live, "si|".into());
        a.live.focus = start;
        a.handle_action(Action::Down, 0);
        assert_eq!(a.live.focus, BrowserFocus::Categories, "{start:?}");
        assert_eq!(
            a.live.category,
            group("g0"),
            "{start:?}: first matching group"
        );

        // A filter that hides the selected category but still matches others:
        // the first match is the first visible entry.
        let mut a = filter_app("gf_down_hidden");
        a.set_category(BrowserId::Live, group("g6"));
        a.set_group_filter(BrowserId::Live, "uk".into());
        a.live.focus = start;
        a.handle_action(Action::Down, 0);
        assert_eq!(a.live.category, group("g3"), "{start:?}");

        // No match at all and a hidden selection: the first visible entry.
        let mut a = filter_app("gf_down_nomatch");
        a.set_category(BrowserId::Live, group("g3"));
        a.set_group_filter(BrowserId::Live, "zzz".into());
        a.live.focus = start;
        a.handle_action(Action::Down, 0);
        assert_eq!(a.live.focus, BrowserFocus::Categories, "{start:?}");
        assert_eq!(a.live.category, CategorySel::Favorites, "{start:?}");

        // No match but a pinned entry selected: unchanged.
        let mut a = filter_app("gf_down_pinned");
        a.set_category(BrowserId::Live, CategorySel::All);
        a.set_group_filter(BrowserId::Live, "zzz".into());
        a.live.focus = start;
        a.handle_action(Action::Down, 0);
        assert_eq!(a.live.category, CategorySel::All, "{start:?}");
    }
}

#[test]
fn clear_control_works_in_movies_and_series() {
    let (mut m, _, _) = movies_app("gf_clear_movies");
    open_movies(&mut m);
    m.set_group_filter(BrowserId::Movies, "act".into());
    m.movies.focus = BrowserFocus::GroupFilter;
    press(&mut m, &[Action::Right, Action::Select]);
    assert!(m.movies.group_filter.is_empty());
    assert_eq!(m.movies.focus, BrowserFocus::GroupFilter);
    let (mut s, _, _) = movies_app("gf_clear_series");
    open_series(&mut s);
    s.set_group_filter(BrowserId::Series, "dra".into());
    s.series.focus = BrowserFocus::GroupFilterClear;
    s.handle_action(Action::Select, 0);
    assert!(s.series.group_filter.is_empty());
    s.handle_action(Action::Right, 0);
    s.handle_action(Action::Right, 0);
    assert_eq!(s.series.focus, BrowserFocus::Search);
}

#[test]
fn clear_control_is_highlighted_when_focused_and_touch_still_clears() {
    let mut a = filter_app("gf_clear_render");
    a.set_group_filter(BrowserId::Live, "uk".into());
    a.live.focus = BrowserFocus::GroupFilterClear;
    let ctx = egui::Context::default();
    skjalf::theme::apply(&ctx);
    let output = live_frame(&ctx, &mut a, vec![]);
    let mut color = None;
    for clipped in &output.shapes {
        text_color(&clipped.shape, "X", &mut color);
    }
    assert_eq!(color, Some(egui::Color32::BLACK), "dark text on the accent");
    click_text(&ctx, &mut a, "X", skjalf::views::live::show);
    assert!(a.live.group_filter.is_empty());
}

#[test]
fn an_unmatched_filter_says_no_groups_match_and_keeps_pinned_entries() {
    for browser in [BrowserId::Live, BrowserId::Movies, BrowserId::Series] {
        let (mut a, _, _) = movies_app("gf_no_match");
        match browser {
            BrowserId::Live => {
                a.activate_main_button(MainButton::LiveTv);
            }
            BrowserId::Movies => open_movies(&mut a),
            BrowserId::Series => open_series(&mut a),
        }
        let show: fn(&egui::Context, &mut App) = match browser {
            BrowserId::Live => skjalf::views::live::show,
            BrowserId::Movies => skjalf::views::movies::show,
            BrowserId::Series => skjalf::views::series::show,
        };
        let (labels, _) = render_labels(&mut a, show);
        assert!(
            !labels.iter().any(|t| t == "No groups match."),
            "{browser:?}: no filter"
        );
        a.set_group_filter(browser, "si|".into());
        assert_eq!(a.categories_len(browser), 2, "{browser:?}");
        assert!(a.no_groups_match(browser));
        let (labels, _) = render_labels(&mut a, show);
        assert_eq!(
            labels.iter().filter(|t| *t == "No groups match.").count(),
            1,
            "{browser:?}"
        );
        assert!(labels.iter().any(|t| t == "All"), "{browser:?}: All stays");
        assert!(labels.iter().any(|t| t.contains("Favorites")));
        a.state_mut(browser).focus = BrowserFocus::Categories;
        a.set_category(browser, CategorySel::All);
        a.handle_action(Action::Down, 0);
        assert!(a.state(browser).focus == BrowserFocus::Categories);
        a.set_group_filter(browser, String::new());
        assert!(!a.no_groups_match(browser));
        let (labels, _) = render_labels(&mut a, show);
        assert!(!labels.iter().any(|t| t == "No groups match."));
    }
}

// -- Favorite groups and visible favorite controls -----------------------------------------

fn focus_group(a: &mut App, id: BrowserId, group_id: &str) {
    a.set_category(id, group(group_id));
    a.state_mut(id).focus = BrowserFocus::Categories;
}

fn text_rects(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
    match shape {
        egui::Shape::Text(text) => out.push((
            text.galley.text().to_owned(),
            egui::Rect::from_min_size(text.pos, text.galley.size()),
        )),
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| text_rects(shape, out)),
        _ => {}
    }
}

#[test]
fn y_on_a_group_toggles_it_with_a_toast_and_persists() {
    let dir = seeded_dir("group_fav");
    let mut a = app_at(&dir);
    assert_eq!(
        sidebar(&a, BrowserId::Live),
        ["\u{2605} Favorites", "All", "News", "Sports"]
    );
    focus_group(&mut a, BrowserId::Live, "g2");
    a.handle_action(Action::Favorite, 0);
    assert!(a
        .favorites
        .contains(a.source_id().unwrap(), "group:live:g2"));
    assert_eq!(a.notice.as_deref(), Some("Added to favorites: Sports"));
    assert_eq!(sidebar(&a, BrowserId::Live)[2..], ["Sports", "News"]);
    assert!(a.category_favorite(BrowserId::Live, 2));
    assert!(!a.category_favorite(BrowserId::Live, 3));
    assert_eq!(a.live.category, group("g2"));
    drop(a);

    let mut b = app_at(&dir);
    assert_eq!(sidebar(&b, BrowserId::Live)[2..], ["Sports", "News"]);
    focus_group(&mut b, BrowserId::Live, "g2");
    b.handle_action(Action::Favorite, 0);
    assert_eq!(b.notice.as_deref(), Some("Removed from favorites: Sports"));
    assert_eq!(sidebar(&b, BrowserId::Live)[2..], ["News", "Sports"]);
    assert_eq!(b.favorites.iter().count(), 0);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn y_on_favorites_or_all_does_nothing() {
    let mut a = app("group_fav_fixed_rows");
    for category in [CategorySel::Favorites, CategorySel::All] {
        a.set_category(BrowserId::Live, category);
        a.live.focus = BrowserFocus::Categories;
        a.notice = None;
        a.handle_action(Action::Favorite, 0);
        assert_eq!(a.favorites.iter().count(), 0);
        assert_eq!(a.notice, None);
    }
}

#[test]
fn favorite_groups_lead_the_sidebar_in_both_sorts_and_under_a_filter() {
    let mut a = app("group_fav_order");
    let mut pl = playlist();
    pl.groups = named_groups(&["Zeta", "alpha", "Mid", "beta"]);
    a.set_playlist(pl);
    for id in ["g0", "g3"] {
        focus_group(&mut a, BrowserId::Live, id);
        a.handle_action(Action::Favorite, 0);
    }
    let tail = |a: &App| sidebar(a, BrowserId::Live)[2..].to_vec();
    assert_eq!(tail(&a), ["Zeta", "beta", "alpha", "Mid"]);
    a.toggle_sort(BrowserId::Live);
    assert_eq!(tail(&a), ["beta", "Zeta", "alpha", "Mid"]);
    a.set_group_filter(BrowserId::Live, "A".into());
    assert_eq!(tail(&a), ["beta", "Zeta", "alpha"]);
    a.toggle_sort(BrowserId::Live);
    assert_eq!(tail(&a), ["Zeta", "beta", "alpha"]);
    let builds = a.group_list_builds(BrowserId::Live);
    for _ in 0..5 {
        a.refresh_browsers();
    }
    assert_eq!(a.group_list_builds(BrowserId::Live), builds);
}

#[test]
fn group_favorites_leave_item_favorites_alone() {
    let mut a = app("group_fav_items");
    a.set_category(BrowserId::Live, group("g1"));
    a.live.focus = BrowserFocus::Items;
    a.handle_action(Action::Favorite, 0);
    a.live.focus = BrowserFocus::Categories;
    a.handle_action(Action::Favorite, 0);
    assert_eq!(a.favorites.iter().count(), 2);
    a.set_category(BrowserId::Live, CategorySel::Favorites);
    assert_eq!(a.group_channel_ids(), ["1"]);
}

#[test]
fn movie_and_series_groups_use_their_own_keys() {
    let (mut a, _, _) = movies_app("group_fav_vod");
    open_movies(&mut a);
    focus_group(&mut a, BrowserId::Movies, "12");
    a.handle_action(Action::Favorite, 0);
    open_series(&mut a);
    focus_group(&mut a, BrowserId::Series, "30");
    a.handle_action(Action::Favorite, 0);
    let source = a.source_id().unwrap().to_owned();
    assert!(a.favorites.contains(&source, "group:movie:12"));
    assert!(a.favorites.contains(&source, "group:series:30"));
    assert_eq!(a.favorites.iter().count(), 2);
    assert!(a.category_favorite(BrowserId::Movies, 2));
    assert!(a.category_favorite(BrowserId::Series, 2));
    assert!(!a.category_favorite(BrowserId::Live, 2));
}

#[test]
fn hint_text_follows_focus_and_favorite_state() {
    let mut a = app("hint_text");
    a.set_category(BrowserId::Live, group("g1"));
    a.live.focus = BrowserFocus::Items;
    assert_eq!(
        a.browser_hint(BrowserId::Live),
        "A Open   Y Favorite   X Search   L1/R1 Page   B Back"
    );
    a.handle_action(Action::Favorite, 0);
    assert!(a.browser_hint(BrowserId::Live).contains("Y Unfavorite"));

    a.live.focus = BrowserFocus::Categories;
    assert_eq!(
        a.browser_hint(BrowserId::Live),
        "A Open   Y Favorite group   X Search   B Back"
    );
    a.handle_action(Action::Favorite, 0);
    assert!(a
        .browser_hint(BrowserId::Live)
        .contains("Y Unfavorite group"));
    for category in [CategorySel::Favorites, CategorySel::All] {
        a.set_category(BrowserId::Live, category);
        assert_eq!(
            a.browser_hint(BrowserId::Live),
            "A Open   X Search   B Back"
        );
    }
    for focus in [BrowserFocus::Search, BrowserFocus::GroupFilter] {
        a.live.focus = focus;
        assert_eq!(a.browser_hint(BrowserId::Live), "A Edit   B Back");
    }
    a.live.focus = BrowserFocus::Items;
    a.handle_action(Action::Epg, 0);
    assert_eq!(
        a.browser_hint(BrowserId::Live),
        "Type, then Enter or B to finish"
    );
    a.handle_action(Action::Back, 0);
    let after = a.browser_hint(BrowserId::Live);
    assert!(after.starts_with("A Open"), "{after}");
}

#[test]
fn hint_strip_is_on_screen_and_below_the_last_list_row() {
    for (width, height) in [(1280.0_f32, 800.0_f32), (640.0, 400.0)] {
        let mut a = app("hint_strip");
        let mut playlist = a.playlist.clone().unwrap();
        playlist.channels = (0..100)
            .map(|n| Channel {
                id: n.to_string(),
                name: format!("HINT_ROW_{n:03}"),
                num: Some(n),
                logo: None,
                group_id: "g1".into(),
                url: "/tmp/no-video".into(),
            })
            .collect();
        a.set_playlist(playlist);
        a.set_category(BrowserId::Live, group("g1"));
        let ctx = egui::Context::default();
        skjalf::theme::apply(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height));
        let mut output = None;
        for frame in 0..4 {
            output = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(f64::from(frame) * 0.1),
                    ..Default::default()
                },
                |ctx| skjalf::views::live::show(ctx, &mut a),
            ));
        }
        let mut texts = Vec::new();
        let mut row_clips = Vec::new();
        for clipped in &output.unwrap().shapes {
            let before = texts.len();
            text_rects(&clipped.shape, &mut texts);
            row_clips.extend(
                texts[before..]
                    .iter()
                    .filter(|(text, _)| text.contains("HINT_ROW_"))
                    .map(|_| clipped.clip_rect),
            );
        }
        assert!(texts.iter().all(|(text, _)| !text.contains('\u{fffd}')));
        let (_, strip) = texts
            .iter()
            .find(|(text, _)| text.contains("Y Favorite"))
            .unwrap_or_else(|| panic!("no hint strip at {width}x{height}"));
        assert!(screen.contains_rect(*strip), "{strip:?} in {screen:?}");
        assert!(!row_clips.is_empty());
        for clip in row_clips {
            assert!(clip.max.y <= strip.min.y, "{clip:?} over {strip:?}");
        }
    }
}

#[test]
fn empty_favorites_text_names_the_item_and_the_button() {
    let mut a = app("empty_favorites_live");
    a.set_category(BrowserId::Live, CategorySel::Favorites);
    let (labels, _) = render_labels(&mut a, skjalf::views::live::show);
    assert!(labels
        .iter()
        .any(|text| text == "No favorites yet. Highlight a channel and press Y."));

    let (mut a, _, _) = movies_app("empty_favorites_vod");
    open_movies(&mut a);
    a.set_category(BrowserId::Movies, CategorySel::Favorites);
    let (labels, _) = render_labels(&mut a, skjalf::views::movies::show);
    assert!(labels
        .iter()
        .any(|text| text == "No favorites yet. Highlight a movie and press Y."));
    open_series(&mut a);
    a.set_category(BrowserId::Series, CategorySel::Favorites);
    let (labels, _) = render_labels(&mut a, skjalf::views::series::show);
    assert!(labels
        .iter()
        .any(|text| text == "No favorites yet. Highlight a series and press Y."));
}

#[test]
fn live_bar_star_is_reachable_and_toggles_the_playing_channel() {
    let mut a = app("bar_star");
    a.engine = Some(Box::new(MockEngine::default()));
    a.handle_action(Action::Select, 0);
    assert_eq!(a.router.current(), View::Player);
    let next = skjalf::app::bar_controls(false)
        .iter()
        .position(|control| *control == BarControl::Next)
        .unwrap();
    a.player_focus = PlayerFocus::Controls(next);
    a.handle_action(Action::Right, 0);
    assert_eq!(a.player_focus, PlayerFocus::Controls(next + 1));
    assert!(!a.playing_is_favorite());
    a.handle_action(Action::Select, 0);
    assert!(a.favorites.contains(a.source_id().unwrap(), "1"));
    assert!(a.playing_is_favorite());
    a.handle_action(Action::Select, 0);
    assert!(!a.playing_is_favorite());
    a.handle_action(Action::Left, 0);
    assert_eq!(a.player_focus, PlayerFocus::Controls(next));
}

#[test]
fn clicking_a_result_ends_search_editing_in_every_browser() {
    let (mut a, _, _) = movies_app("click_ends_edit");
    open_movies(&mut a);
    all_movies(&mut a);
    a.start_search_edit(BrowserId::Movies);
    assert!(a.search_editing());
    a.click_item(BrowserId::Movies, 0, 0);
    assert_eq!(a.router.current(), View::MovieDetails);
    assert!(!a.search_editing());
    a.handle_action(Action::Select, 1);
    assert_eq!(a.router.current(), View::Player, "Select reaches the page");
    a.handle_action(Action::Back, 2);
    a.handle_action(Action::Back, 3);

    open_series(&mut a);
    a.start_search_edit(BrowserId::Series);
    a.click_item(BrowserId::Series, 0, 4);
    assert_eq!(a.router.current(), View::SeriesDetails);
    assert!(!a.search_editing());

    a.handle_action(Action::Back, 5);
    a.start_group_filter_edit(BrowserId::Series);
    assert!(a.group_filter_editing());
    a.end_text_edit();
    assert!(!a.group_filter_editing());
}

#[test]
fn touching_another_episode_or_season_drops_the_resume_choice() {
    let (mut a, _, engine) = movies_app("stale_choice");
    open_series(&mut a);
    open_series_details(&mut a);
    a.select_season(1);
    a.series_details.as_mut().unwrap().focus = SeriesFocus::Episodes;
    watch_episode(&mut a, &engine, 900.0, 0);
    a.handle_action(Action::Select, 5_000);
    assert!(a.series_details.as_ref().unwrap().choice.is_some());
    a.click_episode(1, 6_000);
    assert_eq!(a.router.current(), View::Player);
    assert!(a.series_details.as_ref().unwrap().choice.is_none());
    a.handle_action(Action::Back, 7_000);
    assert_eq!(a.router.current(), View::SeriesDetails);
    assert!(a.series_details.as_ref().unwrap().choice.is_none());

    a.series_details.as_mut().unwrap().episode = 0;
    a.handle_action(Action::Select, 8_000);
    assert!(a.series_details.as_ref().unwrap().choice.is_some());
    a.select_season(0);
    assert!(a.series_details.as_ref().unwrap().choice.is_none());
}

#[test]
fn window_title_and_main_header_read_skjalf_tv() {
    assert_eq!(skjalf::APP_NAME, "Skjalf TV");
    let mut a = main_app("main_header");
    let (labels, _) = render_labels(&mut a, skjalf::views::main_menu::show);
    assert!(labels.iter().any(|text| text == "Skjalf TV"), "{labels:?}");
}

#[test]
fn an_app_without_a_store_reports_storage_unavailable_on_save() {
    let blocker = std::env::temp_dir().join(format!("skjalf-no-store-{}", std::process::id()));
    std::fs::write(&blocker, "").unwrap();
    let mut a = App::with_store(&blocker.join("skjalf"));
    std::fs::remove_file(&blocker).ok();
    assert!(a.store.is_none());
    a.toggle_sort(BrowserId::Live);
    assert!(a
        .notice
        .as_deref()
        .unwrap_or_default()
        .contains("Storage is unavailable"));
}
