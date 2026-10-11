// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::Error;
use crate::{
    VelloHybridRenderer, VelloHybridScene,
    image_registry::{HybridImageUploadSession, RegisteredImage},
};
use glifo::Glyph as VelloGlyph;
use imaging::{
    BlurredRoundedRect, ClipRef, Composite, FillRef, GeometryRef, GlyphRunRef, GroupRef, MaskMode,
    PaintSink, StrokeRef,
    record::{Scene, replay_transformed},
};
use kurbo::{Affine, Shape as _};
use peniko::{Brush, ImageBrush, Style};
use std::sync::Arc;

/// Borrowed adapter that streams `imaging` commands into an existing [`VelloHybridScene`].
pub struct VelloHybridSceneSink<'a> {
    scene: &'a mut vello_gpu::Scene,
    external_images: &'a mut Vec<Arc<crate::ExternalTexture>>,
    external_bindings:
        Option<&'a std::collections::HashMap<imaging::ExternalImage, Arc<crate::ExternalTexture>>>,
    image_leases: &'a mut Vec<Arc<RegisteredImage>>,
    scene_renderer: &'a mut Option<Arc<()>>,
    finished: bool,
    resources: SceneSinkResources<'a>,
    image_upload: Option<HybridImageUploadSession<'a>>,
    tolerance: f64,
    error: Option<Error>,
    layer_stack: Vec<LayerFrame>,
}

#[derive(Debug)]
struct PendingMask {
    scene: Scene,
    transform: Affine,
}

#[derive(Debug)]
enum LayerFrame {
    Clip,
    Group { mask: Option<Box<PendingMask>> },
}

enum SceneSinkResources<'a> {
    Owned(Box<glifo::GlyphPrepCache>),
    Borrowed(&'a mut vello_gpu::Resources),
}

// A standalone sink records glyph outlines without allocating renderer-owned atlas resources.
struct UncachedGlyphBackend<'a> {
    scene: &'a mut vello_gpu::Scene,
    cache: &'a mut glifo::GlyphPrepCache,
}

impl<'a> glifo::GlyphRunBackend<'a> for UncachedGlyphBackend<'a> {
    fn atlas_cache(self, _enabled: bool) -> Self {
        self
    }

    fn fill_glyphs<G>(
        self,
        run: glifo::GlyphRun<'a>,
        glyphs: G,
    ) -> Result<(), glifo::GlyphRenderError>
    where
        G: Iterator<Item = VelloGlyph> + Clone,
    {
        run.build(glyphs, self.cache.as_mut(), glifo::AtlasCacher::Disabled)
            .fill_glyphs(self.scene)
    }

    fn stroke_glyphs<G>(
        self,
        run: glifo::GlyphRun<'a>,
        glyphs: G,
    ) -> Result<(), glifo::GlyphRenderError>
    where
        G: Iterator<Item = VelloGlyph> + Clone,
    {
        let mut run = run.build(glyphs, self.cache.as_mut(), glifo::AtlasCacher::Disabled);
        let stroke = self.scene.stroke().clone();
        let mut adjusted = stroke.clone();
        adjusted.width *= run.stroke_adjustment();
        self.scene.set_stroke(adjusted);
        let result = run.stroke_glyphs(self.scene);
        self.scene.set_stroke(stroke);
        result
    }

    fn render_decoration<G>(
        self,
        run: glifo::GlyphRun<'a>,
        glyphs: G,
        x_range: core::ops::RangeInclusive<f32>,
        baseline_y: f32,
        offset: f32,
        size: f32,
        buffer: f32,
    ) where
        G: Iterator<Item = VelloGlyph> + Clone,
    {
        run.build(glyphs, self.cache.as_mut(), glifo::AtlasCacher::Disabled)
            .render_decoration(x_range, baseline_y, offset, size, buffer, self.scene);
    }
}

impl Drop for VelloHybridSceneSink<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.scene.reset();
            self.image_leases.clear();
            self.external_images.clear();
            *self.scene_renderer = None;
            if let Some(mut upload) = self.image_upload.take()
                && let SceneSinkResources::Borrowed(resources) = &mut self.resources
            {
                upload.finish(resources, false);
            }
        }
    }
}

