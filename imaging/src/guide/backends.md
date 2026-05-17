# Backend Map

Start with `imaging_vello_cpu` if you are learning the crate and want pixels. It is CPU-based, has
fewer setup steps than the GPU paths, and still exercises the Vello-side rendering work.

```sh
cargo run -p imaging_examples --example render_png_vello_cpu -- out.png
```

This page is a map, not a feature matrix. Backend coverage is still moving.

## Core And Tools

`imaging`

The core command stream and retained scene crate. Use this for [`crate::Painter`],
[`crate::PaintSink`], [`crate::record::Scene`], validation, diagnostics, and replay.

`imaging_examples`

Runnable examples for learning the crate. These live outside the core crate so examples can pull in
renderer and image-writing dependencies without changing the core dependency surface.

`imaging_conformance`

Contract tests for backend behavior at the command-stream level.

`imaging_snapshot_tests`

Shared visual cases and backend snapshot tests.

`imaging_wind_tunnel`

Benchmark and measurement crate.

## Renderers

`imaging_vello_cpu`

Vello CPU backend. This is the first backend to try for examples, tests, and local image output.

`imaging_vello`

Vello backend. Use this when working with the Vello renderer path.

`imaging_vello_hybrid`

Vello sparse/hybrid backend using `wgpu`. This needs a working GPU/device setup and is more
experimental than the CPU example path.

`imaging_wgpu`

Shared traits and target types for rendering into application-owned `wgpu` textures.

`imaging_skia`

Skia backend. Useful when you need Skia integration or want to compare behavior with a mature 2D
renderer.

`imaging_tiny_skia`

CPU renderer using `tiny-skia`. Useful for local rendering and backend comparisons.

## Importers

`svg_imaging`

Adapter from SVG documents into `imaging` commands.

`velato_imaging`

Adapter from Velato/Lottie-style content into `imaging` commands.

## Practical Rule

Use [`crate::record::Scene`] while you are learning the model. Add a backend when you want pixels,
texture integration, or backend-specific behavior.
