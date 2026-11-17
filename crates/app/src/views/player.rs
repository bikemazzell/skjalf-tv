//! Player view: fullscreen libmpv GL video + OSD overlays (zap, info, EPG).

use std::sync::Arc;

use egui::Color32;
use egui_glow::glow::HasContext;
use skjalf_core::catalog::format_duration;
use skjalf_core::display::visual_text;
use skjalf_core::nav::OsdKind;

use crate::app::{App, PlayerFocus};

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
                    ui.label(egui::RichText::new("No video").weak());
                });
            }
        });

    if video_tapped {
        app.player.show_osd(OsdKind::Info, now_ms);
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
