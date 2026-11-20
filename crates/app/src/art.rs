//! Artwork loading: posters, covers, logos, and captured frames.
//!
//! The UI asks for a texture by key every frame. The first request queues a
//! job; background workers read the disk cache or download the image, then
//! decode and shrink it, so the UI thread only uploads finished pixels. Keys
//! are `http(s)` URLs, or `frame://` keys that only the disk cache serves.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::SystemTime;

use skjalf_core::art_cache::ArtCache;
use skjalf_core::xtream::HttpFetcher;

/// Longest side of a decoded image, in pixels.
pub const MAX_SIDE: u32 = 600;
/// Largest image download.
const MAX_DOWNLOAD: usize = 8 * 1024 * 1024;
/// Disk cache limit.
const CACHE_BYTES: u64 = 300 * 1024 * 1024;
/// Textures kept in memory; the least recently used go first.
const MAX_TEXTURES: usize = 300;
const WORKERS: usize = 2;

pub type Fetcher = Arc<dyn HttpFetcher + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtState {
    Unknown,
    Loading,
    Ready,
    Failed,
}

enum Entry {
    Loading,
    Ready {
        texture: egui::TextureHandle,
        used: u64,
    },
    Failed,
}

/// Jobs, newest first, shared with the workers. `closed` ends them.
#[derive(Default)]
struct Queue {
    jobs: Mutex<VecDeque<String>>,
    ready: Condvar,
    closed: AtomicBool,
}

pub struct ArtLoader {
    entries: HashMap<String, Entry>,
    queue: Arc<Queue>,
    fetcher: Arc<RwLock<Fetcher>>,
    results: Receiver<(String, Result<egui::ColorImage, String>)>,
    cache: Arc<ArtCache>,
    tick: u64,
}

impl ArtLoader {
    pub fn new(fetcher: Fetcher, cache_dir: PathBuf) -> Self {
        let queue = Arc::new(Queue::default());
        let fetcher = Arc::new(RwLock::new(fetcher));
        let (tx, results) = mpsc::channel();
        let cache = Arc::new(ArtCache::new(&cache_dir, CACHE_BYTES));
        for _ in 0..WORKERS {
            let (queue, fetcher, cache, tx) =
                (queue.clone(), fetcher.clone(), cache.clone(), tx.clone());
            std::thread::spawn(move || work(&queue, &fetcher, &cache, &tx));
        }
        Self {
            entries: HashMap::new(),
            queue,
            fetcher,
            results,
            cache,
            tick: 0,
        }
    }

    /// Later downloads use `fetcher` (a changed User-Agent or TLS setting).
    pub fn set_fetcher(&mut self, fetcher: Fetcher) {
        if let Ok(mut slot) = self.fetcher.write() {
            *slot = fetcher;
        }
    }

    /// The texture for `key`, or `None` while it loads or after it failed.
    pub fn texture(&mut self, ctx: &egui::Context, key: &str) -> Option<egui::TextureHandle> {
        self.tick += 1;
        match self.entries.get_mut(key) {
            Some(Entry::Ready { texture, used }) => {
                *used = self.tick;
                return Some(texture.clone());
            }
            Some(Entry::Loading | Entry::Failed) => return None,
            None => {}
        }
        self.entries.insert(key.to_owned(), Entry::Loading);
        if let Ok(mut jobs) = self.queue.jobs.lock() {
            jobs.push_front(key.to_owned());
        }
        self.queue.ready.notify_one();
        self.poll(ctx);
        None
    }

    pub fn state(&self, key: &str) -> ArtState {
        match self.entries.get(key) {
            None => ArtState::Unknown,
            Some(Entry::Loading) => ArtState::Loading,
            Some(Entry::Ready { .. }) => ArtState::Ready,
            Some(Entry::Failed) => ArtState::Failed,
        }
    }

    pub fn is_cached(&self, key: &str) -> bool {
        self.cache.contains(key)
    }

