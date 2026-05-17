<div align="center">

# SVG Imaging

**SVG parsing and rendering through imaging.**

[![Latest published version.](https://img.shields.io/crates/v/svg_imaging.svg)](https://crates.io/crates/svg_imaging)
[![Documentation build status.](https://img.shields.io/docsrs/svg_imaging.svg)](https://docs.rs/svg_imaging)
[![Apache 2.0 or MIT license.](https://img.shields.io/badge/license-Apache--2.0_OR_MIT-blue.svg)](#license)
\
[![GitHub Actions CI status.](https://img.shields.io/github/actions/workflow/status/forest-rs/imaging/ci.yml?logo=github&label=CI)](https://github.com/forest-rs/imaging/actions)

</div>

<!-- We use cargo-rdme to update the README with the contents of lib.rs.
To edit the following section, update it in lib.rs, then run:
cargo rdme --workspace-project=svg_imaging --heading-base-level=0
Full documentation at https://github.com/orium/cargo-rdme -->

<!-- Intra-doc links used in lib.rs may be evaluated here. -->

<!-- cargo-rdme start -->

`svg_imaging`: parse SVG with `usvg` and render it through `imaging`.

The crate intentionally sits between SVG parsing and backend rendering:
- [`usvg`] parses and normalizes SVG input.
- `svg_imaging` lowers supported SVG semantics into a crate-local render plan.
- [`imaging`] executes that plan through [`imaging::Painter`].

This crate is explicit about gaps. Unsupported features are reported in [`RenderReport`]
instead of being silently discarded.

Current support includes path fills and strokes, gradients, paint order, isolated group
compositing, clip paths including referenced clip-path chains, masks lowered through reusable
`imaging` mask definitions, text lowered through `usvg`'s flattened vector output, nested SVG
`<image>` nodes, and raster PNG/JPEG/GIF/WebP `<image>` nodes. Filters and pattern paints are
still reported as unsupported.

Text rendering depends on the parse options' font database. `usvg::Options::default()` starts
with an empty font database, so callers that need text should load fonts into
[`ParseOptions`] before parsing.

`svg_imaging` is a `no_std` plus `alloc` crate. Its optional `std` feature only enables
additional `std` integration and dependency features, and the current dependency stack still
effectively requires `std` today.

```rust
use imaging::{Painter, record::Scene};
use svg_imaging::{ParseOptions, RenderOptions, SvgDocument};

let svg = br#"
    <svg xmlns='http://www.w3.org/2000/svg' width='16' height='16'>
        <rect x='1' y='1' width='14' height='14' fill='#3465a4'/>
    </svg>
"#;

let document = SvgDocument::from_data(svg, &ParseOptions::default())?;
let mut scene = Scene::new();
let mut painter = Painter::new(&mut scene);
let report = document.render(&mut painter, &RenderOptions::default())?;

assert!(report.unsupported_features.is_empty());
assert!(!scene.commands().is_empty());
```

<!-- cargo-rdme end -->

[`imaging`]: https://docs.rs/imaging/latest/imaging/
[`imaging::Painter`]: https://docs.rs/imaging/latest/imaging/struct.Painter.html
[`ParseOptions`]: https://docs.rs/svg_imaging/latest/svg_imaging/type.ParseOptions.html
[`RenderReport`]: https://docs.rs/svg_imaging/latest/svg_imaging/struct.RenderReport.html
[`usvg`]: https://docs.rs/usvg/latest/usvg/

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
