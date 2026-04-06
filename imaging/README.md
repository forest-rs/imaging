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
use imaging::{
    BlurredRoundedRect, ClipRef, FillRef, GlyphRunRef, GroupRef, PaintSink, Painter, StrokeRef,
};
use kurbo::Rect;
use peniko::Color;

#[derive(Default)]
struct CountingSink {
    fills: usize,
    clips: usize,
}

impl PaintSink for CountingSink {
    fn push_clip(&mut self, _clip: ClipRef<'_>) {
        self.clips += 1;
    }

    fn pop_clip(&mut self) {}

    fn push_group(&mut self, _group: GroupRef<'_>) {}

    fn pop_group(&mut self) {}

    fn fill(&mut self, _draw: FillRef<'_>) {
        self.fills += 1;
    }

    fn stroke(&mut self, _draw: StrokeRef<'_>) {}

    fn glyph_run(
        &mut self,
        _draw: GlyphRunRef<'_>,
        _glyphs: &mut dyn Iterator<Item = imaging::record::Glyph>,
    ) {}

    fn blurred_rounded_rect(&mut self, _draw: BlurredRoundedRect) {}
}

let mut sink = CountingSink::default();

{
    let mut painter = Painter::new(&mut sink);
    painter.fill_rect(Rect::new(0.0, 0.0, 64.0, 64.0), Color::from_rgb8(0x2a, 0x6f, 0xdb));
    painter.with_fill_clip(Rect::new(8.0, 8.0, 56.0, 56.0), |p| {
        p.fill_rect(Rect::new(16.0, 16.0, 48.0, 48.0), Color::from_rgb8(0x2a, 0x6f, 0xdb));
    });
}

assert_eq!(sink.fills, 2);
assert_eq!(sink.clips, 1);
```

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

The API is intentionally small and experimental; expect breaking changes while we iterate.

<!-- cargo-rdme end -->

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
