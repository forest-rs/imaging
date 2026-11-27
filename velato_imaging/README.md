<div align="center">

# Velato Imaging

**Velato rendering through imaging.**

[![Latest published version.](https://img.shields.io/crates/v/velato_imaging.svg)](https://crates.io/crates/velato_imaging)
[![Documentation build status.](https://img.shields.io/docsrs/velato_imaging.svg)](https://docs.rs/velato_imaging)
[![Apache 2.0 or MIT license.](https://img.shields.io/badge/license-Apache--2.0_OR_MIT-blue.svg)](#license)
\
[![GitHub Actions CI status.](https://img.shields.io/github/actions/workflow/status/forest-rs/imaging/ci.yml?logo=github&label=CI)](https://github.com/forest-rs/imaging/actions)

</div>

<!-- We use cargo-rdme to update the README with the contents of lib.rs.
To edit the following section, update it in lib.rs, then run:
cargo rdme --workspace-project=velato_imaging --heading-base-level=0
Full documentation at https://github.com/orium/cargo-rdme -->

<!-- Intra-doc links used in lib.rs may be evaluated here. -->

<!-- cargo-rdme start -->

Render Velato animations through `imaging`.

This crate adapts Velato's [`velato::RenderSink`] abstraction to [`imaging::PaintSink`].
Use [`ImagingSink`] when you want to stream Velato output directly into an imaging backend or
recorder, or use [`RendererExt`] when you want convenience helpers on [`velato::Renderer`].

```rust
use imaging::record::Scene;
use kurbo::Affine;
use velato::Composition;
use velato_imaging::{RendererExt, velato};

let composition = Composition::default();
let mut renderer = velato::Renderer::new();
let mut scene = Scene::new();

renderer.append_to_imaging(&composition, 0.0, Affine::IDENTITY, 1.0, &mut scene)?;
assert_eq!(scene.validate(), Ok(()));
```

<!-- cargo-rdme end -->

[`imaging::PaintSink`]: https://docs.rs/imaging/latest/imaging/trait.PaintSink.html
[`ImagingSink`]: https://docs.rs/velato_imaging/latest/velato_imaging/struct.ImagingSink.html
[`RendererExt`]: https://docs.rs/velato_imaging/latest/velato_imaging/trait.RendererExt.html
[`velato::Renderer`]: https://docs.rs/velato/latest/velato/struct.Renderer.html
[`velato::RenderSink`]: https://docs.rs/velato/latest/velato/trait.RenderSink.html

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
