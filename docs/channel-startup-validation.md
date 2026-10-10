# Channel startup validation

Validated on 2026-10-10 for version 0.5.0.

## Behavior

Apple's local MPEG-TS-to-HLS server now publishes its playlist after two completed segments.
The first three segments each contain one video keyframe interval, then short intervals are grouped
into about two-second segments. Every segment retains program tables; the advertised target duration
covers both phases. Audio-only startup segments are about one second. Reconnects restart the short
segment phase with a discontinuity and preserve the sequence numbering.

For a provider with one-second keyframes, the required media before the first playlist falls from
three two-second segments (six seconds) to two one-second segments (two seconds). This is the media
preparation budget, not an end-to-end first-frame guarantee. Provider delays, the existing switch
grace period, keyframe intervals and decoder startup still contribute to elapsed time.

AVPlayer live items use `automaticallyWaitsToMinimizeStalling = false` and a one-second preferred
forward buffer. Movies keep the automatic wait and default forward buffer. The app first requests
the local live playlist with URLSession, then gives its ready URL to AVPlayer. In the simulator,
opening a playlist whose initial response took four seconds left AVPlayer's item unresolved;
preparing the same playlist first allowed playback. Direct provider HLS URLs skip this preparation
request, so it does not open another provider connection.

Windows uses libmpv rather than the mpegts.js player described in the original guide. Live startup
uses a 0.25-second initial cache wait and a one-second analysis budget. After the first playback
restart, the cache wait returns to one second. Reconnects restore normal analysis, so a stream that
needs more probing can recover. The existing three-second read-ahead cache, memory limit and movie
settings remain in place.

## Native measurements

Times start at channel selection, before the engine prepares playback. Retries reset the timer.
Apple logs AVKit display readiness; Windows logs the first libmpv `playback-restart` event as a
playback-start proxy. Each attempt logs once; later cache refills and seeks do not add startup logs.
The log contains milliseconds and platform only, without provider URLs or channel names.

| Environment | Controlled stream | Observed startup |
| --- | --- | --- |
| iPhone 17 Pro simulator, iOS 26.5, Debug | H.264/AAC, 640×360, 25 fps, one-second keyframes | 2,355 ms |
| Same iOS simulator | H.264/AAC, 640×360, 25 fps, two-second keyframes | 4,504 and 4,551 ms |
| Windows 11 x64, native Tauri/libmpv development app | H.264/AC-3, 640×360, 25 fps, one-second keyframes | 1,397 ms initial selection; 1,742 ms channel switch |

The fixtures were generated with FFmpeg and served over loopback at their average media bitrate.
Apple used a synthetic Xtream catalog with separate one- and two-second channels. Windows used the
existing synthetic catalog; both selected channel entries served the same H.264/AC-3 fixture for
this timing check. Windows pacing used packet-aligned chunks every 100 ms. An earlier provider
using 1,316-byte chunks at very short intervals underfed the player on Windows and was excluded from
the final timing results. Its cache pauses did confirm that playback resumed and the loading
indicator cleared when data returned. With corrected pacing, video progressed normally for more
than 30 seconds after a channel switch without a loading overlay.

These are controlled smoke measurements, not averages or a claim about every provider. No reliable
native before/after percentage was established. Physical iPhone, iPad and Apple TV timing was not
measured; iOS runtime checks exercise the player model shared with tvOS.

## Regression checks

- `cargo test -p fluxtv-core -p fluxtv-ffi -p fluxtv-windows`: 41 core, two FFI and four Windows
  backend tests passed on macOS.
- `cargo clippy -p fluxtv-core -p fluxtv-ffi -p fluxtv-windows --all-targets -- -D warnings`: passed.
- Windows frontend TypeScript/Vite production build: passed on macOS and Windows.
- iOS simulator XCTest suite: six passed; final iOS and tvOS simulator builds passed without warnings.
- Native Windows playback and zapping: visible video, one startup line per selection, smooth
  progress with the corrected paced fixture and successful native option updates.
- Native iOS playback: visible first frame for both keyframe intervals, one startup line per launch.

Segmenter tests cover 0.4-second and one-second keyframes, normal segment grouping after startup,
stable target duration, keyframe/PSI presence, four-second keyframes, timestamp wrap, reconnect
discontinuities, and audio-only streams. The HTTP regression holds the provider after exactly two
closed segments, requires a playlist before a third can exist, and retrieves both advertised
segments. Existing cancellation and movie range tests continue to pass.

## Repeat a timing check

1. Serve a valid MPEG-TS fixture at its media rate on loopback, with a known keyframe interval.
   For example, encode `testsrc2=size=640x360:rate=25` with libx264, `-g 25 -sc_threshold 0`,
   and AAC or AC-3 audio; use `-g 50` for two-second keyframes.
2. Add its synthetic M3U or Xtream source to a development app. Use a fresh native process;
   a frontend hot reload can retain native playback from an earlier run.
3. On Apple, filter Xcode/Console by subsystem `com.soundtrackgeek.fluxtv`, category `Playback`.
   On Windows, attach the WebView console with `F12` before selecting the channel.
4. Select the channel and record `time to first frame`. Check the picture and continued playback,
   then switch channels and verify a new timing line.
5. Keep fixture pacing, provider conditions and build configuration constant when comparing changes.
