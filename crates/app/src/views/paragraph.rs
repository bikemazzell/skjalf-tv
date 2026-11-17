//! Paragraph text for provider descriptions. Left-to-right text wraps as
//! usual. Right-to-left text is wrapped first and reordered per line, then
//! right-aligned; the result is cached per (text, width).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use egui::{Align, Color32, Label, Layout, TextWrapMode};
use skjalf_core::display::{has_rtl, wrap_rtl, WrappedLine};

const CACHE_LIMIT: usize = 32;

type Wrapped = Arc<Vec<WrappedLine>>;

#[derive(Clone, Default)]
struct WrapCache(Arc<Mutex<HashMap<(String, u32), Wrapped>>>);

pub fn show(ui: &mut egui::Ui, text: &str) {
    if !has_rtl(text) {
        ui.label(text);
        return;
    }
    let width = (ui.available_width() - 4.0).max(40.0);
    let lines = wrapped_lines(ui, text, width);
    ui.with_layout(Layout::top_down(Align::Max), |ui| {
        for line in lines.iter() {
            if line.rtl {
                ui.add(Label::new(line.text.as_str()).wrap_mode(TextWrapMode::Extend));
            } else {
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    ui.label(line.text.as_str())
                });
            }
        }
    });
}

fn wrapped_lines(ui: &egui::Ui, text: &str, width: f32) -> Wrapped {
    let id = egui::Id::new("rtl-paragraph-cache");
    let cache: WrapCache = ui
        .ctx()
        .data_mut(|data| data.get_temp_mut_or_default::<WrapCache>(id).clone());
    let key = (text.to_owned(), width.round() as u32);
    if let Some(lines) = cache.0.lock().ok().and_then(|map| map.get(&key).cloned()) {
        return lines;
    }
    let font = egui::TextStyle::Body.resolve(ui.style());
    let measure = |line: &str| {
        ui.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(line.to_owned(), font.clone(), Color32::WHITE)
                .size()
                .x
        })
    };
    let lines = Arc::new(wrap_rtl(text, width, &measure));
    if let Ok(mut map) = cache.0.lock() {
        if map.len() >= CACHE_LIMIT {
            map.clear();
        }
        map.insert(key, lines.clone());
    }
    lines
}
