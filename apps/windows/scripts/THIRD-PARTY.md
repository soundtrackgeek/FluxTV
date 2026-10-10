# FluxTV Windows player libraries

FluxTV dynamically links these replaceable LGPL libraries. DLLs and notices are installed in `lib/`
next to FluxTV.exe; they may be replaced with ABI-compatible builds for the same architecture.
FluxTV itself remains MIT licensed.

- **libmpv-wrapper v0.1.1**: LGPL-2.1, [source](https://github.com/nini22P/libmpv-wrapper/tree/v0.1.1).
  Its license is bundled in `notices/`. [Build instructions](https://github.com/nini22P/libmpv-wrapper).
- **libmpv (2026-10-09, 06ee185d3f)**: LGPL-2.1-or-later build from
  [zhongfly/mpv-winbuild](https://github.com/zhongfly/mpv-winbuild/releases/tag/2026-10-09-06ee185d3f).
  [mpv source](https://github.com/mpv-player/mpv/tree/06ee185d3f), including license files.
  The LGPL build uses FFmpeg b91a82d6d: [source](https://github.com/FFmpeg/FFmpeg/tree/b91a82d6d).
  [Dependency sources and build recipes](https://github.com/zhongfly/mpv-winbuild).
- **tauri-plugin-libmpv 0.3.2** and **tauri-plugin-libmpv-api 0.3.2**: MPL-2.0,
  [source and license](https://github.com/nini22P/tauri-plugin-libmpv).

The exact archive URLs and SHA-256 hashes are in the bundled `runtime.json` receipt and the
repository's `scripts/mpv-runtime.json`. Redistributors must provide corresponding source for
the LGPL binaries and dependencies, including their build recipes, with their release (or a
valid written source offer). The upstream links above identify source; they are not a source offer.
