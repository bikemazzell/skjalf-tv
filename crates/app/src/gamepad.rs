//! Gamepad input: Linux `evdev` devices (`/dev/input/event*`) mapped to
//! [`Action`]s.
//!
//! In Steam Deck Game Mode, Steam Input exposes the controls as a standard
//! evdev gamepad (D-pad + A/B/X/Y + shoulders), so no SDL is required.
//! Mapping is pure and unit-tested; device I/O lives in a per-device thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::input::Action;

// Linux event codes (see linux/input-event-codes.h).
pub const BTN_SOUTH: u16 = 0x130; // A / Cross
pub const BTN_EAST: u16 = 0x131; // B / Circle
pub const BTN_NORTH: u16 = 0x133; // Y / Triangle
pub const BTN_WEST: u16 = 0x134; // X / Square
pub const BTN_TL: u16 = 0x136; // L1
pub const BTN_TR: u16 = 0x137; // R1
pub const BTN_SELECT: u16 = 0x13a;
pub const BTN_START: u16 = 0x13b;
pub const BTN_DPAD_UP: u16 = 0x220;
pub const BTN_DPAD_DOWN: u16 = 0x221;
pub const BTN_DPAD_LEFT: u16 = 0x222;
pub const BTN_DPAD_RIGHT: u16 = 0x223;
pub const ABS_HAT0X: u16 = 0x10;
pub const ABS_HAT0Y: u16 = 0x11;
pub const ABS_X: u16 = 0x00;
pub const ABS_Y: u16 = 0x01;

const STICK_DEAD_ZONE: f32 = 0.35;
const FIRST_REPEAT_MS: u64 = 300;
const REPEAT_MS: u64 = 140;

/// One calibrated stick axis. A held direction repeats at a fixed rate.
pub struct StickRepeater {
    axis: u16,
    min: i32,
    max: i32,
    held: Option<Action>,
    next_repeat: u64,
    repeat_armed: bool,
}

impl StickRepeater {
    pub fn new(axis: u16, min: i32, max: i32) -> Self {
        Self {
            axis,
            min,
            max,
            held: None,
            next_repeat: 0,
            repeat_armed: false,
        }
    }

    pub fn observe(&mut self, value: i32, now_ms: u64) -> Option<Action> {
        let center = (self.min as f32 + self.max as f32) / 2.0;
        let delta = value as f32 - center;
        let range = if delta >= 0.0 {
            self.max as f32 - center
        } else {
            center - self.min as f32
        };
        let normalized = if range > 0.0 { delta / range } else { 0.0 };
        let direction = if normalized > STICK_DEAD_ZONE {
            Some(if self.axis == ABS_X {
                Action::Right
            } else {
                Action::Down
            })
        } else if normalized < -STICK_DEAD_ZONE {
            Some(if self.axis == ABS_X {
                Action::Left
            } else {
                Action::Up
            })
        } else {
            None
        };
        if direction != self.held {
            self.held = direction;
            self.next_repeat = now_ms + FIRST_REPEAT_MS;
            self.repeat_armed = direction.is_some();
            direction
        } else {
            None
        }
    }

    pub fn tick(&mut self, now_ms: u64, repeat_enabled: bool) -> Option<Action> {
        if !repeat_enabled {
            self.repeat_armed = false;
        }
        if self.repeat_armed && now_ms >= self.next_repeat {
            if let Some(action) = self.held {
                self.next_repeat = now_ms + REPEAT_MS;
                return Some(action);
            }
        }
        None
    }

    fn reset(&mut self) {
        self.held = None;
        self.repeat_armed = false;
    }
}

/// Fuse the D-pad button and hat reports for one axis before repeating.
struct DpadAxis {
    negative: Action,
    positive: Action,
    negative_button: bool,
    positive_button: bool,
    hat: i32,
    held: Option<Action>,
    next_repeat: u64,
    repeat_armed: bool,
}

impl DpadAxis {
    fn new(negative: Action, positive: Action) -> Self {
        Self {
            negative,
            positive,
            negative_button: false,
            positive_button: false,
            hat: 0,
            held: None,
            next_repeat: 0,
            repeat_armed: false,
        }
    }

    fn observe_button(&mut self, positive: bool, pressed: bool, now_ms: u64) -> Option<Action> {
        if positive {
            self.positive_button = pressed;
        } else {
            self.negative_button = pressed;
        }
        self.changed(now_ms)
    }

