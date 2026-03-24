// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// Portions of this file are derived from `anyrender_skia` in the AnyRender project:
// <https://github.com/dioxuslabs/anyrender>
// Original source: `crates/anyrender_skia/src/metal.rs`
// Adapted here for imaging's offscreen Ganesh renderer.

#![allow(unsafe_code, reason = "Metal and Skia FFI setup requires raw handles")]

#[cfg(feature = "wgpu")]
use core::ffi::c_void;

use objc2::rc::Retained;
use objc2_metal::{MTLCreateSystemDefaultDevice, MTLDevice};
use skia_safe as sk;

use crate::Error;
#[cfg(feature = "wgpu")]
use crate::{
    SkiaRenderer, color_space_for_wgpu_texture_format, color_type_for_wgpu_texture_format,
    ganesh::GaneshBackend,
};

#[derive(Debug)]
pub(crate) struct MetalBackend {
    context: sk::gpu::DirectContext,
}

impl MetalBackend {
    /// Create a Metal-backed Skia Ganesh context for the default system device.
    ///
    /// This is the internal Apple backend path used when `imaging_skia` owns the Metal objects.
    pub(crate) fn new() -> Result<Self, Error> {
        let device = MTLCreateSystemDefaultDevice()
            .ok_or(Error::CreateGpuContext("no Metal device found"))?;
        let command_queue = device.newCommandQueue().ok_or(Error::CreateGpuContext(
            "unable to create Metal command queue",
        ))?;

        let backend = unsafe {
            sk::gpu::mtl::BackendContext::new(
                Retained::as_ptr(&device) as sk::gpu::mtl::Handle,
                Retained::as_ptr(&command_queue) as sk::gpu::mtl::Handle,
            )
        };

        let context = sk::gpu::direct_contexts::make_metal(&backend, None).ok_or(
            Error::CreateGpuContext("unable to create Skia Metal context"),
        )?;

        Ok(Self { context })
    }

    #[cfg(feature = "wgpu")]
    /// Wrap caller-provided Metal device and queue handles in a Skia Ganesh context.
    ///
    /// This is the backend-side bridge for higher-level Metal and `wgpu` interop constructors.
    pub(crate) unsafe fn from_handles(
        device: sk::gpu::mtl::Handle,
        command_queue: sk::gpu::mtl::Handle,
    ) -> Result<Self, Error> {
        let backend = unsafe { sk::gpu::mtl::BackendContext::new(device, command_queue) };

        let context = sk::gpu::direct_contexts::make_metal(&backend, None).ok_or(
            Error::CreateGpuContext("unable to create Skia Metal context"),
        )?;

        Ok(Self { context })
    }

    /// Borrow the underlying Skia direct context for rendering or surface creation.
    pub(crate) fn direct_context(&mut self) -> &mut sk::gpu::DirectContext {
        &mut self.context
    }
}

#[cfg(feature = "wgpu")]
impl SkiaRenderer {
    /// Create an offscreen renderer backed by a caller-owned Metal device and command queue.
    ///
    /// This is the bridge used when the application already chose Metal through `wgpu` and wants
    /// Skia to share that backend ownership model without targeting an existing texture.
    ///
    /// # Safety
    ///
    /// The supplied handles must refer to a valid `MTLDevice` and `MTLCommandQueue` that belong to
    /// the same device and remain valid for the renderer lifetime.
    pub unsafe fn try_new_metal_from_handles(
        width: u16,
        height: u16,
        device: sk::gpu::mtl::Handle,
        command_queue: sk::gpu::mtl::Handle,
    ) -> Result<Self, Error> {
        let width = i32::from(width);
        let height = i32::from(height);
        let mut backend = unsafe { GaneshBackend::from_metal_handles(device, command_queue)? };
        let info = sk::ImageInfo::new(
            (width, height),
            sk::ColorType::RGBA8888,
            sk::AlphaType::Premul,
            None,
        );
        let surface = sk::gpu::surfaces::render_target(
            backend.direct_context(),
            sk::gpu::Budgeted::Yes,
            &info,
            None,
            sk::gpu::SurfaceOrigin::TopLeft,
            None,
            None,
            None,
        )
        .ok_or(Error::CreateGpuSurface)?;
        Ok(Self::from_backend_surface(backend, surface))
    }

    /// Create an offscreen renderer from raw Objective-C pointers to a Metal device and queue.
    ///
    /// This exists for FFI layers that need the device-based Metal bridge but cannot traffic in
    /// typed Metal handles directly.
    ///
    /// # Safety
    ///
    /// `device` and `command_queue` must be valid pointers to `MTLDevice` and `MTLCommandQueue`
    /// that belong to the same device.
    pub unsafe fn try_new_metal_from_raw_pointers_without_texture(
        width: u16,
        height: u16,
        device: *mut c_void,
        command_queue: *mut c_void,
    ) -> Result<Self, Error> {
        unsafe {
            Self::try_new_metal_from_handles(
                width,
                height,
                device.cast_const().cast(),
                command_queue.cast_const().cast(),
            )
        }
    }

