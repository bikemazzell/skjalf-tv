//! Settings/favorites/playlist persistence: round-trips + corrupt recovery.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use skjalf_core::browse::SortMode;
use skjalf_core::catalog::{ContentKey, ContentKind, Movie, MovieCatalog};
use skjalf_core::model::{
    Favorites, Playlist, RefreshInterval, RefreshTimes, Settings, SortModes, Source, SourceKind,
    StreamFormat,
};
use skjalf_core::store::Store;

static SEQ: AtomicU32 = AtomicU32::new(0);

fn tempdir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "skjalf-store-test-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    dir
}

fn pl() -> Playlist {
    Playlist {
        source: Source {
            id: "s1".into(),
            name: "XC".into(),
            kind: SourceKind::Xtream {
                host: "http://x.tv:8080".into(),
                username: "u".into(),
                password: "p".into(),
            },
            stream_format: StreamFormat::Ts,
        },
        groups: vec![skjalf_core::model::Group {
            id: "1".into(),
            name: "News".into(),
        }],
        channels: vec![skjalf_core::model::Channel {
            id: "101".into(),
            name: "CNN".into(),
            num: Some(1),
            logo: None,
            group_id: "1".into(),
            url: "http://x.tv:8080/live/{user}/{pass}/101.ts".into(),
        }],
    }
}

