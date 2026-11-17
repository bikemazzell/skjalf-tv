//! Pure 10-foot navigation state machines (router, focus, zapping, OSD).

/// Top-level view. Details screens extend this list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Setup,
    Main,
    Live,
    Movies,
    MovieDetails,
    Series,
    SeriesDetails,
    Settings,
    Player,
}

impl View {
    /// Views whose Up/Down selection repeats while the direction is held.
    pub fn repeats_held_direction(self) -> bool {
        matches!(
            self,
            View::Live | View::Movies | View::Series | View::SeriesDetails
        )
    }
}

/// View stack. The root never pops, so Back cannot leave the app.
#[derive(Debug)]
pub struct Router {
    stack: Vec<View>,
}

impl Router {
    pub fn new(root: View) -> Self {
        Self { stack: vec![root] }
    }

    pub fn current(&self) -> View {
        *self.stack.last().expect("router stack is never empty")
    }

    pub fn push(&mut self, view: View) {
        self.stack.push(view);
    }

    /// Pop the current view and return the new current view, or `None` when
    /// only the root remains.
    pub fn back(&mut self) -> Option<View> {
        if self.stack.len() > 1 {
            self.stack.pop();
            Some(self.current())
        } else {
            None
        }
    }

    /// Drop the whole stack and start again from `root`.
    pub fn reset(&mut self, root: View) {
        self.stack.clear();
        self.stack.push(root);
    }
}

/// Channel-number zapping: buffered digits with a 2.5 s completion timeout.
///
/// Time is an injected millisecond counter so tests need no clock.
#[derive(Debug, Default)]
pub struct ZapBuffer {
    digits: String,
    deadline: u64,
}

const ZAP_TIMEOUT_MS: u64 = 2500;
const ZAP_MAX_DIGITS: usize = 5;

impl ZapBuffer {
    /// Buffer a digit (0–9), restarting the completion timeout.
    pub fn push(&mut self, digit: u8, now_ms: u64) {
        if self.digits.len() >= ZAP_MAX_DIGITS {
            return;
        }
        self.digits.push((b'0' + digit.min(9)) as char);
        self.deadline = now_ms + ZAP_TIMEOUT_MS;
    }

    pub fn digits(&self) -> &str {
        &self.digits
    }

    pub fn is_expired(&self, now_ms: u64) -> bool {
        now_ms >= self.deadline
    }

    /// Consume the buffered number once the timeout elapsed (or nothing).
    pub fn take_if_expired(&mut self, now_ms: u64) -> Option<u32> {
        if !self.digits.is_empty() && self.is_expired(now_ms) {
            self.force()
        } else {
            None
        }
    }

    /// Consume the buffered number immediately (e.g. Enter pressed).
    pub fn force(&mut self) -> Option<u32> {
        let n = self.digits.parse().ok();
        self.digits.clear();
        n
    }
}

/// What the player OSD currently shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OsdKind {
    /// Zap overlay: typed digits / channel number + name.
    Zap(String),
    /// Now/next info bar.
    Info,
    /// EPG side panel.
    Epg,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Osd {
    pub kind: OsdKind,
    /// Millisecond timestamp at which the OSD hides.
    pub hide_at: u64,
}

/// Player-view state: current channel + OSD with auto-hide.
#[derive(Debug, Default)]
pub struct PlayerModel {
    pub channel_id: Option<String>,
    osd: Option<Osd>,
}

const OSD_TTL_MS: u64 = 3000;

impl PlayerModel {
    pub fn show_osd(&mut self, kind: OsdKind, now_ms: u64) {
        self.osd = Some(Osd {
            kind,
            hide_at: now_ms + OSD_TTL_MS,
        });
    }

    pub fn osd(&self) -> &Osd {
        static HIDDEN: std::sync::OnceLock<Osd> = std::sync::OnceLock::new();
        self.osd.as_ref().unwrap_or(HIDDEN.get_or_init(|| Osd {
            kind: OsdKind::Info,
            hide_at: 0,
        }))
    }

    pub fn osd_visible(&self, now_ms: u64) -> bool {
        self.osd.as_ref().is_some_and(|o| now_ms < o.hide_at)
    }

    pub fn hide_osd(&mut self) {
        self.osd = None;
    }
}

/// Next/previous channel id within a group's channel list (wraps).
pub fn step_channel(ids: &[String], current: &str, forward: bool) -> Option<String> {
    let idx = ids.iter().position(|id| id == current)?;
    if ids.is_empty() {
        return None;
    }
    let next = if forward {
        (idx + 1) % ids.len()
    } else {
        (idx + ids.len() - 1) % ids.len()
    };
    Some(ids[next].clone())
}
