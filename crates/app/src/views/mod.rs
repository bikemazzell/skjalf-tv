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
