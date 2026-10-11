// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::Error;
use crate::vello::{self, Glyph as VelloGlyph};
use crate::{ExternalTexture, VelloScene};
use imaging::{
    BlurredRoundedRect, ClipRef, Composite, FillRef, GeometryRef, GlyphRunRef, GroupRef, MaskMode,
    PaintSink, StrokeRef,
    record::{Scene, replay_transformed},
};
use kurbo::{Affine, Rect};
use peniko::{Brush, Fill};
use std::{boxed::Box, collections::HashMap, sync::Arc};

/// Borrowed adapter that streams `imaging` commands into an existing [`VelloScene`].
///
/// Use this when you want a backend-native retained scene without owning a full
/// [`crate::VelloRenderer`]. Call [`VelloSceneSink::finish`] after streaming to surface any
/// deferred translation errors and to confirm the Vello layer stack is balanced.
pub struct VelloSceneSink<'a> {
    scene: &'a mut vello::Scene,
    images: &'a mut Vec<Arc<ExternalTexture>>,
    renderer: &'a mut Option<Arc<()>>,
    binding_renderer: Option<&'a Arc<()>>,
    bindings: Option<&'a HashMap<imaging::ExternalImage, Arc<ExternalTexture>>>,
    finished: bool,
    surface_clip: Rect,
    error: Option<Error>,
    layer_stack: Vec<LayerFrame>,
}

#[derive(Clone, Debug)]
struct PendingMask {
    scene: Scene,
    mode: MaskMode,
    transform: Affine,
}

#[derive(Clone, Debug)]
enum LayerFrame {
    Clip,
    Group { mask: Option<Box<PendingMask>> },
}

impl Drop for VelloSceneSink<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.scene.reset();
            self.images.clear();
            *self.renderer = None;
        }
    }
}

impl core::fmt::Debug for VelloSceneSink<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VelloSceneSink")
            .field("surface_clip", &self.surface_clip)
            .field("error", &self.error)
            .field("layer_stack_depth", &self.layer_stack.len())
            .finish_non_exhaustive()
    }
}

impl<'a> VelloSceneSink<'a> {
    /// Wrap an existing [`VelloScene`].
    ///
    /// `surface_clip` is used as the fallback layer bounds for isolated groups that do not
    /// provide an explicit clip.
    pub fn new(scene: &'a mut VelloScene, surface_clip: Rect) -> Self {
        Self {
            scene: &mut scene.native,
            images: &mut scene.images,
            renderer: &mut scene.renderer,
            bindings: None,
            binding_renderer: None,
            finished: false,
            surface_clip,
            error: None,
            layer_stack: Vec::new(),
        }
    }

    /// Record a scene using this renderer's external image bindings.
    pub fn with_renderer(
        scene: &'a mut VelloScene,
        surface_clip: Rect,
        renderer: &'a crate::VelloRenderer,
    ) -> Self {
        Self::with_bindings(
            scene,
            surface_clip,
            &renderer.state.external_images,
            &renderer.state.identity,
        )
    }

    pub(crate) fn with_bindings(
        scene: &'a mut VelloScene,
        surface_clip: Rect,
        bindings: &'a HashMap<imaging::ExternalImage, Arc<ExternalTexture>>,
        identity: &'a Arc<()>,
    ) -> Self {
        let wrong_renderer = scene
            .renderer
            .as_ref()
            .is_some_and(|held| !Arc::ptr_eq(held, identity));
        let mut sink = Self::new(scene, surface_clip);
        sink.bindings = Some(bindings);
        sink.binding_renderer = Some(identity);
        if wrong_renderer {
            sink.error = Some(Error::WrongRenderer);
        }
        sink
    }

