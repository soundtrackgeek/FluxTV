//! Local media server on `127.0.0.1` for AVPlayer.
//!
//! - **Live** (`/live/<token>/index.m3u8`): one upstream MPEG-TS connection is cut into HLS
//!   segments by [`fluxtv_core::hls::Segmenter`] and published as a sliding-window live
//!   playlist. Dropped connections are re-established with an `EXT-X-DISCONTINUITY`. When
//!   the player stops asking for the playlist (paused, backgrounded) the upstream connection
//!   is closed after a short idle period and transparently reopened on the next request, so a
//!   paused player doesn't hold the account's only connection.
//! - **Movies**: the file is sniffed first. MP4 is passed through with byte ranges
//!   (`/vod/<token>/movie.<ext>`), like the Windows proxy. Movies stored as MPEG-TS are served
//!   as a seekable HLS VOD playlist (`/vod/<token>/index.m3u8`) whose segments are byte ranges
//!   of the file, sized from its bitrate. Matroska isn't playable by AVPlayer and is reported.
//!
//! Only one upstream connection is open at a time: starting a stream cancels the previous
//! one and waits briefly for the provider to notice before connecting. Session tokens are
//! random, and upstream URLs (which embed the password) never leave this module.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use fluxtv_core::hls::{self, PlaylistEntry, Segment, Segmenter, StreamInfo, TsProbe, TS_PACKET};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Handle;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::types::{PlaybackStatus, StreamState};

/// Segments kept in memory (late requests and the playlist window).
const KEEP_SEGMENTS: usize = 12;
/// Segments listed in the live playlist.
const PLAYLIST_WINDOW: usize = 6;
/// AVPlayer starts three target durations from the live edge; wait for that much.
const START_SEGMENTS: usize = 3;
const START_TIMEOUT: Duration = Duration::from_secs(25);
/// Reconnect when the upstream sends nothing for this long.
const READ_STALL: Duration = Duration::from_secs(15);
/// Close the upstream when the player hasn't requested anything for this long.
const IDLE_STOP: Duration = Duration::from_secs(30);
/// Consecutive connection attempts without data before giving up.
const MAX_FAILURES: u32 = 6;
/// Pause after closing one upstream connection before opening the next (1-connection accounts).
const SWITCH_GRACE: Duration = Duration::from_millis(300);
const KEEP_ALIVE: Duration = Duration::from_secs(120);
/// Bytes read from the start and end of a movie to identify it and measure its duration.
const PROBE_HEAD: u64 = TS_PACKET as u64 * 2048;
const PROBE_TAIL: u64 = TS_PACKET as u64 * 16384;
/// Target length of TS movie segments.
const VOD_SEGMENT_SECS: f64 = 6.0;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct MediaServer {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    handle: Handle,
    addr: Mutex<SocketAddr>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    live: Option<Arc<LiveSession>>,
    vod: Option<Arc<VodSession>>,
    /// Whatever currently holds the upstream connection.
    active: Option<Lease>,
    next_lease: u64,
    /// When a running upstream connection was last cut.
    released_at: Option<Instant>,
}

impl State {
    /// Cancel the current holder of the upstream connection.
    fn release(&mut self) {
        if let Some(active) = self.active.take()
            && !active.cancel.is_cancelled()
        {
            active.cancel.cancel();
            self.released_at = Some(Instant::now());
        }
    }
}

/// The right to use the single upstream connection, until cancelled by the next user.
#[derive(Clone)]
struct Lease {
    id: u64,
    cancel: CancellationToken,
}

struct VodSession {
    token: String,
    upstream: String,
    /// Set for movies stored as MPEG-TS (served as HLS); `None` means pass-through.
    ts: Option<TsVod>,
    /// Segment requests of one movie queue up instead of cancelling each other.
    fetch: tokio::sync::Mutex<()>,
    /// Where the provider's redirects ended up. Reusing it avoids a redirect chain per
    /// request, which providers rate-limit (HTTP 429) when a player fetches many ranges.
    resolved: Mutex<Option<String>>,
}

enum FetchError {
    Superseded,
    Status(u16),
    Other(String),
}

impl FetchError {
    fn message(&self) -> String {
        match self {
            FetchError::Superseded => "superseded".into(),
            FetchError::Status(s) => format!("provider returned HTTP {s}"),
            FetchError::Other(m) => m.clone(),
        }
    }
}

struct Fetched {
    body: Vec<u8>,
    total: Option<u64>,
    final_url: String,
}

