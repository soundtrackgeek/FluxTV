//! `FluxCore`: the façade every platform talks to.
//!
//! It owns the database and HTTP client and implements the workflows that span both:
//! adding sources, refreshing catalogs and guides, and building playback URLs.

use std::sync::Arc;
use std::time::Duration;

use crate::error::{Error, Result};
use crate::m3u;
use crate::models::*;
use crate::store::Store;
use crate::time::now;
use crate::xmltv::{self, XmltvItem};
use crate::xtream::{XtreamClient, XtreamCredentials};

/// Catalogs are refreshed automatically when older than this.
pub const CATALOG_MAX_AGE_SECS: i64 = 24 * 3600;
/// Guides are refreshed automatically when older than this.
pub const EPG_MAX_AGE_SECS: i64 = 12 * 3600;
/// Programmes that ended more than this long ago are not imported.
const EPG_KEEP_PAST_SECS: i64 = 24 * 3600;

pub const USER_AGENT: &str = concat!("FluxTV/", env!("CARGO_PKG_VERSION"));

pub type ProgressFn = Arc<dyn Fn(Progress) + Send + Sync>;

/// Build the shared HTTP client used for API calls, playlists and guides.
pub fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(90))
        .build()
        .map_err(Error::from)
}

pub struct FluxCore {
    store: Arc<Store>,
    http: reqwest::Client,
}

impl FluxCore {
    pub fn open(db_path: impl AsRef<std::path::Path>) -> Result<Self> {
        Ok(Self { store: Arc::new(Store::open(db_path)?), http: http_client()? })
    }

