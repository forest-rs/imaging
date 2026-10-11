// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Vello backend for `imaging`.
//!
//! This crate provides a headless CPU/GPU renderer that consumes encoded [`VelloScene`] values
//! and renders them to GPU targets or RGBA8 image data using `vello` + `wgpu`.
//!
//! Semantic [`imaging::record::Scene`] values can be lowered to native Vello scenes through
//! [`VelloRenderer::encode_scene`].
//!
//! In UI integrations, the host application should usually own the `wgpu` device, queue, and
//! presentation targets, then pass those handles into [`VelloRenderer`].
//!
//! This crate uses Vello 0.11 with `wgpu` 30.
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
//! # Record An Encoded Scene
//!
//! If you want a backend-native retained scene without going through a renderer, wrap a
//! mutable [`VelloScene`] with [`VelloSceneSink`].
//!
//! ```no_run
//! use imaging::Painter;
//! use imaging_vello::{VelloSceneSink, VelloScene};
//! use kurbo::Rect;
//! use peniko::{Brush, Color};
//!
//! fn main() -> Result<(), imaging_vello::Error> {
//!     let paint = Brush::Solid(Color::from_rgb8(0x1d, 0x4e, 0x89));
//!     let mut scene = VelloScene::new();
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
//! # Render An Encoded Scene
//!
//! If you already have a native Vello scene, hand it directly to [`VelloRenderer`].
//!
//! ```no_run
//! use imaging::Painter;
//! use imaging_vello::{VelloRenderer, VelloSceneSink, VelloScene};
//! use kurbo::Rect;
//! use peniko::{Brush, Color};
//!
//! fn main() -> Result<(), imaging_vello::Error> {
//!     let paint = Brush::Solid(Color::from_rgb8(0xd9, 0x77, 0x06));
//!     let mut scene = VelloScene::new();
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
//! # let mut renderer: imaging_vello::VelloRenderer = todo!();
//! # let texture: imaging_vello::wgpu::Texture = todo!();
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

mod scene;
mod scene_sink;
pub use scene::VelloScene;
mod wgpu_support;

use imaging::RgbaImage;
use imaging::record::{Scene, ValidateError, replay};
use imaging::render::{
    GpuReadbackError, ImageBufferFormat, ImageBufferTarget, ImageRenderer, ImageRendererError,
    ImageTargetError, RenderContentError, RenderSource, RenderUnsupportedError,
};
use imaging_wgpu::{TextureRenderer, TextureRendererError, TextureTargetError, TextureViewTarget};
use kurbo::Rect;
use std::{collections::HashMap, sync::Arc};

pub use vello;

pub use crate::vello::wgpu;
use crate::vello::{AaConfig, RenderParams};
use crate::wgpu_support::{
    OffscreenTarget, ReadbackError, create_texture, read_texture_into, read_texture_into_target,
};

pub use scene_sink::VelloSceneSink;

