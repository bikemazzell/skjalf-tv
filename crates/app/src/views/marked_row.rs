//! A list row with a watched or in-progress marker: dimmed text and a
//! "Watched" pill, or a "m:ss left" pill with a thin progress bar. All
//! drawing uses painter shapes, so row height never changes.

use egui::{Color32, FontId, Pos2, Rect, Response, Sense, Vec2};
use skjalf_core::progress::{format_left, Marker};

const DIM: f32 = 0.55;
const PILL_PADDING: Vec2 = Vec2::new(10.0, 3.0);
const BAR_HEIGHT: f32 = 3.0;

fn pill_label(marker: Marker) -> Option<String> {
    match marker {
        Marker::None => None,
        Marker::Watched => Some("Watched".into()),
        Marker::InProgress {
            left_secs: Some(left),
            ..
        } => Some(format_left(left)),
        Marker::InProgress { .. } => Some("In progress".into()),
    }
}

/// Draw a clickable row. `max_rows` limits the text lines (rows use 1).
pub fn show(
    ui: &mut egui::Ui,
    size: Vec2,
    text: &str,
    marker: Marker,
    selected: bool,
    max_rows: usize,
) -> Response {
    show_with_detail(ui, size, text, None, marker, selected, max_rows)
}

/// Like [`show`], with an optional second line under a one-line title.
pub fn show_with_detail(
    ui: &mut egui::Ui,
    size: Vec2,
    text: &str,
    detail: Option<&str>,
    marker: Marker,
    selected: bool,
    max_rows: usize,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
    let painter = ui.painter_at(rect);
    let background = if selected {
        crate::theme::ACCENT
    } else if response.hovered() {
        Color32::from_gray(52)
    } else {
        Color32::from_gray(38)
    };
    painter.rect_filled(rect, 4.0, background);

    let pill = pill_label(marker).map(|label| {
        let galley = painter.layout_no_wrap(label, FontId::proportional(14.0), Color32::WHITE);
        let pill_size = galley.size() + PILL_PADDING * 2.0;
        let pill_rect = Rect::from_min_size(
            Pos2::new(
                rect.max.x - pill_size.x - 8.0,
                rect.center().y - pill_size.y / 2.0,
            ),
            pill_size,
        );
        (galley, pill_rect)
    });
    let reserved = pill
        .as_ref()
        .map_or(0.0, |(_, pill_rect)| pill_rect.width() + 8.0);

    let text_color = match (selected, marker) {
        (true, _) => Color32::BLACK,
        (false, Marker::Watched) => Color32::WHITE.gamma_multiply(DIM),
        (false, _) => Color32::WHITE,
    };
    let mut job = egui::text::LayoutJob::simple(
        text.to_owned(),
        egui::TextStyle::Button.resolve(ui.style()),
        text_color,
        (rect.width() - 24.0 - reserved).max(40.0),
    );
    job.wrap.max_rows = max_rows;
    job.wrap.break_anywhere = true;
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    let detail_galley = detail.map(|detail| {
        let mut job = egui::text::LayoutJob::simple(
            detail.to_owned(),
            FontId::proportional(14.0),
            text_color.gamma_multiply(0.8),
            (rect.width() - 24.0).max(40.0),
        );
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    });
    let total = galley.size().y + detail_galley.as_ref().map_or(0.0, |g| g.size().y);
    let top = rect.center().y - total / 2.0;
    if let Some(detail_galley) = detail_galley {
        painter.galley(
            Pos2::new(rect.min.x + 12.0, top + galley.size().y),
            detail_galley,
            text_color,
        );
    }
    painter.galley(Pos2::new(rect.min.x + 12.0, top), galley, text_color);

    if let Some((label, pill_rect)) = pill {
        let fill = match (selected, marker) {
            (true, _) => Color32::from_black_alpha(200),
            (false, Marker::Watched) => Color32::from_rgb(40, 90, 62),
            (false, _) => Color32::from_gray(80),
        };
        painter.rect_filled(pill_rect, pill_rect.height() / 2.0, fill);
        painter.galley(pill_rect.min + PILL_PADDING, label, Color32::WHITE);
    }
    if let Marker::InProgress {
        fraction: Some(fraction),
        ..
    } = marker
    {
        let bar = Rect::from_min_max(
            Pos2::new(rect.min.x, rect.max.y - BAR_HEIGHT),
            Pos2::new(rect.min.x + rect.width() * fraction as f32, rect.max.y),
        );
        let color = if selected {
            Color32::BLACK
        } else {
            crate::theme::ACCENT
        };
        painter.rect_filled(bar, 0.0, color);
    }
    response
}
