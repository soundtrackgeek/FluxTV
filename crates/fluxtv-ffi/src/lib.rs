//! Swift bindings (UniFFI) for the FluxTV Apple apps.
//!
//! [`FluxEngine`] wraps [`fluxtv_core::FluxCore`] with a Tokio runtime and a local media
//! server ([`media`]) that turns MPEG-TS live streams into HLS for AVPlayer. Passwords are
//! never written to the database: the apps keep accounts in the Keychain and hand them over
//! with [`FluxEngine::sync_accounts`] at launch.

mod media;
mod types;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use fluxtv_core::{FluxCore, NewSource, Progress, ProgressFn};

pub use types::*;

uniffi::setup_scaffolding!();

/// Settings key mapping a Keychain account key to its database source id.
fn account_setting(key: &str) -> String {
    format!("apple.account.{key}")
}

#[derive(uniffi::Object)]
pub struct FluxEngine {
    core: Arc<FluxCore>,
    runtime: Mutex<Option<tokio::runtime::Runtime>>,
    handle: tokio::runtime::Handle,
    media: media::MediaServer,
    progress: Arc<Mutex<HashMap<i64, RefreshProgress>>>,
}

impl FluxEngine {
    /// Run a future on the engine's runtime. The returned `JoinHandle` can be awaited from
    /// any executor, which is how UniFFI drives Swift `async` calls.
    async fn spawn<T: Send + 'static>(
        &self,
        fut: impl Future<Output = Result<T, FluxError>> + Send + 'static,
    ) -> Result<T, FluxError> {
        self.handle.spawn(fut).await.map_err(|e| FluxError::Internal { message: e.to_string() })?
    }

    fn source_info(&self, id: i64) -> Result<SourceInfo, FluxError> {
        let store = self.core.store();
        let s = store.source(id)?;
        let live = store.channel_count(id, fluxtv_core::ContentKind::Live)?;
        let movies = store.channel_count(id, fluxtv_core::ContentKind::Movie)?;
        let key = store.setting(&format!("apple.source.{id}"))?.unwrap_or_default();
        Ok(SourceInfo::from_core(s, key, live, movies))
    }
}

impl Drop for FluxEngine {
    fn drop(&mut self) {
        self.media.stop();
        if let Some(rt) = self.runtime.lock().unwrap_or_else(|e| e.into_inner()).take() {
            rt.shutdown_background();
        }
    }
}

