// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `wgpu` texture rendering traits for `imaging` backends.
//!
//! The default feature set enables `wgpu-29`. Consumers that need `wgpu-27` or `wgpu-28`
//! should disable default features and enable the matching version feature.
//!
//! Workspace-wide `--all-features` builds enable multiple lanes, so this crate resolves that case
//! deterministically by exporting `wgpu-29` as [`wgpu`]. Backends that must stay on an older
//! `wgpu` version should use the explicit [`v27`] or [`v28`] module.
//!
//! This crate is `std`-only because it depends on `wgpu`.

#![deny(unsafe_code)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use std::boxed::Box;

#[cfg(not(any(feature = "wgpu-27", feature = "wgpu-28", feature = "wgpu-29")))]
compile_error!("Enable one of `wgpu-27`, `wgpu-28`, or `wgpu-29`.");

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

macro_rules! define_wgpu_lane {
    ($module:ident, $wgpu:ident) => {
        /// Version-specific `wgpu` texture rendering traits and target wrappers.
        pub mod $module {
            use imaging::render::{ImageRenderer, RenderSource};
            use std::vec::Vec;

            pub use $wgpu as wgpu;

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
                ) -> Result<(), crate::TextureRendererError>;

                /// Render a source and return a backend-owned texture.
                fn render_source_texture(
                    &mut self,
                    source: &mut dyn RenderSource,
                    width: u32,
                    height: u32,
                ) -> Result<Self::Texture, crate::TextureRendererError>;
            }
        }
    };
}

#[cfg(feature = "wgpu-27")]
define_wgpu_lane!(v27, wgpu_27);
#[cfg(feature = "wgpu-28")]
define_wgpu_lane!(v28, wgpu_28);
#[cfg(feature = "wgpu-29")]
define_wgpu_lane!(v29, wgpu_29);

#[cfg(all(
    not(any(feature = "wgpu-28", feature = "wgpu-29")),
    feature = "wgpu-27"
))]
pub use v27::{TextureRenderer, TextureViewTarget, wgpu};
#[cfg(all(not(feature = "wgpu-29"), feature = "wgpu-28"))]
pub use v28::{TextureRenderer, TextureViewTarget, wgpu};
#[cfg(feature = "wgpu-29")]
pub use v29::{TextureRenderer, TextureViewTarget, wgpu};