impl VodSession {
    /// Fetch a byte range, preferring the resolved URL and backing off when rate-limited.
    async fn fetch(&self, inner: &Inner, from: u64, to: u64, lease: &Lease) -> Result<Vec<u8>, FetchError> {
        let mut attempt = 0;
        loop {
            let resolved = lock(&self.resolved).clone();
            let url = resolved.as_deref().unwrap_or(&self.upstream);
            match inner.fetch_range(url, from, to, lease).await {
                Ok(f) => {
                    *lock(&self.resolved) = Some(f.final_url);
                    return Ok(f.body);
                }
                Err(FetchError::Superseded) => return Err(FetchError::Superseded),
                Err(e) if attempt >= 3 => return Err(e),
                Err(e) => {
                    attempt += 1;
                    if resolved.is_some() && !matches!(e, FetchError::Status(429)) {
                        // The provider's redirect target may have expired: start over.
                        *lock(&self.resolved) = None;
                    } else {
                        let wait = Duration::from_millis(800 * attempt);
                        tokio::select! {
                            _ = lease.cancel.cancelled() => return Err(FetchError::Superseded),
                            _ = tokio::time::sleep(wait) => {}
                        }
                    }
                }
            }
        }
    }
}

struct TsVod {
    probe: TsProbe,
    size: u64,
    segment_bytes: u64,
    durations: Vec<f64>,
}

impl TsVod {
    fn range(&self, n: usize) -> Option<(u64, u64)> {
        let start = self.probe.sync_offset as u64 + n as u64 * self.segment_bytes;
        (start < self.size).then(|| (start, (start + self.segment_bytes).min(self.size) - 1))
    }
}

struct LiveSession {
    token: String,
    upstream: String,
    segmenter: Mutex<Segmenter>,
    data: Mutex<LiveData>,
    notify: Notify,
}

struct LiveData {
    segments: VecDeque<Arc<Segment>>,
    evicted_discontinuities: u64,
    target: u32,
    info: StreamInfo,
    state: StreamState,
    message: Option<String>,
    reconnects: u32,
    last_access: Instant,
    running: bool,
}

impl LiveSession {
    fn new(upstream: String) -> Arc<Self> {
        Arc::new(Self {
            token: new_token(),
            upstream,
            segmenter: Mutex::new(Segmenter::new()),
            data: Mutex::new(LiveData {
                segments: VecDeque::new(),
                evicted_discontinuities: 0,
                target: 2,
                info: StreamInfo::default(),
                state: StreamState::Connecting,
                message: None,
                reconnects: 0,
                last_access: Instant::now(),
                running: false,
            }),
            notify: Notify::new(),
        })
    }

    fn update(&self, f: impl FnOnce(&mut LiveData)) {
        f(&mut lock(&self.data));
        self.notify.notify_waiters();
    }

    /// Feed upstream bytes; returns true when new segments were published.
    fn feed(&self, bytes: &[u8]) -> bool {
        let (segments, info, target) = {
            let mut seg = lock(&self.segmenter);
            seg.push(bytes);
            (seg.take_segments(), seg.info().clone(), seg.target_duration())
        };
        if segments.is_empty() {
            return false;
        }
        self.update(|d| {
            for s in segments {
                d.segments.push_back(Arc::new(s));
                while d.segments.len() > KEEP_SEGMENTS {
                    if d.segments.pop_front().is_some_and(|s| s.discontinuity) {
                        d.evicted_discontinuities += 1;
                    }
                }
            }
            d.info = info;
            if let Some(t) = target {
                d.target = t;
            }
            d.state = StreamState::Playing;
            d.message = None;
        });
        true
    }

    fn playlist(&self) -> String {
        let d = lock(&self.data);
        let start = d.segments.len().saturating_sub(PLAYLIST_WINDOW);
        let entries: Vec<PlaylistEntry> = d
            .segments
            .iter()
            .skip(start)
            .map(|s| PlaylistEntry { sequence: s.sequence, duration: s.duration, discontinuity: s.discontinuity })
            .collect();
        // The discontinuity sequence of the first listed segment counts every discontinuity
        // up to and including it.
        let listed_first = d.segments.iter().take(start + 1).filter(|s| s.discontinuity).count() as u64;
        hls::render_live_playlist(&entries, d.target, d.evicted_discontinuities + listed_first)
    }
}

fn new_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

