//! Settings view: source actions and a credential-free status area.

use egui::{Button, RichText};
use skjalf_core::model::wait_label;
use skjalf_core::tracks::language_pref_label;

use crate::app::{App, SettingsFocus};

pub fn show(ctx: &egui::Context, app: &mut App) {
    if let Some(message) = &app.notice {
        egui::TopBottomPanel::bottom("settings_notice").show(ctx, |ui| {
            ui.label(skjalf_core::display::visual_text(message));
        });
    }
    let mut activated = None;
    egui::CentralPanel::default().show(ctx, |ui| {
        super::back_button(ui, app);
        ui.add_space(4.0);
        ui.heading("Settings");
        ui.add_space(8.0);
        let width = ui.available_width().min(520.0);
        let busy = app.connect_rx.is_some();
        for button in SettingsFocus::ALL {
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
    if let Some(button) = activated {
        app.activate_settings_button(button);
    }
}
