//! Captured frames for movies and series without art. A capture runs only on
//! a details screen, while nothing plays, after the account reports a free
//! connection. Leaving the screen or pressing Play cancels it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread::JoinHandle;

use skjalf_core::nav::View;
use skjalf_core::xtream;

use super::movies::DetailInfo;
use super::App;
use crate::art::ArtState;

pub struct FrameJob {
    key: String,
    view: View,
    cancel: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    result: Receiver<Result<Vec<u8>, String>>,
}

impl App {
    pub fn art_is_cached(&self, key: &str) -> bool {
        self.art.is_cached(key)
    }

    pub fn frame_capture_idle(&self) -> bool {
        self.frame_job.is_none()
    }

    /// Advance the capture; call once per frame.
    pub(super) fn tick_frame_capture(&mut self) {
        if let Some(job) = &mut self.frame_job {
            if self.router.current() != job.view {
                return self.cancel_frame_capture();
            }
            match job.result.try_recv() {
                Ok(Ok(bytes)) => {
                    let key = job.key.clone();
                    self.art.insert(&key, &bytes);
                    self.frame_job = None;
                }
                Ok(Err(_)) | Err(TryRecvError::Disconnected) => self.frame_job = None,
                Err(TryRecvError::Empty) => {}
            }
            return;
        }
        let Some(capturer) = self.frame_capturer.clone() else {
            return;
        };
        let Some((key, url)) = self.frame_target() else {
            return;
        };
        if !self.frames_tried.insert(key.clone()) || self.art.is_cached(&key) {
            return;
        }
        let (Some(source), fetcher) = (self.settings.source.clone(), self.fetcher.clone()) else {
            return;
        };
        let url = self.render_template(&url);
        let user_agent = self.settings.effective_user_agent();
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, result) = mpsc::channel();
        let flag = cancel.clone();
        let handle = std::thread::spawn(move || {
            let outcome = match xtream::free_connection(fetcher.as_ref(), &source) {
                Ok(true) => capturer.capture(&url, &user_agent, &flag),
                Ok(false) => Err("No free connection.".into()),
                Err(error) => Err(error.to_string()),
            };
            let _ = tx.send(outcome);
        });
        self.frame_job = Some(FrameJob {
            key,
            view: self.router.current(),
            cancel,
            handle: Some(handle),
            result,
        });
    }

    /// Stop a running capture and wait until its connection is closed.
    pub(super) fn cancel_frame_capture(&mut self) {
        if let Some(mut job) = self.frame_job.take() {
            job.cancel.store(true, Ordering::SeqCst);
            if let Some(handle) = job.handle.take() {
                let _ = handle.join();
            }
        }
    }

    /// The frame key and stream URL for the details screen on view, when its
    /// item has no usable art.
    fn frame_target(&self) -> Option<(String, String)> {
        let source = self.settings.source.as_ref()?;
        let missing =
            |url: Option<&String>| url.is_none_or(|url| self.art.state(url) == ArtState::Failed);
        match self.router.current() {
            View::MovieDetails => {
                let details = self.movie_details.as_ref()?;
                let poster = match &details.info {
                    DetailInfo::Loading => return None,
                    DetailInfo::Ready(info) => info.poster.as_ref(),
                    DetailInfo::Unavailable => None,
                }
                .or(details.movie.icon.as_ref());
                if !missing(poster) {
                    return None;
                }
                let url = xtream::movie_url(source, &details.movie).ok()?;
                Some((format!("frame://movie/{}", details.movie.id), url))
            }
            View::SeriesDetails => {
                let details = self.series_details.as_ref()?;
                if !missing(details.series.cover.as_ref()) {
                    return None;
                }
                let DetailInfo::Ready(info) = &details.info else {
                    return None;
                };
                let episode = info.seasons.first()?.episodes.first()?;
                let url = xtream::episode_url(source, episode).ok()?;
                Some((format!("frame://series/{}", details.series.id), url))
            }
            _ => None,
        }
    }
}
