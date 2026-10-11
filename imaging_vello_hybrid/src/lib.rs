// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Vello hybrid backend for `imaging`.
//!
//! This crate provides a headless CPU/GPU renderer that consumes native
//! [`VelloHybridScene`] values and renders them to GPU targets or RGBA8 image data using
//! `vello_gpu` + `wgpu`.
//!
//! Semantic [`imaging::record::Scene`] values can be lowered to native hybrid scenes through
//! [`VelloHybridRenderer::encode_scene`].
//!
//! In UI integrations, the host application should usually own the `wgpu` device, queue, and
//! presentation targets, then pass those handles into [`VelloHybridRenderer`].
//!
//! Recorded scenes with inline image brushes are uploaded through a renderer-scoped image registry
//! and translated to backend-managed opaque image ids. Encoded scenes retain these uploads
//! until reset or dropped. Use [`VelloHybridSceneSink::with_renderer`]
//! when recording directly into a native [`VelloHybridScene`] and you want the same image
//! support.
//!
//! Alpha masks render the mask scene into an isolated `DestIn` layer, preserving its paint,
//! blending, and image bindings. Luminance masks remain unsupported by this backend.
//!
//! Filter chains use nested single-primitive layers. Blurs and drop shadows require equal
//! X/Y standard deviations; anisotropic filters return [`Error::UnsupportedFilter`].
//! Blurred rounded rectangles use the renderer's native analytic draw.
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
//! # Record A Hybrid Scene
//!
//! If you want a backend-native retained scene without owning a full renderer, wrap an existing
//! [`VelloHybridScene`] with [`VelloHybridSceneSink`].
//!
//! ```no_run
//! use imaging::Painter;
//! use imaging_vello_hybrid::{VelloHybridScene, VelloHybridSceneSink};
//! use kurbo::Rect;
//! use peniko::{Brush, Color};
//!
//! fn main() -> Result<(), imaging_vello_hybrid::Error> {
//!     let paint = Brush::Solid(Color::from_rgb8(0x1d, 0x4e, 0x89));
//!     let mut scene = VelloHybridScene::new(128, 128);
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
//! # Record Image Brushes
//!
//! Use [`VelloHybridSceneSink::with_renderer`] when recording image brushes directly into a
//! native [`VelloHybridScene`]. The sink uploads images through the renderer and reuses them
//! across later recordings and renders.
//!
//! ```no_run
//! use std::sync::Arc;
//!
//! use imaging::Painter;
//! use imaging_vello_hybrid::{VelloHybridRenderer, VelloHybridScene, VelloHybridSceneSink};
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
//!     let mut scene = VelloHybridScene::new(128, 128);
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
//! # Render An Encoded Hybrid Scene
//!
//! If you already have a native hybrid scene, hand it directly to [`VelloHybridRenderer`].
//!
//! ```no_run
//! use imaging::Painter;
//! use imaging_vello_hybrid::{VelloHybridRenderer, VelloHybridScene, VelloHybridSceneSink};
//! use kurbo::Rect;
//! use peniko::{Brush, Color};
//!
//! fn main() -> Result<(), imaging_vello_hybrid::Error> {
//!     let paint = Brush::Solid(Color::from_rgb8(0xd9, 0x77, 0x06));
//!     let mut scene = VelloHybridScene::new(128, 128);
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

//! # External Images
//!
//! Bind a caller-owned texture to an [`imaging::ExternalImage`], then use it through the
//! normal image brush or drawing API. Encoded scenes retain their bindings after unbinding.
//! The texture must come from the renderer's device; see the `TextureImageRenderer`
//! implementation below for format, usage, and alpha requirements.
//!
//! ```no_run
//! use imaging::{ExternalImage, Painter, record};
//! use imaging_wgpu::TextureImageRenderer;
//! use peniko::ImageAlphaType;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let mut renderer: imaging_vello_hybrid::VelloHybridRenderer = todo!();
//! # let texture: imaging_vello_hybrid::wgpu::Texture = todo!();
//! let image = ExternalImage::new(2, 2, ImageAlphaType::AlphaPremultiplied);
//! renderer.bind_image(&image, &texture)?;
//! let mut scene = record::Scene::new();
//! Painter::new(&mut scene).draw_image(&image, kurbo::Affine::IDENTITY);
//! let encoded = renderer.encode_scene(&scene, 2, 2)?;
//! renderer.unbind_image(&image);
//! let output = renderer.render(&encoded, 2, 2)?;
//! # Ok(())
//! # }
//! ```

