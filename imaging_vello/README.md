# `imaging_vello`

Vello (GPU) backend for the `imaging` command stream.

This backend records `imaging` commands into a `vello::Scene`.

Rendering a `vello::Scene` to pixels requires `wgpu` device/queue setup; snapshot testing keeps
that orchestration in `imaging_snapshot_tests` (mirroring Vello’s own headless test patterns).

## Notes

- `vello` 0.7.0 does not correctly support blend layers nested under `push_clip_layer`
  (see vello#1198), so “non-isolated blend” semantics can differ inside non-isolated clips.
- `vello` does not expose per-draw blend modes; `imaging_vello` emulates them by wrapping the draw
  in a layer whose clip matches the draw geometry.
- `Compose::Copy` with a fully transparent solid source is emulated as `Compose::DestOut` with an
  opaque source to preserve coverage/AA for “punch” operations.

## Migration note

Older versions provided `VelloRenderer` with built-in `wgpu` setup and RGBA readback. The backend
now exposes `VelloRecorder`, which only records to `vello::Scene`. Use a test harness (or a
separate utility crate) to render the `vello::Scene` via `wgpu`.
