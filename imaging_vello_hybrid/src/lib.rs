// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Vello hybrid backend for `imaging`.
//!
//! This crate provides a headless CPU/GPU renderer that consumes `imaging::record::Scene` or a
//! native [`vello_hybrid::Scene`] and produces an RGBA8 image buffer using `vello_hybrid` +
//! `wgpu`.
//!
//! Recorded scenes with inline image brushes are uploaded through a renderer-scoped image registry
//! and translated to backend-managed opaque image ids. Use [`VelloHybridSceneSink::with_renderer`]
//! when recording directly into a native [`vello_hybrid::Scene`] and you want the same image
//! support.
//!
//! # Render A Recorded Scene
//!
//! Record commands into [`imaging::record::Scene`], then render them with
//! [`VelloHybridRenderer`].
//!
//! ```no_run
//! use imaging::{Painter, record};
//! use imaging_vello_hybrid::VelloHybridRenderer;
//! use kurbo::Rect;
//! use peniko::{Brush, Color};
//!
//! fn main() -> Result<(), imaging_vello_hybrid::Error> {
//!     let paint = Brush::Solid(Color::from_rgb8(0x2a, 0x6f, 0xdb));
//!     let mut scene = record::Scene::new();
//!
//!     {
//!         let mut painter = Painter::new(&mut scene);
//!         painter.fill_rect(Rect::new(0.0, 0.0, 128.0, 128.0), &paint);
//!     }
//!
//!     let mut renderer = VelloHybridRenderer::try_new(128, 128)?;
//!     let rgba = renderer.render_scene_rgba8(&scene)?;
//!     assert_eq!(rgba.len(), 128 * 128 * 4);
//!     Ok(())
//! }
//! ```
//!
//! # Record Into `vello_hybrid::Scene`
//!
//! If you want a backend-native retained scene without owning a full renderer, wrap an existing
//! [`vello_hybrid::Scene`] with [`VelloHybridSceneSink`].
//!
//! ```no_run
//! use imaging::Painter;
//! use imaging_vello_hybrid::VelloHybridSceneSink;
//! use kurbo::Rect;
//! use peniko::{Brush, Color};
//!
//! fn main() -> Result<(), imaging_vello_hybrid::Error> {
//!     let paint = Brush::Solid(Color::from_rgb8(0x1d, 0x4e, 0x89));
//!     let mut scene = vello_hybrid::Scene::new(128, 128);
//!     scene.reset();
//!
//!     {
//!         let mut sink = VelloHybridSceneSink::new(&mut scene);
//!         let mut painter = Painter::new(&mut sink);
//!         painter.fill_rect(Rect::new(0.0, 0.0, 128.0, 128.0), &paint);
//!         sink.finish()?;
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! Use [`VelloHybridSceneSink::with_renderer`] instead when the scene uses image brushes.
//!
//! # Record Image Brushes Into `vello_hybrid::Scene`
//!
//! Use [`VelloHybridSceneSink::with_renderer`] when recording image brushes directly into a
//! native [`vello_hybrid::Scene`]. The sink uploads images through the renderer and reuses them
//! across later recordings and renders.
//!
//! ```no_run
//! use std::sync::Arc;
//!
//! use imaging::Painter;
//! use imaging_vello_hybrid::{VelloHybridRenderer, VelloHybridSceneSink};
//! use kurbo::Rect;
//! use peniko::{Blob, Brush, ImageAlphaType, ImageBrush, ImageData, ImageFormat};
//!
//! fn main() -> Result<(), imaging_vello_hybrid::Error> {
//!     let image = ImageData {
//!         data: Blob::new(Arc::new([
//!             0xff, 0x20, 0x20, 0xff, 0x20, 0xff, 0x20, 0xff, 0x20, 0x20, 0xff, 0xff, 0xff,
//!             0xff, 0x20, 0xff,
//!         ])),
//!         format: ImageFormat::Rgba8,
//!         alpha_type: ImageAlphaType::Alpha,
//!         width: 2,
//!         height: 2,
//!     };
//!     let brush = Brush::Image(ImageBrush::new(image));
//!
//!     let mut renderer = VelloHybridRenderer::try_new(128, 128)?;
//!     let mut scene = vello_hybrid::Scene::new(128, 128);
//!     scene.reset();
//!
//!     {
//!         let mut sink = VelloHybridSceneSink::with_renderer(&mut scene, &mut renderer);
//!         let mut painter = Painter::new(&mut sink);
//!         painter.fill_rect(Rect::new(0.0, 0.0, 128.0, 128.0), &brush);
//!         sink.finish()?;
//!     }
//!
//!     let rgba = renderer.render_vello_hybrid_scene_rgba8(&scene)?;
//!     assert_eq!(rgba.len(), 128 * 128 * 4);
//!     Ok(())
//! }
//! ```
//!
//! # Render A Native `vello_hybrid::Scene`
//!
//! If you already have a native hybrid scene, hand it directly to [`VelloHybridRenderer`].
//!
//! ```no_run
//! use imaging::Painter;
//! use imaging_vello_hybrid::{VelloHybridRenderer, VelloHybridSceneSink};
//! use kurbo::Rect;
//! use peniko::{Brush, Color};
//!
//! fn main() -> Result<(), imaging_vello_hybrid::Error> {
//!     let paint = Brush::Solid(Color::from_rgb8(0xd9, 0x77, 0x06));
//!     let mut scene = vello_hybrid::Scene::new(128, 128);
//!     scene.reset();
//!
//!     {
//!         let mut sink = VelloHybridSceneSink::new(&mut scene);
//!         let mut painter = Painter::new(&mut sink);
//!         painter.fill_rect(Rect::new(16.0, 16.0, 112.0, 112.0), &paint);
//!         sink.finish()?;
//!     }
//!
//!     let mut renderer = VelloHybridRenderer::try_new(128, 128)?;
//!     let rgba = renderer.render_vello_hybrid_scene_rgba8(&scene)?;
//!     assert_eq!(rgba.len(), 128 * 128 * 4);
//!     Ok(())
//! }
//! ```

