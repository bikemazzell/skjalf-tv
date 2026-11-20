//! Player engine: `PlayerEngine` trait + libmpv implementation (rsmpv).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rsmpv::render::OwnedRenderContext;
use rsmpv::{Mpv, Node};
use skjalf_core::tracks::{SubtitlePref, Track, TrackKind};

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
    /// Vertical subtitle position in percent of the screen; 100 is the bottom.
    fn set_subtitle_position(&mut self, _percent: u8) -> Result<(), PlayerError> {
        Ok(())
    }
    /// The User-Agent for stream requests from the next load on.
    fn set_user_agent(&mut self, _user_agent: &str) -> Result<(), PlayerError> {
        Ok(())
    }
    /// Audio and subtitle tracks of the current file (empty when idle).
    fn tracks(&self) -> Vec<Track> {
        Vec::new()
    }
    /// Switch a track of the current file; `None` turns subtitles off.
    fn select_track(&mut self, _kind: TrackKind, _id: Option<i64>) -> Result<(), PlayerError> {
        Ok(())
    }
    /// Remembered languages that pick the tracks of every later file. Call
    /// before `load_at`: it also undoes manual switches in the last file.
    fn set_track_prefs(
        &mut self,
        _audio_lang: Option<&str>,
        _subtitles: &SubtitlePref,
    ) -> Result<(), PlayerError> {
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
    /// After a track switch, the short wait lasts until playback passes
    /// this position.
    track_wait_after: Option<f64>,
}

/// Forward buffer limit when the cache lives on disk. In memory, mpv keeps
/// its own default of 150 MiB.
const DISK_CACHE_LIMIT: &str = "4GiB";
/// The wait that "start now" sets: play what is already buffered.
const START_NOW_WAIT: f64 = 0.5;
/// How far playback runs past a track switch before the stall wait returns.
const TRACK_SWITCH_RUN: f64 = 1.0;
/// mpv's own stall wait, used until a buffering plan arrives.
const DEFAULT_STALL_WAIT: f64 = 1.0;

