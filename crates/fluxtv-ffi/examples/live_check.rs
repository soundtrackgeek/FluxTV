//! End-to-end check of the Apple engine against a real provider: add the account the way
//! the apps do, refresh, search, start a live channel and print the local HLS URL.
//!
//! Reads `iptv_username`, `iptv_password` and optional `iptv_server` from the environment.
//!
//!   cargo run -p fluxtv-ffi --example live_check --release -- "nrk1 hd" 120
//!   cargo run -p fluxtv-ffi --example live_check --release -- "id:movie:19" 120
use std::time::{Duration, Instant};

use fluxtv_ffi::*;

fn main() -> Result<(), FluxError> {
    let user = std::env::var("iptv_username").expect("iptv_username not set");
    let pass = std::env::var("iptv_password").expect("iptv_password not set");
    let server = std::env::var("iptv_server").unwrap_or_else(|_| "http://ourxtream.com".into());
    let mut args = std::env::args().skip(1);
    let query = args.next().unwrap_or_else(|| "nrk1 hd".into());
    let serve_secs: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(60);

    let db = std::env::temp_dir().join("fluxtv-live-check.db");
    let engine = FluxEngine::new(db.to_string_lossy().into_owned())?;
    let rt = tokio::runtime::Runtime::new().unwrap();

    let t = Instant::now();
    let v = rt.block_on(engine.validate_account(AccountInput {
        name: String::new(),
        kind: SourceKind::Xtream,
        url: server,
        username: Some(user),
        password: Some(pass),
        epg_url: None,
    }))?;
    println!("validated '{}' ({:?}) in {:?}; status {:?}", v.name, v.kind, t.elapsed(), v.info.map(|i| i.status));
    let sources = engine.sync_accounts(vec![SavedAccount {
        key: "live-check".into(),
        name: v.name,
        kind: v.kind,
        url: v.url,
        username: v.username,
        password: v.password,
        epg_url: v.epg_url,
    }])?;
    let id = sources[0].id;
    let stale = engine.staleness(id)?;
    if stale.catalog || stale.guide {
        let t = Instant::now();
        let r = rt.block_on(engine.refresh(id, false))?;
        println!("refresh: {} channels, {} programmes in {:?} (guide error: {:?})", r.channels, r.programmes, t.elapsed(), r.guide_error);
    }
    // `id:movie:<id>` / `id:live:<id>` plays a specific item, anything else searches live TV.
    let (kind, ch) = match query.strip_prefix("id:") {
        Some(rest) => {
            let (k, cid) = rest.split_once(':').expect("id:<live|movie>:<id>");
            let kind = if k == "movie" { ContentKind::Movie } else { ContentKind::Live };
            (kind, engine.channel(id, kind, cid.to_string())?)
        }
        None => {
            let hits = engine.search(id, ContentKind::Live, query.clone(), 5)?;
            (ContentKind::Live, hits.into_iter().next().expect("no channel found"))
        }
    };
    println!("playing '{}' ({})", ch.name, ch.id);
    let p = engine.play(id, kind, ch.id.clone())?;
    println!("URL {}", p.url);
    let end = Instant::now() + Duration::from_secs(serve_secs);
    while Instant::now() < end {
        std::thread::sleep(Duration::from_secs(5));
        let s = engine.playback_status();
        println!(
            "status {:?} video {:?} audio {:?} {:.1} Mbit/s reconnects {} {}",
            s.state,
            s.video_codec,
            s.audio_codecs,
            s.bitrate as f64 / 1e6,
            s.reconnects,
            s.message.unwrap_or_default()
        );
    }
    engine.stop_playback();
    Ok(())
}
