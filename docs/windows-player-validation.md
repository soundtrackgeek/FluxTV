# Windows libmpv player validation

Date: 2026-10-10. Implementation: 0.4.0–0.4.3.

## Environment and fixtures

Native checks used a Windows 11 x86_64 PC through Windows App, with WebView2 and the pinned
LGPL runtime from `apps/windows/scripts/mpv-runtime.json`. Testing used an isolated Git checkout
and a local HTTP fixture provider, without sending provider credentials to a test service.

FFmpeg generated 60-second 640×360, 25 fps test-pattern clips with a 48 kHz sine-wave audio track:
H.264/AC-3, H.264/E-AC-3, H.264/DTS and MPEG-2/AC-3 in MPEG-TS. A Matroska movie contained
AC-3 (`eng`) and E-AC-3 (`nor`) tracks and an embedded SubRip subtitle. The fixture server paced
live TS responses and supported HTTP byte ranges for the movie.

## Native checks

| Check | Result |
| --- | --- |
| H.264 + AC-3 live TS | Decoded video and `ac3` audio |
| H.264 + E-AC-3 live TS | Decoded video and `eac3` audio |
| H.264 + DTS live TS | Decoded video and `dts` audio |
| MPEG-2 + AC-3 live TS | Decoded video; UI reported MPEG-2 and AC-3 |
| Windows audio output | AC-3, E-AC-3 and DTS produced float PCM, 44.1 kHz stereo, through WASAPI |
| Fullscreen | Video filled the window; opaque browsing panels stayed hidden |
| Exit fullscreen / channel zap | Returned to the stage; fullscreen Down switched AC-3 to E-AC-3 |
| Floating mini-player | Video continued visibly above both settings and the guide, then returned to the normal stage |
| Pause / mute | Paused timecode stayed unchanged; mute icon and slider reflected mute; both resumed |
| Live end-of-stream | Reconnecting indicator appeared, then video resumed with a fresh timecode |
| Provider outage | Stopping the local fixture server exhausted the bounded reconnects and displayed Retry / external-player actions |
| Manual retry | Restarting the fixture server and clicking Retry resumed AC-3 playback with decoded video |
| External-player handoff | Built-in playback stopped; the configured external mpv opened and played the fixture |
| MKV seeking | Duration showed 01:00; seek moved the decoded timecode to the selected position |
| MKV audio selection | Switching to track 2 changed the UI and decoder to E-AC-3 |
| Embedded SubRip | Track selection rendered the subtitle inside the windowed video; fullscreen also rendered it |
| Movie end / replay | Play button restarted the ended movie from the beginning |
| IPC restriction | A direct synchronous node getter call was rejected by Tauri permissions |

Audio decoding and the Windows output pipeline were inspected directly; physical speaker output
was not assessed through the remote desktop session. H.264/HEVC hardware decoding is configured
with `hwdec=auto-safe`; successful playback alone does not prove a particular GPU decoder was used.

The pinned upstream wrapper has an unsafe synchronous node getter. FluxTV does not expose that
operation: player state uses observed-property events, including the structured track list.
See [capability rationale](../apps/windows/src-tauri/capabilities/README.md).

## Automated checks

- TypeScript and Vite production builds passed on macOS and Windows.
- `cargo check -p fluxtv-windows` passed.
- `cargo clippy -p fluxtv-windows --all-targets -- -D warnings` passed.
- macOS: 4 Windows backend tests, 38 core tests and 1 FFI test passed.
- Windows: `cargo test -p fluxtv-windows -- --test-threads=1` passed all 6 tests, including native
  Credential Manager integration tests. Three real HTTP tests cover movie range headers,
  cancellation while waiting for headers, and channel replacement while that connection lock is held.
- Runtime setup downloaded, checksum-verified and extracted both x86_64 and aarch64 archives;
  cached x86_64 DLLs and notices were verified again on Windows.
- The package's `tauri` script ran runtime setup explicitly before invoking the CLI.

## Packaging checks

`pnpm tauri build --debug` completed both x64 installers with the production frontend embedded:
`FluxTV_0.4.3_x64-setup.exe` (NSIS, 35.31 MiB) and `FluxTV_0.4.3_x64_en-US.msi` (49.79 MiB).
They are in `C:\_code\FluxTV\target\debug\bundle\` on the Windows PC.

The NSIS payload was extracted into the isolated QA checkout and its executable was run without
a Vite development server. The payload contained `libmpv-2.dll`, `libmpv-wrapper.dll`, the runtime
receipt, third-party notice, and mpv/FFmpeg/wrapper license and copyright files. Native movie
playback and subtitle rendering worked with those packaged resources.

## Coverage limits

Native playback was tested on x86_64 Windows 11. ARM64 runtime setup was checked, but native
ARM64 playback, all GPU/driver combinations, HEVC streams and surround/passthrough outputs were
not exercised. These were debug Rust packages; release optimization, signing, installer execution
and installation on a clean Windows machine were not exercised. Decoding still depends on the
actual stream being valid and the bundled mpv/FFmpeg build supporting its codecs. Public binary
releases must provide corresponding LGPL source or a valid source offer as described in the
bundled third-party notice.
