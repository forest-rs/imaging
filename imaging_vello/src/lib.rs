// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Vello backend for `imaging`.
//!
//! This crate provides a headless CPU/GPU renderer that consumes `imaging::record::Scene` (or
//! accepts commands directly via `imaging::PaintSink`) and produces an RGBA8 image buffer using
//! `vello` + `wgpu`.

#![deny(unsafe_code)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use imaging::{
    BlurredRoundedRect, ClipRef, Composite, FillRef, GeometryRef, GlyphRunRef, GroupRef, PaintSink,
    StrokeRef,
    record::{Scene, ValidateError, replay},
};
use kurbo::{Affine, Rect};
use peniko::{Brush, Fill};
use std::sync::mpsc;
use vello::wgpu;
use vello::{AaConfig, Glyph as VelloGlyph, RenderParams};

/// Errors that can occur when rendering via Vello.
#[derive(Debug)]
pub enum Error {
    /// The scene is invalid (unbalanced stacks).
    InvalidScene(ValidateError),
    /// An image brush was encountered; this backend does not support it.
    UnsupportedImageBrush,
    /// A filter configuration could not be translated.
    UnsupportedFilter,
    /// Glyph draws with non-default blend modes are not supported by this backend yet.
    UnsupportedGlyphBlend,
    /// Blurred rounded rect draws with non-default blend modes are not supported by this backend yet.
    UnsupportedBlurredRoundedRectBlend,
    /// The clip/group stack was not well-nested for this backend.
    ///
    /// Vello uses a single layer stack for both clipping and blending; `imaging` tracks these as
    /// separate stacks, so scenes that interleave them (e.g. `push_clip`, `push_group`, `pop_clip`)
    /// cannot be represented directly.
    UnbalancedLayerStack,
    /// No suitable GPU adapter was found.
    NoAdapter,
    /// A GPU device could not be created.
    RequestDevice,
    /// Vello returned a render error.
    Render(vello::Error),
    /// An internal invariant was violated.
    Internal(&'static str),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum LayerKind {
    Clip,
    Group,
}

/// Renderer that executes `imaging` commands using `vello` + `wgpu`.
pub struct VelloRenderer {
    scene: vello::Scene,
    renderer: vello::Renderer,

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
    layer_stack: Vec<LayerKind>,
}

impl core::fmt::Debug for VelloRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VelloRenderer")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("tolerance", &self.tolerance)
            .field("error", &self.error)
            .field("layer_stack_depth", &self.layer_stack.len())
            .finish_non_exhaustive()
    }
}

impl VelloRenderer {
    /// Create a renderer for a fixed-size target.
    pub fn new(width: u16, height: u16) -> Self {
        Self::try_new(width, height).expect("create imaging_vello renderer")
    }

    /// Create a renderer for a fixed-size target.
    ///
    /// This is fallible because `wgpu` may not be able to find a compatible adapter/device
    /// in some sandboxed or headless environments.
    pub fn try_new(width: u16, height: u16) -> Result<Self, Error> {
        let (device, queue) = pollster::block_on(init_device_and_queue())?;
        let (texture, texture_view, readback, bytes_per_row) =
            create_targets(&device, width, height);

        let renderer = vello::Renderer::new(&device, vello::RendererOptions::default())
            .map_err(Error::Render)?;

        Ok(Self {
            scene: vello::Scene::new(),
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
            layer_stack: Vec::new(),
        })
    }

    /// Set the curve flattening tolerance used when converting rounded rects to paths.
    pub fn set_tolerance(&mut self, tolerance: f64) {
        self.tolerance = tolerance;
    }

    /// Reset the internal Vello scene and state.
    pub fn reset(&mut self) {
        self.scene.reset();
        self.error = None;
        self.layer_stack.clear();
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
        if !self.layer_stack.is_empty() {
            return Err(Error::Internal("unbalanced layer stack"));
        }

        let params = RenderParams {
            base_color: peniko::Color::from_rgba8(0, 0, 0, 0),
            width: u32::from(self.width),
            height: u32::from(self.height),
            antialiasing_method: AaConfig::Area,
        };

        self.renderer
            .render_to_texture(
                &self.device,
                &self.queue,
                &self.scene,
                &self.texture_view,
                &params,
            )
            .map_err(Error::Render)?;

        readback_rgba8(
            &self.device,
            &self.queue,
            &self.texture,
            &self.readback,
            self.bytes_per_row,
            self.width,
            self.height,
        )
    }

    fn set_error_once(&mut self, err: Error) {
        if self.error.is_none() {
            self.error = Some(err);
        }
    }

    fn brush_to_brush(&mut self, brush: &Brush, composite: Composite) -> Option<Brush> {
        let brush = brush.clone().multiply_alpha(composite.alpha);
        match brush {
            Brush::Solid(_) | Brush::Gradient(_) | Brush::Image(_) => Some(brush),
        }
    }

    fn surface_clip(&self) -> Rect {
        Rect::new(0.0, 0.0, f64::from(self.width), f64::from(self.height))
    }

    fn push_layer_kind(&mut self, kind: LayerKind) {
        self.layer_stack.push(kind);
    }

    fn pop_layer_kind(&mut self, expected: LayerKind) -> bool {
        match self.layer_stack.pop() {
            Some(kind) if kind == expected => true,
            _ => {
                self.set_error_once(Error::UnbalancedLayerStack);
                false
            }
        }
    }

