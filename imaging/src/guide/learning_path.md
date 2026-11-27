# Learning path

Run these examples from a workspace checkout. Their source lives in
`imaging_examples/examples/`.

## Record a scene

```sh
cargo run -p imaging_examples --example hello_scene
```

This example creates a [`crate::record::Scene`] and draws two shapes with a [`crate::Painter`].
It validates the scene and prints the command and diagnostic counts.

## Implement a sink

```sh
cargo run -p imaging_examples --example counting_sink
```

This example implements [`crate::PaintSink`] and checks that it receives two fills and one clip.
The same authoring API can target a renderer or a scene.

## Replay a recording

```sh
cargo run -p imaging_examples --example replay_scene
```

The example records a scene, validates it, and replays it into a second scene with
[`crate::record::replay`]. It checks that both recordings match. One draw is annotated with
[`crate::with_context!`] so a diagnostic can identify where the drawing command originated.

## Render a PNG

```sh
cargo run -p imaging_examples --example render_png_vello_cpu -- out.png
```

This example passes a recorded scene to the Vello CPU renderer and encodes the returned RGBA8
pixels as a PNG.

Read the [mental model](crate::guide::mental_model) for the difference between streaming and
retained workflows, or use the [backend map](crate::guide::backends) to find a renderer.
