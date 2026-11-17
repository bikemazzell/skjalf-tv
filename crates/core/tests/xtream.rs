//! Xtream Codes API client tests against a mock `HttpFetcher`.

use chrono::NaiveDate;
use skjalf_core::model::{Activation, Source, SourceKind, StreamFormat};
use skjalf_core::xtream;

/// Routes URLs to canned bodies.
struct MockFetch {
    routes: Vec<(String, String)>,
}

impl MockFetch {
    fn new(routes: &[(&str, &str)]) -> Self {
        Self {
            routes: routes
                .iter()
                .map(|(u, b)| (u.to_string(), b.to_string()))
                .collect(),
        }
    }
}

impl skjalf_core::xtream::HttpFetcher for MockFetch {
    fn get(&self, url: &str) -> Result<String, skjalf_core::xtream::FetchError> {
        self.routes
            .iter()
            .find(|(u, _)| u == url)
            .map(|(_, b)| b.clone())
            .ok_or_else(|| skjalf_core::xtream::FetchError(format!("no route for {url}")))
    }
}

fn src() -> Source {
    Source {
        id: "s1".into(),
        name: "XC".into(),
        kind: SourceKind::Xtream {
            host: "http://xc.tv:8080".into(),
            username: "u1".into(),
            password: "p1".into(),
        },
        stream_format: StreamFormat::Ts,
    }
}

fn user_info(status: &str, exp: &str) -> String {
    format!(
        r#"{{"user_info":{{"status":"{status}","exp_date":"{exp}"}},"server_info":{{"url":"xc.tv"}}}}"#
    )
}

#[test]
fn login_active_with_future_expiry() {
    let f = MockFetch::new(&[(
        "http://xc.tv:8080/player_api.php?username=u1&password=p1",
        &user_info("Active", "4102444800"), // 2100-01-01
    )]);
    assert_eq!(xtream::login(&f, &src()).unwrap(), Activation::Active);
}

#[test]
fn login_active_but_date_passed_is_expired() {
    let f = MockFetch::new(&[(
        "http://xc.tv:8080/player_api.php?username=u1&password=p1",
        &user_info("Active", "1762300800"), // 2025-11-05
    )]);
    match xtream::login(&f, &src()).unwrap() {
        Activation::Expired { date } => {
            assert_eq!(date, NaiveDate::from_ymd_opt(2025, 11, 5).unwrap())
        }
        other => panic!("expected Expired, got {other:?}"),
    }
}

#[test]
fn login_status_mapping() {
    let f = MockFetch::new(&[(
        "http://xc.tv:8080/player_api.php?username=u1&password=p1",
        &user_info("Expired", "1735689600"),
    )]);
    assert!(matches!(
        xtream::login(&f, &src()).unwrap(),
        Activation::Expired { .. }
    ));

    let f = MockFetch::new(&[(
        "http://xc.tv:8080/player_api.php?username=u1&password=p1",
        &user_info("Banned", "4102444800"),
    )]);
    assert_eq!(xtream::login(&f, &src()).unwrap(), Activation::Banned);

    let f = MockFetch::new(&[(
        "http://xc.tv:8080/player_api.php?username=u1&password=p1",
        &user_info("Disabled", "4102444800"),
    )]);
    assert_eq!(xtream::login(&f, &src()).unwrap(), Activation::Invalid);

    let f = MockFetch::new(&[(
        "http://xc.tv:8080/player_api.php?username=u1&password=p1",
        &user_info("Whatever", "4102444800"),
    )]);
    assert_eq!(xtream::login(&f, &src()).unwrap(), Activation::Invalid);
}

#[test]
fn login_missing_exp_date_stays_active() {
    let f = MockFetch::new(&[(
        "http://xc.tv:8080/player_api.php?username=u1&password=p1",
        r#"{"user_info":{"status":"Active"}}"#,
    )]);
    assert_eq!(xtream::login(&f, &src()).unwrap(), Activation::Active);
}

