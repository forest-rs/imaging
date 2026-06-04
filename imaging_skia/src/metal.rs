// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

#![allow(unsafe_code, reason = "Metal interop requires raw handle bridging")]

use core::ptr::NonNull;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal::{MTLCommandQueue, MTLDevice as _};
use skia_safe as sk;

use crate::{Error, color_space_for_wgpu_texture_format, color_type_for_wgpu_texture_format};

#[derive(Debug)]
pub(crate) struct MetalBackend {
    context: sk::gpu::DirectContext,
    _command_queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
}

impl MetalBackend {
    pub(crate) fn from_wgpu(device: &wgpu::Device, _queue: &wgpu::Queue) -> Result<Self, Error> {
        let device = unsafe {
            device
                .as_hal::<wgpu::hal::api::Metal>()
                .ok_or(Error::CreateGpuContext("missing Metal device"))?
        };
        let command_queue =
            device
                .raw_device()
                .newCommandQueue()
                .ok_or(Error::CreateGpuContext(
                    "unable to create Metal command queue",
                ))?;

        let backend = unsafe {
            sk::gpu::mtl::BackendContext::new(
                Retained::as_ptr(device.raw_device()) as sk::gpu::mtl::Handle,
                Retained::as_ptr(&command_queue) as sk::gpu::mtl::Handle,
            )
        };
        let context = sk::gpu::direct_contexts::make_metal(&backend, None).ok_or(
            Error::CreateGpuContext("unable to create Skia Metal context"),
        )?;
        Ok(Self {
            context,
            _command_queue: command_queue,
        })
    }

    pub(crate) fn direct_context(&mut self) -> &mut sk::gpu::DirectContext {
        &mut self.context
    }

    pub(crate) fn wrap_texture(&mut self, texture: &wgpu::Texture) -> Result<sk::Surface, Error> {
        let width = i32::try_from(texture.width())
            .map_err(|_| Error::Internal("texture width overflow"))?;
        let height = i32::try_from(texture.height())
            .map_err(|_| Error::Internal("texture height overflow"))?;
        let format = texture.format();
        let hal_texture = unsafe {
            texture
                .as_hal::<wgpu::hal::api::Metal>()
                .ok_or(Error::CreateGpuSurface)?
        };
        let texture_handle = NonNull::from(hal_texture.raw_handle()).as_ptr();
        let texture_info = unsafe { sk::gpu::mtl::TextureInfo::new(texture_handle as _) };
        let backend_texture = unsafe {
            sk::gpu::backend_textures::make_mtl(
                (width, height),
                sk::gpu::Mipmapped::No,
                &texture_info,
                "imaging_skia metal texture",
            )
        };
        sk::gpu::surfaces::wrap_backend_texture(
            self.direct_context(),
            &backend_texture,
            sk::gpu::SurfaceOrigin::TopLeft,
            0,
            color_type_for_wgpu_texture_format(format)?,
            color_space_for_wgpu_texture_format(format),
            None,
        )
        .ok_or(Error::CreateGpuSurface)
    }

    pub(crate) fn supports_texture_format(
        texture_format: wgpu::TextureFormat,
    ) -> Result<(), Error> {
        match texture_format {
            wgpu::TextureFormat::Rgba8Unorm
            | wgpu::TextureFormat::Bgra8Unorm
            | wgpu::TextureFormat::Rgb10a2Unorm
            | wgpu::TextureFormat::Rgba16Float => Ok(()),
            _ => Err(Error::UnsupportedGpuTextureFormat),
        }
    }
}