impl MediaServer {
    pub async fn start(http: reqwest::Client, handle: Handle) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let inner = Arc::new(Inner { http, handle: handle.clone(), addr: Mutex::new(addr), state: Mutex::default() });
        handle.spawn(accept_loop(inner.clone(), listener));
        log::info!("media server listening on {addr}");
        Ok(Self { inner })
    }

    fn base(&self) -> String {
        format!("http://{}", lock(&self.inner.addr))
    }

    /// Start a live stream and return its playlist URL. The previous stream is stopped.
    pub fn start_live(&self, upstream: String) -> String {
        let session = LiveSession::new(upstream);
        {
            let mut st = lock(&self.inner.state);
            st.live = Some(session.clone());
            st.vod = None;
        }
        self.inner.clone().spawn_live(session.clone());
        format!("{}/live/{}/index.m3u8", self.base(), session.token)
    }

    /// Inspect a movie and return the URL AVPlayer should open. The previous stream is stopped.
    pub async fn prepare_vod(&self, upstream: String, extension: &str) -> Result<String, String> {
        {
            let mut st = lock(&self.inner.state);
            st.live = None;
            st.vod = None;
        }
        let lease = self.inner.acquire().await;
        let result = self.inner.probe_vod(&upstream, &lease).await;
        self.inner.finish(&lease);
        let token = new_token();
        let (ts, resolved) = result?;
        let url = if ts.is_some() {
            format!("{}/vod/{token}/index.m3u8", self.base())
        } else {
            let ext: String = extension.chars().filter(char::is_ascii_alphanumeric).take(5).collect();
            format!("{}/vod/{token}/movie.{}", self.base(), if ext.is_empty() { "mp4" } else { &ext })
        };
        lock(&self.inner.state).vod =
            Some(Arc::new(VodSession {
                token,
                upstream,
                ts,
                fetch: tokio::sync::Mutex::new(()),
                resolved: Mutex::new(Some(resolved)),
            }));
        Ok(url)
    }

    /// Close any upstream connection and forget the current sessions.
    pub fn stop(&self) {
        let mut st = lock(&self.inner.state);
        st.live = None;
        st.vod = None;
        st.release();
    }

    pub fn status(&self) -> PlaybackStatus {
        let live = lock(&self.inner.state).live.clone();
        let Some(s) = live else {
            return PlaybackStatus {
                state: StreamState::Idle,
                video_codec: None,
                audio_codecs: Vec::new(),
                unsupported: Vec::new(),
                message: None,
                reconnects: 0,
                bitrate: 0,
            };
        };
        let d = lock(&s.data);
        let (bytes, secs) = d.segments.iter().fold((0usize, 0f64), |(b, t), s| (b + s.data.len(), t + s.duration));
        PlaybackStatus {
            state: d.state,
            video_codec: d.info.video.clone(),
            audio_codecs: d.info.audio.clone(),
            unsupported: d.info.unsupported(),
            message: d.message.clone(),
            reconnects: d.reconnects,
            bitrate: if secs > 0.0 { (bytes as f64 * 8.0 / secs) as u64 } else { 0 },
        }
    }
}

impl Inner {
    /// Take over the single upstream slot: cancel the previous holder and give the provider a
    /// moment to register the closed connection.
    async fn acquire(&self) -> Lease {
        let (lease, released_at) = {
            let mut st = lock(&self.state);
            st.release();
            st.next_lease += 1;
            let lease = Lease { id: st.next_lease, cancel: CancellationToken::new() };
            st.active = Some(lease.clone());
            (lease, st.released_at)
        };
        if let Some(wait) = released_at.and_then(|t| SWITCH_GRACE.checked_sub(t.elapsed())) {
            tokio::time::sleep(wait).await;
        }
        lease
    }

    /// The holder finished its request cleanly; the next one needn't wait.
    fn finish(&self, lease: &Lease) {
        let mut st = lock(&self.state);
        if st.active.as_ref().is_some_and(|a| a.id == lease.id) {
            st.active = None;
        }
    }

    /// The holder's connection was cut mid-transfer; give the provider a moment.
    fn abandon(&self, lease: &Lease) {
        let mut st = lock(&self.state);
        if st.active.as_ref().is_some_and(|a| a.id == lease.id) {
            st.release();
        }
    }