#[uniffi::export]
impl FluxEngine {
    /// Open (or create) the database at `database_path` and start the local media server.
    #[uniffi::constructor]
    pub fn new(database_path: String) -> Result<Arc<Self>, FluxError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(3)
            .thread_name("fluxtv")
            .enable_all()
            .build()
            .map_err(|e| FluxError::Internal { message: e.to_string() })?;
        let core = Arc::new(FluxCore::open(&database_path)?);
        let handle = runtime.handle().clone();
        let media = runtime
            .block_on(media::MediaServer::start(core.http().clone(), handle.clone()))
            .map_err(|e| FluxError::Internal { message: format!("media server: {e}") })?;
        Ok(Arc::new(Self {
            core,
            runtime: Mutex::new(Some(runtime)),
            handle,
            media,
            progress: Arc::default(),
        }))
    }

    /// Core version string (`FluxTV/x.y.z`).
    pub fn version(&self) -> String {
        fluxtv_core::library::USER_AGENT.to_string()
    }

    // ----- accounts ------------------------------------------------------------------------

    /// Normalize and validate account input against the provider (Xtream logins are checked).
    /// Pasted Xtream `get.php` links become Xtream accounts.
    pub async fn validate_account(&self, input: AccountInput) -> Result<ValidatedAccount, FluxError> {
        let core = self.core.clone();
        self.spawn(async move {
            let (s, info) = core.prepare_source(input.into_core()).await?;
            Ok(ValidatedAccount {
                name: s.name,
                kind: s.kind.into(),
                url: s.url,
                username: s.username,
                password: s.password,
                epg_url: s.epg_url,
                info: info.map(Into::into),
            })
        })
        .await
    }

    /// Make the database match the Keychain: add or update every account (keeping cached
    /// catalogs, favorites and history), remove sources whose account is gone, and load the
    /// passwords into memory for this session.
    pub fn sync_accounts(&self, accounts: Vec<SavedAccount>) -> Result<Vec<SourceInfo>, FluxError> {
        let store = self.core.store();
        let mut keep = Vec::new();
        let mut out = Vec::new();
        for a in accounts {
            let key_setting = account_setting(&a.key);
            let new = NewSource {
                name: a.name.clone(),
                kind: a.kind.into(),
                url: a.url.clone(),
                username: a.username.clone(),
                password: None,
                epg_url: a.epg_url.clone(),
            };
            let existing = store
                .setting(&key_setting)?
                .and_then(|v| v.parse::<i64>().ok())
                .and_then(|id| store.source(id).ok());
            let id = match existing {
                Some(s) => {
                    let changed_origin = s.url != new.url || s.username != new.username || s.kind != new.kind;
                    store.update_source(s.id, &new)?;
                    if changed_origin {
                        // Different server or login: the cached catalog is someone else's.
                        store.replace_catalog(
                            s.id,
                            &[fluxtv_core::ContentKind::Live, fluxtv_core::ContentKind::Movie, fluxtv_core::ContentKind::Series],
                            &[],
                            &[],
                        )?;
                        store.set_setting(&format!("apple.stale.{}", s.id), Some("1"))?;
                    }
                    s.id
                }
                None => {
                    let s = store.insert_source(&new)?;
                    store.set_setting(&key_setting, Some(&s.id.to_string()))?;
                    store.set_setting(&format!("apple.source.{}", s.id), Some(&a.key))?;
                    s.id
                }
            };
            store.set_runtime_password(id, a.password.clone());
            keep.push(id);
            out.push(self.source_info(id)?);
        }
        for s in store.sources()? {
            if !keep.contains(&s.id) {
                if let Some(key) = store.setting(&format!("apple.source.{}", s.id))? {
                    store.set_setting(&account_setting(&key), None)?;
                }
                store.set_setting(&format!("apple.source.{}", s.id), None)?;
                store.set_setting(&format!("apple.stale.{}", s.id), None)?;
                store.set_runtime_password(s.id, None);
                store.delete_source(s.id)?;
            }
        }
        Ok(out)
    }

    pub fn source(&self, source_id: i64) -> Result<SourceInfo, FluxError> {
        self.source_info(source_id)
    }

    /// Subscription status from the provider (Xtream only).
    pub async fn account_info(&self, source_id: i64) -> Result<Option<AccountInfo>, FluxError> {
        let core = self.core.clone();
        self.spawn(async move { Ok(core.account_info(source_id).await?.map(Into::into)) }).await
    }

    // ----- refresh -------------------------------------------------------------------------

    /// Which parts of a source are out of date.
    pub fn staleness(&self, source_id: i64) -> Result<Staleness, FluxError> {
        let (catalog, guide) = self.core.staleness(source_id)?;
        let forced = self.core.store().setting(&format!("apple.stale.{source_id}"))?.is_some();
        Ok(Staleness { catalog: catalog || forced, guide: guide || forced })
    }

    /// Refresh the catalog and then the guide (only stale parts unless `force`).
    /// Poll [`refresh_progress`](Self::refresh_progress) for status while it runs.
    pub async fn refresh(&self, source_id: i64, force: bool) -> Result<RefreshOutcome, FluxError> {
        let stale = self.staleness(source_id)?;
        let (do_catalog, do_guide) = (force || stale.catalog, force || stale.guide);
        let mut outcome = RefreshOutcome::default();
        if !do_catalog && !do_guide {
            return Ok(outcome);
        }
        {
            let mut map = self.progress.lock().unwrap_or_else(|e| e.into_inner());
            if map.get(&source_id).is_some_and(|p| !p.done) {
                outcome.already_running = true;
                return Ok(outcome);
            }
            let stage = if do_catalog { "catalog" } else { "epg" };
            map.insert(
                source_id,
                RefreshProgress { stage: stage.into(), message: "Starting".into(), fraction: None, done: false },
            );
        }
        let progress_map = self.progress.clone();
        let progress: ProgressFn = Arc::new(move |p: Progress| {
            progress_map.lock().unwrap_or_else(|e| e.into_inner()).insert(
                p.source_id,
                RefreshProgress { stage: p.stage, message: p.message, fraction: p.fraction, done: false },
            );
        });

        let core = self.core.clone();
        let result = self
            .spawn(async move {
                let mut outcome = RefreshOutcome::default();
                if do_catalog {
                    let r = core.refresh_catalog(source_id, progress.clone()).await?;
                    core.store().set_setting(&format!("apple.stale.{source_id}"), None)?;
                    outcome.catalog_refreshed = true;
                    outcome.channels = r.channels as u64;
                }
                if do_guide {
                    // A broken guide shouldn't hide a working catalog.
                    match core.refresh_epg(source_id, progress).await {
                        Ok(r) => {
                            outcome.guide_refreshed = true;
                            outcome.programmes = r.programmes as u64;
                        }
                        Err(e) => outcome.guide_error = Some(e.to_string()),
                    }
                }
                Ok(outcome)
            })
            .await;

        let mut map = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        let message = match &result {
            Ok(o) if o.guide_error.is_some() => "Guide update failed".to_string(),
            Ok(_) => "Up to date".to_string(),
            Err(e) => e.to_string(),
        };
        map.insert(source_id, RefreshProgress { stage: "done".into(), message, fraction: Some(1.0), done: true });
        result
    }

    pub fn refresh_progress(&self, source_id: i64) -> Option<RefreshProgress> {
        self.progress.lock().unwrap_or_else(|e| e.into_inner()).get(&source_id).cloned()
    }

    // ----- catalog -------------------------------------------------------------------------

    pub fn categories(&self, source_id: i64, kind: ContentKind) -> Result<Vec<Category>, FluxError> {
        Ok(self.core.store().categories(source_id, kind.into())?.into_iter().map(Into::into).collect())
    }

    pub fn channels(&self, source_id: i64, kind: ContentKind, category_id: String) -> Result<Vec<Channel>, FluxError> {
        Ok(self.core.store().channels_in_category(source_id, kind.into(), &category_id)?.into_iter().map(Into::into).collect())
    }

    pub fn channel(&self, source_id: i64, kind: ContentKind, channel_id: String) -> Result<Channel, FluxError> {
        Ok(self.core.store().channel(source_id, kind.into(), &channel_id)?.into())
    }

    /// Ranked search; every query word must match.
    pub fn search(&self, source_id: i64, kind: ContentKind, query: String, limit: u32) -> Result<Vec<Channel>, FluxError> {
        Ok(self
            .core
            .store()
            .search_channels(source_id, kind.into(), &query, limit as usize)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    pub fn favorites(&self, source_id: i64, kind: ContentKind) -> Result<Vec<Channel>, FluxError> {
        Ok(self.core.store().favorites(source_id, kind.into())?.into_iter().map(Into::into).collect())
    }

    /// Returns the new favorite state.
    pub fn toggle_favorite(&self, source_id: i64, kind: ContentKind, channel_id: String) -> Result<bool, FluxError> {
        Ok(self.core.store().toggle_favorite(source_id, kind.into(), &channel_id)?)
    }

    /// Favorite ids (including channels not in the current catalog), for backups.
    pub fn favorite_ids(&self, source_id: i64, kind: ContentKind) -> Result<Vec<String>, FluxError> {
        Ok(self.core.store().favorite_ids(source_id, kind.into())?)
    }

    /// Restore favorites from a backup (adds; never removes).
    pub fn restore_favorites(&self, source_id: i64, kind: ContentKind, channel_ids: Vec<String>) -> Result<(), FluxError> {
        let store = self.core.store();
        for id in channel_ids {
            store.set_favorite(source_id, kind.into(), &id, true)?;
        }
        Ok(())
    }

    pub fn recent(&self, source_id: i64, kind: ContentKind, limit: u32) -> Result<Vec<Channel>, FluxError> {
        Ok(self.core.store().recent(source_id, kind.into(), limit as usize)?.into_iter().map(Into::into).collect())
    }

    pub fn clear_history(&self, source_id: i64) -> Result<(), FluxError> {
        Ok(self.core.store().clear_history(source_id)?)
    }

    // ----- guide ---------------------------------------------------------------------------

    /// What is on now and next for each XMLTV id (in the same order).
    pub fn now_next(&self, source_id: i64, epg_ids: Vec<String>) -> Result<Vec<NowNext>, FluxError> {
        Ok(self
            .core
            .store()
            .now_next(source_id, &epg_ids, fluxtv_core::time::now())?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Programmes overlapping `[from, to)` (Unix seconds) per XMLTV id.
    pub fn guide(&self, source_id: i64, epg_ids: Vec<String>, from: i64, to: i64) -> Result<Vec<GuideRow>, FluxError> {
        Ok(self.core.store().guide(source_id, &epg_ids, from, to)?.into_iter().map(Into::into).collect())
    }

    // ----- playback ------------------------------------------------------------------------

    /// Prepare playback and return a URL for AVPlayer. Live MPEG-TS streams are served as HLS
    /// by the local media server, and so are movies stored as MPEG-TS; MP4 movies are passed
    /// through. Blocks briefly for movies while the file is inspected. Starting a stream always
    /// stops the previous upstream connection first, so single-connection accounts keep working
    /// when zapping.
    pub fn play(&self, source_id: i64, kind: ContentKind, channel_id: String) -> Result<Playback, FluxError> {
        let store = self.core.store();
        let channel = store.channel(source_id, kind.into(), &channel_id)?;
        let upstream = self.core.stream_url(source_id, kind.into(), &channel_id)?;
        let path = upstream.split(['?', '#']).next().unwrap_or_default().to_ascii_lowercase();
        let live = kind == ContentKind::Live;
        let url = if path.ends_with(".m3u8") {
            // Already HLS (M3U playlists): AVPlayer can fetch it directly.
            self.media.stop();
            upstream
        } else if live {
            self.media.start_live(upstream)
        } else {
            // Sniffs the file (MP4 / MPEG-TS / MKV) with a couple of range requests first.
            let ext = channel.container.clone().unwrap_or_else(|| "mp4".into());
            self.handle
                .block_on(self.media.prepare_vod(upstream, &ext))
                .map_err(|message| FluxError::Invalid { message })?
        };
        if let Err(e) = store.record_watch(source_id, kind.into(), &channel_id) {
            log::warn!("could not record history: {e}");
        }
        Ok(Playback { url, live, channel: channel.into() })
    }

    /// Close the upstream connection (player dismissed).
    pub fn stop_playback(&self) {
        self.media.stop();
    }

    /// State of the current live stream, for status overlays and error messages.
    pub fn playback_status(&self) -> PlaybackStatus {
        self.media.status()
    }
}
