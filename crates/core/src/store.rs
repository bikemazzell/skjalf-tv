//! Local persistence: `settings.toml`, `favorites.json`, `cache/<hash>.json`.
//!
//! Reads are forgiving: a missing or corrupt file yields defaults/`None`
//! instead of failing the app (fast-forward to a fresh state).
//! Writes are atomic (temp file + rename in the same directory) and
//! owner-only: settings may hold provider credentials.

use crate::catalog::{MovieCatalog, SeriesCatalog};
use crate::model::{Favorites, Playlist, Settings, SourceKind};
use crate::progress::Progress;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
#[error("store io: {0}")]
pub struct StoreError(#[from] pub std::io::Error);

#[derive(Clone)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// Open (and create) the app data directory.
    pub fn open(dir: &Path) -> Result<Self, StoreError> {
        std::fs::create_dir_all(dir.join("cache"))?;
        restrict(dir, true);
        Ok(Self {
            dir: dir.to_owned(),
        })
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn read(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.path(name)).ok()
    }

    /// Atomic write: a sibling temp file renamed over the target, so a crash
    /// mid-write can never truncate an existing file.
    fn write(&self, name: &str, body: &str) -> Result<(), StoreError> {
        let path = self.path(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = sibling_temp(&path);
        std::fs::write(&tmp, body)?;
        restrict(&tmp, false);
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    // -- settings ------------------------------------------------------------

    pub fn load_settings(&self) -> Result<Settings, StoreError> {
        Ok(self
            .read("settings.toml")
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default())
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<(), StoreError> {
        let body = toml::to_string(settings).map_err(io_err)?;
        self.write("settings.toml", &body)
    }

    // -- favorites -----------------------------------------------------------

    pub fn load_favorites(&self) -> Result<Favorites, StoreError> {
        Ok(self
            .read("favorites.json")
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default())
    }

    pub fn save_favorites(&self, favorites: &Favorites) -> Result<(), StoreError> {
        let body = serde_json::to_string_pretty(favorites).map_err(io_err)?;
        self.write("favorites.json", &body)
    }

    // -- playback progress ---------------------------------------------------

    /// A missing or corrupt file loads as empty.
    pub fn load_progress(&self) -> Progress {
        self.read("progress.json")
            .and_then(|body| serde_json::from_str(&body).ok())
            .unwrap_or_default()
    }

    pub fn save_progress(&self, progress: &Progress) -> Result<(), StoreError> {
        let body = serde_json::to_string(progress).map_err(io_err)?;
        self.write("progress.json", &body)
    }

    // -- playlist cache --------------------------------------------------------

    pub fn load_playlist(&self, source_id: &str) -> Option<Playlist> {
        let path = self.dir.join("cache").join(cache_name(source_id));
        let body = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&body).ok()
    }

    pub fn save_playlist(&self, playlist: &Playlist) -> Result<(), StoreError> {
        // Clear the separate Xtream user/password fields. Source IDs, M3U
        // URLs, channel URLs, and names may still contain private data, so
        // cache files must remain owner-only.
        let mut cached = playlist.clone();
        if let SourceKind::Xtream {
            username, password, ..
        } = &mut cached.source.kind
        {
            username.clear();
            password.clear();
        }
        let body = serde_json::to_string_pretty(&cached).map_err(io_err)?;
        self.write(&format!("cache/{}", cache_name(&playlist.source.id)), &body)
    }
}

impl Store {
    // -- movie catalog cache -------------------------------------------------

    pub fn load_movies(&self, source_id: &str) -> Option<MovieCatalog> {
        let path = self
            .dir
            .join("cache")
            .join(cache_name_for(source_id, "movies"));
        let mut catalog: MovieCatalog = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
        catalog.source_id = source_id.to_owned();
        Some(catalog)
    }

    /// Compact JSON; catalogs hold no credentials or stream URLs.
    pub fn save_movies(&self, catalog: &MovieCatalog) -> Result<(), StoreError> {
        let body = serde_json::to_string(catalog).map_err(io_err)?;
        self.write(
            &format!("cache/{}", cache_name_for(&catalog.source_id, "movies")),
            &body,
        )
    }
}

impl Store {
    pub fn load_series(&self, source_id: &str) -> Option<SeriesCatalog> {
        let path = self
            .dir
            .join("cache")
            .join(cache_name_for(source_id, "series"));
        let mut catalog: SeriesCatalog = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
        catalog.source_id = source_id.to_owned();
        Some(catalog)
    }

    pub fn save_series(&self, catalog: &SeriesCatalog) -> Result<(), StoreError> {
        let body = serde_json::to_string(catalog).map_err(io_err)?;
        self.write(
            &format!("cache/{}", cache_name_for(&catalog.source_id, "series")),
            &body,
        )
    }
}

fn cache_name_for(source_id: &str, kind: &str) -> String {
    let name = cache_name(source_id);
    let stem = name.strip_suffix(".json").unwrap_or(&name);
    format!("{stem}-{kind}.json")
}

/// Filesystem-safe fixed-length cache key for a source id (FNV-1a 64-bit).
/// Raw ids are URLs (slashes, `..`, way past NAME_MAX) and must not become
/// path components.
fn cache_name(source_id: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in source_id.bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}.json")
}

/// `<file>.<ext>` → `<file>.<ext>.tmp` in the same directory (rename target).
/// Copy the data directory of the pre-rename app to `new`. Does nothing when
/// `new` exists or `old` does not. The copy goes through a sibling temporary
/// directory renamed into place, so a failure never leaves a partial `new`.
/// Returns whether a copy happened.
pub fn migrate_data_dir(old: &Path, new: &Path) -> Result<bool, StoreError> {
    if new.symlink_metadata().is_ok() || !old.is_dir() {
        return Ok(false);
    }
    let staging = sibling_temp(new);
    let copied = std::fs::remove_dir_all(&staging)
        .or_else(|error| match error.kind() {
            std::io::ErrorKind::NotFound => Ok(()),
            _ => Err(error),
        })
        .and_then(|()| copy_tree(old, &staging, 0, false))
        .and_then(|()| std::fs::rename(&staging, new));
    if let Err(error) = copied {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error.into());
    }
    Ok(true)
}

