//! Player engine: `PlayerEngine` trait + libmpv implementation (rsmpv).

use std::path::Path;
use std::sync::{Arc, Mutex};

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
    /// How much video to buffer for the file just loaded. Call after
    /// `load_at` and `pause(false)`: a start wait pauses the file until the
    /// buffer fills.
    fn set_buffering(&mut self, _plan: BufferPlan) -> Result<(), PlayerError> {
        Ok(())
    }
    /// Buffer state of the current file, if the engine reports one.
    fn buffer_status(&self) -> Option<BufferStatus> {
        None
    }
    /// End a buffering wait and play what is already buffered.
    fn start_now(&mut self) -> Result<(), PlayerError> {
        Ok(())
    }
}

/// Seconds of video to buffer before playback starts and after a stall.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BufferPlan {
    /// 0 starts as soon as mpv can play.
    pub start_secs: f64,
    pub stall_secs: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BufferStatus {
    /// Playback waits for the buffer to fill.
    pub waiting: bool,
    /// Seconds of video buffered ahead of the playback position.
    pub ahead_secs: Option<f64>,
    /// Seconds the current wait needs.
    pub target_secs: f64,
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
///
/// The UI thread also renders video, so per-frame code never waits on the mpv
/// core: while mpv reconfigures video it waits for the render thread, and a
/// synchronous property read from that thread then deadlocks until mpv gives
/// up and leaves the video black. State comes from observed properties
/// instead, and per-frame changes use asynchronous requests.
pub struct MpvEngine {
    mpv: Arc<Mpv>,
    render: Option<Arc<Mutex<OwnedRenderContext>>>,
    render_error: Arc<Mutex<Option<String>>>,
    state: Mutex<Observed>,
    /// Seconds to buffer before the paused file starts.
    start_target: Option<f64>,
    /// The configured stall wait.
    stall_wait: f64,
    /// The stall wait to restore after "start now" ended a stall early.
    pending_stall_wait: Option<f64>,
}

/// Forward buffer limit when the cache lives on disk. In memory, mpv keeps
/// its own default of 150 MiB.
const DISK_CACHE_LIMIT: &str = "4GiB";
/// The wait that "start now" sets: play what is already buffered.
const START_NOW_WAIT: f64 = 0.5;
/// mpv's own stall wait, used until a buffering plan arrives.
const DEFAULT_STALL_WAIT: f64 = 1.0;

const OBSERVED: [(&str, rsmpv::Format); 7] = [
    ("time-pos", rsmpv::Format::Double),
    ("duration", rsmpv::Format::Double),
    ("demuxer-cache-duration", rsmpv::Format::Double),
    ("demuxer-cache-idle", rsmpv::Format::Flag),
    ("paused-for-cache", rsmpv::Format::Flag),
    ("core-idle", rsmpv::Format::Flag),
    ("eof-reached", rsmpv::Format::Flag),
];

/// Playback state from mpv's event queue, for the current file only.
#[derive(Debug)]
struct Observed {
    /// Property events before `StartFile` still describe the previous file.
    started: bool,
    position: Option<f64>,
    duration: Option<f64>,
    ahead: Option<f64>,
    cache_idle: bool,
    paused_for_cache: bool,
    core_idle: bool,
    /// With `keep-open` mpv pauses at the end instead of ending the file.
    eof: bool,
    ended_reported: bool,
    events: Vec<PlayerEvent>,
}

impl Default for Observed {
    fn default() -> Self {
        Self {
            started: false,
            position: None,
            duration: None,
            ahead: None,
            cache_idle: false,
            paused_for_cache: false,
            core_idle: true,
            eof: false,
            ended_reported: false,
            events: Vec::new(),
        }
    }
}

impl Observed {
    /// Forget the previous file; keep undelivered events.
    fn reset_for_load(&mut self) {
        let events = std::mem::take(&mut self.events);
        *self = Self {
            events,
            ..Self::default()
        };
    }

    fn apply(&mut self, event: rsmpv::Event) {
        match event {
            rsmpv::Event::StartFile { .. } => self.started = true,
            rsmpv::Event::FileLoaded => self.events.push(PlayerEvent::Loaded),
            rsmpv::Event::EndFile {
                reason: rsmpv::EndFileReason::Eof,
                ..
            } => self.eof = true,
            rsmpv::Event::EndFile {
                reason: rsmpv::EndFileReason::Error,
                error,
                ..
            } => self.events.push(PlayerEvent::Failed(
                error.map_or_else(|| "Media could not be opened.".into(), |e| e.to_string()),
            )),
            rsmpv::Event::SetPropertyReply {
                result: Err(error), ..
            } => self.events.push(PlayerEvent::Failed(error.to_string())),
            rsmpv::Event::PropertyChange { name, data, .. } if self.started => {
                self.observe(&name, data)
            }
            _ => {}
        }
        if self.eof && !self.ended_reported {
            self.ended_reported = true;
            self.events.push(PlayerEvent::Ended);
        }
    }

    fn observe(&mut self, name: &str, data: rsmpv::PropertyData) {
        let number = match data {
            rsmpv::PropertyData::Double(value) => Some(value),
            _ => None,
        };
        let flag = match data {
            rsmpv::PropertyData::Flag(value) => Some(value),
            _ => None,
        };
        match name {
            "time-pos" => self.position = number,
            "duration" => self.duration = number,
            "demuxer-cache-duration" => self.ahead = number,
            "demuxer-cache-idle" => self.cache_idle = flag.unwrap_or(false),
            "paused-for-cache" => self.paused_for_cache = flag.unwrap_or(false),
            "core-idle" => self.core_idle = flag.unwrap_or(true),
            "eof-reached" => self.eof = flag.unwrap_or(false),
            _ => {}
        }
    }
}

impl MpvEngine {
    /// Engine configured for on-screen playback (vo chosen by render API).
    ///
    /// `ignore_ssl` mirrors the app setting; TLS verification stays ON
    /// otherwise (stream URLs carry credentials). With `disk_cache`, mpv
    /// buffers to an unlinked file there, so long buffers use no memory.
    pub fn new(
        initial_volume: u8,
        ignore_ssl: bool,
        disk_cache: Option<&Path>,
    ) -> Result<Self, PlayerError> {
        let mut builder = Self::base_builder(ignore_ssl)?
            .set_property("vo", "libmpv")?
            .set_property("volume", initial_volume.min(100) as i64)?;
        if let Some(dir) = disk_cache.and_then(Path::to_str) {
            builder = builder
                .set_property("cache-on-disk", true)?
                .set_property("cache-dir", dir)?
                .set_property("demuxer-max-bytes", DISK_CACHE_LIMIT)?;
        }
        Self::from_mpv(builder.build()?)
    }

    fn from_mpv(mpv: Mpv) -> Result<Self, PlayerError> {
        for (name, format) in OBSERVED {
            mpv.observe_property(0, name, format)?;
        }
        Ok(Self {
            mpv: Arc::new(mpv),
            render: None,
            render_error: Arc::new(Mutex::new(None)),
            state: Mutex::new(Observed::default()),
            start_target: None,
            stall_wait: DEFAULT_STALL_WAIT,
            pending_stall_wait: None,
        })
    }

    /// Engine without video/audio output — for integration tests.
    pub fn headless() -> Result<Self, PlayerError> {
        let mpv = Self::base_builder(false)?
            .set_property("vo", "null")?
            .set_property("ao", "null")?
            .build()?;
        Self::from_mpv(mpv)
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

    /// Drain mpv's event queue into the observed state. Never blocks.
    fn pump(&self) -> std::sync::MutexGuard<'_, Observed> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        while let Some(event) = self.mpv.poll_event() {
            state.apply(event);
        }
        state
    }

    fn set_async(&self, name: &str, value: impl Into<rsmpv::Node>) -> Result<(), PlayerError> {
        self.mpv.set_property_async(0, name, value)?;
        Ok(())
    }

    /// Start the paused file once the buffer holds the start wait, or once
    /// mpv stops reading because the file or the cache limit is reached.
    fn apply_start_wait(&mut self) -> Result<(), PlayerError> {
        let Some(target) = self.start_target else {
            return Ok(());
        };
        let full = {
            let state = self.pump();
            state
                .ahead
                .is_some_and(|ahead| ahead >= target || state.cache_idle)
        };
        if full {
            self.start_target = None;
            self.set_async("pause", false)?;
        }
        Ok(())
    }

    /// Restore the stall wait once playback runs; `core-idle` is false
    /// only while video actually plays.
    fn apply_stall_wait(&mut self) -> Result<(), PlayerError> {
        let Some(wait) = self.pending_stall_wait else {
            return Ok(());
        };
        if !self.pump().core_idle {
            self.pending_stall_wait = None;
            self.set_async("cache-pause-wait", wait)?;
        }
        Ok(())
    }
}

