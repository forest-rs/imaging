// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Vello CPU backend for `imaging`.
//!
//! This crate provides a CPU renderer that consumes `imaging::record::Scene` (or accepts commands
//! directly via `imaging::PaintSink`) and produces an RGBA8 image buffer using `vello_cpu`.
//!
//! # Render A Recorded Scene
//!
//! Record commands into [`imaging::record::Scene`], then render them with [`VelloCpuRenderer`].
//!
//! ```no_run
//! use imaging::{Painter, record};
//! use imaging_vello_cpu::VelloCpuRenderer;
//! use kurbo::Rect;
//! use peniko::{Brush, Color};
//!
//! fn main() -> Result<(), imaging_vello_cpu::Error> {
//!     let paint = Brush::Solid(Color::from_rgb8(0x2a, 0x6f, 0xdb));
//!     let mut scene = record::Scene::new();
//!
//!     {
//!         let mut painter = Painter::new(&mut scene);
//!         painter.fill_rect(Rect::new(0.0, 0.0, 128.0, 128.0), &paint);
//!     }
//!
//!     let mut renderer = VelloCpuRenderer::new(128, 128);
//!     let image = renderer.render_scene(&scene, 128, 128)?;
//!     assert_eq!(image.data.len(), 128 * 128 * 4);
//!     Ok(())
//! }
//! ```
//!
//! # Stream Commands Directly
//!
//! [`VelloCpuRenderer`] also implements [`imaging::PaintSink`], so you can stream commands
//! directly and call [`VelloCpuRenderer::finish`] when the frame is complete.
//!
//! ```no_run
//! use imaging::Painter;
//! use imaging_vello_cpu::VelloCpuRenderer;
//! use kurbo::Rect;
//! use peniko::{Brush, Color};
//!
//! fn main() -> Result<(), imaging_vello_cpu::Error> {
//!     let paint = Brush::Solid(Color::from_rgb8(0xd9, 0x77, 0x06));
//!     let mut renderer = VelloCpuRenderer::new(128, 128);
//!
//!     {
//!         let mut painter = Painter::new(&mut renderer);
//!         painter.fill_rect(Rect::new(16.0, 16.0, 112.0, 112.0), &paint);
//!     }
//!
//!     let image = renderer.finish()?;
//!     assert_eq!(image.data.len(), 128 * 128 * 4);
//!     Ok(())
//! }
//! ```

#![deny(unsafe_code)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![no_std]
extern crate alloc;

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use imaging::{
    BlurredRoundedRect, ClipRef, Composite, FillRef, Filter, GeometryRef, GlyphRunRef, GroupRef,
    MaskMode, PaintSink, RgbaImage, StrokeRef,
    record::{Scene, ValidateError, replay, replay_transformed},
    render::{
        ImageBufferFormat, ImageBufferTarget, ImageRenderer, ImageRendererError, ImageTargetError,
        RenderContentError, RenderSource,
    },
};
use kurbo::{Affine, Rect, Shape as _};
use peniko::{BlendMode, Brush, BrushRef, Fill, Style};
use vello_common::filter_effects::{EdgeMode, Filter as VelloFilter, FilterGraph, FilterPrimitive};
use vello_common::glyph::Glyph as VelloGlyph;
use vello_common::paint::{Image as VelloImage, ImageSource};
use vello_cpu::kurbo::{BezPath, StrokeOpts, stroke};
use vello_cpu::{Pixmap, RenderContext, RenderMode, RenderSettings};

