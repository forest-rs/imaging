// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::Error;
use crate::image_registry::ImageBrushResolver;
use imaging::{
    BlurredRoundedRect, ClipRef, Composite, FillRef, GeometryRef, GlyphRunRef, GroupRef, PaintSink,
    StrokeRef,
};
use kurbo::{Affine, Shape as _};
use peniko::{Brush, ImageBrush, Style};
use vello_common::glyph::Glyph as VelloGlyph;

/// Borrowed adapter that streams `imaging` commands into an existing [`vello_hybrid::Scene`].
pub struct VelloHybridSceneSink<'a> {
    scene: &'a mut vello_hybrid::Scene,
    image_resolver: Option<&'a mut dyn ImageBrushResolver>,
    tolerance: f64,
    error: Option<Error>,
    clip_depth: u32,
    group_depth: u32,
}

impl core::fmt::Debug for VelloHybridSceneSink<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VelloHybridSceneSink")
            .field("tolerance", &self.tolerance)
            .field("error", &self.error)
            .field("clip_depth", &self.clip_depth)
            .field("group_depth", &self.group_depth)
            .finish_non_exhaustive()
    }
}

impl<'a> VelloHybridSceneSink<'a> {
    /// Wrap an existing [`vello_hybrid::Scene`].
    pub fn new(scene: &'a mut vello_hybrid::Scene) -> Self {
        Self {
            scene,
            image_resolver: None,
            tolerance: 0.1,
            error: None,
            clip_depth: 0,
            group_depth: 0,
        }
    }

    pub(crate) fn with_image_resolver(
        scene: &'a mut vello_hybrid::Scene,
        image_resolver: &'a mut dyn ImageBrushResolver,
    ) -> Self {
        Self {
            scene,
            image_resolver: Some(image_resolver),
            tolerance: 0.1,
            error: None,
            clip_depth: 0,
            group_depth: 0,
        }
    }

    /// Set the tolerance used when converting rounded rectangles to paths.
    pub fn set_tolerance(&mut self, tolerance: f64) {
        self.tolerance = tolerance;
    }

    /// Return the first deferred translation error, if any, and ensure clip/group stacks are balanced.
    pub fn finish(&mut self) -> Result<(), Error> {
        if let Some(err) = self.error.take() {
            return Err(err);
        }
        if self.clip_depth != 0 {
            return Err(Error::Internal("unbalanced clip stack"));
        }
        if self.group_depth != 0 {
            return Err(Error::Internal("unbalanced group stack"));
        }
        Ok(())
    }

    fn set_error_once(&mut self, err: Error) {
        if self.error.is_none() {
            self.error = Some(err);
        }
    }

    fn brush_to_paint(
        &mut self,
        brush: &Brush,
        composite: Composite,
    ) -> Option<vello_common::paint::PaintType> {
        let brush = brush.clone().multiply_alpha(composite.alpha);
        match brush {
            Brush::Solid(c) => Some(Brush::Solid(c)),
            Brush::Gradient(g) => Some(Brush::Gradient(g)),
            Brush::Image(image) => self.resolve_image_brush(&image).map(Brush::Image),
        }
    }

    fn resolve_image_brush(&mut self, image: &ImageBrush) -> Option<vello_common::paint::Image> {
        let Some(image_resolver) = self.image_resolver.as_deref_mut() else {
            self.set_error_once(Error::UnsupportedImageBrush);
            return None;
        };
        match image_resolver.resolve_image_brush(image) {
            Ok(image) => Some(image),
            Err(err) => {
                self.set_error_once(err);
                None
            }
        }
    }

    fn geometry_to_path(&self, geom: GeometryRef<'_>) -> kurbo::BezPath {
        match geom {
            GeometryRef::Rect(r) => r.to_path(self.tolerance),
            GeometryRef::RoundedRect(rr) => rr.to_path(self.tolerance),
            GeometryRef::Path(p) => p.clone(),
            GeometryRef::OwnedPath(p) => p,
        }
    }

    fn clip_to_path(&mut self, clip: ClipRef<'_>) -> (Affine, kurbo::BezPath, peniko::Fill) {
        match clip {
            ClipRef::Fill {
                transform,
                shape,
                fill_rule,
            } => (transform, self.geometry_to_path(shape), fill_rule),
            ClipRef::Stroke {
                transform,
                shape,
                stroke,
            } => {
                let path = self.geometry_to_path(shape);
                let outline = kurbo::stroke(
                    path.iter(),
                    stroke,
                    &kurbo::StrokeOpts::default(),
                    self.tolerance,
                );
                (transform, outline, peniko::Fill::NonZero)
            }
        }
    }

