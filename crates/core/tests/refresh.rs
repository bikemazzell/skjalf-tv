//! Refresh interval choices and due-time rules.

use skjalf_core::model::RefreshInterval;

const HOUR: u64 = 3600;

#[test]
fn default_is_24_hours_and_choices_cycle() {
    assert_eq!(RefreshInterval::default(), RefreshInterval::Hours24);
    let mut seen = vec![RefreshInterval::Off];
    let mut value = RefreshInterval::Off.next();
    while value != RefreshInterval::Off {
        seen.push(value);
        value = value.next();
    }
    assert_eq!(seen, RefreshInterval::ALL);
    assert_eq!(RefreshInterval::Off.prev(), RefreshInterval::Days7);
    assert_eq!(RefreshInterval::Hours6.prev(), RefreshInterval::Off);
    let labels: Vec<_> = RefreshInterval::ALL.iter().map(|v| v.label()).collect();
    assert_eq!(
        labels,
        ["Off", "6 hours", "12 hours", "24 hours", "48 hours", "7 days"]
    );
}

#[test]
fn due_only_when_older_than_the_interval() {
    let interval = RefreshInterval::Hours12;
    let now = 1_000_000;
    assert!(!interval.is_due(Some(now - 12 * HOUR + 1), now));
    assert!(interval.is_due(Some(now - 12 * HOUR), now));
    assert!(interval.is_due(None, now), "never refreshed");
    assert!(!interval.is_due(Some(now + HOUR), now), "clock moved back");
    assert!(!RefreshInterval::Off.is_due(None, now));
    assert!(!RefreshInterval::Off.is_due(Some(0), now));
    assert_eq!(RefreshInterval::Days7.seconds(), Some(7 * 24 * HOUR));
}