    /// Create a renderer that draws directly into a caller-owned Metal texture.
    ///
    /// This is the low-level Apple interop path used when the embedding application already owns
    /// the Metal device, command queue, and render target and wants Skia to render into that
    /// texture instead of allocating its own surface.
    ///
    /// # Safety
    ///
    /// The supplied handles must refer to a valid `MTLDevice`, `MTLCommandQueue`, and 2D renderable
    /// Metal texture that all belong to the same device and remain valid for the renderer lifetime.
    pub unsafe fn try_new_metal_from_handles_and_texture(
        width: u16,
        height: u16,
        texture_format: wgpu::TextureFormat,
        device: sk::gpu::mtl::Handle,
        command_queue: sk::gpu::mtl::Handle,
        texture: sk::gpu::mtl::Handle,
    ) -> Result<Self, Error> {
        let width = i32::from(width);
        let height = i32::from(height);
        let mut backend = unsafe { GaneshBackend::from_metal_handles(device, command_queue)? };
        let surface = create_wrapped_metal_surface(
            backend.direct_context(),
            width,
            height,
            texture_format,
            texture,
        )?;
        Ok(Self::from_backend_surface(backend, surface))
    }

    /// Create a renderer that draws into a caller-owned Metal texture via raw Objective-C pointers.
    ///
    /// This exists for FFI and integration layers that cannot easily traffic in typed
    /// `skia_safe`/Metal handles but still need to attach Skia rendering to an existing texture.
    ///
    /// # Safety
    ///
    /// `device`, `command_queue`, and `texture` must be valid pointers to `MTLDevice`,
    /// `MTLCommandQueue`, and a renderable `MTLTexture` belonging to that device.
    pub unsafe fn try_new_metal_from_raw_pointers(
        width: u16,
        height: u16,
        texture_format: wgpu::TextureFormat,
        device: *mut c_void,
        command_queue: *mut c_void,
        texture: *mut c_void,
    ) -> Result<Self, Error> {
        unsafe {
            Self::try_new_metal_from_handles_and_texture(
                width,
                height,
                texture_format,
                device.cast_const().cast(),
                command_queue.cast_const().cast(),
                texture.cast_const().cast(),
            )
        }
    }

    /// Retarget the renderer to a different caller-owned Metal texture.
    ///
    /// Use this when the embedding application rotates or recreates its own render targets but
    /// wants to keep the same `SkiaRenderer` and Metal context alive.
    ///
    /// # Safety
    ///
    /// The texture handle must be a valid renderable Metal texture created from the same device as
    /// this renderer's Metal context and must remain valid for the wrapped surface lifetime.
    pub unsafe fn replace_metal_texture(
        &mut self,
        width: u16,
        height: u16,
        texture_format: wgpu::TextureFormat,
        texture: sk::gpu::mtl::Handle,
    ) -> Result<(), Error> {
        let width = i32::from(width);
        let height = i32::from(height);
        self.backend.ensure_current()?;
        self.backend.flush_surface(&mut self.surface);
        let surface = create_wrapped_metal_surface(
            self.backend.direct_context(),
            width,
            height,
            texture_format,
            texture,
        )?;
        self.surface = surface;
        Ok(())
    }

    /// Retarget the renderer to a different Metal texture provided as a raw pointer.
    ///
    /// This is the FFI-oriented companion to [`Self::replace_metal_texture`].
    ///
    /// # Safety
    ///
    /// `texture` must be a valid pointer to a renderable `MTLTexture` created from the same
    /// device as this renderer's Metal context.
    pub unsafe fn replace_metal_texture_raw(
        &mut self,
        width: u16,
        height: u16,
        texture_format: wgpu::TextureFormat,
        texture: *mut c_void,
    ) -> Result<(), Error> {
        unsafe {
            self.replace_metal_texture(width, height, texture_format, texture.cast_const().cast())
        }
    }
}

#[cfg(feature = "wgpu")]
/// Wrap a caller-owned Metal texture in a Skia surface for direct rendering.
fn create_wrapped_metal_surface(
    context: &mut sk::gpu::DirectContext,
    width: i32,
    height: i32,
    texture_format: wgpu::TextureFormat,
    texture: sk::gpu::mtl::Handle,
) -> Result<sk::Surface, Error> {
    let texture_info = unsafe { sk::gpu::mtl::TextureInfo::new(texture) };
    let backend_texture = unsafe {
        sk::gpu::backend_textures::make_mtl(
            (width, height),
            sk::gpu::Mipmapped::No,
            &texture_info,
            "ImagingSkiaWgpuWrappedMetalTexture",
        )
    };
    sk::gpu::surfaces::wrap_backend_texture(
        context,
        &backend_texture,
        sk::gpu::SurfaceOrigin::TopLeft,
        None,
        color_type_for_wgpu_texture_format(texture_format)?,
        color_space_for_wgpu_texture_format(texture_format),
        None,
    )
    .ok_or(Error::CreateGpuSurface)
}
