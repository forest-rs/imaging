# `imaging_vello`

Vello (GPU) backend for the `imaging` command stream.

This backend supports both headless image rendering and host-owned `wgpu` texture/surface
integration.

## Version Selection

`imaging_vello` supports two mutually exclusive Vello compatibility lanes:

- `vello-0-8` (default)
- `vello-0-7`

To integrate with `wgpu` 27 via Vello 0.7, disable default features and opt into `vello-0-7`.
Host code should use the `wgpu` re-export from `imaging_vello`, which always matches the selected
Vello compatibility lane.

## Notes

- This backend requires a working `wgpu` adapter/device. Hosts should usually own device creation
  themselves; test code in this repository uses local helper functions rather than a public
  bootstrap API.
- `vello` 0.7.0 does not correctly support blend layers nested under `push_clip_layer`
  (see vello#1198), so “non-isolated blend” semantics can differ inside non-isolated clips.
- `vello` does not expose per-draw blend modes; `imaging_vello` emulates them by wrapping the draw
  in a layer whose clip matches the draw geometry.
- `Compose::Copy` with a fully transparent solid source is emulated as `Compose::DestOut` with an
  opaque source to preserve coverage/AA for “punch” operations.
