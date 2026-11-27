<div align="center">

# Imaging

**Backend-agnostic imaging IR and recording for forest-rs.**

[![Latest published version.](https://img.shields.io/crates/v/imaging.svg)](https://crates.io/crates/imaging)
[![Documentation build status.](https://img.shields.io/docsrs/imaging.svg)](https://docs.rs/imaging)
[![Apache 2.0 or MIT license.](https://img.shields.io/badge/license-Apache--2.0_OR_MIT-blue.svg)](#license)
\
[![GitHub Actions CI status.](https://img.shields.io/github/actions/workflow/status/forest-rs/imaging/ci.yml?logo=github&label=CI)](https://github.com/forest-rs/imaging/actions)

</div>

<!-- We use cargo-rdme to update the README with the contents of lib.rs.
To edit the following section, update it in lib.rs, then run:
cargo rdme --workspace-project=imaging --heading-base-level=0
Full documentation at https://github.com/orium/cargo-rdme -->

<!-- Intra-doc links used in lib.rs may be evaluated here. -->

<!-- cargo-rdme start -->

`imaging`: backend-agnostic 2D imaging recording + streaming API.

`imaging` has two primary workflows:
- Painting: stream borrowed commands into any [`PaintSink`] with [`Painter`].
- Recording: retain an owned command stream in [`record::Scene`] for validation and replay.

Start with [`guide::learning_path`] for runnable examples. Use [`guide::mental_model`] to
understand the command stream and [`guide::backends`] to choose a renderer.

The root of the crate is intentionally focused on the streaming surface and the shared drawing
vocabulary. Retained scene data and low-level recording payloads live under [`record`].

Optional scoped context annotations can be attached during painting with
[`Painter::with_context`], [`Painter::with_context_ref`], or the [`with_context!`] macro.
Recorded scenes intern context strings and file names only when contexts are actually emitted,
so structured context values like widget IDs or child indices do not require eager string
formatting on the hot path.

# Painting

Use [`Painter`] when you want to stream commands directly into a renderer, backend-native
recorder, validator, or any other [`PaintSink`] implementation without first constructing owned
retained payloads.

```rust
use imaging::{Painter, kurbo::Rect, peniko::Color, record::Scene};

let mut scene = Scene::new();
let mut painter = Painter::new(&mut scene);
painter.fill_rect(Rect::new(0.0, 0.0, 64.0, 64.0), Color::WHITE);
painter.with_fill_clip(Rect::new(8.0, 8.0, 56.0, 56.0), |painter| {
    painter.fill_rect(Rect::new(16.0, 16.0, 48.0, 48.0), Color::from_rgb8(0x2a, 0x6f, 0xdb));
});
```

The [counting sink example] implements [`PaintSink`] to receive the same commands.

[counting sink example]: https://github.com/forest-rs/imaging/blob/main/imaging_examples/examples/counting_sink.rs

# Recording

Use [`record::Scene`] when you want an owned, backend-agnostic recording you can retain,
validate, diagnose, compare in tests, and replay into another sink later.

```rust
use imaging::{record, Painter};
use kurbo::Rect;
use peniko::Color;

let mut scene = record::Scene::new();

{
    let mut painter = Painter::new(&mut scene);
    painter.fill_rect(Rect::new(0.0, 0.0, 64.0, 64.0), Color::from_rgb8(0x12, 0x34, 0x56));
}

scene.validate()?;
assert!(scene.diagnose().is_empty());
assert_eq!(scene.commands().len(), 1);

let mut replayed = record::Scene::new();
record::replay(&scene, &mut replayed);
assert_eq!(scene, replayed);
```

Low-level retained payloads like [`record::Draw`], [`record::Clip`], and [`record::Group`] are
also public under [`record`] when you need exact control over the recorded representation.

# Feature Flags

The default feature set enables `std` support in the geometry and color dependencies. For
`no_std` builds, disable default features and enable `libm`:

```toml
imaging = { version = "0.0.1", default-features = false, features = ["libm"] }
```

Images use [`ImageBrush`] and [`ImageBrushRef`] throughout fills, strokes, glyph runs, and
[`Painter::draw_image`]. [`Image::Raster`] holds shared [`peniko::ImageData`] pixels;
[`Image::External`] holds a typed resource handle bound by a renderer. Core recordings own
handles without depending on GPU types. Encoded scenes keep the backend resources they sample
alive, and unsupported or missing bindings produce explicit errors.

The API is intentionally small and experimental; expect breaking changes while we iterate.

<!-- cargo-rdme end -->

[`guide::backends`]: https://docs.rs/imaging/latest/imaging/guide/backends/
[`guide::learning_path`]: https://docs.rs/imaging/latest/imaging/guide/learning_path/
[`guide::mental_model`]: https://docs.rs/imaging/latest/imaging/guide/mental_model/
[`PaintSink`]: https://docs.rs/imaging/latest/imaging/trait.PaintSink.html
[`Painter`]: https://docs.rs/imaging/latest/imaging/struct.Painter.html
[`Painter::with_context`]: https://docs.rs/imaging/latest/imaging/struct.Painter.html#method.with_context
[`Painter::with_context_ref`]: https://docs.rs/imaging/latest/imaging/struct.Painter.html#method.with_context_ref
[`record`]: https://docs.rs/imaging/latest/imaging/record/
[`record::Clip`]: https://docs.rs/imaging/latest/imaging/record/enum.Clip.html
[`record::Draw`]: https://docs.rs/imaging/latest/imaging/record/enum.Draw.html
[`record::Group`]: https://docs.rs/imaging/latest/imaging/record/struct.Group.html
[`record::Scene`]: https://docs.rs/imaging/latest/imaging/record/struct.Scene.html
[`with_context!`]: https://docs.rs/imaging/latest/imaging/macro.with_context.html

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