/// Errors that can occur when rendering via Vello CPU.
#[derive(Debug)]
pub enum Error {
    /// The scene is invalid (unbalanced stacks).
    InvalidScene(ValidateError),
    /// An internal invariant was violated.
    Internal(&'static str),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl core::error::Error for Error {}

/// Renderer that executes `imaging` commands using `vello_cpu`.
#[derive(Debug)]
pub struct VelloCpuRenderer {
    ctx: RenderContext,
    width: u16,
    height: u16,
    tolerance: f64,
    error: Option<Error>,
    clip_depth: u32,
    group_depth: u32,
    mask_cache: VecDeque<CachedMask>,
}

#[derive(Clone, Debug)]
struct CachedMask {
    scene: Scene,
    mode: MaskMode,
    transform: Affine,
    mask: vello_cpu::Mask,
}

impl VelloCpuRenderer {
    fn checked_size(width: u32, height: u32) -> Result<(u16, u16), Error> {
        let width = u16::try_from(width).map_err(|_| Error::Internal("render width too large"))?;
        let height =
            u16::try_from(height).map_err(|_| Error::Internal("render height too large"))?;
        Ok((width, height))
    }

    fn render_settings() -> RenderSettings {
        RenderSettings {
            render_mode: RenderMode::OptimizeSpeed,
            ..RenderSettings::default()
        }
    }

    /// Create a renderer with an initial target size.
    ///
    /// Scene rendering methods resize this target on demand. The renderer uses Vello CPU's
    /// `OptimizeSpeed` mode by default to keep snapshots stable.
    pub fn new(width: u16, height: u16) -> Self {
        let ctx = RenderContext::new_with(width, height, Self::render_settings());
        Self {
            ctx,
            width,
            height,
            tolerance: 0.1,
            error: None,
            clip_depth: 0,
            group_depth: 0,
            mask_cache: VecDeque::new(),
        }
    }

    /// Set the tolerance used when converting shapes to paths.
    pub fn set_tolerance(&mut self, tolerance: f64) {
        if self.tolerance != tolerance {
            self.tolerance = tolerance;
            self.clear_cached_masks();
        }
    }

    /// Reset the internal Vello CPU context and local error state.
    pub fn reset(&mut self) {
        self.ctx.reset();
        self.error = None;
        self.clip_depth = 0;
        self.group_depth = 0;
    }

    /// Drop any realized mask artifacts cached by the renderer.
    ///
    /// The cache is renderer-scoped so unchanged masked subscenes can be reused across renders.
    /// Call this if you need to release memory aggressively or after changing assumptions that
    /// affect mask realization outside the recorded scene itself.
    pub fn clear_cached_masks(&mut self) {
        self.mask_cache.clear();
    }

    fn resize(&mut self, width: u16, height: u16) {
        if self.width == width && self.height == height {
            return;
        }

        self.ctx = RenderContext::new_with(width, height, Self::render_settings());
        self.width = width;
        self.height = height;
        self.clear_cached_masks();
        self.error = None;
        self.clip_depth = 0;
        self.group_depth = 0;
    }

    /// Render a recorded scene into an RGBA8 image (unpremultiplied).
    pub fn render_scene_into(
        &mut self,
        scene: &Scene,
        width: u16,
        height: u16,
        image: &mut RgbaImage,
    ) -> Result<(), Error> {
        scene.validate().map_err(Error::InvalidScene)?;
        self.resize(width, height);
        self.reset();
        replay(scene, self);
        self.finish_into(image)
    }

    /// Render a recorded scene and return an RGBA8 image (unpremultiplied).
    pub fn render_scene(
        &mut self,
        scene: &Scene,
        width: u16,
        height: u16,
    ) -> Result<RgbaImage, Error> {
        let mut image = RgbaImage::new(u32::from(width), u32::from(height));
        self.render_scene_into(scene, width, height, &mut image)?;
        Ok(image)
    }

    /// Finish rendering the current command stream into an RGBA8 image (unpremultiplied).
    pub fn finish_into(&mut self, image: &mut RgbaImage) -> Result<(), Error> {
        if let Some(err) = self.error.take() {
            return Err(err);
        }
        if self.clip_depth != 0 {
            return Err(Error::Internal("unbalanced clip stack"));
        }
        if self.group_depth != 0 {
            return Err(Error::Internal("unbalanced group stack"));
        }

        image.resize(u32::from(self.width), u32::from(self.height));
        self.ctx.flush();
        self.ctx.render_to_buffer(
            image.data.as_mut_slice(),
            self.width,
            self.height,
            Self::render_settings().render_mode,
        );
        unpremultiply_rgba8_in_place(image.data.as_mut_slice());
        Ok(())
    }

    fn finish_into_target(&mut self, target: ImageBufferTarget<'_>) -> Result<(), Error> {
        if let Some(err) = self.error.take() {
            return Err(err);
        }
        if self.clip_depth != 0 {
            return Err(Error::Internal("unbalanced clip stack"));
        }
        if self.group_depth != 0 {
            return Err(Error::Internal("unbalanced group stack"));
        }
        if target.width != u32::from(self.width) || target.height != u32::from(self.height) {
            return Err(Error::Internal(
                "image target dimensions do not match renderer output",
            ));
        }
        let width_bytes = usize::from(self.width) * 4;
        if target.bytes_per_row != width_bytes {
            return Err(Error::Internal(
                "image target row stride must be tightly packed",
            ));
        }
        let required_len = target
            .bytes_per_row
            .checked_mul(usize::from(self.height))
            .expect("image target byte length should fit in usize");
        if target.data.len() < required_len {
            return Err(Error::Internal("image target buffer is too small"));
        }

        self.ctx.flush();
        self.ctx.render_to_buffer(
            &mut target.data[..required_len],
            self.width,
            self.height,
            Self::render_settings().render_mode,
        );
        unpremultiply_rgba8_in_place(&mut target.data[..required_len]);
        Ok(())
    }

    /// Finish rendering the current command stream and return an RGBA8 image (unpremultiplied).
    pub fn finish(&mut self) -> Result<RgbaImage, Error> {
        let mut image = RgbaImage::new(u32::from(self.width), u32::from(self.height));
        self.finish_into(&mut image)?;
        Ok(image)
    }

    fn set_error_once(&mut self, err: Error) {
        if self.error.is_none() {
            self.error = Some(err);
        }
    }

    fn rect_fits_viewport_fast_path(&self, rect: &Rect, transform: Affine) -> bool {
        let transformed = transform.transform_rect_bbox(*rect);
        transformed.x0 >= 0.0
            && transformed.y0 >= 0.0
            && transformed.x1 <= f64::from(self.width)
            && transformed.y1 <= f64::from(self.height)
    }

    fn brush_to_paint(
        &mut self,
        brush: BrushRef<'_>,
        composite: Composite,
    ) -> Option<vello_cpu::PaintType> {
        let brush = brush.to_owned().multiply_alpha(composite.alpha);
        let paint: vello_cpu::PaintType = match brush {
            Brush::Solid(c) => Brush::Solid(c),
            Brush::Gradient(g) => Brush::Gradient(g),
            Brush::Image(image) => Brush::Image(VelloImage {
                image: ImageSource::from_peniko_image_data(&image.image),
                sampler: image.sampler,
            }),
        };
        Some(paint)
    }

    fn geometry_to_path(&self, geom: GeometryRef<'_>) -> BezPath {
        match geom {
            GeometryRef::Rect(r) => r.to_path(self.tolerance),
            GeometryRef::RoundedRect(rr) => rr.to_path(self.tolerance),
            GeometryRef::Path(p) => p.clone(),
            GeometryRef::OwnedPath(p) => p,
        }
    }

    fn clip_to_path(&mut self, clip: ClipRef<'_>) -> (Affine, BezPath, Fill) {
        match clip {
            ClipRef::Fill {
                transform,
                shape,
                fill_rule,
            } => (transform, self.geometry_to_path(shape), fill_rule),
            ClipRef::Stroke {
                transform,
                shape,
                stroke: style,
            } => {
                let path = self.geometry_to_path(shape);
                let outline = stroke(path.iter(), style, &StrokeOpts::default(), self.tolerance);
                (transform, outline, Fill::NonZero)
            }
        }
    }

    fn filters_to_vello(&mut self, filters: &[Filter]) -> Option<VelloFilter> {
        if filters.is_empty() {
            return None;
        }

        let mut graph = FilterGraph::new();
        let mut last = None;
        for f in filters {
            let primitive = match *f {
                Filter::Flood { color } => FilterPrimitive::Flood { color },
                Filter::Blur {
                    std_deviation_x,
                    std_deviation_y,
                } => FilterPrimitive::GaussianBlur {
                    std_deviation: std_deviation_x.max(std_deviation_y),
                    edge_mode: EdgeMode::None,
                },
                Filter::DropShadow {
                    dx,
                    dy,
                    std_deviation_x,
                    std_deviation_y,
                    color,
                } => FilterPrimitive::DropShadow {
                    dx,
                    dy,
                    std_deviation: std_deviation_x.max(std_deviation_y),
                    color,
                    edge_mode: EdgeMode::None,
                },
                Filter::Offset { dx, dy } => FilterPrimitive::Offset { dx, dy },
            };
            last = Some(graph.add(primitive, None));
        }
        if let Some(out) = last {
            graph.set_output(out);
        }
        Some(VelloFilter {
            graph: Arc::new(graph),
        })
    }

    fn draw_glyph_run(
        &mut self,
        glyph_run: GlyphRunRef<'_>,
        glyphs: &mut dyn Iterator<Item = imaging::record::Glyph>,
    ) {
        let Some(paint) = self.brush_to_paint(glyph_run.brush, glyph_run.composite) else {
            return;
        };
        self.ctx.set_transform(glyph_run.transform);
        self.ctx.set_paint(paint);
        self.ctx.set_blend_mode(glyph_run.composite.blend);

        match glyph_run.style {
            Style::Fill(fill_rule) => {
                self.ctx.set_fill_rule(*fill_rule);
                let builder = self
                    .ctx
                    .glyph_run(glyph_run.font)
                    .font_size(glyph_run.font_size)
                    .hint(glyph_run.hint)
                    .normalized_coords(glyph_run.normalized_coords);
                let builder = if let Some(transform) = glyph_run.glyph_transform {
                    builder.glyph_transform(transform)
                } else {
                    builder
                };
                let glyphs = glyphs.map(|glyph| VelloGlyph {
                    id: glyph.id,
                    x: glyph.x,
                    y: glyph.y,
                });
                builder.fill_glyphs(glyphs);
            }
            Style::Stroke(stroke) => {
                self.ctx.set_stroke(stroke.clone());
                let builder = self
                    .ctx
                    .glyph_run(glyph_run.font)
                    .font_size(glyph_run.font_size)
                    .hint(glyph_run.hint)
                    .normalized_coords(glyph_run.normalized_coords);
                let builder = if let Some(transform) = glyph_run.glyph_transform {
                    builder.glyph_transform(transform)
                } else {
                    builder
                };
                let glyphs = glyphs.map(|glyph| VelloGlyph {
                    id: glyph.id,
                    x: glyph.x,
                    y: glyph.y,
                });
                builder.stroke_glyphs(glyphs);
            }
        }
    }

    fn draw_blurred_rounded_rect(&mut self, draw: BlurredRoundedRect) {
        self.ctx.set_transform(draw.transform);
        self.ctx
            .set_paint(draw.color.multiply_alpha(draw.composite.alpha));
        self.ctx.set_blend_mode(draw.composite.blend);
        self.ctx.fill_blurred_rounded_rect(
            &draw.rect,
            f64_to_f32(draw.radius),
            f64_to_f32(draw.std_dev),
        );
    }

    fn render_mask(
        &mut self,
        scene: &Scene,
        mode: MaskMode,
        transform: Affine,
    ) -> Option<vello_cpu::Mask> {
        if let Some(mask) = self.lookup_cached_mask(scene, mode, transform) {
            return Some(mask);
        }

        let mut renderer = Self::new(self.width, self.height);
        renderer.set_tolerance(self.tolerance);
        replay_transformed(scene, &mut renderer, transform);
        if let Some(err) = renderer.error.take() {
            self.set_error_once(err);
            return None;
        }
        if renderer.clip_depth != 0 {
            self.set_error_once(Error::Internal("unbalanced clip stack in mask scene"));
            return None;
        }
        if renderer.group_depth != 0 {
            self.set_error_once(Error::Internal("unbalanced group stack in mask scene"));
            return None;
        }

        let mut pixmap = Pixmap::new(self.width, self.height);
        renderer.ctx.flush();
        renderer.ctx.render_to_pixmap(&mut pixmap);
        let mask = match mode {
            MaskMode::Alpha => vello_cpu::Mask::new_alpha(&pixmap),
            MaskMode::Luminance => vello_cpu::Mask::new_luminance(&pixmap),
        };
        self.store_cached_mask(scene, mode, transform, mask.clone());
        Some(mask)
    }

    fn lookup_cached_mask(
        &self,
        scene: &Scene,
        mode: MaskMode,
        transform: Affine,
    ) -> Option<vello_cpu::Mask> {
        self.mask_cache
            .iter()
            .find(|entry| {
                entry.mode == mode && entry.transform == transform && entry.scene == *scene
            })
            .map(|entry| entry.mask.clone())
    }

    fn store_cached_mask(
        &mut self,
        scene: &Scene,
        mode: MaskMode,
        transform: Affine,
        mask: vello_cpu::Mask,
    ) {
        // TODO: If more backends end up wanting realized-mask caches, add a portable scene/cache
        // key at the imaging layer instead of retaining whole scenes in backend-local caches.
        self.mask_cache.push_back(CachedMask {
            scene: scene.clone(),
            mode,
            transform,
            mask,
        });
    }
}

impl ImageRenderer for VelloCpuRenderer {
    fn supported_image_formats(&self) -> Vec<ImageBufferFormat> {
        vec![ImageBufferFormat::Rgba8Unorm]
    }

    fn render_source_into(
        &mut self,
        source: &mut dyn RenderSource,
        target: ImageBufferTarget<'_>,
    ) -> Result<(), ImageRendererError> {
        if target.format != ImageBufferFormat::Rgba8Unorm {
            return Err(ImageRendererError::Target(
                ImageTargetError::UnsupportedTargetFormat,
            ));
        }
        let (width, height) =
            Self::checked_size(target.width, target.height).map_err(map_image_renderer_error)?;
        source
            .validate()
            .map_err(Error::InvalidScene)
            .map_err(map_image_renderer_error)?;
        self.resize(width, height);
        self.reset();
        source.paint_into(self);
        self.finish_into_target(target)
            .map_err(map_image_renderer_error)
    }
}

fn map_image_renderer_error(error: Error) -> ImageRendererError {
    match error {
        Error::InvalidScene(error) => {
            ImageRendererError::Content(RenderContentError::InvalidScene(error))
        }
        Error::Internal("image target dimensions do not match renderer output") => {
            ImageRendererError::Target(ImageTargetError::InvalidTarget(
                "image target dimensions do not match renderer output",
            ))
        }
        Error::Internal("image target row stride must be tightly packed") => {
            ImageRendererError::Target(ImageTargetError::InvalidTarget(
                "image target row stride must be tightly packed",
            ))
        }
        Error::Internal("image target buffer is too small") => {
            ImageRendererError::Target(ImageTargetError::InvalidTargetBuffer)
        }
        Error::Internal("render width too large" | "render height too large") => {
            ImageRendererError::Target(ImageTargetError::DimensionsTooLarge)
        }
        other => ImageRendererError::backend(other),
    }
}

#[inline]
#[allow(
    clippy::cast_possible_truncation,
    reason = "Backend API consumes f32 blur parameters; truncation from finite f64 IR values is acceptable."
)]
fn f64_to_f32(v: f64) -> f32 {
    v as f32
}

