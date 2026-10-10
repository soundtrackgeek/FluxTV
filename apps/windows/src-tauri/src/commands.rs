//! Tauri commands: the IPC surface used by the React frontend.
//!
//! All commands are `async` so they run on the Tokio pool instead of the UI thread.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use fluxtv_core::{
    AccountInfo, Category, Channel, ContentKind, FluxCore, GuideRow, NewSource, NowNext, Progress, ProgressFn, Source,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::external::{self, ExternalPlayer};
use crate::proxy::Proxy;

pub struct AppState {
    pub core: Arc<FluxCore>,
    pub proxy: Proxy,
    pub refreshing: Mutex<HashSet<i64>>,
}

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

const SETTING_EXTERNAL_PLAYER: &str = "external_player";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: String,
    core_version: String,
}

#[tauri::command]
pub async fn app_info() -> AppInfo {
    AppInfo { version: env!("CARGO_PKG_VERSION").into(), core_version: fluxtv_core::library::USER_AGENT.into() }
}

// ----- sources -------------------------------------------------------------------------------

#[tauri::command]
pub async fn list_sources(state: State<'_, AppState>) -> CmdResult<Vec<Source>> {
    state.core.store().sources().map_err(err)
}

#[tauri::command]
pub async fn add_source(state: State<'_, AppState>, input: NewSource) -> CmdResult<Source> {
    state.core.add_source(input).await.map_err(err)
}

#[tauri::command]
pub async fn update_source(state: State<'_, AppState>, id: i64, input: NewSource) -> CmdResult<Source> {
    state.core.update_source(id, input).await.map_err(err)
}

#[tauri::command]
pub async fn delete_source(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    let core = state.core.clone();
    tokio::task::spawn_blocking(move || core.store().delete_source(id)).await.map_err(err)?.map_err(err)
}