    /// GET a byte range, reading at most the requested length. Also reports the total file
    /// size (from `Content-Range` / `Content-Length`) and the URL after redirects.
    async fn fetch_range(&self, url: &str, from: u64, to: u64, lease: &Lease) -> Result<Fetched, FetchError> {
        let req = self.http.get(url).header(reqwest::header::RANGE, format!("bytes={from}-{to}"));
        let mut resp = tokio::select! {
            _ = lease.cancel.cancelled() => return Err(FetchError::Superseded),
            r = req.send() => r.map_err(|e| FetchError::Other(fluxtv_core::Error::from(e).to_string()))?,
        };
        if !resp.status().is_success() {
            return Err(FetchError::Status(resp.status().as_u16()));
        }
        let final_url = resp.url().to_string();
        let total = resp
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit('/').next())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .or_else(|| (resp.status() == reqwest::StatusCode::OK).then(|| resp.content_length()).flatten());
        let limit = (to - from + 1) as usize;
        let mut body = Vec::with_capacity(limit.min(16 << 20));
        while body.len() < limit {
            let chunk = tokio::select! {
                _ = lease.cancel.cancelled() => return Err(FetchError::Superseded),
                c = tokio::time::timeout(READ_STALL, resp.chunk()) => c,
            };
            match chunk {
                Ok(Ok(Some(b))) => body.extend_from_slice(&b[..b.len().min(limit - body.len())]),
                Ok(Ok(None)) => break,
                Ok(Err(e)) => return Err(FetchError::Other(fluxtv_core::Error::from(e).to_string())),
                Err(_) => return Err(FetchError::Other("the provider stopped sending data".into())),
            }
        }
        Ok(Fetched { body, total, final_url })
    }

    /// Identify a movie's container; for MPEG-TS, measure it and plan HLS segments.
    /// Also returns the URL the provider redirected to.
    async fn probe_vod(&self, url: &str, lease: &Lease) -> Result<(Option<TsVod>, String), String> {
        let first = self.fetch_range(url, 0, PROBE_HEAD - 1, lease).await.map_err(|e| e.message())?;
        let (head, total, resolved) = (first.body, first.total, first.final_url);
        if head.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
            return Err("This movie is in Matroska (MKV) format, which Apple devices can't play.".into());
        }
        if head.len() >= 8 && head[4..8] == *b"ftyp" || !hls::looks_like_ts(&head) {
            return Ok((None, resolved)); // MP4 (or unknown): let AVPlayer read it directly
        }
        let size = total.ok_or("the provider did not report the movie's size")?;
        let probe = hls::probe_ts(&head).ok_or("could not read the movie's stream information")?;
        let tail_from = size.saturating_sub(PROBE_TAIL);
        let tail = self.fetch_range(&resolved, tail_from, size - 1, lease).await.map_err(|e| e.message())?.body;
        let last = hls::last_pts(&tail, probe.timing_pid).ok_or("could not find the end of the movie")?;
        let duration = hls::pts_span(probe.first_pts, last);
        if !(1.0..=24.0 * 3600.0).contains(&duration) {
            return Err("could not determine the movie's length".into());
        }
        let payload = size - probe.sync_offset as u64;
        let bytes_per_sec = payload as f64 / duration;
        let packets = ((bytes_per_sec * VOD_SEGMENT_SECS) as u64 / TS_PACKET as u64).max(512);
        let segment_bytes = packets * TS_PACKET as u64;
        let count = payload.div_ceil(segment_bytes) as usize;
        let durations = (0..count)
            .map(|i| {
                let len = segment_bytes.min(payload - i as u64 * segment_bytes);
                len as f64 / bytes_per_sec
            })
            .collect();
        log::info!("TS movie: {duration:.0} s, {count} segments of {segment_bytes} bytes");
        Ok((Some(TsVod { probe, size, segment_bytes, durations }), resolved))
    }

    fn is_current(&self, session: &Arc<LiveSession>) -> bool {
        lock(&self.state).live.as_ref().is_some_and(|l| Arc::ptr_eq(l, session))
    }

    fn spawn_live(self: Arc<Self>, session: Arc<LiveSession>) {
        {
            let mut d = lock(&session.data);
            if d.running {
                return;
            }
            d.running = true;
            d.last_access = Instant::now();
        }
        let handle = self.handle.clone();
        handle.spawn(async move {
            let lease = self.acquire().await;
            if self.is_current(&session) && !lease.cancel.is_cancelled() {
                self.run_live(&session, &lease.cancel).await;
            }
            // Ended on its own (idle, failed): the connection was just cut.
            self.abandon(&lease);
            session.update(|d| d.running = false);
        });
    }

    async fn run_live(&self, s: &LiveSession, cancel: &CancellationToken) {
        let mut failures = 0u32;
        let mut first = true;
        'connect: loop {
            if !first {
                lock(&s.segmenter).discontinuity();
                s.update(|d| {
                    d.state = StreamState::Reconnecting;
                    d.reconnects += 1;
                });
            }
            first = false;

            let resp = tokio::select! {
                _ = cancel.cancelled() => return,
                r = self.http.get(&s.upstream).send() => r.and_then(|r| r.error_for_status()),
            };
            let mut got_segments = false;
            let reason = match resp {
                Err(e) => fluxtv_core::Error::from(e).to_string(),
                Ok(mut resp) => loop {
                    let chunk = tokio::select! {
                        _ = cancel.cancelled() => return,
                        c = tokio::time::timeout(READ_STALL, resp.chunk()) => c,
                    };
                    match chunk {
                        Ok(Ok(Some(bytes))) => {
                            got_segments |= s.feed(&bytes);
                            if lock(&s.data).last_access.elapsed() > IDLE_STOP {
                                log::info!("live stream idle, closing upstream until the player returns");
                                s.update(|d| d.state = StreamState::Idle);
                                return;
                            }
                        }
                        Ok(Ok(None)) => break "the provider ended the stream".to_string(),
                        Ok(Err(e)) => break fluxtv_core::Error::from(e).to_string(),
                        Err(_) => break "no data from the provider".to_string(),
                    }
                },
            };
            log::warn!("live upstream interrupted: {reason}");
            failures = if got_segments { 1 } else { failures + 1 };
            if failures >= MAX_FAILURES {
                s.update(|d| {
                    d.state = StreamState::Failed;
                    d.message = Some(reason);
                });
                return;
            }
            s.update(|d| d.message = Some(reason));
            let backoff = Duration::from_millis(400 * 2u64.pow(failures.min(4))).min(Duration::from_secs(4));
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(backoff) => continue 'connect,
            }
        }
    }
}

