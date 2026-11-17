//! Steam Game Mode keyboard request. URI carries no user text or credentials.

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt, Window};
use x11rb::rust_connection::RustConnection;

pub const OPEN_URI: &str = "steam://open/keyboard";
pub const CLOSE_URI: &str = "steam://close/keyboard";

pub trait KeyboardLauncher: Send + Sync {
    fn send(&self, uri: &'static str) -> Receiver<Result<(), String>>;

    /// Observe a native Steam keyboard close after it has visibly opened.
    fn watch_dismissal(&self, _cancel: Arc<AtomicBool>) -> Option<Receiver<()>> {
        None
    }
}

pub struct SteamUriLauncher;

impl KeyboardLauncher for SteamUriLauncher {
    fn send(&self, uri: &'static str) -> Receiver<Result<(), String>> {
        send(uri)
    }

    fn watch_dismissal(&self, cancel: Arc<AtomicBool>) -> Option<Receiver<()>> {
        Some(watch_dismissal(cancel))
    }
}

#[derive(Default)]
struct OverlayPhase {
    saw_keyboard: bool,
}

impl OverlayPhase {
    fn observe(&mut self, state: Option<(u32, u32)>) -> bool {
        match state {
            Some((1, 2)) => self.saw_keyboard = true,
            Some((0, 0)) if self.saw_keyboard => return true,
            _ => {}
        }
        false
    }
}

/// On the tested SteamOS 3.9 Game Mode, SteamUI uses X display :0 and games
/// use another display. SteamUI's window exposes CARDINAL32 properties while
/// its keyboard is visible. This observation is scoped to one request.
fn watch_dismissal(cancel: Arc<AtomicBool>) -> Receiver<()> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let Ok((conn, screen)) = x11rb::connect(Some(":0")) else {
            return; // Other gamescope layouts keep manual Back/Enter behavior.
        };
        let root = conn.setup().roots[screen].root;
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut phase = OverlayPhase::default();
        let mut target = None;
        let mut missing_since = None;
        while !cancel.load(Ordering::Relaxed) {
            if target.is_none() {
                target = steam_ui_window(&conn, root);
            }
            if let Some((window, overlay, focus)) = target {
                let state = match (
                    cardinal(&conn, window, overlay),
                    cardinal(&conn, window, focus),
                ) {
                    (Some(a), Some(b)) => Some((a, b)),
                    _ => None,
                };
                if state.is_some() {
                    missing_since = None;
                } else {
                    target = None;
                    missing_since.get_or_insert_with(Instant::now);
                }
                if phase.observe(state) {
                    if !cancel.load(Ordering::Relaxed) {
                        let _ = tx.send(());
                    }
                    return;
                }
            }
            if !phase.saw_keyboard && Instant::now() >= deadline {
                return; // URI failed or SteamUI did not show the keyboard.
            }
            if missing_since.is_some_and(|start| start.elapsed() >= Duration::from_secs(1)) {
                return; // SteamUI disappeared; leave manual Back available.
            }
            std::thread::sleep(Duration::from_millis(80));
        }
    });
    rx
}

fn cardinal(conn: &RustConnection, window: Window, atom: Atom) -> Option<u32> {
    conn.get_property(false, window, atom, AtomEnum::CARDINAL, 0, 1)
        .ok()?
        .reply()
        .ok()?
        .value32()?
        .next()
}

fn steam_ui_window(conn: &RustConnection, root: Window) -> Option<(Window, Atom, Atom)> {
    let overlay = conn
        .intern_atom(true, b"STEAM_OVERLAY")
        .ok()?
        .reply()
        .ok()?
        .atom;
    let focus = conn
        .intern_atom(true, b"STEAM_INPUT_FOCUS")
        .ok()?
        .reply()
        .ok()?
        .atom;
    if overlay == 0 || focus == 0 {
        return None;
    }
    // SteamUI's observed window is a root child on SteamOS 3.9. A shallow
    // scan also handles an X11 window manager that reparents that window.
    let mut candidates = vec![(root, 0)];
    while let Some((window, depth)) = candidates.pop() {
        if cardinal(conn, window, overlay).is_some() && cardinal(conn, window, focus).is_some() {
            return Some((window, overlay, focus));
        }
        if depth < 2 {
            if let Ok(cookie) = conn.query_tree(window) {
                if let Ok(reply) = cookie.reply() {
                    candidates.extend(reply.children.into_iter().take(512).map(|w| (w, depth + 1)));
                }
            }
        }
    }
    None
}

/// Send one URI without blocking the UI. The worker waits for its child.
pub fn send(uri: &'static str) -> Receiver<Result<(), String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = Command::new("steam")
            .arg(uri)
            .status()
            .map_err(|error| error.to_string())
            .and_then(|status| {
                if status.success() {
                    Ok(())
                } else {
                    Err(format!("Steam returned {status}"))
                }
            });
        let _ = tx.send(result);
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_requests_never_contain_field_text() {
        assert_eq!(OPEN_URI, "steam://open/keyboard");
        assert_eq!(CLOSE_URI, "steam://close/keyboard");
    }

    #[test]
    fn overlay_must_open_before_zero_means_dismissal() {
        let mut phase = OverlayPhase::default();
        assert!(!phase.observe(Some((0, 0))));
        assert!(!phase.observe(None));
        assert!(!phase.observe(Some((1, 0))));
        assert!(!phase.observe(Some((1, 2))));
        assert!(!phase.observe(Some((1, 2))));
        assert!(!phase.observe(None));
        assert!(phase.observe(Some((0, 0))));
    }
}
