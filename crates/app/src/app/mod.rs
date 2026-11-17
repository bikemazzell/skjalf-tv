//! Application state: store + playlist + navigation + player wiring.
//!
//! All state mutations flow through [`App::handle_action`], which is pure
//! with respect to the injected engine and clock — unit-testable headlessly.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use skjalf_core::browse::{BrowserState, CatalogIndex};
use skjalf_core::catalog::{ContentKey, MovieCatalog, SeriesCatalog};
use skjalf_core::display::DisplayNames;
use skjalf_core::model::{
    Activation, Channel, EpgEntry, Favorites, Playlist, Settings, Source, SourceKind,
};
use skjalf_core::nav::{step_channel, OsdKind, PlayerModel, Router, View, ZapBuffer};
use skjalf_core::progress::{MarkerIndex, Progress, ProgressEntry};
use skjalf_core::store::Store;
use skjalf_core::xtream::HttpFetcher;
use skjalf_core::{m3u, template, xtream};

use crate::input::Action;
use crate::net::UreqFetcher;
use crate::player::{PlayerEngine, PlayerEvent};
use crate::setup_model::{Field, Row, SetupModel};
use skjalf_core::browse::BrowserFocus;

mod browser;
mod catalog_jobs;
mod main_menu;
mod movies;
mod playback;
mod progress;
mod refresh;
mod series;
mod settings;
mod transport;

pub use browser::ItemRow;
pub use catalog_jobs::{CatalogSlot, LoadOrigin, LoadStatus, Loaded};
pub use main_menu::{rating_detail, MainButton, MainRow, RowTile};
pub use movies::{DetailInfo, MovieButton, MovieDetails};
pub use playback::{EpisodeRef, OnDemand, Playback};
pub use refresh::{unix_now, RefreshKind};
pub use series::{ResumeChoice, SeriesDetails, SeriesFocus};
pub use settings::SettingsFocus;
pub use transport::{bar_controls, BarControl, MediaClock};

/// Group id of the pinned Favorites pseudo-group.
pub const FAVORITES_GROUP: &str = "\u{2605} Favorites";

/// How long a fetched EPG stays fresh before a background refresh.
pub const EPG_TTL_MS: u64 = 60_000;

/// A background EPG fetch result, tagged with the channel it was issued for.
pub type EpgResult = (String, Result<Vec<EpgEntry>, String>);

/// Result of the background connect flow.
pub enum ConnectOutcome {
    Ready(Playlist),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaybackStatus {
    Idle,
    Loading,
    Playing,
    Failed(String),
}

/// A browser that owns a search field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BrowserId {
    Live,
    Movies,
    Series,
}

