mod commands;
mod external;
mod proxy;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use tauri::Manager;

use commands::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let core = Arc::new(fluxtv_core::FluxCore::open(data_dir.join("fluxtv.db"))?);
            let proxy = tauri::async_runtime::block_on(proxy::Proxy::start(core.http().clone()))?;
            app.manage(AppState { core, proxy, refreshing: Mutex::new(HashSet::new()) });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::list_sources,
            commands::add_source,
            commands::update_source,
            commands::delete_source,
            commands::account_info,
            commands::refresh_source,
            commands::get_categories,
            commands::get_channels,
            commands::get_favorites,
            commands::get_recent,
            commands::clear_history,
            commands::search_channels,
            commands::toggle_favorite,
            commands::now_next,
            commands::get_guide,
            commands::play,
            commands::stop_playback,
            commands::external_player,
            commands::open_external,
            commands::get_setting,
            commands::set_setting,
        ])
        .run(tauri::generate_context!())
        .expect("error while running FluxTV");
}