#[test]
fn settings_roundtrip() {
    let dir = tempdir();
    let store = Store::open(&dir).unwrap();
    let mut s = Settings {
        volume: 37,
        last_channel: Some(("s1".into(), "101".into())),
        ignore_ssl: true,
        source: None,
        sort_modes: SortModes {
            live: SortMode::Name,
            movies: SortMode::Name,
            series: SortMode::Name,
        },
        refresh_interval: RefreshInterval::Days7,
        last_refresh: [(
            "s1".to_owned(),
            RefreshTimes {
                live: Some(1_700_000_000),
                movies: None,
                series: Some(5),
                movies_opened: true,
                series_opened: false,
            },
        )]
        .into(),
        start_buffer_secs: 600,
        stall_buffer_secs: 30,
    };
    store.save_settings(&s).unwrap();
    assert!(
        dir.join("settings.toml").exists(),
        "hand-editable TOML file"
    );
    s.volume = 100;
    store.save_settings(&s).unwrap();
    assert_eq!(store.load_settings().unwrap(), s);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn settings_without_sort_modes_load_as_provider_order() {
    let dir = tempdir();
    let store = Store::open(&dir).unwrap();
    std::fs::write(
        dir.join("settings.toml"),
        "volume = 60\nignore_ssl = false\n",
    )
    .unwrap();
    let loaded = store.load_settings().unwrap();
    assert_eq!(loaded.sort_modes.live, SortMode::Provider);
    assert_eq!(loaded.refresh_interval, RefreshInterval::Hours24);
    assert!(loaded.last_refresh.is_empty());
    assert_eq!(loaded.volume, 60);
    assert_eq!(loaded.start_buffer_secs, 0);
    assert_eq!(loaded.stall_buffer_secs, 1);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn missing_settings_is_default() {
    let dir = tempdir();
    let store = Store::open(&dir).unwrap();
    assert_eq!(store.load_settings().unwrap(), Settings::default());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn corrupt_settings_recovers_to_default() {
    let dir = tempdir();
    let store = Store::open(&dir).unwrap();
    std::fs::write(dir.join("settings.toml"), "not [ valid toml {{{{").unwrap();
    assert_eq!(store.load_settings().unwrap(), Settings::default());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn favorites_roundtrip_and_corrupt_recovery() {
    let dir = tempdir();
    let store = Store::open(&dir).unwrap();
    let mut f = Favorites::default();
    f.toggle("s1", "101");
    f.toggle("s1", "102");
    store.save_favorites(&f).unwrap();
    assert_eq!(store.load_favorites().unwrap(), f);

    std::fs::write(dir.join("favorites.json"), "{broken").unwrap();
    assert_eq!(store.load_favorites().unwrap(), Favorites::default());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn group_favorites_roundtrip_beside_item_favorites() {
    let dir = tempdir();
    let store = Store::open(&dir).unwrap();
    let group_key = ContentKey::group_favorite_key(ContentKind::Live, "g1");
    let mut f = Favorites::default();
    f.toggle("s1", "101");
    f.toggle("s1", &group_key);
    store.save_favorites(&f).unwrap();
    let loaded = store.load_favorites().unwrap();
    assert_eq!(loaded, f);
    assert!(loaded.contains("s1", &group_key));
    assert!(loaded.contains("s1", "101"));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn playlist_cache_roundtrip_and_corrupt_recovery() {
    let dir = tempdir();
    let store = Store::open(&dir).unwrap();
    store.save_playlist(&pl()).unwrap();
    let loaded = store.load_playlist("s1").unwrap();
    assert_eq!(loaded.channels, pl().channels, "channels cached verbatim");
    assert_eq!(loaded.groups, pl().groups);
    match loaded.source.kind {
        SourceKind::Xtream {
            host,
            username,
            password,
        } => {
            assert_eq!(host, "http://x.tv:8080", "non-secret config kept");
            assert!(username.is_empty(), "username not persisted");
            assert!(password.is_empty(), "password not persisted");
        }
        _ => panic!("source kind preserved"),
    }

    // Corrupt the (single) cache file; reads then yield None.
    let cache = dir.join("cache");
    let entry = std::fs::read_dir(&cache).unwrap().next().unwrap().unwrap();
    std::fs::write(entry.path(), "]]not json").unwrap();
    assert_eq!(
        store.load_playlist("s1"),
        None,
        "corrupt cache reads as absent"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn cache_filename_is_hashed_and_flat() {
    let dir = tempdir();
    let store = Store::open(&dir).unwrap();
    let mut playlist = pl();
    playlist.source.id = "../../evil very long source id with spaces and ://".into();
    store.save_playlist(&playlist).unwrap();

    let cache = dir.join("cache");
    let names: Vec<String> = std::fs::read_dir(&cache)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 1, "one flat cache file");
    let (stem, ext) = names[0].split_once('.').unwrap();
    assert_eq!(ext, "json");
    assert_eq!(stem.len(), 16, "fixed-length hex stem, got {stem}");
    assert_eq!(
        store.load_playlist(&playlist.source.id).unwrap().channels,
        playlist.channels,
        "round-trips through the hashed name"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[cfg(unix)]
#[test]
fn saved_files_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir();
    let store = Store::open(&dir).unwrap();
    store.save_settings(&Settings::default()).unwrap();
    store.save_favorites(&Favorites::default()).unwrap();
    store.save_playlist(&pl()).unwrap();

    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode();
    assert_eq!(mode(&dir) & 0o777, 0o700, "store dir is private");
    assert_eq!(mode(&dir.join("settings.toml")) & 0o777, 0o600);
    assert_eq!(mode(&dir.join("favorites.json")) & 0o777, 0o600);
    let cache_file = std::fs::read_dir(dir.join("cache"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(mode(&cache_file) & 0o777, 0o600);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn saves_leave_no_temp_files() {
    let dir = tempdir();
    let store = Store::open(&dir).unwrap();
    store.save_settings(&Settings::default()).unwrap();
    store.save_favorites(&Favorites::default()).unwrap();
    let all: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        all.iter().all(|n| !n.ends_with(".tmp")),
        "atomic writes clean up, found {all:?}"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn open_creates_missing_dir() {
    let dir = tempdir().join("nested/deeper");
    let store = Store::open(&dir).unwrap();
    store.save_settings(&Settings::default()).unwrap();
    assert!(dir.join("settings.toml").exists());
    std::fs::remove_dir_all(dir.parent().unwrap()).ok();
}

fn movie_catalog() -> MovieCatalog {
    MovieCatalog {
        source_id: "xtream:http://host/u".into(),
        groups: vec![skjalf_core::model::Group {
            id: "12".into(),
            name: "Action".into(),
        }],
        movies: vec![Movie {
            id: "1".into(),
            name: "Example \u{1D3F}".into(),
            group_id: "12".into(),
            icon: None,
            added: Some(1_700_000_000),
            rating: Some("7".into()),
            extension: "mkv".into(),
        }],
    }
}

#[test]
fn movie_catalog_roundtrips_compactly_and_privately() {
    let dir = tempdir();
    let store = Store::open(&dir).unwrap();
    let catalog = movie_catalog();
    store.save_movies(&catalog).unwrap();
    assert_eq!(store.load_movies(&catalog.source_id), Some(catalog.clone()));
    assert_eq!(store.load_movies("other"), None);
    let files: Vec<_> = std::fs::read_dir(dir.join("cache"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(files.len(), 1);
    let name = files[0].file_name().unwrap().to_str().unwrap();
    assert!(name.ends_with("-movies.json"), "{name}");
    let body = std::fs::read_to_string(&files[0]).unwrap();
    assert!(!body.contains('\n'), "compact JSON");
    assert!(!body.contains("http"), "no URLs");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&files[0]).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn corrupt_movie_cache_is_none() {
    let dir = tempdir();
    let store = Store::open(&dir).unwrap();
    let catalog = movie_catalog();
    store.save_movies(&catalog).unwrap();
    let path = std::fs::read_dir(dir.join("cache"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::write(path, "{broken").unwrap();
    assert_eq!(store.load_movies(&catalog.source_id), None);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn series_catalog_roundtrips_without_source_identity() {
    use skjalf_core::catalog::{SeriesCatalog, SeriesItem};
    let dir = tempdir();
    let store = Store::open(&dir).unwrap();
    let catalog = SeriesCatalog {
        source_id: "xtream:http://host/u".into(),
        groups: vec![],
        series: vec![SeriesItem {
            id: "9".into(),
            name: "Show".into(),
            group_id: "1".into(),
            cover: None,
            last_modified: Some(5),
            rating: None,
        }],
    };
    store.save_series(&catalog).unwrap();
    assert_eq!(store.load_series(&catalog.source_id), Some(catalog.clone()));
    let name = std::fs::read_dir(dir.join("cache"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert!(name.to_str().unwrap().ends_with("-series.json"));
    assert!(!std::fs::read_to_string(name).unwrap().contains("http"));
    std::fs::remove_dir_all(dir).ok();
}