impl PlayerEngine for MpvEngine {
    fn load_at(&mut self, url: &str, start_secs: Option<f64>) -> Result<(), PlayerError> {
        let start = start_secs.map_or_else(|| "none".to_owned(), |secs| format!("{secs:.3}"));
        self.mpv.set_property("start", start.as_str())?;
        self.pump().reset_for_load();
        self.start_target = None;
        self.pending_stall_wait = None;
        self.mpv.command(&["loadfile", url, "replace"])?;
        Ok(())
    }

    fn seek_relative(&mut self, secs: f64) -> Result<(), PlayerError> {
        {
            let mut state = self.pump();
            state.eof = false;
            state.ended_reported = false;
        }
        self.mpv
            .command(&["seek", &format!("{secs:.3}"), "relative"])?;
        Ok(())
    }

    fn position(&self) -> Option<f64> {
        self.pump().position
    }

    fn duration(&self) -> Option<f64> {
        self.pump().duration
    }

    fn stop(&mut self) -> Result<(), PlayerError> {
        self.start_target = None;
        self.pending_stall_wait = None;
        self.mpv.command(&["stop"])?;
        self.pump().reset_for_load();
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
        if let Err(error) = self
            .apply_start_wait()
            .and_then(|()| self.apply_stall_wait())
        {
            events.push(PlayerEvent::Failed(error.0));
        }
        events.append(&mut self.pump().events);
        events
    }

