//! M3U / M3U Plus playlist parsing.
//!
//! ```text
//! #EXTM3U url-tvg="http://example.com/epg.xml"
//! #EXTINF:-1 tvg-id="nrk1.no" tvg-logo="http://.../nrk1.png" group-title="Norway",NRK1 HD
//! http://example.com/live/user/pass/45094.ts
//! ```

use std::collections::HashMap;

use crate::models::{Category, Channel, ContentKind};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct M3uEntry {
    pub title: String,
    pub url: String,
    /// All `key="value"` attributes from the `#EXTINF` line, lowercased keys.
    pub attrs: HashMap<String, String>,
    /// Group from `#EXTGRP:` when `group-title` is missing.
    pub group: Option<String>,
}

impl M3uEntry {
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs.get(key).map(String::as_str).filter(|v| !v.trim().is_empty())
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct M3uPlaylist {
    /// Guide URL advertised in the header (`url-tvg` / `x-tvg-url`).
    pub epg_url: Option<String>,
    pub entries: Vec<M3uEntry>,
}

/// Parse `key="value"` pairs. Values may contain spaces and commas.
fn parse_attrs(s: &str, out: &mut HashMap<String, String>) {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let key_start = i;
        while i < bytes.len() && bytes[i] != b'=' && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let key = &s[key_start..i];
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1; // '='
        let value = if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
            let quote = bytes[i];
            i += 1;
            let v_start = i;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            let v = &s[v_start..i];
            i += 1;
            v
        } else {
            let v_start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            &s[v_start..i]
        };
        if !key.is_empty() {
            out.insert(key.to_ascii_lowercase(), value.to_string());
        }
    }
}

/// Split `#EXTINF:` payload into (attribute section, title) at the first comma outside quotes.
fn split_extinf(payload: &str) -> (&str, &str) {
    let mut in_quotes = false;
    for (i, c) in payload.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => return (&payload[..i], payload[i + 1..].trim()),
            _ => {}
        }
    }
    (payload, "")
}

pub fn parse(text: &str) -> M3uPlaylist {
    let mut playlist = M3uPlaylist::default();
    let mut current: Option<M3uEntry> = None;

    for raw in text.lines() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        if line.is_empty() {
            continue;
        }
        if let Some(header) = line.strip_prefix("#EXTM3U") {
            let mut attrs = HashMap::new();
            parse_attrs(header, &mut attrs);
            playlist.epg_url = ["url-tvg", "x-tvg-url", "tvg-url"]
                .iter()
                .find_map(|k| attrs.get(*k))
                // Some providers list several comma-separated guides; take the first.
                .and_then(|v| v.split(',').next())
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty());
        } else if let Some(payload) = line.strip_prefix("#EXTINF:") {
            let (head, title) = split_extinf(payload);
            let mut entry = M3uEntry { title: title.to_string(), ..Default::default() };
            // `head` starts with the duration (`-1`), followed by attributes.
            let attr_part = head.trim_start().trim_start_matches(|c: char| c == '-' || c == '.' || c.is_ascii_digit());
            parse_attrs(attr_part, &mut entry.attrs);
            current = Some(entry);
        } else if let Some(group) = line.strip_prefix("#EXTGRP:") {
            if let Some(e) = current.as_mut() {
                e.group = Some(group.trim().to_string());
            }
        } else if line.starts_with('#') {
            // #EXTVLCOPT, #KODIPROP, comments: ignored.
        } else if let Some(mut entry) = current.take() {
            entry.url = line.to_string();
            if entry.title.is_empty() {
                entry.title = entry.attr("tvg-name").unwrap_or(&entry.url).to_string();
            }
            playlist.entries.push(entry);
        }
    }
    playlist
}

/// Infer the content kind from an Xtream-style URL path or file extension.
pub fn kind_from_url(url: &str) -> ContentKind {
    let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    if path.contains("/movie/") {
        ContentKind::Movie
    } else if path.contains("/series/") {
        ContentKind::Series
    } else if [".mp4", ".mkv", ".avi", ".mov", ".m4v", ".wmv"].iter().any(|ext| path.ends_with(ext)) {
        ContentKind::Movie
    } else {
        ContentKind::Live
    }
}

/// A stable id for an entry: the numeric stream id for Xtream-style URLs, otherwise a hash of the URL.
pub fn stable_id(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    if let Some(last) = path.rsplit('/').next() {
        let stem = last.split('.').next().unwrap_or(last);
        if !stem.is_empty() && stem.len() <= 12 && stem.bytes().all(|b| b.is_ascii_digit()) {
            return stem.to_string();
        }
    }
    // FNV-1a 64-bit: stable across runs and platforms (unlike std's randomized hasher).
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in url.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("u{h:016x}")
}

