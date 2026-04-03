// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Vello hybrid backend for `imaging`.
//!
//! This crate provides a headless CPU/GPU renderer that consumes native
//! [`vello_hybrid::Scene`] values and renders them to GPU targets or RGBA8 image data using
//! `vello_hybrid` + `wgpu`.
//!
//! Semantic [`imaging::record::Scene`] values can be lowered to native hybrid scenes through
//! [`VelloHybridRenderer::encode_scene`].
//!
//! In UI integrations, the host application should usually own the `wgpu` device, queue, and
//! presentation targets, then pass those handles into [`VelloHybridRenderer`].
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
//!     # let device: imaging_vello_hybrid::wgpu::Device = todo!();
//!     # let queue: imaging_vello_hybrid::wgpu::Queue = todo!();
//!     let mut renderer = VelloHybridRenderer::new(device, queue);
//!     let native = renderer.encode_scene(&scene, 128, 128)?;
//!     let image = renderer.render(&native, 128, 128)?;
//!     assert_eq!(image.width, 128);
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
//!     # let device: imaging_vello_hybrid::wgpu::Device = todo!();
//!     # let queue: imaging_vello_hybrid::wgpu::Queue = todo!();
//!     let mut renderer = VelloHybridRenderer::new(device, queue);
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
//!     let image = renderer.render(&scene, 128, 128)?;
//!     assert_eq!(image.width, 128);
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
//!     # let device: imaging_vello_hybrid::wgpu::Device = todo!();
//!     # let queue: imaging_vello_hybrid::wgpu::Queue = todo!();
//!     let mut renderer = VelloHybridRenderer::new(device, queue);
//!     let image = renderer.render(&scene, 128, 128)?;
//!     assert_eq!(image.width, 128);
//!     Ok(())
//! }
//! ```

#![deny(unsafe_code)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod image_registry;
mod scene_sink;
mod wgpu_support;

use image_registry::{HybridImageRegistry, HybridImageUploadSession};
use imaging::RgbaImage;
use imaging::record::{Scene, ValidateError, replay};
use vello_hybrid::{RenderError, RenderSize, RenderTargetConfig};
pub use wgpu;
use wgpu::{CommandEncoderDescriptor, TextureFormat};

use crate::wgpu_support::{
    OffscreenTarget, ReadbackError, read_texture_into, unpremultiply_rgba8_in_place,
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
    target: OffscreenTarget,
    width: u16,
    height: u16,
    tolerance: f64,
    image_registry: HybridImageRegistry,
}

impl VelloHybridRenderer {
    /// Create a renderer bound to an existing `wgpu` device and queue.
    #[must_use]
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let target = OffscreenTarget::new(&device, 1, 1);

        let renderer = vello_hybrid::Renderer::new(
            &device,
            &RenderTargetConfig {
                format: TextureFormat::Rgba8Unorm,
                width: 1,
                height: 1,
            },
        );

        Self {
            renderer,
            device,
            queue,
            target,
            width: 1,
            height: 1,
            tolerance: 0.1,
            image_registry: HybridImageRegistry::new(),
        }
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

    /// Lower a semantic [`imaging::record::Scene`] into a native [`vello_hybrid::Scene`].
    pub fn encode_scene(
        &mut self,
        scene: &Scene,
        width: u16,
        height: u16,
    ) -> Result<vello_hybrid::Scene, Error> {
        scene.validate().map_err(Error::InvalidScene)?;
        self.resize_target(width, height);
        let mut native = vello_hybrid::Scene::new(width, height);
        native.reset();
        let tolerance = self.tolerance;
        {
            let mut sink = VelloHybridSceneSink::with_renderer(&mut native, self);
            sink.set_tolerance(tolerance);
            replay(scene, &mut sink);
            sink.finish()?;
        }
        Ok(native)
    }

    /// Render a native [`vello_hybrid::Scene`] into an RGBA8 image (unpremultiplied).
    pub fn render_into(
        &mut self,
        scene: &vello_hybrid::Scene,
        width: u16,
        height: u16,
        image: &mut RgbaImage,
    ) -> Result<(), Error> {
        self.resize_target(width, height);
        let texture_view = self.target.texture_view().clone();
        let target_width = self.target.width();
        let target_height = self.target.height();
        self.render_to_view(scene, &texture_view, target_width, target_height)?;
        readback_into(
            &self.device,
            &self.queue,
            self.target.texture(),
            target_width,
            target_height,
            image,
        )
    }

    /// Render a native [`vello_hybrid::Scene`] and return an RGBA8 image (unpremultiplied).
    pub fn render(
        &mut self,
        scene: &vello_hybrid::Scene,
        width: u16,
        height: u16,
    ) -> Result<RgbaImage, Error> {
        let mut image = RgbaImage::new(u32::from(width), u32::from(height));
        self.render_into(scene, width, height, &mut image)?;
        Ok(image)
    }