    pub fn with_store(store: Store) -> Result<Self> {
        Ok(Self { store: Arc::new(store), http: http_client()? })
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    fn xtream(&self, source: &Source) -> Result<XtreamClient> {
        let creds = XtreamCredentials::new(
            &source.url,
            source.username.as_deref().unwrap_or_default(),
            source.password.as_deref().unwrap_or_default(),
        )?;
        Ok(XtreamClient::new(self.http.clone(), creds))
    }

    /// Validate and normalize user input into a source that can be stored.
    ///
    /// Pasting an Xtream `get.php?username=...` link as an M3U source converts it to an Xtream
    /// source automatically, which unlocks the richer API (categories, VOD, account info).
    pub async fn prepare_source(&self, mut s: NewSource) -> Result<(NewSource, Option<AccountInfo>)> {
        s.name = s.name.trim().to_string();
        s.url = s.url.trim().to_string();
        s.epg_url = s.epg_url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
        s.username = s.username.map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
        s.password = s.password.filter(|p| !p.is_empty());

        if s.kind == SourceKind::M3u
            && let Some(creds) = XtreamCredentials::from_url(&s.url) {
                s.kind = SourceKind::Xtream;
                s.url = creds.base;
                s.username = Some(creds.username);
                s.password = Some(creds.password);
            }
        match s.kind {
            SourceKind::Xtream => {
                if let Some(creds) = XtreamCredentials::from_url(&s.url) {
                    s.url = creds.base;
                    s.username = s.username.or(Some(creds.username));
                    s.password = s.password.or(Some(creds.password));
                } else {
                    s.url = crate::xtream::normalize_base(&s.url)?;
                }
                let (Some(user), Some(pass)) = (&s.username, &s.password) else {
                    return Err(Error::Invalid("username and password are required".into()));
                };
                let client = XtreamClient::new(self.http.clone(), XtreamCredentials::new(&s.url, user, pass)?);
                let account = client.account().await?;
                if s.name.is_empty() {
                    s.name = url::Url::parse(&s.url)
                        .ok()
                        .and_then(|u| u.host_str().map(str::to_string))
                        .unwrap_or_else(|| "IPTV".into());
                }
                Ok((s, Some(account)))
            }
            SourceKind::M3u => {
                let url = url::Url::parse(&s.url).map_err(|e| Error::Invalid(format!("playlist URL: {e}")))?;
                if s.name.is_empty() {
                    s.name = url.host_str().unwrap_or("Playlist").to_string();
                }
                Ok((s, None))
            }
        }
    }

    pub async fn add_source(&self, s: NewSource) -> Result<Source> {
        let (s, _) = self.prepare_source(s).await?;
        self.store.insert_source(&s)
    }

    pub async fn update_source(&self, id: i64, s: NewSource) -> Result<Source> {
        let existing = self.store.source(id)?;
        let mut s = s;
        if s.password.as_deref().is_none_or(str::is_empty) {
            s.password = existing.password.clone();
        }
        let (s, _) = self.prepare_source(s).await?;
        self.store.update_source(id, &s)
    }

    pub async fn account_info(&self, source_id: i64) -> Result<Option<AccountInfo>> {
        let source = self.store.source(source_id)?;
        match source.kind {
            SourceKind::Xtream => Ok(Some(self.xtream(&source)?.account().await?)),
            SourceKind::M3u => Ok(None),
        }
    }

    /// The upstream URL to play. May embed credentials: never hand it to untrusted code.
    pub fn stream_url(&self, source_id: i64, kind: ContentKind, channel_id: &str) -> Result<String> {
        let source = self.store.source(source_id)?;
        let channel = self.store.channel(source_id, kind, channel_id)?;
        match source.kind {
            SourceKind::Xtream => {
                Ok(self.xtream(&source)?.credentials().stream_url(kind, &channel.id, channel.container.as_deref()))
            }
            SourceKind::M3u => channel.url.ok_or_else(|| Error::NotFound(format!("stream URL for {channel_id}"))),
        }
    }

    pub fn epg_url(&self, source: &Source) -> Result<Option<String>> {
        if let Some(u) = source.epg_url.as_ref().filter(|u| !u.is_empty()) {
            return Ok(Some(u.clone()));
        }
        match source.kind {
            SourceKind::Xtream => Ok(Some(self.xtream(source)?.credentials().epg_url())),
            SourceKind::M3u => Ok(self.store.setting(&format!("m3u_epg_url.{}", source.id))?),
        }
    }

    /// Which parts of a source are stale and should be refreshed in the background.
    pub fn staleness(&self, source_id: i64) -> Result<(bool, bool)> {
        let s = self.store.source(source_id)?;
        let t = now();
        let catalog = s.catalog_updated_at.is_none_or(|u| t - u > CATALOG_MAX_AGE_SECS);
        let epg = s.epg_updated_at.is_none_or(|u| t - u > EPG_MAX_AGE_SECS);
        Ok((catalog, epg))
    }

    async fn download(&self, url: &str, source_id: i64, stage: &str, what: &str, progress: &ProgressFn) -> Result<Vec<u8>> {
        let mut resp = self.http.get(url).send().await?.error_for_status()?;
        let total = resp.content_length().filter(|t| *t > 0);
        let mut buf: Vec<u8> = Vec::with_capacity(total.unwrap_or(1 << 20) as usize);
        let mut last_report = 0usize;
        while let Some(chunk) = resp.chunk().await? {
            buf.extend_from_slice(&chunk);
            if buf.len() - last_report >= 512 * 1024 {
                last_report = buf.len();
                let mb = buf.len() as f64 / 1_048_576.0;
                progress(Progress {
                    source_id,
                    stage: stage.into(),
                    message: format!("Downloading {what} ({mb:.1} MB)"),
                    fraction: total.map(|t| (buf.len() as f32 / t as f32).min(1.0) * 0.6),
                    done: false,
                });
            }
        }
        Ok(buf)
    }

    fn report(progress: &ProgressFn, source_id: i64, stage: &str, message: impl Into<String>, fraction: Option<f32>) {
        progress(Progress { source_id, stage: stage.into(), message: message.into(), fraction, done: false });
    }

    /// Download categories and channels (live, movies, series) and replace the cached catalog.
    pub async fn refresh_catalog(&self, source_id: i64, progress: ProgressFn) -> Result<RefreshReport> {
        let source = self.store.source(source_id)?;
        let stage = "catalog";
        let (categories, channels, kinds) = match source.kind {
            SourceKind::Xtream => {
                let client = self.xtream(&source)?;
                let mut categories = Vec::new();
                let mut channels = Vec::new();
                // Series are skipped for now: they need per-show episode lookups (`get_series_info`)
                // before anything is playable.
                let steps = [(ContentKind::Live, "live channels"), (ContentKind::Movie, "movies")];
                for (i, (kind, label)) in steps.iter().enumerate() {
                    Self::report(&progress, source_id, stage, format!("Loading {label}"), Some(i as f32 / 2.0 * 0.8));
                    let cats = client.categories(*kind).await?;
                    let items = client.streams(*kind).await?;
                    categories.extend(cats);
                    channels.extend(items);
                }
                (categories, channels, vec![ContentKind::Live, ContentKind::Movie])
            }
            SourceKind::M3u => {
                let bytes = self.download(&source.url, source_id, stage, "playlist", &progress).await?;
                Self::report(&progress, source_id, stage, "Parsing playlist", Some(0.7));
                let text = String::from_utf8_lossy(&bytes);
                let playlist = m3u::parse(&text);
                if playlist.entries.is_empty() && !text.trim_start().starts_with("#EXTM3U") {
                    return Err(Error::Parse("the URL did not return an M3U playlist".into()));
                }
                if let Some(epg) = &playlist.epg_url {
                    self.store.set_setting(&format!("m3u_epg_url.{source_id}"), Some(epg))?;
                }
                let (cats, chans) = m3u::into_catalog(&playlist);
                (cats, chans, vec![ContentKind::Live, ContentKind::Movie, ContentKind::Series])
            }
        };

        // Categories referenced by streams but missing from the category list (common on
        // Xtream panels) get a placeholder so the channels stay reachable.
        let mut categories = categories;
        {
            let known: std::collections::HashSet<(ContentKind, &str)> =
                categories.iter().map(|c| (c.kind, c.id.as_str())).collect();
            let mut missing: Vec<Category> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for ch in &channels {
                if !known.contains(&(ch.kind, ch.category_id.as_str())) && seen.insert((ch.kind, ch.category_id.clone())) {
                    missing.push(Category {
                        id: ch.category_id.clone(),
                        name: if ch.category_id.is_empty() { "Uncategorized".into() } else { format!("Category {}", ch.category_id) },
                        kind: ch.kind,
                        channel_count: 0,
                    });
                }
            }
            categories.extend(missing);
        }

        Self::report(&progress, source_id, stage, format!("Saving {} channels", channels.len()), Some(0.85));
        let store = self.store.clone();
        let report = RefreshReport { categories: categories.len(), channels: channels.len(), ..Default::default() };
        let matched = tokio::task::spawn_blocking(move || -> Result<usize> {
            store.replace_catalog(source_id, &kinds, &categories, &channels)?;
            store.resolve_epg_matches(source_id)
        })
        .await
        .map_err(|e| Error::Parse(format!("background task failed: {e}")))??;

        progress(Progress {
            source_id,
            stage: stage.into(),
            message: format!("{} channels loaded", report.channels),
            fraction: Some(1.0),
            done: true,
        });
        Ok(RefreshReport { epg_matched_by_name: matched, ..report })
    }

    /// Download and import the XMLTV guide.
    pub async fn refresh_epg(&self, source_id: i64, progress: ProgressFn) -> Result<RefreshReport> {
        let source = self.store.source(source_id)?;
        let stage = "epg";
        let Some(url) = self.epg_url(&source)? else {
            progress(Progress {
                source_id,
                stage: stage.into(),
                message: "No guide URL configured".into(),
                fraction: None,
                done: true,
            });
            return Ok(RefreshReport::default());
        };
        let bytes = self.download(&url, source_id, stage, "TV guide", &progress).await?;
        Self::report(&progress, source_id, stage, "Importing TV guide", Some(0.65));

        let store = self.store.clone();
        let (epg_channels, programmes, matched) = tokio::task::spawn_blocking(move || -> Result<(usize, usize, usize)> {
            let keep_after = now() - EPG_KEEP_PAST_SECS;
            let (c, p) = store.import_epg(source_id, keep_after, |sink| {
                xmltv::parse(xmltv::maybe_gunzip(&bytes), |item| match item {
                    XmltvItem::Channel(c) => sink.channel(&c),
                    XmltvItem::Programme(p) => sink.programme(&p),
                })
            })?;
            let matched = store.resolve_epg_matches(source_id)?;
            Ok((c, p, matched))
        })
        .await
        .map_err(|e| Error::Parse(format!("background task failed: {e}")))??;

        progress(Progress {
            source_id,
            stage: stage.into(),
            message: format!("Guide loaded: {programmes} programmes on {epg_channels} channels"),
            fraction: Some(1.0),
            done: true,
        });
        Ok(RefreshReport { epg_channels, programmes, epg_matched_by_name: matched, ..Default::default() })
    }
}
