# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

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