// ----- HTTP -------------------------------------------------------------------------------

async fn accept_loop(inner: Arc<Inner>, mut listener: TcpListener) {
    let mut errors = 0u32;
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                errors = 0;
                let _ = stream.set_nodelay(true);
                inner.handle.spawn(serve_connection(inner.clone(), stream));
            }
            Err(e) => {
                // iOS may reclaim listening sockets while the app is suspended: rebind.
                errors += 1;
                log::warn!("media server accept failed: {e}");
                tokio::time::sleep(Duration::from_millis(200)).await;
                if errors >= 3 {
                    let addr = *lock(&inner.addr);
                    let rebound = match TcpListener::bind(addr).await {
                        Ok(l) => Ok(l),
                        Err(_) => TcpListener::bind(("127.0.0.1", 0)).await,
                    };
                    if let Ok(l) = rebound {
                        if let Ok(a) = l.local_addr() {
                            *lock(&inner.addr) = a;
                        }
                        listener = l;
                        errors = 0;
                    }
                }
            }
        }
    }
}

struct Request {
    method: String,
    path: String,
    range: Option<String>,
    close: bool,
}

async fn read_request(reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>) -> std::io::Result<Option<Request>> {
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            return Ok(None);
        }
        if !line.trim().is_empty() {
            break;
        }
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let http10 = parts.next().is_some_and(|v| v.eq_ignore_ascii_case("HTTP/1.0"));
    let mut req = Request { method, path, range: None, close: http10 };
    for _ in 0..100 {
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            return Ok(None);
        }
        let l = line.trim_end();
        if l.is_empty() {
            return Ok(Some(req));
        }
        if let Some((name, value)) = l.split_once(':') {
            let value = value.trim();
            if name.eq_ignore_ascii_case("range") {
                req.range = Some(value.to_string());
            } else if name.eq_ignore_ascii_case("connection") {
                req.close = value.eq_ignore_ascii_case("close");
            }
        }
    }
    Ok(None)
}

async fn serve_connection(inner: Arc<Inner>, stream: TcpStream) {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    loop {
        let req = match tokio::time::timeout(KEEP_ALIVE, read_request(&mut reader)).await {
            Ok(Ok(Some(r))) => r,
            _ => return,
        };
        match handle(&inner, &req, &mut write).await {
            Ok(true) if !req.close => continue,
            _ => return,
        }
    }
}

async fn respond(
    w: &mut OwnedWriteHalf,
    status: u16,
    content_type: &str,
    extra: &[(&str, String)],
    body: &[u8],
    head_only: bool,
) -> std::io::Result<bool> {
    let reason = match status {
        200 => "OK",
        206 => "Partial Content",
        404 => "Not Found",
        405 => "Method Not Allowed",
        410 => "Gone",
        416 => "Range Not Satisfiable",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Status",
    };
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\n",
        body.len()
    );
    for (k, v) in extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    w.write_all(head.as_bytes()).await?;
    if !head_only {
        w.write_all(body).await?;
    }
    Ok(true)
}

