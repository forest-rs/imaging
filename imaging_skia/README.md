# `imaging_skia`

Skia backend for the `imaging` command stream.

`SkiaRenderer` is the Ganesh-backed GPU renderer. `SkiaCpuRenderer` is the previous raster path.

The Ganesh backend is adapted in part from `anyrender_skia` from the AnyRender project, with the
borrowed backend-initialization code carrying attribution in the copied source files.

## Building

`skia-safe` / `skia-bindings` normally download prebuilt Skia binaries at build time. In offline
or sandboxed environments, set `SKIA_BINARIES_URL` to a local `tar.gz` (downloaded ahead of time):

```sh
SKIA_BINARIES_URL='file:///absolute/path/to/skia-binaries-....tar.gz' cargo build
```