    fn observe_hat(&mut self, value: i32, now_ms: u64) -> Option<Action> {
        if !(-1..=1).contains(&value) {
            return None;
        }
        self.hat = value;
        self.changed(now_ms)
    }

    fn changed(&mut self, now_ms: u64) -> Option<Action> {
        let direction = if self.hat < 0 || (self.hat == 0 && self.negative_button) {
            Some(self.negative)
        } else if self.hat > 0 || self.positive_button {
            Some(self.positive)
        } else {
            None
        };
        if direction != self.held {
            self.held = direction;
            self.next_repeat = now_ms + FIRST_REPEAT_MS;
            self.repeat_armed = direction.is_some();
            direction
        } else {
            None
        }
    }

    fn tick(&mut self, now_ms: u64, repeat_enabled: bool) -> Option<Action> {
        if !repeat_enabled {
            self.repeat_armed = false;
        }
        if self.repeat_armed && now_ms >= self.next_repeat {
            self.next_repeat = now_ms + REPEAT_MS;
            return self.held;
        }
        None
    }

    fn reset(&mut self) {
        *self = Self::new(self.negative, self.positive);
    }
}

/// Map a button press (key code) to an action.
pub fn button_to_action(code: u16) -> Option<Action> {
    match code {
        BTN_SOUTH => Some(Action::Select),
        BTN_EAST => Some(Action::Back),
        BTN_NORTH => Some(Action::Epg),
        BTN_WEST => Some(Action::Favorite),
        BTN_TL => Some(Action::ChUp),
        BTN_TR => Some(Action::ChDown),
        BTN_START => Some(Action::Quit),
        BTN_SELECT => Some(Action::Options),
        BTN_DPAD_UP => Some(Action::Up),
        BTN_DPAD_DOWN => Some(Action::Down),
        BTN_DPAD_LEFT => Some(Action::Left),
        BTN_DPAD_RIGHT => Some(Action::Right),
        _ => None,
    }
}

/// Map a hat axis value (−1/0/+1) to an action; 0 (release) maps to None.
pub fn hat_to_action(axis: u16, value: i32) -> Option<Action> {
    match (axis, value) {
        (ABS_HAT0X, -1) => Some(Action::Left),
        (ABS_HAT0X, 1) => Some(Action::Right),
        (ABS_HAT0Y, -1) => Some(Action::Up),
        (ABS_HAT0Y, 1) => Some(Action::Down),
        _ => None,
    }
}

fn key_event_action(
    code: u16,
    value: i32,
    horizontal: &mut DpadAxis,
    vertical: &mut DpadAxis,
    now_ms: u64,
) -> Option<Action> {
    match (code, value) {
        (BTN_DPAD_LEFT, 0 | 1) => horizontal.observe_button(false, value == 1, now_ms),
        (BTN_DPAD_RIGHT, 0 | 1) => horizontal.observe_button(true, value == 1, now_ms),
        (BTN_DPAD_UP, 0 | 1) => vertical.observe_button(false, value == 1, now_ms),
        (BTN_DPAD_DOWN, 0 | 1) => vertical.observe_button(true, value == 1, now_ms),
        (_, 1) => button_to_action(code),
        _ => None, // key release and kernel repeat never repeat A/B
    }
}

/// A device is a gamepad when it has the A button (filters keyboards, mice,
/// touchpads, IR remotes with only keyboard keys).
pub fn is_gamepad(supported_keys: &[u16]) -> bool {
    supported_keys.contains(&BTN_SOUTH)
}

/// Handle to the background gamepad reader threads.
pub struct GamepadThread {
    rx: Receiver<GamepadAction>,
    alive: Arc<AtomicBool>,
    focused: Arc<AtomicBool>,
    repeat_enabled: Arc<AtomicBool>,
    reset_held: Arc<AtomicBool>,
    last_drain: Instant,
}

/// A first press or a held-direction repeat from the gamepad reader.
pub struct GamepadAction {
    pub action: Action,
    pub repeated: bool,
    at: Instant,
}

