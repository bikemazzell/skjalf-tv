//! Skjalf TV application crate: eframe shell, player engine, input, views.

pub mod app;
pub mod detect;
pub mod fonts;
pub mod gamepad;
pub mod glloader;
pub mod input;
pub mod net;
pub mod player;
pub mod setup_model;
pub mod steam_keyboard;
pub mod theme;
pub mod views;

use std::path::PathBuf;
use std::time::Duration;

use skjalf_core::nav::View;

use app::App;
use input::Action;

/// Display name: window title and Main header.
pub const APP_NAME: &str = "Skjalf TV";

fn config_base() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        })
}

/// Per-user config/state directory (`$XDG_CONFIG_HOME/skjalf`).
pub fn config_dir() -> PathBuf {
    config_base().join("skjalf")
}

/// Launch the app window (dark theme, fullscreen in Game Mode).
pub fn run() -> eframe::Result {
    let game_mode = detect::from_env();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("skjalf")
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!(
                    "../../../packaging/icons/skjalf-256.png"
                ))
                .expect("embedded icon is a valid PNG"),
            )
            .with_inner_size([1280.0, 800.0])
            .with_fullscreen(game_mode),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|cc| {
            theme::apply(&cc.egui_ctx);
            fonts::install(&cc.egui_ctx);
            let mut app = App::with_store(&config_dir());
            app.game_mode = game_mode;
            app.auto_refresh = true;
            app.gamepad = gamepad::GamepadThread::spawn();

            // Attach the mpv render context while the GL context is current.
            let (vol, ignore_ssl) = (app.settings.volume, app.settings.ignore_ssl);
            match player::MpvEngine::new(vol, ignore_ssl) {
                Ok(mut engine) if cc.gl.is_some() => {
                    // SAFETY: the eframe creator closure runs with the GL
                    // context current on this thread.
                    let ctx_cb = cc.egui_ctx.clone();
                    match unsafe {
                        engine.attach_render(glloader::get_proc_address, move || {
                            ctx_cb.request_repaint();
                        })
                    } {
                        Ok(()) => app.engine = Some(Box::new(engine)),
                        Err(error) => app.notice = Some(format!("Video setup failed: {error}")),
                    }
                }
                Ok(_) => app.notice = Some("OpenGL video output is unavailable.".into()),
                Err(error) => app.notice = Some(format!("Player setup failed: {error}")),
            }
            Ok(Box::new(app))
        }),
    )
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        theme::adapt_display(ctx, self.game_mode);
        let now_ms = (ctx.input(|i| i.time) * 1000.0) as u64;

        // Native Steam keyboard dismissal must reach the edit state before
        // the next controller event is routed.
        self.poll_keyboard();

        // Input → actions: keyboard events + gamepad queue.
        let events: Vec<egui::Event> = ctx.input(|i| i.events.clone());
        let mut consumed = Vec::new();
        for (index, event) in events.iter().enumerate() {
            if self.pump_event(event, now_ms) {
                consumed.push(index);
            }
        }
        ctx.input_mut(|input| {
            for index in consumed.into_iter().rev() {
                input.events.remove(index);
            }
        });
        if self.text_focus_release {
            ctx.memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            });
            self.text_focus_release = false;
        }
        if let Some(gp) = &mut self.gamepad {
            let focused = ctx.input(|input| input.viewport().focused) == Some(true);
            let view_at_drain = self.router.current();
            let queued = gp.drain(focused, view_at_drain.repeats_held_direction());
            for event in queued {
                // A Select may change the view in this same batch. Never carry
                // a queued held-direction repeat into the next view.
                if !event.repeated {
                    self.handle_action(event.action, now_ms);
                } else if self.router.current() == view_at_drain {
                    self.handle_repeat(event.action);
                }
            }
        }

        self.poll_non_keyboard_jobs(now_ms);
        self.live_refresh();

        if self.wants_exit() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        match self.router.current() {
            View::Setup => {
                let keyboard_field = self.active_steam_keyboard_field();
                let (connect, cancel, activated_field) = views::setup::show(
                    ctx,
                    &mut self.setup,
                    self.connect_rx.is_some(),
                    self.editing_source,
                    keyboard_field,
                );
                if let Some(connect) = connect {
                    self.start_connect(connect.0);
                }
                if let Some(field) = activated_field {
                    self.activate_setup_field(field);
                }
                if cancel {
                    self.finish_setup_edit();
                    self.handle_action(Action::Back, now_ms);
                }
            }
            View::Main => views::main_menu::show(ctx, self),
            View::Live => views::live::show(ctx, self),
            View::Settings => views::settings::show(ctx, self),
            View::Movies => views::movies::show(ctx, self),
            View::MovieDetails => views::movie_details::show(ctx, self, now_ms),
            View::Series => views::series::show(ctx, self),
            View::SeriesDetails => views::series_details::show(ctx, self, now_ms),
            View::Player => views::player::show(ctx, self, now_ms),
        }

        // Keep the loop warm for OSD timers and background jobs.
        ctx.request_repaint_after(Duration::from_millis(50));
    }
}
