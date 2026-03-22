// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Painter-style authoring helpers built on top of [`PaintSink`].

use core::borrow::Borrow;

use kurbo::{Affine, Rect, Stroke};
use peniko::{BrushRef, Style};

use crate::{
    BlurredRoundedRect, ClipRef, Composite, FillRef, GeometryRef, GlyphRunRef, GroupRef,
    NormalizedCoord, PaintSink, StrokeRef, record::Glyph,
};

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

    /// Start configuring a fill draw.
    pub fn fill<'b>(
        &'b mut self,
        shape: impl Into<GeometryRef<'b>>,
        brush: impl Into<BrushRef<'b>>,
    ) -> FillBuilder<'b, S> {
        FillBuilder {
            sink: self.sink,
            draw: FillRef::new(shape, brush),
        }
    }

    /// Fill a rectangle using default transform, fill rule, brush transform, and composite state.
    pub fn fill_rect<'b>(&mut self, rect: Rect, brush: impl Into<BrushRef<'b>>) {
        self.sink.fill(FillRef::new(rect, brush));
    }

    /// Start configuring a stroke draw.
    pub fn stroke<'b>(
        &'b mut self,
        shape: impl Into<GeometryRef<'b>>,
        stroke: &'b Stroke,
        brush: impl Into<BrushRef<'b>>,
    ) -> StrokeBuilder<'b, S> {
        StrokeBuilder {
            sink: self.sink,
            draw: StrokeRef::new(shape, stroke, brush),
        }
    }

    /// Start configuring a glyph run.
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
    pub fn with_fill_clip<'b>(
        &'b mut self,
        shape: impl Into<GeometryRef<'b>>,
        f: impl FnOnce(&mut Painter<'_, S>),
    ) {
        self.with_clip(ClipRef::fill(shape), f);
    }

    /// Push a fill-style clip with an explicit transform, run the provided closure, then pop the
    /// clip.
    pub fn with_fill_clip_transformed<'b>(
        &'b mut self,
        shape: impl Into<GeometryRef<'b>>,
        transform: Affine,
        f: impl FnOnce(&mut Painter<'_, S>),
    ) {
        self.with_clip(ClipRef::fill(shape).with_transform(transform), f);
    }

    /// Push a stroke-style clip, run the provided closure, then pop the clip.
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
    use alloc::vec;
    use alloc::vec::Vec;
    use kurbo::{Point, Shape as _, Vec2};
    use peniko::Fill;

    use crate::{BlurredRoundedRect, GroupRef};

    #[derive(Default)]
    struct RecordingSink {
        pushed_clips: Vec<crate::record::Clip>,
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

        fn glyph_run(&mut self, _draw: GlyphRunRef<'_>, _glyphs: &mut dyn Iterator<Item = Glyph>) {}

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
            vec![crate::record::Clip::Fill {
                transform,
                shape: crate::record::Geometry::Rect(Rect::new(0.0, 0.0, 10.0, 12.0)),
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
        let line = kurbo::Line::new(Point::new(1.0, 2.0), Point::new(11.0, 15.0));
        let path = line.to_path(0.1);

        painter.with_stroke_clip_transformed(path.clone(), &stroke, transform, |_| {});

        assert_eq!(
            sink.pushed_clips,
            vec![crate::record::Clip::Stroke {
                transform,
                shape: crate::record::Geometry::Path(path),
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
        G: Borrow<Glyph>,
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
