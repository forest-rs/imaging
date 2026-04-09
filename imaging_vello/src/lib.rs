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
//! If you want a backend-native retained scene without going through a renderer, wrap a
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
use imaging::render::{
    GpuReadbackError, ImageBufferFormat, ImageBufferTarget, ImageRenderer, ImageRendererError,
    ImageTargetError, RenderContentError, RenderSource, RenderUnsupportedError,
};
use imaging_wgpu::{TextureRenderer, TextureRendererError, TextureTargetError, TextureViewTarget};
use kurbo::Rect;

#[cfg(feature = "vello-0-7")]
pub use vello_07 as vello;
#[cfg(all(not(feature = "vello-0-7"), feature = "vello-0-8"))]
pub use vello_08 as vello;

pub use crate::vello::wgpu;
use crate::vello::{AaConfig, RenderParams};
use crate::wgpu_support::{
    OffscreenTarget, ReadbackError, read_texture_into, read_texture_into_target,
};

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

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl core::error::Error for Error {}

struct VelloRendererState {
    renderer: vello::Renderer,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

/// Renderer that executes `imaging` commands into caller-owned texture views using `vello` +
/// `wgpu`.
pub struct VelloRenderer {
    state: VelloRendererState,
    target: Option<OffscreenTarget>,
}

impl core::fmt::Debug for VelloRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VelloRenderer").finish_non_exhaustive()
    }
}

impl VelloRendererState {
    fn checked_size(width: u32, height: u32) -> Result<(u16, u16), Error> {
        let width = u16::try_from(width).map_err(|_| Error::Internal("render width too large"))?;
        let height =
            u16::try_from(height).map_err(|_| Error::Internal("render height too large"))?;
        Ok((width, height))
    }

    fn new(device: wgpu::Device, queue: wgpu::Queue) -> Result<Self, Error> {
        let renderer = vello::Renderer::new(&device, vello::RendererOptions::default())
            .map_err(Error::Render)?;

        Ok(Self {
            renderer,
            device,
            queue,
        })
    }

    /// Lower a semantic [`imaging::record::Scene`] into a native [`crate::vello::Scene`].
    fn encode_scene(&self, scene: &Scene, width: u32, height: u32) -> Result<vello::Scene, Error> {
        scene.validate().map_err(Error::InvalidScene)?;
        let mut native = vello::Scene::new();
        let bounds = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        let mut sink = VelloSceneSink::new(&mut native, bounds);
        replay(scene, &mut sink);
        sink.finish()?;
        Ok(native)
    }