/// Errors that can occur when rendering via Vello.
#[derive(Debug)]
pub enum Error {
    /// The scene is invalid (unbalanced stacks).
    InvalidScene(ValidateError),
    /// An external image has no binding on this renderer.
    UnboundImage(imaging::ExternalImage),
    /// The encoded scene belongs to a different renderer.
    WrongRenderer,
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

#[derive(Debug)]
pub(crate) struct ExternalTexture {
    image: peniko::ImageData,
    texture: wgpu::Texture,
}

struct VelloRendererState {
    identity: Arc<()>,
    external_images: HashMap<imaging::ExternalImage, Arc<ExternalTexture>>,
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
            identity: Arc::new(()),
            external_images: HashMap::new(),
            renderer,
            device,
            queue,
        })
    }

    /// Lower a semantic [`imaging::record::Scene`] into a encoded [`VelloScene`].
    fn encode_scene(&self, scene: &Scene, width: u32, height: u32) -> Result<VelloScene, Error> {
        scene.validate().map_err(Error::InvalidScene)?;
        let mut native = VelloScene::new();
        let bounds = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        let mut sink = VelloSceneSink::with_bindings(
            &mut native,
            bounds,
            &self.external_images,
            &self.identity,
        );
        replay(scene, &mut sink);
        sink.finish()?;
        Ok(native)
    }

    fn encode_source<S: RenderSource + ?Sized>(
        &self,
        source: &mut S,
        width: u32,
        height: u32,
    ) -> Result<VelloScene, Error> {
        source.validate().map_err(Error::InvalidScene)?;
        let mut native = VelloScene::new();
        let bounds = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        let mut sink = VelloSceneSink::with_bindings(
            &mut native,
            bounds,
            &self.external_images,
            &self.identity,
        );
        source.paint_into(&mut sink);
        sink.finish()?;
        Ok(native)
    }

    fn render_to_view(
        &mut self,
        scene: &VelloScene,
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

        if let Some(identity) = &scene.renderer
            && !Arc::ptr_eq(identity, &self.identity)
        {
            return Err(Error::WrongRenderer);
        }
        // Overrides are scoped to this submission. Reinstalling marks atlas pixels dirty, so
        // caller-owned textures updated between renders are always recopied.
        for binding in &scene.images {
            self.renderer.override_image(
                &binding.image,
                Some(wgpu::TexelCopyTextureInfoBase {
                    texture: binding.texture.clone(),
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                }),
            );
        }
        let result = self
            .renderer
            .render_to_texture(
                &self.device,
                &self.queue,
                &scene.native,
                texture_view,
                &params,
            )
            .map_err(Error::Render);
        for binding in &scene.images {
            self.renderer.override_image(&binding.image, None);
        }
        result
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

    /// Lower a semantic [`imaging::record::Scene`] into a encoded [`VelloScene`].
    pub fn encode_scene(
        &self,
        scene: &Scene,
        width: u32,
        height: u32,
    ) -> Result<VelloScene, Error> {
        self.state.encode_scene(scene, width, height)
    }

    fn encode_source<S: RenderSource + ?Sized>(
        &mut self,
        source: &mut S,
        width: u32,
        height: u32,
    ) -> Result<VelloScene, Error> {
        self.state.encode_source(source, width, height)
    }

    /// Render a encoded [`VelloScene`] into a caller-provided texture view.
    pub fn render_to_texture_view(
        &mut self,
        scene: &VelloScene,
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
        let (target_width, target_height) =
            VelloRendererState::checked_size(width, height).map_err(map_texture_renderer_error)?;
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

impl VelloRenderer {
    /// Render a encoded [`VelloScene`] into an RGBA8 image (unpremultiplied).
    pub fn render_into(
        &mut self,
        scene: &VelloScene,
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

    /// Render a encoded [`VelloScene`] and return an RGBA8 image (unpremultiplied).
    pub fn render(
        &mut self,
        scene: &VelloScene,
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
        Error::UnboundImage(image) => {
            TextureRendererError::Content(RenderContentError::UnboundImage(image))
        }
        Error::InvalidScene(error) => {
            TextureRendererError::Content(RenderContentError::InvalidScene(error))
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

/// Compute Vello external images use RGBA8 textures with `COPY_SRC` usage.
impl imaging_wgpu::TextureImageRenderer for VelloRenderer {
    fn bind_image(
        &mut self,
        handle: &imaging::ExternalImage,
        texture: &wgpu::Texture,
    ) -> Result<(), imaging_wgpu::TextureImageError> {
        use imaging_wgpu::TextureImageError;
        imaging_wgpu::validate_image_texture(handle, texture)?;
        if texture.format() != wgpu::TextureFormat::Rgba8Unorm {
            return Err(TextureImageError::UnsupportedFormat);
        }
        if !texture.usage().contains(wgpu::TextureUsages::COPY_SRC) {
            return Err(TextureImageError::InvalidTexture(
                "Vello external images require COPY_SRC usage",
            ));
        }
        if self.state.external_images.contains_key(handle) {
            return Err(TextureImageError::AlreadyBound);
        }
        // The placeholder stays inside this adapter. Every submission installs its GPU override
        // before Vello can resolve it; no fabricated raster pixels enter the common recording.
        let image = peniko::ImageData {
            data: peniko::Blob::new(Arc::new([])),
            format: peniko::ImageFormat::Rgba8,
            alpha_type: handle.alpha_type(),
            width: handle.width(),
            height: handle.height(),
        };
        self.state.external_images.insert(
            handle.clone(),
            Arc::new(ExternalTexture {
                image,
                texture: texture.clone(),
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
    use imaging::{
        Painter,
        render::{ImageBufferTarget, ImageTargetError},
    };
    use kurbo::Rect;
    use peniko::Color;
    use pollster::block_on;

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
                    label: Some("imaging_vello test device"),
                    required_features: wgpu::Features::empty(),
                    ..Default::default()
                })
                .await
                .map_err(|_| ())
        })
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
        let mut renderer = VelloRenderer::new(device.clone(), queue.clone()).unwrap();
        let descriptor = wgpu::TextureDescriptor {
            label: Some("external image test"),
            size: wgpu::Extent3d {
                width: 2,
                height: 2,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
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
        let handle = ExternalImage::new(2, 2, peniko::ImageAlphaType::AlphaPremultiplied);
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
            let mut sink = VelloSceneSink::with_renderer(
                &mut appended,
                Rect::new(0.0, 0.0, 2.0, 2.0),
                &renderer,
            );
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
        let mut other = VelloRenderer::new(device.clone(), queue.clone()).unwrap();
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
    #[cfg_attr(skip_gpu_tests, ignore)]
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
    #[cfg_attr(skip_gpu_tests, ignore)]
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
    #[cfg_attr(skip_gpu_tests, ignore)]
    fn texture_view_render_smoke() {
        let Ok((device, queue)) = try_init_device_and_queue() else {
            return;
        };
        let mut renderer = VelloRenderer::new(device.clone(), queue.clone()).unwrap();

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

        let mut image = RgbaImage::new(32, 32);
        read_texture_into(&device, &queue, &texture, 32, 32, &mut image).unwrap();
        assert_solid_rgba_image(&image, [0x1d, 0x4e, 0x89, 0xff]);
    }

    #[test]
    #[cfg_attr(skip_gpu_tests, ignore)]
    fn render_source_to_texture_smoke() {
        let Ok((device, queue)) = try_init_device_and_queue() else {
            return;
        };
        let mut renderer = VelloRenderer::new(device.clone(), queue.clone()).unwrap();

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

        let mut image = RgbaImage::new(24, 24);
        read_texture_into(&device, &queue, &texture, 24, 24, &mut image).unwrap();
        assert_solid_rgba_image(&image, [0x1d, 0x4e, 0x89, 0xff]);
    }

    #[test]
    #[cfg_attr(skip_gpu_tests, ignore)]
    fn render_source_texture_returns_independent_texture() {
        let Ok((device, queue)) = try_init_device_and_queue() else {
            return;
        };
        let mut renderer = VelloRenderer::new(device.clone(), queue.clone()).unwrap();

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
    #[cfg_attr(skip_gpu_tests, ignore)]
    fn render_source_into_rejects_short_row_stride_as_target_error() {
        let Ok((device, queue)) = try_init_device_and_queue() else {
            return;
        };
        let mut renderer = VelloRenderer::new(device, queue).unwrap();
        let scene = solid_scene(Color::from_rgb8(0x2a, 0x6f, 0xdb), 4.0, 4.0);
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
                "image target row stride is smaller than the rendered width",
            ))
        ));
    }

    #[test]
    #[cfg_attr(skip_gpu_tests, ignore)]
    fn render_source_into_rejects_short_buffer_as_target_error() {
        let Ok((device, queue)) = try_init_device_and_queue() else {
            return;
        };
        let mut renderer = VelloRenderer::new(device, queue).unwrap();
        let scene = solid_scene(Color::from_rgb8(0x2a, 0x6f, 0xdb), 4.0, 4.0);
        let mut data = vec![0; 15];
        let mut source = &scene;

        let error = ImageRenderer::render_source_into(
            &mut renderer,
            &mut source,
            ImageBufferTarget {
                data: &mut data,
                width: 4,
                height: 4,
                bytes_per_row: 4 * 4,
                format: ImageBufferFormat::Rgba8Unorm,
            },
        )
        .unwrap_err();

        assert!(matches!(
            error,
            ImageRendererError::Target(ImageTargetError::InvalidTargetBuffer)
        ));
    }

    #[test]
    fn supported_texture_formats_are_rgba8_only() {
        assert_eq!(
            supported_texture_formats(),
            vec![wgpu::TextureFormat::Rgba8Unorm]
        );
    }

    #[test]
    #[cfg_attr(skip_gpu_tests, ignore)]
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
    #[test]
    #[cfg_attr(skip_gpu_tests, ignore)]
    fn glyph_composite_alpha_is_applied_once() {
        let Ok((device, queue)) = try_init_device_and_queue() else {
            assert!(
                std::env::var("IMAGING_CI_GPU_SUPPORT").as_deref() != Ok("yes"),
                "GPU initialization failed"
            );
            return;
        };
        let mut renderer = VelloRenderer::new(device, queue).unwrap();
        let font = peniko::FontData::new(
            peniko::Blob::new(Arc::new(
                include_bytes!("../../test_assets/fonts/NotoSans-Regular.ttf").as_slice(),
            )),
            0,
        );
        let brushes = [
            peniko::Brush::Solid(Color::WHITE),
            peniko::Brush::Gradient(
                peniko::Gradient::new_linear((0.0, 0.0), (64.0, 0.0))
                    .with_stops([Color::WHITE, Color::BLACK]),
            ),
            peniko::Brush::Image(peniko::ImageBrush::new(peniko::ImageData {
                data: peniko::Blob::new(Arc::new([255_u8; 4])),
                format: peniko::ImageFormat::Rgba8,
                alpha_type: peniko::ImageAlphaType::Alpha,
                width: 1,
                height: 1,
            })),
        ];
        for brush in &brushes {
            for style in [
                peniko::Style::Fill(peniko::Fill::NonZero),
                peniko::Style::Stroke(kurbo::Stroke::new(3.0)),
            ] {
                let mut scene = Scene::new();
                Painter::new(&mut scene)
                    .glyphs(&font, brush)
                    .font_size(48.0)
                    .composite(imaging::Composite {
                        alpha: 0.5,
                        ..Default::default()
                    })
                    .draw(
                        &style,
                        [imaging::record::Glyph {
                            id: 36,
                            x: 8.0,
                            y: 52.0,
                        }],
                    );
                let native = renderer.encode_scene(&scene, 64, 64).unwrap();
                let image = renderer.render(&native, 64, 64).unwrap();
                let alpha = image
                    .data
                    .chunks_exact(4)
                    .map(|pixel| pixel[3])
                    .max()
                    .unwrap();
                assert!(
                    (127..=128).contains(&alpha),
                    "{brush:?}/{style:?}: maximum alpha was {alpha}"
                );
            }
        }
    }
}