fn unpremultiply_rgba8_in_place(bytes: &mut [u8]) {
    for rgba in bytes.chunks_exact_mut(4) {
        let alpha = rgba[3];
        if alpha == 0 || alpha == u8::MAX {
            continue;
        }
        rgba[0] = unpremultiply_channel(rgba[0], alpha);
        rgba[1] = unpremultiply_channel(rgba[1], alpha);
        rgba[2] = unpremultiply_channel(rgba[2], alpha);
    }
}

fn unpremultiply_channel(channel: u8, alpha: u8) -> u8 {
    if alpha == 0 {
        return 0;
    }
    if alpha == u8::MAX {
        return channel;
    }

    let value = (u32::from(channel) * 255 + u32::from(alpha) / 2) / u32::from(alpha);
    u8::try_from(value.min(u32::from(u8::MAX))).expect("unpremultiplied channel must fit in u8")
}

impl PaintSink for VelloCpuRenderer {
    fn push_clip(&mut self, clip: ClipRef<'_>) {
        if self.error.is_some() {
            return;
        }
        let (xf, path, fill_rule) = self.clip_to_path(clip);
        self.ctx.set_transform(xf);
        self.ctx.set_fill_rule(fill_rule);
        self.ctx.push_clip_path(&path);
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
        self.ctx.pop_clip_path();
        self.clip_depth -= 1;
    }

