//! Media transport bar: icon buttons drawn with painter shapes (no font
//! glyphs), a seek bar for on-demand playback, and time text.

use egui::{Color32, Pos2, Rect, Sense, Shape, Stroke, Vec2};
use skjalf_core::catalog::format_duration;
use skjalf_core::display::visual_text;

use crate::app::{bar_controls, App, BarControl, PlayerFocus};

const BUTTON_WIDTH: f32 = crate::theme::MIN_HIT * 1.3;
const ICON_HALF: f32 = 9.0;

#[derive(Clone, Copy)]
struct BarState {
    paused: bool,
    muted: bool,
    favorite: bool,
}

pub fn control_id(control: BarControl) -> egui::Id {
    egui::Id::new(("transport-control", control))
}

pub fn seek_bar_id() -> egui::Id {
    egui::Id::new("transport-seek-bar")
}

pub fn show(ctx: &egui::Context, app: &mut App, now_ms: u64) {
    let on_demand = app.is_on_demand();
    let width = (ctx.content_rect().width() - 24.0).max(280.0);
    egui::Area::new("player_controls".into())
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -12.0))
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(Color32::from_black_alpha(210))
                .inner_margin(8.0)
                .show(ui, |ui| {
                    ui.set_width(width - 16.0);
                    if on_demand {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(visual_text(&app.now_playing_title())).strong(),
                            )
                            .truncate(),
                        );
                        seek_row(ui, app, now_ms);
                    } else {
                        live_title_row(ui, app);
                    }
                    button_row(ui, app, now_ms);
                });
        });
}

fn fraction(position: Option<f64>, duration: Option<f64>) -> f32 {
    match (position, duration) {
        (Some(position), Some(duration)) if duration > 0.0 => {
            (position / duration).clamp(0.0, 1.0) as f32
        }
        _ => 0.0,
    }
}

fn clock_text(secs: Option<f64>) -> String {
    secs.map_or_else(|| "--:--".into(), |secs| format_duration(secs as u64))
}

fn seek_row(ui: &mut egui::Ui, app: &mut App, now_ms: u64) {
    let (position, duration) = (app.media.position, app.media.duration);
    let mut text = format!("{} / {}", clock_text(position), clock_text(duration));
    if let Some(ahead) = app.media.buffer.and_then(|buffer| buffer.ahead_secs) {
        text.push_str(&format!("  ({} buffered)", format_duration(ahead as u64)));
    }
    let galley = ui.painter().layout_no_wrap(
        text,
        egui::TextStyle::Body.resolve(ui.style()),
        Color32::WHITE,
    );
    let row_height = crate::theme::MIN_HIT * 0.7;
    let (row, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), row_height), Sense::hover());
    let text_width = galley.size().x + 12.0;
    let bar = Rect::from_min_max(row.min, Pos2::new(row.max.x - text_width, row.max.y));
    let response = ui.interact(bar, seek_bar_id(), Sense::click_and_drag());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, true, "Seek bar"));
    if response.clicked() || response.dragged() {
        if let Some(pointer) = response.interact_pointer_pos() {
            let at = ((pointer.x - bar.min.x) / bar.width().max(1.0)).clamp(0.0, 1.0);
            app.seek_to_fraction(f64::from(at), now_ms);
        }
    }
    let focused = app.player_focus == PlayerFocus::Seek;
    let painter = ui.painter();
    let track = Rect::from_center_size(bar.center(), Vec2::new(bar.width(), 8.0));
    painter.rect_filled(track, 4.0, Color32::from_gray(80));
    let filled = Rect::from_min_max(
        track.min,
        Pos2::new(
            track.min.x + track.width() * fraction(position, duration),
            track.max.y,
        ),
    );
    painter.rect_filled(filled, 4.0, crate::theme::ACCENT);
    painter.circle_filled(
        Pos2::new(filled.max.x, track.center().y),
        if focused { 10.0 } else { 7.0 },
        Color32::WHITE,
    );
    if focused {
        painter.rect_stroke(
            bar.expand(2.0),
            6.0,
            Stroke::new(2.0_f32, crate::theme::ACCENT),
            egui::StrokeKind::Outside,
        );
    }
    painter.galley(
        Pos2::new(
            row.max.x - galley.size().x,
            row.center().y - galley.size().y / 2.0,
        ),
        galley,
        Color32::WHITE,
    );
}

