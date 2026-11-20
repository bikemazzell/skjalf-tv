//! Player view: fullscreen libmpv GL video + OSD overlays (zap, info, EPG).

use std::sync::Arc;

use egui::Color32;
use egui_glow::glow::HasContext;
use skjalf_core::catalog::format_duration;
use skjalf_core::display::visual_text;
use skjalf_core::nav::OsdKind;
use skjalf_core::tracks::{selected, track_label, Track, TrackKind};

use crate::app::{App, PlayerFocus};

/// The tracks panel's step buttons (touch, and tests).
pub fn tracks_button_id(kind: TrackKind, forward: bool) -> egui::Id {
    egui::Id::new(("tracks-step", kind == TrackKind::Audio, forward))
}

pub fn show(ctx: &egui::Context, app: &mut App, now_ms: u64) {
    let visible = app.player.osd_visible(now_ms);
    let kind = app.player.osd().kind.clone();

    // Video area.
    let render_ctx = app.engine.as_ref().and_then(|e| e.render_context());
    let render_error = app.engine.as_ref().and_then(|e| e.render_error_sink());
    let mut video_tapped = false;
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(Color32::BLACK))
        .show(ctx, |ui| {
            let rect = ui.max_rect();
            video_tapped = ui
                .interact(rect, ui.id().with("video_touch"), egui::Sense::click())
                .clicked();
            if let Some(rc) = render_ctx {
                let callback = egui::PaintCallback {
                    rect,
                    callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
                        let fbo = rsmpv::render::OpenGlFbo {
                            fbo: unsafe {
                                painter
                                    .gl()
                                    .get_parameter_i32(egui_glow::glow::FRAMEBUFFER_BINDING)
                            },
                            width: info.screen_size_px[0] as i32,
                            height: info.screen_size_px[1] as i32,
                            internal_format: 0,
                        };
                        let result = if let Ok(mut ctx) = rc.lock() {
                            // flip_y: GL origin is bottom-left, mpv's top-left.
                            ctx.render_opengl(fbo, true, true)
                                .map_err(|error| error.to_string())
                        } else {
                            Err("Video renderer lock failed.".into())
                        };
                        if let (Err(error), Some(sink)) = (result, &render_error) {
                            if let Ok(mut slot) = sink.lock() {
                                *slot = Some(format!("Video render failed: {error}"));
                            }
                        }
                    })),
                };
                ui.painter().add(callback);
            } else {
                ui.centered_and_justified(|ui| {
                    // Not selectable: a selectable label would take video taps.
                    ui.add(
                        egui::Label::new(egui::RichText::new("No video").weak()).selectable(false),
                    );
                });
            }
        });

    if video_tapped {
        if let PlayerFocus::Tracks(_) = app.player_focus {
            app.player_focus = PlayerFocus::Video;
        } else {
            app.player.show_osd(OsdKind::Info, now_ms);
        }
    }
    if let PlayerFocus::Tracks(row) = app.player_focus {
        if let Some((kind, forward)) = tracks_panel(ctx, &app.tracks(), row) {
            app.switch_track(kind, forward);
        }
    }

    if let Some(buffer) = app.media.buffer.filter(|buffer| buffer.waiting) {
        let ahead = buffer.ahead_secs.unwrap_or(0.0).min(buffer.target_secs);
        let percent = if buffer.target_secs > 0.0 {
            (ahead / buffer.target_secs * 100.0).floor()
        } else {
            0.0
        };
        egui::Area::new("buffering".into())
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(Color32::from_black_alpha(210))
                    .inner_margin(16.0)
                    .show(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(format!(
                                        "Buffering {} of {} ({percent:.0}%)",
                                        format_duration(ahead as u64),
                                        format_duration(buffer.target_secs.ceil() as u64)
                                    ))
                                    .size(crate::theme::HEADING_SIZE)
                                    .color(Color32::WHITE),
                                )
                                .wrap_mode(egui::TextWrapMode::Extend),
                            );
                            ui.label(
                                egui::RichText::new("Press A to start now")
                                    .color(Color32::LIGHT_GRAY),
                            );
                        });
                    });
            });
    }

    if visible && kind == OsdKind::Epg && !app.is_on_demand() {
        egui::Area::new("program_list".into())
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -96.0))
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(Color32::from_black_alpha(210))
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        ui.set_min_width(
                            (ctx.input(|i| i.viewport_rect().width()) - 48.0).max(300.0),
                        );
                        if app.epg.is_empty() {
                            ui.label("No program information.");
                        }
                        for entry in app.epg.iter().take(4) {
                            ui.label(format!(
                                "{}-{}  {}",
                                entry.start.format("%H:%M"),
                                entry.end.format("%H:%M"),
                                visual_text(&entry.title)
                            ));
                        }
                    });
            });
    }

    if visible {
        if let OsdKind::Zap(text) = &kind {
            egui::Area::new("zap_notice".into())
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-40.0, 40.0))
                .show(ctx, |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(visual_text(text))
                                .size(44.0)
                                .color(Color32::WHITE),
                        )
                        .wrap_mode(egui::TextWrapMode::Extend),
                    );
                });
        }
    }
    if let Some((text, until_ms)) = &app.status {
        if now_ms < *until_ms {
            egui::Area::new("status_notice".into())
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-24.0, 24.0))
                .show(ctx, |ui| {
                    egui::Frame::NONE
                        .fill(Color32::from_black_alpha(190))
                        .inner_margin(8.0)
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(visual_text(text))
                                        .size(crate::theme::HEADING_SIZE)
                                        .color(Color32::WHITE),
                                )
                                .wrap_mode(egui::TextWrapMode::Extend),
                            );
                        });
                });
        }
    }

    if let Some(message) = &app.notice {
        let width = (ctx.content_rect().width() - 48.0).max(200.0);
        egui::Area::new("player_notice".into())
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 20.0))
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(Color32::from_black_alpha(220))
                    .show(ui, |ui| {
                        ui.set_max_width(width);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(visual_text(message)).color(Color32::WHITE),
                            )
                            .truncate(),
                        );
                    });
            });
    }

    if visible || video_tapped || app.paused || app.player_focus != PlayerFocus::Video {
        super::transport_bar::show(ctx, app, now_ms);
    }
}