async fn handle(inner: &Arc<Inner>, req: &Request, w: &mut OwnedWriteHalf) -> std::io::Result<bool> {
    let head_only = req.method == "HEAD";
    if req.method != "GET" && !head_only {
        return respond(w, 405, "text/plain", &[], b"method not allowed", false).await;
    }
    let path = req.path.split('?').next().unwrap_or_default();
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match segments.as_slice() {
        ["live", token, file] => serve_live(inner, token, file, w, head_only).await,
        ["vod", token, file] => serve_vod(inner, token, file, req, w).await,
        _ => respond(w, 404, "text/plain", &[], b"not found", head_only).await,
    }
}

async fn serve_live(
    inner: &Arc<Inner>,
    token: &str,
    file: &str,
    w: &mut OwnedWriteHalf,
    head_only: bool,
) -> std::io::Result<bool> {
    let session = lock(&inner.state).live.clone().filter(|s| s.token == token);
    let Some(s) = session else {
        return respond(w, 410, "text/plain", &[], b"stream stopped", head_only).await;
    };
    let resume = {
        let mut d = lock(&s.data);
        d.last_access = Instant::now();
        !d.running && d.state != StreamState::Failed
    };
    if resume {
        inner.clone().spawn_live(s.clone());
    }

    if file == "index.m3u8" {
        let deadline = tokio::time::Instant::now() + START_TIMEOUT;
        loop {
            let notified = s.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let d = lock(&s.data);
                if d.segments.len() >= START_SEGMENTS || d.state == StreamState::Failed {
                    break;
                }
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                break;
            }
        }
        let (empty, message) = {
            let d = lock(&s.data);
            (d.segments.is_empty(), d.message.clone())
        };
        if empty {
            let msg = message.unwrap_or_else(|| "the channel did not start".into());
            return respond(w, 503, "text/plain", &[], msg.as_bytes(), head_only).await;
        }
        let body = s.playlist();
        return respond(w, 200, "application/vnd.apple.mpegurl", &[], body.as_bytes(), head_only).await;
    }

    let seq = file.strip_suffix(".ts").and_then(|n| n.parse::<u64>().ok());
    let segment = seq.and_then(|n| lock(&s.data).segments.iter().find(|x| x.sequence == n).cloned());
    match segment {
        Some(seg) => respond(w, 200, "video/mp2t", &[], &seg.data, head_only).await,
        None => respond(w, 404, "text/plain", &[], b"segment expired", head_only).await,
    }
}

async fn serve_vod(
    inner: &Arc<Inner>,
    token: &str,
    file: &str,
    req: &Request,
    w: &mut OwnedWriteHalf,
) -> std::io::Result<bool> {
    let head_only = req.method == "HEAD";
    let session = lock(&inner.state).vod.clone().filter(|v| v.token == token);
    let Some(vod) = session else {
        return respond(w, 410, "text/plain", &[], b"stream stopped", head_only).await;
    };
    match (&vod.ts, file) {
        (Some(ts), "index.m3u8") => {
            let body = hls::render_vod_playlist(&ts.durations);
            respond(w, 200, "application/vnd.apple.mpegurl", &[], body.as_bytes(), head_only).await
        }
        (Some(ts), _) => {
            let range = file.strip_suffix(".ts").and_then(|n| n.parse::<usize>().ok()).and_then(|n| ts.range(n).map(|r| (n, r)));
            let Some((n, (from, to))) = range else {
                return respond(w, 404, "text/plain", &[], b"no such segment", head_only).await;
            };
            if head_only {
                return respond(w, 200, "video/mp2t", &[], &[], true).await;
            }
            let _queue = vod.fetch.lock().await;
            let lease = inner.acquire().await;
            let mut data = match vod.fetch(inner, from, to, &lease).await {
                Ok(d) => {
                    inner.finish(&lease);
                    d
                }
                Err(e) => {
                    log::warn!("movie segment {n} failed: {}", e.message());
                    inner.abandon(&lease);
                    return respond(w, 502, "text/plain", &[], e.message().as_bytes(), false).await;
                }
            };
            data.truncate(data.len() / TS_PACKET * TS_PACKET);
            if let Some(patched) = &ts.probe.patched_pmt {
                hls::patch_pmt_packets(&mut data, ts.probe.pmt_pid, patched);
            }
            // Later segments start mid-stream: lead with the program tables.
            let body = if n > 0 { [ts.probe.psi.as_slice(), &data].concat() } else { data };
            respond(w, 200, "video/mp2t", &[], &body, false).await
        }
        (None, _) => serve_passthrough(inner, &vod, file, req, w).await,
    }
}

