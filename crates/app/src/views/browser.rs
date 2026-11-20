//! Shared browser view (Live, Movies, Series): category sidebar (Favorites,
//! All, groups), search row, and the item list.
//!
//! The visible rows come from the cached `BrowserState::filtered` list, never
//! from a per-frame filter. Rows render through `ScrollArea::show_rows`; the
//! selection scrolls into view whenever it changes (controller navigation),
//! not on every frame.

use egui::{Button, RichText, TextEdit};
use skjalf_core::browse::{BrowserFocus, CategorySel};

use crate::app::{App, BrowserId, TextTarget};

const CLEAR_WIDTH: f32 = 90.0;
const SORT_WIDTH: f32 = 170.0;
/// Below this width the Clear and Sort buttons move under the field.
const WIDE_SEARCH_ROW: f32 = 560.0;

pub fn search_field_id(id: BrowserId) -> egui::Id {
    egui::Id::new(("browser-search-field", id))
}

pub fn group_filter_id(id: BrowserId) -> egui::Id {
    egui::Id::new(("browser-group-filter", id))
}

pub fn show(ctx: &egui::Context, app: &mut App, id: BrowserId) {
    if let Some(message) = &app.notice {
        egui::TopBottomPanel::bottom(egui::Id::new(("browser-notice", id))).show(ctx, |ui| {
            ui.label(skjalf_core::display::visual_text(message));
        });
    }
    if let Some(message) = app.browser_notice(id) {
        egui::CentralPanel::default().show(ctx, |ui| {
            super::back_button(ui, app);
            ui.add_space(12.0);
            ui.heading(id.title());
            ui.add_space(8.0);
            ui.label(RichText::new(message).size(crate::theme::HEADING_SIZE));
        });
        return;
    }

    egui::TopBottomPanel::bottom(egui::Id::new(("browser-hint", id))).show(ctx, |ui| {
        ui.label(RichText::new(app.browser_hint(id)).weak());
    });

    // Selection-change detection for scroll-into-view.
    let state = app.state(id);
    let sel = (app.category_row(id), state.selected, state.filtered.len());
    let jump = app.last_browser_sel[id as usize] != Some(sel);
    app.last_browser_sel[id as usize] = Some(sel);

    egui::SidePanel::left(egui::Id::new(("browser-groups", id)))
        .resizable(false)
        .exact_width((ctx.content_rect().width() * 0.22).clamp(220.0, 360.0))
        .show(ctx, |ui| {
            super::back_button(ui, app);
            group_filter(ui, app, id);
            ui.add_space(4.0);
            let stride = crate::theme::MIN_HIT + ui.spacing().item_spacing.y;
            app.page_rows[id as usize][0] =
                (ui.available_height() / stride).floor().max(1.0) as usize;
            let mut scroll = egui::ScrollArea::vertical().id_salt(("categories", id));
            if jump {
                let offset =
                    super::centered_row_offset(ui, app.category_row(id), app.categories_len(id));
                scroll = scroll.vertical_scroll_offset(offset);
            }
            scroll.show(ui, |ui| categories(ui, app, id));
        });

    egui::CentralPanel::default().show(ctx, |ui| {
        search_row(ui, app, id);
        ui.heading(app.category_title(id));
        ui.add_space(6.0);
        items(ui, app, id, jump);
    });
}

fn group_filter(ui: &mut egui::Ui, app: &mut App, id: BrowserId) {
    ui.horizontal(|ui| {
        let clear_width = crate::theme::MIN_HIT;
        let width = (ui.available_width() - clear_width - ui.spacing().item_spacing.x).max(60.0);
        let mut filter = app.state(id).group_filter.clone();
        let response = ui.add_sized(
            [width, crate::theme::MIN_HIT],
            TextEdit::singleline(&mut filter)
                .id(group_filter_id(id))
                .hint_text("Filter groups"),
        );
        if app.take_focus_request(TextTarget::GroupFilter(id)) {
            response.request_focus();
        }
        if response.clicked() || response.gained_focus() {
            app.start_group_filter_edit(id);
        }
        if app.state(id).focus == BrowserFocus::GroupFilter {
            response.highlight();
        }
        if filter != app.state(id).group_filter {
            app.set_group_filter(id, filter);
        }
        let clear_focused = app.state(id).focus == BrowserFocus::GroupFilterClear;
        let label = if clear_focused {
            RichText::new("X").color(egui::Color32::BLACK)
        } else {
            RichText::new("X")
        };
        let clear = ui
            .add_sized(
                [clear_width, crate::theme::MIN_HIT],
                Button::new(label).selected(clear_focused),
            )
            .on_hover_text("Clear group filter");
        if clear.clicked() {
            app.end_text_edit();
            app.set_group_filter(id, String::new());
        }
    });
}

fn categories(ui: &mut egui::Ui, app: &mut App, id: BrowserId) {
    let current_row = app.category_row(id);
    for row in 0..app.categories_len(id) {
        let mut label = app.category_label(id, row).unwrap_or_default();
        if app.category_favorite(id, row) {
            label = format!("\u{2605} {label}");
        }
        let selected = current_row == row;
        let focused = selected && app.state(id).focus == BrowserFocus::Categories;
        let text = if row == 0 {
            RichText::new(label).color(crate::theme::ACCENT)
        } else {
            RichText::new(label)
        };
        let text = if focused {
            text.color(egui::Color32::BLACK)
        } else {
            text
        };
        let resp = ui.add_sized(
            [ui.available_width(), crate::theme::MIN_HIT],
            Button::new(text).selected(focused),
        );
        if resp.clicked() {
            app.end_text_edit();
            if let Some(category) = app.category_at(id, row) {
                app.set_category(id, category);
            }
            app.state_mut(id).focus = BrowserFocus::Categories;
        }
    }
    if app.no_groups_match(id) {
        ui.label(RichText::new("No groups match.").weak());
    }
}

