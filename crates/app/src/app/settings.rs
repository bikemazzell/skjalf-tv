//! Settings screen: source editing, content refresh, and connection status.

use skjalf_core::model::SourceKind;
use skjalf_core::nav::View;

use skjalf_core::catalog::format_unix_local;

use super::{App, LoadStatus, RefreshKind};
use crate::input::Action;
use crate::setup_model::SetupModel;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsFocus {
    #[default]
    EditSource,
    RefreshContent,
    RefreshInterval,
}

impl App {
    pub(super) fn enter_settings(&mut self) {
        self.settings_focus = SettingsFocus::default();
        self.router.push(View::Settings);
    }

    pub(super) fn handle_settings(&mut self, action: Action) {
        match action {
            Action::Up => {
                self.settings_focus = match self.settings_focus {
                    SettingsFocus::RefreshInterval => SettingsFocus::RefreshContent,
                    _ => SettingsFocus::EditSource,
                }
            }
            Action::Down => {
                self.settings_focus = match self.settings_focus {
                    SettingsFocus::EditSource => SettingsFocus::RefreshContent,
                    _ => SettingsFocus::RefreshInterval,
                }
            }
            Action::Left | Action::Right
                if self.settings_focus == SettingsFocus::RefreshInterval =>
            {
                self.cycle_refresh_interval(action == Action::Right)
            }
            Action::Select => self.activate_settings_button(self.settings_focus),
            Action::Back => {
                self.router.back();
            }
            _ => {}
        }
    }

    pub fn activate_settings_button(&mut self, button: SettingsFocus) {
        self.settings_focus = button;
        match button {
            SettingsFocus::EditSource => self.open_source_editor(),
            SettingsFocus::RefreshContent => self.refresh_content(),
            SettingsFocus::RefreshInterval => self.cycle_refresh_interval(true),
        }
    }

    pub fn open_source_editor(&mut self) {
        if self.connect_rx.is_some() {
            self.notice = Some("Wait for the refresh to finish before editing the source.".into());
            return;
        }
        if let Some(source) = &self.settings.source {
            self.setup = SetupModel::from_source(source);
            self.editing_source = true;
            self.router.push(View::Setup);
        }
    }

    pub fn refresh_content(&mut self) {
        if let Some(source) = self.settings.source.clone() {
            self.start_connect(source);
        }
        self.start_movie_load(super::LoadOrigin::Network);
        self.start_series_load(super::LoadOrigin::Network);
    }

    fn last_refresh_text(&self, kind: RefreshKind) -> String {
        self.last_refresh(kind)
            .map_or_else(|| "never".into(), format_unix_local)
    }

    /// Credential-free status lines for the Settings view.
    pub fn settings_status(&self) -> Vec<String> {
        let kind = match self.settings.source.as_ref().map(|source| &source.kind) {
            Some(SourceKind::Xtream { .. }) => "Xtream account",
            Some(SourceKind::M3u { .. }) => "M3U playlist",
            None => "No source",
        };
        let mut lines = vec![
            format!("Source: {kind}"),
            format!(
                "Channels: {}  Last refresh: {}",
                self.playlist.as_ref().map_or(0, |pl| pl.channels.len()),
                self.last_refresh_text(RefreshKind::Live)
            ),
        ];
        if self.connect_rx.is_some() {
            lines.push("Refreshing\u{2026}".into());
        } else if let Some(status) = &self.refresh_status {
            lines.push(status.clone());
        }
        if self.has_xtream_source() {
            let count = self
                .movie_slot
                .loaded
                .as_ref()
                .map_or(0, |loaded| loaded.data.movies.len());
            let updated = self.last_refresh_text(RefreshKind::Movies);
            lines.push(match &self.movie_slot.status {
                LoadStatus::Loading => {
                    format!("Movies: {count}  Last refresh: {updated} (loading\u{2026})")
                }
                LoadStatus::Failed(message) => {
                    format!("Movies: {count}  Last refresh: {updated} (refresh failed: {message})")
                }
                _ => format!("Movies: {count}  Last refresh: {updated}"),
            });
            let count = self
                .series_slot
                .loaded
                .as_ref()
                .map_or(0, |loaded| loaded.data.series.len());
            let updated = self.last_refresh_text(RefreshKind::Series);
            lines.push(match &self.series_slot.status {
                LoadStatus::Loading => {
                    format!("Series: {count}  Last refresh: {updated} (loading\u{2026})")
                }
                LoadStatus::Failed(message) => {
                    format!("Series: {count}  Last refresh: {updated} (refresh failed: {message})")
                }
                _ => format!("Series: {count}  Last refresh: {updated}"),
            });
        }
        lines
    }
}
