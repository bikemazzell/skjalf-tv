//! Movie catalog and detail parsing against synthetic provider shapes.

use std::cell::RefCell;

use skjalf_core::catalog::{ContentKey, ContentKind};
use skjalf_core::model::{Source, SourceKind, StreamFormat};
use skjalf_core::xtream::{self, FetchError, FetchLimits, HttpFetcher};

struct Routes {
    routes: Vec<(&'static str, String)>,
    limits: RefCell<Vec<(String, FetchLimits)>>,
}

impl Routes {
    fn new(routes: Vec<(&'static str, &str)>) -> Self {
        Self {
            routes: routes.into_iter().map(|(a, b)| (a, b.to_owned())).collect(),
            limits: RefCell::new(Vec::new()),
        }
    }
}

impl HttpFetcher for Routes {
    fn get(&self, url: &str) -> Result<String, FetchError> {
        self.get_with(url, FetchLimits::DEFAULT)
    }

    fn get_with(&self, url: &str, limits: FetchLimits) -> Result<String, FetchError> {
        let action = url.split("action=").nth(1).unwrap_or("");
        self.limits.borrow_mut().push((action.to_owned(), limits));
        self.routes
            .iter()
            .find(|(route, _)| action.starts_with(route))
            .map(|(_, body)| body.clone())
            .ok_or_else(|| FetchError("no route".into()))
    }
}

fn source() -> Source {
    Source {
        id: "xtream:test".into(),
        name: "t".into(),
        kind: SourceKind::Xtream {
            host: "http://tv.example.com:8080/".into(),
            username: "us er".into(),
            password: "p/w".into(),
        },
        stream_format: StreamFormat::Ts,
    }
}

const CATEGORIES: &str = r#"[{"category_id":"12","category_name":"Action","parent_id":0},{"category_id":13,"category_name":"Drama"}]"#;

#[test]
fn movie_catalog_parses_mixed_shapes() {
    let streams = r#"[
        {"num":1,"name":"Alpha","stream_type":"movie","stream_id":1001,"stream_icon":"http://img/a.jpg","rating":"7.2","rating_5based":3.6,"tmdb":"55","added":"1700000000","category_id":"12","category_ids":[12],"container_extension":"mkv","custom_sid":null},
        {"name":"Beta","stream_id":"1002","stream_icon":"","rating":"","added":1700000001,"category_id":13,"container_extension":null,"tmdb":55},
        {"name":"Gamma","stream_id":1003,"rating":8,"category_id":"999","container_extension":"mp4"},
        {"name":"Delta","stream_id":1001,"category_id":"12"},
        {"name":"No id"},
        {"name":"Null category","stream_id":1004,"category_id":null}
    ]"#;
    let f = Routes::new(vec![
        ("get_vod_categories", CATEGORIES),
        ("get_vod_streams", streams),
    ]);
    let catalog = xtream::movie_catalog(&f, &source()).unwrap();
    assert_eq!(catalog.source_id, "xtream:test");
    let ids: Vec<_> = catalog.movies.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["1001", "1002", "1003", "1001#2", "1004"]);
    let alpha = &catalog.movies[0];
    assert_eq!(alpha.rating.as_deref(), Some("7.2"));
    assert_eq!(alpha.added, Some(1_700_000_000));
    assert_eq!(alpha.extension, "mkv");
    assert_eq!(alpha.icon.as_deref(), Some("http://img/a.jpg"));
    let beta = &catalog.movies[1];
    assert_eq!(beta.rating, None, "empty rating is missing");
    assert_eq!(beta.icon, None);
    assert_eq!(beta.extension, "mp4", "missing extension defaults");
    assert_eq!(beta.group_id, "13");
    assert_eq!(catalog.movies[2].rating.as_deref(), Some("8"));
    let ungrouped = catalog.groups.last().unwrap();
    assert_eq!(ungrouped.name, "Ungrouped");
    assert_eq!(catalog.movies[2].group_id, ungrouped.id);
    assert_eq!(catalog.movies[4].group_id, ungrouped.id);
    assert_eq!(catalog.groups.len(), 3);
}

#[test]
fn catalog_requests_use_the_catalog_limits() {
    let f = Routes::new(vec![
        ("get_vod_categories", "[]"),
        ("get_vod_streams", "[]"),
    ]);
    xtream::movie_catalog(&f, &source()).unwrap();
    let limits = f.limits.borrow();
    assert_eq!(limits[0].1, FetchLimits::DEFAULT);
    assert_eq!(limits[1].1, FetchLimits::CATALOG);
    assert_eq!(FetchLimits::CATALOG.max_bytes, 256 * 1024 * 1024);
    assert_eq!(FetchLimits::DETAILS.max_bytes, 8 * 1024 * 1024);
}

