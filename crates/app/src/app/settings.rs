//! Settings screen: source editing, content refresh, and connection status.

use skjalf_core::model::SourceKind;
use skjalf_core::nav::View;

use skjalf_core::catalog::format_unix_local;
use skjalf_core::tracks::cycle_language_pref;

use super::{App, LoadStatus, RefreshKind};
use crate::input::Action;
use crate::setup_model::SetupModel;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsFocus {
    #[default]
    EditSource,
    RefreshContent,
    RefreshInterval,
    StartBuffer,
    StallBuffer,
    AudioLanguage,
    Subtitles,
    SubtitleLanguage,
    UserAgent,
}

impl SettingsFocus {
    /// Top-to-bottom order on the Settings screen.
    pub const ALL: [SettingsFocus; 9] = [
        SettingsFocus::EditSource,
        SettingsFocus::RefreshContent,
        SettingsFocus::RefreshInterval,
        SettingsFocus::StartBuffer,
        SettingsFocus::StallBuffer,
        SettingsFocus::AudioLanguage,
        SettingsFocus::Subtitles,
        SettingsFocus::SubtitleLanguage,
        SettingsFocus::UserAgent,
    ];

    fn step(self, down: bool) -> Self {
        let index = Self::ALL.iter().position(|item| *item == self).unwrap_or(0);
        let next = if down {
            (index + 1).min(Self::ALL.len() - 1)
        } else {
            index.saturating_sub(1)
        };
        Self::ALL[next]
    }

    /// Rows that change a value with Left and Right.
    fn cycles(self) -> bool {
        matches!(
            self,
            SettingsFocus::RefreshInterval
                | SettingsFocus::StartBuffer
                | SettingsFocus::StallBuffer
                | SettingsFocus::AudioLanguage
                | SettingsFocus::Subtitles
                | SettingsFocus::SubtitleLanguage
        )
    }
}

impl App {
    pub(super) fn enter_settings(&mut self) {
        self.settings_focus = SettingsFocus::default();
        self.router.push(View::Settings);
    }

    pub(super) fn handle_settings(&mut self, action: Action) {
        match action {
            Action::Up | Action::Down => {
                self.settings_focus = self.settings_focus.step(action == Action::Down)
            }
            Action::Left | Action::Right if self.settings_focus.cycles() => {
                self.cycle_setting(self.settings_focus, action == Action::Right)
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
            SettingsFocus::UserAgent => self.start_text_edit(super::TextTarget::UserAgent),
            SettingsFocus::RefreshInterval
            | SettingsFocus::StartBuffer
            | SettingsFocus::StallBuffer
            | SettingsFocus::AudioLanguage
            | SettingsFocus::Subtitles
            | SettingsFocus::SubtitleLanguage => self.cycle_setting(button, true),
        }
    }

    fn cycle_setting(&mut self, row: SettingsFocus, forward: bool) {
        use skjalf_core::model::{cycle_choice, STALL_BUFFER_CHOICES, START_BUFFER_CHOICES};
        match row {
            SettingsFocus::RefreshInterval => return self.cycle_refresh_interval(forward),
            SettingsFocus::StartBuffer => {
                self.settings.start_buffer_secs = cycle_choice(
                    &START_BUFFER_CHOICES,
                    self.settings.start_buffer_secs,
                    forward,
                )
            }
            SettingsFocus::StallBuffer => {
                self.settings.stall_buffer_secs = cycle_choice(
                    &STALL_BUFFER_CHOICES,
                    self.settings.stall_buffer_secs,
                    forward,
                )
            }
            SettingsFocus::AudioLanguage => {
                self.settings.audio_lang =
                    cycle_language_pref(self.settings.audio_lang.as_deref(), forward)
            }
            SettingsFocus::Subtitles => self.settings.subtitles_on = !self.settings.subtitles_on,
            SettingsFocus::SubtitleLanguage => {
                self.settings.subtitle_lang =
                    cycle_language_pref(self.settings.subtitle_lang.as_deref(), forward)
            }
            SettingsFocus::EditSource
            | SettingsFocus::RefreshContent
            | SettingsFocus::UserAgent => return,
        }
        self.save_settings();
    }

    pub fn user_agent_editing(&self) -> bool {
        self.editing == Some(super::TextTarget::UserAgent)
    }

    pub fn set_user_agent_text(&mut self, text: String) {
        self.settings.user_agent = text;
    }

    /// Save the edited User-Agent and use it for every later request.
    pub(super) fn apply_user_agent(&mut self) {
        self.save_settings();
        let user_agent = self.settings.effective_user_agent();
        self.fetcher = std::sync::Arc::new(
            crate::net::UreqFetcher::new(self.settings.ignore_ssl).with_user_agent(&user_agent),
        );
        if let Some(engine) = &mut self.engine {
            if let Err(error) = engine.set_user_agent(&user_agent) {
                self.notice = Some(format!("User Agent error: {error}"));
            }
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