#[test]
fn playlist_builds_groups_channels_and_templated_urls() {
    let f = MockFetch::new(&[
        (
            "http://xc.tv:8080/player_api.php?username=u1&password=p1&action=get_live_categories",
            r#"[{"category_id":"10","category_name":"News"},{"category_id":"20","category_name":"Sports"}]"#,
        ),
        (
            "http://xc.tv:8080/player_api.php?username=u1&password=p1&action=get_live_streams",
            r#"[
                {"num":1,"name":"CNN","stream_id":101,"stream_icon":"http://l/cnn.png","category_id":10},
                {"num":0,"name":"NoNum","stream_id":102,"category_id":"20"},
                {"name":"NoNum2","stream_id":103,"category_id":20}
            ]"#,
        ),
    ]);
    let pl = xtream::playlist(&f, &src()).unwrap();

    assert_eq!(pl.groups.len(), 2);
    assert_eq!(pl.groups[0].name, "News");
    assert_eq!(pl.groups[1].id, "20");
    assert_eq!(pl.channels.len(), 3);

    let cnn = pl.find_by_num(1).unwrap();
    assert_eq!(cnn.id, "101");
    assert_eq!(cnn.group_id, "10");
    assert_eq!(cnn.logo.as_deref(), Some("http://l/cnn.png"));
    assert_eq!(cnn.url, "http://xc.tv:8080/live/{user}/{pass}/101.ts");

    let no_num = pl.find("102").unwrap();
    assert_eq!(no_num.num, None, "num 0 is treated as absent");
    let no_num2 = pl.find("103").unwrap();
    assert_eq!(no_num2.num, None, "missing num is absent");
    assert_eq!(no_num2.group_id, "20", "string category_id accepted");
}

#[test]
fn playlist_m3u8_format() {
    let f = MockFetch::new(&[
        (
            "http://xc.tv:8080/player_api.php?username=u1&password=p1&action=get_live_categories",
            r#"[{"category_id":"1","category_name":"G"}]"#,
        ),
        (
            "http://xc.tv:8080/player_api.php?username=u1&password=p1&action=get_live_streams",
            r#"[{"num":1,"name":"A","stream_id":9,"category_id":1}]"#,
        ),
    ]);
    let mut source = src();
    source.stream_format = StreamFormat::M3u8;
    let pl = xtream::playlist(&f, &source).unwrap();
    assert_eq!(
        pl.channels[0].url,
        "http://xc.tv:8080/live/{user}/{pass}/9.m3u8"
    );
}

#[test]
fn orphan_streams_have_group_and_unique_identity() {
    let f = MockFetch::new(&[
        (
            "http://xc.tv:8080/player_api.php?username=u1&password=p1&action=get_live_categories",
            r#"[{"category_id":"10","category_name":"News"}]"#,
        ),
        (
            "http://xc.tv:8080/player_api.php?username=u1&password=p1&action=get_live_streams",
            r#"[{"stream_id":1,"name":"A","category_id":null},{"stream_id":1,"name":"B","category_id":"missing"}]"#,
        ),
    ]);
    let pl = xtream::playlist(&f, &src()).unwrap();
    assert_eq!(pl.channels.len(), 2);
    assert_ne!(pl.channels[0].id, pl.channels[1].id);
    assert!(pl.groups.iter().any(|g| g.name == "Ungrouped"));
    assert!(pl
        .channels
        .iter()
        .all(|ch| pl.groups.iter().any(|g| g.id == ch.group_id)));
}

#[test]
fn short_epg_decodes_base64_and_parses_times() {
    use base64::Engine;
    let title = base64::engine::general_purpose::STANDARD.encode("News Night");
    let desc = base64::engine::general_purpose::STANDARD.encode("Today's headlines");
    let f = MockFetch::new(&[(
        "http://xc.tv:8080/player_api.php?username=u1&password=p1&action=get_short_epg&stream_id=101&limit=2",
        &format!(
            r#"{{"epg_listings":[
                {{"title":"{title}","description":"{desc}","start":"2026-11-15 20:00:00","end":"2026-11-15 21:00:00"}},
                {{"title":"","description":"","start":"2026-11-15 21:00:00","end":"2026-11-15 22:00:00"}}
            ]}}"#
        ),
    )]);
    let epg = xtream::short_epg(&f, &src(), "101", 2).unwrap();
    assert_eq!(epg.len(), 2);
    assert_eq!(epg[0].title, "News Night");
    assert_eq!(epg[0].description, "Today's headlines");
    assert_eq!(epg[0].start.to_string(), "2026-11-15 20:00:00");
    assert_eq!(epg[0].end.to_string(), "2026-11-15 21:00:00");
}

#[test]
fn network_error_propagates() {
    let f = MockFetch::new(&[]);
    assert!(xtream::login(&f, &src()).is_err());
    assert!(xtream::playlist(&f, &src()).is_err());
    assert!(xtream::short_epg(&f, &src(), "1", 1).is_err());
}

#[test]
fn playlist_rejects_m3u_source() {
    let f = MockFetch::new(&[]);
    let mut source = src();
    source.kind = SourceKind::M3u {
        url: "http://x/y.m3u".into(),
    };
    assert!(xtream::playlist(&f, &source).is_err());
}
