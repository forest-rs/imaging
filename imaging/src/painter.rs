// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Painter-style authoring helpers built on top of [`PaintSink`].

use core::borrow::Borrow;

use kurbo::{Affine, BezPath, CubicBez, Line, QuadBez, Rect, RoundedRect, Stroke};
use peniko::{BrushRef, ImageBrushRef, Style};

use crate::{
    BlurredRoundedRect, ClipRef, Composite, FillRef, GeometryRef, GlyphRunRef, GroupRef,
    NormalizedCoord, PaintSink, StrokeRef, record,
};

const DEFAULT_SHAPE_TOLERANCE: f64 = 0.1;

/// Shape accepted by [`Painter`] fill and stroke entry points.
///
/// Cheap retained geometry like [`Rect`], [`RoundedRect`], and [`BezPath`] stays in its native
/// representation. Other supported Kurbo shapes are flattened into a path using a default
/// tolerance of `0.1`.
pub trait PaintShape<'a> {
    /// Convert this shape into borrowed imaging geometry.
    #[must_use]
    fn into_geometry_ref(self) -> GeometryRef<'a>;
}

impl<'a> PaintShape<'a> for GeometryRef<'a> {
    fn into_geometry_ref(self) -> Self {
        self
    }
}

impl<'a> PaintShape<'a> for Rect {
    fn into_geometry_ref(self) -> GeometryRef<'a> {
        self.into()
    }
}

impl<'a> PaintShape<'a> for &'a Rect {
    fn into_geometry_ref(self) -> GeometryRef<'a> {
        (*self).into()
    }
}

impl<'a> PaintShape<'a> for RoundedRect {
    fn into_geometry_ref(self) -> GeometryRef<'a> {
        self.into()
    }
}

impl<'a> PaintShape<'a> for &'a RoundedRect {
    fn into_geometry_ref(self) -> GeometryRef<'a> {
        (*self).into()
    }
}

impl<'a> PaintShape<'a> for &'a BezPath {
    fn into_geometry_ref(self) -> GeometryRef<'a> {
        self.into()
    }
}

impl<'a> PaintShape<'a> for BezPath {
    fn into_geometry_ref(self) -> GeometryRef<'a> {
        self.into()
    }
}

impl<'a> PaintShape<'a> for record::Geometry {
    fn into_geometry_ref(self) -> GeometryRef<'a> {
        self.into()
    }
}

impl<'a> PaintShape<'a> for &'a record::Geometry {
    fn into_geometry_ref(self) -> GeometryRef<'a> {
        self.into()
    }
}

macro_rules! impl_path_shape {
    ($($ty:ty),* $(,)?) => {
        $(
            impl<'a> PaintShape<'a> for $ty {
                fn into_geometry_ref(self) -> GeometryRef<'a> {
                    GeometryRef::OwnedPath(kurbo::Shape::to_path(&self, DEFAULT_SHAPE_TOLERANCE))
                }
            }

            impl<'a> PaintShape<'a> for &'a $ty {
                fn into_geometry_ref(self) -> GeometryRef<'a> {
                    GeometryRef::OwnedPath(kurbo::Shape::to_path(self, DEFAULT_SHAPE_TOLERANCE))
                }
            }
        )*
    };
}

impl_path_shape!(
    kurbo::Arc,
    kurbo::Circle,
    CubicBez,
    kurbo::Ellipse,
    Line,
    QuadBez,
);

/// Painter-style authoring wrapper over a [`PaintSink`].
#[derive(Debug)]
pub struct Painter<'a, S: PaintSink + ?Sized = dyn PaintSink + 'a> {
    sink: &'a mut S,
}

