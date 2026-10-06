# FluxTV: improvements, new features and useful services

Review date: 2026-10-04 · Codebase version: 0.2.0 (`0ca0dff`)

This review covers the shared Rust core (`crates/fluxtv-core`), the Apple FFI and media server
(`crates/fluxtv-ffi`), the Windows app (`apps/windows`, Tauri 2 + React) and the Apple app
(`apps/apple`, SwiftUI). Each item says what the code does **today**, then what to change and **how**.
File references are relative to the repository root.

The foundation is strong. The core is platform-independent and well tested, imports stream instead
of building a DOM, credentials stay off the UI, and both apps keep to a single upstream connection.
Most suggestions below build on that work rather than replace it.

---

## Part 1: 20 features to improve

### Security and data

#### 1. Store Windows credentials in Windows Credential Manager
**Today:** On Windows, the Xtream password is stored in plain text in the `sources.password` column of
`%APPDATA%\com.fluxtv.app\fluxtv.db` ([store.rs](../crates/fluxtv-core/src/store.rs) schema). The welcome
screen says "credentials are stored locally on this PC", which is true, but anyone who copies the file can
read them. The Apple apps already avoid this with Keychain items and `Store::set_runtime_password`.
**How:** Use the [`keyring`](https://lib.rs/crates/keyring) crate with the `windows-native` feature.
On startup, load each source's secret and call the existing `set_runtime_password`. Add a one-time
migration that moves existing passwords into Credential Manager and sets the column to `NULL`. M3U playlist
URLs and the `m3u_epg_url.{id}` setting often embed credentials too, so store them the same way. This item
is already on the README roadmap.

#### 2. Use real schema migrations and allow concurrent reads
**Today:** `Store::migrate()` ([store.rs:193](../crates/fluxtv-core/src/store.rs#L193)) runs
`CREATE TABLE IF NOT EXISTS` and writes `schema_version = 1`. The first new column (several items below
need one) has no upgrade path. All reads also share one `Mutex<Connection>`
([store.rs:110](../crates/fluxtv-core/src/store.rs#L110)), so a slow guide query blocks the channel list.
**How:** Adopt [`rusqlite_migration`](https://docs.rs/crate/rusqlite_migration/latest), which tracks the
version in `PRAGMA user_version` and offers a `.validate()` test helper. Keep the current schema as
migration 1. Because the database already uses WAL mode, add a small pool of read-only connections
(2–3) for queries and keep the single writer for imports.

#### 3. Type the Windows IPC, return structured errors and restrict settings access
**Today:** The types in [api.ts](../apps/windows/src/api.ts) are maintained by hand to mirror the Rust
structs, and they have already drifted: `ContentKind` is `"live" | "movie"` while Rust also has `series`.
Every command error is flattened to a string, so the UI cannot tell "wrong password" from "provider
offline". `get_setting` and `set_setting` ([commands.rs:277](../apps/windows/src-tauri/src/commands.rs#L277))
accept any key, so the webview can read `m3u_epg_url.*` values that may contain credentials.
**How:** Generate bindings and typed events with
[tauri-specta](https://github.com/specta-rs/tauri-specta). Serialize
`fluxtv_core::Error` as a tagged enum (`{ kind: "auth" | "network" | "status" | … , message }`), as the
Swift side already gets through `FluxError`. Replace the generic settings commands with typed ones, or with
an allowlist of UI keys.

### Playback

#### 4. Play AC-3 and E-AC-3 (and more) in the Windows built-in player
**Today:** The built-in player is mpegts.js on Media Source Extensions. WebView2 cannot decode Dolby
audio there, so these channels show a warning and offer an external player
([Player.tsx](../apps/windows/src/components/Player.tsx), `unsupportedAudio`). Many European DVB
channels carry AC-3 audio.
**How (pick one):**
- **Best long-term option:** embed libmpv with
  [tauri-plugin-libmpv](https://depscope.dev/pkg/npm/tauri-plugin-libmpv-api). This handles every codec
  and container (MKV, MPEG-2, AC-3, DTS) and adds hardware decoding and track selection.
  [MaxVideoPlayer](https://gitblind.noratr.app/MaxMB15/MaxVideoPlayer) is an IPTV player built on Tauri 2
  + React + libmpv that shows this works. Use the LGPL builds of `libmpv-2.dll`.
- **Quickest fix:** ship ffmpeg as a Tauri sidecar and, only when the PMT shows AC-3/E-AC-3, have the
  proxy pipe the stream through `ffmpeg -c:v copy -c:a aac -f mpegts`. The core's `hls::Segmenter`
  already parses the PMT (`StreamInfo`), so the proxy can make this decision before the player starts.
- **Browser-only option:** decode in WASM with [@mediabunny/ac3](https://mediabunny.dev/guide/extensions/ac3).
  This is more work to synchronize with MSE video.

#### 5. Start channels faster on both platforms
**Today:** On Apple, the media server waits for `START_SEGMENTS = 3` segments of about 2 s each
([media.rs:38](../crates/fluxtv-ffi/src/media.rs#L38)) after a 300 ms switch grace period, and AVPlayer
starts three target durations from the live edge. That means roughly 6 s or more of buffering before the
first picture. On Windows, mpegts.js runs with a 384 KB stash buffer
([Player.tsx:119](../apps/windows/src/components/Player.tsx#L119)).
**How:**
- Cut the first few segments at every keyframe (about 1 s) and switch to about 2 s segments once playback
  has started.
- Set `automaticallyWaitsToMinimizeStalling = false` and a small `preferredForwardBufferDuration` for
  live items.
- Return the playlist as soon as two short segments exist.
- On Windows, lower `stashInitialSize` for the first second.

Add a "time to first frame" log line on both platforms so each change can be measured.

#### 6. Let the user choose the stream format (TS or HLS)
**Today:** Xtream live URLs are always `.ts` ([xtream.rs:185](../crates/fluxtv-core/src/xtream.rs#L185)),
even when the account's `allowed_output_formats` includes `m3u8`. On Apple, every channel therefore goes
through the local segmenter, although `FluxEngine::play` already passes `.m3u8` URLs straight to AVPlayer.
**How:** Add a per-source setting (Auto / MPEG-TS / HLS). In *Auto*, prefer `m3u8` on Apple when the
account allows it, and fall back to TS when it fails. Keep TS on Windows, where mpegts.js is the engine.

#### 7. Share one media layer between Windows and Apple
**Today:** Two separate proxies implement the "one upstream connection" rule: an axum proxy on Windows
([proxy.rs](../apps/windows/src-tauri/src/proxy.rs)) and a hand-written HTTP server on Apple
([media.rs](../crates/fluxtv-ffi/src/media.rs), about 1,000 lines). They have drifted. On Windows, every
request cancels the previous one and sleeps 250 ms ([proxy.rs:127](../apps/windows/src-tauri/src/proxy.rs#L127)),
including byte-range requests inside the same movie, so each seek pays that delay. The Apple server queues
requests from the same movie, and it is also the only one that handles server-side reconnects, codec
detection and MKV sniffing.
**How:** Extract a `crates/fluxtv-media` crate that owns the lease, upstream reconnects, VOD sniffing and
live HLS, and serve it with axum on both platforms (axum is already compiled for Windows). Windows then
gains early codec warnings and server-side reconnects, and both apps get one implementation to fix.

#### 8. Send events from Rust to Swift instead of polling
**Today:** Swift polls `refresh_progress` every 400 ms ([AppModel.swift:212](../apps/apple/FluxTV/App/AppModel.swift#L212))
and `playback_status` every 2 s ([PlayerModel.swift:224](../apps/apple/FluxTV/Player/PlayerModel.swift#L224)).
`FluxEngine::play` is a synchronous call that uses `block_on` for movies
([lib.rs:359](../crates/fluxtv-ffi/src/lib.rs#L359)).
**How:** UniFFI supports [callback interfaces and async foreign traits](https://mozilla.github.io/uniffi-rs/0.30/futures.html).
Export an `EngineObserver` trait (`refresh_progress`, `playback_state`, `catalog_changed`) that Swift
implements and bridges to an `AsyncStream`. Make `play` an `async fn`, like `validate_account`. The UI
then updates instantly and stops waking the CPU for nothing on Apple TV.

#### 9. Make playback errors specific and add diagnostics
**Today:** Failures end in generic messages such as "the channel may be offline, or another device is
using your connection". There is no way to export logs for a bug report.
**How:**
- When a stream fails, call `account()` and, if `active_connections >= max_connections`, say so directly
  ("Your account is in use on another device").
- Map HTTP 401/403/404/458/509 to specific messages.
- Add a "Stream info" panel (codec, resolution, bitrate, reconnects, dropped frames). `PlaybackStatus`
  and mpegts.js `STATISTICS_INFO` already provide most of these values.
- Add "Export diagnostics": a rotating log file with credentials scrubbed. `Error::from(reqwest::Error)`
  already strips URLs, so reuse that.

### Catalog and guide

#### 10. Make catalog refresh parallel, retried and safe against empty responses
**Today:** Xtream live and movie catalogs are fetched one after the other
([library.rs:210](../crates/fluxtv-core/src/library.rs#L210)), with no retries. `replace_catalog` deletes
the old catalog unconditionally, so a provider that returns `[]` under load wipes the user's channel list.
Windows skips the guide when the catalog fails ([commands.rs:117](../apps/windows/src-tauri/src/commands.rs#L117)),
while Apple continues with it.
**How:**
- Fetch categories and streams for every kind concurrently with `tokio::try_join!`.
- Retry timeouts and 5xx responses with backoff (`reqwest-middleware` + `reqwest-retry`, or a small loop).
- Refuse to replace a catalog when the new one is empty, or less than, say, 20% of the old one, and report
  "provider returned an incomplete list".
- On Windows, still refresh the guide when only the catalog failed, as Apple does.

#### 11. Keep memory bounded when importing playlists, guides and JSON
**Today:** `FluxCore::download` collects the whole response in a `Vec<u8>`
([library.rs:173](../crates/fluxtv-core/src/library.rs#L173)) before parsing. reqwest's `gzip` feature
decompresses transfer-encoded responses, so the buffer holds the full XML. Xtream's `xmltv.php` is often
hundreds of MB uncompressed, which is risky within Apple TV's memory limits. Xtream JSON is
parsed into a `serde_json::Value` and then converted again with `from_value`
([xtream.rs:236–290](../crates/fluxtv-core/src/xtream.rs#L236)), so two full copies of a 100k-entry VOD list
are in memory at once.
**How:**
- Stream guide downloads to a temporary file, or feed the parser on a blocking thread through
  `tokio_util::io::StreamReader` + `SyncIoBridge`. `quick-xml` already parses incrementally.
- Deserialize JSON directly into `Vec<RawStream>`; the lenient field deserializers already exist.
- Use a sequence visitor so one malformed entry is skipped instead of failing the whole list.

#### 12. Search: indexed, typo-tolerant, and across all content
**Today:** Search runs `LIKE '%token%'` plus a `REPLACE(name_norm, ' ', '') LIKE` over every row
([store.rs:423](../crates/fluxtv-core/src/store.rs#L423)). That is fast for about 55k live channels, but it
scans the whole table, which becomes slow with large VOD or series catalogs, and it has no typo tolerance.
Search covers one kind at a time.
**How:**
- Add an external-content FTS5 table with `tokenize='trigram'`. Trigram indexes preserve `LIKE`
  substring semantics but are indexed ([SQLite forum](https://sqlite.org/forum/forumpost/76b444bebe?t=c)).
  Keep the `LIKE` path for tokens shorter than 3 characters.
- Re-rank the candidates with [`nucleo-matcher`](https://docs.rs/crate/nucleo-matcher/latest), so
  `nkr1` still finds `NRK1`.
- Show one result list with Live / Movies / Series sections.

#### 13. Improve guide matching and let users fix it
**Today:** `resolve_epg_matches` ([store.rs:590](../crates/fluxtv-core/src/store.rs#L590)) matches names
exactly, then with a looser key, and the first guide channel that matches wins. There is no UI to correct a
wrong or missing match, and no correction for providers whose XMLTV times are off by an hour.
**How:**
- Score candidates (Jaro–Winkler from `strsim`, plus a bonus when the guide id's country suffix such as
  `.no` or `.uk` matches the channel prefix such as `NOR|`).
- Add the alternative names from the iptv-org database (see Part 3).
- Store user overrides in an `epg_overrides` table that survives refreshes, and add a "Wrong guide? Pick
  channel…" action that searches `epg_channels`.
- Add a per-source "Guide time offset" setting, applied at import. TiviMate offers the same setting.

#### 14. Make the TV guide navigable and scalable
**Today:** The Windows guide shows a fixed window from 2 h back to 22 h ahead
([Guide.tsx:13](../apps/windows/src/components/Guide.tsx#L13)). The Apple guide shows 12 h, at most 250
channels, and builds every row up front with a non-lazy `VStack` ([GuideView.swift:15](../apps/apple/FluxTV/Views/GuideView.swift#L15)).
**How:**
- Add a day picker, a "Now" button and "jump to time".
- Load programmes for the visible time range only, and extend the range on horizontal scroll.
- On Apple, switch to `LazyVStack` with lazy row loading and remove the 250-channel cap.
- Color programmes by category (sports, news, film); the `category` field is already stored.
- Use arrow keys in the Windows grid and focus movement on tvOS.

#### 15. Honor per-stream HTTP headers and allow a custom User-Agent
**Today:** `#EXTVLCOPT:http-user-agent`, `http-referrer` and `#KODIPROP` lines are ignored
([m3u.rs:120](../crates/fluxtv-core/src/m3u.rs#L120)). Streams that need them fail. Every request uses
`FluxTV/0.2.0`, which providers that allowlist known players may reject. The `url-tvg` header keeps only the first of several
comma-separated guides ([m3u.rs:105](../crates/fluxtv-core/src/m3u.rs#L105)).
**How:** Parse these options into an `M3uEntry.headers` map, store them in a JSON column on `channels`, and
apply them in the proxy and media server. Add a per-source User-Agent override with presets
("VLC", "TiviMate", "Kodi"). Keep all guide URLs (see feature 11).

### Interface

#### 16. Move display-name parsing into the core
**Today:** `splitPrefix` and `initials` exist twice, and the two versions behave differently: Swift strips
a leading `|` (as in `|SPT| SPORT BOXING`) and TypeScript does not ([util.ts:48](../apps/windows/src/util.ts#L48)
vs [Formatting.swift:43](../apps/apple/FluxTV/Shared/Formatting.swift#L43)).
**How:** Compute `display_name`, `prefix` and quality tags (`UHD`, `FHD`, `HEVC`, `RAW`, `50fps`) in
`search.rs` during import, store them, and return them on `Channel`. Both front ends then show the same
names and can show quality badges. This also enables grouping of variants (feature 10).

#### 17. Load and cache logos better on Windows, with fallbacks
**Today:** The webview loads every logo directly from provider hosts (CSP `img-src http: https:`). The only
cache is WebView2's HTTP cache, dead links are requested again each session, and the user's IP address is
sent to many third-party hosts. The Apple `ImageCache` remembers failures only in memory.
**How:** Serve logos through the local proxy (`/logo/<hash>`), with a disk cache, a persistent
"known dead" list and downscaling (`image` crate). When a logo is missing or dead, fall back to
[tv-logos](https://github.com/tv-logo/tv-logos) or the iptv-org `logos.json`,
matched by guide id or name.

#### 18. Favorites: custom order, sync between devices, export
**Today:** Favorites are always ordered by date added ([store.rs:486](../crates/fluxtv-core/src/store.rs#L486)).
On Apple, the favorites backup lives in per-device `UserDefaults`
([AppModel.swift:280](../apps/apple/FluxTV/App/AppModel.swift#L280)), so favorites added on iPhone do not
appear on Apple TV, even though accounts sync through iCloud Keychain.
**How:**
- Add a `position` column, with drag to reorder on Windows (dnd-kit) and `.onMove` on iOS.
- Move the Apple backup to [`NSUbiquitousKeyValueStore`](https://developer.apple.com/documentation/foundation/nsubiquitouskeyvaluestore.md).
  Its limits (1 MB, 1,024 keys) are plenty for id lists, and the `didChangeExternallyNotification`
  observer can merge changes.
- On Windows, add JSON export and import of favorites and settings.

#### 19. Movies: a proper browser and details page
**Today:** On Windows, movies appear in the same list layout as channels, with no posters and no details.
On Apple, they appear in a poster grid with no details page. The Xtream fields `added`, `rating` and
`tmdb` are not parsed in `RawStream` ([xtream.rs](../crates/fluxtv-core/src/xtream.rs)).
**How:**
- Parse `added` and `rating` and add sort options: recently added, rating, A–Z.
- Fetch `get_vod_info` when a movie is opened (plot, cast, genre, runtime, backdrop, trailer, `tmdb_id`)
  and cache it in a `vod_info` table.
- Add a poster grid on Windows and a details screen with Play / Resume on both platforms.

### Engineering

#### 20. Add CI, linting, fuzzing and frontend tests
**Today:** There is no GitHub Actions workflow; only Xcode Cloud builds Apple. The React code has
`eslint-disable` comments, but ESLint is not installed. There are no frontend tests. The M3U, XMLTV and TS
parsers all process untrusted network input but have no fuzzing.
**How:**
- Add a GitHub Actions workflow that runs `cargo fmt --check`, `cargo clippy -D warnings`, and
  `cargo test -p fluxtv-core -p fluxtv-ffi` on macOS and Windows, plus `pnpm tsc && pnpm build` and a
  `tauri build` artifact for Windows.
- Add `typescript-eslint` and Vitest (for `util.ts`, `store.ts` and the EPG cache).
- Add `cargo-fuzz` targets for `m3u::parse`, `xmltv::parse` and `hls::Segmenter::push`, and `proptest` for
  the timestamp and PTS-wrap math.

---

## Part 2: 20 features worth adding

#### 1. Series (TV shows)
Series are on the roadmap. M3U `/series/` entries are already imported as `ContentKind::Series` but appear
in neither app (the Windows `ContentKind` type has no `series`, and the Swift views never use it).
**How:** Fetch `get_series_categories` and `get_series` with the catalog. Fetch `get_series_info` (seasons,
episodes, plots) when a show is opened and cache it. Play episodes with the existing VOD path
(`/series/u/p/{episode_id}.{ext}`). Add "Next episode" with auto-play and season/episode resume.

#### 2. Catch-up and archive playback
`catchup_days` is parsed and shown as a badge, but nothing can be played yet.
**How:**
- Xtream: `/timeshift/{user}/{pass}/{duration_min}/{YYYY-MM-DD:HH-MM}/{stream_id}.ts`.
- M3U: support the `catchup`, `catchup-source` and `catchup-days` attributes with the `default`, `append`,
  `shift` and `flussonic` modes used by Kodi IPTV Simple ([spec summary](https://uniplayer.net/docs/specs/catchup/)).
- In both guides, past programmes on catch-up channels get a "Watch from start" action. The playing live
  programme gets "Restart".

#### 3. Pause and rewind live TV (local timeshift buffer)
Today, pausing releases the connection after 30 s ([media.rs:43](../crates/fluxtv-ffi/src/media.rs#L43)),
the media server keeps only 12 segments (about 24 s), and resuming jumps to live.
**How:** Spill segments to a ring buffer on disk (for example 30–90 minutes in Caches), publish a playlist
with a long DVR window so AVPlayer shows a scrubber, and keep the upstream open while paused (with a
setting, because it holds the account's connection). On Windows, the shared media layer (improvement 7)
would let the WebView play the same HLS through hls.js.

#### 4. Recording (DVR) and scheduled recordings
**How:** On Windows, write the upstream TS to disk alongside playback, or instead of it. Add "Record" to
programme details and the guide, with the start and stop times taken from the guide plus padding. Use a
scheduler that respects the single-connection limit and warns about conflicts. Recordings appear in a
"Recordings" section and play through the existing VOD path. On Apple, background limits make this
practical only while the app is in the foreground, or on a Mac.

#### 5. Programme reminders
**How:** Add "Remind me" on future programmes. Use local notifications (`UNUserNotificationCenter` on
Apple, `tauri-plugin-notification` on Windows) that tune to the channel when tapped. Store reminders by
guide id and start time so they survive guide refreshes.

#### 6. Programme search and "What's on" by genre
**How:** Add an FTS5 index over `programmes(title, subtitle, description)` and a "Search the guide" mode
(for example "champions league" → every upcoming match, with channel and time). Add genre filters (Sports,
News, Films, Kids) using `programmes.category`, and a "Starting soon" list.

#### 7. Home screen
**How:** Add a landing view with these rows: *Continue watching* (movies and episodes), *On now in
favorites* (with progress bars), *Starting soon in favorites*, *Recently added movies* (needs `added`, see
improvement 19) and *Recently watched channels*. On tvOS, this view replaces "Live TV" as the first tab.

#### 8. Resume position for movies and episodes
**How:** Add a `playback_positions(source_id, kind, id, position_s, duration_s, updated_at)` table, and
save the position every 10 s (from AVPlayer's periodic time observer or the HTML `timeupdate` event). Offer
"Resume from 42:10" when a movie is opened again, and count anything past 95% as watched. This also feeds
the home screen and Trakt (Part 3).

#### 9. Multi-view
**How:** When `max_connections > 1`, allow 2–4 channels at once (a grid on Windows, multiview on iPad and
Apple TV). Generalize the media lease from one slot to N slots, and route audio to the focused tile.

#### 10. Alternate streams and automatic failover
Providers often carry the same channel several times (HD, FHD, RAW, "backup"), sometimes across several
sources.
**How:** Group variants by matched guide id and normalized display name (improvement 16). If a stream
fails or stalls repeatedly, switch to the next variant, and show a "Quality: Auto / UHD / FHD / HD" picker.
This also works across multiple sources, giving provider-level failover.

#### 11. Multiple and external guide sources
Multiple guide sources per provider are on the roadmap.
**How:** Add an `epg_sources(source_id, url, priority, shift_minutes)` table and import each source. When
several have the same channel, the higher priority wins. Keep all `url-tvg` URLs (improvement 15). Let
users add a public XMLTV URL for channels their provider has no guide for.

#### 12. Parental controls and category management
**How:**
- Hide categories or channels, for example adult groups or foreign-language packs. Store a `hidden` flag
  keyed by id so it survives refreshes.
- Add a PIN lock for selected categories and for Settings, and auto-hide categories whose names match
  adult keywords.
- Allow categories to be renamed and reordered.

The same hide list lets users remove the hundreds of event and placeholder categories providers publish.

#### 13. Custom channel groups
**How:** Add user-created groups ("My Sports", "Kids") that mix channels from any category or source,
with custom channel numbers. Show them in the sidebar or tabs, the guide picker and the zapping playlist.

#### 14. Apple platform integration: Top Shelf, widgets, Siri and deep links
**How:**
- **tvOS Top Shelf extension** ([`TVTopShelfContentProvider`](https://developer.apple.com/documentation/tvservices/tvtopshelfsectionedcontent.md)):
  a sectioned row of favorite channels with what is on now, which tune directly when selected.
- **iOS widgets** (WidgetKit): "On now in favorites".
- **App Intents** for Siri and Shortcuts: "Play NRK1 in FluxTV".
- **A `fluxtv://play?source=…&kind=live&id=…` URL scheme**, which all of the above use.

The shared data these extensions need (favorites and now/next) belongs in an App Group container.

#### 15. Windows system integration: media keys and a mini player
**How:**
- Set `navigator.mediaSession.metadata` (channel, programme, logo) and handle its actions, so hardware
  media keys and the Windows media overlay work through the [Media Session API](https://developer.mozilla.org/en-US/docs/Web/API/Media_Session_API).
- Add Picture-in-Picture (`video.requestPictureInPicture()`) or an always-on-top mini-player window.
- Add `tauri-plugin-single-instance`, so opening a second copy focuses the first one.

#### 16. Auto-update and signed releases on Windows
**How:** Add the [Tauri updater plugin](https://v2.tauri.app/plugin/updater/), which requires signed
update bundles and works with the NSIS installer. Publish `latest.json` on GitHub Releases from the CI
workflow (improvement 20). Sign the installer to avoid SmartScreen warnings, and consider a winget
manifest.

#### 17. Audio and subtitle track selection, and movie subtitles
**How:** On Apple, keep every audio PID in the HLS output and let AVPlayer's built-in menu handle
selection. If needed, add `EXT-X-MEDIA` renditions. On Windows, this mostly comes with libmpv (improvement 4).
For movies, add subtitle search through the [OpenSubtitles REST API](https://www.mintlify.com/logscore/plank/integrations/opensubtitles),
using `tmdb_id` from `get_vod_info`.

#### 18. Casting to Chromecast
AirPlay already works on Apple.
**How:** On Windows, discover devices over mDNS and control them with [`rust_cast`](https://docs.rs/crate/rust_cast).
On iOS, use the Google Cast SDK. The device must be able to reach the stream, so serve a LAN-bound session
with a token (preferably live HLS from the shared media layer), and treat this as an opt-in mode because it
exposes the proxy on the LAN.

#### 19. More platforms: macOS, Linux and Android TV
**How:**
- **macOS and Linux:** the Tauri app is nearly portable. `external.rs` splits `PATH` on `;`
  ([external.rs:17](../apps/windows/src-tauri/src/external.rs#L17)); use `std::env::split_paths` and add
  mpv and VLC paths per OS. On Mac, a "Designed for iPad" or Catalyst build of the Apple app is another
  route.
- **Android and Android TV:** generate Kotlin bindings from the same UniFFI crate and play with Media3
  ExoPlayer, which plays MPEG-TS directly, so no segmenter is needed. Use Compose for TV for the interface
  ([example IPTV app](https://github.com/SERFF/iptv-app)).

#### 20. TV remote conveniences, localization and accessibility
**How:**
- Type a channel number to jump to it (keyboard digits, or the remote's number pad on Android TV).
- Add "Last channel" recall (Backspace, or a long press on Menu), a sleep timer, an option to start with
  the last channel, and an OSD channel banner when zapping.
- Localize with String Catalogs on Apple and i18next or Lingui on Windows, starting with Norwegian.
  Dates already use `Intl` and `formatted()`.
- Make an accessibility pass: `aria-label`s on icon buttons (titles only today), roving focus in lists and
  the guide, and VoiceOver labels for programme progress.

---

## Part 3: Services, APIs and libraries that could help

Grouped by what they would improve. "Fit" ties each one to the items above.

### Playback engines and media processing

| What | Why it helps FluxTV | Fit / notes |
| --- | --- | --- |
| [tauri-plugin-libmpv](https://depscope.dev/pkg/npm/tauri-plugin-libmpv-api) + libmpv | Native mpv engine inside the Tauri window: every codec and container, hardware decoding, track selection | Improvement 4. Needs `libmpv-wrapper.dll` + `libmpv-2.dll` (use LGPL builds). [MaxVideoPlayer](https://gitblind.noratr.app/MaxMB15/MaxVideoPlayer) shows it working in a Tauri + React IPTV player |
| [SwiftVLC](https://github.com/harflabs/SwiftVLC) (libVLC 4 for Swift) | A fallback engine on Apple for what AVPlayer cannot play: MKV, MPEG-2 video, DTS | iOS/tvOS 18+, `@Observable`, PiP; MIT wrapper, LGPL libVLC. The release zip is a few hundred MB, so use it only as a fallback |
| [matroska-demuxer](https://docs.rs/matroska-demuxer) + [muxide](https://docs.rs/crate/muxide/latest) | Pure-Rust MKV → fragmented MP4, which would make the roadmap item "MKV via fMP4 HLS in the core" possible without FFmpeg | muxide writes H.264/HEVC/AV1 + AAC fMP4; check AC-3/E-AC-3 support before relying on it. [sheathe](https://docs.rs/crate/sheathe-cli/0.3.0) only demuxes VP8/VP9/AV1/Opus from MKV, so it is not suitable for typical IPTV movies |
| FFmpeg as a Tauri sidecar | On-the-fly AC-3 → AAC audio transcoding (video copied) for the Windows built-in player | Improvement 4. Use LGPL builds. **Avoid FFmpegKit** on Apple: it was [retired in 2025](https://cdn.jsdelivr.net/gh/arthenica/ffmpeg-kit@main/README.md) |
| [@mediabunny/ac3](https://mediabunny.dev/guide/extensions/ac3) | WASM AC-3/E-AC-3 decoder built from FFmpeg's decoder, for browser-only playback | More work to synchronize with MSE; an option if native code is not wanted |
| [Media3 ExoPlayer](https://developer.android.com/media/implement/surfaces/android-tv) | The natural player for an Android TV front end; plays TS and HLS directly | Feature 19 |

### Channel, guide and metadata sources

| What | Why it helps FluxTV | Fit / notes |
| --- | --- | --- |
| [iptv-org API](https://github.com/iptv-org/api) (`channels.json`, `logos.json`, `guides.json`) | An open channel database with alternative names, countries, logos and links to guide sources: better guide matching and logo fallbacks | Improvements 13 and 17; feature 11. Download and cache it periodically; do not query it per channel |
| [iptv-org/epg](https://github.com/iptv-org/epg) | Open-source guide grabber for hundreds of sites; users can generate XMLTV for channels their provider has no guide for | Feature 11 |
| [tv-logos](https://github.com/tv-logo/tv-logos) | 512 px PNG logos for dark backgrounds, organized by country | Improvement 17. Licensed CC BY-SA / BY-NC-SA, "free for personal use": check before bundling in a store app |
| [TMDB API](https://developer.themoviedb.org/docs/faq) | Posters, backdrops, plots, cast and ratings for movies and series, especially for M3U sources without `get_vod_info` | Improvement 19; features 1 and 7. Free for **non-commercial** use with attribution ("This product uses the TMDB API but is not endorsed or certified by TMDB"); a commercial App Store release needs a license |
| Xtream `get_vod_info` / `get_series_info` / `get_short_epg` | Already available from the user's provider: movie details, episodes and a guide fallback for providers with a broken `xmltv.php` | Improvement 19; feature 1. `get_short_epg` can fill now/next when XMLTV is missing |
| [OpenSubtitles REST API](https://www.mintlify.com/logscore/plank/integrations/opensubtitles) | Subtitles for movies and series | Feature 17. Needs a user account for more than a few downloads per day; check current quotas |
| [Trakt API](https://apis.io/apis/trakt/trakt-scrobble-api/) | Scrobbling (start/pause/stop) and watch-history sync for movies and series | Features 1 and 8. Its OAuth device-code flow suits Apple TV |

### Platform APIs (Apple and Windows)

| What | Why it helps FluxTV | Fit |
| --- | --- | --- |
| [`NSUbiquitousKeyValueStore`](https://developer.apple.com/documentation/foundation/nsubiquitouskeyvaluestore.md) | Syncs favorites, hidden categories and settings between iPhone, iPad and Apple TV (1 MB / 1,024 keys) | Improvement 18; feature 12 |
| [TV Services / Top Shelf](https://developer.apple.com/documentation/tvservices.md) | Favorite channels on the Apple TV home screen | Feature 14 |
| WidgetKit, App Intents, `UNUserNotificationCenter` | Widgets, Siri/Shortcuts, reminders | Features 5 and 14 |
| [Media Session API](https://developer.mozilla.org/en-US/docs/Web/API/Media_Session_API) | Hardware media keys and the Windows media overlay from the WebView | Feature 15 |
| [Tauri updater](https://v2.tauri.app/plugin/updater/), notification, single-instance and deep-link plugins | Signed auto-updates, reminders, one running copy, `fluxtv://` links | Features 5, 14, 15 and 16 |

### Rust and TypeScript libraries

| What | Why it helps FluxTV | Fit |
| --- | --- | --- |
| [`keyring`](https://lib.rs/crates/keyring) (`windows-native`) | Windows Credential Manager storage | Improvement 1 |
| [`rusqlite_migration`](https://docs.rs/crate/rusqlite_migration/latest) | Versioned schema migrations using `user_version`, with a validation test helper | Improvement 2 |
| [SQLite FTS5 trigram](https://sqlite.org/forum/forumpost/76b444bebe?t=c) | Indexed substring search for channels and programmes | Improvement 12; feature 6 |
| [`nucleo-matcher`](https://docs.rs/crate/nucleo-matcher/latest) | fzf-quality fuzzy ranking (used by Helix) that handles Unicode well | Improvement 12 |
| `strsim` | Jaro–Winkler similarity for guide name matching | Improvement 13 |
| [tauri-specta](https://github.com/specta-rs/tauri-specta) | Generated TypeScript types for commands and events; optional TanStack Query helpers | Improvement 3 |
| [UniFFI callback interfaces / async](https://mozilla.github.io/uniffi-rs/0.30/futures.html) | Push progress and playback events to Swift and Kotlin | Improvement 8 |
| `reqwest-middleware` + `reqwest-retry` | Retries with backoff for catalog, guide and API calls | Improvement 10 |
| [`rust_cast`](https://docs.rs/crate/rust_cast) | Chromecast control from the Rust side | Feature 18 |
| `cargo-fuzz`, `proptest`, Vitest, typescript-eslint | Hardening the parsers and testing the frontend | Improvement 20 |

### Crash reporting and analytics (opt-in)

| What | Why it helps FluxTV | Notes |
| --- | --- | --- |
| [Sentry](https://docs.sentry.io/platforms/rust/data-management/sensitive-data) (Rust, Cocoa and React SDKs) | One crash and error view across the core, Swift and the WebView | **Scrub every URL in `before_send`**: stream, guide and API URLs contain usernames and passwords. Make it opt-in |
| [TelemetryDeck](https://telemetrydeck.com/docs/guides/swift-setup) | Privacy-first usage analytics for the Apple apps (no personal data, no consent banner needed), free up to 100K signals per month | Useful for decisions such as how many users have AC-3 channels or use the guide |
| Xcode Organizer / MetricKit | Free crash and hang reports for the TestFlight and App Store builds | Already available; no SDK needed |

---

## Suggested order

1. **Quick wins (days):** improvements 1 (Windows credentials), 10 (catalog safety and retries),
   15 (M3U headers), 3 (settings allowlist), 9 (connection-limit message), 16 (shared display names),
   and feature 15 (media keys).
2. **Foundations (1–2 weeks):** improvements 2 (migrations), 20 (CI), 8 (UniFFI events), 11 (bounded
   memory) and 7 (shared media layer). Many later features depend on these.
3. **High-value features:** catch-up (feature 2), series (feature 1), resume and home screen (features 8
   and 7), libmpv on Windows (improvement 4), guide navigation (improvement 14) and Top Shelf (feature 14).
4. **Larger projects:** timeshift and DVR (features 3 and 4), multi-view (feature 9), casting (feature 18),
   Android TV (feature 19).
