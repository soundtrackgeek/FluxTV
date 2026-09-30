//! Local HTTP stream proxy.
//!
//! The webview never sees upstream URLs (they contain the IPTV password). Instead each
//! playback gets an unguessable session URL on `127.0.0.1`, and the proxy relays bytes
//! from the provider. It also enforces a single active upstream connection, since many
//! IPTV subscriptions allow only one: starting a new stream cancels the previous one
//! *before* connecting upstream, so channel switching doesn't trip the provider's limit.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures_util::StreamExt;
use tokio_util::sync::CancellationToken;

const MAX_SESSIONS: usize = 32;

struct Session {
    upstream: String,
    created: std::time::Instant,
}

struct Inner {
    http: reqwest::Client,
    sessions: Mutex<HashMap<String, Session>>,
    active: Mutex<Option<CancellationToken>>,
}

/// Handle shared with Tauri commands.
#[derive(Clone)]
pub struct Proxy {
    addr: SocketAddr,
    inner: Arc<Inner>,
}

impl Proxy {
    /// Bind on an ephemeral localhost port and serve in the background.
    pub async fn start(http: reqwest::Client) -> std::io::Result<Self> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let inner = Arc::new(Inner {
            http,
            sessions: Mutex::new(HashMap::new()),
            active: Mutex::new(None),
        });
        let app = Router::new()
            .route("/stream/{session}", get(stream).options(preflight))
            .with_state(inner.clone());
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                log::error!("stream proxy stopped: {e}");
            }
        });
        log::info!("stream proxy listening on {addr}");
        Ok(Self { addr, inner })
    }

    /// Register an upstream URL and return the local URL that plays it.
    pub fn register(&self, upstream: String, extension: &str) -> String {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let mut sessions = self.inner.sessions.lock().unwrap();
        if sessions.len() >= MAX_SESSIONS
            && let Some(oldest) = sessions.iter().min_by_key(|(_, s)| s.created).map(|(k, _)| k.clone()) {
                sessions.remove(&oldest);
            }
        sessions.insert(id.clone(), Session { upstream, created: std::time::Instant::now() });
        // The extension is cosmetic, but helps external players pick a demuxer quickly.
        format!("http://{}/stream/{}.{}", self.addr, id, extension)
    }

    /// Abort whatever is currently streaming (e.g. when the player is closed).
    pub fn stop(&self) {
        if let Some(t) = self.inner.active.lock().unwrap().take() {
            t.cancel();
        }
    }
}

fn cors(headers: &mut HeaderMap) {
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    headers.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("Content-Length, Content-Range, Accept-Ranges"),
    );
}

async fn preflight() -> Response {
    let mut resp = StatusCode::NO_CONTENT.into_response();
    let h = resp.headers_mut();
    cors(h);
    h.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, OPTIONS"));
    h.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("*"));
    resp
}

fn error(status: StatusCode, msg: &str) -> Response {
    let mut resp = (status, msg.to_string()).into_response();
    cors(resp.headers_mut());
    resp
}

async fn stream(
    State(inner): State<Arc<Inner>>,
    Path(session): Path<String>,
    method: Method,
    req_headers: HeaderMap,
) -> Response {
    let id = session.split('.').next().unwrap_or_default();
    let Some(upstream) = inner.sessions.lock().unwrap().get(id).map(|s| s.upstream.clone()) else {
        return error(StatusCode::NOT_FOUND, "unknown stream session");
    };

    // Cancel the previous stream first and give the provider a moment to see the
    // connection close, otherwise a 1-connection account rejects the new one.
    let token = CancellationToken::new();
    let previous = inner.active.lock().unwrap().replace(token.clone());
    if let Some(prev) = previous
        && !prev.is_cancelled() {
            prev.cancel();
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    if token.is_cancelled() {
        return error(StatusCode::GONE, "superseded by another stream");
    }

    let mut req = inner.http.request(method, &upstream);
    for name in [header::RANGE, header::ACCEPT] {
        if let Some(v) = req_headers.get(&name) {
            req = req.header(name, v.clone());
        }
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            let msg = fluxtv_core::Error::from(e).to_string();
            log::warn!("upstream connect failed: {msg}");
            return error(StatusCode::BAD_GATEWAY, &msg);
        }
    };
    let status = resp.status();
    if !status.is_success() {
        log::warn!("upstream returned {status}");
        return error(
            StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
            &format!("provider returned HTTP {}", status.as_u16()),
        );
    }

    let mut out = Response::builder().status(status.as_u16());
    let headers = out.headers_mut().unwrap();
    for name in [header::CONTENT_TYPE, header::CONTENT_LENGTH, header::CONTENT_RANGE, header::ACCEPT_RANGES] {
        if let Some(v) = resp.headers().get(name.as_str())
            && let Ok(v) = HeaderValue::from_bytes(v.as_bytes()) {
                headers.insert(name, v);
            }
    }
    headers.entry(header::CONTENT_TYPE).or_insert(HeaderValue::from_static("video/mp2t"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    cors(headers);

    // Relay bytes until the upstream ends, the client disconnects (the body is dropped),
    // or a newer stream cancels this one.
    let body = resp.bytes_stream().take_until(token.cancelled_owned());
    out.body(Body::from_stream(body)).unwrap_or_else(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "proxy error"))
}
