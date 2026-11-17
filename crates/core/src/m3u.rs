//! M3U playlist parsing: `#EXTM3U` + `#EXTINF` extended attribute form.

use crate::model::{Channel, Group, Playlist, Source};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("invalid M3U playlist: {0}")]
pub struct M3uError(String);

/// Parse an M3U playlist body into a [`Playlist`].
///
/// - `tvg-id` becomes the channel id (falls back to the URL, then name+index),
/// - `tvg-chno` → `num`, `tvg-logo` → `logo`, `group-title` → group id/name,
/// - channels without a group land in a synthetic `Ungrouped` group
///   (created on first use, like any other group, so they stay reachable).
pub fn parse(source: Source, text: &str) -> Result<Playlist, M3uError> {
    if !text.trim_start().starts_with("#EXTM3U") {
        return Err(M3uError("missing #EXTM3U header".into()));
    }

    let mut groups: Vec<Group> = Vec::new();
    let mut group_index: HashMap<String, usize> = HashMap::new();
    let mut channels: Vec<Channel> = Vec::new();
    let mut pending: Option<(Vec<(String, String)>, String)> = None;
    let mut auto_id = 0u32;
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let (attrs, name) = split_extinf(rest);
            pending = Some((attrs, name.trim().to_owned()));
        } else if line.starts_with('#') {
            // Other directives (#EXTVLCOPT, #KODIPROP, …) are ignored.
        } else if let Some((attrs, name)) = pending.take() {
            let url = line.to_owned();
            let attr = |k: &str| {
                attrs
                    .iter()
                    .find(|(key, _)| key == k)
                    .map(|(_, v)| v.trim().to_owned())
            };
            let group_name = attr("group-title").unwrap_or_else(|| "Ungrouped".into());
            let group_id = {
                let idx = *group_index.entry(group_name.clone()).or_insert_with(|| {
                    groups.push(Group {
                        id: group_name.clone(),
                        name: group_name.clone(),
                    });
                    groups.len() - 1
                });
                groups[idx].id.clone()
            };
            auto_id += 1;
            let base_id = attr("tvg-id")
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| {
                    if !url.is_empty() {
                        url.clone()
                    } else {
                        format!("{}#{auto_id}", name)
                    }
                });
            // Duplicate tvg-ids would collapse channels (`find` returns the
            // first); suffix copies instead so every channel stays reachable.
            let mut id = base_id.clone();
            let mut copy = 1;
            while !seen_ids.insert(id.clone()) {
                copy += 1;
                id = format!("{base_id}#{copy}");
            }
            channels.push(Channel {
                id,
                name: if name.is_empty() {
                    format!("Channel {auto_id}")
                } else {
                    name
                },
                num: attr("tvg-chno").and_then(|n| n.parse().ok()),
                logo: attr("tvg-logo").filter(|l| !l.is_empty()),
                group_id,
                url,
            });
        }
        // A URL with no preceding #EXTINF is skipped.
    }

    let mut used_numbers: std::collections::HashSet<u32> =
        channels.iter().filter_map(|channel| channel.num).collect();
    let mut next_number = 1u32;
    for channel in &mut channels {
        if channel.num.is_none() {
            while used_numbers.contains(&next_number) {
                next_number = next_number.checked_add(1).unwrap_or(1);
            }
            channel.num = Some(next_number);
            used_numbers.insert(next_number);
        }
    }

    Ok(Playlist {
        source,
        groups,
        channels,
    })
}

/// Split `"-1 tvg-id=\"x\" …,Title"` into attributes and title, honouring
/// quotes so attribute values may contain commas.
fn split_extinf(rest: &str) -> (Vec<(String, String)>, String) {
    let mut in_quotes = false;
    let comma = rest
        .char_indices()
        .find(|&(i, c)| {
            if c == '"' {
                in_quotes = !in_quotes;
            }
            c == ',' && !in_quotes && i > 0
        })
        .map(|(i, _)| i);
    let (head, name) = match comma {
        Some(i) => (&rest[..i], rest[i + 1..].to_owned()),
        None => (rest, String::new()),
    };
    (parse_attrs(head), name)
}

/// Parse `key="value"` pairs from the `#EXTINF` head (before the title comma).
///
/// Scanning is char-boundary safe: indices only ever come from `find` /
/// `char_indices`, so multi-byte UTF-8 in or around attributes cannot panic.
fn parse_attrs(head: &str) -> Vec<(String, String)> {
    let mut attrs = Vec::new();
    let mut rest = head;
    while let Some(eq) = rest.find("=\"") {
        let before = &rest[..eq];
        // The key is the alphanumeric/`-` run ending right before `="`.
        let key_start = before
            .char_indices()
            .filter(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '-'))
            .map(|(i, c)| i + c.len_utf8())
            .next_back()
            .unwrap_or(0);
        let key = &before[key_start..];
        let after = &rest[eq + 2..];
        match after.find('"') {
            Some(end) if !key.is_empty() => {
                attrs.push((key.to_owned(), after[..end].to_owned()));
                rest = &after[end + 1..];
            }
            // Unterminated quote or stray `="`: nothing more to collect.
            _ => break,
        }
    }
    attrs
}
