# `imaging_snapshot_tests`

Kompari-based image snapshot tests for `imaging` backends.

## Running

- Run one backend directly:
  `cargo test -p imaging_snapshot_tests --features vello_cpu --test vello_cpu_snapshots`
- Run the repeatable CPU release benchmark set:
  `./imaging_snapshot_tests/scripts/bench_cpu_release.sh tiny_skia`
- Override loops or narrow to specific cases:
  `./imaging_snapshot_tests/scripts/bench_cpu_release.sh tiny_skia 120 gm_image_brushes gm_gradients_sweep`
- Bless new baselines:
  `IMAGING_TEST=accept cargo test -p imaging_snapshot_tests --features vello_cpu --test vello_cpu_snapshots`
- Write all current outputs for inspection:
  `IMAGING_TEST=generate-all cargo test -p imaging_snapshot_tests --features vello_cpu --test vello_cpu_snapshots -- --nocapture`
- Restrict to one case:
  `IMAGING_CASE=gm_strokes cargo test -p imaging_snapshot_tests --features vello_cpu --test vello_cpu_snapshots -- --nocapture`
- Enable per-case logging:
  `IMAGING_TEST_VERBOSE=1 cargo test -p imaging_snapshot_tests --features vello_cpu --test vello_cpu_snapshots -- --nocapture`

## Reports

Use the workspace `xtask` wrapper when you want a smoother review flow.

- Regenerate current images for one backend:
  `cargo run -p xtask -- generate --backend vello_cpu`
- Generate current images and write an HTML diff report:
  `cargo run -p xtask -- report --backend vello_cpu --generate`
- Start a local review server:
  `cargo run -p xtask -- review --backend vello_cpu --generate`
- Narrow generation and reporting to one case:
  `cargo run -p xtask -- report --backend vello_cpu --generate --case gm_strokes`
- Write the report to a specific file:
  `cargo run -p xtask -- report --backend vello_cpu --generate --output /tmp/vello_cpu_report.html`

Supported `xtask` backends are:
- `skia`
- `tiny_skia`
- `vello`
- `vello_cpu`
- `vello_hybrid`

The report compares:
- blessed snapshots in `imaging_snapshot_tests/tests/snapshots/<backend>`
- current outputs in `imaging_snapshot_tests/tests/current/<backend>`
