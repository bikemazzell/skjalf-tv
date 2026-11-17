//! M3U playlist parsing (`#EXTM3U` + `#EXTINF` attribute form).

use skjalf_core::model::{Group, Playlist, Source, SourceKind, StreamFormat};

fn parse(text: &str) -> Playlist {
    let source = Source {
        id: "s".into(),
        name: "list".into(),
        kind: SourceKind::M3u {
            url: "http://x/y.m3u".into(),
        },
        stream_format: StreamFormat::Ts,
    };
    skjalf_core::m3u::parse(source, text).expect("parse should succeed")
}

#[test]
fn parses_fixture_attributes() {
    let text = include_str!("fixtures/sample.m3u");
    let pl = parse(text);

    assert_eq!(
        pl.groups,
        vec![
            Group {
                id: "News".into(),
                name: "News".into()
            },
            Group {
                id: "Sports".into(),
                name: "Sports".into()
            },
            Group {
                id: "Ungrouped".into(),
                name: "Ungrouped".into()
            },
        ]
    );
    assert_eq!(pl.channels.len(), 4);

    let cnn = pl.find("cnn.us").expect("tvg-id becomes channel id");
    assert_eq!(cnn.name, "CNN International");
    assert_eq!(cnn.num, Some(1));
    assert_eq!(cnn.logo.as_deref(), Some("http://l/cnn.png"));
    assert_eq!(cnn.group_id, "News");
    assert_eq!(cnn.url, "http://srv.example/live/{user}/{pass}/1.ts");
}

#[test]
fn bare_entries_get_ungrouped_and_sequential_numbers() {
    let pl = parse(include_str!("fixtures/sample.m3u"));
    let sky = pl.channels.iter().find(|c| c.name == "Sky Sports").unwrap();
    assert_eq!(sky.group_id, "Sports");
    assert!(
        sky.num.is_some(),
        "unnumbered channel can be reached by number"
    );

    let bare = pl
        .channels
        .iter()
        .find(|c| c.name == "Bare Channel")
        .unwrap();
    assert_eq!(bare.group_id, "Ungrouped");
    assert_eq!(bare.logo, None);
    assert!(!bare.id.is_empty(), "synthetic id assigned");

    // The Ungrouped group exists so ungrouped channels are reachable in Home.
    let ungrouped_ids: Vec<_> = pl.channels_in_group("Ungrouped").collect();
    assert_eq!(ungrouped_ids.len(), 1);
    assert_eq!(ungrouped_ids[0].name, "Bare Channel");
}

#[test]
fn generated_numbers_skip_all_explicit_numbers() {
    let pl = parse("#EXTM3U\n#EXTINF:-1,A\nhttp://a\n#EXTINF:-1 tvg-chno=\"1\",B\nhttp://b\n#EXTINF:-1,C\nhttp://c\n#EXTINF:-1 tvg-chno=\"3\",D\nhttp://d\n");
    assert_eq!(
        pl.channels.iter().map(|ch| ch.num).collect::<Vec<_>>(),
        [Some(2), Some(1), Some(4), Some(3)]
    );
}

#[test]
fn whitespace_and_extra_attrs_tolerated() {
    let text = "#EXTM3U\n\
                #EXTINF:-1 tvg-logo=\"l.png\" extra-attr=\"zz\" group-title=\" G \" ,Padded Name \n\
                http://a/b.ts\n";
    let pl = parse(text);
    assert_eq!(pl.channels[0].name, "Padded Name");
    assert_eq!(pl.channels[0].group_id, "G");
    assert_eq!(pl.channels[0].logo.as_deref(), Some("l.png"));
}

#[test]
fn missing_url_line_is_skipped() {
    let text = "#EXTM3U\n#EXTINF:-1,Orphan\n#EXTINF:-1,Ok\nhttp://a/ok.ts\n";
    let pl = parse(text);
    assert_eq!(pl.channels.len(), 1);
    assert_eq!(pl.channels[0].name, "Ok");
}

#[test]
fn windows_line_endings() {
    let text = "#EXTM3U\r\n#EXTINF:-1 group-title=\"N\",A\r\nhttp://a\r\n";
    let pl = parse(text);
    assert_eq!(pl.channels[0].name, "A");
    assert_eq!(pl.channels[0].url, "http://a");
}

#[test]
fn duplicate_names_get_unique_ids() {
    let text = "#EXTM3U\n#EXTINF:-1,X\nhttp://a\n#EXTINF:-1,X\nhttp://b\n";
    let pl = parse(text);
    assert_ne!(pl.channels[0].id, pl.channels[1].id);
}

#[test]
fn duplicate_tvg_ids_are_uniquified() {
    let text = "#EXTM3U\n\
                #EXTINF:-1 tvg-id=\"dup\",A\nhttp://a/1\n\
                #EXTINF:-1 tvg-id=\"dup\",B\nhttp://a/2\n\
                #EXTINF:-1 tvg-id=\"dup\",C\nhttp://a/3\n";
    let pl = parse(text);
    let ids: Vec<&str> = pl.channels.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids.len(), 3);
    assert_eq!(
        ids.iter().filter(|&&id| id == "dup").count(),
        1,
        "first keeps the id"
    );
    let unique: std::collections::HashSet<&str> = ids.iter().copied().collect();
    assert_eq!(unique.len(), 3, "all ids distinct: {ids:?}");
}

#[test]
fn non_ascii_outside_quotes_does_not_panic() {
    let text = "#EXTM3U\n\
                #EXTINF:ü tvg-chno=\"7\" group-title=\"Nöws\",Café TV\n\
                http://a/ü.ts\n";
    let pl = parse(text);
    assert_eq!(pl.channels.len(), 1);
    assert_eq!(pl.channels[0].num, Some(7));
    assert_eq!(pl.channels[0].group_id, "Nöws");
    assert_eq!(pl.channels[0].name, "Café TV");
    assert_eq!(pl.channels[0].url, "http://a/ü.ts");
}

#[test]
fn not_m3u_is_rejected() {
    let source = Source {
        id: "s".into(),
        name: "l".into(),
        kind: SourceKind::M3u { url: "u".into() },
        stream_format: StreamFormat::Ts,
    };
    assert!(skjalf_core::m3u::parse(source, "<html>not a playlist</html>").is_err());
}