impl core::fmt::Debug for VelloHybridSceneSink<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VelloHybridSceneSink")
            .field("tolerance", &self.tolerance)
            .field("error", &self.error)
            .field("layer_stack_depth", &self.layer_stack.len())
            .finish_non_exhaustive()
    }
}

impl<'a> VelloHybridSceneSink<'a> {
    /// Wrap an existing [`VelloHybridScene`].
    pub fn new(scene: &'a mut VelloHybridScene) -> Self {
        Self {
            scene: &mut scene.native,
            external_images: &mut scene.external_images,
            external_bindings: None,
            image_leases: &mut scene.images,
            scene_renderer: &mut scene.renderer,
            finished: false,
            resources: SceneSinkResources::Owned(Box::default()),
            image_upload: None,
            tolerance: 0.1,
            error: None,
            layer_stack: Vec::new(),
        }
    }

    /// Wrap an existing [`VelloHybridScene`] and use `renderer` to upload image brushes on
    /// demand.
    ///
    /// This is the native-scene path for image brushes. Uploaded images are cached on the
    /// renderer and reused across subsequent recordings and renders.
    pub fn with_renderer(
        scene: &'a mut VelloHybridScene,
        renderer: &'a mut VelloHybridRenderer,
    ) -> Self {
        if let Some(identity) = &scene.renderer
            && !Arc::ptr_eq(identity, &renderer.state.identity)
        {
            let mut sink = Self::new(scene);
            sink.error = Some(Error::WrongRenderer);
            return sink;
        }
        scene.renderer = Some(Arc::clone(&renderer.state.identity));
        let state = &mut renderer.state;
        let encoder = state
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("imaging_vello_hybrid scene upload images"),
            });
        let image_upload = state.image_registry.begin_upload_session(
            &mut state.resources,
            &mut state.renderer,
            &state.device,
            &state.queue,
            encoder,
        );
        Self {
            scene: &mut scene.native,
            external_images: &mut scene.external_images,
            external_bindings: Some(&state.external_images),
            image_leases: &mut scene.images,
            scene_renderer: &mut scene.renderer,
            finished: false,
            resources: SceneSinkResources::Borrowed(&mut state.resources),
            image_upload: Some(image_upload),
            tolerance: 0.1,
            error: None,
            layer_stack: Vec::new(),
        }
    }

    /// Set the tolerance used when converting rounded rectangles to paths.
    pub fn set_tolerance(&mut self, tolerance: f64) {
        self.tolerance = tolerance;
    }

    /// Complete recording and report translation or unbalanced-stack errors.
    ///
    /// A failed recording clears the scene and releases its image references. Dropping the
    /// sink without calling this method also discards the recording.
    pub fn finish(mut self) -> Result<(), Error> {
        let result = if let Some(err) = self.error.take() {
            Err(err)
        } else if !self.layer_stack.is_empty() {
            Err(Error::Internal("unbalanced layer stack"))
        } else {
            Ok(())
        };

        if result.is_err() {
            self.scene.reset();
            self.image_leases.clear();
            self.external_images.clear();
            *self.scene_renderer = None;
        }
        self.finished = true;
        if let Some(mut image_upload) = self.image_upload.take()
            && let SceneSinkResources::Borrowed(resources) = &mut self.resources
        {
            image_upload.finish(resources, result.is_ok());
        }

        result
    }

    fn set_error_once(&mut self, err: Error) {
        if self.error.is_none() {
            self.error = Some(err);
        }
    }

    fn brush_to_paint(
        &mut self,
        brush: imaging::BrushRef<'_>,
        composite: Composite,
    ) -> Option<vello_common::paint::PaintType> {
        if let imaging::BrushRef::Image(imaging::ImageBrushRef {
            image: imaging::ImageRef::External(image),
            mut sampler,
        }) = brush
        {
            let Some(binding) = self
                .external_bindings
                .and_then(|bindings| bindings.get(image))
            else {
                self.set_error_once(Error::UnboundImage(image.clone()));
                return None;
            };
            let index = self
                .external_images
                .iter()
                .position(|held| Arc::ptr_eq(held, binding))
                .unwrap_or_else(|| {
                    self.external_images.push(Arc::clone(binding));
                    self.external_images.len() - 1
                });
            sampler.alpha *= composite.alpha;
            let source = vello_common::paint::ImageSource::external_texture(
                vello_common::TextureId(index as u64),
                vello_common::geometry::RectU16::new(
                    0,
                    0,
                    u16::try_from(image.width()).expect("binding dimensions were checked"),
                    u16::try_from(image.height()).expect("binding dimensions were checked"),
                ),
                true,
            );
            return Some(Brush::Image(vello_common::paint::Image {
                image: source,
                sampler,
            }));
        }
        let brush = brush
            .as_raster()
            .expect("external images were resolved above");
        let brush = brush.to_owned().multiply_alpha(composite.alpha);
        match brush {
            Brush::Solid(c) => Some(Brush::Solid(c)),
            Brush::Gradient(g) => Some(Brush::Gradient(g)),
            Brush::Image(image) => self.resolve_image_brush(&image).map(Brush::Image),
        }
    }

    fn resolve_image_brush(&mut self, image: &ImageBrush) -> Option<vello_common::paint::Image> {
        let Some(image_upload) = self.image_upload.as_mut() else {
            self.set_error_once(Error::UnsupportedImageBrush);
            return None;
        };
        match image_upload.resolve_image_brush(
            match &mut self.resources {
                SceneSinkResources::Borrowed(resources) => resources,
                SceneSinkResources::Owned(_) => {
                    unreachable!("image upload requires renderer resources")
                }
            },
            image,
        ) {
            Ok((image, lease)) => {
                if !self
                    .image_leases
                    .iter()
                    .any(|held| Arc::ptr_eq(held, &lease))
                {
                    self.image_leases.push(lease);
                }
                Some(image)
            }
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

    fn draw_glyph_run(
        &mut self,
        glyph_run: GlyphRunRef<'_>,
        glyphs: &mut dyn Iterator<Item = imaging::record::Glyph>,
    ) {
        if glyph_run.composite.blend.is_destructive() {
            self.set_error_once(Error::UnsupportedGlyphBlend);
            return;
        }
        let Some(paint) = self.brush_to_paint(glyph_run.brush, glyph_run.composite) else {
            return;
        };
        self.scene.set_transform(glyph_run.transform);
        self.scene
            .set_paint_transform(glyph_run.brush_transform.unwrap_or(Affine::IDENTITY));
        self.scene.set_blend_mode(glyph_run.composite.blend);
        self.scene.set_paint(paint);
        // TODO: Revisit this allocation. `PaintSink` currently gives us a
        // one-shot iterator, while glifo's glyph builders need a cloneable
        // glyph source.
        let glyphs = glyphs
            .map(|glyph| VelloGlyph {
                id: glyph.id,
                x: glyph.x,
                y: glyph.y,
            })
            .collect::<Vec<_>>();

        match glyph_run.style {
            Style::Fill(fill) => self.scene.set_fill_rule(*fill),
            Style::Stroke(stroke) => self.scene.set_stroke(stroke.clone()),
        }
        let result = match &mut self.resources {
            SceneSinkResources::Borrowed(resources) => {
                let builder = self
                    .scene
                    .glyph_run(resources, glyph_run.font)
                    .font_size(glyph_run.font_size)
                    .hint(glyph_run.hint)
                    .normalized_coords(glyph_run.normalized_coords);
                let builder = if let Some(transform) = glyph_run.glyph_transform {
                    builder.glyph_transform(transform)
                } else {
                    builder
                };
                match glyph_run.style {
                    Style::Fill(_) => builder.fill_glyphs(glyphs.into_iter()),
                    Style::Stroke(_) => builder.stroke_glyphs(glyphs.into_iter()),
                }
            }
            SceneSinkResources::Owned(cache) => {
                let builder = glifo::GlyphRunBuilder::new(
                    glyph_run.font.clone(),
                    glyph_run.transform,
                    glyph_run.brush_transform.unwrap_or(Affine::IDENTITY),
                    UncachedGlyphBackend {
                        scene: self.scene,
                        cache,
                    },
                )
                .font_size(glyph_run.font_size)
                .hint(glyph_run.hint)
                .normalized_coords(glyph_run.normalized_coords);
                let builder = if let Some(transform) = glyph_run.glyph_transform {
                    builder.glyph_transform(transform)
                } else {
                    builder
                };
                match glyph_run.style {
                    Style::Fill(_) => builder.fill_glyphs(glyphs.into_iter()),
                    Style::Stroke(_) => builder.stroke_glyphs(glyphs.into_iter()),
                }
            }
        };
        if result.is_err() {
            self.set_error_once(Error::Internal("glyph rendering failed"));
        }
    }

    fn draw_blurred_rounded_rect(&mut self, _draw: BlurredRoundedRect) {
        self.set_error_once(Error::UnsupportedBlurredRoundedRect);
    }

    fn fill_destructive(
        &mut self,
        clip: ClipRef<'_>,
        blend: peniko::BlendMode,
        paint: vello_common::paint::PaintType,
        paint_transform: Affine,
    ) {
        let (transform, path, fill_rule) = self.clip_to_path(clip);
        self.scene.set_transform(transform);
        self.scene.set_fill_rule(fill_rule);
        self.scene
            .push_layer(Some(&path), Some(blend), None, None, None);
        self.scene.set_blend_mode(peniko::BlendMode::default());
        self.scene.set_transform(Affine::IDENTITY);
        self.scene.set_paint_transform(paint_transform);
        self.scene.set_paint(paint);
        // The layer clip supplies shape coverage. Painting the whole layer avoids applying
        // antialiasing twice and keeps destructive composition confined to the draw geometry.
        self.scene.fill_rect(&kurbo::Rect::new(
            0.0,
            0.0,
            f64::from(self.scene.width()),
            f64::from(self.scene.height()),
        ));
        self.scene.pop_layer();
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
        self.layer_stack.push(LayerFrame::Clip);
    }

    fn pop_clip(&mut self) {
        if self.error.is_some() {
            return;
        }
        if !matches!(self.layer_stack.pop(), Some(LayerFrame::Clip)) {
            self.set_error_once(Error::Internal("unbalanced clip stack"));
            return;
        }
        self.scene.pop_clip();
    }

    fn push_group(&mut self, group: GroupRef<'_>) {
        if self.error.is_some() {
            return;
        }
        if group
            .mask
            .as_ref()
            .is_some_and(|mask| mask.mask.mode != MaskMode::Alpha)
        {
            self.set_error_once(Error::UnsupportedMask);
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
        self.layer_stack.push(LayerFrame::Group {
            mask: group.mask.map(|mask| {
                Box::new(PendingMask {
                    scene: mask.mask.scene.clone(),
                    transform: mask.transform,
                })
            }),
        });
    }

    fn pop_group(&mut self) {
        if self.error.is_some() {
            return;
        }
        let Some(LayerFrame::Group { mask }) = self.layer_stack.pop() else {
            self.set_error_once(Error::Internal("unbalanced group stack"));
            return;
        };
        if let Some(mask) = mask {
            // Keep the mask's original paints: only the alpha of the fully composited mask
            // participates in DestIn. Converting individual draws changes blend semantics.
            self.scene.set_transform(Affine::IDENTITY);
            self.scene.push_blend_layer(peniko::Compose::DestIn.into());
            replay_transformed(&mask.scene, self, mask.transform);
            self.scene.pop_layer();
        }
        self.scene.pop_layer();
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

        if blend.is_destructive() {
            self.fill_destructive(
                ClipRef::Fill {
                    transform: draw.transform,
                    shape: draw.shape,
                    fill_rule: draw.fill_rule,
                },
                blend,
                paint,
                draw.transform * draw.brush_transform.unwrap_or(Affine::IDENTITY),
            );
            return;
        }
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

        if blend.is_destructive() {
            self.fill_destructive(
                ClipRef::Stroke {
                    transform: draw.transform,
                    shape: draw.shape,
                    stroke: draw.stroke,
                },
                blend,
                paint,
                draw.transform * draw.brush_transform.unwrap_or(Affine::IDENTITY),
            );
            return;
        }
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

    fn glyph_run(
        &mut self,
        draw: GlyphRunRef<'_>,
        glyphs: &mut dyn Iterator<Item = imaging::record::Glyph>,
    ) {
        if self.error.is_some() {
            return;
        }
        self.draw_glyph_run(draw, glyphs);
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
    use imaging::{Filter, MaskMode, MaskRef, record};
    use peniko::{Blob, ImageAlphaType, ImageData, ImageFormat};
    use std::sync::Arc;

    #[test]
    fn hybrid_scene_sink_reports_clip_underflow() {
        let mut scene = VelloHybridScene::new(32, 32);
        scene.reset();
        let mut sink = VelloHybridSceneSink::new(&mut scene);
        sink.pop_clip();
        assert!(matches!(
            sink.finish(),
            Err(Error::Internal("unbalanced clip stack"))
        ));
    }

    #[test]
    fn hybrid_scene_sink_rejects_crossed_clip_and_group_stacks() {
        let mut scene = VelloHybridScene::new(32, 32);
        let mut sink = VelloHybridSceneSink::new(&mut scene);
        sink.push_group(GroupRef::new());
        sink.push_clip(ClipRef::fill(kurbo::Rect::new(0.0, 0.0, 16.0, 16.0)));
        sink.pop_group();
        assert!(matches!(
            sink.finish(),
            Err(Error::Internal("unbalanced group stack"))
        ));
    }

    #[test]
    fn hybrid_scene_sink_rejects_filters() {
        let mut scene = VelloHybridScene::new(32, 32);
        scene.reset();
        let mut sink = VelloHybridSceneSink::new(&mut scene);
        sink.push_group(GroupRef::new().with_filters(&[Filter::blur(2.0)]));
        assert!(matches!(sink.finish(), Err(Error::UnsupportedFilter)));
    }

    #[test]
    fn hybrid_scene_sink_rejects_image_brushes_without_resolver() {
        let mut scene = VelloHybridScene::new(32, 32);
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

    #[test]
    fn hybrid_scene_sink_reports_unrenderable_glyphs() {
        let font = peniko::FontData::new(
            Blob::new(Arc::new(include_bytes!(
                "../../test_assets/fonts/NotoSans-Regular.ttf"
            ))),
            0,
        );
        let mut scene = VelloHybridScene::new(32, 32);
        let mut sink = VelloHybridSceneSink::new(&mut scene);
        imaging::Painter::new(&mut sink)
            .glyphs(&font, peniko::Color::BLACK)
            .draw(
                &Style::Fill(peniko::Fill::NonZero),
                [record::Glyph {
                    id: u32::MAX,
                    x: 0.0,
                    y: 0.0,
                }],
            );
        assert!(matches!(
            sink.finish(),
            Err(Error::Internal("glyph rendering failed"))
        ));
    }

    #[test]
    fn hybrid_scene_sink_rejects_destructive_glyph_blends() {
        let font = peniko::FontData::new(Blob::new(Arc::new([0_u8; 4])), 0);
        let mut scene = VelloHybridScene::new(32, 32);
        let mut sink = VelloHybridSceneSink::new(&mut scene);
        imaging::Painter::new(&mut sink)
            .glyphs(&font, peniko::Color::BLACK)
            .composite(Composite::new(peniko::Compose::Copy.into(), 1.0))
            .draw(
                &Style::Fill(peniko::Fill::NonZero),
                core::iter::empty::<record::Glyph>(),
            );
        assert!(matches!(sink.finish(), Err(Error::UnsupportedGlyphBlend)));
    }

    #[test]
    fn hybrid_scene_sink_rejects_luminance_masks() {
        let mut mask = Scene::new();
        mask.fill(FillRef::new(
            kurbo::Rect::new(0.0, 0.0, 8.0, 8.0),
            peniko::Color::WHITE,
        ));
        let mut content = Scene::new();
        content.fill(FillRef::new(
            kurbo::Rect::new(1.0, 1.0, 7.0, 7.0),
            peniko::Color::BLACK,
        ));

        let mut scene = VelloHybridScene::new(32, 32);
        scene.reset();
        let mut sink = VelloHybridSceneSink::new(&mut scene);
        sink.push_group(GroupRef::new().with_mask(MaskRef::new(MaskMode::Luminance, &mask)));
        sink.fill(FillRef::new(
            kurbo::Rect::new(1.0, 1.0, 7.0, 7.0),
            peniko::Color::BLACK,
        ));
        sink.pop_group();
        assert!(matches!(sink.finish(), Err(Error::UnsupportedMask)));
    }
}
