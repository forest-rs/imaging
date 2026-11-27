# Backend map

The PNG example uses `imaging_vello_cpu` to render on the CPU. Run it from the workspace root.

```sh
cargo run -p imaging_examples --example render_png_vello_cpu -- out.png
```

| Crate | Role |
| --- | --- |
| `imaging_vello_cpu` | Vello CPU rendering to an image buffer, from a recorded scene or streamed commands |
| `imaging_tiny_skia` | tiny-skia CPU rendering to an image buffer, from a recorded scene or streamed commands |
| `imaging_vello` | Vello rendering through `wgpu`, to a texture or an image buffer via readback |
| `imaging_vello_hybrid` | Hybrid CPU/GPU Vello rendering through `wgpu`, to a texture or an image buffer via readback |
| `imaging_skia` | Skia integration for drawing into a canvas or producing an image buffer |
| `imaging_wgpu` | Shared texture renderer traits and target types for application-owned `wgpu` textures |

For Vello GPU integrations, the application usually owns the device, queue, and presentation
targets. See each backend's crate documentation for its drawing feature coverage and setup.

The `svg_imaging` and `velato_imaging` adapters lower input to `imaging` commands. The
`imaging_conformance` crate checks shared behavior, and `imaging_snapshot_tests` compares
rendered output against reference images.
