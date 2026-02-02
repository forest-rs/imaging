# `imaging_skia`

Skia backend for the `imaging` command stream.

## Building

`skia-safe` / `skia-bindings` normally download prebuilt Skia binaries at build time. In offline
or sandboxed environments, set `SKIA_BINARIES_URL` to a local `tar.gz` (downloaded ahead of time):

```sh
SKIA_BINARIES_URL='file:///absolute/path/to/skia-binaries-....tar.gz' cargo build
```