/// Copy a tree. Failures under the top-level `cache` directory are skipped
/// because the next refresh refetches them; any other failure aborts.
fn copy_tree(from: &Path, to: &Path, depth: usize, lenient: bool) -> std::io::Result<()> {
    std::fs::create_dir(to)?;
    restrict(to, true);
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        let kind = entry.file_type()?;
        let child_lenient = lenient || (depth == 0 && entry.file_name() == "cache");
        let copied = if kind.is_dir() {
            copy_tree(&entry.path(), &target, depth + 1, child_lenient)
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target).map(|_| restrict(&target, false))
        } else {
            Ok(())
        };
        if !lenient && !child_lenient {
            copied?;
        }
    }
    Ok(())
}

fn sibling_temp(path: &Path) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(".tmp");
    PathBuf::from(os)
}

/// Restrict permissions: 0o700 for directories, 0o600 for files (best-effort).
#[cfg(unix)]
fn restrict(path: &Path, dir: bool) {
    use std::os::unix::fs::PermissionsExt;
    let mode = if dir { 0o700 } else { 0o600 };
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn restrict(_path: &Path, _dir: bool) {}

/// serde failures are treated as IO-level failures for callers.
fn io_err<E: std::fmt::Display>(e: E) -> StoreError {
    StoreError(std::io::Error::other(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_name_is_stable_and_hex() {
        assert_eq!(cache_name("s1"), cache_name("s1"));
        assert_ne!(cache_name("s1"), cache_name("s2"));
        let name = cache_name("http://a/b?x=1");
        let (stem, ext) = name.split_once('.').unwrap();
        assert_eq!(ext, "json");
        assert_eq!(stem.len(), 16, "fixed-length hex stem");
        assert!(stem.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