    /// Render a native [`vello_hybrid::Scene`] into a caller-provided texture view.
    pub fn render_to_texture_view(
        &mut self,
        scene: &vello_hybrid::Scene,
        texture_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> Result<(), Error> {
        self.render_to_view(scene, texture_view, width, height)
    }

    fn resize_target(&mut self, width: u16, height: u16) {
        if self.width == width && self.height == height {
            return;
        }

        self.target
            .resize(&self.device, u32::from(width), u32::from(height));
        self.width = width;
        self.height = height;
    }

    fn render_to_view(
        &mut self,
        scene: &vello_hybrid::Scene,
        texture_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> Result<(), Error> {
        let render_size = RenderSize { width, height };
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
                texture_view,
            )
            .map_err(Error::Render)?;

        self.queue.submit([encoder.finish()]);
        Ok(())
    }
}

fn readback_into(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
    image: &mut RgbaImage,
) -> Result<(), Error> {
    read_texture_into(device, queue, texture, width, height, image).map_err(map_readback_error)?;
    unpremultiply_rgba8_in_place(&mut image.data);
    Ok(())
}

fn map_readback_error(err: ReadbackError) -> Error {
    match err {
        ReadbackError::DevicePoll => Error::Internal("device poll failed"),
        ReadbackError::CallbackDropped => Error::Internal("map_async callback dropped"),
        ReadbackError::BufferMap => Error::Internal("buffer map failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imaging::{Painter, record::Scene};
    use kurbo::Rect;
    use peniko::{Blob, Brush, Color, ImageAlphaType, ImageBrush, ImageData, ImageFormat};
    use pollster::block_on;
    use std::sync::Arc;
    use wgpu::Extent3d;

    fn try_init_device_and_queue() -> Result<(wgpu::Device, wgpu::Queue), ()> {
        block_on(async {
            let instance = wgpu::Instance::default();
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::default(),
                    force_fallback_adapter: false,
                    compatible_surface: None,
                })
                .await
                .map_err(|_| ())?;
            adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("imaging_vello_hybrid test device"),
                    required_features: wgpu::Features::empty(),
                    ..Default::default()
                })
                .await
                .map_err(|_| ())
        })
    }

    #[test]
    fn render_renders_encoded_scene() {
        let Ok((device, queue)) = try_init_device_and_queue() else {
            return;
        };
        let mut renderer = VelloHybridRenderer::new(device, queue);

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

        let native = renderer.encode_scene(&scene, 48, 48).unwrap();
        let image = renderer.render(&native, 48, 48).unwrap();
        assert_eq!(image.width, 48);
        assert_eq!(image.height, 48);
    }

    #[test]
    fn texture_view_render_smoke() {
        let Ok((device, queue)) = try_init_device_and_queue() else {
            return;
        };
        let mut renderer = VelloHybridRenderer::new(device.clone(), queue);

        let mut scene = Scene::new();
        {
            let mut painter = Painter::new(&mut scene);
            painter
                .fill(
                    Rect::new(0.0, 0.0, 24.0, 24.0),
                    Color::from_rgb8(0xd9, 0x77, 0x06),
                )
                .draw();
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("imaging_vello_hybrid target"),
            size: Extent3d {
                width: 24,
                height: 24,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let native = renderer.encode_scene(&scene, 24, 24).unwrap();
        renderer
            .render_to_texture_view(&native, &texture_view, 24, 24)
            .unwrap();
    }

    #[test]
    fn app_owned_wgpu_renders() {
        let Ok((device, queue)) = try_init_device_and_queue() else {
            return;
        };
        let mut renderer = VelloHybridRenderer::new(device, queue);

        let mut scene = Scene::new();
        {
            let mut painter = Painter::new(&mut scene);
            painter
                .fill(
                    Rect::new(0.0, 0.0, 20.0, 20.0),
                    Color::from_rgb8(0xd9, 0x77, 0x06),
                )
                .draw();
        }

        let native = renderer.encode_scene(&scene, 20, 20).unwrap();
        let image = renderer.render(&native, 20, 20).unwrap();
        assert_eq!(image.width, 20);
        assert_eq!(image.height, 20);
    }

    #[test]
    fn native_scene_with_image_brush_survives_resize() {
        let Ok((device, queue)) = try_init_device_and_queue() else {
            return;
        };
        let mut renderer = VelloHybridRenderer::new(device, queue);

        let image = ImageData {
            data: Blob::new(Arc::new([
                0xff, 0x20, 0x20, 0xff, 0x20, 0xff, 0x20, 0xff, 0x20, 0x20, 0xff, 0xff, 0xff, 0xff,
                0x20, 0xff,
            ])),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: 2,
            height: 2,
        };
        let brush = Brush::Image(ImageBrush::new(image));

        let mut scene = Scene::new();
        {
            let mut painter = Painter::new(&mut scene);
            painter.fill(Rect::new(0.0, 0.0, 20.0, 20.0), &brush).draw();
        }

        let native = renderer.encode_scene(&scene, 20, 20).unwrap();
        let resize_scene = Scene::new();
        let _ = renderer.encode_scene(&resize_scene, 24, 24).unwrap();

        let image = renderer.render(&native, 20, 20).unwrap();
        assert_eq!(image.width, 20);
        assert_eq!(image.height, 20);
    }
}