    fn push_group(&mut self, group: GroupRef<'_>) {
        if self.error.is_some() {
            return;
        }
        let (clip_path, clip_transform) = match group.clip {
            None => (None, Affine::IDENTITY),
            Some(clip) => {
                let (xf, path, fill_rule) = self.clip_to_path(clip);
                self.ctx.set_fill_rule(fill_rule);
                (Some(path), xf)
            }
        };

        self.ctx.set_transform(clip_transform);

        let blend: Option<BlendMode> = Some(group.composite.blend);
        let opacity: Option<f32> = Some(group.composite.alpha);
        let mask = group
            .mask
            .and_then(|mask| self.render_mask(mask.mask.scene, mask.mask.mode, mask.transform));
        let filter = self.filters_to_vello(group.filters);
        self.ctx
            .push_layer(clip_path.as_ref(), blend, opacity, mask, filter);
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
        self.ctx.pop_layer();
        self.group_depth -= 1;
    }

    fn fill(&mut self, draw: FillRef<'_>) {
        if self.error.is_some() {
            return;
        }

        let Some(paint) = self.brush_to_paint(draw.brush, draw.composite) else {
            return;
        };
        self.ctx.set_transform(draw.transform);
        self.ctx.set_fill_rule(draw.fill_rule);
        self.ctx
            .set_paint_transform(draw.brush_transform.unwrap_or(Affine::IDENTITY));
        self.ctx.set_blend_mode(draw.composite.blend);
        self.ctx.set_paint(paint);

        match draw.shape {
            GeometryRef::Rect(r) => {
                if self.rect_fits_viewport_fast_path(&r, draw.transform) {
                    self.ctx.fill_rect(&r);
                } else {
                    // Work around a vello_cpu 0.0.7 rect fast-path panic for strips that land
                    // outside the viewport. Fall back to path filling until the upstream fix lands.
                    let path = r.to_path(self.tolerance);
                    self.ctx.fill_path(&path);
                }
            }
            GeometryRef::RoundedRect(rr) => {
                let path = rr.to_path(self.tolerance);
                self.ctx.fill_path(&path);
            }
            GeometryRef::Path(p) => self.ctx.fill_path(p),
            GeometryRef::OwnedPath(p) => self.ctx.fill_path(&p),
        }
    }