#![deny(unsafe_code)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod image_registry;
mod scene_sink;

use image_registry::{HybridImageRegistry, HybridImageUploadSession};
use imaging::record::{Scene, ValidateError, replay};
use std::sync::mpsc;
use vello_hybrid::{RenderError, RenderSize, RenderTargetConfig};
use wgpu::{
    CommandEncoderDescriptor, Extent3d, TextureDescriptor, TextureDimension, TextureFormat,
};

pub use scene_sink::VelloHybridSceneSink;

/// Errors that can occur when rendering via Vello hybrid.
#[derive(Debug)]
pub enum Error {
    /// The scene is invalid (unbalanced stacks).
    InvalidScene(ValidateError),
    /// An image brush was encountered on a sink path that has no renderer-backed image resolver.
    UnsupportedImageBrush,
    /// A filter configuration could not be translated.
    UnsupportedFilter,
    /// Masks are not supported by this backend yet.
    UnsupportedMask,
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
    image_registry: HybridImageRegistry,
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

        let renderer = vello_hybrid::Renderer::new(
            &device,
            &RenderTargetConfig {
                format: TextureFormat::Rgba8Unorm,
                width: u32::from(width),
                height: u32::from(height),
            },
        );

        Ok(Self {
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
            image_registry: HybridImageRegistry::new(),
        })
    }

    /// Set the tolerance used when converting shapes to paths.
    pub fn set_tolerance(&mut self, tolerance: f64) {
        self.tolerance = tolerance;
    }

    pub(crate) fn begin_image_upload_session(
        &mut self,
        label: &'static str,
    ) -> HybridImageUploadSession<'_> {
        let encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor { label: Some(label) });
        self.image_registry.begin_upload_session(
            &mut self.renderer,
            &self.device,
            &self.queue,
            encoder,
        )
    }

    /// Destroy all uploaded hybrid image resources cached by this renderer.
    pub fn clear_cached_images(&mut self) {
        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("imaging_vello_hybrid clear cached images"),
            });
        self.image_registry
            .clear(&mut self.renderer, &self.device, &self.queue, &mut encoder);
        self.queue.submit([encoder.finish()]);
    }

    /// Render a recorded scene and return an RGBA8 buffer (unpremultiplied).
    ///
    /// Inline image brushes are uploaded on demand and cached for the lifetime of this renderer
    /// (or until [`Self::clear_cached_images`] is called).
    pub fn render_scene_rgba8(&mut self, scene: &Scene) -> Result<Vec<u8>, Error> {
        scene.validate().map_err(Error::InvalidScene)?;
        let mut native = vello_hybrid::Scene::new(self.width, self.height);
        native.reset();
        let tolerance = self.tolerance;
        {
            let mut sink = VelloHybridSceneSink::with_renderer(&mut native, self);
            sink.set_tolerance(tolerance);
            replay(scene, &mut sink);
            sink.finish()?;
        }
        self.render_vello_hybrid_scene_rgba8(&native)
    }

    /// Render a native [`vello_hybrid::Scene`] and return an RGBA8 buffer (unpremultiplied).
    pub fn render_vello_hybrid_scene_rgba8(
        &mut self,
        scene: &vello_hybrid::Scene,
    ) -> Result<Vec<u8>, Error> {
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
                scene,
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
