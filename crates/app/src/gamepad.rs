//! Gamepad input: Linux `evdev` devices (`/dev/input/event*`) mapped to
//! [`Action`]s.
//!
//! In Steam Deck Game Mode, Steam Input exposes the controls as a standard
//! evdev gamepad (D-pad + A/B/X/Y + shoulders), so no SDL is required.
//! Mapping is pure and unit-tested; device I/O lives in a per-device thread.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
const MIRROR_WINDOW: Duration = Duration::from_millis(150);
const DEVICE_SCAN_INTERVAL: Duration = Duration::from_millis(500);

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

/// Picks one controller at a time from several devices that may mirror each
/// other (Steam Input's virtual pad duplicates the raw device it wraps).
#[derive(Default)]
struct Arbiter {
    owner: Option<(u64, Instant)>,
}

impl Arbiter {
    fn accept(&mut self, device: u64, repeated: bool, at: Instant) -> bool {
        match self.owner {
            Some((owner, _)) if owner == device => {
                self.owner = Some((device, at));
                true
            }
            Some((_, last)) if at.saturating_duration_since(last) <= MIRROR_WINDOW => false,
            _ if repeated => false,
            _ => {
                self.owner = Some((device, at));
                true
            }
        }
    }
}

/// Handle to the background gamepad reader threads.
pub struct GamepadThread {
    rx: Receiver<GamepadAction>,
    alive: Arc<AtomicBool>,
    focused: Arc<AtomicBool>,
    repeat_enabled: Arc<AtomicBool>,
    reset_generation: Arc<AtomicU64>,
    arbiter: Arbiter,
    last_drain: Instant,
}

/// A first press or a held-direction repeat from one gamepad reader.
pub struct GamepadAction {
    pub action: Action,
    pub repeated: bool,
    device: u64,
    at: Instant,
}

impl GamepadThread {
    /// Read every controller now and after future device changes.
    /// Input from one controller at a time passes; its mirrors are dropped.
    pub fn spawn() -> Option<Self> {
        let (tx, rx) = mpsc::channel();
        let alive = Arc::new(AtomicBool::new(true));
        let monitor_alive = alive.clone();
        let focused = Arc::new(AtomicBool::new(false));
        let repeat_enabled = Arc::new(AtomicBool::new(false));
        let reset_generation = Arc::new(AtomicU64::new(0));
        let reader_focus = focused.clone();
        let reader_repeat = repeat_enabled.clone();
        let reader_reset = reset_generation.clone();
        std::thread::spawn(move || {
            discover_loop(tx, monitor_alive, reader_focus, reader_repeat, reader_reset);
        });
        Some(Self {
            rx,
            alive,
            focused,
            repeat_enabled,
            reset_generation,
            arbiter: Arbiter::default(),
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
            self.reset_generation.fetch_add(1, Ordering::Relaxed);
            self.rx.try_iter().for_each(drop);
            return Vec::new();
        }
        let arbiter = &mut self.arbiter;
        self.rx
            .try_iter()
            .filter(|event| now.duration_since(event.at) <= Duration::from_millis(200))
            .filter(|event| arbiter.accept(event.device, event.repeated, event.at))
            .collect()
    }
}

impl Drop for GamepadThread {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
    }
}

fn input_device_names() -> Vec<OsString> {
    let mut names: Vec<OsString> = std::fs::read_dir("/dev/input")
        .map(|entries| entries.flatten().map(|entry| entry.file_name()).collect())
        .unwrap_or_default();
    names.sort();
    names
}

fn unclaimed<T>(found: Vec<(PathBuf, T)>, active: &HashSet<PathBuf>) -> Vec<(PathBuf, T)> {
    found
        .into_iter()
        .filter(|(path, _)| !active.contains(path))
        .collect()
}