    fn encode_source<S: RenderSource + ?Sized>(
        &self,
        source: &mut S,
        width: u32,
        height: u32,
    ) -> Result<vello::Scene, Error> {
        source.validate().map_err(Error::InvalidScene)?;
        let mut native = vello::Scene::new();
        let bounds = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        let mut sink = VelloSceneSink::new(&mut native, bounds);
        source.paint_into(&mut sink);
        sink.finish()?;
        Ok(native)
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

impl VelloRenderer {
    /// Create a renderer bound to an existing `wgpu` device and queue.
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Result<Self, Error> {
        Ok(Self {
            state: VelloRendererState::new(device, queue)?,
            target: None,
        })
    }

    /// Lower a semantic [`imaging::record::Scene`] into a native [`crate::vello::Scene`].
    pub fn encode_scene(
        &self,
        scene: &Scene,
        width: u32,
        height: u32,
    ) -> Result<vello::Scene, Error> {
        self.state.encode_scene(scene, width, height)
    }

    fn encode_source<S: RenderSource + ?Sized>(
        &mut self,
        source: &mut S,
        width: u32,
        height: u32,
    ) -> Result<vello::Scene, Error> {
        self.state.encode_source(source, width, height)
    }

    /// Render a native [`crate::vello::Scene`] into a caller-provided texture view.
    pub fn render_to_texture_view(
        &mut self,
        scene: &vello::Scene,
        texture_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> Result<(), Error> {
        self.state
            .render_to_view(scene, texture_view, width, height)
    }

    fn ensure_target(&mut self, width: u16, height: u16) -> &OffscreenTarget {
        let target = self.target.get_or_insert_with(|| {
            OffscreenTarget::new(&self.state.device, u32::from(width), u32::from(height))
        });
        target.resize(&self.state.device, u32::from(width), u32::from(height));
        target
    }
}

fn supported_texture_formats() -> Vec<wgpu::TextureFormat> {
    vec![wgpu::TextureFormat::Rgba8Unorm]
}

fn supported_image_formats() -> Vec<ImageBufferFormat> {
    vec![ImageBufferFormat::Rgba8Unorm]
}

impl TextureRenderer for VelloRenderer {
    type TextureTarget = TextureViewTarget;
    type Texture = wgpu::Texture;

    fn supported_texture_formats(&self) -> Vec<wgpu::TextureFormat> {
        supported_texture_formats()
    }

    fn render_source_into_texture(
        &mut self,
        source: &mut dyn RenderSource,
        target: TextureViewTarget,
    ) -> Result<(), TextureRendererError> {
        let native = self
            .encode_source(source, target.width, target.height)
            .map_err(map_texture_renderer_error)?;
        self.render_to_texture_view(&native, &target.view, target.width, target.height)
            .map_err(map_texture_renderer_error)
    }

    fn render_source_texture(
        &mut self,
        source: &mut dyn RenderSource,
        width: u32,
        height: u32,
    ) -> Result<Self::Texture, TextureRendererError> {
        let native = self
            .encode_source(source, width, height)
            .map_err(map_texture_renderer_error)?;
        let (width, height) =
            VelloRendererState::checked_size(width, height).map_err(map_texture_renderer_error)?;
        let target = self.ensure_target(width, height);
        let texture = target.texture().clone();
        let texture_view = target.texture_view().clone();
        let target_width = target.width();
        let target_height = target.height();
        self.render_to_texture_view(&native, &texture_view, target_width, target_height)
            .map_err(map_texture_renderer_error)?;
        Ok(texture)
    }
}

impl VelloRenderer {
    /// Render a native [`crate::vello::Scene`] into an RGBA8 image (unpremultiplied).
    pub fn render_into(
        &mut self,
        scene: &vello::Scene,
        width: u16,
        height: u16,
        image: &mut RgbaImage,
    ) -> Result<(), Error> {
        let target = self.ensure_target(width, height);
        let texture_view = target.texture_view().clone();
        let target_texture = target.texture().clone();
        let target_width = target.width();
        let target_height = target.height();
        self.render_to_texture_view(scene, &texture_view, target_width, target_height)?;
        readback_into(
            &self.state.device,
            &self.state.queue,
            &target_texture,
            width,
            height,
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
}

impl ImageRenderer for VelloRenderer {
    fn supported_image_formats(&self) -> Vec<ImageBufferFormat> {
        supported_image_formats()
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
        let texture = <Self as TextureRenderer>::render_source_texture(
            self,
            source,
            target.width,
            target.height,
        )
        .map_err(map_texture_to_image_error)?;
        read_texture_into_target(
            &self.state.device,
            &self.state.queue,
            &texture,
            target.width,
            target.height,
            target.data,
            target.bytes_per_row,
        )
        .map_err(map_readback_image_error)
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
        ReadbackError::InvalidTargetStride => Error::Internal("image target row stride too small"),
        ReadbackError::InvalidTargetBuffer => Error::Internal("image target buffer too small"),
    }
}

fn map_texture_renderer_error(error: Error) -> TextureRendererError {
    match error {
        Error::InvalidScene(error) => {
            TextureRendererError::Content(RenderContentError::InvalidScene(error))
        }
        Error::UnsupportedImageBrush => {
            TextureRendererError::Unsupported(RenderUnsupportedError::ImageBrush)
        }
        Error::UnsupportedFilter => {
            TextureRendererError::Unsupported(RenderUnsupportedError::Filter)
        }
        Error::UnsupportedMask => TextureRendererError::Unsupported(RenderUnsupportedError::Mask),
        Error::UnsupportedGlyphBlend => {
            TextureRendererError::Unsupported(RenderUnsupportedError::Glyph)
        }
        Error::UnsupportedBlurredRoundedRectBlend => {
            TextureRendererError::Unsupported(RenderUnsupportedError::BlurredRoundedRect)
        }
        Error::UnbalancedLayerStack => {
            TextureRendererError::Unsupported(RenderUnsupportedError::UnbalancedLayerStack)
        }
        Error::Internal("render width too large" | "render height too large") => {
            TextureRendererError::Target(TextureTargetError::DimensionsTooLarge)
        }
        other => TextureRendererError::backend(other),
    }
}

fn map_texture_to_image_error(error: TextureRendererError) -> ImageRendererError {
    match error {
        TextureRendererError::Content(error) => ImageRendererError::Content(error),
        TextureRendererError::Target(error) => match error {
            TextureTargetError::InvalidTarget(message) => {
                ImageRendererError::Target(ImageTargetError::InvalidTarget(message))
            }
            TextureTargetError::DimensionsTooLarge => {
                ImageRendererError::Target(ImageTargetError::DimensionsTooLarge)
            }
            TextureTargetError::UnsupportedTextureFormat => {
                ImageRendererError::Target(ImageTargetError::UnsupportedTargetFormat)
            }
            TextureTargetError::CreateGpuContext(message) => {
                ImageRendererError::Target(ImageTargetError::InvalidTarget(message))
            }
            TextureTargetError::CreateGpuSurface => {
                ImageRendererError::Target(ImageTargetError::InvalidTarget(
                    "backend could not wrap the texture as a GPU render surface",
                ))
            }
            TextureTargetError::UnsupportedGpuBackend => {
                ImageRendererError::Target(ImageTargetError::InvalidTarget(
                    "no supported GPU backend was available for the supplied wgpu setup",
                ))
            }
        },
        TextureRendererError::Unsupported(error) => ImageRendererError::Unsupported(error),
        TextureRendererError::Backend(error) => ImageRendererError::Backend(error),
    }
}

fn map_readback_image_error(error: ReadbackError) -> ImageRendererError {
    match error {
        ReadbackError::DevicePoll => ImageRendererError::Readback(GpuReadbackError::DevicePoll),
        ReadbackError::CallbackDropped => {
            ImageRendererError::Readback(GpuReadbackError::CallbackDropped)
        }
        ReadbackError::BufferMap => ImageRendererError::Readback(GpuReadbackError::BufferMap),
        ReadbackError::InvalidTargetStride => {
            ImageRendererError::Target(ImageTargetError::InvalidTarget(
                "image target row stride is smaller than the rendered width",
            ))
        }
        ReadbackError::InvalidTargetBuffer => {
            ImageRendererError::Target(ImageTargetError::InvalidTargetBuffer)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imaging::{Painter, render::ImageTargetError};
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
    fn render_source_renders_scene() {
        let Ok((device, queue)) = try_init_device_and_queue() else {
            return;
        };
        let mut renderer = VelloRenderer::new(device, queue).unwrap();

        let mut scene = Scene::new();
        {
            let mut painter = Painter::new(&mut scene);
            painter
                .fill(
                    Rect::new(0.0, 0.0, 40.0, 40.0),
                    Color::from_rgb8(0x2a, 0x6f, 0xdb),
                )
                .draw();
        }

        let mut source = &scene;
        let image = ImageRenderer::render_source(&mut renderer, &mut source, 40, 40).unwrap();
        assert_eq!(image.width, 40);
        assert_eq!(image.height, 40);
    }

    #[test]
    fn invalid_target_stride_maps_to_image_target_error() {
        assert!(matches!(
            map_readback_image_error(ReadbackError::InvalidTargetStride),
            ImageRendererError::Target(ImageTargetError::InvalidTarget(
                "image target row stride is smaller than the rendered width",
            ))
        ));
    }

    #[test]
    fn invalid_target_buffer_maps_to_image_target_error() {
        assert!(matches!(
            map_readback_image_error(ReadbackError::InvalidTargetBuffer),
            ImageRendererError::Target(ImageTargetError::InvalidTargetBuffer)
        ));
    }

    #[test]
    fn supported_image_formats_are_rgba8_only() {
        assert_eq!(
            supported_image_formats(),
            vec![ImageBufferFormat::Rgba8Unorm]
        );
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
    fn render_source_to_texture_smoke() {
        let Ok((device, queue)) = try_init_device_and_queue() else {
            return;
        };
        let mut renderer = VelloRenderer::new(device.clone(), queue).unwrap();

        let mut scene = Scene::new();
        {
            let mut painter = Painter::new(&mut scene);
            painter
                .fill(
                    Rect::new(0.0, 0.0, 24.0, 24.0),
                    Color::from_rgb8(0x1d, 0x4e, 0x89),
                )
                .draw();
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("imaging_vello target"),
            size: wgpu::Extent3d {
                width: 24,
                height: 24,
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
        let mut source = &scene;
        TextureRenderer::render_source_into_texture(
            &mut renderer,
            &mut source,
            TextureViewTarget::new(&texture_view, 24, 24),
        )
        .unwrap();
    }

    #[test]
    fn supported_texture_formats_are_rgba8_only() {
        assert_eq!(
            supported_texture_formats(),
            vec![wgpu::TextureFormat::Rgba8Unorm]
        );
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
