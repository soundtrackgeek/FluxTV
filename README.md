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
      │ Windows           │           │ Apple             │
      │ Tauri 2 + React/TS│           │ SwiftUI + AVKit   │
      │ apps/windows      │           │ iOS, iPadOS, tvOS │
      │                   │           │ apps/apple        │
      └───────────────────┘           │ via crates/       │
                                      │   fluxtv-ffi      │
                                      │ (UniFFI + local   │
                                      │  HLS server)      │
                                      └───────────────────┘
```

## Features (Windows)

- **Providers**: Xtream Codes (server + username + password) and plain M3U/M3U8 playlists with an optional XMLTV guide URL.
  Pasting an Xtream `get.php?username=…&password=…` link is detected automatically and upgraded to the Xtream API.
- **Windows Credential Manager** stores Xtream passwords, M3U playlist URLs and explicit or
  playlist-discovered guide URLs. Existing source credentials migrate automatically on upgrade.
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

## Features (iPhone, iPad and Apple TV)

The Apple apps share the Rust core through UniFFI (`crates/fluxtv-ffi`) and play everything with
AVPlayer, so they get the system player UI, Picture in Picture, AirPlay and the Apple TV remote for free.

- **Providers**: Xtream Codes and M3U playlists, validated with the provider before saving.
- **Accounts live in the Keychain** (never in the database). They are iCloud Keychain items, so an
  account added on iPhone also appears on Apple TV, where typing a password with the remote is painful.
- **Live TV** by category, favorites and recently watched, with now-playing info and progress bars.
- **Live MPEG-TS works on AVPlayer**: many Xtream accounts only allow the `ts` output format, which
  AVPlayer cannot play. The core repackages the stream into keyframe-aligned HLS segments on a local
  server (`127.0.0.1`, random per-session URLs) without transcoding. DVB AC-3/E-AC-3 audio is relabelled
  so Apple's demuxer accepts it, dropped connections reconnect automatically, and a paused player
  releases the upstream connection after 30 seconds and resumes transparently.
- **Single-connection friendly**: the previous upstream connection is closed before the next opens,
  so channel zapping works on accounts limited to one connection.
- **Movies** with seeking: MP4 files are passed through; movies stored as MPEG-TS are served as a
  seekable HLS VOD playlist. Matroska (MKV) movies are not playable yet and say so.
- **Search** (same ranking as Windows), **TV guide grid**, programme details and "watch" from the guide.
- **iPhone**: video with channel info, now/next and a channel switcher in portrait; full screen in
  landscape. Background audio and Picture in Picture. iPad gets a sidebar.
- **Apple TV**: the native full-screen player; swipe down for a **Channels** tab to zap, and
  previous/next channel and favorite actions in the transport bar.
- Catalog and guide are cached in SQLite (on Apple TV in the purgeable cache directory; favorites are
  backed up to user defaults so they survive a purge).

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
  src/hls.rs               MPEG-TS → HLS segmenter and playlists (Apple playback)
  examples/probe.rs        End-to-end smoke test against a real provider
crates/fluxtv-ffi/         UniFFI bindings for the Apple apps
  src/lib.rs               FluxEngine: the API Swift calls
  src/media.rs             Local media server: live HLS, TS movies as HLS VOD, MP4 pass-through
  examples/live_check.rs   Play a channel or movie through the engine and print the local URL
apps/windows/              Windows app (Tauri 2 + React + TypeScript)
  src/                     React UI
  src-tauri/src/proxy.rs   Local stream proxy (single upstream connection)
  src-tauri/src/commands.rs Tauri commands used by the UI
  src-tauri/src/external.rs mpv / VLC detection and launch
apps/apple/                iOS / iPadOS / tvOS app (SwiftUI)
  FluxTV.xcodeproj         Targets "FluxTV iOS", "FluxTV tvOS" and FluxTVTests
  FluxTV/App/              App model, Keychain account store
  FluxTV/Views/            Live TV, movies, guide, search, settings, account form
  FluxTV/Player/           AVPlayer / AVPlayerViewController (PiP, tvOS transport bar)
  FluxTV/Core/FFI/         Generated Swift bindings (regenerated by the build)
  scripts/build-rust.sh    Xcode build phase: builds the Rust library and bindings
  scripts/make-icons.swift Renders the iOS icon and tvOS layered icons / top shelf images
  ci_scripts/              Xcode Cloud: installs Rust
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

On first launch, add your provider. Catalogs, favorites and settings are stored in
`%APPDATA%\com.fluxtv.app\fluxtv.db`. Source passwords and playlist/guide URLs are stored as generic
credentials under `com.fluxtv.app` in **Windows Credential Manager**, for the current Windows user.

On upgrade, FluxTV moves existing source secrets into Credential Manager, verifies them, clears their
SQLite fields and removes leftover copies from the database's free pages and WAL. If credential storage
fails, the app stops with an error and keeps unmigrated values for a retry. New saves never fall back to
plaintext. Editing or deleting a provider replaces or removes its credentials; interrupted cleanup is
retried on the next launch. Copying the database to another PC does not transfer these secrets: add the
provider again on that PC. Individual M3U channel URLs remain part of the local catalog cache.

## Getting started (iOS, iPadOS, tvOS)

Prerequisites:

- macOS with Xcode 26 or later
- [Rust](https://rustup.rs) (stable). The build adds the Apple targets it needs automatically, or:

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim aarch64-apple-tvos aarch64-apple-tvos-sim
```

