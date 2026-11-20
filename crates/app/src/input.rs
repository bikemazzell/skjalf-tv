//! Input mapping: raw egui input events → semantic [`Action`]s.
//!
//! Views interpret actions differently (10-foot model): the same `Up` moves
//! the cursor in Live and zaps a channel in Player. Gamepad events (M10) map
//! onto the same actions.

use egui::{Event, Key};

/// A semantic user intent, view-agnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Up,
    Down,
    Left,
    Right,
    /// "A" button / Enter — activate the focused control.
    Select,
    /// "B" button / Escape — back, delete.
    Back,
    Digit(u8),
    Favorite,
    Epg,
    /// View button / `o`: the audio and subtitle panel in the player.
    Options,
    /// Explicit prev/next channel (PageUp/PageDown, L1/R1).
    ChUp,
    ChDown,
    Quit,
}

/// Map one egui input event to an action, if any.
///
/// Digits and letters arrive as `Event::Text` (egui has no digit `Key`s).
pub fn event_to_action(event: &Event) -> Option<Action> {
    match event {
        Event::Key {
            key, pressed: true, ..
        } => key_action(*key),
        Event::Text(text) => text_action(text),
        _ => None,
    }
}

fn key_action(key: Key) -> Option<Action> {
    match key {
        Key::ArrowUp => Some(Action::Up),
        Key::ArrowDown => Some(Action::Down),
        Key::ArrowLeft => Some(Action::Left),
        Key::ArrowRight => Some(Action::Right),
        Key::Enter => Some(Action::Select),
        Key::Escape | Key::Backspace | Key::BrowserBack => Some(Action::Back),
        Key::PageUp => Some(Action::ChUp),
        Key::PageDown => Some(Action::ChDown),
        _ => None,
    }
}

fn text_action(text: &str) -> Option<Action> {
    let mut chars = text.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None; // single characters only
    }
    match c {
        '0'..='9' => Some(Action::Digit(c as u8 - b'0')),
        'f' | 'F' => Some(Action::Favorite),
        'e' | 'E' => Some(Action::Epg),
        'o' | 'O' => Some(Action::Options),
        'q' | 'Q' => Some(Action::Quit),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(k: Key) -> Event {
        Event::Key {
            key: k,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }
    }

    #[test]
    fn key_releases_are_ignored() {
        assert_eq!(
            event_to_action(&Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: Default::default(),
            }),
            None
        );
    }

    #[test]
    fn arrows_and_buttons() {
        assert_eq!(event_to_action(&key(Key::ArrowUp)), Some(Action::Up));
        assert_eq!(event_to_action(&key(Key::ArrowLeft)), Some(Action::Left));
        assert_eq!(event_to_action(&key(Key::Enter)), Some(Action::Select));
        assert_eq!(event_to_action(&key(Key::Escape)), Some(Action::Back));
        assert_eq!(event_to_action(&key(Key::BrowserBack)), Some(Action::Back));
        assert_eq!(event_to_action(&key(Key::PageUp)), Some(Action::ChUp));
        assert_eq!(event_to_action(&key(Key::Tab)), None);
    }

    #[test]
    fn digits_and_letters_from_text_events() {
        assert_eq!(
            event_to_action(&Event::Text("5".into())),
            Some(Action::Digit(5))
        );
        assert_eq!(
            event_to_action(&Event::Text("0".into())),
            Some(Action::Digit(0))
        );
        assert_eq!(
            event_to_action(&Event::Text("f".into())),
            Some(Action::Favorite)
        );
        assert_eq!(event_to_action(&Event::Text("E".into())), Some(Action::Epg));
        assert_eq!(
            event_to_action(&Event::Text("o".into())),
            Some(Action::Options)
        );
        assert_eq!(
            event_to_action(&Event::Text("q".into())),
            Some(Action::Quit)
        );
        assert_eq!(event_to_action(&Event::Text("x".into())), None);
        assert_eq!(event_to_action(&Event::Text("42".into())), None);
        assert_eq!(event_to_action(&Event::Text("".into())), None);
    }

    #[test]
    fn other_events_ignored() {
        assert_eq!(event_to_action(&Event::Paste("x".into())), None);
    }
}
