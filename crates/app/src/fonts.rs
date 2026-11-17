//! System font fallbacks: provider names use glyphs egui's bundled fonts lack.

use std::path::{Path, PathBuf};
use std::process::Command;

use egui::{FontData, FontDefinitions, FontFamily};

const FALLBACK_FAMILIES: [&str; 5] = [
    "DejaVu Sans",
    "Noto Sans",
    "Noto Sans Arabic",
    "Noto Sans Symbols",
    "Noto Sans Symbols 2",
];
const FALLBACK_FILES: [&str; 5] = [
    "DejaVuSans.ttf",
    "NotoSans-Regular.ttf",
    "NotoSansArabic-Regular.ttf",
    "NotoSansSymbols-Regular.ttf",
    "NotoSansSymbols2-Regular.ttf",
];
const FONT_ROOTS: [&str; 2] = ["/usr/share/fonts", "/usr/local/share/fonts"];

/// A fallback font loaded from the system.
pub struct SystemFont {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// Resolve and read the fallback fonts that exist on this machine, once each.
pub fn load_system_fonts() -> Vec<SystemFont> {
    let mut paths: Vec<PathBuf> = Vec::new();
    match FALLBACK_FAMILIES
        .iter()
        .map(|family| fc_match(family))
        .collect::<Option<Vec<_>>>()
    {
        Some(matches) => paths.extend(matches.into_iter().flatten()),
        None => paths.extend(scan_font_roots()),
    }
    let mut fonts: Vec<SystemFont> = Vec::new();
    let mut seen = Vec::new();
    for path in paths {
        if seen.contains(&path) {
            continue;
        }
        seen.push(path.clone());
        if let Ok(bytes) = std::fs::read(&path) {
            let name = format!(
                "system-{}",
                path.file_stem().and_then(|s| s.to_str()).unwrap_or("font")
            );
            fonts.push(SystemFont { name, bytes });
        }
    }
    fonts
}

/// `Some(path)` when fontconfig answers with the requested family, `Some(None)`
/// when it answers with a different family, `None` when `fc-match` is missing.
fn fc_match(family: &str) -> Option<Option<PathBuf>> {
    let output = Command::new("fc-match")
        .arg("--format=%{family}\t%{file}")
        .arg(family)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let (families, file) = text.split_once('\t')?;
    let squash = |name: &str| name.replace(' ', "").to_ascii_lowercase();
    let matches_family = families
        .split(',')
        .any(|candidate| squash(candidate) == squash(family));
    Some(
        matches_family
            .then(|| PathBuf::from(file))
            .filter(|p| p.is_file()),
    )
}

fn scan_font_roots() -> Vec<PathBuf> {
    let mut found = Vec::new();
    for root in FONT_ROOTS {
        collect_named(Path::new(root), 0, &mut found);
    }
    found.sort_by_key(|path| {
        FALLBACK_FILES
            .iter()
            .position(|name| path.file_name().is_some_and(|f| f == *name))
    });
    found
}

fn collect_named(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && depth < 4 {
            collect_named(&path, depth + 1, found);
        } else if FALLBACK_FILES
            .iter()
            .any(|name| path.file_name().is_some_and(|f| f == *name))
        {
            found.push(path);
        }
    }
}

/// egui's defaults first, then each fallback appended to both families.
pub fn definitions(fonts: Vec<SystemFont>) -> FontDefinitions {
    let mut definitions = FontDefinitions::default();
    for font in fonts {
        definitions.font_data.insert(
            font.name.clone(),
            std::sync::Arc::new(FontData::from_owned(font.bytes)),
        );
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            if let Some(names) = definitions.families.get_mut(&family) {
                names.push(font.name.clone());
            }
        }
    }
    definitions
}

pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(definitions(load_system_fonts()));
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\u{1D3F}\u{1D2C}\u{1D42} \u{1D34}\u{1D30} \u{2076}\u{2070} \u{25C9} \u{26BD} \u{0639}\u{0631}\u{0628}\u{064A}";

    #[test]
    fn fallback_fonts_cover_provider_name_glyphs() {
        let fonts = load_system_fonts();
        let names: Vec<_> = fonts.iter().map(|font| font.name.clone()).collect();
        println!("fallback fonts found: {names:?}");
        let ctx = egui::Context::default();
        ctx.set_fonts(definitions(fonts));
        let _ = ctx.run(egui::RawInput::default(), |_| {});
        let font_id = egui::FontId::proportional(18.0);
        let has = |c: char| ctx.fonts_mut(|fonts| fonts.has_glyph(&font_id, c));
        let any_found = |needles: &[&str]| {
            needles
                .iter()
                .any(|needle| names.iter().any(|name| name.contains(needle)))
        };
        for c in SAMPLE.chars().filter(|c| !c.is_whitespace()) {
            let covered = if ('\u{0600}'..='\u{06FF}').contains(&c) {
                any_found(&["Arabic", "DejaVuSans"])
            } else if matches!(c, '\u{25C9}' | '\u{26BD}') {
                any_found(&["Symbols", "DejaVuSans"])
            } else {
                any_found(&["DejaVuSans", "NotoSans-Regular"])
            };
            if covered {
                assert!(has(c), "U+{:04X} has no glyph", c as u32);
            } else {
                println!("skipping U+{:04X}: no covering font found", c as u32);
            }
        }
    }

    #[test]
    fn shaped_persian_presentation_forms_have_glyphs() {
        let fonts = load_system_fonts();
        let has_arabic_font = fonts
            .iter()
            .any(|font| font.name.contains("Arabic") || font.name.contains("DejaVuSans"));
        let ctx = egui::Context::default();
        ctx.set_fonts(definitions(fonts));
        let _ = ctx.run(egui::RawInput::default(), |_| {});
        if !has_arabic_font {
            println!("skipping: no Arabic-capable fallback font on this machine");
            return;
        }
        let font_id = egui::FontId::proportional(18.0);
        let shaped = skjalf_core::display::visual_text("دنیای دیگر سلام");
        for c in shaped.chars().filter(|c| !c.is_whitespace()) {
            assert!(
                ctx.fonts_mut(|fonts| fonts.has_glyph(&font_id, c)),
                "U+{:04X} has no glyph",
                c as u32
            );
        }
    }

    #[test]
    fn egui_defaults_stay_first_and_missing_fonts_change_nothing() {
        let plain = definitions(Vec::new());
        let defaults = FontDefinitions::default();
        assert_eq!(plain.families, defaults.families);
        let with = definitions(vec![SystemFont {
            name: "extra".into(),
            bytes: Vec::new(),
        }]);
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            let names = &with.families[&family];
            assert_eq!(names.last().map(String::as_str), Some("extra"));
            assert_eq!(names[..names.len() - 1], defaults.families[&family][..]);
        }
    }
}