fn discover_loop(
    tx: Sender<GamepadAction>,
    alive: Arc<AtomicBool>,
    focused: Arc<AtomicBool>,
    repeat_enabled: Arc<AtomicBool>,
    reset_generation: Arc<AtomicU64>,
) {
    let (done_tx, done_rx) = mpsc::channel();
    let mut active: HashSet<PathBuf> = HashSet::new();
    let mut known_names: Option<Vec<OsString>> = None;
    let mut next_device_id = 0u64;
    while alive.load(Ordering::Relaxed) {
        let mut reader_exited = false;
        while let Ok(done) = done_rx.try_recv() {
            active.remove(&done);
            reader_exited = true;
        }
        let names = input_device_names();
        if reader_exited || known_names.as_ref() != Some(&names) {
            known_names = Some(names);
            let gamepads = evdev::enumerate()
                .filter(|(_, device)| {
                    let keys: Vec<u16> = device
                        .supported_keys()
                        .map(|keys| keys.iter().map(|key| key.0).collect())
                        .unwrap_or_default();
                    is_gamepad(&keys)
                })
                .collect();
            for (path, device) in unclaimed(gamepads, &active) {
                active.insert(path.clone());
                let device_id = next_device_id;
                next_device_id += 1;
                let reader_tx = tx.clone();
                let reader_done = done_tx.clone();
                let reader_alive = alive.clone();
                let reader_focus = focused.clone();
                let reader_repeat = repeat_enabled.clone();
                let reader_reset = reset_generation.clone();
                std::thread::spawn(move || {
                    read_loop(
                        device,
                        device_id,
                        reader_tx,
                        reader_alive,
                        reader_focus,
                        reader_repeat,
                        reader_reset,
                    );
                    let _ = reader_done.send(path);
                });
            }
        }
        std::thread::sleep(DEVICE_SCAN_INTERVAL);
    }
}

