//! Series details: header, season list, and episode list.

use egui::{Align, Button, RichText};
use skjalf_core::catalog::{format_duration, format_rating};

use crate::app::{App, DetailInfo, SeriesFocus};

pub fn show(ctx: &egui::Context, app: &mut App, now_ms: u64) {
    let mut toggle_favorite = false;
    let mut retry = false;
    let mut chosen_season = None;
    let mut chosen_episode = None;
    let mut chosen_resume = None;
    let favorite_label = if app.series_is_favorite() {
        "Remove Favorite"
    } else {
        "Favorite"
    };
    let selection = app
        .series_details
        .as_ref()
        .map(|details| (details.season, details.episode));
    let jump = selection.is_some() && app.last_episode_sel != selection;
    app.last_episode_sel = selection;

    egui::TopBottomPanel::top("series_header").show(ctx, |ui| {
        super::back_button(ui, app);
        let Some(details) = &app.series_details else {
            return;
        };
        ui.heading(details.display_name.replace(['\n', '\r'], " "));
        if let DetailInfo::Ready(info) = &details.info {
            let rating = format_rating(info.rating.as_deref())
                .or_else(|| format_rating(details.series.rating.as_deref()));
            let line: Vec<String> = [
                rating.map(|rating| format!("Rating {rating}")),
                info.genre.clone(),
                info.release_date.clone(),
            ]
            .into_iter()
            .flatten()
            .collect();
            if !line.is_empty() {
                ui.label(line.join("  \u{b7}  "));
            }
            if let Some(plot) = &info.plot {
                egui::ScrollArea::vertical()
                    .id_salt("series_plot")
                    .max_height(crate::theme::MIN_HIT * 2.0)
                    .show(ui, |ui| super::paragraph::show(ui, plot));
            }
        } else if let Some(rating) = format_rating(details.series.rating.as_deref()) {
            ui.label(format!("Rating {rating}"));
        }
        let focused = details.focus == SeriesFocus::Favorite;
        let label = RichText::new(favorite_label);
        let label = if focused {
            label.color(egui::Color32::BLACK)
        } else {
            label
        };
        if ui
            .add_sized(
                [200.0, crate::theme::MIN_HIT],
                Button::new(label).selected(focused),
            )
            .clicked()
        {
            toggle_favorite = true;
        }
    });

    let Some(details) = &app.series_details else {
        egui::CentralPanel::default().show(ctx, |_| {});
        return;
    };
    match &details.info {
        DetailInfo::Loading => {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label("Loading episodes...");
            });
        }
        DetailInfo::Unavailable => {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label("Could not load episodes.");
                let focused = details.focus == SeriesFocus::Retry;
                let text = RichText::new("Retry");
                let text = if focused {
                    text.color(egui::Color32::BLACK)
                } else {
                    text
                };
                if ui
                    .add_sized(
                        [200.0, crate::theme::MIN_HIT],
                        Button::new(text).selected(focused),
                    )
                    .clicked()
                {
                    retry = true;
                }
            });
        }
        DetailInfo::Ready(info) => {
            egui::SidePanel::left("series_seasons")
                .resizable(false)
                .exact_width((ctx.content_rect().width() * 0.25).clamp(180.0, 320.0))
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("seasons")
                        .show(ui, |ui| {
                            for (position, season) in info.seasons.iter().enumerate() {
                                let selected = details.season == position;
                                let focused = selected && details.focus == SeriesFocus::Seasons;
                                let text = RichText::new(&season.name);
                                let text = if focused {
                                    text.color(egui::Color32::BLACK)
                                } else {
                                    text
                                };
                                let response = ui.add_sized(
                                    [ui.available_width(), crate::theme::MIN_HIT],
                                    Button::new(text).selected(selected),
                                );
                                if selected && jump {
                                    response.scroll_to_me(Some(Align::Center));
                                }
                                if response.clicked() {
                                    chosen_season = Some(position);
                                }
                            }
                        });
                });
            egui::CentralPanel::default().show(ctx, |ui| {
                if let Some(choice) = details.choice {
                    let position = app
                        .episode_resume(choice.season, choice.episode)
                        .unwrap_or_default();
                    ui.horizontal(|ui| {
                        for (resume, label) in [
                            (true, format!("Resume {}", format_duration(position as u64))),
                            (false, "Play from Start".to_owned()),
                        ] {
                            let focused = choice.resume_focused == resume;
                            let text = RichText::new(label);
                            let text = if focused {
                                text.color(egui::Color32::BLACK)
                            } else {
                                text
                            };
                            if ui
                                .add_sized(
                                    [220.0, crate::theme::MIN_HIT],
                                    Button::new(text).selected(focused),
                                )
                                .clicked()
                            {
                                chosen_resume = Some(resume);
                            }
                        }
                    });
                }
                let episodes = details.episodes();
                if episodes.is_empty() {
                    ui.label("No episodes available.");
                    return;
                }
                let mut scroll = egui::ScrollArea::vertical().id_salt(("episodes", details.season));
                if jump {
                    let stride = crate::theme::MIN_HIT + ui.spacing().item_spacing.y;
                    let top = details.episode as f32 * stride;
                    let offset =
                        (top - (ui.available_height() - crate::theme::MIN_HIT) / 2.0).max(0.0);
                    scroll = scroll.vertical_scroll_offset(offset);
                }
                scroll.show_rows(ui, crate::theme::MIN_HIT, episodes.len(), |ui, range| {
                    for position in range {
                        let episode = &episodes[position];
                        let duration = episode
                            .duration_secs
                            .map(|secs| format!("   {}", format_duration(secs)))
                            .unwrap_or_default();
                        let focused =
                            details.episode == position && details.focus == SeriesFocus::Episodes;
                        let text = format!(
                            "E{}  {}{duration}",
                            episode.number,
                            details
                                .episode_titles
                                .get(details.season)
                                .and_then(|titles| titles.get(position))
                                .map_or(episode.title.as_str(), String::as_str)
                        );
                        let response = super::marked_row::show(
                            ui,
                            egui::vec2(ui.available_width(), crate::theme::MIN_HIT),
                            &text,
                            app.markers.episode(&episode.id),
                            focused,
                            1,
                        );
                        if response.clicked() {
                            chosen_episode = Some(position);
                        }
                    }
                });
            });
        }
    }

    if let Some(resume) = chosen_resume {
        app.confirm_resume_choice(resume, now_ms);
    }
    if toggle_favorite {
        app.toggle_series_favorite(now_ms);
    }
    if retry {
        app.retry_series_info();
    }
    if let Some(season) = chosen_season {
        app.select_season(season);
    }
    if let Some(episode) = chosen_episode {
        app.click_episode(episode, now_ms);
    }
}
