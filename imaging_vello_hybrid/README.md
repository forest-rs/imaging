# `imaging_vello_hybrid`

Vello hybrid (CPU/GPU via `wgpu`) backend for the `imaging` command stream.

This backend supports both headless image rendering and host-owned `wgpu` texture integration.

## Notes

- This backend requires a working `wgpu` adapter/device. Hosts should usually own device creation
  themselves; test code in this repository uses local helper functions rather than a public
  bootstrap API.
- Recorded `imaging::record::Scene` values can use inline image brushes; the renderer uploads and
  caches them behind the scenes. Direct native-scene recording can use image brushes too via
  `VelloHybridSceneSink::with_renderer`; the plain `VelloHybridSceneSink::new` constructor stays
  limited to non-image brushes.
- Use `VelloHybridSceneSink::new` for solid/gradient-only native scene recording.
- Use `VelloHybridSceneSink::with_renderer` for native scene recording that needs image brushes.
- Group-level filters are currently not supported by `vello_hybrid`; `imaging_vello_hybrid`
  returns `Error::UnsupportedFilter` if a scene uses them.
- Workaround for vello#1408: `Compose::Copy` with a fully transparent solid paint is mapped to
  `Compose::Clear` to avoid a vello_hybrid optimization that skips generating strips for invisible
  paints.
