// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Internal Ganesh backend selection and texture wrapping.

use skia_safe as sk;

use crate::Error;
#[cfg(all(windows, feature = "gpu"))]
use crate::d3d::Dx12Backend;
#[cfg(all(any(target_os = "macos", target_os = "ios"), feature = "gpu"))]
use crate::metal::MetalBackend;
#[cfg(all(not(any(target_os = "macos", target_os = "ios")), feature = "gpu"))]
use crate::vulkan::VulkanBackend;

pub(crate) enum GaneshBackend {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    Metal(MetalBackend),
    #[cfg(windows)]
    Dx12(Dx12Backend),
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    Vulkan(VulkanBackend),
}

impl core::fmt::Debug for GaneshBackend {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            Self::Metal(_) => f.write_str("GaneshBackend::Metal"),
            #[cfg(windows)]
            Self::Dx12(_) => f.write_str("GaneshBackend::Dx12"),
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            Self::Vulkan(_) => f.write_str("GaneshBackend::Vulkan"),
        }
    }
}

impl GaneshBackend {
    pub(crate) fn from_wgpu(
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<Self, Error> {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        {
            let _ = adapter;
            return Ok(Self::Metal(MetalBackend::from_wgpu(device, queue)?));
        }
        #[cfg(windows)]
        {
            return match adapter.get_info().backend {
                wgpu::Backend::Dx12 => {
                    Ok(Self::Dx12(Dx12Backend::from_wgpu(adapter, device, queue)?))
                }
                wgpu::Backend::Vulkan => Ok(Self::Vulkan(VulkanBackend::from_wgpu(device, queue)?)),
                _ => Err(Error::UnsupportedGpuBackend),
            };
        }
        #[cfg(not(any(target_os = "macos", target_os = "ios", windows)))]
        {
            let _ = adapter;
            return Ok(Self::Vulkan(VulkanBackend::from_wgpu(device, queue)?));
        }
        #[allow(
            unreachable_code,
            reason = "platform cfgs can compile out all concrete backend branches"
        )]
        Err(Error::UnsupportedGpuBackend)
    }

    pub(crate) fn direct_context(&mut self) -> &mut sk::gpu::DirectContext {
        match self {
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            Self::Metal(backend) => backend.direct_context(),
            #[cfg(windows)]
            Self::Dx12(backend) => backend.direct_context(),
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            Self::Vulkan(backend) => backend.direct_context(),
        }
    }

    pub(crate) fn wrap_texture(&mut self, texture: &wgpu::Texture) -> Result<sk::Surface, Error> {
        match self {
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            Self::Metal(backend) => backend.wrap_texture(texture),
            #[cfg(windows)]
            Self::Dx12(backend) => backend.wrap_texture(texture),
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            Self::Vulkan(backend) => backend.wrap_texture(texture),
        }
    }

    pub(crate) fn flush_surface(&mut self, surface: &mut sk::Surface) {
        self.direct_context()
            .flush_and_submit_surface(surface, sk::gpu::SyncCpu::No);
    }
}
