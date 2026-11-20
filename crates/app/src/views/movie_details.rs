//! Movie details: catalog data first, provider metadata when it arrives.

use egui::{Button, RichText};
use skjalf_core::catalog::{format_duration, format_rating};

use crate::app::{App, DetailInfo};

pub fn show(ctx: &egui::Context, app: &mut App, now_ms: u64) {
    let art = app.movie_details_art();
    super::poster_panel(ctx, app, art);
    let mut activated = None;
    egui::CentralPanel::default().show(ctx, |ui| {
        super::back_button(ui, app);
        let Some(details) = &app.movie_details else {
            return;
        };
        ui.add_space(8.0);
        ui.heading(details.display_name.replace(['\n', '\r'], " "));
        if let Some(rating) = format_rating(details.movie.rating.as_deref()) {
            ui.label(format!("Rating {rating}"));
        }
        ui.add_space(12.0);
        let focus = app.movie_focus();
        ui.horizontal(|ui| {
            for button in app.movie_buttons() {
                let selected = focus == button;
                let text = RichText::new(app.movie_button_label(button));
                let text = if selected {
                    text.color(egui::Color32::BLACK)
                } else {
                    text
                };
                if ui
                    .add_sized(
                        [200.0, crate::theme::MIN_HIT],
                        Button::new(text).selected(selected),
                    )
                    .clicked()
                {
                    activated = Some(button);
                }
            }
        });
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .id_salt("movie_metadata")
            .auto_shrink([false, false])
            .show(ui, |ui| match &details.info {
                DetailInfo::Loading => {
                    ui.label("Loading details...");
                }
                DetailInfo::Unavailable => {
                    ui.label("Details unavailable.");
                }
                DetailInfo::Ready(info) => {
                    let rows = [
                        ("Release", info.release_date.clone()),
                        ("Duration", info.duration_secs.map(format_duration)),
                        ("Genre", info.genre.clone()),
                        ("Director", info.director.clone()),
                    ];
                    for (label, value) in rows {
                        if let Some(value) = value {
                            ui.label(format!("{label}: {value}"));
                        }
                    }
                    if let Some(cast) = &info.cast {
                        ui.label("Cast:");
                        super::paragraph::show(ui, cast);
                    }
                    if let Some(description) = &info.description {
                        ui.add_space(6.0);
                        super::paragraph::show(ui, description);
                    }
                }
            });
    });
    if let Some(button) = activated {
        app.activate_movie_button(button, now_ms);
    }
}
