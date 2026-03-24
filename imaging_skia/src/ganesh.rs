// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Internal Ganesh backend selection and lifecycle management.

use skia_safe as sk;

use crate::Error;
#[cfg(any(target_os = "macos", target_os = "ios"))]
use crate::metal;
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
use crate::opengl;
#[cfg(all(feature = "vulkan", not(any(target_os = "macos", target_os = "ios"))))]
use crate::vulkan;

/// The concrete Skia Ganesh backend driving a `SkiaRenderer`.
pub(crate) enum GaneshBackend {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    /// Internal or externally wrapped Metal backend used on Apple platforms.
    ///
    /// This variant backs both the default Apple renderer path and the Metal-backed `wgpu`
    /// interop constructors.
    Metal(metal::MetalBackend),
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    /// Internal headless OpenGL backend owned entirely by `imaging_skia`.
    ///
    /// This is the default non-Apple backend when the `vulkan` feature is not enabled.
    OpenGl(opengl::OpenGlBackend),
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    /// Skia GL context created from a caller-managed external OpenGL or GLES context.
    ///
    /// This variant is used by explicit GL interop constructors that attach Skia to textures in an
    /// already-current application-owned context instead of creating a headless GL backend.
    ExternalGl(sk::gpu::DirectContext),
    #[cfg(all(feature = "vulkan", not(any(target_os = "macos", target_os = "ios"))))]
    /// Internal or externally wrapped Vulkan backend used on non-Apple Vulkan builds.
    ///
    /// This variant backs both the default Vulkan renderer path and Vulkan-based `wgpu` interop.
    Vulkan(vulkan::VulkanBackend),
}

impl core::fmt::Debug for GaneshBackend {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            Self::Metal(_) => f.write_str("GaneshBackend::Metal"),
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            Self::OpenGl(_) => f.write_str("GaneshBackend::OpenGl"),
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            Self::ExternalGl(_) => f.write_str("GaneshBackend::ExternalGl"),
            #[cfg(all(feature = "vulkan", not(any(target_os = "macos", target_os = "ios"))))]
            Self::Vulkan(_) => f.write_str("GaneshBackend::Vulkan"),
        }
    }
}

impl GaneshBackend {
    /// Create the default Ganesh backend for the current platform and feature set.
    pub(crate) fn new() -> Result<Self, Error> {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        {
            return Ok(Self::Metal(metal::MetalBackend::new()?));
        }
        #[cfg(all(
            not(any(target_os = "macos", target_os = "ios")),
            not(feature = "vulkan")
        ))]
        {
            return Ok(Self::OpenGl(opengl::OpenGlBackend::new()?));
        }
        #[cfg(all(feature = "vulkan", not(any(target_os = "macos", target_os = "ios"))))]
        {
            return Ok(Self::Vulkan(vulkan::VulkanBackend::new()?));
        }
        #[allow(
            unreachable_code,
            reason = "Platform/feature cfgs can make all concrete branches unavailable."
        )]
        Err(Error::UnsupportedGpuBackend)
    }

    /// Borrow the active Skia direct context from the selected backend.
    pub(crate) fn direct_context(&mut self) -> &mut sk::gpu::DirectContext {
        match self {
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            Self::Metal(backend) => backend.direct_context(),
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            Self::OpenGl(backend) => backend.direct_context(),
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            Self::ExternalGl(context) => context,
            #[cfg(all(feature = "vulkan", not(any(target_os = "macos", target_os = "ios"))))]
            Self::Vulkan(backend) => backend.direct_context(),
        }
    }

    #[cfg(all(feature = "wgpu", any(target_os = "macos", target_os = "ios")))]
    /// Wrap caller-provided Metal device and queue handles in a Ganesh backend.
    pub(crate) unsafe fn from_metal_handles(
        device: sk::gpu::mtl::Handle,
        command_queue: sk::gpu::mtl::Handle,
    ) -> Result<Self, Error> {
        Ok(Self::Metal(unsafe {
            metal::MetalBackend::from_handles(device, command_queue)?
        }))
    }

    /// Ensure the backend's graphics context is current when the platform requires it.
    pub(crate) fn ensure_current(&mut self) -> Result<(), Error> {
        match self {
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            Self::Metal(_) => Ok(()),
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            Self::OpenGl(backend) => backend.ensure_current(),
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            Self::ExternalGl(_) => Ok(()),
            #[cfg(all(feature = "vulkan", not(any(target_os = "macos", target_os = "ios"))))]
            Self::Vulkan(_) => Ok(()),
        }
    }

    /// Flush pending drawing work for the given surface through the active backend.
    pub(crate) fn flush_surface(&mut self, surface: &mut sk::Surface) {
        self.direct_context()
            .flush_and_submit_surface(surface, sk::gpu::SyncCpu::No);
    }
}

/// Allocate the default offscreen Ganesh surface used by owned GPU renderer paths.
pub(crate) fn create_surface(
    context: &mut sk::gpu::DirectContext,
    width: i32,
    height: i32,
) -> Result<sk::Surface, Error> {
    let info = sk::ImageInfo::new(
        (width, height),
        sk::ColorType::RGBA8888,
        sk::AlphaType::Premul,
        None,
    );
    sk::gpu::surfaces::render_target(
        context,
        sk::gpu::Budgeted::Yes,
        &info,
        None,
        sk::gpu::SurfaceOrigin::TopLeft,
        None,
        None,
        None,
    )
    .ok_or(Error::CreateGpuSurface)
}