#![deny(unsafe_code)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod image_registry;
mod scene;
mod scene_sink;
mod wgpu_support;

use image_registry::HybridImageRegistry;
use imaging::RgbaImage;
use imaging::record::{Scene, ValidateError, replay};
use imaging::render::{
    GpuReadbackError, ImageBufferFormat, ImageBufferTarget, ImageRenderer, ImageRendererError,
    ImageTargetError, RenderContentError, RenderSource, RenderUnsupportedError,
};
pub use imaging_wgpu::wgpu;
use imaging_wgpu::{TextureRenderer, TextureViewTarget};
use imaging_wgpu::{TextureRendererError, TextureTargetError};
use std::sync::Arc;
use vello_gpu::{RenderError, RenderSize, RenderTargetConfig, TextureBindings};
use wgpu::{CommandEncoderDescriptor, TextureFormat};

use crate::wgpu_support::{
    OffscreenTarget, ReadbackError, create_texture, read_texture_into, read_texture_into_target,
    unpremultiply_rgba8_in_place, unpremultiply_rgba8_target,
};

pub use scene::VelloHybridScene;
pub use scene_sink::VelloHybridSceneSink;

/// Errors that can occur when rendering via Vello hybrid.
#[derive(Debug)]
pub enum Error {
    /// The encoded scene belongs to a different renderer.
    WrongRenderer,
    /// An external image has no binding on this renderer.
    UnboundImage(imaging::ExternalImage),
    /// The scene is invalid (unbalanced stacks).
    InvalidScene(ValidateError),
    /// An image brush was encountered on a sink path that has no renderer-backed image resolver.
    UnsupportedImageBrush,
    /// Destructive glyph blending is not supported by Vello GPU.
    UnsupportedGlyphBlend,
    /// A filter configuration could not be translated.
    UnsupportedFilter,
    /// Luminance masks are not supported by this backend yet.
    UnsupportedMask,
    /// Blurred rounded rectangle parameters cannot be represented by the native renderer.
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
    identity: Arc<()>,
    renderer: vello_gpu::Renderer,
    resources: vello_gpu::Resources,
    device: wgpu::Device,
    queue: wgpu::Queue,
    tolerance: f64,
    image_registry: HybridImageRegistry,
    external_images: std::collections::HashMap<imaging::ExternalImage, Arc<ExternalTexture>>,
}

#[derive(Debug)]
pub(crate) struct ExternalTexture {
    view: wgpu::TextureView,
}

