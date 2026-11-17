//! Setup view: source form with native text editing.

use egui::{Button, TextEdit};

use crate::setup_model::{Connect, Field, SetupModel};

/// Render the setup view; returns a [`Connect`] when the user commits a
/// complete source (keyboard Select or mouse click on Connect).
pub fn show(
    ctx: &egui::Context,
    m: &mut SetupModel,
    busy: bool,
    editing: bool,
    keyboard_field: Option<Field>,
) -> (Option<Connect>, bool, Option<Field>) {
    let mut connect = None;
    let mut cancel = false;
    let mut activated_field = None;
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.add_space(12.0);
        ui.heading("Skjalf TV \u{2014} IPTV Setup");
        if editing && ui.button("Cancel Edit").clicked() {
            cancel = true;
        }
        ui.add_space(8.0);

        if let Some(field) = keyboard_field {
            // Game Mode's Steam keyboard covers the lower half of the Deck.
            // Keep the one active native editor at the top while it is open.
            ui.label(field_label(field));
            let width = ui.available_width().min(700.0);
            let focus_request = m.focus_request.take();
            let response = field_editor(ui, m, field, width, focus_request);
            if response.clicked() || response.gained_focus() {
                m.focus_field(field);
                activated_field = Some(field);
            }
            ui.label("B / Return closes the Steam keyboard");
            return;
        }

        if let Some(msg) = &m.message {
            ui.colored_label(egui::Color32::LIGHT_BLUE, msg);
        }

        // -- form -----------------------------------------------------------
        egui::Grid::new("form")
            .num_columns(2)
            .spacing([24.0, 10.0])
            .show(ui, |ui| {
                let focus_request = m.focus_request.take();
                ui.label("Source");
                ui.horizontal(|ui| {
                    let xtream_selected = m.xtream && m.row_is_kind();
                    if ui
                        .add(
                            Button::new(egui::RichText::new("Xtream").color(if xtream_selected {
                                egui::Color32::BLACK
                            } else {
                                egui::Color32::WHITE
                            }))
                            .selected(xtream_selected),
                        )
                        .clicked()
                    {
                        m.set_xtream(true);
                    }
                    let m3u_selected = !m.xtream && m.row_is_kind();
                    if ui
                        .add(
                            Button::new(egui::RichText::new("M3U").color(if m3u_selected {
                                egui::Color32::BLACK
                            } else {
                                egui::Color32::WHITE
                            }))
                            .selected(m3u_selected),
                        )
                        .clicked()
                    {
                        m.set_xtream(false);
                    }
                });
                ui.end_row();

                for (i, field) in m.fields().iter().enumerate() {
                    ui.label(field_label(*field));
                    let selected = m.row_is_field(i);
                    let width = ui.available_width().clamp(320.0, 700.0);
                    let response = field_editor(ui, m, *field, width, focus_request);
                    if response.clicked() || response.gained_focus() {
                        m.focus_field(*field);
                        activated_field = Some(*field);
                    }
                    if selected {
                        response.highlight();
                    }
                    ui.end_row();
                }

                ui.label("");
                let connect_selected = m.row_is_connect();
                if ui
                    .add_enabled_ui(!busy, |ui| {
                        ui.add_sized(
                            [220.0, crate::theme::MIN_HIT],
                            Button::new(egui::RichText::new("Connect").color(
                                if connect_selected {
                                    egui::Color32::BLACK
                                } else {
                                    egui::Color32::WHITE
                                },
                            ))
                            .selected(connect_selected),
                        )
                    })
                    .inner
                    .clicked()
                {
                    connect = m.connect();
                }
                ui.end_row();
            });
    });
    (connect, cancel, activated_field)
}

fn field_id(field: Field) -> egui::Id {
    egui::Id::new(("setup-field", field_label(field)))
}

fn field_editor(
    ui: &mut egui::Ui,
    m: &mut SetupModel,
    field: Field,
    width: f32,
    focus_request: Option<Field>,
) -> egui::Response {
    let response = ui.add_sized(
        [width, crate::theme::MIN_HIT],
        TextEdit::singleline(m.field_text_mut(field))
            .id(field_id(field))
            .password(field == Field::Password)
            .hint_text("Enter text"),
    );
    if focus_request == Some(field) {
        response.request_focus();
    }
    response
}

fn field_label(field: Field) -> &'static str {
    match field {
        Field::Host => "Host",
        Field::Username => "Username",
        Field::Password => "Password",
        Field::Url => "M3U URL",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 400.0),
            )),
            ..Default::default()
        }
    }

    #[test]
    fn active_game_mode_field_stays_above_keyboard_at_deck_scale() {
        for (field, editing) in [
            (Field::Host, false),
            (Field::Username, true),
            (Field::Password, true),
            (Field::Url, true),
        ] {
            let ctx = egui::Context::default();
            let mut model = SetupModel {
                xtream: field != Field::Url,
                url: if field == Field::Url {
                    format!("https://example.com/{}", "a".repeat(1000))
                } else {
                    String::new()
                },
                ..Default::default()
            };
            model.activate_field(field);
            let _ = ctx.run(input(), |ctx| {
                show(ctx, &mut model, false, editing, Some(field));
            });
            let rect = ctx.read_response(field_id(field)).unwrap().rect;
            assert!(rect.max.y < 200.0, "{field:?} is at {rect:?}");
            assert!(rect.width() > 200.0, "{field:?} is too narrow: {rect:?}");
            if field == Field::Url {
                assert_eq!(model.url.len(), "https://example.com/".len() + 1000);
            }
        }
    }

    #[test]
    fn field_keeps_focus_in_compact_view_then_releases_it_on_exit() {
        let ctx = egui::Context::default();
        let mut model = SetupModel {
            host: "example.com".into(),
            ..Default::default()
        };
        model.activate_field(Field::Host);
        let _ = ctx.run(input(), |ctx| {
            show(ctx, &mut model, false, false, None);
        });
        let id = field_id(Field::Host);
        assert_eq!(ctx.memory(|m| m.focused()), Some(id));
        let _ = ctx.run(input(), |ctx| {
            show(ctx, &mut model, false, false, Some(Field::Host));
        });
        assert_eq!(ctx.memory(|m| m.focused()), Some(id));
        assert_eq!(model.host, "example.com");
        let mut typing = input();
        typing.events.push(egui::Event::Text("x".into()));
        let _ = ctx.run(typing, |ctx| {
            show(ctx, &mut model, false, false, Some(Field::Host));
        });
        assert!(model.host.contains('x'));
        ctx.memory_mut(|memory| memory.surrender_focus(id));
        let _ = ctx.run(input(), |ctx| {
            show(ctx, &mut model, false, false, None);
        });
        assert_eq!(ctx.memory(|m| m.focused()), None);
        assert_eq!(model.row, crate::setup_model::Row::Field(0));
    }

    #[test]
    fn compact_password_field_paints_masked_text() {
        let ctx = egui::Context::default();
        let mut model = SetupModel {
            password: "VisibleOnlyInModel123".into(),
            ..Default::default()
        };
        model.activate_field(Field::Password);
        let output = ctx.run(input(), |ctx| {
            show(ctx, &mut model, false, false, Some(Field::Password));
        });
        assert_eq!(model.password, "VisibleOnlyInModel123");
        assert!(!format!("{:?}", output.shapes).contains("VisibleOnlyInModel123"));
    }
}
