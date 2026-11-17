//! Player engine: `PlayerEngine` trait + libmpv implementation (rsmpv).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rsmpv::render::OwnedRenderContext;
use rsmpv::Mpv;

/// Media commands the UI can issue to the player.
pub trait PlayerEngine {
    /// Start playing `url` at `start_secs`, replacing anything current.
    /// `None` starts at the beginning, so a previous start never leaks in.
    fn load_at(&mut self, url: &str, start_secs: Option<f64>) -> Result<(), PlayerError>;
    /// Start playing `url` from the beginning (zap semantics).
    fn load(&mut self, url: &str) -> Result<(), PlayerError> {
        self.load_at(url, None)
    }
    fn seek_relative(&mut self, _secs: f64) -> Result<(), PlayerError> {
        Ok(())
    }
    /// Playback position in seconds, if a file is playing.
    fn position(&self) -> Option<f64> {
        None
    }
    /// Media duration in seconds, if known.
    fn duration(&self) -> Option<f64> {
        None
    }
    fn stop(&mut self) -> Result<(), PlayerError>;
    /// 0–100.
    fn set_volume(&mut self, volume: u8) -> Result<(), PlayerError>;
    fn pause(&mut self, paused: bool) -> Result<(), PlayerError>;
    fn is_paused(&self) -> bool;
    /// Drain playback state changes from the engine's persistent event queue.
    fn take_events(&mut self) -> Vec<PlayerEvent> {
        Vec::new()
    }
    /// Drain asynchronously reported playback errors (default: none).
    fn take_errors(&mut self) -> Vec<String> {
        Vec::new()
    }
    /// Shared render context for the GL video bridge, if attached.
    fn render_context(&self) -> Option<Arc<Mutex<OwnedRenderContext>>> {
        None
    }
    /// Shared error slot written by the GL paint callback and drained by the app.
    fn render_error_sink(&self) -> Option<Arc<Mutex<Option<String>>>> {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerEvent {
    Loaded,
    /// The file played to its end.
    Ended,
    Failed(String),
}

#[derive(Debug, thiserror::Error)]
#[error("player: {0}")]
pub struct PlayerError(pub String);

impl From<rsmpv::Error> for PlayerError {
    fn from(e: rsmpv::Error) -> Self {
        PlayerError(e.to_string())
    }
}

/// A libmpv-backed engine. The optional render context renders video into an
/// OpenGL framebuffer (see [`crate::glbridge`]).
pub struct MpvEngine {
    mpv: Arc<Mpv>,
    render: Option<Arc<Mutex<OwnedRenderContext>>>,
    render_error: Arc<Mutex<Option<String>>>,
    ended: EndWatch,
}

/// Detects end of file. With `keep-open` mpv pauses at the end instead of
/// sending end-file, so the `eof-reached` property is polled at a low rate.
struct EndWatch {
    reported: bool,
    last_check: Option<Instant>,
}

impl EndWatch {
    const CHECK_EVERY: Duration = Duration::from_millis(500);

    fn new() -> Self {
        Self {
            reported: false,
            last_check: None,
        }
    }

    fn reset(&mut self) {
        self.reported = false;
        self.last_check = None;
    }
}

impl MpvEngine {
    /// Engine configured for on-screen playback (vo chosen by render API).
    ///
    /// `ignore_ssl` mirrors the app setting; TLS verification stays ON
    /// otherwise (stream URLs carry credentials).
    pub fn new(initial_volume: u8, ignore_ssl: bool) -> Result<Self, PlayerError> {
        let mpv = Self::base_builder(ignore_ssl)?
            .set_property("vo", "libmpv")?
            .set_property("volume", initial_volume.min(100) as i64)?
            .build()?;
        Ok(Self {
            mpv: Arc::new(mpv),
            render: None,
            render_error: Arc::new(Mutex::new(None)),
            ended: EndWatch::new(),
        })
    }

    /// Engine without video/audio output — for integration tests.
    pub fn headless() -> Result<Self, PlayerError> {
        let mpv = Self::base_builder(false)?
            .set_property("vo", "null")?
            .set_property("ao", "null")?
            .build()?;
        Ok(Self {
            mpv: Arc::new(mpv),
            render: None,
            render_error: Arc::new(Mutex::new(None)),
            ended: EndWatch::new(),
        })
    }

    fn base_builder(ignore_ssl: bool) -> Result<rsmpv::Builder, PlayerError> {
        Ok(Mpv::builder()?
            .set_property("hwdec", "auto-safe")?
            .set_property("cache", "yes")?
            .set_property("prefetch-playlist", "yes")?
            .set_property("keep-open", "yes")?
            .set_property("idle", "yes")?
            .set_property("tls-verify", !ignore_ssl)?)
    }

    /// Attach the GL render context (called once on the GL thread) and hook
    /// mpv's update callback to `repaint` (drive repaints at video rate).
    ///
    /// # Safety
    /// The caller must guarantee a current OpenGL context on this thread for
    /// the duration of the call (eframe's app-creator and paint callbacks do).
    pub unsafe fn attach_render(
        &mut self,
        get_proc_address: impl Fn(&str) -> *mut std::ffi::c_void + Send + 'static,
        repaint: impl Fn() + Send + Sync + 'static,
    ) -> Result<(), PlayerError> {
        let mut ctx = rsmpv::render::RenderContext::new_opengl(
            self.mpv.clone(),
            false,
            Box::new(get_proc_address),
        )
        .map_err(|e| PlayerError(e.to_string()))?;
        ctx.set_update_callback(repaint);
        self.render = Some(Arc::new(Mutex::new(ctx)));
        Ok(())
    }

    /// Shared render context for the egui paint callback (None until
    /// [`Self::attach_render`]).
    pub fn render_context(&self) -> Option<Arc<Mutex<OwnedRenderContext>>> {
        self.render.clone()
    }

    pub fn with_client<T>(
        &self,
        f: impl FnOnce(&rsmpv::Client<'_>) -> Result<T, PlayerError>,
    ) -> Result<T, PlayerError> {
        let client = self.mpv.create_client(Some("skjalf"))?;
        f(&client)
    }

    /// Poll one pending mpv event (None if queue empty).
    pub fn poll_event(&self) -> Option<rsmpv::Event> {
        self.mpv.poll_event()
    }
}

impl MpvEngine {
    fn push_ended(&mut self, events: &mut Vec<PlayerEvent>) {
        if !self.ended.reported {
            self.ended.reported = true;
            events.push(PlayerEvent::Ended);
        }
    }

    fn check_end_of_file(&mut self, events: &mut Vec<PlayerEvent>) {
        if self.ended.reported
            || self
                .ended
                .last_check
                .is_some_and(|at| at.elapsed() < EndWatch::CHECK_EVERY)
        {
            return;
        }
        self.ended.last_check = Some(Instant::now());
        if self.mpv.get_property("eof-reached").unwrap_or(false) {
            self.push_ended(events);
        }
    }
}

impl PlayerEngine for MpvEngine {
    fn load_at(&mut self, url: &str, start_secs: Option<f64>) -> Result<(), PlayerError> {
        let start = start_secs.map_or_else(|| "none".to_owned(), |secs| format!("{secs:.3}"));
        self.mpv.set_property("start", start.as_str())?;
        self.ended.reset();
        self.mpv.command(&["loadfile", url, "replace"])?;
        Ok(())
    }

    fn seek_relative(&mut self, secs: f64) -> Result<(), PlayerError> {
        self.ended.reset();
        self.mpv
            .command(&["seek", &format!("{secs:.3}"), "relative"])?;
        Ok(())
    }

    fn position(&self) -> Option<f64> {
        self.mpv.get_property("time-pos").ok()
    }

    fn duration(&self) -> Option<f64> {
        self.mpv.get_property("duration").ok()
    }

    fn stop(&mut self) -> Result<(), PlayerError> {
        self.mpv.command(&["stop"])?;
        Ok(())
    }

    fn set_volume(&mut self, volume: u8) -> Result<(), PlayerError> {
        self.mpv
            .set_property("volume", i64::from(volume.min(100)))?;
        Ok(())
    }

    fn pause(&mut self, paused: bool) -> Result<(), PlayerError> {
        self.mpv.set_property("pause", paused)?;
        Ok(())
    }

    fn is_paused(&self) -> bool {
        self.mpv.get_property("pause").unwrap_or(false)
    }

    fn take_errors(&mut self) -> Vec<String> {
        self.take_events()
            .into_iter()
            .filter_map(|event| match event {
                PlayerEvent::Failed(error) => Some(error),
                PlayerEvent::Loaded | PlayerEvent::Ended => None,
            })
            .collect()
    }

    fn take_events(&mut self) -> Vec<PlayerEvent> {
        let mut events = Vec::new();
        if let Ok(mut error) = self.render_error.lock() {
            if let Some(message) = error.take() {
                events.push(PlayerEvent::Failed(message));
            }
        }
        while let Some(event) = self.poll_event() {
            match event {
                rsmpv::Event::FileLoaded => events.push(PlayerEvent::Loaded),
                rsmpv::Event::EndFile {
                    reason: rsmpv::EndFileReason::Eof,
                    ..
                } => self.push_ended(&mut events),
                rsmpv::Event::EndFile {
                    reason: rsmpv::EndFileReason::Error,
                    error,
                    ..
                } => events.push(PlayerEvent::Failed(
                    error.map_or_else(|| "Media could not be opened.".into(), |e| e.to_string()),
                )),
                _ => {}
            }
        }
        self.check_end_of_file(&mut events);
        events
    }

    fn render_context(&self) -> Option<Arc<Mutex<OwnedRenderContext>>> {
        MpvEngine::render_context(self)
    }

    fn render_error_sink(&self) -> Option<Arc<Mutex<Option<String>>>> {
        Some(self.render_error.clone())
    }
}