    fn render_context(&self) -> Option<Arc<Mutex<OwnedRenderContext>>> {
        MpvEngine::render_context(self)
    }

    fn render_error_sink(&self) -> Option<Arc<Mutex<Option<String>>>> {
        Some(self.render_error.clone())
    }

    fn set_buffering(&mut self, plan: BufferPlan) -> Result<(), PlayerError> {
        self.mpv.set_property("cache-pause-wait", plan.stall_secs)?;
        self.stall_wait = plan.stall_secs;
        self.pending_stall_wait = None;
        self.start_target = (plan.start_secs > 0.0).then_some(plan.start_secs);
        if self.start_target.is_some() {
            self.mpv.set_property("pause", true)?;
        }
        Ok(())
    }

    fn buffer_status(&self) -> Option<BufferStatus> {
        let state = self.pump();
        Some(BufferStatus {
            waiting: self.start_target.is_some() || state.paused_for_cache,
            ahead_secs: state.ahead,
            target_secs: self.start_target.unwrap_or(self.stall_wait),
        })
    }

    fn start_now(&mut self) -> Result<(), PlayerError> {
        if self.start_target.take().is_some() {
            return self.set_async("pause", false);
        }
        if self.pending_stall_wait.is_none() {
            self.pending_stall_wait = Some(self.stall_wait);
        }
        self.set_async("cache-pause-wait", START_NOW_WAIT)
    }
}
