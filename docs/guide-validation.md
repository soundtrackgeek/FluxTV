# TV guide validation — 0.6.0

Date: 2026-10-10. Improvement 14 in `improvements-and-features.md`.

## Result

Windows and Apple guides support a day picker, previous/next day, Now and jump to time.
Queries follow the visible rows and horizontal viewport, with a half-hour buffer on either side.
The timeline extends in six-hour steps at its edges. Programme data is replaced as the viewport
moves instead of accumulating every day/channel visited. Category colors distinguish sports,
news and films; provider category text is retained in programme accessibility labels/details.
Apple TV uses native focusable buttons, focus sections and horizontal reveal of focused programmes.
Its time menu offers quarter-hour jumps; iPhone/iPad and Windows accept individual minutes.

## Automated checks

- `cd apps/windows && pnpm test:guide`: four passing tests covering viewport query bounds,
  stable scroll buckets, local calendar/DST jumps, programme selection across schedule gaps,
  and category recognition.
- `cd apps/windows && pnpm build`: TypeScript and production Vite build passed.
- XcodeBuildMCP: both **FluxTV iOS** and **FluxTV tvOS** simulator builds passed.
- `cargo test -p fluxtv-core -p fluxtv-ffi`: 43 passing Rust tests.
- **FluxTV iOS** test suite: eight passing tests, including two new tests for viewport bounds
  and preserving the visible time when extending the timeline backwards.

## Rendered Windows guide

The flow under test is: open the guide → pick a day/time or scroll/move with arrow keys →
show the corresponding programmes and channel while keeping requests bounded.

Browser plugin not available; used the Playwright CLI with Chromium and a temporary local
fixture harness at `http://127.0.0.1:1431/`. The harness rendered the production `Guide` component,
stylesheet and app store, substituting only the guide API and synthetic channels/programmes.
No fixture, screenshot or browser script was added to the application bundle.

| Check | Result |
| --- | --- |
| Page identity and meaningful content | Passed: fixture title/URL and rendered guide |
| Framework error overlay | None |
| Console health | No application warnings/errors; fixture favicon 404 only |
| Screenshot review | Passed at 1440 × 900 and Windows minimum size 960 × 600 |
| Day picker, previous/next day, Now, jump | Passed, including 2026-10-25 (Oslo DST), midnight and Now within the same window |
| Guide refresh | Preserved the browsed day/time |
| Arrow keys and Enter | Passed: channel → programme → next channel → details |
| Horizontal extension | Passed at both timeline edges |
| Channel scale | Channel 5,000 reachable; fewer than 40 rows/guide IDs per request |
| Query bounds | At 1440 px, at most 19 IDs and 5.5 hours per request in the scripted run |
| Asynchronous provider switch | A delayed old response was discarded; only the current source was displayed |
| Error/retry | Failure displayed; retry returned to programme data |
| Empty/missing guide | Separate messages for an empty time range and an unmatched channel |

The interaction run caught and fixed an error-banner layout loop and a date control that
changed to the previous day when jumping to midnight. The same interactions passed after repair.

## Apple runtime check

On iPhone 17 Pro / iOS 26.5 Simulator, rendered a temporary category of 1,200 channels and
three days of half-hour programme fixtures. The guide rendered only visible/lazy rows,
with fixed channel labels, timeline headings, category colors and the current-time line.
Next day updated the heading/programmes to October 11; Now restored October 10/current time.
The simulator database and preferences were backed up and restored after validation.

Debug builds can open the guide directly, with an existing test account/catalog:

```bash
SIMCTL_CHILD_FLUXTV_DEBUG_TAB=guide xcrun simctl launch booted com.soundtrackgeek.fluxtv
```

This option is ignored in release builds.

## Coverage limits

The rendered Windows check used Chromium and a stubbed IPC result; native Windows WebView2/libmpv
was not exercised for this UI change. Apple TV compiled and launched, but remote focus traversal,
iPad layout and physical Apple devices were not exercised. Provider guide depth still determines
which dates have programmes; navigating outside that depth shows an empty range. Past programmes
still offer live channel playback; catch-up playback remains a separate feature.
