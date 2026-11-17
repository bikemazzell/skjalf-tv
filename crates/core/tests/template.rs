//! `{user}`/`{pass}` stream URL templating.

use skjalf_core::template::render;

#[test]
fn substitutes_credentials() {
    assert_eq!(
        render("http://h:8080/live/{user}/{pass}/101.ts", "alice", "s3cret"),
        "http://h:8080/live/alice/s3cret/101.ts"
    );
}

#[test]
fn url_without_placeholders_unchanged() {
    assert_eq!(
        render("http://h/live/sky.m3u8", "alice", "s3cret"),
        "http://h/live/sky.m3u8"
    );
}

#[test]
fn special_characters_are_percent_encoded() {
    assert_eq!(
        render("http://h/live/{user}/{pass}/1.ts", "a b/c", "p@ss+word"),
        "http://h/live/a%20b%2Fc/p%40ss%2Bword/1.ts"
    );
}

#[test]
fn repeated_placeholders_all_replaced() {
    assert_eq!(
        render("http://{user}:{pass}@h/{user}/1.ts", "u", "p"),
        "http://u:p@h/u/1.ts"
    );
}

#[test]
fn only_one_placeholder_present() {
    assert_eq!(render("http://h/{pass}/1.ts", "u", "p"), "http://h/p/1.ts");
}

#[test]
fn empty_credentials_substitute_empty() {
    assert_eq!(
        render("http://h/{user}/{pass}/1.ts", "", ""),
        "http://h///1.ts"
    );
}
