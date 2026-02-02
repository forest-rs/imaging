# `imaging_snapshot_tests`

Kompari-based image snapshot tests for `imaging` backends.

## Running

- `cargo test -p imaging_snapshot_tests --features vello_cpu --test vello_cpu_snapshots`
- Accept new baselines: `IMAGING_TEST=accept cargo test -p imaging_snapshot_tests --features vello_cpu --test vello_cpu_snapshots`