/// The text field that currently receives raw key and text events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextTarget {
    SetupField(Field),
    Search(BrowserId),
    /// The sidebar's "Filter groups" field.
    GroupFilter(BrowserId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayerFocus {
    #[default]
    Video,
    /// An index into [`transport::bar_controls`].
    Controls(usize),
    /// The on-demand seek bar.
    Seek,
}

/// The whole app; one instance lives for the process lifetime.
pub struct App {
    pub store: Option<Store>,
    pub settings: Settings,
    pub favorites: Favorites,
    pub playlist: Option<Playlist>,
    pub router: Router,
    pub live: BrowserState,
    group_lists: [browser::GroupList; 3],
    /// Visible rows per page for L1/R1 jumps: `[sidebar, items]` per browser.
    pub page_rows: [[usize; 2]; 3],
    pub movies: BrowserState,
    pub series: BrowserState,
    pub movie_slot: CatalogSlot<MovieCatalog>,
    pub series_slot: CatalogSlot<SeriesCatalog>,
    pub series_details: Option<SeriesDetails>,
    /// Last rendered series-details selection; drives scroll-into-view.
    pub last_episode_sel: Option<(usize, usize)>,
    pub movie_details: Option<MovieDetails>,
    details_request: u64,
    catalog_tx: Sender<catalog_jobs::CatalogMsg>,
    catalog_rx: Receiver<catalog_jobs::CatalogMsg>,
    pub playback: Playback,
    pub paused: bool,
    /// Muted for this session; the saved volume is untouched.
    pub muted: bool,
    /// Status line over the video with its expiry time (ms).
    pub status: Option<(String, u64)>,
    pub media: MediaClock,
    media_ended: bool,
    last_progress_save_ms: u64,
    progress_save_failed: bool,
    pub progress: Progress,
    /// Continue Watching entries for the current source, newest first.
    pub continue_items: Vec<ProgressEntry>,
    /// Continue Watching tile titles as drawn, rebuilt with the items.
    pub continue_titles: Vec<String>,
    /// Watched and in-progress markers, rebuilt when progress changes.
    pub markers: MarkerIndex,
    /// Unix seconds; injectable for tests.
    pub clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    /// Interval refreshes run only when the shell enables them.
    pub auto_refresh: bool,
    next_refresh_check: u64,
    live_index: CatalogIndex,
    live_group_order: Vec<usize>,
    live_group_display: DisplayNames,
    catalog_generation: u64,
    favorites_generation: u64,
    /// Last rendered Live selection; drives scroll-into-view on change.
    pub last_browser_sel: [Option<(usize, usize, usize)>; 3],
    pub main_focus: MainButton,
    /// The Main content row holding focus, or `None` for the buttons.
    pub main_row: Option<MainRow>,
    main_row_sel: [usize; 3],
    /// Last rendered Main focus; drives scroll-into-view when it changes or
    /// returns after another screen.
    pub last_main_scroll_key: Option<(MainButton, Option<MainRow>, usize)>,
    /// Buttons per row in the last rendered Main layout; drives Up/Down.
    pub main_columns: usize,
    pub settings_focus: SettingsFocus,
    /// Outcome of the last connect or refresh, shown in Settings.
    pub refresh_status: Option<String>,
    pub player: PlayerModel,
    pub player_focus: PlayerFocus,
    pub playback_status: PlaybackStatus,
    /// Feedback shown on the current view until the next operation replaces it.
    pub notice: Option<String>,
    pub zap: ZapBuffer,
    pub engine: Option<Box<dyn PlayerEngine>>,
    pub setup: SetupModel,
    pub editing_source: bool,
    editing: Option<TextTarget>,
    /// Set when editing ends; the UI layer drops egui text focus.
    pub text_focus_release: bool,
    focus_request: Option<TextTarget>,
    pub game_mode: bool,
    pub keyboard_launcher: Arc<dyn crate::steam_keyboard::KeyboardLauncher>,
    steam_keyboard_open: bool,
    steam_keyboard_rx: Option<Receiver<Result<(), String>>>,
    steam_keyboard_dismiss_rx: Option<Receiver<()>>,
    steam_keyboard_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// HTTP backend shared by background workers (injectable in tests).
    pub fetcher: Arc<dyn HttpFetcher + Send + Sync>,
    /// Background playlist fetch (setup connect).
    pub connect_rx: Option<Receiver<ConnectOutcome>>,
    /// Background EPG fetch; carries the channel id it was issued for.
    pub epg_rx: Option<Receiver<EpgResult>>,
    pub epg: Vec<EpgEntry>,
    /// Channel + time of the last EPG fetch (TTL guard).
    pub epg_fetched: Option<(String, u64)>,
    /// Background gamepad reader (None on machines without a gamepad).
    pub gamepad: Option<crate::gamepad::GamepadThread>,
    /// Set when an action requests app termination; the eframe layer polls it.
    pub exit_requested: bool,
}

impl App {
    /// The app over a store directory, without a player engine. A directory
    /// that cannot be opened leaves the app without storage for the session.
    pub fn with_store(dir: &std::path::Path) -> Self {
        let store = Store::open(dir).ok();
        let settings = store
            .as_ref()
            .and_then(|s| s.load_settings().ok())
            .unwrap_or_default();
        let favorites = store
            .as_ref()
            .and_then(|s| s.load_favorites().ok())
            .unwrap_or_default();
        let cached_playlist = settings
            .source
            .as_ref()
            .and_then(|src| store.as_ref().and_then(|s| s.load_playlist(&src.id)));
        let initial = if cached_playlist.is_some() {
            View::Main
        } else {
            View::Setup
        };
        let (catalog_tx, catalog_rx) = mpsc::channel();
        let mut app = Self {
            store,
            fetcher: Arc::new(UreqFetcher::new(settings.ignore_ssl)),
            settings,
            favorites,
            playlist: None,
            router: Router::new(initial),
            live: BrowserState::default(),
            group_lists: Default::default(),
            page_rows: [[8, 8]; 3],
            movies: BrowserState::default(),
            series: BrowserState::default(),
            movie_slot: CatalogSlot::default(),
            series_slot: CatalogSlot::default(),
            series_details: None,
            last_episode_sel: None,
            movie_details: None,
            details_request: 0,
            catalog_tx,
            catalog_rx,
            playback: Playback::default(),
            paused: false,
            muted: false,
            status: None,
            media: MediaClock::default(),
            media_ended: false,
            last_progress_save_ms: 0,
            progress_save_failed: false,
            progress: Progress::default(),
            continue_items: Vec::new(),
            continue_titles: Vec::new(),
            markers: MarkerIndex::default(),
            clock: Arc::new(unix_now),
            auto_refresh: false,
            next_refresh_check: 0,
            live_index: CatalogIndex::default(),
            live_group_order: Vec::new(),
            live_group_display: DisplayNames::default(),
            catalog_generation: 0,
            favorites_generation: 0,
            last_browser_sel: [None; 3],
            main_focus: MainButton::default(),
            main_row: None,
            main_row_sel: [0; 3],
            last_main_scroll_key: None,
            main_columns: MainButton::ALL.len(),
            settings_focus: SettingsFocus::default(),
            refresh_status: None,
            player: PlayerModel::default(),
            player_focus: PlayerFocus::default(),
            playback_status: PlaybackStatus::Idle,
            notice: None,
            zap: ZapBuffer::default(),
            engine: None,
            setup: SetupModel::default(),
            editing_source: false,
            editing: None,
            text_focus_release: false,
            focus_request: None,
            game_mode: false,
            keyboard_launcher: Arc::new(crate::steam_keyboard::SteamUriLauncher),
            steam_keyboard_open: false,
            steam_keyboard_rx: None,
            steam_keyboard_dismiss_rx: None,
            steam_keyboard_cancel: None,
            connect_rx: None,
            epg_rx: None,
            epg: Vec::new(),
            epg_fetched: None,
            gamepad: None,
            exit_requested: false,
        };
        app.live.sort = app.settings.sort_modes.live;
        app.movies.sort = app.settings.sort_modes.movies;
        app.series.sort = app.settings.sort_modes.series;
        app.progress = app
            .store
            .as_ref()
            .map(|store| store.load_progress())
            .unwrap_or_default();
        app.refresh_continue_items();
        app.start_cache_loads();
        if let Some(playlist) = cached_playlist {
            app.set_playlist(playlist);
        }
        app.select_startup_group();
        if let Some((source_id, channel_id)) = app.settings.last_channel.clone() {
            if app.source_id() == Some(source_id.as_str())
                && app
                    .playlist
                    .as_ref()
                    .is_some_and(|pl| pl.find(&channel_id).is_some())
            {
                app.sync_live_to_channel(&channel_id);
            }
        }
        app
    }

    /// Production constructor: real store + mpv engine.
    pub fn new(store_dir: &std::path::Path, initial_volume: u8) -> Self {
        let mut app = Self::with_store(store_dir);
        let ignore_ssl = app.settings.ignore_ssl;
        if let Ok(engine) = crate::player::MpvEngine::new(initial_volume, ignore_ssl) {
            app.engine = Some(Box::new(engine));
        }
        app
    }

    /// Current source id (playlist cache key).
    pub fn source_id(&self) -> Option<&str> {
        self.settings.source.as_ref().map(|s| s.id.as_str())
    }

    /// Feed one raw egui input event through the action pipeline.
    ///
    /// While a text target is active, typing (keyboard/paste) goes straight
    /// into its field instead of triggering global shortcuts (`q` types a
    /// "q"; it does not quit the app).
    pub fn pump_event(&mut self, event: &egui::Event, now_ms: u64) -> bool {
        if self.editing.is_some() {
            if let egui::Event::Key {
                key, pressed: true, ..
            } = event
            {
                if matches!(
                    key,
                    egui::Key::Enter | egui::Key::Escape | egui::Key::BrowserBack
                ) {
                    self.finish_text_edit();
                    return true;
                }
            }
            return false;
        }
        if self.router.current() == View::Setup {
            if let egui::Event::Key {
                key, pressed: true, ..
            } = event
            {
                if matches!(
                    key,
                    egui::Key::ArrowUp
                        | egui::Key::ArrowDown
                        | egui::Key::ArrowLeft
                        | egui::Key::ArrowRight
                        | egui::Key::Enter
                        | egui::Key::Escape
                        | egui::Key::BrowserBack
                ) {
                    if let Some(action) = crate::input::event_to_action(event) {
                        self.handle_action(action, now_ms);
                    }
                    return true;
                }
            }
            return false;
        }
        if let Some(action) = crate::input::event_to_action(event) {
            self.handle_action(action, now_ms);
            return true;
        }
        false
    }

    /// Apply one user action for the current view.
    pub fn handle_action(&mut self, action: Action, now_ms: u64) {
        if self.editing.is_some() {
            match action {
                Action::Back => self.finish_text_edit(),
                Action::Quit => self.exit_requested = true,
                _ => {}
            }
            return;
        }
        if action == Action::Quit {
            self.save_progress(now_ms);
            self.close_steam_keyboard();
            self.exit_requested = true;
            return;
        }
        match self.router.current() {
            View::Setup => self.handle_setup(action, now_ms),
            View::Main => self.handle_main(action),
            View::Live => self.handle_browser(BrowserId::Live, action, now_ms),
            View::Movies => self.handle_browser(BrowserId::Movies, action, now_ms),
            View::MovieDetails => self.handle_movie_details(action, now_ms),
            View::Series => self.handle_browser(BrowserId::Series, action, now_ms),
            View::SeriesDetails => self.handle_series_details(action, now_ms),
            View::Settings => self.handle_settings(action),
            View::Player => self.handle_player(action, now_ms),
        }
        if self.router.current() != View::Main {
            self.last_main_scroll_key = None;
        }
        self.drop_stale_zap();
    }

    /// Buffered channel digits belong to Live and Player only.
    fn drop_stale_zap(&mut self) {
        if !matches!(self.router.current(), View::Live | View::Player) {
            self.zap = ZapBuffer::default();
        }
    }

    fn handle_setup(&mut self, action: Action, _now_ms: u64) {
        if self.connect_rx.is_some() {
            return; // busy connecting
        }
        if action == Action::Back && self.editing_source {
            self.close_steam_keyboard();
            self.editing_source = false;
            self.router.back();
            self.notice = Some("Source edit canceled.".into());
            return;
        }
        let opening_field = if action == Action::Select {
            match self.setup.row {
                Row::Field(index) => self.setup.fields().get(index).copied(),
                _ => None,
            }
        } else {
            None
        };
        if let Some(crate::setup_model::Connect(source)) = self.setup.input(action) {
            self.start_connect(source);
        } else if let Some(field) = opening_field {
            self.activate_setup_field(field);
        }
    }

    pub fn activate_setup_field(&mut self, field: Field) {
        if self.router.current() != View::Setup || self.connect_rx.is_some() {
            return;
        }
        self.setup.activate_field(field);
        if self.editing != Some(TextTarget::SetupField(field)) {
            self.editing = Some(TextTarget::SetupField(field));
            self.open_steam_keyboard();
        }
    }

    pub fn setup_field_editing(&self) -> bool {
        matches!(self.editing, Some(TextTarget::SetupField(_)))
    }

    pub fn group_filter_editing(&self) -> bool {
        matches!(self.editing, Some(TextTarget::GroupFilter(_)))
    }

    pub fn search_editing(&self) -> bool {
        matches!(self.editing, Some(TextTarget::Search(_)))
    }

    pub fn active_steam_keyboard_field(&self) -> Option<Field> {
        match self.editing {
            Some(TextTarget::SetupField(field)) if self.game_mode && self.steam_keyboard_open => {
                Some(field)
            }
            _ => None,
        }
    }

    /// End the active text edit. Closing the field releases egui focus; a
    /// search hands control back to its results.
    fn finish_text_edit(&mut self) {
        let target = self.editing.take();
        self.text_focus_release = true;
        self.close_steam_keyboard();
        match target {
            Some(TextTarget::Search(id)) => {
                let state = self.state_mut(id);
                state.focus = if state.filtered.is_empty() {
                    BrowserFocus::Search
                } else {
                    BrowserFocus::Items
                };
            }
            Some(TextTarget::GroupFilter(id)) => self.focus_first_matching_group(id),
            _ => {}
        }
    }

    /// A click or touch elsewhere ends any active search or filter edit.
    pub fn end_text_edit(&mut self) {
        if matches!(
            self.editing,
            Some(TextTarget::Search(_) | TextTarget::GroupFilter(_))
        ) {
            self.finish_text_edit();
        }
    }

    pub fn finish_setup_edit(&mut self) {
        self.finish_text_edit();
    }

    /// Spawn the background connect flow (login + playlist or M3U parse).
    pub fn start_connect(&mut self, source: Source) {
        if self.connect_rx.is_some() {
            return;
        }
        if self.setup_field_editing() {
            self.finish_text_edit();
        }
        let fetcher = self.fetcher.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let outcome = match &source.kind {
                SourceKind::Xtream { .. } => match xtream::login(fetcher.as_ref(), &source) {
                    Ok(Activation::Active) => match xtream::playlist(fetcher.as_ref(), &source) {
                        Ok(pl) => ConnectOutcome::Ready(pl),
                        Err(e) => ConnectOutcome::Failed(e.to_string()),
                    },
                    Ok(Activation::Expired { date }) => {
                        ConnectOutcome::Failed(format!("Account expired on {date}."))
                    }
                    Ok(Activation::Banned) => ConnectOutcome::Failed("Account banned.".into()),
                    Ok(Activation::Invalid) => {
                        ConnectOutcome::Failed("Invalid username or password.".into())
                    }
                    Err(e) => ConnectOutcome::Failed(e.to_string()),
                },
                SourceKind::M3u { url } => match fetcher.get(url) {
                    Ok(text) => match m3u::parse(source.clone(), &text) {
                        Ok(pl) => ConnectOutcome::Ready(pl),
                        Err(e) => ConnectOutcome::Failed(e.to_string()),
                    },
                    Err(e) => ConnectOutcome::Failed(e.to_string()),
                },
            };
            let _ = tx.send(outcome);
        });
        self.setup.message = Some("Connecting\u{2026}".into());
        self.connect_rx = Some(rx);
    }

    /// Poll background jobs; call once per frame before rendering.
    pub fn poll_jobs(&mut self, now_ms: u64) {
        self.poll_keyboard();
        self.poll_non_keyboard_jobs(now_ms);
    }

    /// Process Steam keyboard changes before input routing in each UI frame.
    pub fn poll_keyboard(&mut self) {
        if let Some(rx) = self.steam_keyboard_rx.take() {
            match rx.try_recv() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    self.steam_keyboard_open = false;
                    self.stop_keyboard_monitor();
                    let message = format!("Steam keyboard failed: {error}");
                    self.setup.message = Some(message.clone());
                    self.notice = Some(message);
                }
                Err(mpsc::TryRecvError::Empty) => self.steam_keyboard_rx = Some(rx),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.steam_keyboard_open = false;
                    self.stop_keyboard_monitor();
                }
            }
        }
        if let Some(rx) = self.steam_keyboard_dismiss_rx.take() {
            match rx.try_recv() {
                Ok(()) if self.steam_keyboard_open => {
                    // Steam already closed it; retain the selected field row.
                    self.steam_keyboard_open = false;
                    self.finish_text_edit();
                }
                Ok(()) | Err(mpsc::TryRecvError::Disconnected) => self.stop_keyboard_monitor(),
                Err(mpsc::TryRecvError::Empty) => self.steam_keyboard_dismiss_rx = Some(rx),
            }
        }
    }

    fn stop_keyboard_monitor(&mut self) {
        if let Some(cancel) = self.steam_keyboard_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.steam_keyboard_dismiss_rx = None;
    }

    /// Poll background jobs; call once per frame before rendering.
    pub fn poll_non_keyboard_jobs(&mut self, now_ms: u64) {
        if let Some(rx) = self.connect_rx.take() {
            match rx.try_recv() {
                Ok(outcome) => match outcome {
                    ConnectOutcome::Ready(pl) => {
                        let count = pl.channels.len();
                        let keep_player = self.router.current() == View::Player
                            && self.source_id() == Some(pl.source.id.as_str())
                            && (self.is_on_demand()
                                || self
                                    .player
                                    .channel_id
                                    .as_ref()
                                    .is_some_and(|id| pl.find(id).is_some()));
                        let same_source = self.source_id() == Some(pl.source.id.as_str());
                        self.settings.source = Some(pl.source.clone());
                        self.set_playlist(pl);
                        self.editing_source = false;
                        if keep_player {
                            if let Some(id) = self.player.channel_id.clone() {
                                self.sync_live_to_channel(&id);
                            }
                        } else {
                            if self.router.current() == View::Player {
                                if let Some(engine) = &mut self.engine {
                                    let _ = engine.stop();
                                }
                                self.playback_status = PlaybackStatus::Idle;
                                self.player.channel_id = None;
                                self.router.back();
                            }
                            if self.router.current() == View::Setup {
                                self.router.reset(View::Main);
                            }
                            if !same_source {
                                self.reset_live();
                                self.reset_catalogs();
                            }
                        }
                        self.refresh_status = Some(format!("Last refresh: {count} channels."));
                        self.notify_refresh(format!("Connected. {count} channels available."));
                        self.save_all();
                        self.record_refresh(RefreshKind::Live);
                    }
                    ConnectOutcome::Failed(e) => {
                        self.refresh_status = Some(format!("Last refresh failed: {e}"));
                        self.setup.message = Some(e.clone());
                        self.notify_refresh(e);
                    }
                },
                Err(mpsc::TryRecvError::Disconnected) => {
                    // Worker died without a verdict — unblock setup.
                    self.refresh_status = Some("Last refresh failed.".into());
                    self.setup.message = Some("Connection failed.".into());
                    self.notify_refresh("Connection failed.".into());
                }
                Err(mpsc::TryRecvError::Empty) => self.connect_rx = Some(rx),
            }
        }
        // EPG worker; drop results that no longer match the playing channel.
        if let Some(rx) = self.epg_rx.take() {
            match rx.try_recv() {
                Ok((channel_id, _)) if self.player.channel_id.as_deref() != Some(&channel_id) => {}
                Ok((_, Ok(entries))) => self.epg = entries,
                Ok((_, Err(_))) => self.epg.clear(),
                Err(mpsc::TryRecvError::Disconnected) => {} // retried after TTL
                Err(mpsc::TryRecvError::Empty) => self.epg_rx = Some(rx),
            }
        }
        // Zap buffer auto-commit.
        self.poll_catalog_jobs();
        self.run_refresh_schedule();
        self.drop_stale_zap();
        if let Some(num) = self.zap.take_if_expired(now_ms) {
            self.zap_to(num, now_ms);
        }
        // Async playback failures (mpv end-file errors) surface on the OSD.
        let events = self
            .engine
            .as_mut()
            .map_or_else(Vec::new, |e| e.take_events());
        for event in events {
            match event {
                PlayerEvent::Loaded => {
                    self.playback_status = PlaybackStatus::Playing;
                    if self
                        .notice
                        .as_deref()
                        .is_some_and(|message| message.starts_with("Loading "))
                    {
                        self.notice = None;
                    }
                }
                PlayerEvent::Ended => self.finish_progress(now_ms),
                PlayerEvent::Failed(err) => {
                    let message = format!("Playback error: {err}");
                    self.playback_status = PlaybackStatus::Failed(message.clone());
                    self.notice = Some(message.clone());
                    self.player.show_osd(OsdKind::Zap(message), now_ms);
                }
            }
        }
        self.refresh_media_clock(now_ms);
        self.save_progress_periodically(now_ms);
        // Periodic EPG refresh while watching (TTL-guarded, cheap).
        if self.router.current() == View::Player {
            self.fetch_epg(now_ms);
        }
    }

    fn open_steam_keyboard(&mut self) {
        if self.game_mode && !self.steam_keyboard_open {
            self.steam_keyboard_open = true;
            let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
            self.steam_keyboard_dismiss_rx = self.keyboard_launcher.watch_dismissal(cancel.clone());
            self.steam_keyboard_cancel = Some(cancel);
            self.steam_keyboard_rx =
                Some(self.keyboard_launcher.send(crate::steam_keyboard::OPEN_URI));
        }
    }

    fn close_steam_keyboard(&mut self) {
        self.stop_keyboard_monitor();
        if self.steam_keyboard_open {
            self.steam_keyboard_open = false;
            self.steam_keyboard_rx = Some(
                self.keyboard_launcher
                    .send(crate::steam_keyboard::CLOSE_URI),
            );
        }
    }

    pub fn steam_keyboard_active(&self) -> bool {
        self.steam_keyboard_open
    }

    fn save_all(&mut self) {
        // Record the source even when no store is available (headless tests);
        // it drives `{user}`/`{pass}` templating.
        if let Some(pl) = &self.playlist {
            self.settings.source = Some(pl.source.clone());
        }
        let Some(store) = &self.store else {
            self.notice = Some("Storage is unavailable. Changes were not saved.".into());
            return;
        };
        let mut failures = Vec::new();
        if store.save_settings(&self.settings).is_err() {
            failures.push("settings");
        }
        if store.save_favorites(&self.favorites).is_err() {
            failures.push("favorites");
        }
        if let Some(pl) = &self.playlist {
            if store.save_playlist(pl).is_err() {
                failures.push("playlist");
            }
        }
        if !failures.is_empty() {
            self.notice = Some(format!("Could not save {}.", failures.join(", ")));
        }
    }

    fn save_settings(&mut self) {
        if let Some(store) = &self.store {
            if store.save_settings(&self.settings).is_err() {
                self.notice = Some("Could not save settings.".into());
            }
        } else {
            self.notice = Some("Storage is unavailable. Settings were not saved.".into());
        }
    }

    /// Play a channel (templating credentials for Xtream URLs).
    pub fn play(&mut self, channel: &Channel, now_ms: u64) {
        if self.editing.is_some() {
            self.finish_text_edit();
        }
        let url = self.render_template(&channel.url);
        if !self.load_media(&url, None, now_ms) {
            return; // stay on the current view; nothing is playing
        }
        self.playback_status = PlaybackStatus::Loading;
        self.notice = Some(format!("Loading {}…", channel.name));
        self.player.channel_id = Some(channel.id.clone());
        self.playback = Playback::Live;
        if let Some(source_id) = self.source_id() {
            self.settings.last_channel = Some((source_id.to_owned(), channel.id.clone()));
            self.save_settings();
        }
        self.enter_player(now_ms);
        self.sync_live_to_channel(&channel.id);
        self.fetch_epg(now_ms);
    }

    /// Fill `{user}`/`{pass}` placeholders (M3U urls are already final).
    fn render_template(&self, url: &str) -> String {
        if let Some(SourceKind::Xtream {
            username, password, ..
        }) = self.settings.source.as_ref().map(|s| &s.kind)
        {
            template::render(url, username, password)
        } else {
            url.to_owned()
        }
    }

    fn step_channel(&mut self, forward: bool, now_ms: u64) {
        let Some(current) = self.player.channel_id.clone() else {
            return;
        };
        let ids = self.group_channel_ids();
        if let Some(next) = step_channel(&ids, &current, forward) {
            if let Some(ch) = self.playlist.as_ref().and_then(|p| p.find(&next)) {
                let ch = ch.clone();
                let focus = self.player_focus;
                self.play(&ch, now_ms);
                if self.router.current() == View::Player {
                    self.player_focus = focus;
                }
            }
        }
    }

    pub(super) fn adjust_volume(&mut self, delta: i16, now_ms: u64) {
        let previous = self.settings.volume;
        self.settings
            .set_volume((self.settings.volume as i16 + delta).clamp(0, 100) as u8);
        if let Some(engine) = &mut self.engine {
            if let Err(error) = engine.set_volume(self.settings.volume) {
                self.settings.set_volume(previous);
                let message = format!("Volume error: {error}");
                self.notice = Some(message.clone());
                self.player.show_osd(OsdKind::Zap(message), now_ms);
                return;
            }
        }
        self.muted = false;
        self.notice = None;
        self.save_settings();
        self.show_status(format!("Volume {}%", self.settings.volume), now_ms);
    }

    pub fn toggle_favorite(&mut self, key: &ContentKey, name: &str, now_ms: u64) {
        self.toggle_favorite_stored(&key.favorite_key(), name, now_ms);
    }

    fn toggle_favorite_stored(&mut self, stored: &str, name: &str, now_ms: u64) {
        let Some(source_id) = self.source_id().map(str::to_owned) else {
            return;
        };
        self.favorites.toggle(&source_id, stored);
        self.favorites_generation += 1;
        let save_error = if let Some(store) = &self.store {
            store
                .save_favorites(&self.favorites)
                .err()
                .map(|_| "Could not save favorites.")
        } else {
            Some("Storage is unavailable. Favorites were not saved.")
        };
        let star = if self.favorites.contains(&source_id, stored) {
            "Added to"
        } else {
            "Removed from"
        };
        let browsing = matches!(
            self.router.current(),
            View::Live | View::Movies | View::MovieDetails | View::Series | View::SeriesDetails
        );
        self.notice = save_error
            .map(str::to_owned)
            .or_else(|| browsing.then(|| format!("{star} favorites: {name}")));
        self.refresh_browsers();
        self.player.show_osd(
            OsdKind::Zap(format!("\u{2605} {star} favorites \u{2014} {name}")),
            now_ms,
        );
    }

    fn zap_to(&mut self, num: u32, now_ms: u64) {
        if let Some(ch) = self
            .playlist
            .as_ref()
            .and_then(|p| p.find_by_num(num))
            .cloned()
        {
            self.play(&ch, now_ms);
        } else {
            self.notice = Some(format!("No channel {num}."));
            self.player
                .show_osd(OsdKind::Zap(format!("No channel {num}")), now_ms);
        }
    }

    /// Refresh the zap OSD with digits typed so far.
    fn show_zap_osd(&mut self, now_ms: u64) {
        let digits = self.zap.digits().to_owned();
        let name = digits
            .parse::<u32>()
            .ok()
            .and_then(|n| self.playlist.as_ref().and_then(|p| p.find_by_num(n)))
            .map(|c| format!(" \u{00b7} {}", c.name))
            .unwrap_or_default();
        self.player
            .show_osd(OsdKind::Zap(format!("{digits}{name}")), now_ms);
    }

    /// Fetch now/next EPG for the playing channel in the background; the TTL
    /// guard both prevents duplicate workers and re-arms refresh/expiry
    /// (failed fetches retry after [`EPG_TTL_MS`]).
    fn fetch_epg(&mut self, now_ms: u64) {
        let (Some(source), Some(channel_id)) =
            (self.settings.source.clone(), self.player.channel_id.clone())
        else {
            return;
        };
        if let Some((prev, at)) = &self.epg_fetched {
            if prev == &channel_id && now_ms.saturating_sub(*at) < EPG_TTL_MS {
                return; // fresh enough
            }
        }
        let SourceKind::Xtream { .. } = &source.kind else {
            return; // M3U has no EPG API
        };
        let provider_id = self
            .playlist
            .as_ref()
            .and_then(|playlist| playlist.find(&channel_id))
            .and_then(|channel| channel.url.rsplit('/').next())
            .and_then(|name| name.rsplit_once('.'))
            .map_or_else(|| channel_id.clone(), |(id, _)| id.to_owned());
        self.epg_fetched = Some((channel_id.clone(), now_ms));
        let fetcher = self.fetcher.clone();
        let (tx, rx) = mpsc::channel();
        let tagged_channel = channel_id.clone();
        std::thread::spawn(move || {
            let res = xtream::short_epg(fetcher.as_ref(), &source, &provider_id, 4)
                .map_err(|e| e.to_string());
            let _ = tx.send((tagged_channel, res));
        });
        self.epg_rx = Some(rx);
    }

    /// True when the last action requested app exit.
    pub fn wants_exit(&self) -> bool {
        self.exit_requested
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.save_progress(0);
        self.stop_keyboard_monitor();
    }
}
