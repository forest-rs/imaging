<div align="center">

# Imaging Vello Hybrid

**Vello hybrid CPU/GPU backend for the imaging command stream.**

[![Latest published version.](https://img.shields.io/crates/v/imaging_vello_hybrid.svg)](https://crates.io/crates/imaging_vello_hybrid)
[![Documentation build status.](https://img.shields.io/docsrs/imaging_vello_hybrid.svg)](https://docs.rs/imaging_vello_hybrid)
[![Apache 2.0 or MIT license.](https://img.shields.io/badge/license-Apache--2.0_OR_MIT-blue.svg)](#license)
\
[![GitHub Actions CI status.](https://img.shields.io/github/actions/workflow/status/forest-rs/imaging/ci.yml?logo=github&label=CI)](https://github.com/forest-rs/imaging/actions)

</div>

<!-- We use cargo-rdme to update the README with the contents of lib.rs.
To edit the following section, update it in lib.rs, then run:
cargo rdme --workspace-project=imaging_vello_hybrid --heading-base-level=0
Full documentation at https://github.com/orium/cargo-rdme -->

<!-- Intra-doc links used in lib.rs may be evaluated here. -->

<!-- cargo-rdme start -->

Vello hybrid backend for `imaging`.

This crate provides a headless CPU/GPU renderer that consumes native
[`VelloHybridScene`] values and renders them to GPU targets or RGBA8 image data using
`vello_gpu` + `wgpu`.

Semantic [`imaging::record::Scene`] values can be lowered to native hybrid scenes through
[`VelloHybridRenderer::encode_scene`].

In UI integrations, the host application should usually own the `wgpu` device, queue, and
presentation targets, then pass those handles into [`VelloHybridRenderer`].

Recorded scenes with inline image brushes are uploaded through a renderer-scoped image registry
and translated to backend-managed opaque image ids. Encoded scenes retain these uploads
until reset or dropped. Use [`VelloHybridSceneSink::with_renderer`]
when recording directly into a native [`VelloHybridScene`] and you want the same image
support.

Alpha masks render the mask scene into an isolated `DestIn` layer, preserving its paint,
blending, and image bindings. Luminance masks remain unsupported by this backend.

Filter chains use nested single-primitive layers. Blurs and drop shadows require equal
X/Y standard deviations in destination coordinates. Transforms that require an anisotropic
or sheared blur kernel return [`Error::UnsupportedFilter`].
Blurred rounded rectangles use the renderer's native analytic draw.

Synthetic emboldening uses glifo's native outline expansion. In glifo 0.4,
unhinted fills expand in font units rather than font-size pixels, and absorbed
transform scales do not scale the expansion amount.

# Render A Recorded Scene

Record commands into [`imaging::record::Scene`], then render them with
[`VelloHybridRenderer`].

```rust
use imaging::{Painter, record};
use imaging_vello_hybrid::VelloHybridRenderer;
use kurbo::Rect;
use peniko::{Brush, Color};

fn main() -> Result<(), imaging_vello_hybrid::Error> {
    let paint = Brush::Solid(Color::from_rgb8(0x2a, 0x6f, 0xdb));
    let mut scene = record::Scene::new();

    {
        let mut painter = Painter::new(&mut scene);
        painter.fill_rect(Rect::new(0.0, 0.0, 128.0, 128.0), &paint);
    }

    let mut renderer = VelloHybridRenderer::new(device, queue);
    let native = renderer.encode_scene(&scene, 128, 128)?;
    let image = renderer.render(&native, 128, 128)?;
    assert_eq!(image.width, 128);
    Ok(())
}
```

# Record A Hybrid Scene

If you want a backend-native retained scene without owning a full renderer, wrap an existing
[`VelloHybridScene`] with [`VelloHybridSceneSink`].

```rust
use imaging::Painter;
use imaging_vello_hybrid::{VelloHybridScene, VelloHybridSceneSink};
use kurbo::Rect;
use peniko::{Brush, Color};

fn main() -> Result<(), imaging_vello_hybrid::Error> {
    let paint = Brush::Solid(Color::from_rgb8(0x1d, 0x4e, 0x89));
    let mut scene = VelloHybridScene::new(128, 128);
    scene.reset();

    {
        let mut sink = VelloHybridSceneSink::new(&mut scene);
        let mut painter = Painter::new(&mut sink);
        painter.fill_rect(Rect::new(0.0, 0.0, 128.0, 128.0), &paint);
        sink.finish()?;
    }

    Ok(())
}
```

Use [`VelloHybridSceneSink::with_renderer`] instead when the scene uses image brushes.

# Record Image Brushes

Use [`VelloHybridSceneSink::with_renderer`] when recording image brushes directly into a
native [`VelloHybridScene`]. The sink uploads images through the renderer and reuses them
across later recordings and renders.

```rust
use std::sync::Arc;

use imaging::Painter;
use imaging_vello_hybrid::{VelloHybridRenderer, VelloHybridScene, VelloHybridSceneSink};
use kurbo::Rect;
use peniko::{Blob, Brush, ImageAlphaType, ImageBrush, ImageData, ImageFormat};

fn main() -> Result<(), imaging_vello_hybrid::Error> {
    let image = ImageData {
        data: Blob::new(Arc::new([
            0xff, 0x20, 0x20, 0xff, 0x20, 0xff, 0x20, 0xff, 0x20, 0x20, 0xff, 0xff, 0xff,
            0xff, 0x20, 0xff,
        ])),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width: 2,
        height: 2,
    };
    let brush = Brush::Image(ImageBrush::new(image));

    let mut renderer = VelloHybridRenderer::new(device, queue);
    let mut scene = VelloHybridScene::new(128, 128);
    scene.reset();

    {
        let mut sink = VelloHybridSceneSink::with_renderer(&mut scene, &mut renderer);
        let mut painter = Painter::new(&mut sink);
        painter.fill_rect(Rect::new(0.0, 0.0, 128.0, 128.0), &brush);
        sink.finish()?;
    }

    let image = renderer.render(&scene, 128, 128)?;
    assert_eq!(image.width, 128);
    Ok(())
}
```

# Render An Encoded Hybrid Scene

If you already have a native hybrid scene, hand it directly to [`VelloHybridRenderer`].

```rust
use imaging::Painter;
use imaging_vello_hybrid::{VelloHybridRenderer, VelloHybridScene, VelloHybridSceneSink};
use kurbo::Rect;
use peniko::{Brush, Color};

fn main() -> Result<(), imaging_vello_hybrid::Error> {
    let paint = Brush::Solid(Color::from_rgb8(0xd9, 0x77, 0x06));
    let mut scene = VelloHybridScene::new(128, 128);
    scene.reset();

    {
        let mut sink = VelloHybridSceneSink::new(&mut scene);
        let mut painter = Painter::new(&mut sink);
        painter.fill_rect(Rect::new(16.0, 16.0, 112.0, 112.0), &paint);
        sink.finish()?;
    }

    let mut renderer = VelloHybridRenderer::new(device, queue);
    let image = renderer.render(&scene, 128, 128)?;
    assert_eq!(image.width, 128);
    Ok(())
}
```
# External Images

Bind a caller-owned texture to an [`imaging::ExternalImage`], then use it through the
normal image brush or drawing API. Encoded scenes retain their bindings after unbinding.
The texture must come from the renderer's device; see the `TextureImageRenderer`
implementation below for format, usage, and alpha requirements.

```rust
use imaging::{ExternalImage, Painter, record};
use imaging_wgpu::TextureImageRenderer;
use peniko::ImageAlphaType;

let image = ExternalImage::new(2, 2, ImageAlphaType::AlphaPremultiplied);
renderer.bind_image(&image, &texture)?;
let mut scene = record::Scene::new();
Painter::new(&mut scene).draw_image(&image, kurbo::Affine::IDENTITY);
let encoded = renderer.encode_scene(&scene, 2, 2)?;
renderer.unbind_image(&image);
let output = renderer.render(&encoded, 2, 2)?;
```

<!-- cargo-rdme end -->

[`imaging::record::Scene`]: https://docs.rs/imaging/latest/imaging/record/struct.Scene.html
[`VelloHybridRenderer`]: https://docs.rs/imaging_vello_hybrid/latest/imaging_vello_hybrid/struct.VelloHybridRenderer.html
[`VelloHybridRenderer::encode_scene`]: https://docs.rs/imaging_vello_hybrid/latest/imaging_vello_hybrid/struct.VelloHybridRenderer.html#method.encode_scene
[`VelloHybridSceneSink`]: https://docs.rs/imaging_vello_hybrid/latest/imaging_vello_hybrid/struct.VelloHybridSceneSink.html
[`VelloHybridSceneSink::with_renderer`]: https://docs.rs/imaging_vello_hybrid/latest/imaging_vello_hybrid/struct.VelloHybridSceneSink.html#method.with_renderer
[`vello_gpu::Scene`]: https://docs.rs/vello_gpu/latest/vello_gpu/struct.Scene.html

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

## Caller-owned encoding

`VelloHybridRenderer::encode_to_texture_view` appends rendering to a caller-owned
command encoder. Existing `render_to_texture_view` callers need no migration;
they still submit immediately. Use the encoding method for timestamp queries or
to combine rendering and presentation work in one submission. Submit on the same
device and queue supplied at renderer construction. Discard the encoder on error.
