# imaging

`imaging` is a Rust command stream for 2D drawing. Application and toolkit code use
`Painter` to send drawing commands to a `PaintSink`. You can stream commands directly to a
backend or record them in a `record::Scene` for validation, replay, and tests.

## Get started

Run these examples from a workspace checkout. `hello_scene` builds and validates a small scene.

```sh
cargo run -p imaging_examples --example hello_scene
```

`render_png_vello_cpu` renders a scene with the Vello CPU backend and writes `out.png`.

```sh
cargo run -p imaging_examples --example render_png_vello_cpu -- out.png
```

The [learning path] also shows how to implement a sink and replay a recording. Open the full
guide and API reference with `cargo doc -p imaging --open`.

## Workspace

| Crate | Role |
| --- | --- |
| [`imaging`](imaging/) | `no_std` command vocabulary, authoring, recording, validation, and replay |
| [`imaging_examples`](imaging_examples/examples/) | Runnable examples for the command stream |
| [`imaging_vello_cpu`](imaging_vello_cpu/) | CPU rendering with Vello |
| [`imaging_tiny_skia`](imaging_tiny_skia/) | CPU rendering with tiny-skia |
| [`imaging_vello`](imaging_vello/) | GPU rendering with Vello and `wgpu` |
| [`imaging_vello_hybrid`](imaging_vello_hybrid/) | Hybrid CPU/GPU rendering with Vello and `wgpu` |
| [`imaging_skia`](imaging_skia/) | Skia integration |
| [`imaging_wgpu`](imaging_wgpu/) | Shared renderer traits for application-owned `wgpu` textures |
| [`svg_imaging`](svg_imaging/) | SVG adapter |
| [`velato_imaging`](velato_imaging/) | Velato/Lottie adapter |
| [`imaging_conformance`](imaging_conformance/) | Shared backend contract tests |
| [`imaging_snapshot_tests`](imaging_snapshot_tests/) | Visual cases and backend snapshot tests |
| [`imaging_wind_tunnel`](imaging_wind_tunnel/) | Benchmarks and measurements |

The core API is experimental and may change. Drawing feature support varies across backends.
GPU rendering requires a supported device.

## License

Licensed under either [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.

[learning path]: imaging/src/guide/learning_path.md
