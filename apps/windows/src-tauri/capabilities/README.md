# Native player permissions

Grant only the libmpv commands used by `useMpv`: init, destroy, command, set-property and
set-video-margin-ratio. Player state arrives through observed-property events.

Do not enable `libmpv:default` or `libmpv:allow-get-property` with the pinned wrapper v0.1.1.
Its synchronous node getter passes a pointer-sized output buffer where libmpv expects an
`mpv_node` struct, causing a native access violation. Windows playback QA reproduced this
through a diagnostic query. Observed node properties (including track-list) use the separate
event conversion path and work correctly. Revisit this restriction after upgrading and
verifying an upstream fix.
