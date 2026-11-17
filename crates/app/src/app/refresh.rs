//! Background refresh: last-refresh bookkeeping and the interval schedule.

use skjalf_core::nav::View;

use super::{App, LoadOrigin};

/// How often the app checks whether a refresh is due.
const CHECK_INTERVAL_SECS: u64 = 600;

pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshKind {
    Live,
    Movies,
    Series,
}

impl App {
    pub fn last_refresh(&self, kind: RefreshKind) -> Option<u64> {
        let times = self.settings.last_refresh.get(self.source_id()?)?;
        match kind {
            RefreshKind::Live => times.live,
            RefreshKind::Movies => times.movies,
            RefreshKind::Series => times.series,
        }
    }

    /// Remember that a catalog kind was opened, so scheduled refresh keeps
    /// trying it even if it never loaded.
    pub(super) fn record_opened(&mut self, kind: RefreshKind) {
        let Some(source_id) = self.source_id().map(str::to_owned) else {
            return;
        };
        let times = self.settings.last_refresh.entry(source_id).or_default();
        let opened = match kind {
            RefreshKind::Live => return,
            RefreshKind::Movies => &mut times.movies_opened,
            RefreshKind::Series => &mut times.series_opened,
        };
        if !*opened {
            *opened = true;
            self.save_settings();
        }
    }

    fn was_opened(&self, kind: RefreshKind) -> bool {
        let times = self
            .source_id()
            .and_then(|id| self.settings.last_refresh.get(id));
        match kind {
            RefreshKind::Live => true,
            RefreshKind::Movies => times.is_some_and(|t| t.movies_opened),
            RefreshKind::Series => times.is_some_and(|t| t.series_opened),
        }
    }

    pub(super) fn record_refresh(&mut self, kind: RefreshKind) {
        let Some(source_id) = self.source_id().map(str::to_owned) else {
            return;
        };
        let now = (self.clock)();
        let times = self.settings.last_refresh.entry(source_id).or_default();
        match kind {
            RefreshKind::Live => times.live = Some(now),
            RefreshKind::Movies => times.movies = Some(now),
            RefreshKind::Series => times.series = Some(now),
        }
        self.save_settings();
    }

    pub fn refresh_is_due(&self, kind: RefreshKind) -> bool {
        self.settings
            .refresh_interval
            .is_due(self.last_refresh(kind), (self.clock)())
    }

    /// Messages for refresh outcomes appear on Setup and Settings only.
    pub(super) fn notify_refresh(&mut self, message: String) {
        if matches!(self.router.current(), View::Setup | View::Settings) {
            self.notice = Some(message);
        }
    }

    /// Start a refresh for each kind that is due. Called every frame; runs
    /// the check at most once per [`CHECK_INTERVAL_SECS`].
    pub(super) fn run_refresh_schedule(&mut self) {
        if !self.auto_refresh {
            return;
        }
        let now = (self.clock)();
        if now < self.next_refresh_check {
            return;
        }
        self.next_refresh_check = now + CHECK_INTERVAL_SECS;
        self.refresh_due_kinds();
    }

    pub(super) fn refresh_due_kinds(&mut self) {
        let Some(source) = self.settings.source.clone() else {
            return;
        };
        if self.refresh_is_due(RefreshKind::Live)
            && self.connect_rx.is_none()
            && self.router.current() != View::Setup
        {
            self.start_connect(source);
        }
        if self.has_xtream_source()
            && (self.movie_slot.loaded.is_some()
                || (self.was_opened(RefreshKind::Movies) && !self.movie_slot.is_loading()))
            && self.refresh_is_due(RefreshKind::Movies)
        {
            self.start_movie_load(LoadOrigin::Network);
        }
        if self.has_xtream_source()
            && (self.series_slot.loaded.is_some()
                || (self.was_opened(RefreshKind::Series) && !self.series_slot.is_loading()))
            && self.refresh_is_due(RefreshKind::Series)
        {
            self.start_series_load(LoadOrigin::Network);
        }
    }

    pub fn cycle_refresh_interval(&mut self, forward: bool) {
        let interval = self.settings.refresh_interval;
        self.settings.refresh_interval = if forward {
            interval.next()
        } else {
            interval.prev()
        };
        self.save_settings();
    }
}
