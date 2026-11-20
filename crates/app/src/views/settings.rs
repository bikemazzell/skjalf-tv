//! Settings view: source actions and a credential-free status area.

use egui::{Button, RichText};
use skjalf_core::model::{wait_label, DEFAULT_USER_AGENT};
use skjalf_core::tracks::language_pref_label;

use crate::app::{App, SettingsFocus, TextTarget};

/// The Settings content is a centered column, at most this wide.
const COLUMN_WIDTH: f32 = 640.0;

pub fn show(ctx: &egui::Context, app: &mut App) {
    if let Some(message) = &app.notice {
        egui::TopBottomPanel::bottom("settings_notice").show(ctx, |ui| {
            ui.label(skjalf_core::display::visual_text(message));
        });
    }
    let mut activated = None;
    egui::CentralPanel::default().show(ctx, |ui| {
        let width = ui.available_width().min(COLUMN_WIDTH);
        let margin = (ui.available_width() - width) / 2.0;
        ui.horizontal_top(|ui| {
            ui.add_space(margin);
            ui.vertical(|ui| {
                ui.set_width(width);
                super::back_button(ui, app);
                ui.add_space(4.0);
                ui.heading("Settings");
                ui.add_space(8.0);
                let busy = app.connect_rx.is_some();
                for button in SettingsFocus::ALL {
                    if button == SettingsFocus::UserAgent && app.user_agent_editing() {
                        user_agent_field(ui, app, width);
                        continue;
                    }
                    let label = match button {
                        SettingsFocus::EditSource => "Edit Source".to_owned(),
                        SettingsFocus::RefreshContent => "Refresh Content".to_owned(),
                        SettingsFocus::RefreshInterval => format!(
                            "Refresh Interval: {}",
                            app.settings.refresh_interval.label()
                        ),
                        SettingsFocus::StartBuffer => format!(
                            "Buffer Before Playback: {}",
                            wait_label(app.settings.start_buffer_secs)
                        ),
                        SettingsFocus::StallBuffer => format!(
                            "Buffer After a Stall: {}",
                            wait_label(app.settings.stall_buffer_secs)
                        ),
                        SettingsFocus::AudioLanguage => format!(
                            "Audio Language: {}",
                            language_pref_label(app.settings.audio_lang.as_deref())
                        ),
                        SettingsFocus::Subtitles => format!(
                            "Subtitles: {}",
                            if app.settings.subtitles_on {
                                "On"
                            } else {
                                "Off"
                            }
                        ),
                        SettingsFocus::SubtitleLanguage => format!(
                            "Subtitle Language: {}",
                            language_pref_label(app.settings.subtitle_lang.as_deref())
                        ),
                        SettingsFocus::UserAgent => {
                            format!("User Agent: {}", app.settings.effective_user_agent())
                        }
                    };
                    let selected = app.settings_focus == button;
                    let text = RichText::new(label);
                    let text = if selected {
                        text.color(egui::Color32::BLACK)
                    } else {
                        text
                    };
                    let enabled = !busy || button != SettingsFocus::RefreshContent;
                    if ui
                        .add_enabled(
                            enabled,
                            Button::new(text)
                                .selected(selected)
                                .min_size(egui::vec2(width, crate::theme::MIN_HIT)),
                        )
                        .clicked()
                    {
                        activated = Some(button);
                    }
                }
                ui.add_space(12.0);
                for line in app.settings_status() {
                    ui.label(line);
                }
            });
        });
    });
    if let Some(button) = activated {
        app.activate_settings_button(button);
    }
}

/// The User Agent row while it is edited; empty shows the default as a hint.
fn user_agent_field(ui: &mut egui::Ui, app: &mut App, width: f32) {
    let mut text = app.settings.user_agent.clone();
    let response = ui.add_sized(
        [width, crate::theme::MIN_HIT],
        egui::TextEdit::singleline(&mut text).hint_text(DEFAULT_USER_AGENT),
    );
    if app.take_focus_request(TextTarget::UserAgent) {
        response.request_focus();
    }
    if text != app.settings.user_agent {
        app.set_user_agent_text(text);
    }
}
