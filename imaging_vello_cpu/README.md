<div align="center">

# Imaging Vello CPU

**Vello CPU backend for the imaging command stream.**

[![Latest published version.](https://img.shields.io/crates/v/imaging_vello_cpu.svg)](https://crates.io/crates/imaging_vello_cpu)
[![Documentation build status.](https://img.shields.io/docsrs/imaging_vello_cpu.svg)](https://docs.rs/imaging_vello_cpu)
[![Apache 2.0 or MIT license.](https://img.shields.io/badge/license-Apache--2.0_OR_MIT-blue.svg)](#license)
\
[![GitHub Actions CI status.](https://img.shields.io/github/actions/workflow/status/forest-rs/imaging/ci.yml?logo=github&label=CI)](https://github.com/forest-rs/imaging/actions)

</div>

<!-- We use cargo-rdme to update the README with the contents of lib.rs.
To edit the following section, update it in lib.rs, then run:
cargo rdme --workspace-project=imaging_vello_cpu --heading-base-level=0
Full documentation at https://github.com/orium/cargo-rdme -->

<!-- Intra-doc links used in lib.rs may be evaluated here. -->

<!-- cargo-rdme start -->

Vello CPU backend for `imaging`.

This crate provides a CPU renderer that consumes `imaging::record::Scene` (or accepts commands
directly via `imaging::PaintSink`) and produces an RGBA8 image buffer using `vello_cpu`.

# Render A Recorded Scene

Record commands into [`imaging::record::Scene`], then render them with [`VelloCpuRenderer`].

```rust
use imaging::{Painter, record};
use imaging_vello_cpu::VelloCpuRenderer;
use kurbo::Rect;
use peniko::{Brush, Color};

fn main() -> Result<(), imaging_vello_cpu::Error> {
    let paint = Brush::Solid(Color::from_rgb8(0x2a, 0x6f, 0xdb));
    let mut scene = record::Scene::new();

    {
        let mut painter = Painter::new(&mut scene);
        painter.fill_rect(Rect::new(0.0, 0.0, 128.0, 128.0), &paint);
    }

    let mut renderer = VelloCpuRenderer::new(128, 128);
    let image = renderer.render_scene(&scene, 128, 128)?;
    assert_eq!(image.data.len(), 128 * 128 * 4);
    Ok(())
}
```

# Stream Commands Directly

[`VelloCpuRenderer`] also implements [`imaging::PaintSink`], so you can stream commands
directly and call [`VelloCpuRenderer::finish`] when the frame is complete.

```rust
use imaging::Painter;
use imaging_vello_cpu::VelloCpuRenderer;
use kurbo::Rect;
use peniko::{Brush, Color};

fn main() -> Result<(), imaging_vello_cpu::Error> {
    let paint = Brush::Solid(Color::from_rgb8(0xd9, 0x77, 0x06));
    let mut renderer = VelloCpuRenderer::new(128, 128);

    {
        let mut painter = Painter::new(&mut renderer);
        painter.fill_rect(Rect::new(16.0, 16.0, 112.0, 112.0), &paint);
    }

    let image = renderer.finish()?;
    assert_eq!(image.data.len(), 128 * 128 * 4);
    Ok(())
}
```

<!-- cargo-rdme end -->

[`imaging::PaintSink`]: https://docs.rs/imaging/latest/imaging/trait.PaintSink.html
[`imaging::record::Scene`]: https://docs.rs/imaging/latest/imaging/record/struct.Scene.html
[`VelloCpuRenderer`]: https://docs.rs/imaging_vello_cpu/latest/imaging_vello_cpu/struct.VelloCpuRenderer.html
[`VelloCpuRenderer::finish`]: https://docs.rs/imaging_vello_cpu/latest/imaging_vello_cpu/struct.VelloCpuRenderer.html#method.finish

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
