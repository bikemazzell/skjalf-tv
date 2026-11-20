//! View rendering: pure egui drawing over the [`crate::app::App`] state.

pub mod browser;
pub mod live;
pub mod main_menu;
pub mod marked_row;
pub mod movie_details;
pub mod movies;
pub mod paragraph;
pub mod player;
pub mod series;
pub mod series_details;
pub mod settings;
pub mod setup;
pub mod transport_bar;

/// Touch Back button: top-left, outside the controller focus order, and
/// equal to pressing B.
/// The details art in a right-hand panel; nothing until the art is ready.
/// Draw it before the central panel.
pub fn poster_panel(ctx: &egui::Context, app: &mut crate::app::App, key: Option<String>) {
    let Some(texture) = key.and_then(|key| app.art.texture(ctx, &key)) else {
        return;
    };
    let width = (ctx.content_rect().width() * 0.28).clamp(160.0, 360.0);
    egui::SidePanel::right("details_poster")
        .resizable(false)
        .exact_width(width)
        .show(ctx, |ui| {
            ui.add_space(12.0);
            let art = texture.size_vec2();
            let room = egui::vec2(ui.available_width(), ui.available_height() - 12.0);
            let scale = (room.x / art.x.max(1.0)).min(room.y / art.y.max(1.0));
            ui.add(egui::Image::new((texture.id(), art * scale)));
        });
}

pub fn back_button(ui: &mut egui::Ui, app: &mut crate::app::App) {
    if ui
        .add_sized(
            [crate::theme::MIN_HIT * 2.0, crate::theme::MIN_HIT],
            egui::Button::new("Back"),
        )
        .clicked()
    {
        app.handle_action(crate::input::Action::Back, 0);
    }
}

/// Scroll a focused widget to the middle of its scroll area in one frame.
/// An animated scroll restarts on every held-direction step and makes the
/// list slide back and forth.
pub fn center_now(response: &egui::Response) {
    response.scroll_to_me_animation(
        Some(egui::Align::Center),
        egui::style::ScrollAnimation::none(),
    );
}

/// Scroll offset that centers `selected` in a `ScrollArea::show_rows` list of
/// `MIN_HIT` rows that fills the rest of `ui`.
pub fn centered_row_offset(ui: &egui::Ui, selected: usize, rows: usize) -> f32 {
    centered_offset(
        selected,
        rows,
        crate::theme::MIN_HIT,
        ui.spacing().item_spacing.y,
        ui.available_height(),
    )
}

/// Clamped to the scroll range: egui draws an out-of-range offset for one
/// frame before it clamps, which flickers near the end of the list.
fn centered_offset(selected: usize, rows: usize, row: f32, spacing: f32, viewport: f32) -> f32 {
    let stride = row + spacing;
    let content = (stride * rows as f32 - spacing).max(0.0);
    let max_offset = (content - viewport).max(0.0);
    let top = selected.min(rows.saturating_sub(1)) as f32 * stride;
    (top - (viewport - row) / 2.0).clamp(0.0, max_offset)
}

#[cfg(test)]
mod tests {
    use super::centered_offset;

    const ROW: f32 = 40.0;
    const SPACING: f32 = 8.0;
    const VIEWPORT: f32 = 400.0;

    fn offset(selected: usize, rows: usize) -> f32 {
        centered_offset(selected, rows, ROW, SPACING, VIEWPORT)
    }

    #[test]
    fn rows_near_the_top_do_not_scroll() {
        assert_eq!(offset(0, 100), 0.0);
        assert_eq!(offset(3, 100), 0.0);
    }

    #[test]
    fn middle_rows_are_centered() {
        let top = 50.0 * (ROW + SPACING);
        assert_eq!(offset(50, 100), top - (VIEWPORT - ROW) / 2.0);
    }

    #[test]
    fn rows_near_the_end_stop_at_the_last_row() {
        let max = 100.0 * (ROW + SPACING) - SPACING - VIEWPORT;
        assert_eq!(offset(97, 100), max);
        assert_eq!(offset(99, 100), max);
        assert_eq!(offset(500, 100), max);
    }

    #[test]
    fn a_list_shorter_than_the_viewport_never_scrolls() {
        assert_eq!(offset(5, 6), 0.0);
        assert_eq!(offset(0, 0), 0.0);
    }
}