    fn draw_glyph_run(&mut self, glyph_run: GlyphRunRef<'_>) {
        if glyph_run.composite.blend != peniko::BlendMode::default() {
            self.set_error_once(Error::UnsupportedGlyphBlend);
            return;
        }

        let Some(paint) = self.brush_to_brush(glyph_run.paint, glyph_run.composite) else {
            return;
        };

        let builder = self
            .scene
            .draw_glyphs(glyph_run.font)
            .transform(glyph_run.transform)
            .font_size(glyph_run.font_size)
            .hint(glyph_run.hint)
            .normalized_coords(glyph_run.normalized_coords)
            .brush(&paint)
            .brush_alpha(glyph_run.composite.alpha);
        let builder = builder.glyph_transform(glyph_run.glyph_transform);
        let glyphs = glyph_run.glyphs.iter().map(|glyph| VelloGlyph {
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
}

impl PaintSink for VelloRenderer {
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
        self.push_layer_kind(LayerKind::Clip);
    }

    fn pop_clip(&mut self) {
        if self.error.is_some() {
            return;
        }
        if !self.pop_layer_kind(LayerKind::Clip) {
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
            let clip_box = self.surface_clip();
            self.scene.push_layer(
                Fill::NonZero,
                group.composite.blend,
                group.composite.alpha,
                Affine::IDENTITY,
                &clip_box,
            );
        }
        self.push_layer_kind(LayerKind::Group);
    }

    fn pop_group(&mut self) {
        if self.error.is_some() {
            return;
        }
        if !self.pop_layer_kind(LayerKind::Group) {
            return;
        }
        self.scene.pop_layer();
    }

    fn fill(&mut self, draw: FillRef<'_>) {
        if self.error.is_some() {
            return;
        }

        let Some(paint) = self.brush_to_brush(draw.paint, draw.composite) else {
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
            self.push_layer_kind(LayerKind::Group);
        }

        match draw.shape {
            GeometryRef::Rect(r) => {
                self.scene.fill(
                    draw.fill_rule,
                    draw.transform,
                    &paint,
                    draw.paint_transform,
                    &r,
                );
            }
            GeometryRef::RoundedRect(rr) => {
                self.scene.fill(
                    draw.fill_rule,
                    draw.transform,
                    &paint,
                    draw.paint_transform,
                    &rr,
                );
            }
            GeometryRef::Path(p) => {
                self.scene.fill(
                    draw.fill_rule,
                    draw.transform,
                    &paint,
                    draw.paint_transform,
                    p,
                );
            }
            GeometryRef::OwnedPath(p) => {
                self.scene.fill(
                    draw.fill_rule,
                    draw.transform,
                    &paint,
                    draw.paint_transform,
                    &p,
                );
            }
        }

        if blend != peniko::BlendMode::default() {
            if !self.pop_layer_kind(LayerKind::Group) {
                return;
            }
            self.scene.pop_layer();
        }
    }

    fn stroke(&mut self, draw: StrokeRef<'_>) {
        if self.error.is_some() {
            return;
        }

        let Some(paint) = self.brush_to_brush(draw.paint, draw.composite) else {
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
            self.push_layer_kind(LayerKind::Group);
        }

        match draw.shape {
            GeometryRef::Rect(r) => {
                self.scene.stroke(
                    draw.stroke,
                    draw.transform,
                    &paint,
                    draw.paint_transform,
                    &r,
                );
            }
            GeometryRef::RoundedRect(rr) => {
                self.scene.stroke(
                    draw.stroke,
                    draw.transform,
                    &paint,
                    draw.paint_transform,
                    &rr,
                );
            }
            GeometryRef::Path(p) => {
                self.scene
                    .stroke(draw.stroke, draw.transform, &paint, draw.paint_transform, p);
            }
            GeometryRef::OwnedPath(p) => {
                self.scene.stroke(
                    draw.stroke,
                    draw.transform,
                    &paint,
                    draw.paint_transform,
                    &p,
                );
            }
        }

        if blend != peniko::BlendMode::default() {
            if !self.pop_layer_kind(LayerKind::Group) {
                return;
            }
            self.scene.pop_layer();
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
            label: Some("imaging_vello device"),
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
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("imaging_vello render target"),
        size: wgpu::Extent3d {
            width: u32::from(width),
            height: u32::from(height),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let bytes_per_row = u32::from(width) * 4;
    let padded_bytes_per_row = bytes_per_row.div_ceil(256) * 256;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("imaging_vello readback"),
        size: u64::from(padded_bytes_per_row) * u64::from(height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    (texture, texture_view, readback, padded_bytes_per_row)
}

fn readback_rgba8(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    readback: &wgpu::Buffer,
    bytes_per_row: u32,
    width: u16,
    height: u16,
) -> Result<Vec<u8>, Error> {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("imaging_vello readback"),
    });

    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: u32::from(width),
            height: u32::from(height),
            depth_or_array_layers: 1,
        },
    );

    queue.submit([encoder.finish()]);

    let slice = readback.slice(..);
    let (tx, rx) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|_| Error::Internal("device poll failed"))?;
    rx.recv()
        .map_err(|_| Error::Internal("map_async callback dropped"))?
        .map_err(|_| Error::Internal("buffer map failed"))?;

    let mapped = slice.get_mapped_range();
    let width_bytes = usize::from(width) * 4;

    let mut out = Vec::with_capacity(usize::from(width) * usize::from(height) * 4);
    for row in mapped.chunks_exact(bytes_per_row as usize) {
        out.extend_from_slice(&row[..width_bytes]);
    }
    drop(mapped);
    readback.unmap();
    Ok(out)
}