Open `apps/apple/FluxTV.xcodeproj` and run the **FluxTV iOS** or **FluxTV tvOS** scheme. The "Build Rust
core" phase compiles `crates/fluxtv-ffi` for the selected platform (optimized even in Debug, because guide
imports are heavy) and refreshes the Swift bindings in `FluxTV/Core/FFI` when the Rust API changes.
Signing is automatic with team `3L5769JKCM`; both apps use the bundle identifier `com.soundtrackgeek.fluxtv`
(one App Store Connect record with iOS and tvOS platforms, which also lets iCloud Keychain share accounts).

Build and test from the command line:

```bash
cd apps/apple
xcodebuild -project FluxTV.xcodeproj -scheme "FluxTV iOS" -destination 'platform=iOS Simulator,name=iPhone 17 Pro' test
xcodebuild -project FluxTV.xcodeproj -scheme "FluxTV tvOS" -destination 'generic/platform=tvOS Simulator' build
```

Debug builds accept two launch environment variables for simulator testing (ignored in release builds):
`FLUXTV_SEED_SERVER`, `FLUXTV_SEED_USERNAME` and `FLUXTV_SEED_PASSWORD` add an Xtream account on a fresh
install, and `FLUXTV_DEBUG_AUTOPLAY=<search>` starts the best live match (handy on Apple TV simulators).
With `simctl`, prefix them with `SIMCTL_CHILD_`.

App icons are generated from the logo geometry:

```bash
swift apps/apple/scripts/make-icons.swift apps/apple/FluxTV/Resources/Assets.xcassets
```

### TestFlight

Like Tonehavn, builds go to TestFlight through **Xcode Cloud**. `apps/apple/ci_scripts/ci_post_clone.sh`
installs Rust on the build machine. Create a workflow per scheme (FluxTV iOS, FluxTV tvOS) with an
Archive action and TestFlight (internal testing) as the post-action. The apps declare
`ITSAppUsesNonExemptEncryption = NO`, so builds don't wait for an export compliance answer.

To upload from a Mac instead, archive and export with automatic signing. App Store Connect rejects builds
from outdated Xcode betas, so point `DEVELOPER_DIR` at a release Xcode, and bump `CURRENT_PROJECT_VERSION`
first (each upload needs a new build number):

```bash
cd apps/apple
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
xcodebuild -project FluxTV.xcodeproj -scheme "FluxTV iOS" -destination 'generic/platform=iOS' \
  -archivePath build/FluxTV-iOS.xcarchive -allowProvisioningUpdates archive
xcodebuild -exportArchive -archivePath build/FluxTV-iOS.xcarchive -exportPath build/export-ios \
  -exportOptionsPlist ExportOptions.plist -allowProvisioningUpdates
```

`ExportOptions.plist` uses `method = app-store-connect` and `destination = upload`. For Apple TV, automatic
signing needs a registered Apple TV to make a development profile, so archive unsigned and let the export
apply distribution signing (no devices needed):

```bash
xcodebuild -project FluxTV.xcodeproj -scheme "FluxTV tvOS" -destination 'generic/platform=tvOS' \
  -archivePath build/FluxTV-tvOS.xcarchive CODE_SIGNING_ALLOWED=NO archive
xcodebuild -exportArchive -archivePath build/FluxTV-tvOS.xcarchive -exportPath build/export-tvos \
  -exportOptionsPlist ExportOptions.plist -allowProvisioningUpdates
```

The App Store Connect record is **FluxTV Player** (the name "FluxTV" was taken); on the home screen the
app is still called FluxTV.

## Development

Run the Rust unit tests (core parsers, store, HLS segmenter; FFI media server):

```bash
cargo test -p fluxtv-core -p fluxtv-ffi
```

On Windows, build the frontend first (`cd apps/windows`, `pnpm build`), then run
`cargo test -p fluxtv-windows` from the repository root. This includes an isolated native Credential
Manager test with synthetic credentials; it checks saving, reopening, editing and deleting a source.
The shared core tests also cover legacy migration, database scrubbing and vault failure recovery.

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

`live_check` does the same through the Apple engine, then starts a channel (or a movie) on the local media
server and prints its URL, which you can open in QuickTime Player or Safari on the Mac:

```bash
cargo run -p fluxtv-ffi --example live_check --release -- "nrk1 hd" 120
cargo run -p fluxtv-ffi --example live_check --release -- "id:movie:19" 120
```

## Roadmap

- Matroska (MKV) movies on Apple devices (remux to fragmented MP4 HLS in the core)
- Series (Xtream `get_series_info`) and catch-up / timeshift playback
- Multiple guide sources per provider

## License

MIT
