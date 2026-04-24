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
use imaging::render::{
    GpuReadbackError, ImageBufferFormat, ImageBufferTarget, ImageRenderer, ImageRendererError,
    ImageTargetError, RenderContentError, RenderSource, RenderUnsupportedError,
};
pub use imaging_wgpu::wgpu;
use imaging_wgpu::{TextureRenderer, TextureRendererError, TextureTargetError, TextureViewTarget};
use vello_hybrid::{RenderError, RenderSize, RenderTargetConfig};
use wgpu::{CommandEncoderDescriptor, TextureFormat};

use crate::wgpu_support::{
    OffscreenTarget, ReadbackError, create_texture, read_texture_into, read_texture_into_target,
    unpremultiply_rgba8_in_place, unpremultiply_rgba8_target,
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

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl core::error::Error for Error {}

#[derive(Debug)]
pub(crate) struct VelloHybridRendererState {
    renderer: vello_hybrid::Renderer,
    device: wgpu::Device,
    queue: wgpu::Queue,
    tolerance: f64,
    image_registry: HybridImageRegistry,
}

/// Target-oriented renderer that executes `imaging` commands using `vello_hybrid` + `wgpu`.
///
/// This type owns backend state and uploaded images, but it does not own an offscreen render
/// target. Use it when the host application owns the destination texture view.
#[derive(Debug)]
pub struct VelloHybridRenderer {
    state: VelloHybridRendererState,
    target: Option<OffscreenTarget>,
}
/// [`VelloHybridRenderer`] implements [`TextureRenderer`] with [`TextureViewTarget`].
impl VelloHybridRendererState {
    fn checked_size(width: u32, height: u32) -> Result<(u16, u16), Error> {
        let width = u16::try_from(width).map_err(|_| Error::Internal("render width too large"))?;
        let height =
            u16::try_from(height).map_err(|_| Error::Internal("render height too large"))?;
        Ok((width, height))
    }

    fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
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
            tolerance: 0.1,
            image_registry: HybridImageRegistry::default(),
        }
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

    fn clear_cached_images(&mut self) {
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

impl VelloHybridRenderer {
    /// Create a renderer bound to an existing `wgpu` device and queue.
    #[must_use]
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        Self {
            state: VelloHybridRendererState::new(device, queue),
            target: None,
        }
    }

    /// Set the tolerance used when converting shapes to paths.
    pub fn set_tolerance(&mut self, tolerance: f64) {
        self.state.tolerance = tolerance;
    }

    /// Destroy all uploaded hybrid image resources cached by this renderer.
    pub fn clear_cached_images(&mut self) {
        self.state.clear_cached_images();
    }

    pub(crate) fn begin_image_upload_session(
        &mut self,
        label: &'static str,
    ) -> HybridImageUploadSession<'_> {
        self.state.begin_image_upload_session(label)
    }

    /// Lower a semantic [`imaging::record::Scene`] into a native [`vello_hybrid::Scene`].
    pub fn encode_scene(
        &mut self,
        scene: &Scene,
        width: u16,
        height: u16,
    ) -> Result<vello_hybrid::Scene, Error> {
        scene.validate().map_err(Error::InvalidScene)?;
        let mut native = vello_hybrid::Scene::new(width, height);
        native.reset();
        let tolerance = self.state.tolerance;
        {
            let mut sink = VelloHybridSceneSink::with_renderer(&mut native, self);
            sink.set_tolerance(tolerance);
            replay(scene, &mut sink);
            sink.finish()?;
        }
        Ok(native)
    }

    fn encode_source<S: RenderSource + ?Sized>(
        &mut self,
        source: &mut S,
        width: u32,
        height: u32,
    ) -> Result<vello_hybrid::Scene, Error> {
        source.validate().map_err(Error::InvalidScene)?;
        let (width, height) = VelloHybridRendererState::checked_size(width, height)?;
        let mut native = vello_hybrid::Scene::new(width, height);
        native.reset();
        let tolerance = self.state.tolerance;
        {
            let mut sink = VelloHybridSceneSink::with_renderer(&mut native, self);
            sink.set_tolerance(tolerance);
            source.paint_into(&mut sink);
            sink.finish()?;
        }
        Ok(native)
    }

    /// Render a native [`vello_hybrid::Scene`] into a caller-provided texture view.
    pub fn render_to_texture_view(
        &mut self,
        scene: &vello_hybrid::Scene,
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

fn supported_texture_formats() -> Vec<TextureFormat> {
    vec![TextureFormat::Rgba8Unorm]
}

fn supported_image_formats() -> Vec<ImageBufferFormat> {
    vec![ImageBufferFormat::Rgba8Unorm]
}

impl TextureRenderer for VelloHybridRenderer {
    type TextureTarget = TextureViewTarget;
    type Texture = wgpu::Texture;

    fn supported_texture_formats(&self) -> Vec<TextureFormat> {
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
        let (target_width, target_height) = VelloHybridRendererState::checked_size(width, height)
            .map_err(map_texture_renderer_error)?;
        let texture = create_texture(
            &self.state.device,
            u32::from(target_width),
            u32::from(target_height),
        );
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.render_to_texture_view(
            &native,
            &texture_view,
            u32::from(target_width),
            u32::from(target_height),
        )
        .map_err(map_texture_renderer_error)?;
        Ok(texture)
    }
}

impl VelloHybridRenderer {
    /// Render a native [`vello_hybrid::Scene`] into an RGBA8 image (unpremultiplied).
    pub fn render_into(
        &mut self,
        scene: &vello_hybrid::Scene,
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
}

impl ImageRenderer for VelloHybridRenderer {
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
        readback_into_target(&self.state.device, &self.state.queue, &texture, target)
            .map_err(map_readback_image_error)
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

fn readback_into_target(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    target: ImageBufferTarget<'_>,
) -> Result<(), ReadbackError> {
    let width = target.width;
    let height = target.height;
    let bytes_per_row = target.bytes_per_row;
    let data = target.data;
    read_texture_into_target(
        device,
        queue,
        texture,
        width,
        height,
        &mut *data,
        bytes_per_row,
    )?;
    unpremultiply_rgba8_target(&mut *data, width, bytes_per_row);
    Ok(())
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
        Error::UnsupportedBlurredRoundedRect => {
            TextureRendererError::Unsupported(RenderUnsupportedError::BlurredRoundedRect)
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
    use imaging::{Painter, record::Scene, render::ImageTargetError};
    use kurbo::Rect;
    use peniko::{Blob, Brush, Color, ImageAlphaType, ImageBrush, ImageData, ImageFormat};
    use pollster::block_on;
    use std::sync::Arc;
    use wgpu::Extent3d;

    fn solid_scene(color: Color, width: f64, height: f64) -> Scene {
        let mut scene = Scene::new();
        {
            let mut painter = Painter::new(&mut scene);
            painter
                .fill(Rect::new(0.0, 0.0, width, height), color)
                .draw();
        }
        scene
    }

    fn assert_solid_rgba_image(image: &RgbaImage, expected: [u8; 4]) {
        assert_eq!(
            image.data.len(),
            usize::try_from(image.width).unwrap() * usize::try_from(image.height).unwrap() * 4
        );
        for (index, pixel) in image.data.chunks_exact(4).enumerate() {
            assert_eq!(pixel, expected, "pixel {index} did not match");
        }
    }

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
    fn render_source_renders_scene() {
        let Ok((device, queue)) = try_init_device_and_queue() else {
            return;
        };
        let mut renderer = VelloHybridRenderer::new(device, queue);

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
        let mut renderer = VelloHybridRenderer::new(device.clone(), queue.clone());

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

        let mut image = RgbaImage::new(24, 24);
        read_texture_into(&device, &queue, &texture, 24, 24, &mut image).unwrap();
        assert_solid_rgba_image(&image, [0xd9, 0x77, 0x06, 0xff]);
    }

    #[test]
    fn render_source_to_texture_smoke() {
        let Ok((device, queue)) = try_init_device_and_queue() else {
            return;
        };
        let mut renderer = VelloHybridRenderer::new(device.clone(), queue.clone());

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
        let mut source = &scene;
        TextureRenderer::render_source_into_texture(
            &mut renderer,
            &mut source,
            TextureViewTarget::new(&texture_view, 24, 24),
        )
        .unwrap();

        let mut image = RgbaImage::new(24, 24);
        read_texture_into(&device, &queue, &texture, 24, 24, &mut image).unwrap();
        assert_solid_rgba_image(&image, [0x1d, 0x4e, 0x89, 0xff]);
    }

    #[test]
    fn render_source_texture_returns_independent_texture() {
        let Ok((device, queue)) = try_init_device_and_queue() else {
            return;
        };
        let mut renderer = VelloHybridRenderer::new(device.clone(), queue.clone());

        let first_scene = solid_scene(Color::from_rgb8(0xff, 0x00, 0x00), 8.0, 8.0);
        let second_scene = solid_scene(Color::from_rgb8(0x00, 0xff, 0x00), 8.0, 8.0);

        let mut first_source = &first_scene;
        let first_texture =
            TextureRenderer::render_source_texture(&mut renderer, &mut first_source, 8, 8).unwrap();

        let mut second_source = &second_scene;
        let _second_texture =
            TextureRenderer::render_source_texture(&mut renderer, &mut second_source, 8, 8)
                .unwrap();

        let mut image = RgbaImage::new(8, 8);
        read_texture_into(&device, &queue, &first_texture, 8, 8, &mut image).unwrap();
        assert_solid_rgba_image(&image, [0xff, 0x00, 0x00, 0xff]);
    }

    #[test]
    fn supported_texture_formats_are_rgba8_only() {
        assert_eq!(supported_texture_formats(), vec![TextureFormat::Rgba8Unorm]);
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
