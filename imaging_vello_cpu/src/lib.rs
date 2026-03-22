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
//!     let rgba = renderer.render_scene_rgba8(&scene)?;
//!     assert_eq!(rgba.len(), 128 * 128 * 4);
//!     Ok(())
//! }
//! ```
//!
//! # Stream Commands Directly
//!
//! [`VelloCpuRenderer`] also implements [`imaging::PaintSink`], so you can stream commands
//! directly and call [`VelloCpuRenderer::finish_rgba8`] when the frame is complete.
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
//!     let rgba = renderer.finish_rgba8()?;
//!     assert_eq!(rgba.len(), 128 * 128 * 4);
//!     Ok(())
//! }
//! ```

#![deny(unsafe_code)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use imaging::{
    BlurredRoundedRect, ClipRef, Composite, FillRef, Filter, GeometryRef, GlyphRunRef, GroupRef,
    PaintSink, StrokeRef,
    record::{Scene, ValidateError, replay},
};
use kurbo::{Affine, Shape as _};
use peniko::{BlendMode, Brush, BrushRef, Fill, Style};
use std::vec::Vec;
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
    /// An image brush was encountered; this backend does not support it.
    UnsupportedImageBrush,
    /// A filter configuration could not be translated.
    UnsupportedFilter,
    /// An internal invariant was violated.
    Internal(&'static str),
}

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
}

impl VelloCpuRenderer {
    /// Create a renderer for a fixed-size target.
    ///
    /// The renderer uses Vello CPU's `OptimizeSpeed` mode by default to keep snapshots stable.
    pub fn new(width: u16, height: u16) -> Self {
        let settings = RenderSettings {
            render_mode: RenderMode::OptimizeSpeed,
            ..RenderSettings::default()
        };
        let ctx = RenderContext::new_with(width, height, settings);
        Self {
            ctx,
            width,
            height,
            tolerance: 0.1,
            error: None,
            clip_depth: 0,
            group_depth: 0,
        }
    }

    /// Set the tolerance used when converting shapes to paths.
    pub fn set_tolerance(&mut self, tolerance: f64) {
        self.tolerance = tolerance;
    }

    /// Reset the internal Vello CPU context and local error state.
    pub fn reset(&mut self) {
        self.ctx.reset();
        self.error = None;
        self.clip_depth = 0;
        self.group_depth = 0;
    }

    /// Render a recorded scene and return an RGBA8 buffer (unpremultiplied).
    pub fn render_scene_rgba8(&mut self, scene: &Scene) -> Result<Vec<u8>, Error> {
        scene.validate().map_err(Error::InvalidScene)?;
        self.reset();
        replay(scene, self);
        self.finish_rgba8()
    }

    /// Finish rendering the current command stream and return an RGBA8 buffer (unpremultiplied).
    pub fn finish_rgba8(&mut self) -> Result<Vec<u8>, Error> {
        if let Some(err) = self.error.take() {
            return Err(err);
        }
        if self.clip_depth != 0 {
            return Err(Error::Internal("unbalanced clip stack"));
        }
        if self.group_depth != 0 {
            return Err(Error::Internal("unbalanced group stack"));
        }

        let mut pixmap = Pixmap::new(self.width, self.height);
        self.ctx.flush();
        self.ctx.render_to_pixmap(&mut pixmap);

        let unpremul = pixmap.take_unpremultiplied();
        let mut bytes = Vec::with_capacity(unpremul.len() * 4);
        for p in unpremul {
            bytes.extend_from_slice(&[p.r, p.g, p.b, p.a]);
        }
        Ok(bytes)
    }

    fn set_error_once(&mut self, err: Error) {
        if self.error.is_none() {
            self.error = Some(err);
        }
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
        } else {
            self.set_error_once(Error::UnsupportedFilter);
            return None;
        }
        Some(VelloFilter {
            graph: std::sync::Arc::new(graph),
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
}

#[inline]
#[allow(
    clippy::cast_possible_truncation,
    reason = "Backend API consumes f32 blur parameters; truncation from finite f64 IR values is acceptable."
)]
fn f64_to_f32(v: f64) -> f32 {
    v as f32
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
        let filter = self.filters_to_vello(group.filters);
        self.ctx
            .push_layer(clip_path.as_ref(), blend, opacity, None, filter);
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
            GeometryRef::Rect(r) => self.ctx.fill_rect(&r),
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