impl GamepadThread {
    /// Discover a controller now and after future device changes.
    /// Only one reader owns the active controller at a time.
    pub fn spawn() -> Option<Self> {
        let (tx, rx) = mpsc::channel();
        let alive = Arc::new(AtomicBool::new(true));
        let monitor_alive = alive.clone();
        let focused = Arc::new(AtomicBool::new(false));
        let repeat_enabled = Arc::new(AtomicBool::new(false));
        let reset_held = Arc::new(AtomicBool::new(false));
        let reader_focus = focused.clone();
        let reader_repeat = repeat_enabled.clone();
        let reader_reset = reset_held.clone();
        std::thread::spawn(move || {
            discover_loop(tx, monitor_alive, reader_focus, reader_repeat, reader_reset);
        });
        Some(Self {
            rx,
            alive,
            focused,
            repeat_enabled,
            reset_held,
            last_drain: Instant::now(),
        })
    }

    /// Drop stale input and all input on focus return. Repeats run only in list views.
    pub fn drain(&mut self, focused: bool, repeat_enabled: bool) -> Vec<GamepadAction> {
        let now = Instant::now();
        let was_focused = self.focused.swap(focused, Ordering::Relaxed);
        let gap = now.duration_since(self.last_drain) > Duration::from_millis(250);
        self.last_drain = now;
        self.repeat_enabled
            .store(focused && repeat_enabled, Ordering::Relaxed);
        if !focused || !was_focused || gap {
            self.reset_held.store(true, Ordering::Relaxed);
            self.rx.try_iter().for_each(drop);
            return Vec::new();
        }
        self.rx
            .try_iter()
            .filter(|event| now.duration_since(event.at) <= Duration::from_millis(200))
            .collect()
    }
}

impl Drop for GamepadThread {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
    }
}

