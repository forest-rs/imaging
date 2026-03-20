# `imaging`

Backend-agnostic 2D imaging recording + streaming API.

This crate is `no_std` by default (uses `alloc`); enable the `std` feature when needed.

## API shape

`imaging` has two primary public layers:

- `PaintSink` + `Painter`: the borrowed streaming/authoring API used to stream commands directly
  into scenes, renderers, or backend-native recorders
- `Scene`: the owned, backend-agnostic semantic recording used for validation, replay, retention,
  and backend-independent storage

It also exposes low-level recording payloads like `Draw`, `Clip`, `Group`, and `GlyphRun`.
Those are intended as exact recording data, not the preferred authoring surface.

Pre-1.0 note: the streaming surface moved from the old owned `Sink` shape to the borrowed
`PaintSink`/`Painter` model so command authoring does not have to construct owned IR payloads
up-front.