    /// Store bytes under `key` (a captured frame); the next request shows them.
    pub fn insert(&mut self, key: &str, bytes: &[u8]) {
        // A failed write only means the frame is captured again next session.
        let _ = self.cache.put(key, bytes, SystemTime::now());
        self.forget(key);
    }

    /// Forget `key` so the next request loads it again (a new frame capture).
    pub fn forget(&mut self, key: &str) {
        if !matches!(self.entries.get(key), Some(Entry::Loading)) {
            self.entries.remove(key);
        }
    }

    /// Upload finished images; call once per frame.
    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok((key, result)) = self.results.try_recv() {
            let entry = match result {
                Ok(image) => Entry::Ready {
                    texture: ctx.load_texture(&key, image, egui::TextureOptions::LINEAR),
                    used: self.tick,
                },
                Err(_) => Entry::Failed,
            };
            self.entries.insert(key, entry);
            ctx.request_repaint();
        }
        self.evict();
    }

    fn evict(&mut self) {
        let ready = |entry: &Entry| match entry {
            Entry::Ready { used, .. } => Some(*used),
            _ => None,
        };
        let count = self.entries.values().filter_map(ready).count();
        if count <= MAX_TEXTURES {
            return;
        }
        let mut used: Vec<(u64, String)> = self
            .entries
            .iter()
            .filter_map(|(key, entry)| Some((ready(entry)?, key.clone())))
            .collect();
        used.sort();
        for (_, key) in used.into_iter().take(count - MAX_TEXTURES) {
            self.entries.remove(&key);
        }
    }
}

impl Drop for ArtLoader {
    fn drop(&mut self) {
        // Take the lock so no worker sits between its check and its wait.
        let _jobs = self.queue.jobs.lock();
        self.queue.closed.store(true, Ordering::SeqCst);
        self.queue.ready.notify_all();
    }
}

fn work(
    queue: &Queue,
    fetcher: &RwLock<Fetcher>,
    cache: &ArtCache,
    tx: &Sender<(String, Result<egui::ColorImage, String>)>,
) {
    loop {
        let key = {
            let Ok(mut jobs) = queue.jobs.lock() else {
                return;
            };
            loop {
                if queue.closed.load(Ordering::SeqCst) {
                    return;
                }
                if let Some(key) = jobs.pop_front() {
                    break key;
                }
                jobs = match queue.ready.wait(jobs) {
                    Ok(jobs) => jobs,
                    Err(_) => return,
                };
            }
        };
        let result = load(&key, fetcher, cache).and_then(|bytes| decode_thumbnail(&bytes));
        if tx.send((key, result)).is_err() {
            return; // the loader is gone
        }
    }
}

fn load(key: &str, fetcher: &RwLock<Fetcher>, cache: &ArtCache) -> Result<Vec<u8>, String> {
    if let Some(bytes) = cache.get(key, SystemTime::now()) {
        return Ok(bytes);
    }
    if !(key.starts_with("http://") || key.starts_with("https://")) {
        return Err("Not cached.".into());
    }
    let fetcher = fetcher
        .read()
        .map_err(|_| "Fetcher unavailable.".to_owned())?
        .clone();
    let bytes = fetcher
        .get_bytes(key, MAX_DOWNLOAD)
        .map_err(|error| error.0)?;
    // A failed cache write only costs a later download.
    let _ = cache.put(key, &bytes, SystemTime::now());
    Ok(bytes)
}

/// Decode an image and shrink it to at most [`MAX_SIDE`] on its long side.
pub fn decode_thumbnail(bytes: &[u8]) -> Result<egui::ColorImage, String> {
    let image = image::load_from_memory(bytes).map_err(|error| error.to_string())?;
    let image = if image.width().max(image.height()) > MAX_SIDE {
        image.thumbnail(MAX_SIDE, MAX_SIDE)
    } else {
        image
    };
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        size,
        rgba.as_raw(),
    ))
}
