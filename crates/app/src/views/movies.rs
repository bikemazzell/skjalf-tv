//! Movies browser.

use crate::app::{App, BrowserId};

pub fn search_field_id() -> egui::Id {
    super::browser::search_field_id(BrowserId::Movies)
}

pub fn show(ctx: &egui::Context, app: &mut App) {
    super::browser::show(ctx, app, BrowserId::Movies);
}