fn read_loop(
    mut device: evdev::Device,
    device_id: u64,
    tx: Sender<GamepadAction>,
    alive: Arc<AtomicBool>,
    focused: Arc<AtomicBool>,
    repeat_enabled: Arc<AtomicBool>,
    reset_generation: Arc<AtomicU64>,
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
    let mut seen_reset = reset_generation.load(Ordering::Relaxed);
    while alive.load(Ordering::Relaxed) {
        let now_ms = clock.elapsed().as_millis() as u64;
        let generation = reset_generation.load(Ordering::Relaxed);
        let reset_requested = generation != seen_reset;
        seen_reset = generation;
        if reset_requested || !focused.load(Ordering::Relaxed) {
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
                        if !send_once(&tx, device_id, &mut last_sent, action, now_ms, false) {
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
            if !send_once(&tx, device_id, &mut last_sent, action, now_ms, true) {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(16));
    }
}

fn send_once(
    tx: &Sender<GamepadAction>,
    device: u64,
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
        device,
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

    fn gamepad_with_channel() -> (Sender<GamepadAction>, GamepadThread) {
        let (tx, rx) = mpsc::channel();
        let gamepad = GamepadThread {
            rx,
            alive: Arc::new(AtomicBool::new(true)),
            focused: Arc::new(AtomicBool::new(false)),
            repeat_enabled: Arc::new(AtomicBool::new(false)),
            reset_generation: Arc::new(AtomicU64::new(0)),
            arbiter: Arbiter::default(),
            last_drain: Instant::now(),
        };
        (tx, gamepad)
    }

    fn press(device: u64, action: Action, repeated: bool) -> GamepadAction {
        GamepadAction {
            action,
            repeated,
            device,
            at: Instant::now(),
        }
    }

    #[test]
    fn queued_input_is_dropped_on_focus_return_and_after_frame_gap() {
        let (tx, mut gamepad) = gamepad_with_channel();
        let down = || press(0, Action::Down, false);
        tx.send(down()).unwrap();
        assert!(gamepad.drain(true, true).is_empty()); // initial focus
        assert_eq!(gamepad.reset_generation.load(Ordering::Relaxed), 1);
        tx.send(down()).unwrap();
        assert_eq!(gamepad.drain(true, true).len(), 1);
        assert_eq!(gamepad.reset_generation.load(Ordering::Relaxed), 1);
        tx.send(down()).unwrap();
        assert!(gamepad.drain(false, true).is_empty());
        assert_eq!(gamepad.reset_generation.load(Ordering::Relaxed), 2);
        tx.send(down()).unwrap();
        assert!(gamepad.drain(true, true).is_empty()); // focus return
        assert_eq!(gamepad.reset_generation.load(Ordering::Relaxed), 3);
        gamepad.last_drain = Instant::now() - Duration::from_secs(1);
        tx.send(down()).unwrap();
        assert!(gamepad.drain(true, true).is_empty()); // stalled viewport
        assert_eq!(gamepad.reset_generation.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn drain_drops_mirrored_device_input_and_dropped_events_do_not_own() {
        let (tx, mut gamepad) = gamepad_with_channel();
        assert!(gamepad.drain(true, true).is_empty());
        tx.send(press(0, Action::Select, false)).unwrap();
        tx.send(press(1, Action::Back, false)).unwrap(); // mirror, other mapping
        tx.send(press(0, Action::Down, false)).unwrap();
        let actions: Vec<Action> = gamepad
            .drain(true, true)
            .into_iter()
            .map(|event| event.action)
            .collect();
        assert_eq!(actions, [Action::Select, Action::Down]);

        tx.send(press(1, Action::Up, false)).unwrap();
        gamepad.last_drain = Instant::now() - Duration::from_secs(1);
        assert!(gamepad.drain(true, true).is_empty()); // gap drop, arbiter untouched
        assert_eq!(gamepad.arbiter.owner.map(|(device, _)| device), Some(0));
    }

    #[test]
    fn first_press_takes_ownership_and_owner_events_pass() {
        let start = Instant::now();
        let mut arbiter = Arbiter::default();
        assert!(arbiter.accept(3, false, start));
        assert!(arbiter.accept(3, true, start + Duration::from_secs(5)));
        assert!(arbiter.accept(3, false, start + Duration::from_secs(6)));
    }

    #[test]
    fn mirror_within_window_is_dropped() {
        let start = Instant::now();
        let mut arbiter = Arbiter::default();
        assert!(arbiter.accept(0, false, start));
        assert!(!arbiter.accept(1, false, start + Duration::from_millis(5)));
        assert!(!arbiter.accept(1, false, start + MIRROR_WINDOW));
        assert!(arbiter.accept(0, false, start + Duration::from_millis(100)));
        assert!(!arbiter.accept(1, false, start + Duration::from_millis(240)));
    }

    #[test]
    fn press_after_quiet_window_takes_ownership_from_the_old_owner() {
        let start = Instant::now();
        let mut arbiter = Arbiter::default();
        assert!(arbiter.accept(0, false, start));
        let later = start + MIRROR_WINDOW + Duration::from_millis(1);
        assert!(arbiter.accept(1, false, later));
        assert!(!arbiter.accept(0, false, later + Duration::from_millis(10)));
        assert!(arbiter.accept(1, true, later + Duration::from_millis(20)));
    }

    #[test]
    fn repeats_from_a_non_owner_never_pass_or_take_ownership() {
        let start = Instant::now();
        let mut arbiter = Arbiter::default();
        assert!(!arbiter.accept(0, true, start));
        assert!(arbiter.owner.is_none());
        assert!(arbiter.accept(0, false, start));
        assert!(!arbiter.accept(1, true, start + Duration::from_secs(1)));
        assert!(arbiter.accept(0, true, start + Duration::from_secs(2)));
    }

    #[test]
    fn unclaimed_keeps_only_paths_without_a_reader() {
        let active: HashSet<PathBuf> = [PathBuf::from("/dev/input/event5")].into();
        let found = vec![
            (PathBuf::from("/dev/input/event3"), 'a'),
            (PathBuf::from("/dev/input/event5"), 'b'),
            (PathBuf::from("/dev/input/event7"), 'c'),
        ];
        assert_eq!(
            unclaimed(found, &active),
            [
                (PathBuf::from("/dev/input/event3"), 'a'),
                (PathBuf::from("/dev/input/event7"), 'c')
            ]
        );
        assert!(unclaimed(Vec::<(PathBuf, ())>::new(), &active).is_empty());
    }
}
