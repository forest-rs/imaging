// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `wgpu` texture rendering traits for `imaging` backends.
//!
//! These traits use `wgpu` 30.
//!
//! This crate is `std`-only because it depends on `wgpu`.

#![deny(unsafe_code)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use imaging::render::{ImageRenderer, RenderSource};
use std::boxed::Box;

pub use wgpu;

/// Failure to bind a texture to an external image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextureImageError {
    /// Texture dimensions, usage, or layout do not match the image.
    InvalidTexture(&'static str),
    /// The renderer cannot sample this texture format.
    UnsupportedFormat,
    /// The renderer cannot sample this alpha encoding without conversion.
    UnsupportedAlphaType,
    /// This handle already has a binding; unbind it before binding another resource.
    AlreadyBound,
}

impl core::fmt::Display for TextureImageError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidTexture(message) => f.write_str(message),
            Self::UnsupportedFormat => f.write_str("unsupported external image texture format"),
            Self::UnsupportedAlphaType => f.write_str("unsupported external image alpha encoding"),
            Self::AlreadyBound => f.write_str("external image is already bound"),
        }
    }
}
impl core::error::Error for TextureImageError {}

/// Renderer capability for sampling caller-owned GPU images.
///
/// Bindings use the whole base mip of a single-layer, single-sample 2D texture from the same
/// device as the renderer. Pixel dimensions and alpha encoding come from the image handle;
/// texture formats and required usage flags are backend-specific.
///
/// Encoded scenes retain the binding they used. Unbinding prevents subsequent encodings from
/// resolving the handle, while existing encoded scenes continue to work. Texture contents remain
/// live: submit writes on the renderer's queue before rendering scenes that sample them.
pub trait TextureImageRenderer {
    /// Bind a texture to an image handle, retaining a reference to the texture.
    fn bind_image(
        &mut self,
        image: &imaging::ExternalImage,
        texture: &wgpu::Texture,
    ) -> Result<(), TextureImageError>;

    /// Remove a binding for future scene encodings. Return whether it existed.
    fn unbind_image(&mut self, image: &imaging::ExternalImage) -> bool;
}

/// Validate the dimensions and base texture layout shared by texture-image backends.
pub fn validate_image_texture(
    image: &imaging::ExternalImage,
    texture: &wgpu::Texture,
) -> Result<(), TextureImageError> {
    if image.width() == 0
        || image.height() == 0
        || texture.width() != image.width()
        || texture.height() != image.height()
    {
        return Err(TextureImageError::InvalidTexture(
            "external image dimensions must match a nonempty texture",
        ));
    }
    if texture.dimension() != wgpu::TextureDimension::D2
        || texture.depth_or_array_layers() != 1
        || texture.sample_count() != 1
    {
        return Err(TextureImageError::InvalidTexture(
            "external images require a single-layer, single-sample 2D texture",
        ));
    }
    Ok(())
}

/// Shared texture-render-target failures surfaced by texture renderers.
#[derive(Debug)]
pub enum TextureTargetError {
    /// The supplied texture target is not compatible with the renderer.
    InvalidTarget(&'static str),
    /// The requested render dimensions exceed backend limits.
    DimensionsTooLarge,
    /// The backend could not render to the requested texture format.
    UnsupportedTextureFormat,
    /// The backend could not construct a GPU interop context from the supplied handles.
    CreateGpuContext(&'static str),
    /// The backend could not wrap the target texture as a GPU render surface.
    CreateGpuSurface,
    /// No supported GPU backend was available for the supplied `wgpu` setup.
    UnsupportedGpuBackend,
}

impl core::fmt::Display for TextureTargetError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidTarget(message) => f.write_str(message),
            Self::DimensionsTooLarge => f.write_str("render dimensions exceed backend limits"),
            Self::UnsupportedTextureFormat => {
                f.write_str("backend does not support the requested texture format")
            }
            Self::CreateGpuContext(message) => f.write_str(message),
            Self::CreateGpuSurface => {
                f.write_str("backend could not wrap the texture as a GPU render surface")
            }
            Self::UnsupportedGpuBackend => {
                f.write_str("no supported GPU backend was available for the supplied wgpu setup")
            }
        }
    }
}

impl core::error::Error for TextureTargetError {}

/// Shared texture-rendering error type for `wgpu` texture renderers.
#[derive(Debug)]
pub enum TextureRendererError {
    /// Source/content-related failure.
    Content(imaging::render::RenderContentError),
    /// Caller target-related failure.
    Target(TextureTargetError),
    /// Unsupported-feature failure.
    Unsupported(imaging::render::RenderUnsupportedError),
    /// Backend-specific rendering error.
    Backend(Box<dyn core::error::Error + Send + Sync + 'static>),
}

impl TextureRendererError {
    /// Box a backend-specific error value.
    #[must_use]
    pub fn backend(error: impl core::error::Error + Send + Sync + 'static) -> Self {
        Self::Backend(Box::new(error))
    }
}

impl core::fmt::Display for TextureRendererError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Content(error) => core::fmt::Display::fmt(error, f),
            Self::Target(error) => core::fmt::Display::fmt(error, f),
            Self::Unsupported(error) => core::fmt::Display::fmt(error, f),
            Self::Backend(error) => core::fmt::Display::fmt(error, f),
        }
    }
}

impl core::error::Error for TextureRendererError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Content(error) => Some(error),
            Self::Target(error) => Some(error),
            Self::Unsupported(error) => Some(error),
            Self::Backend(error) => Some(error.as_ref()),
        }
    }
}

/// Shared `wgpu` texture-view render target used by view-based [`TextureRenderer`]
/// backends.
#[derive(Clone, Debug)]
pub struct TextureViewTarget {
    /// Render-target texture view.
    pub view: wgpu::TextureView,
    /// Effective render width in pixels.
    pub width: u32,
    /// Effective render height in pixels.
    pub height: u32,
}

impl TextureViewTarget {
    /// Create a shared texture-view render target wrapper.
    #[must_use]
    pub fn new(view: &wgpu::TextureView, width: u32, height: u32) -> Self {
        Self {
            view: view.clone(),
            width,
            height,
        }
    }
}

/// Renderer capability for drawing a [`RenderSource`] into a backend-selected `wgpu`
/// texture target.
///
/// These methods intentionally use names distinct from
/// [`imaging::render::ImageRenderer`] so renderers that implement both traits can be
/// called without fully qualified syntax.
pub trait TextureRenderer: ImageRenderer {
    /// Concrete caller-owned target type consumed by this renderer.
    type TextureTarget;

    /// Owned texture type returned by [`Self::render_source_texture`].
    type Texture;

    /// Return the `wgpu` texture formats this renderer can draw into directly.
    ///
    /// Callers can use this to preflight swapchain or offscreen target formats before
    /// calling [`Self::render_source_into_texture`].
    fn supported_texture_formats(&self) -> Vec<wgpu::TextureFormat>;

    /// Render a source into a caller-provided texture target.
    ///
    /// Renderers should treat the target as a fresh output and may clear or overwrite
    /// any existing contents before drawing.
    fn render_source_into_texture(
        &mut self,
        source: &mut dyn RenderSource,
        target: Self::TextureTarget,
    ) -> Result<(), TextureRendererError>;

    /// Render a source and return a backend-owned texture.
    fn render_source_texture(
        &mut self,
        source: &mut dyn RenderSource,
        width: u32,
        height: u32,
    ) -> Result<Self::Texture, TextureRendererError>;
}
