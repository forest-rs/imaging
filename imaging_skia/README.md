# `imaging_skia`

Skia backend for the `imaging` command stream.

## Building

`skia-safe` / `skia-bindings` normally download prebuilt Skia binaries at build time. In offline
or sandboxed environments, set `SKIA_BINARIES_URL` to a local `tar.gz` (downloaded ahead of time):

```sh
SKIA_BINARIES_URL='file:///absolute/path/to/skia-binaries-....tar.gz' cargo build
```

## GPU Feature

Enable the `gpu` feature to build `SkiaGpuRenderer`, which shares an app-owned `wgpu::Adapter`,
`wgpu::Device`, and `wgpu::Queue` and renders native `skia_safe::Picture` values into
caller-owned `wgpu::Texture` targets.

`SkiaGpuRenderer` selects the Ganesh interop backend at runtime from `adapter.get_info().backend`,
so a Windows host can use either D3D12 or Vulkan depending on how `wgpu` was configured.