/// Byte-range pass-through for files AVPlayer can read itself (MP4).
async fn serve_passthrough(
    inner: &Arc<Inner>,
    vod: &VodSession,
    file: &str,
    req: &Request,
    w: &mut OwnedWriteHalf,
) -> std::io::Result<bool> {
    let head_only = req.method == "HEAD";
    let upstream = lock(&vod.resolved).clone().unwrap_or_else(|| vod.upstream.clone());
    // On failure the redirect target may have expired: use the original URL next time.
    let reset = || *lock(&vod.resolved) = None;
    let lease = inner.acquire().await;
    if lease.cancel.is_cancelled() {
        return respond(w, 410, "text/plain", &[], b"superseded", head_only).await;
    }
    // Providers often reset HEAD requests, so a HEAD is answered from a 1-byte GET.
    let range = if head_only { Some("bytes=0-0".to_string()) } else { req.range.clone() };
    let mut rb = inner.http.get(&upstream);
    if let Some(r) = &range {
        rb = rb.header(reqwest::header::RANGE, r);
    }
    let resp = tokio::select! {
        _ = lease.cancel.cancelled() => return respond(w, 410, "text/plain", &[], b"superseded", head_only).await,
        r = rb.send() => r,
    };
    let mut resp = match resp {
        Ok(r) => r,
        Err(e) => {
            inner.abandon(&lease);
            reset();
            let msg = fluxtv_core::Error::from(e).to_string();
            return respond(w, 502, "text/plain", &[], msg.as_bytes(), head_only).await;
        }
    };
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) && status != 416 {
        inner.abandon(&lease);
        reset();
        let msg = format!("provider returned HTTP {status}");
        return respond(w, 502, "text/plain", &[], msg.as_bytes(), head_only).await;
    }

    let header = |name: reqwest::header::HeaderName| {
        resp.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_string)
    };
    let content_type = header(reqwest::header::CONTENT_TYPE)
        .filter(|t| !t.starts_with("application/octet-stream"))
        .unwrap_or_else(|| mime_for(file).to_string());
    let content_range = header(reqwest::header::CONTENT_RANGE);
    let length = header(reqwest::header::CONTENT_LENGTH).and_then(|l| l.parse::<u64>().ok());

    if head_only {
        inner.abandon(&lease);
        let total = content_range.as_deref().and_then(|r| r.rsplit('/').next()).and_then(|t| t.parse::<u64>().ok());
        let mut head = format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nAccept-Ranges: bytes\r\n");
        if let Some(t) = total.or(length) {
            head.push_str(&format!("Content-Length: {t}\r\n"));
        }
        head.push_str("\r\n");
        w.write_all(head.as_bytes()).await?;
        return Ok(true);
    }

    let reason = match status {
        206 => "Partial Content",
        416 => "Range Not Satisfiable",
        _ => "OK",
    };
    let mut head = format!("HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nAccept-Ranges: bytes\r\n");
    if let Some(range) = &content_range {
        head.push_str(&format!("Content-Range: {range}\r\n"));
    }
    let keep_alive = length.is_some();
    match length {
        Some(l) => head.push_str(&format!("Content-Length: {l}\r\n")),
        None => head.push_str("Connection: close\r\n"),
    }
    head.push_str("\r\n");
    w.write_all(head.as_bytes()).await?;

    let mut sent = 0u64;
    loop {
        let chunk = tokio::select! {
            _ = lease.cancel.cancelled() => return Ok(false),
            c = resp.chunk() => c,
        };
        match chunk {
            Ok(Some(bytes)) => {
                if let Err(e) = w.write_all(&bytes).await {
                    inner.abandon(&lease);
                    return Err(e);
                }
                sent += bytes.len() as u64;
            }
            Ok(None) => break,
            Err(e) => {
                log::warn!("movie upstream interrupted: {}", fluxtv_core::Error::from(e));
                inner.abandon(&lease);
                return Ok(false);
            }
        }
    }
    inner.finish(&lease);
    Ok(keep_alive && length == Some(sent))
}