    /// Complete recording and report translation or unbalanced-stack errors.
    ///
    /// Failed or unfinished recordings are discarded, releasing their image bindings.
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
            self.images.clear();
            *self.renderer = None;
        }
        self.finished = true;
        result
    }

    fn set_error_once(&mut self, err: Error) {
        if self.error.is_none() {
            self.error = Some(err);
        }
    }

    fn brush_to_brush(
        &mut self,
        brush: imaging::BrushRef<'_>,
        composite: Composite,
    ) -> Option<Brush> {
        if let imaging::BrushRef::Image(imaging::ImageBrushRef {
            image: imaging::ImageRef::External(handle),
            mut sampler,
        }) = brush
        {
            let Some(binding) = self.bindings.and_then(|bindings| bindings.get(handle)) else {
                self.set_error_once(Error::UnboundImage(handle.clone()));
                return None;
            };
            *self.renderer = self.binding_renderer.cloned();
            if !self.images.iter().any(|held| Arc::ptr_eq(held, binding)) {
                self.images.push(Arc::clone(binding));
            }
            sampler.alpha *= composite.alpha;
            return Some(Brush::Image(peniko::ImageBrush {
                image: binding.image.clone(),
                sampler,
            }));
        }
        let brush = brush
            .as_raster()
            .expect("external images were resolved above");
        let brush = brush.to_owned().multiply_alpha(composite.alpha);
        match brush {
            Brush::Solid(_) | Brush::Gradient(_) | Brush::Image(_) => Some(brush),
        }
    }

    fn push_clip_frame(&mut self) {
        self.layer_stack.push(LayerFrame::Clip);
    }

    fn push_group_frame(&mut self, mask: Option<PendingMask>) {
        self.layer_stack.push(LayerFrame::Group {
            mask: mask.map(Box::new),
        });
    }

    fn pop_clip_frame(&mut self) -> bool {
        match self.layer_stack.pop() {
            Some(LayerFrame::Clip) => true,
            _ => {
                self.set_error_once(Error::UnbalancedLayerStack);
                false
            }
        }
    }

    fn pop_group_frame(&mut self) -> Option<Option<PendingMask>> {
        match self.layer_stack.pop() {
            Some(LayerFrame::Group { mask }) => Some(mask.map(|mask| *mask)),
            _ => {
                self.set_error_once(Error::UnbalancedLayerStack);
                None
            }
        }
    }

    fn draw_glyph_run(
        &mut self,
        glyph_run: GlyphRunRef<'_>,
        glyphs: &mut dyn Iterator<Item = imaging::record::Glyph>,
    ) {
        if glyph_run.composite.blend != peniko::BlendMode::default() {
            self.set_error_once(Error::UnsupportedGlyphBlend);
            return;
        }
        // The glyph builder applies composite alpha, including to color glyphs.
        let Some(paint) = self.brush_to_brush(glyph_run.brush, Composite::default()) else {
            return;
        };

        let builder = self
            .scene
            .draw_glyphs(glyph_run.font)
            .transform(glyph_run.transform)
            .font_size(glyph_run.font_size)
            .hint(glyph_run.hint)
            .normalized_coords(glyph_run.normalized_coords)
            .brush(&paint);
        let builder = builder.brush_transform(glyph_run.brush_transform);
        let builder = builder.brush_alpha(glyph_run.composite.alpha);
        let builder = builder.glyph_transform(glyph_run.glyph_transform);
        let glyphs = glyphs.map(|glyph| VelloGlyph {
            id: glyph.id,
            x: glyph.x,
            y: glyph.y,
        });
        builder.draw(glyph_run.style, glyphs);
    }

    fn draw_blurred_rounded_rect(&mut self, draw: BlurredRoundedRect) {
        if draw.composite.blend != peniko::BlendMode::default() {
            self.set_error_once(Error::UnsupportedBlurredRoundedRectBlend);
            return;
        }
        self.scene.draw_blurred_rounded_rect(
            draw.transform,
            draw.rect,
            draw.color.multiply_alpha(draw.composite.alpha),
            draw.radius,
            draw.std_dev,
        );
    }

    fn replay_masked_subscene(&mut self, scene: &Scene, transform: Affine) {
        replay_transformed(scene, self, transform);
    }
}