fn extension(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let last = path.rsplit('/').next()?;
    let (_, ext) = last.rsplit_once('.')?;
    (!ext.is_empty() && ext.len() <= 5).then(|| ext.to_ascii_lowercase())
}

/// Convert a parsed playlist into categories and channels.
pub fn into_catalog(playlist: &M3uPlaylist) -> (Vec<Category>, Vec<Channel>) {
    let mut categories: Vec<Category> = Vec::new();
    let mut cat_index: HashMap<(ContentKind, String), usize> = HashMap::new();
    let mut channels = Vec::with_capacity(playlist.entries.len());
    let mut seen = std::collections::HashSet::new();

    for (i, e) in playlist.entries.iter().enumerate() {
        let kind = kind_from_url(&e.url);
        let group = e
            .attr("group-title")
            .map(str::to_string)
            .or_else(|| e.group.clone())
            .unwrap_or_else(|| "Uncategorized".to_string());
        let idx = *cat_index.entry((kind, group.clone())).or_insert_with(|| {
            categories.push(Category { id: group.clone(), name: group.clone(), kind, channel_count: 0 });
            categories.len() - 1
        });
        categories[idx].channel_count += 1;

        let mut id = stable_id(&e.url);
        if !seen.insert((kind, id.clone())) {
            // Duplicate URL in the same playlist: disambiguate by position.
            id = format!("{id}-{i}");
            seen.insert((kind, id.clone()));
        }
        let catchup_days = e
            .attr("catchup-days")
            .or_else(|| e.attr("tvg-rec"))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
        channels.push(Channel {
            id,
            kind,
            num: e.attr("tvg-chno").and_then(|v| v.parse().ok()).unwrap_or(i as i64 + 1),
            name: e.title.clone(),
            logo: e.attr("tvg-logo").map(str::to_string),
            category_id: group,
            epg_id: e.attr("tvg-id").map(str::to_string),
            catchup_days,
            container: if kind == ContentKind::Live { None } else { extension(&e.url) },
            favorite: false,
            url: Some(e.url.clone()),
        });
    }
    (categories, channels)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "#EXTM3U url-tvg=\"http://epg.example/guide.xml.gz\"\n\
#EXTINF:0 CUID=\"4611697\" tvg-name=\"WORLD CUP ᴴᴰ\" tvg-logo=\"http://x/l.png\" group-title=\"WORLD CUP, 2026\",WORLD CUP, REPLAY ᴴᴰ\n\
http://line.example/live/u/p/2085222.ts\n\
\n\
#EXTINF:-1 tvg-id=\"nrk1.no\" group-title=\"NORWAY\" catchup-days=\"7\",NOR| NRK1 HD\n\
#EXTVLCOPT:http-user-agent=Foo\n\
http://line.example/live/u/p/45094.ts\n\
#EXTINF:-1 group-title=\"Movies\",Some Film (2025)\n\
http://line.example/movie/u/p/77.mkv\n\
#EXTINF:-1,No Group\n\
#EXTGRP:Misc\n\
https://cdn.example/stream/index.m3u8?token=abc\n";

    #[test]
    fn parses_entries_and_header() {
        let p = parse(SAMPLE);
        assert_eq!(p.epg_url.as_deref(), Some("http://epg.example/guide.xml.gz"));
        assert_eq!(p.entries.len(), 4);
        let first = &p.entries[0];
        assert_eq!(first.title, "WORLD CUP, REPLAY ᴴᴰ");
        assert_eq!(first.attr("group-title"), Some("WORLD CUP, 2026"));
        assert_eq!(first.attr("cuid"), Some("4611697"));
        assert_eq!(p.entries[1].attr("tvg-id"), Some("nrk1.no"));
        assert_eq!(p.entries[3].group.as_deref(), Some("Misc"));
    }

    #[test]
    fn builds_catalog() {
        let (cats, chans) = into_catalog(&parse(SAMPLE));
        assert_eq!(cats.len(), 4);
        assert_eq!(chans[0].id, "2085222");
        assert_eq!(chans[1].epg_id.as_deref(), Some("nrk1.no"));
        assert_eq!(chans[1].catchup_days, 7);
        assert_eq!(chans[2].kind, ContentKind::Movie);
        assert_eq!(chans[2].container.as_deref(), Some("mkv"));
        assert_eq!(chans[3].category_id, "Misc");
        assert!(chans[3].id.starts_with('u'));
        assert_eq!(chans[3].kind, ContentKind::Live);
    }
}