/// Channel name and, when known, the program on now.
fn live_title_row(ui: &mut egui::Ui, app: &mut App) {
    let logo = app
        .playing_channel_logo()
        .and_then(|key| app.art.texture(ui.ctx(), &key));
    ui.horizontal(|ui| {
        if let Some(logo) = logo {
            let height = ui.text_style_height(&egui::TextStyle::Body) * 1.4;
            let size = logo.size_vec2();
            let width = (height * size.x / size.y.max(1.0)).min(height * 2.0);
            ui.add(egui::Image::new((logo.id(), egui::vec2(width, height))));
        }
        ui.add(
            egui::Label::new(egui::RichText::new(visual_text(&app.now_playing_title())).strong())
                .truncate(),
        );
        if let Some(entry) = app.epg.first() {
            ui.add(
                egui::Label::new(format!(
                    "\u{b7}  Now: {} ({})",
                    visual_text(&entry.title),
                    entry.start.format("%H:%M")
                ))
                .truncate(),
            );
        }
    });
}

fn button_row(ui: &mut egui::Ui, app: &mut App, now_ms: u64) {
    let on_demand = app.is_on_demand();
    let controls = bar_controls(on_demand);
    ui.horizontal(|ui| {
        for (index, control) in controls.iter().enumerate() {
            let focused = app.player_focus == PlayerFocus::Controls(index);
            let state = BarState {
                paused: app.paused,
                muted: app.muted,
                favorite: app.playing_is_favorite(),
            };
            if bar_button(ui, *control, focused, state).clicked() {
                app.player_focus = PlayerFocus::Controls(index);
                app.activate_bar_control(*control, now_ms);
            }
            if *control == BarControl::Volume {
                ui.label(if app.muted {
                    "Muted".to_owned()
                } else {
                    format!("{}%", app.settings.volume)
                });
            }
        }
    });
}

fn bar_button(
    ui: &mut egui::Ui,
    control: BarControl,
    focused: bool,
    state: BarState,
) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(BUTTON_WIDTH, crate::theme::MIN_HIT),
        Sense::hover(),
    );
    let response = ui.interact(rect, control_id(control), Sense::click());
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, control.label()));
    let tooltip = if control == BarControl::Favorite && state.favorite {
        "Remove Favorite"
    } else {
        control.label()
    };
    let response = response.on_hover_text(tooltip);
    let background = if focused {
        crate::theme::ACCENT
    } else if response.hovered() {
        Color32::from_gray(64)
    } else {
        Color32::from_gray(38)
    };
    ui.painter().rect_filled(rect, 6.0, background);
    let color = if focused {
        Color32::BLACK
    } else {
        Color32::WHITE
    };
    paint_icon(ui.painter(), rect.center(), control, color, state);
    response
}

fn triangle(center: Pos2, half: f32, right: bool, color: Color32) -> Shape {
    let tip = if right { half } else { -half };
    Shape::convex_polygon(
        vec![
            Pos2::new(center.x + tip, center.y),
            Pos2::new(center.x - tip, center.y - half),
            Pos2::new(center.x - tip, center.y + half),
        ],
        color,
        Stroke::NONE,
    )
}

fn bar(center: Pos2, half: f32, color: Color32) -> Shape {
    Shape::rect_filled(
        Rect::from_center_size(center, Vec2::new(3.0, half * 2.0)),
        1.0,
        color,
    )
}

fn paint_star(painter: &egui::Painter, center: Pos2, radius: f32, color: Color32, filled: bool) {
    let point = |index: usize| {
        let radius = if index.is_multiple_of(2) {
            radius
        } else {
            radius * 0.42
        };
        let angle = std::f32::consts::PI * (index as f32 / 5.0 - 0.5);
        Pos2::new(
            center.x + radius * angle.cos(),
            center.y + radius * angle.sin(),
        )
    };
    let outline: Vec<Pos2> = (0..10).map(point).collect();
    if filled {
        for index in 0..10 {
            painter.add(Shape::convex_polygon(
                vec![center, point(index), point((index + 1) % 10)],
                color,
                Stroke::NONE,
            ));
        }
    } else {
        painter.add(Shape::closed_line(outline, Stroke::new(2.0_f32, color)));
    }
}

