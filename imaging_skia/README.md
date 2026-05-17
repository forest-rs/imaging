<div align="center">

# Imaging Skia

**Skia backend for the imaging command stream.**

[![Latest published version.](https://img.shields.io/crates/v/imaging_skia.svg)](https://crates.io/crates/imaging_skia)
[![Documentation build status.](https://img.shields.io/docsrs/imaging_skia.svg)](https://docs.rs/imaging_skia)
[![Apache 2.0 or MIT license.](https://img.shields.io/badge/license-Apache--2.0_OR_MIT-blue.svg)](#license)
\
[![GitHub Actions CI status.](https://img.shields.io/github/actions/workflow/status/forest-rs/imaging/ci.yml?logo=github&label=CI)](https://github.com/forest-rs/imaging/actions)

</div>

<!-- We use cargo-rdme to update the README with the contents of lib.rs.
To edit the following section, update it in lib.rs, then run:
cargo rdme --workspace-project=imaging_skia --heading-base-level=0
Full documentation at https://github.com/orium/cargo-rdme -->

<!-- Intra-doc links used in lib.rs may be evaluated here. -->

<!-- cargo-rdme start -->

Skia backend for `imaging`.

This crate provides a CPU raster renderer that consumes `imaging::record::Scene` or native
Skia draw targets and produces an RGBA8 image buffer using Skia.

# Render A Recorded Scene

Record commands into [`imaging::record::Scene`], then hand the scene to [`SkiaCpuRenderer`].

```rust
use imaging::{Painter, record};
use imaging_skia::SkiaCpuRenderer;
use kurbo::Rect;
use peniko::{Brush, Color};

fn main() -> Result<(), imaging_skia::Error> {
    let paint = Brush::Solid(Color::from_rgb8(0x2a, 0x6f, 0xdb));
    let mut scene = record::Scene::new();

    {
        let mut painter = Painter::new(&mut scene);
        painter.fill_rect(Rect::new(0.0, 0.0, 128.0, 128.0), &paint);
    }

    let mut renderer = SkiaCpuRenderer::new();
    let image = renderer.render_scene(&scene, 128, 128)?;
    assert_eq!(image.data.len(), 128 * 128 * 4);
    Ok(())
}
```

# Draw Into An Existing `Canvas`

If you already have a Skia canvas, wrap it with [`SkCanvasSink`] and stream commands directly.

```rust
use imaging::Painter;
use imaging_skia::SkCanvasSink;
use kurbo::Rect;
use peniko::{Brush, Color};
use skia_safe::surfaces;

fn main() -> Result<(), imaging_skia::Error> {
    let paint = Brush::Solid(Color::from_rgb8(0x1d, 0x4e, 0x89));
    let mut surface = surfaces::raster_n32_premul((128, 128)).unwrap();

    {
        let mut sink = SkCanvasSink::new(surface.canvas());
        let mut painter = Painter::new(&mut sink);
        painter.fill_rect(Rect::new(0.0, 0.0, 128.0, 128.0), &paint);
        sink.finish()?;
    }

    Ok(())
}
```

# Record A `SkPicture`

Use [`SkPictureRecorderSink`] when you want Skia's native retained recording format.

```rust
use imaging::Painter;
use imaging_skia::SkPictureRecorderSink;
use kurbo::Rect;
use peniko::{Brush, Color};

fn main() -> Result<(), imaging_skia::Error> {
    let paint = Brush::Solid(Color::from_rgb8(0x7c, 0x3a, 0xed));
    let mut sink = SkPictureRecorderSink::new(Rect::new(0.0, 0.0, 128.0, 128.0));

    {
        let mut painter = Painter::new(&mut sink);
        painter.fill_rect(Rect::new(16.0, 16.0, 112.0, 112.0), &paint);
    }

    let picture = sink.finish_picture()?;
    assert_eq!(picture.cull_rect().right, 128.0);
    Ok(())
}
```

# Render A Native `SkPicture`

If you already have a recorded picture, hand it directly to [`SkiaCpuRenderer`].

```rust
use imaging::Painter;
use imaging_skia::{SkPictureRecorderSink, SkiaCpuRenderer};
use kurbo::Rect;
use peniko::{Brush, Color};

fn main() -> Result<(), imaging_skia::Error> {
    let paint = Brush::Solid(Color::from_rgb8(0xd9, 0x77, 0x06));
    let mut sink = SkPictureRecorderSink::new(Rect::new(0.0, 0.0, 128.0, 128.0));

    {
        let mut painter = Painter::new(&mut sink);
        painter.fill_rect(Rect::new(16.0, 16.0, 112.0, 112.0), &paint);
    }

    let picture = sink.finish_picture()?;
    let mut renderer = SkiaCpuRenderer::new();
    let image = renderer.render_picture(&picture, 128, 128)?;
    assert_eq!(image.data.len(), 128 * 128 * 4);
    Ok(())
}
```

# GPU Rendering

Enable the `gpu` feature when you want Skia Ganesh rendering through app-owned `wgpu`
handles. [`SkiaRenderer`] reuses the current backend selected by `wgpu` and renders
native [`skia_safe::Picture`] values into caller-owned `wgpu::Texture` targets while also
supporting RGBA8 image output.

```rust
use imaging::{Painter, record};
use imaging_skia::SkiaRenderer;
use kurbo::Rect;
use peniko::{Brush, Color};

let mut scene = record::Scene::new();
{
    let mut painter = Painter::new(&mut scene);
    painter.fill_rect(
        Rect::new(0.0, 0.0, 128.0, 128.0),
        &Brush::Solid(Color::from_rgb8(0x2a, 0x6f, 0xdb)),
    );
}

let mut renderer = SkiaRenderer::new(adapter, device, queue)?;
let picture = renderer.encode_scene(&scene, 128, 128)?;
renderer.render_picture_to_texture(&picture, &texture)?;
```

<!-- cargo-rdme end -->

[`imaging::record::Scene`]: https://docs.rs/imaging/latest/imaging/record/struct.Scene.html
[`SkCanvasSink`]: https://docs.rs/imaging_skia/latest/imaging_skia/struct.SkCanvasSink.html
[`SkiaCpuRenderer`]: https://docs.rs/imaging_skia/latest/imaging_skia/struct.SkiaCpuRenderer.html
[`SkiaRenderer`]: https://docs.rs/imaging_skia/latest/imaging_skia/struct.SkiaRenderer.html
[`SkPictureRecorderSink`]: https://docs.rs/imaging_skia/latest/imaging_skia/struct.SkPictureRecorderSink.html
[`skia_safe::Picture`]: https://docs.rs/skia-safe/latest/skia_safe/type.Picture.html

## Building

`skia-safe` / `skia-bindings` normally download prebuilt Skia binaries at build time. In offline
or sandboxed environments, set `SKIA_BINARIES_URL` to a local `tar.gz` downloaded ahead of time:

```sh
SKIA_BINARIES_URL='file:///absolute/path/to/skia-binaries-....tar.gz' cargo build
```

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
