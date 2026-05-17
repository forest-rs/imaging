<div align="center">

# Imaging Tiny Skia

**tiny-skia backend for the imaging command stream.**

[![Latest published version.](https://img.shields.io/crates/v/imaging_tiny_skia.svg)](https://crates.io/crates/imaging_tiny_skia)
[![Documentation build status.](https://img.shields.io/docsrs/imaging_tiny_skia.svg)](https://docs.rs/imaging_tiny_skia)
[![Apache 2.0 or MIT license.](https://img.shields.io/badge/license-Apache--2.0_OR_MIT-blue.svg)](#license)
\
[![GitHub Actions CI status.](https://img.shields.io/github/actions/workflow/status/forest-rs/imaging/ci.yml?logo=github&label=CI)](https://github.com/forest-rs/imaging/actions)

</div>

<!-- We use cargo-rdme to update the README with the contents of lib.rs.
To edit the following section, update it in lib.rs, then run:
cargo rdme --workspace-project=imaging_tiny_skia --heading-base-level=0
Full documentation at https://github.com/orium/cargo-rdme -->

<!-- Intra-doc links used in lib.rs may be evaluated here. -->

<!-- cargo-rdme start -->

tiny-skia backend for `imaging`.

This crate provides a CPU renderer that consumes `imaging::record::Scene` values or streaming
`imaging::PaintSink` commands and produces RGBA8 image buffers using `tiny-skia`.

The implementation was integrated from Floem's tiny-skia renderer and adapted to match the
public renderer shape used by the other `imaging_*` backends.

<!-- cargo-rdme end -->

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
