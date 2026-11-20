//! Disk cache for downloaded artwork (posters, covers, logos, captured
//! frames). Files are named by a hash of the URL; past the size limit, the
//! least recently used files go first. Times are passed in for testability.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::store::{fnv_hex, restrict, sibling_temp};

pub struct ArtCache {
    dir: PathBuf,
    max_bytes: u64,
}

impl ArtCache {
    pub fn new(dir: &Path, max_bytes: u64) -> Self {
        Self {
            dir: dir.to_owned(),
            max_bytes,
        }
    }

    fn path(&self, url: &str) -> PathBuf {
        self.dir.join(format!("{}.img", fnv_hex(url)))
    }

    pub fn contains(&self, url: &str) -> bool {
        self.path(url).is_file()
    }

    /// Cached bytes for `url`; a hit counts as a use at `now`.
    pub fn get(&self, url: &str, now: SystemTime) -> Option<Vec<u8>> {
        let path = self.path(url);
        let bytes = std::fs::read(&path).ok()?;
        if let Ok(file) = std::fs::File::options().write(true).open(&path) {
            let _ = file.set_modified(now);
        }
        Some(bytes)
    }

    /// Store `bytes` for `url`, then trim the cache to its limit. Art larger
    /// than the whole cache is not stored.
    pub fn put(&self, url: &str, bytes: &[u8], now: SystemTime) -> std::io::Result<()> {
        if bytes.len() as u64 > self.max_bytes {
            return Ok(());
        }
        std::fs::create_dir_all(&self.dir)?;
        restrict(&self.dir, true);
        let path = self.path(url);
        let tmp = sibling_temp(&path);
        std::fs::write(&tmp, bytes)?;
        restrict(&tmp, false);
        std::fs::File::options()
            .write(true)
            .open(&tmp)?
            .set_modified(now)?;
        std::fs::rename(&tmp, &path)?;
        self.evict()
    }

    fn evict(&self) -> std::io::Result<()> {
        let mut files: Vec<(SystemTime, u64, PathBuf)> = std::fs::read_dir(&self.dir)?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "img"))
            .filter_map(|entry| {
                let meta = entry.metadata().ok()?;
                Some((meta.modified().ok()?, meta.len(), entry.path()))
            })
            .collect();
        files.sort();
        let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
        for (_, len, path) in files {
            if total <= self.max_bytes {
                break;
            }
            std::fs::remove_file(path)?;
            total -= len;
        }
        Ok(())
    }
}
