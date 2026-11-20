//! Background catalog loading: one slot per kind, tagged requests, and
//! stale-result filtering. Catalogs never load on the UI thread.

use std::sync::mpsc::Sender;
use std::sync::{Arc, OnceLock};

use skjalf_core::browse::CatalogIndex;
use skjalf_core::catalog::{group_name_order, MovieCatalog, MovieInfo, SeriesCatalog, SeriesInfo};
use skjalf_core::display::DisplayNames;
use skjalf_core::model::{Group, Source, SourceKind};
use skjalf_core::xtream;

use super::{App, BrowserId};

/// A catalog and its search index, built together off the UI thread.
pub struct Loaded<T> {
    pub data: T,
    pub index: CatalogIndex,
    /// Positions of the newest items for the Main screen, newest first.
    pub latest: Vec<usize>,
    /// Group positions in name order, for the sorted sidebar.
    pub group_order: Vec<usize>,
    /// Group names as drawn (right-to-left text in visual order).
    pub group_display: DisplayNames,
}

/// How many Latest items Main shows per row.
pub const LATEST_LIMIT: usize = 12;

/// Positions of the items with the largest timestamps; ties keep catalog order.
fn newest(stamps: impl Iterator<Item = Option<i64>>) -> Vec<usize> {
    let mut keyed: Vec<(i64, usize)> = stamps
        .enumerate()
        .filter_map(|(position, stamp)| Some((stamp?, position)))
        .collect();
    keyed.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    keyed.truncate(LATEST_LIMIT);
    keyed.into_iter().map(|(_, position)| position).collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadStatus {
    NotLoaded,
    Loading,
    Ready,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadOrigin {
    Cache,
    Network,
}

/// One catalog kind. A failed load never discards `loaded`.
pub struct CatalogSlot<T> {
    pub status: LoadStatus,
    pub loaded: Option<Arc<Loaded<T>>>,
    /// Bumps on every install; part of the browser filter cache key.
    pub installed: u64,
    request: u64,
    origin: Option<LoadOrigin>,
}

impl<T> Default for CatalogSlot<T> {
    fn default() -> Self {
        Self {
            status: LoadStatus::NotLoaded,
            loaded: None,
            installed: 0,
            request: 0,
            origin: None,
        }
    }
}

impl<T> CatalogSlot<T> {
    pub fn index(&self) -> &CatalogIndex {
        static EMPTY: OnceLock<CatalogIndex> = OnceLock::new();
        self.loaded
            .as_ref()
            .map_or_else(|| EMPTY.get_or_init(CatalogIndex::default), |l| &l.index)
    }

    pub fn is_loading(&self) -> bool {
        self.status == LoadStatus::Loading
    }

    /// Network requests may replace a running cache read; nothing replaces
    /// a running network request.
    fn can_start(&self, origin: LoadOrigin) -> bool {
        !self.is_loading()
            || (self.origin == Some(LoadOrigin::Cache) && origin == LoadOrigin::Network)
    }

    fn begin(&mut self, origin: LoadOrigin) -> u64 {
        self.request += 1;
        self.status = LoadStatus::Loading;
        self.origin = Some(origin);
        self.request
    }

    fn accepts(&self, request: u64) -> bool {
        self.request == request
    }

    fn install(&mut self, loaded: Loaded<T>) {
        self.loaded = Some(Arc::new(loaded));
        self.installed += 1;
        self.status = LoadStatus::Ready;
        self.origin = None;
    }

    /// Forget everything for a new source; in-flight results become stale.
    pub fn reset(&mut self) {
        self.request += 1;
        self.status = LoadStatus::NotLoaded;
        self.loaded = None;
        self.installed += 1;
        self.origin = None;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobTag {
    pub source_id: String,
    pub request: u64,
}

pub enum CatalogMsg {
    Movies {
        tag: JobTag,
        origin: LoadOrigin,
        result: Result<Option<Loaded<MovieCatalog>>, String>,
    },
    MovieInfo {
        tag: JobTag,
        movie_id: String,
        result: Result<MovieInfo, String>,
    },
    Series {
        tag: JobTag,
        origin: LoadOrigin,
        result: Result<Option<Loaded<SeriesCatalog>>, String>,
    },
    SeriesInfo {
        tag: JobTag,
        series_id: String,
        result: Result<SeriesInfo, String>,
    },
}

pub fn build_series_index(catalog: SeriesCatalog) -> Loaded<SeriesCatalog> {
    let index = CatalogIndex::build(
        catalog
            .series
            .iter()
            .map(|item| (item.id.as_str(), item.name.as_str(), item.group_id.as_str())),
    );
    let latest = newest(catalog.series.iter().map(|item| item.last_modified));
    let group_order = group_name_order(&catalog.groups);
    let group_display = DisplayNames::build(catalog.groups.iter().map(|g| g.name.as_str()));
    Loaded {
        data: catalog,
        index,
        latest,
        group_order,
        group_display,
    }
}

pub fn build_movie_index(catalog: MovieCatalog) -> Loaded<MovieCatalog> {
    let index = CatalogIndex::build(catalog.movies.iter().map(|movie| {
        (
            movie.id.as_str(),
            movie.name.as_str(),
            movie.group_id.as_str(),
        )
    }));
    let latest = newest(catalog.movies.iter().map(|movie| movie.added));
    let group_order = group_name_order(&catalog.groups);
    let group_display = DisplayNames::build(catalog.groups.iter().map(|g| g.name.as_str()));
    Loaded {
        data: catalog,
        index,
        latest,
        group_order,
        group_display,
    }
}

fn xtream_source(source: &Option<Source>) -> Option<Source> {
    source
        .as_ref()
        .filter(|source| matches!(source.kind, SourceKind::Xtream { .. }))
        .cloned()
}

impl App {
    pub fn has_xtream_source(&self) -> bool {
        xtream_source(&self.settings.source).is_some()
    }

    pub fn series_groups(&self) -> &[Group] {
        self.series_slot
            .loaded
            .as_ref()
            .map_or(&[], |loaded| loaded.data.groups.as_slice())
    }

    pub fn movie_groups(&self) -> &[Group] {
        self.movie_slot
            .loaded
            .as_ref()
            .map_or(&[], |loaded| loaded.data.groups.as_slice())
    }

    /// Read the on-disk catalogs in the background. No network.
    pub(super) fn start_cache_loads(&mut self) {
        self.start_movie_load(LoadOrigin::Cache);
        self.start_series_load(LoadOrigin::Cache);
    }

    pub fn start_series_load(&mut self, origin: LoadOrigin) {
        let Some(source) = xtream_source(&self.settings.source) else {
            return;
        };
        if !self.series_slot.can_start(origin) {
            return;
        }
        let tag = JobTag {
            source_id: source.id.clone(),
            request: self.series_slot.begin(origin),
        };
        let fetcher = self.fetcher.clone();
        let store = self.store.clone();
        let tx = self.catalog_tx.clone();
        std::thread::spawn(move || {
            let result = match origin {
                LoadOrigin::Cache => Ok(store
                    .and_then(|store| store.load_series(&source.id))
                    .map(build_series_index)),
                LoadOrigin::Network => xtream::series_catalog(fetcher.as_ref(), &source)
                    .map_err(|error| error.to_string())
                    .map(|catalog| {
                        if let Some(store) = &store {
                            let _ = store.save_series(&catalog);
                        }
                        Some(build_series_index(catalog))
                    }),
            };
            let _ = tx.send(CatalogMsg::Series {
                tag,
                origin,
                result,
            });
        });
    }

    pub fn start_movie_load(&mut self, origin: LoadOrigin) {
        let Some(source) = xtream_source(&self.settings.source) else {
            return;
        };
        if !self.movie_slot.can_start(origin) {
            return;
        }
        let tag = JobTag {
            source_id: source.id.clone(),
            request: self.movie_slot.begin(origin),
        };
        let fetcher = self.fetcher.clone();
        let store = self.store.clone();
        let tx = self.catalog_tx.clone();
        std::thread::spawn(move || {
            let result = match origin {
                LoadOrigin::Cache => Ok(store
                    .and_then(|store| store.load_movies(&source.id))
                    .map(build_movie_index)),
                LoadOrigin::Network => xtream::movie_catalog(fetcher.as_ref(), &source)
                    .map_err(|error| error.to_string())
                    .map(|catalog| {
                        if let Some(store) = &store {
                            let _ = store.save_movies(&catalog);
                        }
                        Some(build_movie_index(catalog))
                    }),
            };
            let _ = tx.send(CatalogMsg::Movies {
                tag,
                origin,
                result,
            });
        });
    }

    /// Start a network load when a catalog view opens with nothing to show.
    pub(super) fn ensure_catalog(&mut self, id: BrowserId) {
        if !self.has_xtream_source() {
            return;
        }
        match id {
            BrowserId::Movies
                if self.movie_slot.loaded.is_none()
                    && self.movie_slot.status == LoadStatus::NotLoaded =>
            {
                self.start_movie_load(LoadOrigin::Network);
            }
            BrowserId::Series
                if self.series_slot.loaded.is_none()
                    && self.series_slot.status == LoadStatus::NotLoaded =>
            {
                self.start_series_load(LoadOrigin::Network);
            }
            _ => {}
        }
    }

    /// Forget a failed load so the next open retries.
    pub(super) fn retry_catalog(&mut self, id: BrowserId) {
        match id {
            BrowserId::Movies
                if self.movie_slot.loaded.is_none()
                    && matches!(self.movie_slot.status, LoadStatus::Failed(_)) =>
            {
                self.movie_slot.status = LoadStatus::NotLoaded;
            }
            BrowserId::Series
                if self.series_slot.loaded.is_none()
                    && matches!(self.series_slot.status, LoadStatus::Failed(_)) =>
            {
                self.series_slot.status = LoadStatus::NotLoaded;
            }
            _ => {}
        }
        self.ensure_catalog(id);
    }

    pub(super) fn reset_catalogs(&mut self) {
        self.movie_slot.reset();
        self.movie_details = None;
        self.movies = Default::default();
        self.movies.sort = self.settings.sort_modes.movies;
        self.refresh_browser(BrowserId::Movies);
        self.series_slot.reset();
        self.series_details = None;
        self.refresh_continue_items();
        self.series = Default::default();
        self.series.sort = self.settings.sort_modes.series;
        self.refresh_browser(BrowserId::Series);
        self.start_cache_loads();
    }

    pub(super) fn poll_catalog_jobs(&mut self) {
        while let Ok(message) = self.catalog_rx.try_recv() {
            match message {
                CatalogMsg::Movies {
                    tag,
                    origin,
                    result,
                } => self.apply_movie_result(tag, origin, result),
                CatalogMsg::MovieInfo {
                    tag,
                    movie_id,
                    result,
                } => self.apply_movie_info(tag, &movie_id, result),
                CatalogMsg::Series {
                    tag,
                    origin,
                    result,
                } => self.apply_series_result(tag, origin, result),
                CatalogMsg::SeriesInfo {
                    tag,
                    series_id,
                    result,
                } => self.apply_series_info(tag, &series_id, result),
            }
        }
        match self.router.current() {
            skjalf_core::nav::View::Movies => self.ensure_catalog(BrowserId::Movies),
            skjalf_core::nav::View::Series => self.ensure_catalog(BrowserId::Series),
            _ => {}
        }
    }

    fn apply_movie_result(
        &mut self,
        tag: JobTag,
        origin: LoadOrigin,
        result: Result<Option<Loaded<MovieCatalog>>, String>,
    ) {
        if self.source_id() != Some(tag.source_id.as_str()) || !self.movie_slot.accepts(tag.request)
        {
            return;
        }
        match result {
            Ok(Some(loaded)) => {
                self.movie_slot.install(loaded);
                self.after_catalog_install(BrowserId::Movies);
                match origin {
                    LoadOrigin::Network => self.record_refresh(super::RefreshKind::Movies),
                    LoadOrigin::Cache => {
                        if self.auto_refresh && self.refresh_is_due(super::RefreshKind::Movies) {
                            self.start_movie_load(LoadOrigin::Network);
                        }
                    }
                }
            }
            Ok(None) => self.movie_slot.status = LoadStatus::NotLoaded,
            Err(message) => {
                self.movie_slot.status = LoadStatus::Failed(message);
                self.movie_slot.origin = None;
            }
        }
    }

    fn apply_series_result(
        &mut self,
        tag: JobTag,
        origin: LoadOrigin,
        result: Result<Option<Loaded<SeriesCatalog>>, String>,
    ) {
        if self.source_id() != Some(tag.source_id.as_str())
            || !self.series_slot.accepts(tag.request)
        {
            return;
        }
        match result {
            Ok(Some(loaded)) => {
                self.series_slot.install(loaded);
                self.after_catalog_install(BrowserId::Series);
                match origin {
                    LoadOrigin::Network => self.record_refresh(super::RefreshKind::Series),
                    LoadOrigin::Cache => {
                        if self.auto_refresh && self.refresh_is_due(super::RefreshKind::Series) {
                            self.start_series_load(LoadOrigin::Network);
                        }
                    }
                }
            }
            Ok(None) => self.series_slot.status = LoadStatus::NotLoaded,
            Err(message) => {
                self.series_slot.status = LoadStatus::Failed(message);
                self.series_slot.origin = None;
            }
        }
    }

    fn after_catalog_install(&mut self, id: BrowserId) {
        let groups_known = |app: &App, group_id: &str| {
            app.browser_groups(id)
                .iter()
                .any(|group| group.id == group_id)
        };
        if let skjalf_core::browse::CategorySel::Group(group_id) = self.state(id).category.clone() {
            if !groups_known(self, &group_id) {
                self.state_mut(id)
                    .set_category(skjalf_core::browse::CategorySel::All);
            }
        }
        self.refresh_browser(id);
        self.rebuild_continue_art();
    }

    pub(super) fn request_series_info(&mut self, series_id: &str, request: u64) {
        let Some(source) = xtream_source(&self.settings.source) else {
            return;
        };
        let tag = JobTag {
            source_id: source.id.clone(),
            request,
        };
        let fetcher = self.fetcher.clone();
        let tx: Sender<CatalogMsg> = self.catalog_tx.clone();
        let series_id = series_id.to_owned();
        std::thread::spawn(move || {
            let result = xtream::series_info(fetcher.as_ref(), &source, &series_id)
                .map_err(|error| error.to_string());
            let _ = tx.send(CatalogMsg::SeriesInfo {
                tag,
                series_id,
                result,
            });
        });
    }

    pub(super) fn request_movie_info(&mut self, movie_id: &str, request: u64) {
        let Some(source) = xtream_source(&self.settings.source) else {
            return;
        };
        let tag = JobTag {
            source_id: source.id.clone(),
            request,
        };
        let fetcher = self.fetcher.clone();
        let tx: Sender<CatalogMsg> = self.catalog_tx.clone();
        let movie_id = movie_id.to_owned();
        std::thread::spawn(move || {
            let result = xtream::movie_info(fetcher.as_ref(), &source, &movie_id)
                .map_err(|error| error.to_string());
            let _ = tx.send(CatalogMsg::MovieInfo {
                tag,
                movie_id,
                result,
            });
        });
    }
}
