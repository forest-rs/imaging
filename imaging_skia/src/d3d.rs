// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

#![allow(unsafe_code, reason = "D3D12 interop requires raw handle bridging")]

use skia_safe as sk;
use windows::Win32::Graphics::{
    Direct3D12::{D3D12_RESOURCE_STATE_COMMON, ID3D12CommandQueue, ID3D12Device, ID3D12Resource},
    Dxgi::{
        Common::{
            DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
            DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
            DXGI_FORMAT_R10G10B10A2_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT,
            DXGI_FORMAT_R16G16B16A16_UNORM,
        },
        IDXGIAdapter1,
    },
};
use windows::core::Interface as _;

use crate::{Error, color_space_for_wgpu_texture_format, color_type_for_wgpu_texture_format};

#[derive(Debug)]
pub(crate) struct Dx12Backend {
    context: sk::gpu::DirectContext,
}

impl Dx12Backend {
    pub(crate) fn from_wgpu(
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
    ) -> Result<Self, Error> {
        let adapter = unsafe {
            adapter
                .as_hal::<wgpu::hal::api::Dx12>()
                .ok_or(Error::CreateGpuContext("missing D3D12 adapter"))?
        };
        let device = unsafe {
            device
                .as_hal::<wgpu::hal::api::Dx12>()
                .ok_or(Error::CreateGpuContext("missing D3D12 device"))?
        };

        let raw_adapter: IDXGIAdapter1 = adapter
            .raw_adapter()
            .clone()
            .cast()
            .map_err(|_| Error::CreateGpuContext("unable to retrieve D3D12 adapter"))?;
        let raw_device: ID3D12Device = device.raw_device().clone();
        let raw_queue: ID3D12CommandQueue = device.raw_queue().clone();
        let backend_context = sk::gpu::d3d::BackendContext {
            adapter: raw_adapter,
            device: raw_device,
            queue: raw_queue,
            memory_allocator: None,
            protected_context: sk::gpu::Protected::No,
        };
        let context = unsafe { sk::gpu::DirectContext::new_d3d(&backend_context, None) }.ok_or(
            Error::CreateGpuContext("unable to create Skia D3D12 context"),
        )?;
        Ok(Self { context })
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
                .as_hal::<wgpu::hal::api::Dx12>()
                .ok_or(Error::CreateGpuSurface)?
        };
        let resource: ID3D12Resource = unsafe { hal_texture.raw_resource().clone() };
        let texture_info = sk::gpu::d3d::TextureResourceInfo::from_resource(resource)
            .with_state(D3D12_RESOURCE_STATE_COMMON);
        let backend_texture = sk::gpu::BackendTexture::new_d3d(
            (width, height),
            &sk::gpu::d3d::TextureResourceInfo {
                format: dxgi_format_for_wgpu_texture_format(format)?,
                level_count: 1,
                ..texture_info
            },
        );
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
}

pub(crate) fn supports_texture_format(texture_format: wgpu::TextureFormat) -> Result<(), Error> {
    let _ = dxgi_format_for_wgpu_texture_format(texture_format)?;
    Ok(())
}

fn dxgi_format_for_wgpu_texture_format(
    texture_format: wgpu::TextureFormat,
) -> Result<DXGI_FORMAT, Error> {
    // Keep this as a local allowlist rather than forwarding to wgpu-hal's broader DXGI mapping.
    // Skia Ganesh here only supports a small set of render-target formats we intend to expose, and
    // spelling them out keeps that contract obvious at the call site.
    Ok(match texture_format {
        wgpu::TextureFormat::Rgba8Unorm => DXGI_FORMAT_R8G8B8A8_UNORM,
        wgpu::TextureFormat::Rgba8UnormSrgb => DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
        wgpu::TextureFormat::Bgra8Unorm => DXGI_FORMAT_B8G8R8A8_UNORM,
        wgpu::TextureFormat::Bgra8UnormSrgb => DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
        wgpu::TextureFormat::Rgb10a2Unorm => DXGI_FORMAT_R10G10B10A2_UNORM,
        wgpu::TextureFormat::Rgba16Unorm => DXGI_FORMAT_R16G16B16A16_UNORM,
        wgpu::TextureFormat::Rgba16Float => DXGI_FORMAT_R16G16B16A16_FLOAT,
        _ => return Err(Error::UnsupportedGpuTextureFormat),
    })
}
