// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Painter-style authoring helpers built on top of [`PaintSink`].

use core::borrow::Borrow;

use kurbo::{Affine, BezPath, CubicBez, Line, QuadBez, Rect, RoundedRect, Stroke, Vec2};
use peniko::{BrushRef, ImageBrushRef, Style};

use crate::{
    BlurredRoundedRect, ClipRef, Composite, ContextRef, FillRef, GeometryRef, GlyphRunRef,
    GroupRef, MaskMode, NormalizedCoord, PaintSink, SourceLocationRef, StrokeRef, record,
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

    /// Reborrow this painter as a trait-object-backed [`Painter`].
    ///
    /// This is useful when generic code needs to call helpers that are written against
    /// `Painter<'_, dyn PaintSink>` without threading the concrete sink type through the
    /// surrounding API.
    #[must_use]
    pub fn as_dyn(&mut self) -> Painter<'_, dyn PaintSink + '_>
    where
        S: Sized,
    {
        let sink: &mut dyn PaintSink = self.sink_mut();
        Painter::new(sink)
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
            font_embolden: Vec2::ZERO,
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

    /// Push a context annotation onto the context stack.
    ///
    /// This is recorded by sinks that support retained context metadata and ignored by sinks that
    /// do not. Pair with [`Self::pop_context`] or prefer [`Self::with_context`] /
    /// [`crate::with_context!`] when the annotated work fits naturally in a closure.
    pub fn push_context(&mut self, label: &str, source: Option<SourceLocationRef<'_>>) {
        self.sink.push_context(ContextRef::new(label, source));
    }

    /// Pop the most recently pushed context annotation.
    pub fn pop_context(&mut self) {
        self.sink.pop_context();
    }

    /// Push a context annotation, run the provided closure, then pop the context.
    ///
    /// Use this when you want retained scenes and validation errors to preserve a human-meaningful
    /// scope label, optionally with a source location.
    pub fn with_context(
        &mut self,
        label: &str,
        source: Option<SourceLocationRef<'_>>,
        f: impl FnOnce(&mut Painter<'_, S>),
    ) {
        self.push_context(label, source);
        f(self);
        self.pop_context();
    }

    /// Push a clip onto the non-isolated clip stack.
    ///
    /// This must be matched by a later [`Self::pop_clip`]. Prefer [`Self::with_clip`] when the
    /// clipped work fits naturally in a closure.
    pub fn push_clip(&mut self, clip: ClipRef<'_>) {
        self.sink.push_clip(clip);
    }

    /// Pop the most recently pushed non-isolated clip.
    ///
    /// Every [`Self::push_clip`] or `push_*_clip` call must be paired with exactly one
    /// `pop_clip`.
    pub fn pop_clip(&mut self) {
        self.sink.pop_clip();
    }

    /// Push a fill-style clip onto the non-isolated clip stack.
    ///
    /// Defaults:
    /// - clip transform: [`Affine::IDENTITY`]
    /// - fill rule: [`peniko::Fill::NonZero`]
    ///
    /// This must be matched by a later [`Self::pop_clip`]. Prefer [`Self::with_fill_clip`] when
    /// possible.
    pub fn push_fill_clip<'b>(&mut self, shape: impl Into<GeometryRef<'b>>) {
        self.push_clip(ClipRef::fill(shape));
    }

    /// Push a fill-style clip with an explicit transform onto the non-isolated clip stack.
    ///
    /// The clip still uses [`peniko::Fill::NonZero`] unless replaced through [`ClipRef`].
    ///
    /// This must be matched by a later [`Self::pop_clip`]. Prefer
    /// [`Self::with_fill_clip_transformed`] when possible.
    pub fn push_fill_clip_transformed<'b>(
        &mut self,
        shape: impl Into<GeometryRef<'b>>,
        transform: Affine,
    ) {
        self.push_clip(ClipRef::fill(shape).with_transform(transform));
    }

    /// Push a stroke-style clip onto the non-isolated clip stack.
    ///
    /// Default:
    /// - clip transform: [`Affine::IDENTITY`]
    ///
    /// This must be matched by a later [`Self::pop_clip`]. Prefer [`Self::with_stroke_clip`]
    /// when possible.
    pub fn push_stroke_clip<'b>(&mut self, shape: impl Into<GeometryRef<'b>>, stroke: &'b Stroke) {
        self.push_clip(ClipRef::stroke(shape, stroke));
    }

    /// Push a stroke-style clip with an explicit transform onto the non-isolated clip stack.
    ///
    /// This must be matched by a later [`Self::pop_clip`]. Prefer
    /// [`Self::with_stroke_clip_transformed`] when possible.
    pub fn push_stroke_clip_transformed<'b>(
        &mut self,
        shape: impl Into<GeometryRef<'b>>,
        stroke: &'b Stroke,
        transform: Affine,
    ) {
        self.push_clip(ClipRef::stroke(shape, stroke).with_transform(transform));
    }

    /// Push a clip, run the provided closure, then pop the clip.
    pub fn with_clip(&mut self, clip: ClipRef<'_>, f: impl FnOnce(&mut Painter<'_, S>)) {
        self.push_clip(clip);
        {
            let mut painter = Painter::new(self.sink);
            f(&mut painter);
        }
        self.pop_clip();
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
        self.push_fill_clip(shape);
        {
            let mut painter = Painter::new(self.sink);
            f(&mut painter);
        }
        self.pop_clip();
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
        self.push_fill_clip_transformed(shape, transform);
        {
            let mut painter = Painter::new(self.sink);
            f(&mut painter);
        }
        self.pop_clip();
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
        self.push_stroke_clip(shape, stroke);
        {
            let mut painter = Painter::new(self.sink);
            f(&mut painter);
        }
        self.pop_clip();
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
        self.push_stroke_clip_transformed(shape, stroke, transform);
        {
            let mut painter = Painter::new(self.sink);
            f(&mut painter);
        }
        self.pop_clip();
    }

    /// Push an isolated group onto the group stack.
    ///
    /// This must be matched by a later [`Self::pop_group`]. Prefer [`Self::with_group`] when the
    /// grouped work fits naturally in a closure.
    pub fn push_group(&mut self, group: GroupRef<'_>) {
        self.sink.push_group(group);
    }

    /// Pop the most recently pushed isolated group.
    ///
    /// Every [`Self::push_group`] call must be paired with exactly one `pop_group`.
    pub fn pop_group(&mut self) {
        self.sink.pop_group();
    }

    /// Push an isolated group, run the provided closure, then pop the group.
    pub fn with_group(&mut self, group: GroupRef<'_>, f: impl FnOnce(&mut Painter<'_, S>)) {
        self.push_group(group);
        {
            let mut painter = Painter::new(self.sink);
            f(&mut painter);
        }
        self.pop_group();
    }

    /// Record a reusable retained mask definition.
    ///
    /// Prefer this when the same mask will be applied more than once. The returned
    /// [`record::Mask`] can be reused through [`GroupRef::with_mask`] or
    /// [`GroupRef::with_mask_transformed`]. `mode` controls whether the recorded mask scene is
    /// interpreted as alpha or luminance.
    #[must_use]
    pub fn record_mask(
        mode: MaskMode,
        mask: impl FnOnce(&mut Painter<'_, record::Scene>),
    ) -> record::Mask {
        let mut mask_scene = record::Scene::new();
        {
            let mut painter = Painter::new(&mut mask_scene);
            mask(&mut painter);
        }
        record::Mask::new(mode, mask_scene)
    }

    /// Record a temporary one-off mask definition, then paint content through a masked isolated
    /// group.
    ///
    /// Prefer [`Self::record_mask`] plus [`Self::with_group`] when the same mask will be reused.
    pub fn with_masked_group(
        &mut self,
        mode: MaskMode,
        mask: impl FnOnce(&mut Painter<'_, record::Scene>),
        content: impl FnOnce(&mut Painter<'_, S>),
    ) {
        let mask = Self::record_mask(mode, mask);
        self.with_group(GroupRef::new().with_mask(mask.as_ref()), content);
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

        painter.push_fill_clip_transformed(Rect::new(0.0, 0.0, 10.0, 12.0), transform);
        painter.pop_clip();

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

        painter.push_stroke_clip_transformed(path.clone(), &stroke, transform);
        painter.pop_clip();

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
    fn as_dyn_reborrows_and_returns_control_to_original_painter() {
        let mut sink = record::Scene::new();
        let mut painter = Painter::new(&mut sink);

        painter
            .as_dyn()
            .fill(Rect::new(1.0, 2.0, 3.0, 4.0), peniko::Color::BLACK)
            .draw();
        painter
            .fill(Rect::new(5.0, 6.0, 7.0, 8.0), peniko::Color::WHITE)
            .draw();

        assert_eq!(sink.commands().len(), 2);
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

    #[test]
    fn with_fill_clip_still_pushes_and_pops() {
        let mut sink = RecordingSink::default();
        let mut painter = Painter::new(&mut sink);

        painter.with_fill_clip(Rect::new(0.0, 0.0, 5.0, 6.0), |_| {});

        assert_eq!(sink.pushed_clips.len(), 1);
    }

    #[test]
    fn with_context_records_balanced_context_commands() {
        let mut scene = record::Scene::new();
        let mut painter = Painter::new(&mut scene);

        painter.with_context(
            "toolbar/button",
            Some(SourceLocationRef::new("widgets.rs", 7, 3)),
            |p| {
                p.fill(Rect::new(0.0, 0.0, 5.0, 6.0), peniko::Color::BLACK)
                    .draw();
            },
        );

        assert_eq!(
            scene.commands(),
            &[
                record::Command::PushContext(record::ContextId(0)),
                record::Command::Draw(record::DrawId(0)),
                record::Command::PopContext,
            ]
        );
        let context = scene.context(record::ContextId(0));
        assert_eq!(scene.label(context.label), "toolbar/button");
        let source = context.source.as_ref().expect("expected source location");
        assert_eq!(scene.file(source.file), "widgets.rs");
        assert_eq!(source.line, 7);
        assert_eq!(source.column, 3);
    }

    #[test]
    fn with_masked_group_records_reusable_mask_and_group_content() {
        let mut scene = record::Scene::new();
        let mut painter = Painter::new(&mut scene);

        painter.with_masked_group(
            MaskMode::Alpha,
            |mask| {
                mask.fill(Rect::new(0.0, 0.0, 8.0, 8.0), peniko::Color::WHITE)
                    .draw();
            },
            |content| {
                content
                    .fill(Rect::new(1.0, 1.0, 7.0, 7.0), peniko::Color::BLACK)
                    .draw();
            },
        );

        assert_eq!(
            scene.commands(),
            &[
                record::Command::PushGroup(record::GroupId(0)),
                record::Command::Draw(record::DrawId(0)),
                record::Command::PopGroup,
            ]
        );
        assert_eq!(scene.mask(record::MaskId(0)).scene.commands().len(), 1);
        let group = scene.group(record::GroupId(0));
        let mask = group.mask.as_ref().expect("expected group mask");
        assert_eq!(scene.mask(mask.mask).mode, MaskMode::Alpha);
    }

    #[test]
    fn record_mask_returns_reusable_mask_definition() {
        let mask = Painter::<record::Scene>::record_mask(MaskMode::Luminance, |mask| {
            mask.fill(Rect::new(0.0, 0.0, 8.0, 8.0), peniko::Color::WHITE)
                .draw();
        });

        assert_eq!(mask.scene.commands().len(), 1);
        assert_eq!(mask.mode, MaskMode::Luminance);
    }
}

/// Builder for configuring a fill draw before emission.
#[derive(Debug)]
#[must_use = "fill builders do nothing until you call .draw()"]
pub struct FillBuilder<'a, S: ?Sized> {
    sink: &'a mut S,
    draw: FillRef<'a>,
}

impl<'a, S> FillBuilder<'a, S>
where
    S: PaintSink + ?Sized,
{
    /// Set the geometry transform.
    pub fn transform(mut self, transform: Affine) -> Self {
        self.draw.transform = transform;
        self
    }

    /// Set the fill rule.
    pub fn fill_rule(mut self, fill_rule: peniko::Fill) -> Self {
        self.draw.fill_rule = fill_rule;
        self
    }

    /// Set the optional brush-space transform.
    pub fn brush_transform(mut self, brush_transform: Option<Affine>) -> Self {
        self.draw.brush_transform = brush_transform;
        self
    }

    /// Set the per-draw compositing state.
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
#[must_use = "stroke builders do nothing until you call .draw()"]
pub struct StrokeBuilder<'a, S: ?Sized> {
    sink: &'a mut S,
    draw: StrokeRef<'a>,
}

impl<'a, S> StrokeBuilder<'a, S>
where
    S: PaintSink + ?Sized,
{
    /// Set the geometry transform.
    pub fn transform(mut self, transform: Affine) -> Self {
        self.draw.transform = transform;
        self
    }

    /// Set the optional brush-space transform.
    pub fn brush_transform(mut self, brush_transform: Option<Affine>) -> Self {
        self.draw.brush_transform = brush_transform;
        self
    }

    /// Set the per-draw compositing state.
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
#[must_use = "glyph builders do nothing until you call .draw(...)"]
pub struct GlyphRunBuilder<'a, S: ?Sized> {
    sink: &'a mut S,
    font: &'a peniko::FontData,
    transform: Affine,
    glyph_transform: Option<Affine>,
    font_size: f32,
    font_embolden: Vec2,
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
    pub fn transform(mut self, transform: Affine) -> Self {
        self.transform = transform;
        self
    }

    /// Set the per-glyph transform.
    pub fn glyph_transform(mut self, glyph_transform: Option<Affine>) -> Self {
        self.glyph_transform = glyph_transform;
        self
    }

    /// Set the font size in pixels per em.
    pub fn font_size(mut self, font_size: f32) -> Self {
        self.font_size = font_size;
        self
    }

    /// Set the synthetic embolden amount applied to glyph outlines.
    pub fn font_embolden(mut self, font_embolden: Vec2) -> Self {
        self.font_embolden = font_embolden;
        self
    }

    /// Set whether hinting is enabled.
    pub fn hint(mut self, hint: bool) -> Self {
        self.hint = hint;
        self
    }

    /// Set normalized variable-font coordinates.
    pub fn normalized_coords(mut self, normalized_coords: &'a [NormalizedCoord]) -> Self {
        self.normalized_coords = normalized_coords;
        self
    }

    /// Set the per-draw compositing state.
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
                font_embolden: self.font_embolden,
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
