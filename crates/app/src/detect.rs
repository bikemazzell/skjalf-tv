//! Steam Deck Game Mode (gamescope session) detection.
//!
//! Pure: the environment lookup is injected so tests need no process env.

/// True when running inside a Steam gamescope session (Steam Deck Game Mode,
/// desktop gamescope, or nested `gamescope -f --`).
pub fn game_mode(get: impl Fn(&str) -> Option<String>) -> bool {
    if let Some(v) = get("SteamOS").as_deref() {
        if v == "1" {
            return true;
        }
    }
    if let Some(v) = get("SteamGamepadUI").as_deref() {
        if v == "1" {
            return true;
        }
    }
    if let Some(v) = get("XDG_CURRENT_DESKTOP").as_deref() {
        if v.eq_ignore_ascii_case("gamescope") {
            return true;
        }
    }
    matches!(&get("GAMESCOPE_WAYLAND_DISPLAY"), Some(v) if !v.is_empty())
}

/// The real environment, for `main`.
pub fn from_env() -> bool {
    game_mode(|k| std::env::var(k).ok())
}

#[cfg(test)]
mod tests {
    use super::game_mode as gm;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn detects_steamos() {
        let e = env(&[("SteamOS", "1")]);
        assert!(gm(|k| e.get(k).cloned()));
    }

    #[test]
    fn detects_gamepad_ui() {
        let e = env(&[("SteamGamepadUI", "1")]);
        assert!(gm(|k| e.get(k).cloned()));
    }

    #[test]
    fn detects_gamescope_desktop() {
        let e = env(&[("XDG_CURRENT_DESKTOP", "gamescope")]);
        assert!(gm(|k| e.get(k).cloned()));
    }

    #[test]
    fn detects_gamescope_wayland_display() {
        let e = env(&[("GAMESCOPE_WAYLAND_DISPLAY", "gamescope-0")]);
        assert!(gm(|k| e.get(k).cloned()));
    }

    #[test]
    fn desktop_is_not_game_mode() {
        let e = env(&[("XDG_CURRENT_DESKTOP", "GNOME"), ("SteamOS", "0")]);
        assert!(!gm(|k| e.get(k).cloned()));
    }
}