fn discover_loop(
    tx: Sender<GamepadAction>,
    alive: Arc<AtomicBool>,
    focused: Arc<AtomicBool>,
    repeat_enabled: Arc<AtomicBool>,
    reset_held: Arc<AtomicBool>,
) {
    let (done_tx, done_rx) = mpsc::channel();
    let mut active: Option<std::path::PathBuf> = None;
    while alive.load(Ordering::Relaxed) {
        while let Ok(done) = done_rx.try_recv() {
            if active.as_ref() == Some(&done) {
                active = None;
            }
        }
        if active.is_none() {
            for (path, device) in evdev::enumerate() {
                let keys: Vec<u16> = device
                    .supported_keys()
                    .map(|keys| keys.iter().map(|key| key.0).collect())
                    .unwrap_or_default();
                if !is_gamepad(&keys) {
                    continue;
                }
                active = Some(path.clone());
                let reader_tx = tx.clone();
                let reader_done = done_tx.clone();
                let reader_alive = alive.clone();
                let reader_focus = focused.clone();
                let reader_repeat = repeat_enabled.clone();
                let reader_reset = reset_held.clone();
                std::thread::spawn(move || {
                    read_loop(
                        device,
                        reader_tx,
                        reader_alive,
                        reader_focus,
                        reader_repeat,
                        reader_reset,
                    );
                    let _ = reader_done.send(path);
                });
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn read_loop(
    mut device: evdev::Device,
    tx: Sender<GamepadAction>,
    alive: Arc<AtomicBool>,
    focused: Arc<AtomicBool>,
    repeat_enabled: Arc<AtomicBool>,
    reset_held: Arc<AtomicBool>,
) {
    if device.set_nonblocking(true).is_err() {
        return;
    }
    let axis_info: Vec<_> = device
        .get_absinfo()
        .map(|axes| {
            axes.map(|(axis, info)| (axis.0, info.minimum(), info.maximum()))
                .collect()
        })
        .unwrap_or_default();
    let mut x = axis_info
        .iter()
        .find(|(axis, _, _)| *axis == ABS_X)
        .map(|(_, min, max)| StickRepeater::new(ABS_X, *min, *max));
    let mut y = axis_info
        .iter()
        .find(|(axis, _, _)| *axis == ABS_Y)
        .map(|(_, min, max)| StickRepeater::new(ABS_Y, *min, *max));
    let mut horizontal = DpadAxis::new(Action::Left, Action::Right);
    let mut vertical = DpadAxis::new(Action::Up, Action::Down);
    let clock = Instant::now();
    let mut last_sent: Option<(Action, u64)> = None;
    while alive.load(Ordering::Relaxed) {
        let now_ms = clock.elapsed().as_millis() as u64;
        if reset_held.swap(false, Ordering::Relaxed) || !focused.load(Ordering::Relaxed) {
            horizontal.reset();
            vertical.reset();
            if let Some(stick) = &mut x {
                stick.reset();
            }
            if let Some(stick) = &mut y {
                stick.reset();
            }
            match device.fetch_events() {
                Ok(_) => {} // clear presses made while unfocused
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => return, // device removed; discover a replacement
            }
            std::thread::sleep(Duration::from_millis(16));
            continue;
        }
        match device.fetch_events() {
            Ok(events) => {
                for event in events {
                    let action = match event.destructure() {
                        evdev::EventSummary::Key(_, code, value) => {
                            key_event_action(code.0, value, &mut horizontal, &mut vertical, now_ms)
                        }
                        evdev::EventSummary::AbsoluteAxis(_, axis, value) => match axis.0 {
                            ABS_X => x.as_mut().and_then(|stick| stick.observe(value, now_ms)),
                            ABS_Y => y.as_mut().and_then(|stick| stick.observe(value, now_ms)),
                            ABS_HAT0X => horizontal.observe_hat(value, now_ms),
                            ABS_HAT0Y => vertical.observe_hat(value, now_ms),
                            _ => None,
                        },
                        _ => None,
                    };
                    if let Some(action) = action {
                        if !send_once(&tx, &mut last_sent, action, now_ms, false) {
                            return;
                        }
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => return,
        }
        let repeat = repeat_enabled.load(Ordering::Relaxed);
        for action in [
            horizontal.tick(now_ms, repeat),
            vertical.tick(now_ms, repeat),
            x.as_mut().and_then(|stick| stick.tick(now_ms, repeat)),
            y.as_mut().and_then(|stick| stick.tick(now_ms, repeat)),
        ]
        .into_iter()
        .flatten()
        {
            if !send_once(&tx, &mut last_sent, action, now_ms, true) {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(16));
    }
}

fn send_once(
    tx: &Sender<GamepadAction>,
    last: &mut Option<(Action, u64)>,
    action: Action,
    now_ms: u64,
    repeated: bool,
) -> bool {
    if last.is_some_and(|(prev, at)| prev == action && now_ms.saturating_sub(at) < 45) {
        return true;
    }
    *last = Some((action, now_ms));
    tx.send(GamepadAction {
        action,
        repeated,
        at: Instant::now(),
    })
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn face_buttons_map() {
        assert_eq!(button_to_action(BTN_SOUTH), Some(Action::Select));
        assert_eq!(button_to_action(BTN_EAST), Some(Action::Back));
        assert_eq!(button_to_action(BTN_NORTH), Some(Action::Epg));
        assert_eq!(button_to_action(BTN_WEST), Some(Action::Favorite));
        assert_eq!(button_to_action(BTN_TL), Some(Action::ChUp));
        assert_eq!(button_to_action(BTN_TR), Some(Action::ChDown));
        assert_eq!(button_to_action(BTN_START), Some(Action::Quit));
        assert_eq!(button_to_action(BTN_SELECT), Some(Action::Options));
    }

    #[test]
    fn kernel_key_repeat_does_not_repeat_select_or_back() {
        let mut horizontal = DpadAxis::new(Action::Left, Action::Right);
        let mut vertical = DpadAxis::new(Action::Up, Action::Down);
        assert_eq!(
            key_event_action(BTN_SOUTH, 1, &mut horizontal, &mut vertical, 0),
            Some(Action::Select)
        );
        assert_eq!(
            key_event_action(BTN_SOUTH, 2, &mut horizontal, &mut vertical, 1),
            None
        );
        assert_eq!(
            key_event_action(BTN_EAST, 1, &mut horizontal, &mut vertical, 2),
            Some(Action::Back)
        );
        assert_eq!(
            key_event_action(BTN_EAST, 2, &mut horizontal, &mut vertical, 3),
            None
        );
    }

    #[test]
    fn dpad_buttons_map() {
        assert_eq!(button_to_action(BTN_DPAD_UP), Some(Action::Up));
        assert_eq!(button_to_action(BTN_DPAD_DOWN), Some(Action::Down));
        assert_eq!(button_to_action(BTN_DPAD_LEFT), Some(Action::Left));
        assert_eq!(button_to_action(BTN_DPAD_RIGHT), Some(Action::Right));
        assert_eq!(button_to_action(0x123), None);
    }

    #[test]
    fn hat_axes_map_with_release_none() {
        assert_eq!(hat_to_action(ABS_HAT0Y, -1), Some(Action::Up));
        assert_eq!(hat_to_action(ABS_HAT0Y, 1), Some(Action::Down));
        assert_eq!(hat_to_action(ABS_HAT0X, -1), Some(Action::Left));
        assert_eq!(hat_to_action(ABS_HAT0X, 1), Some(Action::Right));
        assert_eq!(hat_to_action(ABS_HAT0X, 0), None);
        assert_eq!(hat_to_action(0x2f, 1), None);
    }

    #[test]
    fn gamepad_detection_requires_a_button() {
        assert!(is_gamepad(&[BTN_EAST, BTN_SOUTH]));
        assert!(!is_gamepad(&[BTN_EAST, 0x101])); // keyboard KEY_Q etc.
    }

    #[test]
    fn stick_dead_zone_and_repeat_are_bounded() {
        let mut stick = StickRepeater::new(ABS_X, -32768, 32767);
        assert_eq!(stick.observe(2000, 0), None);
        assert_eq!(stick.observe(25000, 10), Some(Action::Right));
        assert_eq!(stick.tick(250, true), None);
        assert_eq!(stick.tick(310, true), Some(Action::Right));
        assert_eq!(stick.tick(350, true), None);
        assert_eq!(stick.tick(450, true), Some(Action::Right));
        assert_eq!(stick.observe(0, 460), None);
        assert_eq!(stick.tick(1000, true), None);
    }

    #[test]
    fn held_hat_and_button_share_one_repeat_and_stop_on_release() {
        let mut dpad = DpadAxis::new(Action::Up, Action::Down);
        assert_eq!(dpad.observe_hat(1, 0), Some(Action::Down));
        assert_eq!(dpad.observe_button(true, true, 5), None);
        assert_eq!(dpad.tick(299, true), None);
        assert_eq!(dpad.tick(300, true), Some(Action::Down));
        assert_eq!(dpad.tick(439, true), None);
        assert_eq!(dpad.tick(440, true), Some(Action::Down));
        assert_eq!(dpad.observe_hat(0, 450), None); // button still down
        assert_eq!(dpad.observe_button(true, false, 460), None);
        assert_eq!(dpad.tick(1000, true), None);
    }

    #[test]
    fn dpad_reversal_and_list_exit_rearm_only_after_release() {
        let mut dpad = DpadAxis::new(Action::Up, Action::Down);
        assert_eq!(dpad.observe_button(true, true, 0), Some(Action::Down));
        assert_eq!(dpad.tick(300, false), None); // non-list view
        assert_eq!(dpad.tick(600, true), None); // returning to a list while held
        assert_eq!(dpad.observe_button(true, false, 610), None);
        assert_eq!(dpad.observe_button(false, true, 620), Some(Action::Up));
        assert_eq!(dpad.tick(919, true), None);
        assert_eq!(dpad.tick(920, true), Some(Action::Up));
        assert_eq!(dpad.observe_hat(1, 930), Some(Action::Down));
        assert_eq!(dpad.tick(1229, true), None);
        assert_eq!(dpad.tick(1230, true), Some(Action::Down));
    }

    #[test]
    fn queued_input_is_dropped_on_focus_return_and_after_frame_gap() {
        let (tx, rx) = mpsc::channel();
        let mut gamepad = GamepadThread {
            rx,
            alive: Arc::new(AtomicBool::new(true)),
            focused: Arc::new(AtomicBool::new(false)),
            repeat_enabled: Arc::new(AtomicBool::new(false)),
            reset_held: Arc::new(AtomicBool::new(false)),
            last_drain: Instant::now(),
        };
        let repeat = || GamepadAction {
            action: Action::Down,
            repeated: true,
            at: Instant::now(),
        };
        tx.send(repeat()).unwrap();
        assert!(gamepad.drain(true, true).is_empty()); // initial focus
        tx.send(repeat()).unwrap();
        assert_eq!(gamepad.drain(true, true).len(), 1);
        tx.send(repeat()).unwrap();
        assert!(gamepad.drain(false, true).is_empty());
        tx.send(repeat()).unwrap();
        assert!(gamepad.drain(true, true).is_empty()); // focus return
        assert!(gamepad.reset_held.load(Ordering::Relaxed));
        gamepad.last_drain = Instant::now() - Duration::from_secs(1);
        tx.send(repeat()).unwrap();
        assert!(gamepad.drain(true, true).is_empty()); // stalled viewport
    }
}