    fn draw_glyph_run(&mut self, glyph_run: GlyphRunRef<'_>) {
        let Some(paint) = self.brush_to_paint(glyph_run.brush, glyph_run.composite) else {
            return;
        };
        self.scene.set_transform(glyph_run.transform);
        self.scene.set_blend_mode(glyph_run.composite.blend);
        self.scene.set_paint(paint);

        match glyph_run.style {
            Style::Fill(fill_rule) => {
                self.scene.set_fill_rule(*fill_rule);
                let builder = self
                    .scene
                    .glyph_run(glyph_run.font)
                    .font_size(glyph_run.font_size)
                    .hint(glyph_run.hint)
                    .normalized_coords(glyph_run.normalized_coords);
                let builder = if let Some(transform) = glyph_run.glyph_transform {
                    builder.glyph_transform(transform)
                } else {
                    builder
                };
                let glyphs = glyph_run.glyphs.iter().map(|glyph| VelloGlyph {
                    id: glyph.id,
                    x: glyph.x,
                    y: glyph.y,
                });
                builder.fill_glyphs(glyphs);
            }
            Style::Stroke(stroke) => {
                self.scene.set_stroke(stroke.clone());
                let builder = self
                    .scene
                    .glyph_run(glyph_run.font)
                    .font_size(glyph_run.font_size)
                    .hint(glyph_run.hint)
                    .normalized_coords(glyph_run.normalized_coords);
                let builder = if let Some(transform) = glyph_run.glyph_transform {
                    builder.glyph_transform(transform)
                } else {
                    builder
                };
                let glyphs = glyph_run.glyphs.iter().map(|glyph| VelloGlyph {
                    id: glyph.id,
                    x: glyph.x,
                    y: glyph.y,
                });
                builder.stroke_glyphs(glyphs);
            }
        }
    }

    fn draw_blurred_rounded_rect(&mut self, _draw: BlurredRoundedRect) {
        self.set_error_once(Error::UnsupportedBlurredRoundedRect);
    }
}