#[test]
fn malformed_catalog_reports_a_safe_error() {
    let f = Routes::new(vec![
        ("get_vod_categories", CATEGORIES),
        ("get_vod_streams", "{not json"),
    ]);
    let error = xtream::movie_catalog(&f, &source())
        .unwrap_err()
        .to_string();
    assert!(error.contains("get_vod_streams"), "{error}");
    assert!(!error.contains("p/w") && !error.contains("us er"));
}

#[test]
fn movie_info_reads_metadata_and_falls_back() {
    let body = r#"{"info":{"duration_secs":"5400","duration":"01:30:00","description":"","plot":"A plot.","genre":"Drama","director":"D. Rector","actors":"A, B","releasedate":"2020-01-01","rating":7.5,"backdrop_path":["x"],"bitrate":0},"movie_data":{"stream_id":1001,"container_extension":"mkv"}}"#;
    let f = Routes::new(vec![("get_vod_info", body)]);
    let info = xtream::movie_info(&f, &source(), "1001#2").unwrap();
    assert_eq!(info.description.as_deref(), Some("A plot."));
    assert_eq!(info.genre.as_deref(), Some("Drama"));
    assert_eq!(info.cast.as_deref(), Some("A, B"));
    assert_eq!(info.duration_secs, Some(5400));
    assert_eq!(info.rating.as_deref(), Some("7.5"));
    assert_eq!(info.release_date.as_deref(), Some("2020-01-01"));
    assert_eq!(f.limits.borrow()[0].1, FetchLimits::DETAILS);
}

#[test]
fn movie_info_tolerates_empty_array_and_missing_info() {
    for body in [
        r#"{"info":[],"movie_data":{}}"#,
        r#"{}"#,
        r#"{"info":null}"#,
    ] {
        let f = Routes::new(vec![("get_vod_info", body)]);
        let info = xtream::movie_info(&f, &source(), "1").unwrap();
        assert_eq!(info, Default::default(), "{body}");
    }
}

#[test]
fn movie_url_keeps_placeholders_and_uses_the_provider_id() {
    let f = Routes::new(vec![
        ("get_vod_categories", "[]"),
        (
            "get_vod_streams",
            r#"[{"name":"A","stream_id":7,"container_extension":"mkv"},{"name":"B","stream_id":7}]"#,
        ),
    ]);
    let catalog = xtream::movie_catalog(&f, &source()).unwrap();
    assert_eq!(
        xtream::movie_url(&source(), &catalog.movies[0]).unwrap(),
        "http://tv.example.com:8080/movie/{user}/{pass}/7.mkv"
    );
    assert_eq!(catalog.movies[1].id, "7#2");
    assert_eq!(
        xtream::movie_url(&source(), &catalog.movies[1]).unwrap(),
        "http://tv.example.com:8080/movie/{user}/{pass}/7.mp4"
    );
}

#[test]
fn content_keys_separate_kinds_and_keep_live_keys_plain() {
    assert_eq!(ContentKey::new(ContentKind::Live, "5").favorite_key(), "5");
    assert_eq!(
        ContentKey::new(ContentKind::Movie, "5").favorite_key(),
        "movie:5"
    );
    assert_eq!(
        ContentKey::new(ContentKind::Series, "5").favorite_key(),
        "series:5"
    );
    assert_eq!(
        ContentKey::new(ContentKind::Episode, "5").favorite_key(),
        "episode:5"
    );
    assert_eq!(
        ContentKey::id_of_favorite(ContentKind::Movie, "movie:5"),
        Some("5")
    );
    assert_eq!(ContentKey::id_of_favorite(ContentKind::Movie, "5"), None);
    assert_eq!(
        ContentKey::id_of_favorite(ContentKind::Live, "5"),
        Some("5")
    );
    assert_eq!(
        ContentKey::id_of_favorite(ContentKind::Live, "movie:5"),
        None
    );
}

