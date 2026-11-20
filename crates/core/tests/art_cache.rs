//! Disk cache for downloaded artwork: naming, round trips, and eviction.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime};

use skjalf_core::art_cache::ArtCache;

static SEQ: AtomicU32 = AtomicU32::new(0);

fn dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "skjalf-art-test-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::remove_dir_all(&dir).ok();
    dir
}

fn at(secs: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000 + secs)
}

#[test]
fn stored_art_reads_back_under_a_hashed_name() {
    let dir = dir();
    let cache = ArtCache::new(&dir, 1024);
    assert_eq!(cache.get("https://img.example/a.jpg", at(0)), None);
    cache
        .put("https://img.example/a.jpg", b"\xff\xd8 jpeg", at(1))
        .unwrap();
    cache
        .put("https://img.example/b.jpg", b"other", at(2))
        .unwrap();
    assert_eq!(
        cache.get("https://img.example/a.jpg", at(3)).as_deref(),
        Some(&b"\xff\xd8 jpeg"[..])
    );
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names.len(), 2, "one file per URL, no temp files left");
    for name in names {
        let (stem, ext) = name.split_once('.').unwrap();
        assert_eq!((stem.len(), ext), (16, "img"), "{name}");
        assert!(stem.chars().all(|c| c.is_ascii_hexdigit()));
    }
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn the_least_recently_used_art_goes_first() {
    let dir = dir();
    let cache = ArtCache::new(&dir, 100);
    cache.put("a", &[1; 40], at(1)).unwrap();
    cache.put("b", &[2; 40], at(2)).unwrap();
    assert!(cache.get("a", at(3)).is_some(), "reading a makes it recent");
    cache.put("c", &[3; 40], at(4)).unwrap();
    assert!(
        cache.get("b", at(5)).is_none(),
        "b was the least recently used"
    );
    assert!(cache.get("a", at(6)).is_some());
    assert!(cache.get("c", at(7)).is_some());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn art_larger_than_the_cache_is_not_stored() {
    let dir = dir();
    let cache = ArtCache::new(&dir, 100);
    cache.put("keep", &[1; 40], at(1)).unwrap();
    cache.put("huge", &[2; 200], at(2)).unwrap();
    assert!(cache.get("huge", at(3)).is_none());
    assert!(
        cache.get("keep", at(4)).is_some(),
        "nothing else is evicted for it"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[cfg(unix)]
#[test]
fn the_cache_is_private_to_the_user() {
    use std::os::unix::fs::PermissionsExt;
    let dir = dir();
    let cache = ArtCache::new(&dir, 1024);
    cache
        .put("https://img.example/a.jpg?token=x", b"x", at(1))
        .unwrap();
    assert_eq!(
        std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let file = std::fs::read_dir(&dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(
        std::fs::metadata(file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    std::fs::remove_dir_all(dir).ok();
}