    fn stroke(&mut self, draw: StrokeRef<'_>) {
        if self.error.is_some() {
            return;
        }

        let Some(paint) = self.brush_to_paint(draw.brush, draw.composite) else {
            return;
        };
        self.ctx.set_transform(draw.transform);
        self.ctx.set_stroke(draw.stroke.clone());
        self.ctx
            .set_paint_transform(draw.brush_transform.unwrap_or(Affine::IDENTITY));
        self.ctx.set_blend_mode(draw.composite.blend);
        self.ctx.set_paint(paint);

        match draw.shape {
            GeometryRef::Rect(r) => self.ctx.stroke_rect(&r),
            GeometryRef::RoundedRect(rr) => {
                let path = rr.to_path(self.tolerance);
                self.ctx.stroke_path(&path);
            }
            GeometryRef::Path(p) => self.ctx.stroke_path(p),
            GeometryRef::OwnedPath(p) => self.ctx.stroke_path(&p),
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
    use imaging::{
        Painter,
        render::{ImageBufferTarget, ImageTargetError},
    };
    use kurbo::Rect;
    use peniko::Color;

    fn masked_scene(mode: MaskMode) -> Scene {
        let mut mask = Scene::new();
        {
            let mut painter = Painter::new(&mut mask);
            painter
                .fill(
                    Rect::new(8.0, 8.0, 56.0, 56.0),
                    Color::from_rgba8(255, 255, 255, 160),
                )
                .draw();
        }

        let mut content = Scene::new();
        {
            let mut painter = Painter::new(&mut content);
            painter
                .fill(
                    Rect::new(0.0, 0.0, 64.0, 64.0),
                    Color::from_rgb8(0x2a, 0x6f, 0xdb),
                )
                .draw();
        }

        let mut scene = Scene::new();
        let mask_id = scene.define_mask(imaging::record::Mask::new(mode, mask));
        let group = imaging::record::Group {
            mask: Some(imaging::record::AppliedMask::new(mask_id)),
            ..imaging::record::Group::default()
        };
        scene.push_group(group);
        replay(&content, &mut scene);
        scene.pop_group();
        scene
    }

    #[test]
    fn render_scene_reuses_cached_masks_for_identical_scenes() {
        let scene = masked_scene(MaskMode::Alpha);
        let mut renderer = VelloCpuRenderer::new(64, 64);

        renderer.render_scene(&scene, 64, 64).unwrap();
        assert_eq!(renderer.mask_cache.len(), 1);

        renderer.render_scene(&scene, 64, 64).unwrap();
        assert_eq!(renderer.mask_cache.len(), 1);
    }

    #[test]
    fn clear_cached_masks_drops_realized_masks() {
        let scene = masked_scene(MaskMode::Luminance);
        let mut renderer = VelloCpuRenderer::new(64, 64);

        renderer.render_scene(&scene, 64, 64).unwrap();
        assert_eq!(renderer.mask_cache.len(), 1);

        renderer.clear_cached_masks();
        assert!(renderer.mask_cache.is_empty());

        renderer.render_scene(&scene, 64, 64).unwrap();
        assert_eq!(renderer.mask_cache.len(), 1);
    }

    #[test]
    fn changing_tolerance_clears_cached_masks() {
        let scene = masked_scene(MaskMode::Alpha);
        let mut renderer = VelloCpuRenderer::new(64, 64);

        renderer.render_scene(&scene, 64, 64).unwrap();
        assert_eq!(renderer.mask_cache.len(), 1);

        renderer.set_tolerance(0.25);
        assert!(renderer.mask_cache.is_empty());
    }

    #[test]
    fn render_scene_handles_rects_below_viewport_without_panicking() {
        let mut scene = Scene::new();
        {
            let mut painter = Painter::new(&mut scene);
            painter
                .fill(
                    Rect::new(8.0, 48.0, 56.0, 52.0),
                    Color::from_rgba8(0x14, 0x50, 0xc8, 0xff),
                )
                .transform(Affine::translate((0.0, 24.0)))
                .draw();
        }

        let mut renderer = VelloCpuRenderer::new(64, 64);
        let image = renderer.render_scene(&scene, 64, 64).unwrap();
        assert_eq!(image.data.len(), 64 * 64 * 4);
    }

    #[test]
    fn render_scene_renders_image() {
        let mut renderer = VelloCpuRenderer::new(64, 64);
        let mut scene = Scene::new();
        {
            let mut painter = Painter::new(&mut scene);
            painter
                .fill(
                    Rect::new(0.0, 0.0, 64.0, 64.0),
                    Color::from_rgb8(0x2a, 0x6f, 0xdb),
                )
                .draw();
        }
        let image = renderer.render_scene(&scene, 64, 64).unwrap();
        assert_eq!(image.width, 64);
        assert_eq!(image.height, 64);
    }

    #[test]
    fn render_source_renders_image() {
        let mut renderer = VelloCpuRenderer::new(48, 48);
        let mut scene = Scene::new();
        {
            let mut painter = Painter::new(&mut scene);
            painter
                .fill(
                    Rect::new(0.0, 0.0, 48.0, 48.0),
                    Color::from_rgb8(0x2a, 0x6f, 0xdb),
                )
                .draw();
        }

        let mut source = &scene;
        let image = renderer.render_source(&mut source, 48, 48).unwrap();
        assert_eq!(image.width, 48);
        assert_eq!(image.height, 48);
    }

    #[test]
    fn render_source_into_rejects_short_row_stride_as_target_error() {
        let mut renderer = VelloCpuRenderer::new(4, 4);
        let mut scene = Scene::new();
        {
            let mut painter = Painter::new(&mut scene);
            painter
                .fill(
                    Rect::new(0.0, 0.0, 4.0, 4.0),
                    Color::from_rgb8(0x2a, 0x6f, 0xdb),
                )
                .draw();
        }

        let mut data = vec![0; 4 * 4 * 4];
        let mut source = &scene;
        let error = ImageRenderer::render_source_into(
            &mut renderer,
            &mut source,
            ImageBufferTarget {
                data: &mut data,
                width: 4,
                height: 4,
                bytes_per_row: 12,
                format: ImageBufferFormat::Rgba8Unorm,
            },
        )
        .unwrap_err();

        assert!(matches!(
            error,
            ImageRendererError::Target(ImageTargetError::InvalidTarget(
                "image target row stride must be tightly packed",
            ))
        ));
    }

    #[test]
    fn render_source_into_rejects_short_buffer_as_target_error() {
        let mut renderer = VelloCpuRenderer::new(4, 4);
        let mut scene = Scene::new();
        {
            let mut painter = Painter::new(&mut scene);
            painter
                .fill(
                    Rect::new(0.0, 0.0, 4.0, 4.0),
                    Color::from_rgb8(0x2a, 0x6f, 0xdb),
                )
                .draw();
        }

        let mut data = vec![0; 15];
        let mut source = &scene;
        let error = ImageRenderer::render_source_into(
            &mut renderer,
            &mut source,
            ImageBufferTarget {
                data: &mut data,
                width: 4,
                height: 4,
                bytes_per_row: 16,
                format: ImageBufferFormat::Rgba8Unorm,
            },
        )
        .unwrap_err();

        assert!(matches!(
            error,
            ImageRendererError::Target(ImageTargetError::InvalidTargetBuffer)
        ));
    }
}
