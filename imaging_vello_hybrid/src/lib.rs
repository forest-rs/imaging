// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Vello hybrid backend for `imaging`.
//!
//! This crate provides a headless CPU/GPU renderer that consumes `imaging::Scene` (or accepts
//! commands directly via `imaging::PaintSink`) and produces an RGBA8 image buffer using
//! `vello_hybrid` + `wgpu`.

#![deny(unsafe_code)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use imaging::{
    BlurredRoundedRect, ClipRef, Composite, FillRef, GeometryRef, GlyphRunRef, GroupRef, PaintSink,
    Scene, StrokeRef, replay,
};
use kurbo::{Affine, Shape as _};
use peniko::{Brush, Style};
use std::sync::mpsc;
use vello_common::glyph::Glyph as VelloGlyph;
use vello_hybrid::{RenderError, RenderSize, RenderTargetConfig};
use wgpu::{
    CommandEncoderDescriptor, Extent3d, TextureDescriptor, TextureDimension, TextureFormat,
};

/// Errors that can occur when rendering via Vello hybrid.
#[derive(Debug)]
pub enum Error {
    /// The scene is invalid (unbalanced stacks).
    InvalidScene(imaging::ValidateError),
    /// An image brush was encountered; this backend does not support it.
    UnsupportedImageBrush,
    /// A filter configuration could not be translated.
    UnsupportedFilter,
    /// Blurred rounded rect draws are not supported by this backend yet.
    UnsupportedBlurredRoundedRect,
    /// No suitable GPU adapter was found.
    NoAdapter,
    /// A GPU device could not be created.
    RequestDevice,
    /// Vello hybrid returned a render error.
    Render(RenderError),
    /// An internal invariant was violated.
    Internal(&'static str),
}

/// Renderer that executes `imaging` commands using `vello_hybrid` + `wgpu`.
#[derive(Debug)]
pub struct VelloHybridRenderer {
    scene: vello_hybrid::Scene,
    renderer: vello_hybrid::Renderer,

    device: wgpu::Device,
    queue: wgpu::Queue,

    texture: wgpu::Texture,
    texture_view: wgpu::TextureView,
    readback: wgpu::Buffer,
    bytes_per_row: u32,

    width: u16,
    height: u16,
    tolerance: f64,
    error: Option<Error>,
    clip_depth: u32,
    group_depth: u32,
}

impl VelloHybridRenderer {
    /// Create a renderer for a fixed-size target.
    pub fn new(width: u16, height: u16) -> Self {
        Self::try_new(width, height).expect("create imaging_vello_hybrid renderer")
    }

    /// Create a renderer for a fixed-size target.
    ///
    /// This is fallible because `wgpu` may not be able to find a compatible adapter/device
    /// in some sandboxed or headless environments.
    pub fn try_new(width: u16, height: u16) -> Result<Self, Error> {
        let (device, queue) = pollster::block_on(init_device_and_queue())?;
        let (texture, texture_view, readback, bytes_per_row) =
            create_targets(&device, width, height);

        let mut scene = vello_hybrid::Scene::new(width, height);
        scene.reset();

        let renderer = vello_hybrid::Renderer::new(
            &device,
            &RenderTargetConfig {
                format: TextureFormat::Rgba8Unorm,
                width: u32::from(width),
                height: u32::from(height),
            },
        );

        Ok(Self {
            scene,
            renderer,
            device,
            queue,
            texture,
            texture_view,
            readback,
            bytes_per_row,
            width,
            height,
            tolerance: 0.1,
            error: None,
            clip_depth: 0,
            group_depth: 0,
        })
    }

    /// Set the tolerance used when converting shapes to paths.
    pub fn set_tolerance(&mut self, tolerance: f64) {
        self.tolerance = tolerance;
    }

    /// Reset the internal scene and local error state.
    pub fn reset(&mut self) {
        self.scene.reset();
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

        let render_size = RenderSize {
            width: u32::from(self.width),
            height: u32::from(self.height),
        };
        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("imaging_vello_hybrid render"),
            });

        self.renderer
            .render(
                &self.scene,
                &self.device,
                &self.queue,
                &mut encoder,
                &render_size,
                &self.texture_view,
            )
            .map_err(Error::Render)?;

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.bytes_per_row),
                    rows_per_image: None,
                },
            },
            Extent3d {
                width: u32::from(self.width),
                height: u32::from(self.height),
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit([encoder.finish()]);

        let slice = self.readback.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|_| Error::Internal("device poll failed"))?;
        rx.recv()
            .map_err(|_| Error::Internal("map_async callback dropped"))?
            .map_err(|_| Error::Internal("buffer map failed"))?;

        let mapped = slice.get_mapped_range();
        let width_bytes = usize::from(self.width) * 4;
        let mut pixels = Vec::with_capacity(usize::from(self.width) * usize::from(self.height));
        for row in mapped.chunks_exact(self.bytes_per_row as usize) {
            for px in row[..width_bytes].chunks_exact(4) {
                pixels.push(peniko::color::PremulRgba8::from_u8_array([
                    px[0], px[1], px[2], px[3],
                ]));
            }
        }
        drop(mapped);
        self.readback.unmap();

        let pixmap = vello_common::pixmap::Pixmap::from_parts(pixels, self.width, self.height);
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
        brush: &Brush,
        composite: Composite,
    ) -> Option<vello_common::paint::PaintType> {
        let brush = brush.clone().multiply_alpha(composite.alpha);
        match brush {
            Brush::Solid(c) => Some(Brush::Solid(c)),
            Brush::Gradient(g) => Some(Brush::Gradient(g)),
            Brush::Image(_) => {
                self.set_error_once(Error::UnsupportedImageBrush);
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
        let Some(paint) = self.brush_to_paint(glyph_run.paint, glyph_run.composite) else {
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

impl PaintSink for VelloHybridRenderer {
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
            // vello_hybrid does not support filter layers yet.
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

        let Some(paint) = self.brush_to_paint(draw.paint, draw.composite) else {
            return;
        };
        self.scene.set_transform(draw.transform);
        self.scene.set_fill_rule(draw.fill_rule);
        self.scene
            .set_paint_transform(draw.paint_transform.unwrap_or(Affine::IDENTITY));

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

        let Some(paint) = self.brush_to_paint(draw.paint, draw.composite) else {
            return;
        };
        self.scene.set_transform(draw.transform);
        self.scene.set_stroke(draw.stroke.clone());
        self.scene
            .set_paint_transform(draw.paint_transform.unwrap_or(Affine::IDENTITY));

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

async fn init_device_and_queue() -> Result<(wgpu::Device, wgpu::Queue), Error> {
    let instance = wgpu::Instance::default();
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            force_fallback_adapter: false,
            compatible_surface: None,
        })
        .await
        .map_err(|_| Error::NoAdapter)?;

    adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("imaging_vello_hybrid device"),
            required_features: wgpu::Features::empty(),
            ..Default::default()
        })
        .await
        .map_err(|_| Error::RequestDevice)
}

fn create_targets(
    device: &wgpu::Device,
    width: u16,
    height: u16,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::Buffer, u32) {
    let texture = device.create_texture(&TextureDescriptor {
        label: Some("imaging_vello_hybrid render target"),
        size: Extent3d {
            width: u32::from(width),
            height: u32::from(height),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let bytes_per_row = (u32::from(width) * 4).next_multiple_of(256);
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("imaging_vello_hybrid readback buffer"),
        size: u64::from(bytes_per_row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    (texture, texture_view, readback, bytes_per_row)
}