/// Target-oriented renderer that executes `imaging` commands using `vello_gpu` + `wgpu`.
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
        let (renderer, resources) = vello_gpu::Renderer::new(
            &device,
            &RenderTargetConfig {
                format: TextureFormat::Rgba8Unorm,
                width: 1,
                height: 1,
            },
        );

        Self {
            identity: Arc::new(()),
            renderer,
            resources,
            device,
            queue,
            tolerance: 0.1,
            image_registry: HybridImageRegistry::default(),
            external_images: std::collections::HashMap::new(),
        }
    }

    fn clear_cached_images(&mut self) {
        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("imaging_vello_hybrid clear cached images"),
            });
        self.image_registry
            .clear(&mut self.resources, &mut self.renderer, &mut encoder);
        self.queue.submit([encoder.finish()]);
    }

    /// Lower a semantic [`imaging::record::Scene`] into a native [`VelloHybridScene`].
    fn render_to_view(
        &mut self,
        scene: &VelloHybridScene,
        texture_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> Result<(), Error> {
        if let Some(identity) = &scene.renderer
            && !Arc::ptr_eq(identity, &self.identity)
        {
            return Err(Error::WrongRenderer);
        }
        let (width, height) = Self::checked_size(width, height)?;
        let render_size = RenderSize { width, height };
        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("imaging_vello_hybrid render"),
            });

        let mut bindings = TextureBindings::new();
        for (index, image) in scene.external_images.iter().enumerate() {
            bindings.insert(vello_common::TextureId(index as u64), image.view.clone());
        }
        self.renderer
            .render(
                &scene.native,
                &mut self.resources,
                &self.device,
                &self.queue,
                &mut encoder,
                &render_size,
                texture_view,
                None,
                &bindings,
                vello_gpu::TargetInit::Clear(vello_gpu::ClearSettings::default()),
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

    /// Release unused uploaded images from the cache.
    ///
    /// Images referenced by encoded scenes stay alive until those scenes are reset or dropped.
    /// Call this again afterwards to release their cached allocations immediately.
    pub fn clear_cached_images(&mut self) {
        self.state.clear_cached_images();
    }

    /// Lower a semantic [`imaging::record::Scene`] into a native [`VelloHybridScene`].
    pub fn encode_scene(
        &mut self,
        scene: &Scene,
        width: u16,
        height: u16,
    ) -> Result<VelloHybridScene, Error> {
        scene.validate().map_err(Error::InvalidScene)?;
        let mut native = VelloHybridScene::new(width, height);
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
    ) -> Result<VelloHybridScene, Error> {
        source.validate().map_err(Error::InvalidScene)?;
        let (width, height) = VelloHybridRendererState::checked_size(width, height)?;
        let mut native = VelloHybridScene::new(width, height);
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

    /// Render a native [`VelloHybridScene`] into a caller-provided texture view.
    pub fn render_to_texture_view(
        &mut self,
        scene: &VelloHybridScene,
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
    /// Render a native [`VelloHybridScene`] into an RGBA8 image (unpremultiplied).
    pub fn render_into(
        &mut self,
        scene: &VelloHybridScene,
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

    /// Render a native [`VelloHybridScene`] and return an RGBA8 image (unpremultiplied).
    pub fn render(
        &mut self,
        scene: &VelloHybridScene,
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
        Error::UnboundImage(image) => {
            TextureRendererError::Content(RenderContentError::UnboundImage(image))
        }
        Error::InvalidScene(error) => {
            TextureRendererError::Content(RenderContentError::InvalidScene(error))
        }
        Error::UnsupportedImageBrush => {
            TextureRendererError::Unsupported(RenderUnsupportedError::ImageBrush)
        }
        Error::UnsupportedGlyphBlend => {
            TextureRendererError::Unsupported(RenderUnsupportedError::Glyph)
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

/// Hybrid external images use premultiplied RGBA8 or BGRA8 textures with `TEXTURE_BINDING` usage.
impl imaging_wgpu::TextureImageRenderer for VelloHybridRenderer {
    fn bind_image(
        &mut self,
        image: &imaging::ExternalImage,
        texture: &wgpu::Texture,
    ) -> Result<(), imaging_wgpu::TextureImageError> {
        use imaging_wgpu::TextureImageError;
        imaging_wgpu::validate_image_texture(image, texture)?;
        if image.width() > u32::from(u16::MAX) || image.height() > u32::from(u16::MAX) {
            return Err(TextureImageError::InvalidTexture(
                "external image exceeds hybrid's dimension limit",
            ));
        }
        if image.alpha_type() != peniko::ImageAlphaType::AlphaPremultiplied {
            return Err(TextureImageError::UnsupportedAlphaType);
        }
        if !matches!(
            texture.format(),
            TextureFormat::Rgba8Unorm | TextureFormat::Bgra8Unorm
        ) {
            return Err(TextureImageError::UnsupportedFormat);
        }
        if !texture
            .usage()
            .contains(wgpu::TextureUsages::TEXTURE_BINDING)
        {
            return Err(TextureImageError::InvalidTexture(
                "hybrid external images require TEXTURE_BINDING usage",
            ));
        }
        if self.state.external_images.contains_key(image) {
            return Err(TextureImageError::AlreadyBound);
        }
        self.state.external_images.insert(
            image.clone(),
            Arc::new(ExternalTexture {
                view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            }),
        );
        Ok(())
    }
    fn unbind_image(&mut self, image: &imaging::ExternalImage) -> bool {
        self.state.external_images.remove(image).is_some()
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
                .request_adapter(&wgpu::RequestAdapterOptions::default())
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

    fn image_test_renderer() -> Option<VelloHybridRenderer> {
        if std::env::var("IMAGING_CI_GPU_SUPPORT").as_deref() == Ok("no") {
            return None;
        }
        let Ok((device, queue)) = try_init_device_and_queue() else {
            assert!(
                std::env::var("IMAGING_CI_GPU_SUPPORT").as_deref() != Ok("yes"),
                "GPU initialization failed"
            );
            return None;
        };
        Some(VelloHybridRenderer::new(device, queue))
    }

    fn retained_image_scene(
        renderer: &mut VelloHybridRenderer,
        color: [u8; 4],
    ) -> VelloHybridScene {
        let brush = Brush::Image(ImageBrush::new(ImageData {
            data: Blob::new(Arc::new(color)),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: 1,
            height: 1,
        }));
        let mut scene = Scene::new();
        Painter::new(&mut scene).fill_rect(Rect::new(0.0, 0.0, 8.0, 8.0), &brush);
        renderer.encode_scene(&scene, 8, 8).unwrap()
    }

    #[test]
    fn external_images_retain_bindings_and_sample_live_contents() {
        use imaging::{ExternalImage, ImageBrush};
        use imaging_wgpu::{TextureImageError, TextureImageRenderer};
        if std::env::var("IMAGING_CI_GPU_SUPPORT").is_ok_and(|value| value == "no") {
            return;
        }
        let gpu = try_init_device_and_queue();
        if std::env::var("IMAGING_CI_GPU_SUPPORT").is_ok_and(|value| value == "yes") {
            assert!(gpu.is_ok(), "GPU tests require a device");
        }
        let Ok((device, queue)) = gpu else {
            return;
        };
        let mut renderer = VelloHybridRenderer::new(device.clone(), queue.clone());
        let descriptor = wgpu::TextureDescriptor {
            label: Some("external image test"),
            size: Extent3d {
                width: 2,
                height: 2,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        };
        let texture = device.create_texture(&descriptor);
        let write = |pixel: [u8; 4]| {
            queue.write_texture(
                texture.as_image_copy(),
                &pixel.repeat(4),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(8),
                    rows_per_image: None,
                },
                texture.size(),
            );
        };
        write([128, 0, 0, 128]);
        let handle = ExternalImage::new(2, 2, ImageAlphaType::AlphaPremultiplied);
        let brush = ImageBrush::new(handle.clone());
        let mut recording = Scene::new();
        Painter::new(&mut recording).draw_image(&brush, kurbo::Affine::IDENTITY);
        assert!(matches!(
            renderer.encode_scene(&recording, 2, 2),
            Err(Error::UnboundImage(_))
        ));
        assert!(matches!(
            renderer.bind_image(&ExternalImage::new(1, 2, handle.alpha_type()), &texture),
            Err(TextureImageError::InvalidTexture(_))
        ));
        renderer.bind_image(&handle, &texture).unwrap();
        assert_eq!(
            renderer.bind_image(&handle, &texture),
            Err(TextureImageError::AlreadyBound)
        );
        let allocation = Arc::downgrade(renderer.state.external_images.get(&handle).unwrap());
        let mut encoded = renderer.encode_scene(&recording, 2, 2).unwrap();
        let mut appended = renderer.encode_scene(&recording, 2, 2).unwrap();
        assert_solid_rgba_image(&renderer.render(&encoded, 2, 2).unwrap(), [255, 0, 0, 128]);
        queue.write_texture(
            texture.as_image_copy(),
            &[128, 0, 0, 128, 0, 0, 128, 128].repeat(2),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8),
                rows_per_image: None,
            },
            texture.size(),
        );
        let pattern = ImageBrush::new(handle.clone()).with_sampler(
            peniko::ImageSampler::new()
                .with_extend(peniko::Extend::Repeat)
                .with_quality(peniko::ImageQuality::Low)
                .with_alpha(0.5),
        );
        let mut clipped = Scene::new();
        Painter::new(&mut clipped).with_fill_clip(Rect::new(1.0, 0.0, 3.0, 2.0), |p| {
            p.fill(Rect::new(0.0, 0.0, 4.0, 2.0), &pattern)
                .brush_transform(Some(kurbo::Affine::translate((1.0, 0.0))))
                .composite(imaging::Composite::new(peniko::BlendMode::default(), 0.5))
                .draw();
        });
        let clipped = renderer.encode_scene(&clipped, 4, 2).unwrap();
        let mut pixels = renderer.render(&clipped, 4, 2).unwrap().data;
        // Straight-alpha RGB is unspecified for completely transparent samples.
        for pixel in pixels.chunks_exact_mut(4).filter(|pixel| pixel[3] == 0) {
            pixel[..3].fill(0);
        }
        assert_eq!(
            pixels,
            [0, 0, 0, 0, 255, 0, 0, 32, 0, 0, 255, 32, 0, 0, 0, 0].repeat(2),
            "external brushes must preserve clip, brush transform, sampling, and alpha"
        );
        drop(clipped);
        drop(pattern);
        let mut masked = Scene::new();
        Painter::new(&mut masked).with_masked_group(
            imaging::MaskMode::Alpha,
            |mask| mask.draw_image(&handle, kurbo::Affine::IDENTITY),
            |content| content.fill_rect(Rect::new(0.0, 0.0, 2.0, 2.0), Color::WHITE),
        );
        let masked = renderer.encode_scene(&masked, 2, 2).unwrap();
        assert!(renderer.unbind_image(&handle));
        assert!(!renderer.unbind_image(&handle));
        assert!(matches!(
            renderer.encode_scene(&recording, 2, 2),
            Err(Error::UnboundImage(_))
        ));
        // Rebinding affects new commands without invalidating earlier captured bindings.
        let replacement = device.create_texture(&descriptor);
        queue.write_texture(
            replacement.as_image_copy(),
            &[0_u8, 255, 0, 255].repeat(4),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8),
                rows_per_image: None,
            },
            replacement.size(),
        );
        renderer.bind_image(&handle, &replacement).unwrap();
        let replacement_allocation =
            Arc::downgrade(renderer.state.external_images.get(&handle).unwrap());
        {
            let mut sink = VelloHybridSceneSink::with_renderer(&mut appended, &mut renderer);
            Painter::new(&mut sink).draw_image(&brush, kurbo::Affine::IDENTITY);
            sink.finish().unwrap();
        }
        renderer.unbind_image(&handle);
        drop(replacement);
        assert_solid_rgba_image(&renderer.render(&appended, 2, 2).unwrap(), [0, 255, 0, 255]);
        drop(appended);
        assert!(
            replacement_allocation.upgrade().is_none(),
            "appended binding is released with its scene"
        );
        drop(recording);
        drop(brush);
        drop(handle);
        write([0, 0, 128, 128]);
        drop(texture);
        assert_solid_rgba_image(&renderer.render(&encoded, 2, 2).unwrap(), [0, 0, 255, 128]);
        assert_solid_rgba_image(
            &renderer.render(&masked, 2, 2).unwrap(),
            [255, 255, 255, 128],
        );
        drop(masked);
        let mut other = VelloHybridRenderer::new(device.clone(), queue.clone());
        assert!(matches!(
            other.render(&encoded, 2, 2),
            Err(Error::WrongRenderer)
        ));
        assert!(
            allocation.upgrade().is_some(),
            "encoded scene retains the binding"
        );
        encoded.reset();
        assert!(
            allocation.upgrade().is_none(),
            "reset releases the unbound resource"
        );
        assert_solid_rgba_image(&renderer.render(&encoded, 2, 2).unwrap(), [0, 0, 0, 0]);
    }

    #[test]
    fn retained_images_survive_cache_clear() {
        let Some(mut renderer) = image_test_renderer() else {
            return;
        };
        let old = retained_image_scene(&mut renderer, [255, 0, 0, 255]);
        assert_solid_rgba_image(&renderer.render(&old, 8, 8).unwrap(), [255, 0, 0, 255]);
        renderer.clear_cached_images();
        let _new = retained_image_scene(&mut renderer, [0, 0, 255, 255]);
        assert_solid_rgba_image(&renderer.render(&old, 8, 8).unwrap(), [255, 0, 0, 255]);
    }

    #[test]
    fn retained_images_survive_cache_eviction() {
        let Some(mut renderer) = image_test_renderer() else {
            return;
        };
        renderer.state.image_registry = HybridImageRegistry::new(0);
        let old = retained_image_scene(&mut renderer, [255, 0, 0, 255]);
        assert_solid_rgba_image(&renderer.render(&old, 8, 8).unwrap(), [255, 0, 0, 255]);
        let _new = retained_image_scene(&mut renderer, [0, 0, 255, 255]);
        assert_solid_rgba_image(&renderer.render(&old, 8, 8).unwrap(), [255, 0, 0, 255]);
    }

    #[test]
    fn reset_releases_retained_image_allocations() {
        let Some(mut renderer) = image_test_renderer() else {
            return;
        };
        let mut scene = retained_image_scene(&mut renderer, [255, 0, 0, 255]);
        let image = Arc::downgrade(&scene.images[0]);
        renderer.clear_cached_images();
        assert!(image.upgrade().is_some());
        scene.reset();
        renderer.clear_cached_images();
        assert!(image.upgrade().is_none());
        assert_solid_rgba_image(&renderer.render(&scene, 8, 8).unwrap(), [0; 4]);
    }

    #[test]
    fn unused_images_are_still_evicted() {
        let Some(mut renderer) = image_test_renderer() else {
            return;
        };
        renderer.state.image_registry = HybridImageRegistry::new(0);
        let scene = retained_image_scene(&mut renderer, [255, 0, 0, 255]);
        let image = Arc::downgrade(&scene.images[0]);
        drop(scene);
        let _new = retained_image_scene(&mut renderer, [0, 0, 255, 255]);
        assert!(image.upgrade().is_none());
    }

    #[test]
    fn retained_images_reject_a_different_renderer() {
        let Some(mut renderer) = image_test_renderer() else {
            return;
        };
        let scene = retained_image_scene(&mut renderer, [255, 0, 0, 255]);
        let mut other =
            VelloHybridRenderer::new(renderer.state.device.clone(), renderer.state.queue.clone());
        assert!(matches!(
            other.render(&scene, 8, 8),
            Err(Error::WrongRenderer)
        ));
        assert_solid_rgba_image(&renderer.render(&scene, 8, 8).unwrap(), [255, 0, 0, 255]);
    }

    #[test]
    fn unfinished_image_recording_is_discarded() {
        let Some(mut renderer) = image_test_renderer() else {
            return;
        };
        let scene = solid_scene(Color::from_rgb8(255, 0, 0), 8.0, 8.0);
        let mut native = renderer.encode_scene(&scene, 8, 8).unwrap();
        {
            let mut sink = VelloHybridSceneSink::with_renderer(&mut native, &mut renderer);
            let brush = ImageBrush::new(ImageData {
                data: Blob::new(Arc::new([0, 0, 255, 255])),
                format: ImageFormat::Rgba8,
                alpha_type: ImageAlphaType::Alpha,
                width: 1,
                height: 1,
            });
            Painter::new(&mut sink).fill_rect(Rect::new(0.0, 0.0, 8.0, 8.0), &brush);
        }
        assert!(native.images.is_empty());
        assert_solid_rgba_image(&renderer.render(&native, 8, 8).unwrap(), [0; 4]);
        let next = retained_image_scene(&mut renderer, [0, 0, 255, 255]);
        assert_solid_rgba_image(&renderer.render(&next, 8, 8).unwrap(), [0, 0, 255, 255]);
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
    fn standalone_sink_glyphs_render_without_renderer_resources() {
        let Ok((device, queue)) = try_init_device_and_queue() else {
            return;
        };
        let mut renderer = VelloHybridRenderer::new(device, queue);
        let font = peniko::FontData::new(
            Blob::new(Arc::new(include_bytes!(
                "../../test_assets/fonts/NotoSans-Regular.ttf"
            ))),
            0,
        );

        for style in [
            peniko::Style::Fill(peniko::Fill::NonZero),
            peniko::Style::Stroke(kurbo::Stroke::new(1.5)),
        ] {
            let mut scene = VelloHybridScene::new(64, 64);
            {
                let mut sink = VelloHybridSceneSink::new(&mut scene);
                Painter::new(&mut sink)
                    .glyphs(&font, Color::BLACK)
                    .font_size(40.0)
                    .transform(kurbo::Affine::translate((8.0, 48.0)))
                    .draw(
                        &style,
                        [imaging::record::Glyph {
                            id: 36,
                            x: 0.0,
                            y: 0.0,
                        }],
                    );
                sink.finish().unwrap();
            }
            let image = renderer.render(&scene, 64, 64).unwrap();
            assert!(
                image.data.chunks_exact(4).any(|pixel| pixel[3] > 0),
                "standalone glyphs must produce visible pixels for {style:?}"
            );
        }
    }

    #[test]
    fn destructive_draws_only_affect_their_geometry() {
        let Ok((device, queue)) = try_init_device_and_queue() else {
            return;
        };
        let mut renderer = VelloHybridRenderer::new(device, queue);
        let mut scene = solid_scene(Color::from_rgb8(255, 0, 0), 32.0, 32.0);
        {
            let mut painter = Painter::new(&mut scene);
            painter
                .fill(
                    Rect::new(4.0, 4.0, 12.0, 12.0),
                    Color::from_rgba8(0, 0, 255, 128),
                )
                .composite(imaging::Composite::new(peniko::Compose::Copy.into(), 1.0))
                .draw();
            painter
                .fill(Rect::new(4.5, 18.0, 12.0, 26.0), Color::TRANSPARENT)
                .composite(imaging::Composite::new(peniko::Compose::Copy.into(), 1.0))
                .draw();
            painter
                .stroke(
                    Rect::new(0.0, 0.0, 10.0, 10.0),
                    &kurbo::Stroke::new(4.0),
                    Color::BLACK,
                )
                .transform(kurbo::Affine::translate((18.0, 4.0)))
                .composite(imaging::Composite::new(peniko::Compose::Clear.into(), 1.0))
                .draw();
        }
        let native = renderer.encode_scene(&scene, 32, 32).unwrap();
        let image = renderer.render(&native, 32, 32).unwrap();
        let pixel = |x: usize, y: usize| &image.data[(y * 32 + x) * 4..(y * 32 + x + 1) * 4];
        assert_eq!(pixel(0, 0), [255, 0, 0, 255]);
        assert_eq!(pixel(6, 6), [0, 0, 255, 128]);
        assert_eq!(pixel(6, 20), [0, 0, 0, 0]);
        assert!(
            (127..=128).contains(&pixel(4, 20)[3]),
            "clip coverage must be applied once"
        );
        assert_eq!(pixel(18, 8), [0, 0, 0, 0]);
        assert_eq!(pixel(23, 9), [255, 0, 0, 255]);
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
