//! Dark 10-foot theme: large text, high contrast, Deck-friendly sizes.

use egui::{Context, FontId, Style, TextStyle, Visuals};

/// Base body size in px — Deck rule of thumb is ≥ 12 px; we sit well above.
pub const BODY_SIZE: f32 = 18.0;
pub const HEADING_SIZE: f32 = 28.0;

/// Minimum interactive hit area (Deck thumb-friendly).
pub const MIN_HIT: f32 = 44.0;

pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(94, 213, 148); // Deck green

pub fn apply(ctx: &Context) {
    let mut spacing = egui::style::Spacing::default();
    spacing.interact_size.y = MIN_HIT;
    spacing.button_padding = egui::vec2(16.0, 10.0);

    let mut visuals = Visuals::dark();
    visuals.selection.bg_fill = ACCENT;
    visuals.widgets.inactive.bg_fill = egui::Color32::from_gray(38);
    visuals.widgets.hovered.bg_fill = egui::Color32::from_gray(52);
    visuals.widgets.active.bg_fill = egui::Color32::from_gray(64);

    let style = Style {
        text_styles: [
            (TextStyle::Heading, FontId::proportional(HEADING_SIZE)),
            (TextStyle::Body, FontId::proportional(BODY_SIZE)),
            (TextStyle::Button, FontId::proportional(BODY_SIZE)),
            (TextStyle::Small, FontId::proportional(14.0)),
            (TextStyle::Monospace, FontId::monospace(BODY_SIZE)),
        ]
        .into(),
        visuals,
        spacing,
        ..Default::default()
    };
    ctx.set_style(style);
}

/// Scale by framebuffer size in Game Mode; desktop keeps its native scale.
pub fn adapt_display(ctx: &Context, game_mode: bool) {
    if !game_mode {
        return;
    }
    let physical_height =
        ctx.input(|input| input.viewport_rect().height()) * ctx.pixels_per_point();
    let target = game_mode_scale(physical_height);
    if (ctx.pixels_per_point() - target).abs() > 0.01 {
        ctx.set_pixels_per_point(target);
    }
}

fn game_mode_scale(physical_height: f32) -> f32 {
    if !physical_height.is_finite() || physical_height <= 0.0 {
        return 1.25;
    }
    (1.25 * (physical_height / 800.0).sqrt()).clamp(1.0, 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_mode_scale_tracks_physical_resolution_without_feedback() {
        for native in [1.0, 2.0] {
            let ctx = Context::default();
            let mut previous_height = 0.0;
            for height in [800.0, 1080.0, 2160.0, 800.0] {
                for _ in 0..4 {
                    let ppp = ctx.zoom_factor() * native;
                    let mut input = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(height * 1.6 / ppp, height / ppp),
                        )),
                        ..Default::default()
                    };
                    input
                        .viewports
                        .get_mut(&egui::ViewportId::ROOT)
                        .unwrap()
                        .native_pixels_per_point = Some(native);
                    let _ = ctx.run(input, |ctx| adapt_display(ctx, true));
                }
                let actual = ctx.zoom_factor() * native;
                assert!((actual - game_mode_scale(height)).abs() < 0.01);
                if height != 800.0 || previous_height == 0.0 {
                    assert!(height / actual > previous_height);
                }
                previous_height = height / actual;
            }
            let zoom = ctx.zoom_factor();
            adapt_display(&ctx, false);
            assert_eq!(
                ctx.zoom_factor(),
                zoom,
                "desktop scale must remain untouched"
            );
        }
    }

    #[test]
    fn scale_handles_invalid_and_small_viewports() {
        for height in [f32::NAN, f32::INFINITY, 0.0, -1.0] {
            assert_eq!(game_mode_scale(height), 1.25);
        }
        assert_eq!(game_mode_scale(480.0), 1.0);
        assert_eq!(game_mode_scale(2160.0), 2.0);
    }
}
