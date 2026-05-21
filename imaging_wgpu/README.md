<div align="center">

# Imaging wgpu

**wgpu texture rendering traits for imaging backends.**

[![Latest published version.](https://img.shields.io/crates/v/imaging_wgpu.svg)](https://crates.io/crates/imaging_wgpu)
[![Documentation build status.](https://img.shields.io/docsrs/imaging_wgpu.svg)](https://docs.rs/imaging_wgpu)
[![Apache 2.0 or MIT license.](https://img.shields.io/badge/license-Apache--2.0_OR_MIT-blue.svg)](#license)
\
[![GitHub Actions CI status.](https://img.shields.io/github/actions/workflow/status/forest-rs/imaging/ci.yml?logo=github&label=CI)](https://github.com/forest-rs/imaging/actions)

</div>

<!-- We use cargo-rdme to update the README with the contents of lib.rs.
To edit the following section, update it in lib.rs, then run:
cargo rdme --workspace-project=imaging_wgpu --heading-base-level=0
Full documentation at https://github.com/orium/cargo-rdme -->

<!-- Intra-doc links used in lib.rs may be evaluated here. -->

<!-- cargo-rdme start -->

`wgpu` texture rendering traits for `imaging` backends.

The default feature set enables `wgpu-28`. Consumers that need `wgpu-27` should disable
default features and enable `wgpu-27`.

Workspace-wide `--all-features` builds enable both, so this crate resolves that case
deterministically by exporting `wgpu-28` as [`wgpu`] and keeping `wgpu-27` linked only to
satisfy older dependents that may still request it transitively.

This crate is `std`-only because it depends on `wgpu`.

<!-- cargo-rdme end -->

[`wgpu`]: https://docs.rs/imaging_wgpu/latest/imaging_wgpu/wgpu/

## Minimum supported Rust Version (MSRV)

This crate has been verified to compile with **Rust 1.92** and later.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE] or <http://www.apache.org/licenses/LICENSE-2.0>), or
- MIT license ([LICENSE-MIT] or <http://opensource.org/licenses/MIT>),

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you,
as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

## Contribution

Contributions are welcome by pull request. The [Rust code of conduct] applies.
Please feel free to add your name to the [AUTHORS] file in any substantive pull request.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you,
as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

[LICENSE-APACHE]: https://github.com/forest-rs/imaging/blob/main/LICENSE-APACHE
[LICENSE-MIT]: https://github.com/forest-rs/imaging/blob/main/LICENSE-MIT
[Rust code of conduct]: https://www.rust-lang.org/policies/code-of-conduct
[AUTHORS]: https://github.com/forest-rs/imaging/blob/main/AUTHORS
