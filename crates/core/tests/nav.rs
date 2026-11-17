//! Navigation state machines: router, focus grid, zap buffer, player OSD.

use skjalf_core::nav::{step_channel, OsdKind, PlayerModel, Router, View, ZapBuffer};

// -- Router -----------------------------------------------------------------

#[test]
fn router_back_pops_to_previous_view_and_never_leaves_root() {
    let mut r = Router::new(View::Main);
    assert_eq!(r.current(), View::Main);
    assert_eq!(r.back(), None);
    assert_eq!(r.current(), View::Main);

    r.push(View::Live);
    r.push(View::Player);
    assert_eq!(r.back(), Some(View::Live));
    assert_eq!(r.current(), View::Live);
    assert_eq!(r.back(), Some(View::Main));
    assert_eq!(r.back(), None);
}

#[test]
fn router_reset_clears_the_stack() {
    let mut r = Router::new(View::Setup);
    r.push(View::Settings);
    r.push(View::Setup);
    r.reset(View::Main);
    assert_eq!(r.current(), View::Main);
    assert_eq!(r.back(), None);
}

#[test]
fn only_list_views_repeat_held_directions() {
    assert!(View::Live.repeats_held_direction());
    assert!(View::Movies.repeats_held_direction());
    assert!(View::Series.repeats_held_direction());
    assert!(View::SeriesDetails.repeats_held_direction());
    for view in [
        View::Setup,
        View::Main,
        View::Settings,
        View::Player,
        View::MovieDetails,
    ] {
        assert!(!view.repeats_held_direction(), "{view:?}");
    }
}

// -- ZapBuffer -----------------------------------------------------------------

const T: u64 = 2500;

#[test]
fn zap_buffer_collects_and_expires() {
    let mut z = ZapBuffer::default();
    assert_eq!(z.digits(), "");
    z.push(1, 0);
    z.push(2, 100);
    assert_eq!(z.digits(), "12");
    assert!(!z.is_expired(100 + T - 1));
    assert!(z.is_expired(100 + T));
    assert_eq!(z.take_if_expired(100 + T), Some(12), "expires to number");
    assert_eq!(z.digits(), "", "cleared after resolve");
    assert_eq!(
        z.take_if_expired(100 + T + 9999),
        None,
        "resolve happens once"
    );
}

#[test]
fn zap_buffer_timeout_refreshes_on_new_digit() {
    let mut z = ZapBuffer::default();
    z.push(1, 0);
    z.push(2, 2400); // near the old deadline
    assert!(
        !z.is_expired(2400 + T - 1),
        "second digit restarts the timer"
    );
    assert_eq!(z.digits(), "12");
}

#[test]
fn zap_buffer_caps_at_five_digits() {
    let mut z = ZapBuffer::default();
    for d in [1u8, 2, 3, 4, 5, 6] {
        z.push(d, 0);
    }
    assert_eq!(z.digits(), "12345");
}

#[test]
fn zap_buffer_force_resolves_early() {
    let mut z = ZapBuffer::default();
    z.push(0, 0);
    z.push(1, 10);
    z.push(2, 20);
    assert_eq!(z.force(), Some(12), "leading zero handled");
    assert_eq!(z.digits(), "");
    assert_eq!(ZapBuffer::default().force(), None);
}

// -- PlayerModel / OSD -----------------------------------------------------------

#[test]
fn osd_shows_and_auto_hides() {
    let mut p = PlayerModel::default();
    assert!(!p.osd_visible(0));
    p.show_osd(OsdKind::Zap("10 CNN".into()), 0);
    assert!(p.osd_visible(2999));
    assert!(!p.osd_visible(3000), "TTL is 3 s");
}

#[test]
fn osd_replaced_by_newer_show() {
    let mut p = PlayerModel::default();
    p.show_osd(OsdKind::Zap("10 CNN".into()), 0);
    p.show_osd(OsdKind::Info, 1000);
    assert!(matches!(p.osd().kind, OsdKind::Info));
    assert!(!p.osd_visible(4000), "hide deadline from the latest show");
}

#[test]
fn step_channel_wraps_within_group() {
    let ids = vec!["a".to_string(), "b".into(), "c".into()];
    assert_eq!(step_channel(&ids, "a", false).unwrap(), "c");
    assert_eq!(step_channel(&ids, "a", true).unwrap(), "b");
    assert_eq!(
        step_channel(&ids, "c", true).unwrap(),
        "a",
        "wraps to first"
    );
    assert_eq!(step_channel(&ids, "zz", true), None, "unknown channel");
    assert_eq!(step_channel(&[], "a", true), None);
}