impl PaintSink for VelloHybridSceneSink<'_> {
    fn push_clip(&mut self, clip: ClipRef<'_>) {
        if self.error.is_some() {
            return;
        }
        let (xf, path, fill_rule) = self.clip_to_path(clip);
        self.scene.set_transform(xf);
        self.scene.set_fill_rule(fill_rule);
        self.scene.push_clip_path(&path);
        self.clip_depth += 1;
    }

    fn pop_clip(&mut self) {
        if self.error.is_some() {
            return;
        }
        if self.clip_depth == 0 {
            self.set_error_once(Error::Internal("pop_clip underflow"));
            return;
        }
        self.scene.pop_clip_path();
        self.clip_depth -= 1;
    }

    fn push_group(&mut self, group: GroupRef<'_>) {
        if self.error.is_some() {
            return;
        }
        if !group.filters.is_empty() {
            self.set_error_once(Error::UnsupportedFilter);
            return;
        }
        let clip_path = group.clip.map(|clip| {
            let (xf, path, fill_rule) = self.clip_to_path(clip);
            self.scene.set_transform(xf);
            self.scene.set_fill_rule(fill_rule);
            path
        });

        let blend = Some(group.composite.blend);
        let opacity = Some(group.composite.alpha);
        self.scene
            .push_layer(clip_path.as_ref(), blend, opacity, None, None);
        self.group_depth += 1;
    }

    fn pop_group(&mut self) {
        if self.error.is_some() {
            return;
        }
        if self.group_depth == 0 {
            self.set_error_once(Error::Internal("pop_group underflow"));
            return;
        }
        self.scene.pop_layer();
        self.group_depth -= 1;
    }

    fn fill(&mut self, draw: FillRef<'_>) {
        if self.error.is_some() {
            return;
        }

        let Some(paint) = self.brush_to_paint(draw.brush, draw.composite) else {
            return;
        };
        self.scene.set_transform(draw.transform);
        self.scene.set_fill_rule(draw.fill_rule);
        self.scene
            .set_paint_transform(draw.brush_transform.unwrap_or(Affine::IDENTITY));

        let (blend, paint) = match (&paint, draw.composite.blend.compose) {
            (Brush::Solid(c), peniko::Compose::Copy) if c.components[3] == 0.0 => (
                peniko::BlendMode::new(peniko::Mix::Normal, peniko::Compose::Clear),
                Brush::Solid(peniko::Color::from_rgba8(0, 0, 0, 255)),
            ),
            _ => (draw.composite.blend, paint),
        };

        self.scene.set_blend_mode(blend);
        self.scene.set_paint(paint);

        match draw.shape {
            GeometryRef::Rect(r) => self.scene.fill_rect(&r),
            GeometryRef::RoundedRect(rr) => {
                let path = rr.to_path(self.tolerance);
                self.scene.fill_path(&path);
            }
            GeometryRef::Path(p) => self.scene.fill_path(p),
            GeometryRef::OwnedPath(p) => self.scene.fill_path(&p),
        }
    }

    fn stroke(&mut self, draw: StrokeRef<'_>) {
        if self.error.is_some() {
            return;
        }

        let Some(paint) = self.brush_to_paint(draw.brush, draw.composite) else {
            return;
        };
        self.scene.set_transform(draw.transform);
        self.scene.set_stroke(draw.stroke.clone());
        self.scene
            .set_paint_transform(draw.brush_transform.unwrap_or(Affine::IDENTITY));

        let (blend, paint) = match (&paint, draw.composite.blend.compose) {
            (Brush::Solid(c), peniko::Compose::Copy) if c.components[3] == 0.0 => (
                peniko::BlendMode::new(peniko::Mix::Normal, peniko::Compose::Clear),
                Brush::Solid(peniko::Color::from_rgba8(0, 0, 0, 255)),
            ),
            _ => (draw.composite.blend, paint),
        };

        self.scene.set_blend_mode(blend);
        self.scene.set_paint(paint);

        match draw.shape {
            GeometryRef::Rect(r) => self.scene.stroke_rect(&r),
            GeometryRef::RoundedRect(rr) => {
                let path = rr.to_path(self.tolerance);
                self.scene.stroke_path(&path);
            }
            GeometryRef::Path(p) => self.scene.stroke_path(p),
            GeometryRef::OwnedPath(p) => self.scene.stroke_path(&p),
        }
    }

    fn glyph_run(&mut self, draw: GlyphRunRef<'_>) {
        if self.error.is_some() {
            return;
        }
        self.draw_glyph_run(draw);
    }

    fn blurred_rounded_rect(&mut self, draw: BlurredRoundedRect) {
        if self.error.is_some() {
            return;
        }
        self.draw_blurred_rounded_rect(draw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imaging::Filter;
    use peniko::{Blob, ImageAlphaType, ImageData, ImageFormat};
    use std::sync::Arc;

    #[test]
    fn hybrid_scene_sink_reports_clip_underflow() {
        let mut scene = vello_hybrid::Scene::new(32, 32);
        scene.reset();
        let mut sink = VelloHybridSceneSink::new(&mut scene);
        sink.pop_clip();
        assert!(matches!(
            sink.finish(),
            Err(Error::Internal("pop_clip underflow"))
        ));
    }

    #[test]
    fn hybrid_scene_sink_rejects_filters() {
        let mut scene = vello_hybrid::Scene::new(32, 32);
        scene.reset();
        let mut sink = VelloHybridSceneSink::new(&mut scene);
        sink.push_group(GroupRef::new().with_filters(&[Filter::blur(2.0)]));
        assert!(matches!(sink.finish(), Err(Error::UnsupportedFilter)));
    }

    #[test]
    fn hybrid_scene_sink_rejects_image_brushes_without_resolver() {
        let mut scene = vello_hybrid::Scene::new(32, 32);
        scene.reset();
        let mut sink = VelloHybridSceneSink::new(&mut scene);
        let image = Brush::Image(ImageBrush::new(ImageData {
            data: Blob::new(Arc::new([255_u8; 16])),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: 2,
            height: 2,
        }));
        sink.fill(FillRef::new(kurbo::Rect::new(0.0, 0.0, 8.0, 8.0), &image));
        assert!(matches!(sink.finish(), Err(Error::UnsupportedImageBrush)));
    }
}