fn noun(id: BrowserId) -> &'static str {
    match id {
        BrowserId::Live => "a channel",
        BrowserId::Movies => "a movie",
        BrowserId::Series => "a series",
    }
}

fn items(ui: &mut egui::Ui, app: &mut App, id: BrowserId, jump: bool) {
    let row_count = app.state(id).filtered.len();
    if row_count == 0 {
        let favorites = app.state(id).category == CategorySel::Favorites;
        ui.label(if !app.state(id).query.trim().is_empty() {
            "No matches.".to_owned()
        } else if favorites {
            format!("No favorites yet. Highlight {} and press Y.", noun(id))
        } else {
            "This group has no items. Choose another group.".to_owned()
        });
    }
    let now_ms = (ui.ctx().input(|i| i.time) * 1000.0) as u64;
    let stride = crate::theme::MIN_HIT + ui.spacing().item_spacing.y;
    app.page_rows[id as usize][1] = (ui.available_height() / stride).floor().max(1.0) as usize;
    let mut scroll = egui::ScrollArea::vertical().id_salt(("items", id));
    if jump && row_count > 0 {
        let offset = super::centered_row_offset(ui, app.state(id).selected, row_count);
        scroll = scroll.vertical_scroll_offset(offset);
    }
    let mut clicked = None;
    scroll.show_rows(ui, crate::theme::MIN_HIT, row_count, |ui, range| {
        for position in range {
            let Some(row) = app.browser_row(id, app.state(id).filtered[position]) else {
                continue;
            };
            let focused =
                app.state(id).selected == position && app.state(id).focus == BrowserFocus::Items;
            let star = if row.favorite { "  \u{2605}" } else { "" };
            let art = row
                .art
                .as_deref()
                .and_then(|key| app.art.texture(ui.ctx(), key));
            let label = format!("{}{star}", row.label);
            let resp = super::marked_row::show_with_detail(
                ui,
                egui::vec2(ui.available_width(), crate::theme::MIN_HIT),
                super::marked_row::RowContent {
                    text: &label,
                    detail: None,
                    marker: row.marker,
                    art: art.as_ref(),
                },
                focused,
                1,
            );
            if resp.clicked() {
                clicked = Some(position);
            }
        }
    });
    if let Some(position) = clicked {
        app.click_item(id, position, now_ms);
    }
}

fn search_row(ui: &mut egui::Ui, app: &mut App, id: BrowserId) {
    let wide = ui.available_width() >= WIDE_SEARCH_ROW;
    ui.horizontal(|ui| {
        let buttons = CLEAR_WIDTH + SORT_WIDTH + 2.0 * ui.spacing().item_spacing.x;
        let width = if wide {
            ui.available_width() - buttons
        } else {
            ui.available_width()
        };
        search_field(ui, app, id, width);
        if wide {
            search_buttons(ui, app, id);
        }
    });
    if !wide {
        ui.horizontal(|ui| search_buttons(ui, app, id));
    }
}

fn search_field(ui: &mut egui::Ui, app: &mut App, id: BrowserId, width: f32) {
    let mut query = app.state(id).query.clone();
    let response = ui.add_sized(
        [width, crate::theme::MIN_HIT],
        TextEdit::singleline(&mut query)
            .id(search_field_id(id))
            .hint_text("Search"),
    );
    if app.take_focus_request(TextTarget::Search(id)) {
        response.request_focus();
    }
    if response.clicked() || response.gained_focus() {
        app.start_search_edit(id);
    }
    if app.state(id).focus == BrowserFocus::Search {
        response.highlight();
    }
    if query != app.state(id).query {
        app.set_query(id, query);
    }
}

fn search_buttons(ui: &mut egui::Ui, app: &mut App, id: BrowserId) {
    let clear_focused = app.state(id).focus == BrowserFocus::Clear;
    let clear = RichText::new("Clear");
    let clear = if clear_focused {
        clear.color(egui::Color32::BLACK)
    } else {
        clear
    };
    if ui
        .add_sized(
            [CLEAR_WIDTH, crate::theme::MIN_HIT],
            Button::new(clear).selected(clear_focused),
        )
        .clicked()
    {
        app.end_text_edit();
        app.set_query(id, String::new());
        app.state_mut(id).focus = BrowserFocus::Clear;
    }
    let sort_focused = app.state(id).focus == BrowserFocus::Sort;
    let sort = RichText::new(app.state(id).sort.label());
    let sort = if sort_focused {
        sort.color(egui::Color32::BLACK)
    } else {
        sort
    };
    if ui
        .add_sized(
            [SORT_WIDTH, crate::theme::MIN_HIT],
            Button::new(sort).selected(sort_focused),
        )
        .clicked()
    {
        app.end_text_edit();
        app.toggle_sort(id);
        app.state_mut(id).focus = BrowserFocus::Sort;
    }
}
