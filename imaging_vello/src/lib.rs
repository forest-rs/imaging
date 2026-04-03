// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Vello backend for `imaging`.
//!
//! This crate provides a headless CPU/GPU renderer that consumes native [`vello::Scene`] values
//! and renders them to GPU targets or RGBA8 image data using `vello` + `wgpu`.
//!
//! Semantic [`imaging::record::Scene`] values can be lowered to native Vello scenes through
//! [`VelloRenderer::encode_scene`].
//!
//! In UI integrations, the host application should usually own the `wgpu` device, queue, and
//! presentation targets, then pass those handles into [`VelloRenderer`].
//!
//! Enable exactly one backend compatibility feature:
//!
//! - `vello-0-8` (default)
//! - `vello-0-7`
//!
//! # Render A Recorded Scene
//!
//! Record commands into [`imaging::record::Scene`], then render them with [`VelloRenderer`].
//!
//! ```no_run
//! use imaging::{Painter, record};
//! use imaging_vello::VelloRenderer;
//! use kurbo::Rect;
//! use peniko::{Brush, Color};
//!
//! fn main() -> Result<(), imaging_vello::Error> {
//!     let paint = Brush::Solid(Color::from_rgb8(0x2a, 0x6f, 0xdb));
//!     let mut scene = record::Scene::new();
//!
//!     {
//!         let mut painter = Painter::new(&mut scene);
//!         painter.fill_rect(Rect::new(0.0, 0.0, 128.0, 128.0), &paint);
//!     }
//!
//!     # let device: imaging_vello::wgpu::Device = todo!();
//!     # let queue: imaging_vello::wgpu::Queue = todo!();
//!     let mut renderer = VelloRenderer::new(device, queue)?;
//!     let native = renderer.encode_scene(&scene, 128, 128)?;
//!     let image = renderer.render(&native, 128, 128)?;
//!     assert_eq!(image.width, 128);
//!     Ok(())
//! }
//! ```
//!
//! # Record Into `vello::Scene`
//!
//! If you want a backend-native retained scene without going through [`VelloRenderer`], wrap a
//! mutable [`vello::Scene`] with [`VelloSceneSink`].
//!
//! ```no_run
//! use imaging::Painter;
//! use imaging_vello::{VelloSceneSink, vello};
//! use kurbo::Rect;
//! use peniko::{Brush, Color};
//!
//! fn main() -> Result<(), imaging_vello::Error> {
//!     let paint = Brush::Solid(Color::from_rgb8(0x1d, 0x4e, 0x89));
//!     let mut scene = vello::Scene::new();
//!
//!     {
//!         let bounds = Rect::new(0.0, 0.0, 128.0, 128.0);
//!         let mut sink = VelloSceneSink::new(&mut scene, bounds);
//!         let mut painter = Painter::new(&mut sink);
//!         painter.fill_rect(bounds, &paint);
//!         sink.finish()?;
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! # Render A Native `vello::Scene`
//!
//! If you already have a native Vello scene, hand it directly to [`VelloRenderer`].
//!
//! ```no_run
//! use imaging::Painter;
//! use imaging_vello::{VelloRenderer, VelloSceneSink, vello};
//! use kurbo::Rect;
//! use peniko::{Brush, Color};
//!
//! fn main() -> Result<(), imaging_vello::Error> {
//!     let paint = Brush::Solid(Color::from_rgb8(0xd9, 0x77, 0x06));
//!     let mut scene = vello::Scene::new();
//!
//!     {
//!         let bounds = Rect::new(0.0, 0.0, 128.0, 128.0);
//!         let mut sink = VelloSceneSink::new(&mut scene, bounds);
//!         let mut painter = Painter::new(&mut sink);
//!         painter.fill_rect(bounds, &paint);
//!         sink.finish()?;
//!     }
//!
//!     # let device: imaging_vello::wgpu::Device = todo!();
//!     # let queue: imaging_vello::wgpu::Queue = todo!();
//!     let mut renderer = VelloRenderer::new(device, queue)?;
//!     let image = renderer.render(&scene, 128, 128)?;
//!     assert_eq!(image.width, 128);
//!     Ok(())
//! }
//! ```
//!
//! Note: Vello uses a single layer stack for clipping and blending. Scenes that interleave clips
//! and groups in ways Vello cannot represent may return [`Error::UnbalancedLayerStack`].

#![deny(unsafe_code)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod scene_sink;
mod wgpu_support;

#[cfg(all(feature = "vello-0-7", feature = "vello-0-8"))]
compile_error!("Enable exactly one of `vello-0-7` or `vello-0-8`.");

#[cfg(not(any(feature = "vello-0-7", feature = "vello-0-8")))]
compile_error!("Enable one of `vello-0-7` or `vello-0-8`.");

use imaging::RgbaImage;
use imaging::record::{Scene, ValidateError, replay};
use kurbo::Rect;

#[cfg(feature = "vello-0-7")]
pub use vello_07 as vello;
#[cfg(all(not(feature = "vello-0-7"), feature = "vello-0-8"))]
pub use vello_08 as vello;

pub use crate::vello::wgpu;
use crate::vello::{AaConfig, RenderParams};
use crate::wgpu_support::{OffscreenTarget, ReadbackError, read_texture_into};

pub use scene_sink::VelloSceneSink;

