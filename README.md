# FluxTV

A fast IPTV player built on a **shared Rust core**, with native front ends per platform.

```
                 ┌─────────────────────────────┐
                 │      Shared Rust core       │
                 │  crates/fluxtv-core         │
                 │  M3U / XMLTV parsing        │
                 │  Xtream API client          │
                 │  Channel models, search     │
                 │  Favorites / history        │
                 │  EPG processing, SQLite     │
                 └──────────────┬──────────────┘
                ┌───────────────┴───────────────┐
      ┌─────────▼─────────┐           ┌─────────▼─────────┐
      │ Windows (done)    │           │ Apple (planned)   │
      │ Tauri 2 + React/TS│           │ SwiftUI + AVKit   │
      │ apps/windows      │           │ iOS / tvOS        │
      └───────────────────┘           └───────────────────┘
```

## Features (Windows)

- **Providers**: Xtream Codes (server + username + password) and plain M3U/M3U8 playlists with an optional XMLTV guide URL.
  Pasting an Xtream `get.php?username=…&password=…` link is detected automatically and upgraded to the Xtream API.
- **Live TV** with categories (filterable), a virtualized channel list that handles 50,000+ channels, and live
  "now playing" info with progress bars.
- **Movies (VOD)** from Xtream providers, with seeking.
- **Fast search** across all channels (~20 ms for 55k names). Matching ignores case, accents and the
  superscript tags providers love (`ᴴᴰ`, `ᴿᴬᵂ`, `ᵁᴴᴰ`), so `nrk1 raw` finds `GOLD| NRK1 ᴿᴬᵂ`.
- **TV guide (EPG)**: now/next in the list, programme details and "up next" for the playing channel, and a
  full guide grid (channels × 24 h timeline) for any category or your favorites.
  Channels without an EPG id are matched to the guide by name (thousands of extra channels get guide data).
- **Favorites** and **recently watched**, preserved across playlist refreshes.
- **Built-in player** for MPEG-TS live streams (H.264/HEVC video; AAC, MP3, AC-3 and E-AC-3 audio where Windows
  provides decoders), with automatic reconnect and stall recovery, fullscreen and channel zapping.
- **External player** fallback (mpv or VLC, auto-detected, or a custom path) for anything the built-in player
  can't handle.
- **Single-connection friendly**: switching channels closes the previous upstream connection *before* opening
  the next, so accounts limited to one connection don't get rejected.
- Credentials never reach the web UI: playback goes through a local proxy on `127.0.0.1` with random per-session URLs.
- Catalog and guide are cached in SQLite and refreshed in the background when stale (channels: 24 h, guide: 12 h).

### Keyboard shortcuts

| Key | Action |
| --- | --- |
| `F` / double-click | Toggle fullscreen |
| `Esc` | Exit fullscreen |
| `Space` | Pause / resume |
| `M` | Mute |
| `PgUp` / `PgDn` | Previous / next channel (`↑` / `↓` in fullscreen) |
| `Ctrl+K` or `/` | Search |

## Repository layout

```
Cargo.toml                 Cargo workspace
crates/fluxtv-core/        Shared, platform-independent Rust core
  src/m3u.rs               M3U / M3U Plus parser
  src/xmltv.rs             Streaming XMLTV parser (gzip aware)
  src/xtream.rs            Xtream Codes API client
  src/search.rs            Normalization and ranked search
  src/store.rs             SQLite cache: catalog, EPG, favorites, history, settings
  src/library.rs           FluxCore façade: add/refresh sources, stream URLs
  examples/probe.rs        End-to-end smoke test against a real provider
apps/windows/              Windows app (Tauri 2 + React + TypeScript)
  src/                     React UI
  src-tauri/src/proxy.rs   Local stream proxy (single upstream connection)
  src-tauri/src/commands.rs Tauri commands used by the UI
  src-tauri/src/external.rs mpv / VLC detection and launch
```

## Getting started (Windows)

Prerequisites:

- [Rust](https://rustup.rs) (stable, MSVC toolchain) and the Visual Studio C++ Build Tools
- [Node.js](https://nodejs.org) 20+ and [pnpm](https://pnpm.io)
- WebView2 runtime (preinstalled on Windows 11)
- Optional: [mpv](https://mpv.io) or [VLC](https://www.videolan.org) as an external player

```bash
cd apps/windows
pnpm install
pnpm tauri dev
```

Build an installer (NSIS and MSI in `target/release/bundle/`):

```bash
cd apps/windows
pnpm tauri build
```

On first launch, add your provider. Data is stored in `%APPDATA%\com.fluxtv.app\fluxtv.db`.

## Development

Run the core unit tests:

```bash
cargo test -p fluxtv-core
```

Smoke-test the core against a real Xtream provider. Put credentials in a `.env` file at the repository root
(it is git-ignored):

```
iptv_username=...
iptv_password=...
iptv_server=http://provider.example:8080
```

```bash
cargo run -p fluxtv-core --example probe --release
```

It adds the source to a temporary database, refreshes the catalog and guide, and prints timings, search
results and now-playing info.

## Roadmap

- Apple apps (iOS, tvOS) in SwiftUI on top of `fluxtv-core` via UniFFI bindings
- Series (Xtream `get_series_info`) and catch-up / timeshift playback
- Credential storage in the OS keychain
- Multiple guide sources per provider

## License

MIT
