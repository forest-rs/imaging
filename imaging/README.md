# `imaging`

Backend-agnostic 2D imaging recording + streaming API.

This crate is `no_std` by default (uses `alloc`); enable the `std` feature when needed.

## API shape

`imaging` has two primary public layers:

- `PaintSink` + `Painter`: the borrowed streaming/authoring API used to stream commands directly
  into scenes, renderers, or backend-native recorders
- `record`: the owned, backend-agnostic semantic recording module used for validation, replay,
  retention, and backend-independent storage

The crate root is intentionally biased toward the borrowed streaming surface and shared drawing
vocabulary. Retained scene data and low-level recording payloads live under `imaging::record`, for
example `record::Scene`, `record::Draw`, `record::Clip`, `record::Group`, and `record::GlyphRun`.
Those retained types are exact recording data, not the preferred authoring surface.

Pre-1.0 note: the streaming surface moved from the old owned `Sink` shape to the borrowed
`PaintSink`/`Painter` model so command authoring does not have to construct owned IR payloads
up-front.

Migration note: retained types that previously lived at the crate root now live under
`imaging::record`.
