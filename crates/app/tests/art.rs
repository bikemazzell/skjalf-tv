//! Artwork loading: decode and shrink, download once, disk cache, failures.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use skjalf::art::{decode_thumbnail, ArtLoader, ArtState, MAX_SIDE};
use skjalf_core::xtream::{FetchError, HttpFetcher};

static SEQ: AtomicU32 = AtomicU32::new(0);

fn cache_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "skjalf-art-loader-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::remove_dir_all(&dir).ok();
    dir
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(width, height, image::Rgba([200, 30, 30, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

/// Serves one PNG for every URL, or fails, and counts downloads.
struct Images {
    body: Option<Vec<u8>>,
    downloads: AtomicUsize,
}

impl Images {
    fn new(body: Option<Vec<u8>>) -> Arc<Self> {
        Arc::new(Self {
            body,
            downloads: AtomicUsize::new(0),
        })
    }
}

impl HttpFetcher for Images {
    fn get(&self, _url: &str) -> Result<String, FetchError> {
        Err(FetchError("text not served".into()))
    }
    fn get_bytes(&self, _url: &str, _max: usize) -> Result<Vec<u8>, FetchError> {
        self.downloads.fetch_add(1, Ordering::SeqCst);
        self.body
            .clone()
            .ok_or_else(|| FetchError("offline".into()))
    }
}

/// Poll until `key` leaves the loading state (5 s budget).
fn settle(loader: &mut ArtLoader, ctx: &egui::Context, key: &str) -> ArtState {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        loader.poll(ctx);
        let state = loader.state(key);
        if state != ArtState::Loading || Instant::now() > deadline {
            return state;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

const POSTER: &str = "https://img.example/poster.png";

#[test]
fn thumbnails_shrink_to_the_max_side_and_keep_the_aspect() {
    let big = decode_thumbnail(&png(1200, 1800)).unwrap();
    assert_eq!(big.size, [400, MAX_SIDE as usize]);
    let small = decode_thumbnail(&png(30, 40)).unwrap();
    assert_eq!(small.size, [30, 40], "small images keep their size");
    assert!(decode_thumbnail(b"not an image").is_err());
}

#[test]
fn art_downloads_once_and_then_comes_from_memory() {
    let ctx = egui::Context::default();
    let images = Images::new(Some(png(60, 90)));
    let mut loader = ArtLoader::new(images.clone(), cache_dir());
    assert!(
        loader.texture(&ctx, POSTER).is_none(),
        "first call queues the job"
    );
    assert_eq!(settle(&mut loader, &ctx, POSTER), ArtState::Ready);
    for _ in 0..3 {
        let texture = loader.texture(&ctx, POSTER).expect("ready");
        assert_eq!(texture.size(), [60, 90]);
    }
    assert_eq!(images.downloads.load(Ordering::SeqCst), 1);
}

#[test]
fn a_new_session_reads_art_from_the_disk_cache() {
    let ctx = egui::Context::default();
    let dir = cache_dir();
    let mut first = ArtLoader::new(Images::new(Some(png(60, 90))), dir.clone());
    first.texture(&ctx, POSTER);
    assert_eq!(settle(&mut first, &ctx, POSTER), ArtState::Ready);

    let offline = Images::new(None);
    let mut second = ArtLoader::new(offline.clone(), dir);
    second.texture(&ctx, POSTER);
    assert_eq!(settle(&mut second, &ctx, POSTER), ArtState::Ready);
    assert_eq!(offline.downloads.load(Ordering::SeqCst), 0, "no download");
}

#[test]
fn failed_art_is_not_requested_again() {
    let ctx = egui::Context::default();
    let offline = Images::new(None);
    let mut loader = ArtLoader::new(offline.clone(), cache_dir());
    loader.texture(&ctx, POSTER);
    assert_eq!(settle(&mut loader, &ctx, POSTER), ArtState::Failed);
    for _ in 0..3 {
        assert!(loader.texture(&ctx, POSTER).is_none());
    }
    loader.poll(&ctx);
    assert_eq!(offline.downloads.load(Ordering::SeqCst), 1);
}

#[test]
fn captured_frame_keys_load_only_from_the_cache() {
    let ctx = egui::Context::default();
    let images = Images::new(Some(png(60, 90)));
    let mut loader = ArtLoader::new(images.clone(), cache_dir());
    loader.texture(&ctx, "frame://movie/7");
    assert_eq!(
        settle(&mut loader, &ctx, "frame://movie/7"),
        ArtState::Failed
    );
    assert_eq!(
        images.downloads.load(Ordering::SeqCst),
        0,
        "frames are never downloaded"
    );
}

#[test]
fn a_changed_fetcher_serves_later_downloads() {
    let ctx = egui::Context::default();
    let offline = Images::new(None);
    let online = Images::new(Some(png(10, 10)));
    let mut loader = ArtLoader::new(offline.clone(), cache_dir());
    loader.set_fetcher(online.clone());
    loader.texture(&ctx, POSTER);
    assert_eq!(settle(&mut loader, &ctx, POSTER), ArtState::Ready);
    assert_eq!(offline.downloads.load(Ordering::SeqCst), 0);
    assert_eq!(online.downloads.load(Ordering::SeqCst), 1);
}

#[test]
fn dropping_the_loader_stops_its_workers() {
    let images = Images::new(Some(png(10, 10)));
    let loader = ArtLoader::new(images.clone(), cache_dir());
    assert_eq!(
        Arc::strong_count(&images),
        2,
        "the loader holds the fetcher"
    );
    drop(loader);
    let deadline = Instant::now() + Duration::from_secs(2);
    while Arc::strong_count(&images) > 1 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        Arc::strong_count(&images),
        1,
        "workers exited and released it"
    );
}
