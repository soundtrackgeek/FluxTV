//! Records, enums and errors exported to Swift, with conversions from the core models.
//!
//! The core types stay free of UniFFI; these mirrors define the Swift-facing API and leave
//! out anything that could carry credentials (stream URLs, stored passwords).

use fluxtv_core as core;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum ContentKind {
    Live,
    Movie,
    Series,
}

impl From<ContentKind> for core::ContentKind {
    fn from(k: ContentKind) -> Self {
        match k {
            ContentKind::Live => core::ContentKind::Live,
            ContentKind::Movie => core::ContentKind::Movie,
            ContentKind::Series => core::ContentKind::Series,
        }
    }
}

impl From<core::ContentKind> for ContentKind {
    fn from(k: core::ContentKind) -> Self {
        match k {
            core::ContentKind::Live => ContentKind::Live,
            core::ContentKind::Movie => ContentKind::Movie,
            core::ContentKind::Series => ContentKind::Series,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum SourceKind {
    /// Xtream Codes panel: server + username + password.
    Xtream,
    /// M3U / M3U8 playlist URL with an optional XMLTV guide URL.
    M3u,
}

impl From<SourceKind> for core::SourceKind {
    fn from(k: SourceKind) -> Self {
        match k {
            SourceKind::Xtream => core::SourceKind::Xtream,
            SourceKind::M3u => core::SourceKind::M3u,
        }
    }
}

impl From<core::SourceKind> for SourceKind {
    fn from(k: core::SourceKind) -> Self {
        match k {
            core::SourceKind::Xtream => SourceKind::Xtream,
            core::SourceKind::M3u => SourceKind::M3u,
        }
    }
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum FluxError {
    #[error("{message}")]
    Network { message: String },
    #[error("{message}")]
    Auth { message: String },
    #[error("{message}")]
    NotFound { message: String },
    #[error("{message}")]
    Invalid { message: String },
    #[error("{message}")]
    Data { message: String },
    #[error("{message}")]
    Internal { message: String },
}

impl From<core::Error> for FluxError {
    fn from(e: core::Error) -> Self {
        let message = e.to_string();
        match e {
            core::Error::Http(_) | core::Error::Status(_) => FluxError::Network { message },
            core::Error::Auth(m) => FluxError::Auth { message: m },
            core::Error::NotFound(_) => FluxError::NotFound { message },
            core::Error::Invalid(m) => FluxError::Invalid { message: m },
            core::Error::Parse(_) => FluxError::Data { message },
            core::Error::Db(_) | core::Error::Io(_) => FluxError::Internal { message },
        }
    }
}

/// Account details typed by the user.
#[derive(Clone, Debug, uniffi::Record)]
pub struct AccountInput {
    pub name: String,
    pub kind: SourceKind,
    /// Xtream server (`http://host:port`) or playlist URL. A pasted Xtream `get.php` link works too.
    pub url: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub epg_url: Option<String>,
}

impl AccountInput {
    pub(crate) fn into_core(self) -> core::NewSource {
        core::NewSource {
            name: self.name,
            kind: self.kind.into(),
            url: self.url,
            username: self.username,
            password: self.password,
            epg_url: self.epg_url,
        }
    }
}

/// Normalized account details after validation, ready to be saved to the Keychain.
#[derive(Clone, Debug, uniffi::Record)]
pub struct ValidatedAccount {
    pub name: String,
    pub kind: SourceKind,
    pub url: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub epg_url: Option<String>,
    pub info: Option<AccountInfo>,
}

/// An account as stored in the Keychain. `key` is a stable identifier chosen by the app.
#[derive(Clone, Debug, uniffi::Record)]
pub struct SavedAccount {
    pub key: String,
    pub name: String,
    pub kind: SourceKind,
    pub url: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub epg_url: Option<String>,
}

/// A configured source (no password).
#[derive(Clone, Debug, uniffi::Record)]
pub struct SourceInfo {
    pub id: i64,
    /// Keychain account key.
    pub key: String,
    pub name: String,
    pub kind: SourceKind,
    pub url: String,
    pub username: Option<String>,
    pub epg_url: Option<String>,
    pub catalog_updated_at: Option<i64>,
    pub guide_updated_at: Option<i64>,
    pub live_count: i64,
    pub movie_count: i64,
}

impl SourceInfo {
    pub(crate) fn from_core(s: core::Source, key: String, live_count: i64, movie_count: i64) -> Self {
        Self {
            id: s.id,
            key,
            name: s.name,
            kind: s.kind.into(),
            url: s.url,
            username: s.username,
            epg_url: s.epg_url,
            catalog_updated_at: s.catalog_updated_at,
            guide_updated_at: s.epg_updated_at,
            live_count,
            movie_count,
        }
    }
}

#[derive(Clone, Debug, uniffi::Record)]
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

impl From<core::AccountInfo> for AccountInfo {
    fn from(a: core::AccountInfo) -> Self {
        Self {
            status: a.status,
            expires_at: a.expires_at,
            is_trial: a.is_trial,
            active_connections: a.active_connections,
            max_connections: a.max_connections,
            allowed_formats: a.allowed_formats,
            server_timezone: a.server_timezone,
            message: a.message,
        }
    }
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct Category {
    pub id: String,
    pub name: String,
    pub kind: ContentKind,
    pub channel_count: i64,
}

impl From<core::Category> for Category {
    fn from(c: core::Category) -> Self {
        Self { id: c.id, name: c.name, kind: c.kind.into(), channel_count: c.channel_count }
    }
}

/// A live channel or movie. The stream URL is deliberately absent (it embeds credentials).
#[derive(Clone, Debug, uniffi::Record)]
pub struct Channel {
    pub id: String,
    pub kind: ContentKind,
    pub num: i64,
    pub name: String,
    pub logo: Option<String>,
    pub category_id: String,
    pub epg_id: Option<String>,
    pub catchup_days: i64,
    pub container: Option<String>,
    pub favorite: bool,
}

impl From<core::Channel> for Channel {
    fn from(c: core::Channel) -> Self {
        Self {
            id: c.id,
            kind: c.kind.into(),
            num: c.num,
            name: c.name,
            logo: c.logo,
            category_id: c.category_id,
            epg_id: c.epg_id,
            catchup_days: c.catchup_days,
            container: c.container,
            favorite: c.favorite,
        }
    }
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct Programme {
    pub channel: String,
    /// Unix seconds.
    pub start: i64,
    pub stop: i64,
    pub title: String,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub icon: Option<String>,
    pub episode: Option<String>,
}

impl From<core::Programme> for Programme {
    fn from(p: core::Programme) -> Self {
        Self {
            channel: p.channel,
            start: p.start,
            stop: p.stop,
            title: p.title,
            subtitle: p.subtitle,
            description: p.description,
            category: p.category,
            icon: p.icon,
            episode: p.episode,
        }
    }
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct NowNext {
    pub epg_id: String,
    pub now: Option<Programme>,
    pub next: Option<Programme>,
}

impl From<core::NowNext> for NowNext {
    fn from(n: core::NowNext) -> Self {
        Self { epg_id: n.epg_id, now: n.now.map(Into::into), next: n.next.map(Into::into) }
    }
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct GuideRow {
    pub epg_id: String,
    pub programmes: Vec<Programme>,
}

impl From<core::GuideRow> for GuideRow {
    fn from(g: core::GuideRow) -> Self {
        Self { epg_id: g.epg_id, programmes: g.programmes.into_iter().map(Into::into).collect() }
    }
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct Staleness {
    pub catalog: bool,
    pub guide: bool,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct RefreshProgress {
    /// `catalog`, `epg` or `done`.
    pub stage: String,
    pub message: String,
    pub fraction: Option<f32>,
    pub done: bool,
}

#[derive(Clone, Debug, Default, uniffi::Record)]
pub struct RefreshOutcome {
    pub already_running: bool,
    pub catalog_refreshed: bool,
    pub guide_refreshed: bool,
    pub channels: u64,
    pub programmes: u64,
    /// The guide failed but the catalog may still have been refreshed.
    pub guide_error: Option<String>,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct Playback {
    /// URL to hand to AVPlayer (usually a local `http://127.0.0.1` URL).
    pub url: String,
    pub live: bool,
    pub channel: Channel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum StreamState {
    Idle,
    Connecting,
    Playing,
    Reconnecting,
    Failed,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct PlaybackStatus {
    pub state: StreamState,
    pub video_codec: Option<String>,
    pub audio_codecs: Vec<String>,
    /// Codecs AVPlayer cannot decode (e.g. MPEG-2 video), if detected.
    pub unsupported: Vec<String>,
    pub message: Option<String>,
    pub reconnects: u32,
    /// Average upstream bitrate over the buffered segments, in bits per second.
    pub bitrate: u64,
}
