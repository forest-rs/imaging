<div align="center">

# Imaging Snapshot Tests

**Kompari-based snapshots for imaging backends**

</div>

Development-only snapshot tests for the `imaging` stack.

This crate contains Kompari-based snapshot tests for `imaging` backends.

## Backends

- `vello_cpu` (default for `cargo xtask`)
- `skia`
- `vello_hybrid`
- `vello` (GPU)

## Run tests

- Vello CPU: `cargo test -p imaging_snapshot_tests --features vello_cpu --test vello_cpu_snapshots`
- Skia: `cargo test -p imaging_snapshot_tests --features skia --test skia_snapshots`
- Vello Hybrid: `cargo test -p imaging_snapshot_tests --features vello_hybrid --test vello_hybrid_snapshots`
- Vello GPU: `cargo test -p imaging_snapshot_tests --features vello --test vello_snapshots`

Skia snapshots require Skia binaries. In offline/sandboxed environments, set `SKIA_BINARIES_URL` to a local `file://...tar.gz` for rust-skia.

Equivalent `xtask` wrapper:
- Default backend (`vello_cpu`): `cargo xtask snapshots test`
- Select backend:
  - `cargo xtask snapshots --backend skia test`
  - `cargo xtask snapshots --backend vello_hybrid test`
  - `cargo xtask snapshots --backend vello test`

## Bless / regenerate

Bless current output as the expected snapshots:
- `cargo xtask snapshots test --accept`
- `cargo xtask snapshots --backend skia test --accept`
- `cargo xtask snapshots --backend vello_hybrid test --accept`
- `cargo xtask snapshots --backend vello test --accept`

Generate `tests/current/<backend>/*.png` for review:
- `cargo xtask snapshots test --generate-all`
- `cargo xtask snapshots --backend skia test --generate-all`
- `cargo xtask snapshots --backend vello_hybrid test --generate-all`
- `cargo xtask snapshots --backend vello test --generate-all`

`vello` snapshots will be skipped if no compatible wgpu device is available.

## Filter cases

To run only a subset of snapshot cases, set `IMAGING_CASE` (supports `*` globs).
The `xtask` wrapper exposes this via `--case`:

- Single case: `cargo xtask snapshots test --case gm_gradients_linear`
- Prefix: `cargo xtask snapshots test --case 'gm_clip_*'`
- Multiple patterns (comma/whitespace-separated): `cargo xtask snapshots test --case 'gm_clip_*,gm_gradients_*'`

## Review diffs with `xtask`

`xtask` runs the snapshot tests and produces a Kompari HTML report:
- Default backend (`vello_cpu`): `cargo xtask report`
- Select backend explicitly:
  - `cargo xtask report --backend skia`
  - `cargo xtask report --backend vello_hybrid`
  - `cargo xtask report --backend vello`
- Alternatively:
  - `cargo xtask snapshots --backend skia report`
  - `cargo xtask snapshots --backend vello_hybrid report`
  - `cargo xtask snapshots --backend vello review`

## Minimum supported Rust Version (MSRV)

This workspace has been verified to compile with **Rust 1.88** and later.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE] or <http://www.apache.org/licenses/LICENSE-2.0>), or
- MIT license ([LICENSE-MIT] or <http://opensource.org/licenses/MIT>),

at your option.

[LICENSE-APACHE]: ../LICENSE-APACHE
[LICENSE-MIT]: ../LICENSE-MIT