/// Errors that can occur when rendering via Vello.
#[derive(Debug)]
pub enum Error {
    /// The scene is invalid (unbalanced stacks).
    InvalidScene(ValidateError),
    /// An image brush was encountered; this backend does not support it.
    UnsupportedImageBrush,
    /// A filter configuration could not be translated.
    UnsupportedFilter,
    /// A mask mode or masking primitive is not supported by this backend.
    UnsupportedMask,
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
    /// Vello returned a render error.
    Render(vello::Error),
    /// An internal invariant was violated.
    Internal(&'static str),
}

/// Renderer that executes `imaging` commands using `vello` + `wgpu`.
pub struct VelloRenderer {
    renderer: vello::Renderer,
    device: wgpu::Device,
    queue: wgpu::Queue,
    target: OffscreenTarget,
    width: u16,
    height: u16,
}

impl core::fmt::Debug for VelloRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VelloRenderer")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl VelloRenderer {
    /// Create a renderer bound to an existing `wgpu` device and queue.
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Result<Self, Error> {
        let target = OffscreenTarget::new(&device, 1, 1);
        let renderer = vello::Renderer::new(&device, vello::RendererOptions::default())
            .map_err(Error::Render)?;

        Ok(Self {
            renderer,
            device,
            queue,
            target,
            width: 1,
            height: 1,
        })
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

    /// Lower a semantic [`imaging::record::Scene`] into a native [`crate::vello::Scene`].
    pub fn encode_scene(
        &self,
        scene: &Scene,
        width: u32,
        height: u32,
    ) -> Result<vello::Scene, Error> {
        scene.validate().map_err(Error::InvalidScene)?;
        let mut native = vello::Scene::new();
        let bounds = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        let mut sink = VelloSceneSink::new(&mut native, bounds);
        replay(scene, &mut sink);
        sink.finish()?;
        Ok(native)
    }

    /// Render a native [`crate::vello::Scene`] into an RGBA8 image (unpremultiplied).
    pub fn render_into(
        &mut self,
        scene: &vello::Scene,
        width: u16,
        height: u16,
        image: &mut RgbaImage,
    ) -> Result<(), Error> {
        self.resize_target(width, height);
        let texture_view = self.target.texture_view().clone();
        let width = self.target.width();
        let height = self.target.height();
        self.render_to_view(scene, &texture_view, width, height)?;
        readback_into(
            &self.device,
            &self.queue,
            self.target.texture(),
            self.width,
            self.height,
            image,
        )
    }

    /// Render a native [`crate::vello::Scene`] and return an RGBA8 image (unpremultiplied).
    pub fn render(
        &mut self,
        scene: &vello::Scene,
        width: u16,
        height: u16,
    ) -> Result<RgbaImage, Error> {
        let mut image = RgbaImage::new(u32::from(width), u32::from(height));
        self.render_into(scene, width, height, &mut image)?;
        Ok(image)
    }

    /// Render a native [`crate::vello::Scene`] into a caller-provided texture view.
    pub fn render_to_texture_view(
        &mut self,
        scene: &vello::Scene,
        texture_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> Result<(), Error> {
        self.render_to_view(scene, texture_view, width, height)
    }

    fn render_to_view(
        &mut self,
        scene: &vello::Scene,
        texture_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> Result<(), Error> {
        let params = RenderParams {
            base_color: peniko::Color::from_rgba8(0, 0, 0, 0),
            width,
            height,
            antialiasing_method: AaConfig::Area,
        };

        self.renderer
            .render_to_texture(&self.device, &self.queue, scene, texture_view, &params)
            .map_err(Error::Render)
    }
}

fn readback_into(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u16,
    height: u16,
    image: &mut RgbaImage,
) -> Result<(), Error> {
    read_texture_into(
        device,
        queue,
        texture,
        u32::from(width),
        u32::from(height),
        image,
    )
    .map_err(map_readback_error)
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
    use imaging::Painter;
    use kurbo::Rect;
    use peniko::Color;
    use pollster::block_on;

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
                    label: Some("imaging_vello test device"),
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
        let mut renderer = VelloRenderer::new(device, queue).unwrap();

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
        let native = renderer.encode_scene(&scene, 64, 64).unwrap();
        let image = renderer.render(&native, 64, 64).unwrap();
        assert_eq!(image.width, 64);
        assert_eq!(image.height, 64);
    }

    #[test]
    fn texture_view_render_smoke() {
        let Ok((device, queue)) = try_init_device_and_queue() else {
            return;
        };
        let mut renderer = VelloRenderer::new(device.clone(), queue).unwrap();

        let mut scene = Scene::new();
        {
            let mut painter = Painter::new(&mut scene);
            painter
                .fill(
                    Rect::new(0.0, 0.0, 32.0, 32.0),
                    Color::from_rgb8(0x1d, 0x4e, 0x89),
                )
                .draw();
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("imaging_vello target"),
            size: wgpu::Extent3d {
                width: 32,
                height: 32,
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
        let native = renderer.encode_scene(&scene, 32, 32).unwrap();
        renderer
            .render_to_texture_view(&native, &texture_view, 32, 32)
            .unwrap();
    }

    #[test]
    fn app_owned_wgpu_renders() {
        let Ok((device, queue)) = try_init_device_and_queue() else {
            return;
        };
        let mut renderer = VelloRenderer::new(device, queue).unwrap();

        let mut scene = Scene::new();
        {
            let mut painter = Painter::new(&mut scene);
            painter
                .fill(
                    Rect::new(0.0, 0.0, 16.0, 16.0),
                    Color::from_rgb8(0x2a, 0x6f, 0xdb),
                )
                .draw();
        }

        let native = renderer.encode_scene(&scene, 16, 16).unwrap();
        let image = renderer.render(&native, 16, 16).unwrap();
        assert_eq!(image.width, 16);
        assert_eq!(image.height, 16);
    }
}
