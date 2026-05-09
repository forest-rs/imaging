# imaging

`imaging` is a small Rust command stream for 2D drawing. It is for code that wants to describe
drawing without choosing the final renderer too early.

You can stream commands directly into a backend, or record them into a retained scene for
validation, replay, snapshots, and tests.

This is not a finished graphics stack. It is the shared drawing vocabulary and recording layer used
by the backends in this workspace.

## How To Read It

- Use `Painter` to author drawing commands.
- Implement or use a `PaintSink` to receive those commands.
- Use `record::Scene` when you want an owned recording.
- Add a backend crate when you want pixels or renderer integration.

If you are new to the project, start with the examples crate:

```sh
cargo run -p imaging_examples --example hello_scene
```

To render a PNG through the Vello CPU backend:

```sh
cargo run -p imaging_examples --example render_png_vello_cpu -- out.png
```

The longer path through the concepts is in the `imaging::guide` rustdoc pages:

```sh
cargo doc -p imaging --open
```

## Where It Fits

`imaging` is useful when application or toolkit code wants to describe drawing once and send it to
different targets: a retained scene, a CPU renderer, a GPU renderer, snapshot tests, diagnostics, or
adapter code for another vector format.

It is especially relevant to people working on UI/toolkit rendering, SVG or Velato integration,
backend conformance, and Vello experiments.

## Crates

- `imaging`: `no_std` core `Painter`, `PaintSink`, `record::Scene`, validation, diagnostics,
  and replay.
- `imaging_conformance`: shared backend conformance checks.
- `imaging_examples`: small examples for learning the core model.
- `imaging_skia`: Skia backend.
- `imaging_snapshot_tests`: image snapshot cases shared across backends.
- `imaging_tiny_skia`: CPU renderer based on `tiny-skia`.
- `imaging_vello`: Vello backend.
- `imaging_vello_cpu`: Vello CPU backend; start here if you want pixels quickly.
- `imaging_vello_hybrid`: Vello sparse/hybrid backend using `wgpu`.
- `imaging_wgpu`: shared texture-renderer traits and `wgpu` target glue.
- `imaging_wind_tunnel`: benchmark and measurement crate.
- `svg_imaging`: SVG lowering into `imaging`.
- `velato_imaging`: Velato lowering into `imaging`.

## Current Shape

The core crate is intentionally small and still experimental. The names and crate boundaries may
change while the project settles.

What is useful today:

- recording command streams with `record::Scene`
- validating and diagnosing recorded scenes
- replaying scenes into another sink
- rendering through backend crates for tests and experiments
- using snapshot tests to compare backend behavior

What is still rough:

- backend feature coverage differs
- GPU backends need platform/device setup
- this is not a full tutorial book or stable application API yet
