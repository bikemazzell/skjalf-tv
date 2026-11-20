//! Main view: large destination buttons laid out across the available width.

use egui::{Button, RichText};

use crate::app::{App, MainButton};

const MIN_BUTTON_WIDTH: f32 = 150.0;
const MAX_ROW_WIDTH: f32 = 1800.0;
const TILE_WIDTH: f32 = 260.0;
const TILE_HEIGHT: f32 = crate::theme::MIN_HIT * 1.5;
const BUTTON_HEIGHT: f32 = crate::theme::MIN_HIT * 2.5;

pub fn show(ctx: &egui::Context, app: &mut App) {
    if let Some(account) = &app.account {
        let (text, warn) = skjalf_core::xtream::expiry_notice_now(account.expires);
        let color = if warn {
            egui::Color32::from_rgb(240, 160, 80)
        } else {
            egui::Color32::GRAY
        };
        egui::TopBottomPanel::bottom("account_footer").show(ctx, |ui| {
            ui.label(RichText::new(text).color(color));
        });
    }
    if let Some(message) = &app.notice {
        egui::TopBottomPanel::bottom("main_notice").show(ctx, |ui| {
            ui.label(skjalf_core::display::visual_text(message));
        });
    }
    let mut activated = None;
    let mut opened_tile = None;
    let scroll_key = (
        app.main_focus,
        app.main_row,
        app.main_row.map_or(0, |row| app.main_row_selection(row)),
    );
    let focus_moved = app.last_main_scroll_key != Some(scroll_key);
    egui::CentralPanel::default().show(ctx, |ui| {
        egui::ScrollArea::vertical()
            .id_salt("main_scroll")
            .show(ui, |ui| {
                ui.add_space(16.0);
                ui.heading(crate::APP_NAME);
                ui.add_space(16.0);
                let spacing = ui.spacing().item_spacing.x;
                let row_width = ui.available_width().min(MAX_ROW_WIDTH);
                let columns = (((row_width + spacing) / (MIN_BUTTON_WIDTH + spacing)) as usize)
                    .clamp(1, MainButton::ALL.len());
                app.main_columns = columns;
                let width = (row_width - spacing * (columns - 1) as f32) / columns as f32;
                for row in MainButton::ALL.chunks(columns) {
                    ui.horizontal(|ui| {
                        for button in row {
                            let selected = app.main_focus == *button && app.main_row.is_none();
                            let text =
                                RichText::new(button.label()).size(crate::theme::HEADING_SIZE);
                            let text = if selected {
                                text.color(egui::Color32::BLACK)
                            } else {
                                text
                            };
                            let response = ui.add_sized(
                                [width, BUTTON_HEIGHT],
                                Button::new(text).selected(selected),
                            );
                            if selected && focus_moved {
                                super::center_now(&response);
                            }
                            if response.clicked() {
                                activated = Some(*button);
                            }
                        }
                    });
                }
                for row in app.main_rows() {
                    ui.add_space(12.0);
                    ui.label(RichText::new(row.title()).strong());
                    let focused_row = app.main_row == Some(row);
                    let selected = app.main_row_selection(row);
                    let moved = focus_moved && focused_row;
                    let area = egui::ScrollArea::horizontal()
                        .id_salt(("main_row", row))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                for (position, tile) in
                                    app.main_row_tiles(row).into_iter().enumerate()
                                {
                                    let is_selected = focused_row && position == selected;
                                    let art = tile
                                        .art
                                        .as_deref()
                                        .and_then(|key| app.art.texture(ui.ctx(), key));
                                    let response = super::marked_row::show_with_detail(
                                        ui,
                                        egui::vec2(TILE_WIDTH, TILE_HEIGHT),
                                        super::marked_row::RowContent {
                                            text: &tile.title,
                                            detail: tile.detail.as_deref(),
                                            marker: tile.marker,
                                            art: art.as_ref(),
                                        },
                                        is_selected,
                                        1,
                                    );
                                    if is_selected && moved {
                                        super::center_now(&response);
                                    }
                                    if response.clicked() {
                                        opened_tile = Some((row, position));
                                    }
                                }
                            });
                        });
                    // A nested scroll area swallows the vertical target, so scroll the
                    // row itself into view from the outer area.
                    if moved {
                        ui.scroll_to_rect_animation(
                            area.inner_rect,
                            Some(egui::Align::Center),
                            egui::style::ScrollAnimation::none(),
                        );
                    }
                }
            });
    });
    app.last_main_scroll_key = Some(scroll_key);
    if let Some(button) = activated {
        app.activate_main_button(button);
    }
    if let Some((row, tile)) = opened_tile {
        app.open_main_tile(row, tile);
    }
}