const OBSERVED: [(&str, rsmpv::Format); 8] = [
    ("time-pos", rsmpv::Format::Double),
    ("duration", rsmpv::Format::Double),
    ("demuxer-cache-duration", rsmpv::Format::Double),
    ("demuxer-cache-idle", rsmpv::Format::Flag),
    ("paused-for-cache", rsmpv::Format::Flag),
    ("core-idle", rsmpv::Format::Flag),
    ("eof-reached", rsmpv::Format::Flag),
    ("track-list", rsmpv::Format::Node),
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
    /// A seek is in flight; `eof` may still describe the old position.
    seeking: bool,
    tracks: Vec<Track>,
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
            seeking: false,
            tracks: Vec::new(),
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
            rsmpv::Event::PlaybackRestart => self.seeking = false,
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
        if self.eof && !self.ended_reported && !self.seeking {
            self.ended_reported = true;
            self.events.push(PlayerEvent::Ended);
        }
    }

    /// Playback runs past `position` and does not wait for the cache.
    fn played_past(&self, position: f64) -> bool {
        !self.paused_for_cache && self.position.is_some_and(|p| p > position)
    }

    /// Show a requested switch before mpv reports the new track list.
    fn mark_selected(&mut self, kind: TrackKind, id: Option<i64>) {
        for track in self.tracks.iter_mut().filter(|t| t.kind == kind) {
            track.selected = Some(track.id) == id;
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
            "eof-reached" => {
                self.eof = flag.unwrap_or(false);
                self.seeking = false;
            }
            "track-list" => {
                self.tracks = match &data {
                    rsmpv::PropertyData::Node(list) => parse_track_list(list),
                    _ => Vec::new(),
                }
            }
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
        user_agent: &str,
        disk_cache: Option<&Path>,
    ) -> Result<Self, PlayerError> {
        let mut builder = Self::base_builder(ignore_ssl)?
            .set_property("user-agent", user_agent)?
            .set_property("vo", "libmpv")?
            .set_property("volume", initial_volume.min(100) as i64)?;
        if let Some(dir) = disk_cache.and_then(Path::to_str) {
            builder = builder
                .set_property("cache-on-disk", true)?
                .set_property("demuxer-cache-dir", dir)?
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
            track_wait_after: None,
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
            state.cache_idle || state.ahead.is_some_and(|ahead| ahead >= target)
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
        if self.track_wait_after.is_some() {
            return Ok(()); // the track switch wait restores it
        }
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

impl MpvEngine {
    /// Restore the stall wait once playback runs past a track switch.
    fn apply_track_wait(&mut self) -> Result<(), PlayerError> {
        let Some(after) = self.track_wait_after else {
            return Ok(());
        };
        if self.pump().played_past(after) {
            self.track_wait_after = None;
            self.pending_stall_wait = None;
            self.set_async("cache-pause-wait", self.stall_wait)?;
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
        self.track_wait_after = None;
        self.mpv.command(&["loadfile", url, "replace"])?;
        Ok(())
    }

    fn seek_relative(&mut self, secs: f64) -> Result<(), PlayerError> {
        {
            let mut state = self.pump();
            state.seeking = true;
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
        self.track_wait_after = None;
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
            .and_then(|()| self.apply_track_wait())
            .and_then(|()| self.apply_stall_wait())
        {
            events.push(PlayerEvent::Failed(error.0));
        }
        events.append(&mut self.pump().events);
        if events
            .iter()
            .any(|event| matches!(event, PlayerEvent::Failed(_)))
        {
            self.start_target = None;
            self.pending_stall_wait = None;
        }
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
        self.track_wait_after = None;
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
            target_secs: self
                .start_target
                .unwrap_or(if self.track_wait_after.is_some() {
                    START_NOW_WAIT
                } else {
                    self.stall_wait
                }),
        })
    }

    fn set_user_agent(&mut self, user_agent: &str) -> Result<(), PlayerError> {
        self.mpv.set_property("user-agent", user_agent)?;
        Ok(())
    }

    fn set_subtitle_position(&mut self, percent: u8) -> Result<(), PlayerError> {
        self.set_async("sub-pos", i64::from(percent))
    }

    fn tracks(&self) -> Vec<Track> {
        self.pump().tracks.clone()
    }

    fn select_track(&mut self, kind: TrackKind, id: Option<i64>) -> Result<(), PlayerError> {
        let property = match kind {
            TrackKind::Audio => "aid",
            TrackKind::Sub => "sid",
        };
        match id {
            Some(id) => self.set_async(property, id)?,
            None => self.set_async(property, "no")?,
        }
        // A switch makes mpv read the stream again from the current
        // position; play the refilled buffer at once, not after the full
        // stall wait.
        let position = {
            let mut state = self.pump();
            state.mark_selected(kind, id);
            state.position
        };
        if let Some(position) = position {
            self.set_async("cache-pause-wait", START_NOW_WAIT)?;
            self.track_wait_after = Some(position + TRACK_SWITCH_RUN);
        }
        Ok(())
    }

    fn set_track_prefs(
        &mut self,
        audio_lang: Option<&str>,
        subtitles: &SubtitlePref,
    ) -> Result<(), PlayerError> {
        // On without a language shows any subtitle track. With a language,
        // a missing language shows none rather than another language.
        let (slang, sid, fallback) = match subtitles {
            SubtitlePref::Auto => ("", "auto", "yes"),
            SubtitlePref::Off => ("", "no", "default"),
            SubtitlePref::Lang(code) => (code.as_str(), "auto", "no"),
        };
        self.mpv
            .set_property("alang", audio_lang.unwrap_or_default())?;
        self.mpv.set_property("slang", slang)?;
        self.mpv.set_property("subs-fallback", fallback)?;
        self.mpv.set_property("aid", "auto")?;
        self.mpv.set_property("sid", sid)?;
        Ok(())
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

/// Captures one frame of a stream, for items without art.
pub trait FrameCapturer: Send + Sync {
    /// One JPEG frame from inside `url`. Returns early when `cancel` is set.
    fn capture(&self, url: &str, user_agent: &str, cancel: &AtomicBool) -> Result<Vec<u8>, String>;
}

/// A separate headless mpv that writes one frame with `vo=image`, 10% into
/// the file, then stops. It opens its own connection to the provider.
pub struct MpvFrameCapturer {
    ignore_ssl: bool,
    dir: PathBuf,
}

/// Longest wait for the frame before the capture gives up.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(45);

impl MpvFrameCapturer {
    pub fn new(ignore_ssl: bool, dir: PathBuf) -> Self {
        Self { ignore_ssl, dir }
    }
}

impl FrameCapturer for MpvFrameCapturer {
    fn capture(&self, url: &str, user_agent: &str, cancel: &AtomicBool) -> Result<Vec<u8>, String> {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let out = self.dir.join(format!(
            "capture-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        let result = capture_into(&out, url, user_agent, self.ignore_ssl, cancel);
        let _ = std::fs::remove_dir_all(&out);
        if self.dir.read_dir().is_ok_and(|mut d| d.next().is_none()) {
            let _ = std::fs::remove_dir(&self.dir);
        }
        result
    }
}

fn capture_into(
    out: &Path,
    url: &str,
    user_agent: &str,
    ignore_ssl: bool,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
    let error = |e: rsmpv::Error| e.to_string();
    let mpv = Mpv::builder()
        .and_then(|b| b.set_property("vo", "image"))
        .and_then(|b| b.set_property("vo-image-format", "jpg"))
        .and_then(|b| b.set_property("vo-image-outdir", out.to_string_lossy().as_ref()))
        .and_then(|b| b.set_property("ao", "null"))
        .and_then(|b| b.set_property("aid", "no"))
        .and_then(|b| b.set_property("sid", "no"))
        .and_then(|b| b.set_property("frames", 1_i64))
        .and_then(|b| b.set_property("start", "10%"))
        .and_then(|b| b.set_property("user-agent", user_agent))
        .and_then(|b| b.set_property("tls-verify", !ignore_ssl))
        .and_then(|b| b.build())
        .map_err(error)?;
    mpv.command(&["loadfile", url]).map_err(error)?;
    let deadline = std::time::Instant::now() + CAPTURE_TIMEOUT;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err("Capture cancelled.".into());
        }
        if std::time::Instant::now() > deadline {
            return Err("Capture timed out.".into());
        }
        match mpv.poll_event() {
            Some(rsmpv::Event::EndFile { .. }) => break,
            Some(_) => continue,
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    let frame = std::fs::read_dir(out)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "jpg"))
        .ok_or_else(|| "No frame was written.".to_owned())?;
    std::fs::read(frame).map_err(|e| e.to_string())
}

/// Audio/subtitle entries of mpv's `track-list` (video and malformed
/// entries skipped; the undetermined language `und` counts as none).
pub fn parse_track_list(list: &Node) -> Vec<Track> {
    let Node::Array(items) = list else {
        return Vec::new();
    };
    items.iter().filter_map(parse_track).collect()
}

fn parse_track(item: &Node) -> Option<Track> {
    let Node::Map(fields) = item else {
        return None;
    };
    let get = |key: &str| fields.iter().find(|(k, _)| k == key).map(|(_, v)| v);
    let text = |key: &str| match get(key) {
        Some(Node::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    };
    let flag = |key: &str| matches!(get(key), Some(Node::Flag(true)));
    let kind = match text("type")?.as_str() {
        "audio" => TrackKind::Audio,
        "sub" => TrackKind::Sub,
        _ => return None,
    };
    let Some(Node::Int64(id)) = get("id") else {
        return None;
    };
    let channels = match get("demux-channel-count") {
        Some(Node::Int64(n)) => u32::try_from(*n).ok(),
        _ => None,
    };
    Some(Track {
        id: *id,
        kind,
        lang: text("lang").filter(|l| l != "und"),
        title: text("title"),
        codec: text("codec"),
        channels,
        selected: flag("selected"),
        forced: flag("forced"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsmpv::{Event, PropertyData};

    fn entry(fields: &[(&str, Node)]) -> Node {
        Node::Map(
            fields
                .iter()
                .map(|(k, v)| ((*k).to_owned(), v.clone()))
                .collect(),
        )
    }

    fn s(v: &str) -> Node {
        Node::String(v.into())
    }

    fn audio(id: i64, lang: &str, selected: bool) -> Node {
        entry(&[
            ("id", Node::Int64(id)),
            ("type", s("audio")),
            ("lang", s(lang)),
            ("selected", Node::Flag(selected)),
        ])
    }

    fn track_list_change(list: Node) -> Event {
        Event::PropertyChange {
            userdata: 0,
            name: "track-list".into(),
            data: PropertyData::Node(list),
        }
    }

    #[test]
    fn parses_audio_and_subtitle_tracks_and_skips_video() {
        let list = Node::Array(vec![
            entry(&[("id", Node::Int64(1)), ("type", s("video"))]),
            entry(&[
                ("id", Node::Int64(1)),
                ("type", s("audio")),
                ("lang", s("eng")),
                ("title", s("Main")),
                ("codec", s("ac3")),
                ("demux-channel-count", Node::Int64(6)),
                ("selected", Node::Flag(true)),
            ]),
            audio(2, "und", false),
            entry(&[
                ("id", Node::Int64(1)),
                ("type", s("sub")),
                ("lang", s("ger")),
                ("forced", Node::Flag(true)),
            ]),
        ]);
        let tracks = parse_track_list(&list);
        assert_eq!(
            tracks,
            vec![
                Track {
                    id: 1,
                    kind: TrackKind::Audio,
                    lang: Some("eng".into()),
                    title: Some("Main".into()),
                    codec: Some("ac3".into()),
                    channels: Some(6),
                    selected: true,
                    forced: false,
                },
                Track {
                    id: 2,
                    kind: TrackKind::Audio,
                    lang: None, // "und" = undetermined
                    title: None,
                    codec: None,
                    channels: None,
                    selected: false,
                    forced: false,
                },
                Track {
                    id: 1,
                    kind: TrackKind::Sub,
                    lang: Some("ger".into()),
                    title: None,
                    codec: None,
                    channels: None,
                    selected: false,
                    forced: true,
                },
            ]
        );
    }

    #[test]
    fn malformed_track_list_yields_nothing_usable() {
        assert!(parse_track_list(&Node::None).is_empty());
        let list = Node::Array(vec![s("garbage"), entry(&[("type", s("audio"))])]);
        assert!(parse_track_list(&list).is_empty());
    }

    #[test]
    fn observed_tracks_follow_the_current_file_only() {
        let mut state = Observed::default();
        state.apply(track_list_change(Node::Array(vec![audio(1, "eng", true)])));
        assert!(
            state.tracks.is_empty(),
            "changes before StartFile describe the old file"
        );

        state.apply(Event::StartFile {
            playlist_entry_id: 1,
        });
        state.apply(track_list_change(Node::Array(vec![
            audio(1, "eng", true),
            audio(2, "ger", false),
        ])));
        assert_eq!(state.tracks.len(), 2);

        state.reset_for_load();
        assert!(state.tracks.is_empty(), "a new load forgets the old tracks");
    }

    #[test]
    fn marking_a_selection_updates_observed_tracks_at_once() {
        let mut state = Observed::default();
        state.apply(Event::StartFile {
            playlist_entry_id: 1,
        });
        state.apply(track_list_change(Node::Array(vec![
            audio(1, "eng", true),
            audio(2, "ger", false),
        ])));
        state.mark_selected(TrackKind::Audio, Some(2));
        let selected: Vec<i64> = state
            .tracks
            .iter()
            .filter(|t| t.selected)
            .map(|t| t.id)
            .collect();
        assert_eq!(selected, vec![2]);
    }

    #[test]
    fn the_short_track_switch_wait_ends_once_playback_runs_past_the_switch() {
        let mut state = Observed {
            position: Some(30.0),
            ..Observed::default()
        };
        assert!(!state.played_past(31.0), "not yet past the switch point");
        state.position = Some(31.5);
        state.paused_for_cache = true;
        assert!(!state.played_past(31.0), "still refilling");
        state.paused_for_cache = false;
        assert!(state.played_past(31.0));
        state.position = None;
        assert!(!state.played_past(31.0), "no position, no decision");
    }
}