impl<'a, S> Painter<'a, S>
where
    S: PaintSink + ?Sized,
{
    /// Wrap a paint sink with painter-style authoring helpers.
    #[must_use]
    pub fn new(sink: &'a mut S) -> Self {
        Self { sink }
    }

    /// Borrow the wrapped sink directly.
    ///
    /// This is the low-level escape hatch for APIs that need the underlying [`PaintSink`]
    /// instead of painter-style helpers.
    pub fn sink_mut(&mut self) -> &mut S {
        self.sink
    }

    /// Replay a recorded scene into the wrapped sink.
    ///
    /// This forwards to [`record::replay`] without requiring callers to peel the sink back out of
    /// the painter.
    pub fn replay(&mut self, scene: &record::Scene) {
        record::replay(scene, self.sink);
    }

    /// Start configuring a fill draw.
    ///
    /// Defaults:
    /// - geometry transform: [`Affine::IDENTITY`]
    /// - fill rule: [`peniko::Fill::NonZero`]
    /// - brush transform: `None`
    /// - compositing: [`Composite::default()`]
    pub fn fill<'b>(
        &'b mut self,
        shape: impl PaintShape<'b>,
        brush: impl Into<BrushRef<'b>>,
    ) -> FillBuilder<'b, S> {
        FillBuilder {
            sink: self.sink,
            draw: FillRef::new(shape.into_geometry_ref(), brush),
        }
    }

    /// Fill a rectangle using default draw state.
    ///
    /// Defaults:
    /// - geometry transform: [`Affine::IDENTITY`]
    /// - fill rule: [`peniko::Fill::NonZero`]
    /// - brush transform: `None`
    /// - compositing: [`Composite::default()`]
    pub fn fill_rect<'b>(&mut self, rect: Rect, brush: impl Into<BrushRef<'b>>) {
        self.sink.fill(FillRef::new(rect, brush));
    }

    /// Start configuring a stroke draw.
    ///
    /// Defaults:
    /// - geometry transform: [`Affine::IDENTITY`]
    /// - brush transform: `None`
    /// - compositing: [`Composite::default()`]
    pub fn stroke<'b>(
        &'b mut self,
        shape: impl PaintShape<'b>,
        stroke: &'b Stroke,
        brush: impl Into<BrushRef<'b>>,
    ) -> StrokeBuilder<'b, S> {
        StrokeBuilder {
            sink: self.sink,
            draw: StrokeRef::new(shape.into_geometry_ref(), stroke, brush),
        }
    }

    /// Start configuring a glyph run.
    ///
    /// Defaults:
    /// - run transform: [`Affine::IDENTITY`]
    /// - per-glyph transform: `None`
    /// - font size: `16.0`
    /// - hinting: `false`
    /// - normalized variation coordinates: `&[]`
    /// - compositing: [`Composite::default()`]
    pub fn glyphs<'b>(
        &'b mut self,
        font: &'b peniko::FontData,
        brush: impl Into<BrushRef<'b>>,
    ) -> GlyphRunBuilder<'b, S> {
        GlyphRunBuilder {
            sink: self.sink,
            font,
            transform: Affine::IDENTITY,
            glyph_transform: None,
            font_size: 16.0,
            hint: false,
            normalized_coords: &[],
            brush: brush.into(),
            composite: Composite::default(),
        }
    }

    /// Emit a blurred rounded rectangle draw directly.
    pub fn blurred_rounded_rect(&mut self, draw: BlurredRoundedRect) {
        self.sink.blurred_rounded_rect(draw);
    }

    /// Draw an image at its natural size with the given transform.
    ///
    /// This is shorthand for filling a `0,0,width,height` rectangle with an image brush using:
    /// - fill rule: [`peniko::Fill::NonZero`]
    /// - brush transform: `None`
    /// - compositing: [`Composite::default()`]
    pub fn draw_image<'b>(&'b mut self, image: impl Into<ImageBrushRef<'b>>, transform: Affine) {
        let image = image.into();
        let rect = Rect::new(
            0.0,
            0.0,
            image.image.width as f64,
            image.image.height as f64,
        );
        self.fill(rect, image).transform(transform).draw();
    }

    /// Push a clip, run the provided closure, then pop the clip.
    pub fn with_clip(&mut self, clip: ClipRef<'_>, f: impl FnOnce(&mut Painter<'_, S>)) {
        self.sink.push_clip(clip);
        {
            let mut painter = Painter::new(self.sink);
            f(&mut painter);
        }
        self.sink.pop_clip();
    }

    /// Push a fill-style clip, run the provided closure, then pop the clip.
    ///
    /// Defaults:
    /// - clip transform: [`Affine::IDENTITY`]
    /// - fill rule: [`peniko::Fill::NonZero`]
    pub fn with_fill_clip<'b>(
        &'b mut self,
        shape: impl Into<GeometryRef<'b>>,
        f: impl FnOnce(&mut Painter<'_, S>),
    ) {
        self.with_clip(ClipRef::fill(shape), f);
    }

    /// Push a fill-style clip with an explicit transform, run the provided closure, then pop the
    /// clip.
    ///
    /// The clip still uses [`peniko::Fill::NonZero`] unless replaced through [`ClipRef`].
    pub fn with_fill_clip_transformed<'b>(
        &'b mut self,
        shape: impl Into<GeometryRef<'b>>,
        transform: Affine,
        f: impl FnOnce(&mut Painter<'_, S>),
    ) {
        self.with_clip(ClipRef::fill(shape).with_transform(transform), f);
    }

    /// Push a stroke-style clip, run the provided closure, then pop the clip.
    ///
    /// Default:
    /// - clip transform: [`Affine::IDENTITY`]
    pub fn with_stroke_clip<'b>(
        &'b mut self,
        shape: impl Into<GeometryRef<'b>>,
        stroke: &'b Stroke,
        f: impl FnOnce(&mut Painter<'_, S>),
    ) {
        self.with_clip(ClipRef::stroke(shape, stroke), f);
    }

    /// Push a stroke-style clip with an explicit transform, run the provided closure, then pop the
    /// clip.
    pub fn with_stroke_clip_transformed<'b>(
        &'b mut self,
        shape: impl Into<GeometryRef<'b>>,
        stroke: &'b Stroke,
        transform: Affine,
        f: impl FnOnce(&mut Painter<'_, S>),
    ) {
        self.with_clip(ClipRef::stroke(shape, stroke).with_transform(transform), f);
    }

    /// Push an isolated group, run the provided closure, then pop the group.
    pub fn with_group(&mut self, group: GroupRef<'_>, f: impl FnOnce(&mut Painter<'_, S>)) {
        self.sink.push_group(group);
        {
            let mut painter = Painter::new(self.sink);
            f(&mut painter);
        }
        self.sink.pop_group();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::sync::Arc;
    use alloc::vec;
    use alloc::vec::Vec;
    use kurbo::{Circle, Point, Shape as _, Vec2};
    use peniko::Fill;

    use crate::{BlurredRoundedRect, GroupRef};

    #[derive(Default)]
    struct RecordingSink {
        pushed_clips: Vec<record::Clip>,
    }

    impl PaintSink for RecordingSink {
        fn push_clip(&mut self, clip: ClipRef<'_>) {
            self.pushed_clips.push(clip.to_owned());
        }

        fn pop_clip(&mut self) {}

        fn push_group(&mut self, _group: GroupRef<'_>) {}

        fn pop_group(&mut self) {}

        fn fill(&mut self, _draw: FillRef<'_>) {}

        fn stroke(&mut self, _draw: StrokeRef<'_>) {}

        fn glyph_run(
            &mut self,
            _draw: GlyphRunRef<'_>,
            _glyphs: &mut dyn Iterator<Item = record::Glyph>,
        ) {
        }

        fn blurred_rounded_rect(&mut self, _draw: BlurredRoundedRect) {}
    }

    #[test]
    fn transformed_fill_clip_preserves_explicit_transform() {
        let mut sink = RecordingSink::default();
        let mut painter = Painter::new(&mut sink);
        let transform = Affine::translate((4.0, 7.0));

        painter.with_fill_clip_transformed(Rect::new(0.0, 0.0, 10.0, 12.0), transform, |_| {});

        assert_eq!(
            sink.pushed_clips,
            vec![record::Clip::Fill {
                transform,
                shape: record::Geometry::Rect(Rect::new(0.0, 0.0, 10.0, 12.0)),
                fill_rule: Fill::NonZero,
            }]
        );
    }

    #[test]
    fn transformed_stroke_clip_preserves_explicit_transform() {
        let mut sink = RecordingSink::default();
        let mut painter = Painter::new(&mut sink);
        let transform = Affine::translate((4.0, 7.0));
        let stroke = Stroke::new(3.0).with_caps(kurbo::Cap::Round);
        let line = Line::new(Point::new(1.0, 2.0), Point::new(11.0, 15.0));
        let path = line.to_path(0.1);

        painter.with_stroke_clip_transformed(path.clone(), &stroke, transform, |_| {});

        assert_eq!(
            sink.pushed_clips,
            vec![record::Clip::Stroke {
                transform,
                shape: record::Geometry::Path(path),
                stroke,
            }]
        );
    }

    #[test]
    fn clip_ref_with_transform_replaces_shape_transform() {
        let clip = ClipRef::fill(Rect::new(0.0, 0.0, 5.0, 6.0))
            .with_transform(Affine::translate(Vec2::new(2.0, 3.0)));

        assert_eq!(
            clip,
            ClipRef::Fill {
                transform: Affine::translate((2.0, 3.0)),
                shape: GeometryRef::Rect(Rect::new(0.0, 0.0, 5.0, 6.0)),
                fill_rule: Fill::NonZero,
            }
        );
    }

    #[test]
    fn draw_image_uses_natural_size_rect() {
        let mut scene = record::Scene::new();
        let mut painter = Painter::new(&mut scene);
        let image = peniko::ImageData {
            data: peniko::Blob::new(Arc::new([0_u8; 16])),
            format: peniko::ImageFormat::Rgba8,
            alpha_type: peniko::ImageAlphaType::Alpha,
            width: 2,
            height: 2,
        };
        let transform = Affine::translate((8.0, 9.0));

        painter.draw_image(&image, transform);

        assert_eq!(
            scene.draw_op(record::DrawId(0)),
            &record::Draw::Fill {
                transform,
                fill_rule: Fill::NonZero,
                brush: peniko::Brush::Image(peniko::ImageBrush::new(image)),
                brush_transform: None,
                shape: record::Geometry::Rect(Rect::new(0.0, 0.0, 2.0, 2.0)),
                composite: Composite::default(),
            }
        );
    }

    #[test]
    fn replay_forwards_recorded_scene_into_wrapped_sink() {
        let mut source = record::Scene::new();
        source.draw(record::Draw::Fill {
            transform: Affine::IDENTITY,
            fill_rule: Fill::NonZero,
            brush: peniko::Brush::Solid(peniko::Color::WHITE),
            brush_transform: None,
            shape: record::Geometry::Rect(Rect::new(0.0, 0.0, 2.0, 3.0)),
            composite: Composite::default(),
        });

        let mut sink = record::Scene::new();
        let mut painter = Painter::new(&mut sink);
        painter.replay(&source);

        assert_eq!(source, sink);
    }

    #[test]
    fn sink_mut_exposes_wrapped_sink() {
        let mut sink = record::Scene::new();
        let mut painter = Painter::new(&mut sink);

        painter.sink_mut().fill(FillRef::new(
            Rect::new(1.0, 2.0, 3.0, 4.0),
            peniko::Color::BLACK,
        ));

        assert_eq!(sink.commands().len(), 1);
    }

    #[test]
    fn fill_accepts_circle_shape_directly() {
        let mut scene = record::Scene::new();
        let mut painter = Painter::new(&mut scene);

        painter
            .fill(Circle::new((5.0, 5.0), 3.0), peniko::Color::BLACK)
            .draw();

        match scene.draw_op(record::DrawId(0)) {
            record::Draw::Fill {
                shape: record::Geometry::Path(_),
                ..
            } => {}
            other => panic!("expected path-backed fill draw, got {other:?}"),
        }
    }

    #[test]
    fn borrowed_rect_shape_stays_rect_backed() {
        let rect = Rect::new(1.0, 2.0, 3.0, 4.0);
        let rect_ref: &Rect = &rect;
        let geometry = <&Rect as PaintShape<'_>>::into_geometry_ref(rect_ref);

        assert_eq!(geometry, GeometryRef::Rect(rect));
    }

    #[test]
    fn borrowed_line_shape_flattens_to_path() {
        let line = Line::new((1.0, 2.0), (3.0, 4.0));
        let line_ref: &Line = &line;
        let geometry = <&Line as PaintShape<'_>>::into_geometry_ref(line_ref);

        match geometry {
            GeometryRef::OwnedPath(_) => {}
            other => panic!("expected path-backed borrowed line shape, got {other:?}"),
        }
    }
}

/// Builder for configuring a fill draw before emission.
#[derive(Debug)]
pub struct FillBuilder<'a, S: ?Sized> {
    sink: &'a mut S,
    draw: FillRef<'a>,
}

impl<'a, S> FillBuilder<'a, S>
where
    S: PaintSink + ?Sized,
{
    /// Set the geometry transform.
    #[must_use]
    pub fn transform(mut self, transform: Affine) -> Self {
        self.draw.transform = transform;
        self
    }

    /// Set the fill rule.
    #[must_use]
    pub fn fill_rule(mut self, fill_rule: peniko::Fill) -> Self {
        self.draw.fill_rule = fill_rule;
        self
    }

    /// Set the optional brush-space transform.
    #[must_use]
    pub fn brush_transform(mut self, brush_transform: Option<Affine>) -> Self {
        self.draw.brush_transform = brush_transform;
        self
    }

    /// Set the per-draw compositing state.
    #[must_use]
    pub fn composite(mut self, composite: Composite) -> Self {
        self.draw.composite = composite;
        self
    }

    /// Emit the fill draw to the wrapped sink.
    pub fn draw(self) {
        self.sink.fill(self.draw);
    }
}

/// Builder for configuring a stroke draw before emission.
#[derive(Debug)]
pub struct StrokeBuilder<'a, S: ?Sized> {
    sink: &'a mut S,
    draw: StrokeRef<'a>,
}

impl<'a, S> StrokeBuilder<'a, S>
where
    S: PaintSink + ?Sized,
{
    /// Set the geometry transform.
    #[must_use]
    pub fn transform(mut self, transform: Affine) -> Self {
        self.draw.transform = transform;
        self
    }

    /// Set the optional brush-space transform.
    #[must_use]
    pub fn brush_transform(mut self, brush_transform: Option<Affine>) -> Self {
        self.draw.brush_transform = brush_transform;
        self
    }

    /// Set the per-draw compositing state.
    #[must_use]
    pub fn composite(mut self, composite: Composite) -> Self {
        self.draw.composite = composite;
        self
    }

    /// Emit the stroke draw to the wrapped sink.
    pub fn draw(self) {
        self.sink.stroke(self.draw);
    }
}

/// Builder for configuring a glyph run before emission.
#[derive(Debug)]
pub struct GlyphRunBuilder<'a, S: ?Sized> {
    sink: &'a mut S,
    font: &'a peniko::FontData,
    transform: Affine,
    glyph_transform: Option<Affine>,
    font_size: f32,
    hint: bool,
    normalized_coords: &'a [NormalizedCoord],
    brush: BrushRef<'a>,
    composite: Composite,
}