/// Audio and subtitle rows with step buttons; returns a tapped step.
fn tracks_panel(
    ctx: &egui::Context,
    tracks: &[Track],
    focus: TrackKind,
) -> Option<(TrackKind, bool)> {
    let mut tapped = None;
    egui::Area::new("tracks_panel".into())
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -150.0))
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(Color32::from_black_alpha(220))
                .corner_radius(8.0)
                .inner_margin(16.0)
                .show(ui, |ui| {
                    egui::Grid::new("tracks_rows")
                        .num_columns(4)
                        .spacing([16.0, 8.0])
                        .show(ui, |ui| {
                            for (kind, name) in [(TrackKind::Audio, "Audio"), (TrackKind::Sub, "Subtitles")] {
                                let color = if kind == focus {
                                    crate::theme::ACCENT
                                } else {
                                    Color32::LIGHT_GRAY
                                };
                                ui.label(egui::RichText::new(name).size(crate::theme::HEADING_SIZE).color(color));
                                for (forward, arrow) in [(false, "\u{2039}"), (true, "\u{203a}")] {
                                    if forward {
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(describe(tracks, kind))
                                                    .size(crate::theme::HEADING_SIZE)
                                                    .color(Color32::WHITE),
                                            )
                                            .wrap_mode(egui::TextWrapMode::Extend),
                                        );
                                    }
                                    let size = egui::Vec2::splat(crate::theme::MIN_HIT);
                                    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                                    let response = ui.interact(rect, tracks_button_id(kind, forward), egui::Sense::click());
                                    ui.painter().text(
                                        rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        arrow,
                                        egui::FontId::proportional(crate::theme::HEADING_SIZE),
                                        color,
                                    );
                                    if response.clicked() {
                                        tapped = Some((kind, forward));
                                    }
                                }
                                ui.end_row();
                            }
                        });
                    ui.label(
                        egui::RichText::new("\u{2191}\u{2193} row  \u{00b7}  \u{2190}\u{2192} switch  \u{00b7}  B close")
                            .color(Color32::LIGHT_GRAY),
                    );
                });
        });
    tapped
}

/// The current choice for one kind, e.g. `German · AC3 5.1  (2/3)`.
fn describe(tracks: &[Track], kind: TrackKind) -> String {
    let of_kind: Vec<&Track> = tracks.iter().filter(|t| t.kind == kind).collect();
    if of_kind.is_empty() {
        return "None".into();
    }
    match selected(tracks, kind) {
        Some(current) => {
            let position = of_kind.iter().position(|t| t.id == current.id).unwrap_or(0) + 1;
            format!("{}  ({position}/{})", track_label(current), of_kind.len())
        }
        None => format!("Off  ({} available)", of_kind.len()),
    }
}
