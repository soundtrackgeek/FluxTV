//! Plain data types shared by every platform.
//!
//! These are deliberately simple (strings, integers, options, vectors) so they can be
//! serialized to the Tauri frontend today and exported through UniFFI to Swift later.

use serde::{Deserialize, Serialize};

/// What a playable item is. Xtream servers expose these as separate catalogs;
/// M3U playlists mix them and we infer the kind from the stream URL.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ContentKind {
    Live,
    Movie,
    Series,
}

impl ContentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ContentKind::Live => "live",
            ContentKind::Movie => "movie",
            ContentKind::Series => "series",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "live" => Some(ContentKind::Live),
            "movie" => Some(ContentKind::Movie),
            "series" => Some(ContentKind::Series),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    /// Xtream Codes compatible panel (`player_api.php`).
    Xtream,
    /// Plain M3U / M3U8 playlist URL with an optional XMLTV guide URL.
    M3u,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::Xtream => "xtream",
            SourceKind::M3u => "m3u",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "xtream" => Some(SourceKind::Xtream),
            "m3u" => Some(SourceKind::M3u),
            _ => None,
        }
    }
}

/// A configured IPTV provider. The password never leaves the Rust side.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub id: i64,
    pub name: String,
    pub kind: SourceKind,
    /// Xtream: server base URL (`http://host:port`). M3U: playlist URL.
    pub url: String,
    pub username: Option<String>,
    #[serde(skip_serializing)]
    pub password: Option<String>,
    /// Explicit XMLTV URL. For Xtream sources this defaults to `xmltv.php`.
    pub epg_url: Option<String>,
    pub created_at: i64,
    pub catalog_updated_at: Option<i64>,
    pub epg_updated_at: Option<i64>,
}

/// Input for creating a source.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSource {
    pub name: String,
    pub kind: SourceKind,
    pub url: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub epg_url: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    pub id: String,
    pub name: String,
    pub kind: ContentKind,
    pub channel_count: i64,
}

/// A live channel, movie or series entry.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    /// Stable identifier within a source (Xtream stream id, or derived from the M3U entry).
    pub id: String,
    pub kind: ContentKind,
    pub num: i64,
    pub name: String,
    pub logo: Option<String>,
    pub category_id: String,
    /// XMLTV channel id used to look up programmes (explicit or matched by name).
    pub epg_id: Option<String>,
    /// Number of days of catch-up/archive available (0 = none).
    pub catchup_days: i64,
    /// File extension for movies/series (`mp4`, `mkv`, ...).
    pub container: Option<String>,
    pub favorite: bool,
    /// Direct stream URL for M3U sources. Kept server-side because it may contain credentials.
    #[serde(skip)]
    pub url: Option<String>,
}

/// One programme (show) from the electronic programme guide.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Programme {
    /// XMLTV channel id.
    pub channel: String,
    /// Unix timestamps (seconds, UTC).
    pub start: i64,
    pub stop: i64,
    pub title: String,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub icon: Option<String>,
    pub episode: Option<String>,
}

/// A channel declared in the XMLTV document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EpgChannel {
    pub id: String,
    pub display_names: Vec<String>,
    pub icon: Option<String>,
}

/// The programme currently airing on a channel and the one after it.
#[derive(Clone, Debug, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NowNext {
    pub epg_id: String,
    pub now: Option<Programme>,
    pub next: Option<Programme>,
}

/// Programmes for one channel over a time range (used by the guide grid).
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GuideRow {
    pub epg_id: String,
    pub programmes: Vec<Programme>,
}

/// Subscription details reported by an Xtream server.
#[derive(Clone, Debug, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub status: String,
    pub expires_at: Option<i64>,
    pub is_trial: bool,
    pub active_connections: i64,
    pub max_connections: i64,
    pub allowed_formats: Vec<String>,
    pub server_timezone: Option<String>,
    pub message: Option<String>,
}

/// Progress notification for long-running refresh jobs.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub source_id: i64,
    /// `catalog` or `epg`.
    pub stage: String,
    pub message: String,
    /// 0.0 ..= 1.0 when known.
    pub fraction: Option<f32>,
    pub done: bool,
}

/// Summary returned after a refresh.
#[derive(Clone, Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RefreshReport {
    pub categories: usize,
    pub channels: usize,
    pub epg_channels: usize,
    pub programmes: usize,
    pub epg_matched_by_name: usize,
}