fn paint_icon(
    painter: &egui::Painter,
    center: Pos2,
    control: BarControl,
    color: Color32,
    state: BarState,
) {
    let BarState {
        paused,
        muted,
        favorite,
    } = state;
    let h = ICON_HALF;
    let stroke = Stroke::new(2.5_f32, color);
    let at = |dx: f32| Pos2::new(center.x + dx, center.y);
    match control {
        BarControl::Back => {
            painter.line_segment([at(h), at(-h)], stroke);
            painter.line_segment([at(-h), Pos2::new(center.x, center.y - h)], stroke);
            painter.line_segment([at(-h), Pos2::new(center.x, center.y + h)], stroke);
        }
        BarControl::Rewind => {
            painter.add(triangle(at(-h * 0.5), h * 0.9, false, color));
            painter.add(triangle(at(h * 0.6), h * 0.9, false, color));
        }
        BarControl::Forward => {
            painter.add(triangle(at(-h * 0.6), h * 0.9, true, color));
            painter.add(triangle(at(h * 0.5), h * 0.9, true, color));
        }
        BarControl::PlayPause if paused => {
            painter.add(triangle(at(1.0), h, true, color));
        }
        BarControl::PlayPause => {
            painter.add(bar(at(-4.5), h, color));
            painter.add(bar(at(4.5), h, color));
        }
        BarControl::Previous => {
            painter.add(bar(at(-h), h, color));
            painter.add(triangle(at(2.0), h, false, color));
        }
        BarControl::Next => {
            painter.add(triangle(at(-2.0), h, true, color));
            painter.add(bar(at(h), h, color));
        }
        BarControl::Favorite => paint_star(painter, center, h * 1.25, color, favorite),
        BarControl::Tracks => {
            let frame = Rect::from_center_size(center, Vec2::new(h * 2.4, h * 1.7));
            painter.rect_stroke(
                frame,
                3.0,
                Stroke::new(2.0_f32, color),
                egui::StrokeKind::Inside,
            );
            for (y, from, to) in [(0.2, -0.75, 0.15), (0.2, 0.4, 0.75), (0.55, -0.75, 0.75)] {
                painter.line_segment(
                    [
                        Pos2::new(center.x + h * from, center.y + h * y - h * 0.2),
                        Pos2::new(center.x + h * to, center.y + h * y - h * 0.2),
                    ],
                    Stroke::new(2.0_f32, color),
                );
            }
        }
        BarControl::Volume => {
            painter.add(Shape::rect_filled(
                Rect::from_min_max(
                    Pos2::new(center.x - h, center.y - h * 0.4),
                    Pos2::new(center.x - h * 0.35, center.y + h * 0.4),
                ),
                1.0,
                color,
            ));
            painter.add(Shape::convex_polygon(
                vec![
                    Pos2::new(center.x - h * 0.35, center.y - h * 0.4),
                    Pos2::new(center.x + h * 0.25, center.y - h * 0.95),
                    Pos2::new(center.x + h * 0.25, center.y + h * 0.95),
                    Pos2::new(center.x - h * 0.35, center.y + h * 0.4),
                ],
                color,
                Stroke::NONE,
            ));
            if !muted {
                for radius in [h * 0.6, h * 1.0] {
                    let arc: Vec<Pos2> = (-4..=4)
                        .map(|step| {
                            let angle = step as f32 * 0.1 * std::f32::consts::PI * 0.5;
                            Pos2::new(
                                center.x + h * 0.25 + radius * angle.cos(),
                                center.y + radius * angle.sin(),
                            )
                        })
                        .collect();
                    painter.add(Shape::line(arc, Stroke::new(2.0_f32, color)));
                }
            }
            if muted {
                let cross = Stroke::new(3.0_f32, Color32::from_rgb(230, 80, 80));
                painter.line_segment(
                    [
                        Pos2::new(center.x + h * 0.7, center.y - h),
                        Pos2::new(center.x + h * 1.7, center.y + h),
                    ],
                    cross,
                );
                painter.line_segment(
                    [
                        Pos2::new(center.x + h * 0.7, center.y + h),
                        Pos2::new(center.x + h * 1.7, center.y - h),
                    ],
                    cross,
                );
            }
        }
    }
}
