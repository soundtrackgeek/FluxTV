//! End-to-end smoke test against a real provider.
//!
//! Reads credentials from `.env` in the working directory (or the environment):
//!   iptv_username=...   iptv_password=...   iptv_server=http://host[:port] (optional)
//!
//!   cargo run -p fluxtv-core --example probe --release
use std::sync::Arc;
use std::time::Instant;

use fluxtv_core::*;

fn load_dotenv() {
    for path in [".env", "../.env", "../../.env"] {
        if let Ok(text) = std::fs::read_to_string(path) {
            for line in text.lines() {
                if let Some((k, v)) = line.split_once('=')
                    && std::env::var(k.trim()).is_err() {
                        // SAFETY: single-threaded at this point.
                        unsafe { std::env::set_var(k.trim(), v.trim()) };
                    }
            }
            return;
        }
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    load_dotenv();
    let user = std::env::var("iptv_username").expect("iptv_username not set");
    let pass = std::env::var("iptv_password").expect("iptv_password not set");
    let server = std::env::var("iptv_server").unwrap_or_else(|_| "http://ourxtream.com".into());

    let db = std::env::temp_dir().join("fluxtv-probe.db");
    let _ = std::fs::remove_file(&db);
    let core = FluxCore::open(&db)?;

    let t = Instant::now();
    let source = core
        .add_source(NewSource {
            name: String::new(),
            kind: SourceKind::Xtream,
            url: server,
            username: Some(user),
            password: Some(pass),
            epg_url: None,
        })
        .await?;
    println!("source #{} '{}' added in {:?}", source.id, source.name, t.elapsed());
    let account = core.account_info(source.id).await?.unwrap();
    println!("account: status={} max_connections={} formats={:?}", account.status, account.max_connections, account.allowed_formats);

    let progress: ProgressFn = Arc::new(|p: Progress| {
        if p.done || p.message.starts_with("Loading") || p.message.starts_with("Saving") || p.message.starts_with("Importing") {
            println!("  [{}] {}", p.stage, p.message);
        }
    });

    let t = Instant::now();
    let r = core.refresh_catalog(source.id, progress.clone()).await?;
    println!("catalog: {} categories, {} channels in {:?}", r.categories, r.channels, t.elapsed());

    let t = Instant::now();
    let r = core.refresh_epg(source.id, progress).await?;
    println!(
        "epg: {} channels, {} programmes, {} channels matched by name, in {:?}",
        r.epg_channels, r.programmes, r.epg_matched_by_name, t.elapsed()
    );

    let store = core.store();
    for q in ["nrk1", "tv2 sport", "bbc one", "sky sports main event"] {
        let t = Instant::now();
        let hits = store.search_channels(source.id, ContentKind::Live, q, 5)?;
        println!("search '{q}' ({:?}):", t.elapsed());
        for h in &hits {
            println!("   {} [{}] epg={:?}", h.name, h.id, h.epg_id);
        }
        let ids: Vec<String> = hits.iter().filter_map(|h| h.epg_id.clone()).collect();
        for nn in store.now_next(source.id, &ids, time::now())? {
            if let Some(p) = nn.now {
                println!("   now on {}: {}", nn.epg_id, p.title);
            }
        }
    }
    let with_epg: i64 = store.channels_in_category(source.id, ContentKind::Live, "50")?.iter().filter(|c| c.epg_id.is_some()).count() as i64;
    println!("category 50 channels with EPG: {with_epg}");
    let url = core.stream_url(source.id, ContentKind::Live, "45094")?;
    println!("stream url built ({} chars, scheme {})", url.len(), &url[..url.find(':').unwrap()]);
    println!("db size: {:.1} MB", std::fs::metadata(&db)?.len() as f64 / 1_048_576.0);
    Ok(())
}
