//! FluxTV shared core.
//!
//! Platform-independent IPTV logic shared by the Windows (Tauri) app today and the
//! Apple (SwiftUI) apps later:
//!
//! - [`m3u`]: M3U / M3U Plus playlist parsing
//! - [`xmltv`]: streaming XMLTV (EPG) parsing
//! - [`xtream`]: Xtream Codes API client
//! - [`hls`]: MPEG-TS → HLS repackaging for AVPlayer (Apple apps)
//! - [`search`]: name normalization and ranked search
//! - [`store`]: SQLite cache for catalog, guide, favorites and history
//! - [`library`]: [`FluxCore`], the high-level façade tying it all together

pub mod error;
pub mod hls;
pub mod library;
pub mod m3u;
pub mod models;
pub mod search;
pub mod store;
pub mod time;
pub mod xmltv;
pub mod xtream;

pub use error::{Error, Result};
pub use library::{FluxCore, ProgressFn};
pub use models::*;
pub use store::Store;
