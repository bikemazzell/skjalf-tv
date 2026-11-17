//! `{user}`/`{pass}` stream-URL templating.
//!
//! Playlists are cached with literal placeholders; credentials are only
//! substituted at play time (never persisted in the cache).

/// Percent-encode a URL path/query component (RFC 3986 unreserved kept).
pub fn encode_component(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for b in v.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Replace `{user}`/`{pass}` placeholders with percent-encoded credentials.
pub fn render(url: &str, username: &str, password: &str) -> String {
    url.replace("{user}", &encode_component(username))
        .replace("{pass}", &encode_component(password))
}