#[tauri::command]
pub async fn account_info(state: State<'_, AppState>, source_id: i64) -> CmdResult<Option<AccountInfo>> {
    state.core.account_info(source_id).await.map_err(err)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RefreshEvent {
    source_id: i64,
    stage: String,
    message: String,
}

/// Refresh the catalog and guide in the background. With `force = false` only stale parts
/// are refreshed. Progress is reported through `refresh-progress` events; completion through
/// `catalog-updated` / `epg-updated`, failures through `refresh-error`.
#[tauri::command]
pub async fn refresh_source(app: AppHandle, state: State<'_, AppState>, source_id: i64, force: bool) -> CmdResult<bool> {
    let (catalog_stale, epg_stale) = state.core.staleness(source_id).map_err(err)?;
    let (do_catalog, do_epg) = (force || catalog_stale, force || epg_stale);
    if !do_catalog && !do_epg {
        return Ok(false);
    }
    if !state.refreshing.lock().unwrap().insert(source_id) {
        return Ok(false); // already running
    }
    let core = state.core.clone();
    tauri::async_runtime::spawn(async move {
        let emitter = app.clone();
        let progress: ProgressFn = Arc::new(move |p: Progress| {
            let _ = emitter.emit("refresh-progress", p);
        });
        let fail = |stage: &str, e: fluxtv_core::Error| {
            log::warn!("{stage} refresh failed for source {source_id}: {e}");
            let _ = app.emit(
                "refresh-error",
                RefreshEvent { source_id, stage: stage.into(), message: e.to_string() },
            );
        };
        let mut catalog_ok = true;
        if do_catalog {
            match core.refresh_catalog(source_id, progress.clone()).await {
                Ok(_) => {
                    let _ = app.emit("catalog-updated", source_id);
                }
                Err(e) => {
                    catalog_ok = false;
                    fail("catalog", e);
                }
            }
        }
        if do_epg && catalog_ok {
            match core.refresh_epg(source_id, progress).await {
                Ok(_) => {
                    let _ = app.emit("epg-updated", source_id);
                }
                Err(e) => fail("epg", e),
            }
        }
        if let Some(state) = tauri::Manager::try_state::<AppState>(&app) {
            state.refreshing.lock().unwrap().remove(&source_id);
        }
    });
    Ok(true)
}

// ----- catalog -------------------------------------------------------------------------------

#[tauri::command]
pub async fn get_categories(state: State<'_, AppState>, source_id: i64, kind: ContentKind) -> CmdResult<Vec<Category>> {
    state.core.store().categories(source_id, kind).map_err(err)
}

#[tauri::command]
pub async fn get_channels(
    state: State<'_, AppState>,
    source_id: i64,
    kind: ContentKind,
    category_id: String,
) -> CmdResult<Vec<Channel>> {
    state.core.store().channels_in_category(source_id, kind, &category_id).map_err(err)
}

#[tauri::command]
pub async fn get_favorites(state: State<'_, AppState>, source_id: i64, kind: ContentKind) -> CmdResult<Vec<Channel>> {
    state.core.store().favorites(source_id, kind).map_err(err)
}

#[tauri::command]
pub async fn get_recent(state: State<'_, AppState>, source_id: i64, kind: ContentKind) -> CmdResult<Vec<Channel>> {
    state.core.store().recent(source_id, kind, 60).map_err(err)
}

#[tauri::command]
pub async fn clear_history(state: State<'_, AppState>, source_id: i64) -> CmdResult<()> {
    state.core.store().clear_history(source_id).map_err(err)
}

#[tauri::command]
pub async fn search_channels(
    state: State<'_, AppState>,
    source_id: i64,
    kind: ContentKind,
    query: String,
) -> CmdResult<Vec<Channel>> {
    let core = state.core.clone();
    tokio::task::spawn_blocking(move || core.store().search_channels(source_id, kind, &query, 300))
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
pub async fn toggle_favorite(
    state: State<'_, AppState>,
    source_id: i64,
    kind: ContentKind,
    channel_id: String,
) -> CmdResult<bool> {
    state.core.store().toggle_favorite(source_id, kind, &channel_id).map_err(err)
}

// ----- guide ---------------------------------------------------------------------------------

#[tauri::command]
pub async fn now_next(state: State<'_, AppState>, source_id: i64, epg_ids: Vec<String>) -> CmdResult<Vec<NowNext>> {
    let core = state.core.clone();
    tokio::task::spawn_blocking(move || core.store().now_next(source_id, &epg_ids, fluxtv_core::time::now()))
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
pub async fn get_guide(
    state: State<'_, AppState>,
    source_id: i64,
    epg_ids: Vec<String>,
    from: i64,
    to: i64,
) -> CmdResult<Vec<GuideRow>> {
    let core = state.core.clone();
    tokio::task::spawn_blocking(move || core.store().guide(source_id, &epg_ids, from, to))
        .await
        .map_err(err)?
        .map_err(err)
}

// ----- playback ------------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayInfo {
    /// Local proxy URL for the stream.
    url: String,
    channel: Channel,
}

#[tauri::command]
pub async fn play(state: State<'_, AppState>, source_id: i64, kind: ContentKind, channel_id: String) -> CmdResult<PlayInfo> {
    let store = state.core.store();
    let channel = store.channel(source_id, kind, &channel_id).map_err(err)?;
    let upstream = state.core.stream_url(source_id, kind, &channel_id).map_err(err)?;
    let is_ts = kind == ContentKind::Live
        || upstream.split('?').next().is_some_and(|p| p.ends_with(".ts") || !p.rsplit('/').next().unwrap_or("").contains('.'));
    let ext = if is_ts { "ts" } else { channel.container.as_deref().unwrap_or("mp4") };
    let url = state.proxy.register(upstream, ext);
    if let Err(e) = store.record_watch(source_id, kind, &channel_id) {
        log::warn!("could not record history: {e}");
    }
    Ok(PlayInfo { url, channel })
}

#[tauri::command]
pub async fn stop_playback(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    // Stop the decoder before handing the connection to an external player.
    // Cancelling only the proxy could make native reconnect reclaim it.
    use tauri_plugin_libmpv::MpvExt;
    let _ = tauri::async_runtime::spawn_blocking(move || app.mpv().command("stop", &vec![], "main")).await;
    state.proxy.stop();
    Ok(())
}

#[tauri::command]
pub async fn external_player(state: State<'_, AppState>) -> CmdResult<Option<ExternalPlayer>> {
    let configured = state.core.store().setting(SETTING_EXTERNAL_PLAYER).map_err(err)?;
    Ok(external::detect(configured.as_deref()))
}

#[tauri::command]
pub async fn open_external(
    state: State<'_, AppState>,
    source_id: i64,
    kind: ContentKind,
    channel_id: String,
) -> CmdResult<String> {
    let configured = state.core.store().setting(SETTING_EXTERNAL_PLAYER).map_err(err)?;
    let player = external::detect(configured.as_deref())
        .ok_or_else(|| "No external player found. Install mpv or VLC, or set its path in Settings.".to_string())?;
    let channel = state.core.store().channel(source_id, kind, &channel_id).map_err(err)?;
    let upstream = state.core.stream_url(source_id, kind, &channel_id).map_err(err)?;
    // Route through the proxy too, so the password stays off the process command line and
    // the single-connection rule still applies.
    let ext = if kind == ContentKind::Live { "ts" } else { channel.container.as_deref().unwrap_or("mp4") };
    let url = state.proxy.register(upstream, ext);
    external::launch(&player, &url, &channel.name).map_err(err)?;
    let _ = state.core.store().record_watch(source_id, kind, &channel_id);
    Ok(player.name)
}

// ----- settings ------------------------------------------------------------------------------

#[tauri::command]
pub async fn get_setting(state: State<'_, AppState>, key: String) -> CmdResult<Option<String>> {
    check_ui_setting(&key)?;
    state.core.store().setting(&key).map_err(err)
}

#[tauri::command]
pub async fn set_setting(state: State<'_, AppState>, key: String, value: Option<String>) -> CmdResult<()> {
    check_ui_setting(&key)?;
    state.core.store().set_setting(&key, value.as_deref()).map_err(err)
}

// Guide URLs are credentials; the generic IPC must not expose or modify them.
fn check_ui_setting(key: &str) -> CmdResult<()> {
    if key == SETTING_EXTERNAL_PLAYER { Ok(()) } else { Err("Unknown setting".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_settings_cannot_access_credentials() {
        assert!(check_ui_setting(SETTING_EXTERNAL_PLAYER).is_ok());
        for key in ["m3u_epg_url.1", "schema_version", "credential_scrub_pending", "", "unknown"] {
            assert!(check_ui_setting(key).is_err());
        }
    }
}
