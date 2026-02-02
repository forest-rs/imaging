# `imaging_vello_hybrid`

Vello hybrid (CPU/GPU via `wgpu`) backend for the `imaging` command stream.

This backend is intended for headless/offscreen rendering into an RGBA8 buffer.

## Notes

- This backend requires a working `wgpu` adapter/device. In sandboxed/headless environments it may
  be unavailable; prefer using `VelloHybridRenderer::try_new`.
- Group-level filters are currently not supported by `vello_hybrid`; `imaging_vello_hybrid`
  returns `Error::UnsupportedFilter` if a scene uses them.
- Workaround for vello#1408: `Compose::Copy` with a fully transparent solid paint is mapped to
  `Compose::Clear` to avoid a vello_hybrid optimization that skips generating strips for invisible
  paints.
