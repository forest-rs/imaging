<div align="center">

# Imaging Vello

**Vello GPU backend for the imaging command stream.**

[![Latest published version.](https://img.shields.io/crates/v/imaging_vello.svg)](https://crates.io/crates/imaging_vello)
[![Documentation build status.](https://img.shields.io/docsrs/imaging_vello.svg)](https://docs.rs/imaging_vello)
[![Apache 2.0 or MIT license.](https://img.shields.io/badge/license-Apache--2.0_OR_MIT-blue.svg)](#license)
\
[![GitHub Actions CI status.](https://img.shields.io/github/actions/workflow/status/forest-rs/imaging/ci.yml?logo=github&label=CI)](https://github.com/forest-rs/imaging/actions)

</div>

<!-- We use cargo-rdme to update the README with the contents of lib.rs.
To edit the following section, update it in lib.rs, then run:
cargo rdme --workspace-project=imaging_vello --heading-base-level=0
Full documentation at https://github.com/orium/cargo-rdme -->

<!-- Intra-doc links used in lib.rs may be evaluated here. -->

<!-- cargo-rdme start -->

Vello backend for `imaging`.

This crate provides a headless CPU/GPU renderer that consumes native [`vello::Scene`] values
and renders them to GPU targets or RGBA8 image data using `vello` + `wgpu`.

Semantic [`imaging::record::Scene`] values can be lowered to native Vello scenes through
[`VelloRenderer::encode_scene`].

In UI integrations, the host application should usually own the `wgpu` device, queue, and
presentation targets, then pass those handles into [`VelloRenderer`].

Enable exactly one backend compatibility feature:

- `vello-0-8` (default)
- `vello-0-7`

# Render A Recorded Scene

Record commands into [`imaging::record::Scene`], then render them with [`VelloRenderer`].

```rust
use imaging::{Painter, record};
use imaging_vello::VelloRenderer;
use kurbo::Rect;
use peniko::{Brush, Color};

fn main() -> Result<(), imaging_vello::Error> {
    let paint = Brush::Solid(Color::from_rgb8(0x2a, 0x6f, 0xdb));
    let mut scene = record::Scene::new();

    {
        let mut painter = Painter::new(&mut scene);
        painter.fill_rect(Rect::new(0.0, 0.0, 128.0, 128.0), &paint);
    }

    let mut renderer = VelloRenderer::new(device, queue)?;
    let native = renderer.encode_scene(&scene, 128, 128)?;
    let image = renderer.render(&native, 128, 128)?;
    assert_eq!(image.width, 128);
    Ok(())
}
```

# Record Into `vello::Scene`

If you want a backend-native retained scene without going through a renderer, wrap a
mutable [`vello::Scene`] with [`VelloSceneSink`].

```rust
use imaging::Painter;
use imaging_vello::{VelloSceneSink, vello};
use kurbo::Rect;
use peniko::{Brush, Color};

fn main() -> Result<(), imaging_vello::Error> {
    let paint = Brush::Solid(Color::from_rgb8(0x1d, 0x4e, 0x89));
    let mut scene = vello::Scene::new();

    {
        let bounds = Rect::new(0.0, 0.0, 128.0, 128.0);
        let mut sink = VelloSceneSink::new(&mut scene, bounds);
        let mut painter = Painter::new(&mut sink);
        painter.fill_rect(bounds, &paint);
        sink.finish()?;
    }

    Ok(())
}
```

# Render A Native `vello::Scene`

If you already have a native Vello scene, hand it directly to [`VelloRenderer`].

```rust
use imaging::Painter;
use imaging_vello::{VelloRenderer, VelloSceneSink, vello};
use kurbo::Rect;
use peniko::{Brush, Color};

fn main() -> Result<(), imaging_vello::Error> {
    let paint = Brush::Solid(Color::from_rgb8(0xd9, 0x77, 0x06));
    let mut scene = vello::Scene::new();

    {
        let bounds = Rect::new(0.0, 0.0, 128.0, 128.0);
        let mut sink = VelloSceneSink::new(&mut scene, bounds);
        let mut painter = Painter::new(&mut sink);
        painter.fill_rect(bounds, &paint);
        sink.finish()?;
    }

    let mut renderer = VelloRenderer::new(device, queue)?;
    let image = renderer.render(&scene, 128, 128)?;
    assert_eq!(image.width, 128);
    Ok(())
}
```

Note: Vello uses a single layer stack for clipping and blending. Scenes that interleave clips
and groups in ways Vello cannot represent may return [`Error::UnbalancedLayerStack`].

<!-- cargo-rdme end -->

[`Error::UnbalancedLayerStack`]: https://docs.rs/imaging_vello/latest/imaging_vello/enum.Error.html#variant.UnbalancedLayerStack
[`imaging::record::Scene`]: https://docs.rs/imaging/latest/imaging/record/struct.Scene.html
[`VelloRenderer`]: https://docs.rs/imaging_vello/latest/imaging_vello/struct.VelloRenderer.html
[`VelloRenderer::encode_scene`]: https://docs.rs/imaging_vello/latest/imaging_vello/struct.VelloRenderer.html#method.encode_scene
[`VelloSceneSink`]: https://docs.rs/imaging_vello/latest/imaging_vello/struct.VelloSceneSink.html
[`vello::Scene`]: https://docs.rs/vello/latest/vello/struct.Scene.html

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