impl<'a, S> GlyphRunBuilder<'a, S>
where
    S: PaintSink + ?Sized,
{
    /// Set the global run transform.
    #[must_use]
    pub fn transform(mut self, transform: Affine) -> Self {
        self.transform = transform;
        self
    }

    /// Set the per-glyph transform.
    #[must_use]
    pub fn glyph_transform(mut self, glyph_transform: Option<Affine>) -> Self {
        self.glyph_transform = glyph_transform;
        self
    }

    /// Set the font size in pixels per em.
    #[must_use]
    pub fn font_size(mut self, font_size: f32) -> Self {
        self.font_size = font_size;
        self
    }

    /// Set whether hinting is enabled.
    #[must_use]
    pub fn hint(mut self, hint: bool) -> Self {
        self.hint = hint;
        self
    }

    /// Set normalized variable-font coordinates.
    #[must_use]
    pub fn normalized_coords(mut self, normalized_coords: &'a [NormalizedCoord]) -> Self {
        self.normalized_coords = normalized_coords;
        self
    }

    /// Set the per-draw compositing state.
    #[must_use]
    pub fn composite(mut self, composite: Composite) -> Self {
        self.composite = composite;
        self
    }

    /// Emit the glyph run to the wrapped sink.
    pub fn draw<I, G>(self, style: &'a Style, glyphs: I)
    where
        I: IntoIterator<Item = G>,
        G: Borrow<record::Glyph>,
    {
        let mut glyphs = glyphs.into_iter().map(|glyph| *glyph.borrow());
        self.sink.glyph_run(
            GlyphRunRef {
                font: self.font,
                transform: self.transform,
                glyph_transform: self.glyph_transform,
                font_size: self.font_size,
                hint: self.hint,
                normalized_coords: self.normalized_coords,
                style,
                brush: self.brush,
                composite: self.composite,
            },
            &mut glyphs,
        );
    }
}