#[test]
fn group_keys_are_distinct_per_kind_and_never_read_as_items() {
    let live = ContentKey::group_favorite_key(ContentKind::Live, "7");
    let movie = ContentKey::group_favorite_key(ContentKind::Movie, "7");
    let series = ContentKey::group_favorite_key(ContentKind::Series, "7");
    assert_eq!(live, "group:live:7");
    assert_eq!(movie, "group:movie:7");
    assert_eq!(series, "group:series:7");
    for (kind, key) in [
        (ContentKind::Live, &live),
        (ContentKind::Movie, &movie),
        (ContentKind::Series, &series),
    ] {
        assert_eq!(ContentKey::group_id_of_favorite(kind, key), Some("7"));
        assert_eq!(ContentKey::id_of_favorite(kind, key), None);
    }
    assert_eq!(
        ContentKey::group_id_of_favorite(ContentKind::Movie, &live),
        None
    );
    assert_eq!(
        ContentKey::group_id_of_favorite(ContentKind::Live, "7"),
        None
    );
    assert_eq!(
        ContentKey::group_id_of_favorite(ContentKind::Live, "movie:7"),
        None
    );
}

#[test]
fn series_catalog_parses_mixed_shapes() {
    let streams = r#"[
        {"num":1,"name":"Show A","series_id":2001,"cover":"","plot":"","genre":"","last_modified":"1700000000","rating":"8","backdrop_path":null,"category_id":"30","category_ids":[30]},
        {"name":"Show B","series_id":"2002","cover":"http://img/b.jpg","rating":4.5,"backdrop_path":["x"],"category_id":31,"last_modified":1700000001},
        {"name":"Show C","series_id":2001,"category_id":"nope"},
        {"name":"No id"}
    ]"#;
    let f = Routes::new(vec![
        (
            "get_series_categories",
            r#"[{"category_id":"30","category_name":"Drama"},{"category_id":31,"category_name":"Comedy"}]"#,
        ),
        ("get_series", streams),
    ]);
    let catalog = xtream::series_catalog(&f, &source()).unwrap();
    let ids: Vec<_> = catalog.series.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["2001", "2002", "2001#2"]);
    assert_eq!(catalog.series[0].cover, None);
    assert_eq!(catalog.series[0].rating.as_deref(), Some("8"));
    assert_eq!(catalog.series[0].last_modified, Some(1_700_000_000));
    assert_eq!(catalog.series[1].cover.as_deref(), Some("http://img/b.jpg"));
    assert_eq!(catalog.series[1].rating.as_deref(), Some("4.5"));
    assert_eq!(catalog.groups.last().unwrap().name, "Ungrouped");
    assert_eq!(
        catalog.series[2].group_id,
        catalog.groups.last().unwrap().id
    );
    let limits = f.limits.borrow();
    assert_eq!(limits[1].1, FetchLimits::CATALOG);
    assert!(
        limits
            .iter()
            .all(|(action, _)| !action.starts_with("get_series_info")),
        "episode lists are never fetched with the catalog"
    );
}

#[test]
fn series_info_orders_seasons_numerically_and_episodes_by_number() {
    let body = r#"{
        "seasons":[{"season_number":1,"name":"Opening Act","episode_count":"2","overview":""},{"season_number":"10","name":"Finale"}],
        "info":{"name":"Show","plot":"Plot","genre":"Drama","cast":"A, B","rating":"8","releaseDate":"2020-02-02"},
        "episodes":{
            "10":[{"id":"30010","episode_num":1,"title":"Last","container_extension":"mkv","season":10,"info":{"duration_secs":2700,"plot":"P","rating":7}}],
            "2":[{"id":"30021","episode_num":"2","title":"Second","season":2,"info":[]},{"id":"30020","episode_num":1,"title":"","container_extension":"mp4","season":2,"info":{"duration_secs":"1800"}}],
            "1":[{"id":30002,"episode_num":2,"title":"Two","season":1},{"id":"30001","episode_num":1,"title":"One","season":1},{"episode_num":3,"title":"No id"}]
        }
    }"#;
    let f = Routes::new(vec![("get_series_info", body)]);
    let info = xtream::series_info(&f, &source(), "2001#2").unwrap();
    assert_eq!(info.plot.as_deref(), Some("Plot"));
    assert_eq!(info.release_date.as_deref(), Some("2020-02-02"));
    let numbers: Vec<_> = info.seasons.iter().map(|s| s.number).collect();
    assert_eq!(numbers, [1, 2, 10], "season 10 sorts after season 2");
    let names: Vec<_> = info.seasons.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Opening Act", "Season 2", "Finale"]);
    let first: Vec<_> = info.seasons[0]
        .episodes
        .iter()
        .map(|e| e.id.as_str())
        .collect();
    assert_eq!(
        first,
        ["30001", "30002"],
        "episodes sort by number; missing id skipped"
    );
    let second = &info.seasons[1].episodes;
    assert_eq!(second[0].title, "Episode 1", "missing title is numbered");
    assert_eq!(second[0].duration_secs, Some(1800));
    assert_eq!(second[1].duration_secs, None, "episode info [] is empty");
    assert_eq!(second[1].extension, "mp4", "missing extension defaults");
    assert_eq!(info.seasons[2].episodes[0].duration_secs, Some(2700));
    assert_eq!(f.limits.borrow()[0].1, FetchLimits::DETAILS);
}

