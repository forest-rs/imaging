# Learning Path

This is the short path through `imaging`: run one example, understand where commands go, then look
at a renderer. It is not meant to be a book or a full API reference.

The examples live in the workspace `imaging_examples` crate so the core crate does not need extra
example-only dependencies.

## Draw Something Into A Scene

Run:

```sh
cargo run -p imaging_examples --example hello_scene
```

This creates a [`crate::record::Scene`], wraps it in a [`crate::Painter`], draws a rectangle,
validates the scene, and prints a few facts about the recording.

The useful part is the shape of the code. Most callers should author drawing through
[`crate::Painter`]. The target can be a retained scene, a backend, a validator, or a small tool that
just observes commands.

## Look At `PaintSink`

Run:

```sh
cargo run -p imaging_examples --example counting_sink
```

This example implements a small [`crate::PaintSink`] that counts commands instead of rendering
pixels.

That is the core split in the crate: [`crate::Painter`] emits commands, and [`crate::PaintSink`]
receives them. A renderer can be a sink, but a sink does not have to be a renderer.

## Keep A Recording

Read:

```text
imaging_examples/examples/hello_scene.rs
```

[`crate::record::Scene`] is just one sink. It stores commands so they can be validated, diagnosed,
replayed, compared in tests, or rendered later.

That retained scene is backend-agnostic data. It is deliberately not a renderer.

## Replay A Scene

Run:

```sh
cargo run -p imaging_examples --example replay_scene
```

This records one scene, validates it, replays it into another scene, and checks that the recordings
match.

Replay is the bridge from retained data back into a sink. Once you can replay a scene, the same
recording can feed another retained scene, a backend, diagnostics, or a test harness.

## Render A PNG

Run:

```sh
cargo run -p imaging_examples --example render_png_vello_cpu -- out.png
```

This renders a retained scene through `imaging_vello_cpu` and writes a PNG.

Use this example when you want the quickest visible result. It keeps the rendering path CPU-based
while still exercising the Vello-side backend work.

## Where To Go Next

For visual comparison work, look at the `imaging_snapshot_tests` crate. It holds shared visual cases
and backend-specific snapshot tests.

For source labels and diagnostic context, search the crate docs for [`crate::Painter::with_context`]
and [`crate::with_context!`]. Context is useful when a retained scene needs to explain where a
command came from in application code.