impl PaintSink for VelloSceneSink<'_> {
    fn push_clip(&mut self, clip: ClipRef<'_>) {
        if self.error.is_some() {
            return;
        }

        match clip {
            ClipRef::Fill {
                transform,
                shape,
                fill_rule,
            } => match shape {
                GeometryRef::Rect(r) => self.scene.push_clip_layer(fill_rule, transform, &r),
                GeometryRef::RoundedRect(rr) => {
                    self.scene.push_clip_layer(fill_rule, transform, &rr);
                }
                GeometryRef::Path(p) => self.scene.push_clip_layer(fill_rule, transform, p),
                GeometryRef::OwnedPath(p) => self.scene.push_clip_layer(fill_rule, transform, &p),
            },
            ClipRef::Stroke {
                transform,
                shape,
                stroke,
            } => match shape {
                GeometryRef::Rect(r) => self.scene.push_clip_layer(stroke, transform, &r),
                GeometryRef::RoundedRect(rr) => self.scene.push_clip_layer(stroke, transform, &rr),
                GeometryRef::Path(p) => self.scene.push_clip_layer(stroke, transform, p),
                GeometryRef::OwnedPath(p) => self.scene.push_clip_layer(stroke, transform, &p),
            },
        }
        self.push_clip_frame();
    }

    fn pop_clip(&mut self) {
        if self.error.is_some() {
            return;
        }
        if !self.pop_clip_frame() {
            return;
        }
        self.scene.pop_layer();
    }

    fn push_group(&mut self, group: GroupRef<'_>) {
        if self.error.is_some() {
            return;
        }
        if !group.filters.is_empty() {
            self.set_error_once(Error::UnsupportedFilter);
            return;
        }
        if let Some(clip) = group.clip {
            match clip {
                ClipRef::Fill {
                    transform,
                    shape,
                    fill_rule,
                } => match shape {
                    GeometryRef::Rect(r) => self.scene.push_layer(
                        fill_rule,
                        group.composite.blend,
                        group.composite.alpha,
                        transform,
                        &r,
                    ),
                    GeometryRef::RoundedRect(rr) => self.scene.push_layer(
                        fill_rule,
                        group.composite.blend,
                        group.composite.alpha,
                        transform,
                        &rr,
                    ),
                    GeometryRef::Path(p) => self.scene.push_layer(
                        fill_rule,
                        group.composite.blend,
                        group.composite.alpha,
                        transform,
                        p,
                    ),
                    GeometryRef::OwnedPath(p) => self.scene.push_layer(
                        fill_rule,
                        group.composite.blend,
                        group.composite.alpha,
                        transform,
                        &p,
                    ),
                },
                ClipRef::Stroke {
                    transform,
                    shape,
                    stroke,
                } => match shape {
                    GeometryRef::Rect(r) => self.scene.push_layer(
                        stroke,
                        group.composite.blend,
                        group.composite.alpha,
                        transform,
                        &r,
                    ),
                    GeometryRef::RoundedRect(rr) => self.scene.push_layer(
                        stroke,
                        group.composite.blend,
                        group.composite.alpha,
                        transform,
                        &rr,
                    ),
                    GeometryRef::Path(p) => self.scene.push_layer(
                        stroke,
                        group.composite.blend,
                        group.composite.alpha,
                        transform,
                        p,
                    ),
                    GeometryRef::OwnedPath(p) => self.scene.push_layer(
                        stroke,
                        group.composite.blend,
                        group.composite.alpha,
                        transform,
                        &p,
                    ),
                },
            }
        } else {
            self.scene.push_layer(
                Fill::NonZero,
                group.composite.blend,
                group.composite.alpha,
                Affine::IDENTITY,
                &self.surface_clip,
            );
        }
        self.push_group_frame(group.mask.map(|mask| PendingMask {
            scene: mask.mask.scene.clone(),
            mode: mask.mask.mode,
            transform: mask.transform,
        }));
    }

    fn pop_group(&mut self) {
        if self.error.is_some() {
            return;
        }
        let Some(mask) = self.pop_group_frame() else {
            return;
        };
        if let Some(mask) = mask {
            match mask.mode {
                MaskMode::Alpha => self.scene.push_layer(
                    Fill::NonZero,
                    peniko::Compose::DestIn,
                    1.0,
                    Affine::IDENTITY,
                    &self.surface_clip,
                ),
                MaskMode::Luminance => self.scene.push_luminance_mask_layer(
                    Fill::NonZero,
                    1.0,
                    Affine::IDENTITY,
                    &self.surface_clip,
                ),
            }
            // Render the mask normally into an isolated source. DestIn uses its final alpha,
            // preserving overlap, blend modes, nested groups, and image sampling.
            self.replay_masked_subscene(&mask.scene, mask.transform);
            self.scene.pop_layer();
        }
        self.scene.pop_layer();
    }

    fn fill(&mut self, draw: FillRef<'_>) {
        if self.error.is_some() {
            return;
        }

        let Some(paint) = self.brush_to_brush(draw.brush, draw.composite) else {
            return;
        };

        let (blend, paint) = match (&paint, draw.composite.blend.compose) {
            (Brush::Solid(c), peniko::Compose::Copy) if c.components[3] == 0.0 => (
                peniko::BlendMode::new(peniko::Mix::Normal, peniko::Compose::DestOut),
                Brush::Solid(peniko::Color::from_rgba8(0, 0, 0, 255)),
            ),
            _ => (draw.composite.blend, paint),
        };

        if blend != peniko::BlendMode::default() {
            match &draw.shape {
                GeometryRef::Rect(r) => {
                    self.scene
                        .push_layer(draw.fill_rule, blend, 1.0, draw.transform, r);
                }
                GeometryRef::RoundedRect(rr) => {
                    self.scene
                        .push_layer(draw.fill_rule, blend, 1.0, draw.transform, rr);
                }
                GeometryRef::Path(p) => {
                    self.scene
                        .push_layer(draw.fill_rule, blend, 1.0, draw.transform, *p);
                }
                GeometryRef::OwnedPath(p) => {
                    self.scene
                        .push_layer(draw.fill_rule, blend, 1.0, draw.transform, p);
                }
            }
            self.push_group_frame(None);
        }

        match draw.shape {
            GeometryRef::Rect(r) => {
                self.scene.fill(
                    draw.fill_rule,
                    draw.transform,
                    &paint,
                    draw.brush_transform,
                    &r,
                );
            }
            GeometryRef::RoundedRect(rr) => {
                self.scene.fill(
                    draw.fill_rule,
                    draw.transform,
                    &paint,
                    draw.brush_transform,
                    &rr,
                );
            }
            GeometryRef::Path(p) => {
                self.scene.fill(
                    draw.fill_rule,
                    draw.transform,
                    &paint,
                    draw.brush_transform,
                    p,
                );
            }
            GeometryRef::OwnedPath(p) => {
                self.scene.fill(
                    draw.fill_rule,
                    draw.transform,
                    &paint,
                    draw.brush_transform,
                    &p,
                );
            }
        }

        if blend != peniko::BlendMode::default() {
            if self.pop_group_frame().is_none() {
                return;
            }
            self.scene.pop_layer();
        }
    }

    fn stroke(&mut self, draw: StrokeRef<'_>) {
        if self.error.is_some() {
            return;
        }

        let Some(paint) = self.brush_to_brush(draw.brush, draw.composite) else {
            return;
        };

        let (blend, paint) = match (&paint, draw.composite.blend.compose) {
            (Brush::Solid(c), peniko::Compose::Copy) if c.components[3] == 0.0 => (
                peniko::BlendMode::new(peniko::Mix::Normal, peniko::Compose::DestOut),
                Brush::Solid(peniko::Color::from_rgba8(0, 0, 0, 255)),
            ),
            _ => (draw.composite.blend, paint),
        };

        if blend != peniko::BlendMode::default() {
            match &draw.shape {
                GeometryRef::Rect(r) => {
                    self.scene
                        .push_layer(draw.stroke, blend, 1.0, draw.transform, r);
                }
                GeometryRef::RoundedRect(rr) => {
                    self.scene
                        .push_layer(draw.stroke, blend, 1.0, draw.transform, rr);
                }
                GeometryRef::Path(p) => {
                    self.scene
                        .push_layer(draw.stroke, blend, 1.0, draw.transform, *p);
                }
                GeometryRef::OwnedPath(p) => {
                    self.scene
                        .push_layer(draw.stroke, blend, 1.0, draw.transform, p);
                }
            }
            self.push_group_frame(None);
        }

        match draw.shape {
            GeometryRef::Rect(r) => {
                self.scene.stroke(
                    draw.stroke,
                    draw.transform,
                    &paint,
                    draw.brush_transform,
                    &r,
                );
            }
            GeometryRef::RoundedRect(rr) => {
                self.scene.stroke(
                    draw.stroke,
                    draw.transform,
                    &paint,
                    draw.brush_transform,
                    &rr,
                );
            }
            GeometryRef::Path(p) => {
                self.scene
                    .stroke(draw.stroke, draw.transform, &paint, draw.brush_transform, p);
            }
            GeometryRef::OwnedPath(p) => {
                self.scene.stroke(
                    draw.stroke,
                    draw.transform,
                    &paint,
                    draw.brush_transform,
                    &p,
                );
            }
        }

        if blend != peniko::BlendMode::default() {
            if self.pop_group_frame().is_none() {
                return;
            }
            self.scene.pop_layer();
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
    use imaging::{Filter, MaskMode, MaskRef};
    use peniko::{Blob, Color, Fill, FontData, Style};
    use std::sync::Arc;

    #[test]
    fn vello_scene_sink_reports_unbalanced_layer_stack() {
        let mut scene = VelloScene::new();
        let mut sink = VelloSceneSink::new(&mut scene, Rect::new(0.0, 0.0, 32.0, 32.0));
        sink.pop_group();
        assert!(matches!(sink.finish(), Err(Error::UnbalancedLayerStack)));
    }

    #[test]
    fn vello_scene_sink_rejects_filters() {
        let mut scene = VelloScene::new();
        let mut sink = VelloSceneSink::new(&mut scene, Rect::new(0.0, 0.0, 32.0, 32.0));
        sink.push_group(GroupRef::new().with_filters(&[Filter::blur(2.0)]));
        assert!(matches!(sink.finish(), Err(Error::UnsupportedFilter)));
    }

    #[test]
    fn vello_scene_sink_accepts_glyph_brush_transforms() {
        const TEST_FONT_BYTES: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../test_assets/fonts/NotoSans-Regular.ttf"
        ));

        let font = FontData::new(Blob::new(Arc::new(TEST_FONT_BYTES)), 0);
        let style = Style::Fill(Fill::NonZero);
        let mut glyphs = [imaging::record::Glyph {
            id: 1,
            x: 0.0,
            y: 0.0,
        }]
        .into_iter();

        let mut scene = VelloScene::new();
        let mut sink = VelloSceneSink::new(&mut scene, Rect::new(0.0, 0.0, 32.0, 32.0));
        sink.glyph_run(
            GlyphRunRef::new(&font, &style, Color::BLACK)
                .brush_transform(Some(Affine::translate((-200.0, 0.0)))),
            &mut glyphs,
        );
        sink.finish().expect("glyph brush transform should encode");
    }

    #[test]
    fn vello_scene_sink_supports_luminance_masks() {
        let mut mask = Scene::new();
        mask.fill(FillRef::new(Rect::new(0.0, 0.0, 16.0, 16.0), Color::WHITE));

        let mut scene = VelloScene::new();
        let mut sink = VelloSceneSink::new(&mut scene, Rect::new(0.0, 0.0, 32.0, 32.0));
        sink.push_group(GroupRef::new().with_mask(MaskRef::new(MaskMode::Luminance, &mask)));
        sink.fill(FillRef::new(Rect::new(4.0, 4.0, 20.0, 20.0), Color::BLACK));
        sink.pop_group();
        assert!(matches!(sink.finish(), Ok(())));
    }

    #[test]
    fn vello_scene_sink_supports_alpha_masks() {
        let mut mask = Scene::new();
        mask.fill(FillRef::new(Rect::new(0.0, 0.0, 16.0, 16.0), Color::WHITE));

        let mut scene = VelloScene::new();
        let mut sink = VelloSceneSink::new(&mut scene, Rect::new(0.0, 0.0, 32.0, 32.0));
        sink.push_group(GroupRef::new().with_mask(MaskRef::new(MaskMode::Alpha, &mask)));
        sink.fill(FillRef::new(Rect::new(4.0, 4.0, 20.0, 20.0), Color::BLACK));
        sink.pop_group();
        sink.finish().unwrap();
    }
}
