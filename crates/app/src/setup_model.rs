//! Setup form focus and source validation.

use crate::input::Action;
use skjalf_core::model::{Source, SourceKind, StreamFormat};

/// Editable text fields (which of them exist depends on the source kind).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Field {
    #[default]
    Host,
    Username,
    Password,
    Url,
}

/// Focused form row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Row {
    /// Xtream ↔ M3U toggle.
    #[default]
    Kind,
    /// One of the visible input fields (see [`SetupModel::fields`]).
    Field(usize),
    Connect,
}

/// Emitted when the user commits a complete, valid source.
#[derive(Debug, Clone, PartialEq)]
pub struct Connect(pub Source);

#[derive(Debug)]
pub struct SetupModel {
    /// `true` = Xtream Codes, `false` = M3U URL.
    pub xtream: bool,
    pub host: String,
    pub username: String,
    pub password: String,
    pub url: String,
    pub stream_format: StreamFormat,
    pub active: Field,
    pub row: Row,
    /// TextEdit field to focus after controller navigation.
    pub focus_request: Option<Field>,
    /// Status/error message shown to the user.
    pub message: Option<String>,
}

impl Default for SetupModel {
    fn default() -> Self {
        Self {
            xtream: true,
            host: String::new(),
            username: String::new(),
            password: String::new(),
            url: String::new(),
            stream_format: StreamFormat::default(),
            active: Field::Host,
            row: Row::Kind,
            focus_request: None,
            message: None,
        }
    }
}

impl SetupModel {
    pub fn focus_field(&mut self, field: Field) {
        if let Some(index) = self.fields().iter().position(|item| *item == field) {
            self.active = field;
            self.row = Row::Field(index);
        }
    }

    pub fn activate_field(&mut self, field: Field) {
        self.focus_field(field);
        self.focus_request = Some(field);
    }

    pub fn from_source(source: &Source) -> Self {
        let mut model = Self {
            stream_format: source.stream_format,
            ..Self::default()
        };
        match &source.kind {
            SourceKind::Xtream {
                host,
                username,
                password,
            } => {
                model.host = host.clone();
                model.username = username.clone();
                model.password = password.clone();
            }
            SourceKind::M3u { url } => {
                model.xtream = false;
                model.url = url.clone();
                model.active = Field::Url;
            }
        }
        model
    }

    /// Stable cache key for a source (used by the store).
    pub fn source_id(kind: &SourceKind) -> String {
        match kind {
            SourceKind::Xtream { host, username, .. } => format!("xtream:{host}/{username}"),
            SourceKind::M3u { url } => format!("m3u:{url}"),
        }
    }

    /// Focus helpers for the view (row highlight).
    pub fn row_is_kind(&self) -> bool {
        self.row == Row::Kind
    }
    pub fn row_is_field(&self, i: usize) -> bool {
        self.row == Row::Field(i)
    }
    pub fn row_is_connect(&self) -> bool {
        self.row == Row::Connect
    }

    /// Switch source kind (mouse-friendly variant of the Kind-row toggle).
    pub fn set_xtream(&mut self, xtream: bool) {
        self.xtream = xtream;
        self.focus_field(self.fields()[0]);
    }

    /// Visible form fields for the current source kind.
    pub fn fields(&self) -> &'static [Field] {
        if self.xtream {
            &[Field::Host, Field::Username, Field::Password]
        } else {
            &[Field::Url]
        }
    }

    pub fn field_text(&self, field: Field) -> &str {
        match field {
            Field::Host => &self.host,
            Field::Username => &self.username,
            Field::Password => &self.password,
            Field::Url => &self.url,
        }
    }

    pub fn field_text_mut(&mut self, field: Field) -> &mut String {
        match field {
            Field::Host => &mut self.host,
            Field::Username => &mut self.username,
            Field::Password => &mut self.password,
            Field::Url => &mut self.url,
        }
    }

    /// Advance the model by one action; may emit [`Connect`].
    pub fn input(&mut self, action: Action) -> Option<Connect> {
        self.input_form(action)
    }

    fn input_form(&mut self, action: Action) -> Option<Connect> {
        let fields = self.fields();
        // Vertical row list: Kind, fields, Connect.
        let row_index = |row: Row| match row {
            Row::Kind => 0,
            Row::Field(i) => 1 + i.min(fields.len().saturating_sub(1)),
            Row::Connect => 1 + fields.len(),
        };
        let set_row = |m: &mut Self, index: usize| {
            m.row = if index == 0 {
                Row::Kind
            } else if index <= fields.len() {
                Row::Field(index - 1)
            } else {
                Row::Connect
            };
            if let Row::Field(i) = m.row {
                m.focus_field(fields[i]);
            }
        };

        match action {
            Action::Up | Action::Down => {
                let forward = action == Action::Down;
                let rows = 1 + fields.len() + 1;
                let current = row_index(self.row);
                if forward {
                    if current + 1 >= rows {
                        self.row = Row::Kind;
                    } else {
                        set_row(self, current + 1);
                    }
                } else if current > 0 {
                    set_row(self, current - 1);
                } else {
                    self.row = Row::Connect;
                }
            }
            Action::Left | Action::Right | Action::Select if self.row == Row::Kind => {
                // Toggle stays on the Kind row so Left/Right can flip back.
                self.xtream = !self.xtream;
                self.active = self.fields()[0];
            }
            Action::Left | Action::Right => {
                if let Row::Field(i) = self.row {
                    let len = fields.len();
                    let next = if action == Action::Right {
                        (i + 1) % len
                    } else {
                        (i + len - 1) % len
                    };
                    self.focus_field(fields[next]);
                }
            }
            Action::Select if self.row == Row::Connect => return self.connect(),
            Action::Select => {
                if let Row::Field(i) = self.row {
                    self.focus_field(fields[i]);
                }
            }
            Action::Back => {
                self.row = Row::Kind;
            }
            _ => {}
        }
        None
    }

    /// Validate the form and build the [`Source`]; errors land in `message`.
    pub fn connect(&mut self) -> Option<Connect> {
        let kind = if self.xtream {
            if self.host.trim().is_empty() || self.username.is_empty() || self.password.is_empty() {
                self.message = Some("Fill host, username and password.".into());
                return None;
            }
            SourceKind::Xtream {
                host: normalize_host(&self.host),
                username: self.username.trim().to_owned(),
                password: self.password.clone(),
            }
        } else {
            if self.url.trim().is_empty() {
                self.message = Some("Fill the M3U URL.".into());
                return None;
            }
            SourceKind::M3u {
                url: self.url.trim().to_owned(),
            }
        };
        let name = match &kind {
            SourceKind::Xtream { host, .. } => host
                .trim_start_matches("http://")
                .trim_start_matches("https://")
                .trim_end_matches('/')
                .to_owned(),
            SourceKind::M3u { url } => url.clone(),
        };
        let source = Source {
            id: Self::source_id(&kind),
            name,
            kind,
            stream_format: self.stream_format,
        };
        self.message = None;
        Some(Connect(source))
    }
}