#[test]
fn series_info_tolerates_empty_episodes_and_missing_seasons() {
    for body in [
        r#"{"seasons":[],"info":[],"episodes":[]}"#,
        r#"{"info":{"plot":"x"}}"#,
        r#"{"seasons":null,"episodes":null}"#,
        r#"[]"#.trim(),
    ] {
        let f = Routes::new(vec![("get_series_info", body)]);
        match xtream::series_info(&f, &source(), "1") {
            Ok(info) => assert!(info.seasons.is_empty(), "{body}"),
            Err(error) => assert_eq!(body, "[]", "{error}"),
        }
    }
}

#[test]
fn episode_url_keeps_placeholders() {
    let f = Routes::new(vec![(
        "get_series_info",
        r#"{"episodes":{"1":[{"id":"30001","episode_num":1,"title":"One","container_extension":"mkv"}]}}"#,
    )]);
    let info = xtream::series_info(&f, &source(), "9").unwrap();
    assert_eq!(
        xtream::episode_url(&source(), &info.seasons[0].episodes[0]).unwrap(),
        "http://tv.example.com:8080/series/{user}/{pass}/30001.mkv"
    );
}

#[test]
fn ratings_format_to_one_decimal_and_hide_when_zero_or_unparsable() {
    use skjalf_core::catalog::format_rating;
    assert_eq!(format_rating(Some("7.296")).as_deref(), Some("7.3"));
    assert_eq!(format_rating(Some("8.987")).as_deref(), Some("9.0"));
    assert_eq!(format_rating(Some("7")).as_deref(), Some("7.0"));
    assert_eq!(format_rating(Some(" 6.04 ")).as_deref(), Some("6.0"));
    for hidden in [
        Some("0"),
        Some("0.0"),
        Some(""),
        Some("abc"),
        Some("-3"),
        Some("NaN"),
        None,
    ] {
        assert_eq!(format_rating(hidden), None, "{hidden:?}");
    }
}

#[test]
fn movie_info_reads_the_poster_and_prefers_the_movie_image() {
    let body = r#"{"info":{"movie_image":"https://img.example/p.jpg","cover_big":"https://img.example/big.jpg"}}"#;
    let f = Routes::new(vec![("get_vod_info", body)]);
    let info = xtream::movie_info(&f, &source(), "7").unwrap();
    assert_eq!(info.poster.as_deref(), Some("https://img.example/p.jpg"));

    let body = r#"{"info":{"movie_image":"","cover_big":"http://img.example/big.jpg"}}"#;
    let f = Routes::new(vec![("get_vod_info", body)]);
    let info = xtream::movie_info(&f, &source(), "7").unwrap();
    assert_eq!(info.poster.as_deref(), Some("http://img.example/big.jpg"));
}

#[test]
fn art_urls_must_be_http_or_https() {
    let body = r#"{"info":{"movie_image":"file:///etc/passwd","cover_big":"javascript:x"}}"#;
    let f = Routes::new(vec![("get_vod_info", body)]);
    assert_eq!(xtream::movie_info(&f, &source(), "7").unwrap().poster, None);
}

#[test]
fn series_info_reads_season_covers_and_episode_images() {
    let body = r#"{
        "seasons":[
            {"season_number":1,"name":"Season 1","cover_big":"https://img.example/s1.jpg","cover":"https://img.example/s1-small.jpg"},
            {"season_number":2,"cover":"","cover_tmdb":"https://img.example/s2.jpg"}
        ],
        "info":{},
        "episodes":{
            "1":[{"id":"11","episode_num":1,"info":{"movie_image":"https://img.example/e11.jpg"}}],
            "2":[{"id":"21","episode_num":1,"info":{}}],
            "3":[{"id":"31","episode_num":1}]
        }
    }"#;
    let f = Routes::new(vec![("get_series_info", body)]);
    let info = xtream::series_info(&f, &source(), "5").unwrap();
    let covers: Vec<Option<&str>> = info.seasons.iter().map(|s| s.cover.as_deref()).collect();
    assert_eq!(
        covers,
        [
            Some("https://img.example/s1.jpg"),
            Some("https://img.example/s2.jpg"),
            None
        ]
    );
    assert_eq!(
        info.seasons[0].episodes[0].image.as_deref(),
        Some("https://img.example/e11.jpg")
    );
    assert_eq!(info.seasons[1].episodes[0].image, None);
}
