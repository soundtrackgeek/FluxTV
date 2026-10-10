# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [0.4.1] - 2026-10-10

### Fixed

- Keep the initial loading indicator visible until playback starts.
- Hide opaque browsing panels while native video is fullscreen.
- Cancel a pending upstream request before waiting for its connection lock, so channel switching
  remains responsive when a provider never sends headers; covered by a real HTTP regression test.
- Restrict libmpv IPC to the operations FluxTV uses, excluding the upstream wrapper's unsafe
  synchronous node getter. Player state uses observed-property events.
- Verify bundled license notices along with DLLs when reusing the player runtime.

## [0.4.0] - 2026-10-10

### Added

- Embedded libmpv Windows player with bundled LGPL AC-3, E-AC-3 and DTS audio decoding,
  MPEG-2/H.264/HEVC video, MKV/MP4/MPEG-TS playback and automatic hardware decoding.
- Audio and subtitle track selectors and a movie seek bar in the built-in player.
- Pinned runtime setup with SHA-256 verification, architecture selection, development DLL copying,
  installer resources and third-party notices.

### Changed

- Replaced mpegts.js and WebView2 media decoding with native playback behind the React controls.
  Kept fullscreen, shortcuts, volume/mute persistence, channel zapping and external-player fallback.
- Serialized native lifecycle operations to protect rapid channel changes and React StrictMode cleanup;
  live failures and stalls reconnect with bounded retries.
- Workspace and Windows app version 0.4.0.

### Fixed

- Windows built-in playback no longer depends on optional Windows Dolby audio decoders.
- Cancel pending upstream connections on stop/switch and serialize connection establishment.
  Regression tests cover cancellation before headers and forwarding movie range responses.

## [0.3.0] - 2026-10-10

### Added

- Windows Credential Manager storage for Xtream passwords, M3U playlist URLs and explicit or
  playlist-discovered XMLTV URLs, using the native Windows backend of `keyring`.
- Automatic migration of existing source secrets, with verified writes before removing plaintext
  from SQLite and scrubbing free pages and the WAL. Failed saves retain the previous credentials;
  credential deletion is retried after a temporary vault failure.
- Tests for secure source lifecycle, migration and failure recovery, plus a native Windows
  Credential Manager integration test using synthetic credentials.

### Changed

- Windows settings IPC accepts only the external player setting, preventing access to guide credentials.
- Windows onboarding describes Credential Manager protection. Workspace, core and Windows app version 0.3.0.

## [0.2.0] - 2026-09-30

### Added

- Apple apps (`apps/apple`): SwiftUI app for iPhone, iPad and Apple TV, distributed through TestFlight
  (team `3L5769JKCM`, bundle `com.soundtrackgeek.fluxtv` for both platforms).
  - Accounts stored in the Keychain as iCloud Keychain items, so an account added on iPhone appears on
    Apple TV; passwords never reach the database.
  - Live TV, movies, favorites, recently watched, search, TV guide grid and programme details.
  - AVPlayer playback with Picture in Picture and background audio on iPhone, a portrait channel
    switcher, and the native Apple TV player with a Channels info tab and channel/favorite actions.
  - Generated iOS icon and tvOS layered icons / top shelf images (`scripts/make-icons.swift`).
  - Xcode Cloud support (`ci_scripts/ci_post_clone.sh` installs Rust) and export options for uploads.
  - First TestFlight builds (0.2.0 build 1, iOS and tvOS) uploaded to the "FluxTV Player" record.
- `crates/fluxtv-ffi`: UniFFI bindings (`FluxEngine`) and a local media server for AVPlayer:
  - Live MPEG-TS repackaged into keyframe-aligned HLS (no transcoding), with automatic reconnects and
    idle release of the upstream connection while paused.
  - Movies sniffed by content: MP4 passed through with byte ranges, MPEG-TS served as seekable HLS VOD,
    Matroska reported as unsupported. Provider redirects are reused to avoid HTTP 429 rate limits.
  - One upstream connection at a time, so single-connection accounts can zap.
- Core: `hls` module (TS segmenter, DVB AC-3/E-AC-3 relabelling, live and VOD playlists, TS probing),
  runtime (in-memory) source passwords, and favorite backup/restore.

### Changed

- Workspace, core and Windows app version 0.2.0.

## [0.1.0] - 2026-09-30

### Added

- Shared Rust core (`crates/fluxtv-core`):
  - M3U / M3U Plus playlist parser (attributes, `#EXTGRP`, `url-tvg` header, content kind detection).
  - Streaming XMLTV parser with gzip support and timezone-aware timestamps.
  - Xtream Codes API client with lenient JSON handling, account info, live and VOD catalogs.
  - Detection of pasted Xtream `get.php` links (including the `password=…=m3u_plus` typo).
  - SQLite store for sources, categories, channels, EPG, favorites, watch history and settings.
  - Ranked channel search with Unicode normalization (superscript quality tags, accents, Nordic letters).
  - EPG matching by channel name for channels without an EPG id.
  - `FluxCore` façade with background-friendly catalog and guide refresh and progress reporting.
  - `probe` example for end-to-end testing against a real provider.
- Windows app (`apps/windows`, Tauri 2 + React + TypeScript):
  - Provider onboarding and management (Xtream and M3U), account status display.
  - Live TV and Movies browsing with filterable categories and virtualized lists for 50k+ channels.
  - Global search, favorites, recently watched.
  - Now/next info, programme details, "up next" list and a 24-hour TV guide grid.
  - Built-in MPEG-TS player (mpegts.js) with reconnect, stall recovery, fullscreen, zapping and shortcuts.
  - Native playback with seeking for movies.
  - Local stream proxy that hides credentials and enforces a single upstream connection.
  - External player support (mpv / VLC auto-detection or custom path).