fn mime_for(file: &str) -> &'static str {
    match file.rsplit('.').next().unwrap_or_default() {
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "ts" => "video/mp2t",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    async fn get(addr: &str, path: &str) -> (u16, Vec<u8>) {
        let mut s = TcpStream::connect(addr).await.unwrap();
        s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).await.unwrap();
        let split = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let head = String::from_utf8_lossy(&buf[..split]).to_string();
        let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        (status, buf[split + 4..].to_vec())
    }

    /// A fake provider that serves an endless 25 fps MPEG-TS stream (one packet group per frame).
    async fn fake_provider() -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (mut s, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let _ = tokio::io::AsyncReadExt::read(&mut s, &mut buf).await;
                    let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: video/mp2t\r\n\r\n").await;
                    let mut frame = 0u64;
                    loop {
                        let chunk = crate::media::tests::ts_frame(frame);
                        frame += 1;
                        if s.write_all(&chunk).await.is_err() {
                            return;
                        }
                        // Send faster than real time so the test doesn't take seconds.
                        if frame.is_multiple_of(25) {
                            tokio::time::sleep(Duration::from_millis(5)).await;
                        }
                    }
                });
            }
        });
        format!("http://{addr}/live/u/p/1.ts")
    }

    pub(crate) fn ts_frame(frame: u64) -> Vec<u8> {
        fn packet(pid: u16, pusi: bool, rai: bool, payload: &[u8], cc: u8) -> Vec<u8> {
            let mut p = vec![0x47, (if pusi { 0x40 } else { 0 }) | (pid >> 8) as u8, pid as u8];
            let body = &payload[..payload.len().min(if rai { 182 } else { 184 })];
            if rai || body.len() < 184 {
                let af = 183 - body.len();
                p.push(0x30 | cc);
                p.push(af as u8);
                if af > 0 {
                    p.push(if rai { 0x40 } else { 0 });
                    p.extend(std::iter::repeat_n(0xff, af - 1));
                }
            } else {
                p.push(0x10 | cc);
            }
            p.extend_from_slice(body);
            p
        }
        fn section(t: &[u8]) -> Vec<u8> {
            let mut s = vec![0u8];
            s.extend_from_slice(t);
            s.extend_from_slice(&hls::crc32_mpeg2(t).to_be_bytes());
            s
        }
        let cc = (frame & 0x0f) as u8;
        let mut out = Vec::new();
        if frame.is_multiple_of(25) {
            out.extend(packet(0, true, false, &section(&[0, 0xb0, 13, 0, 1, 0xc1, 0, 0, 0, 1, 0xf0, 0]), cc));
            let pmt = [0x02, 0xb0, 18, 0, 1, 0xc1, 0, 0, 0xe1, 0, 0xf0, 0, 0x1b, 0xe1, 0, 0xf0, 0];
            out.extend(packet(0x1000, true, false, &section(&pmt), cc));
        }
        let pts = 90_000 + frame * 3600;
        let key = frame.is_multiple_of(50);
        let mut pes = vec![0, 0, 1, 0xe0, 0, 0, 0x80, 0x80, 5];
        pes.push(0x21 | (((pts >> 30) & 7) as u8) << 1);
        pes.push((pts >> 22) as u8);
        pes.push(0x01 | (((pts >> 15) & 0x7f) as u8) << 1);
        pes.push((pts >> 7) as u8);
        pes.push(0x01 | ((pts & 0x7f) as u8) << 1);
        pes.extend([0, 0, 1, if key { 0x65 } else { 0x41 }]);
        pes.resize(150, 0xaa);
        out.extend(packet(0x100, true, key, &pes, cc));
        out
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn serves_live_hls_from_ts_upstream() {
        let upstream = fake_provider().await;
        let server = MediaServer::start(reqwest::Client::new(), Handle::current()).await.unwrap();
        let url = server.start_live(upstream);
        let (addr, path) = url.trim_start_matches("http://").split_once('/').unwrap();
        let (status, body) = get(addr, &format!("/{path}")).await;
        let playlist = String::from_utf8(body).unwrap();
        assert_eq!(status, 200, "{playlist}");
        assert!(playlist.starts_with("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n"), "{playlist}");
        let first = playlist.lines().find(|l| l.ends_with(".ts")).unwrap();
        let (status, seg) = get(addr, &format!("/{}", path.replace("index.m3u8", first))).await;
        assert_eq!(status, 200);
        assert_eq!(seg[0], 0x47);
        assert_eq!(seg.len() % 188, 0);
        assert_eq!(server.status().state, StreamState::Playing);
        assert_eq!(server.status().video_codec.as_deref(), Some("H.264"));

        // Switching streams stops the old session.
        server.stop();
        let (status, _) = get(addr, &format!("/{path}")).await;
        assert_eq!(status, 410);
    }
}