/// Prepend `http://` when the scheme is missing.
fn normalize_host(host: &str) -> String {
    let h = host.trim();
    if h.starts_with("http://") || h.starts_with("https://") {
        h.to_owned()
    } else {
        format!("http://{h}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled() -> SetupModel {
        SetupModel {
            xtream: true,
            host: "http://tv.example.com".into(),
            username: "u".into(),
            password: "p".into(),
            ..Default::default()
        }
    }

    #[test]
    fn form_navigation_walks_rows_and_fields() {
        let mut m = SetupModel::default();
        assert_eq!(m.row, Row::Kind);
        m.input(Action::Down);
        assert_eq!(m.row, Row::Field(0));
        assert_eq!(m.active, Field::Host);
        m.input(Action::Right);
        assert_eq!(m.row, Row::Field(1));
        assert_eq!(m.active, Field::Username);
        m.input(Action::Left);
        assert_eq!(m.row, Row::Field(0));
        m.input(Action::Down);
        assert_eq!(m.row, Row::Field(1));
        m.input(Action::Up);
        assert_eq!(m.row, Row::Field(0));
        m.input(Action::Up);
        assert_eq!(m.row, Row::Kind);
        m.input(Action::Up); // wraps to Connect
        assert_eq!(m.row, Row::Connect);
    }

    #[test]
    fn form_navigation_wraps_without_opening_custom_keyboard() {
        let mut m = filled();
        m.row = Row::Connect;
        m.input(Action::Down);
        assert_eq!(m.row, Row::Kind);
        m.input(Action::Up);
        assert_eq!(m.row, Row::Connect);
    }

    #[test]
    fn kind_toggle_switches_fields() {
        let mut m = SetupModel::default();
        assert!(m.xtream);
        m.input(Action::Right); // on Kind row
        assert!(!m.xtream);
        assert_eq!(m.fields(), &[Field::Url]);
        m.input(Action::Right); // toggle back
        assert!(m.xtream);
        assert_eq!(m.fields().len(), 3);
    }

    #[test]
    fn connect_validates_and_normalizes() {
        let mut m = SetupModel::default();
        assert!(m.connect().is_none());
        assert!(m.message.as_deref().unwrap().contains("host"));

        m.host = "tv.example.com".into();
        m.username = "u".into();
        m.password = "p".into();
        let Connect(s) = m.connect().unwrap();
        assert_eq!(
            s.kind,
            SourceKind::Xtream {
                host: "http://tv.example.com".into(),
                username: "u".into(),
                password: "p".into(),
            }
        );
        assert_eq!(s.id, "xtream:http://tv.example.com/u");
        assert_eq!(s.name, "tv.example.com");
        assert_eq!(s.stream_format, StreamFormat::Ts);
    }

    #[test]
    fn connect_m3u_requires_url() {
        let mut m = SetupModel {
            xtream: false,
            ..Default::default()
        };
        assert!(m.connect().is_none());
        let mut m = SetupModel {
            xtream: false,
            url: "http://x/list.m3u".into(),
            ..Default::default()
        };
        let Connect(s) = m.connect().unwrap();
        assert_eq!(s.id, "m3u:http://x/list.m3u");
    }

    #[test]
    fn select_on_connect_emits_source() {
        let mut m = filled();
        m.row = Row::Connect;
        assert!(m.input(Action::Select).is_some());
    }

    #[test]
    fn pointer_focus_and_controller_navigation_share_field() {
        let mut m = filled();
        m.focus_field(Field::Username);
        assert_eq!(m.row, Row::Field(1));
        assert_eq!(m.active, Field::Username);
        m.input(Action::Right);
        assert_eq!(m.active, Field::Password);
        assert_eq!(m.focus_request, None);
        m.input(Action::Back);
        assert_eq!(m.password, "p");
        assert_eq!(m.row, Row::Kind);
    }
}
