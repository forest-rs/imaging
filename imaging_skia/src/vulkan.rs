// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

#![allow(unsafe_code, reason = "Vulkan interop requires raw handle bridging")]

use ash::vk::Handle as _;
use skia_safe as sk;

use crate::{Error, color_space_for_wgpu_texture_format, color_type_for_wgpu_texture_format};

#[derive(Debug)]
pub(crate) struct VulkanBackend {
    context: sk::gpu::DirectContext,
    queue_family_index: u32,
}

impl VulkanBackend {
    pub(crate) fn from_wgpu(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Self, Error> {
        let entry = unsafe {
            ash::Entry::load()
                .map_err(|_| Error::CreateGpuContext("unable to load Vulkan entry"))?
        };
        let device = unsafe {
            device
                .as_hal::<wgpu::hal::api::Vulkan>()
                .ok_or(Error::CreateGpuContext("missing Vulkan device"))?
        };
        let queue = unsafe {
            queue
                .as_hal::<wgpu::hal::api::Vulkan>()
                .ok_or(Error::CreateGpuContext("missing Vulkan queue"))?
        };

        let context = create_gr_context(
            &entry,
            device.shared_instance().raw_instance().handle(),
            device.raw_physical_device(),
            device.raw_device().handle(),
            queue.as_raw(),
            device.queue_family_index(),
            |gpo| unsafe {
                let get_device_proc_addr = device
                    .shared_instance()
                    .raw_instance()
                    .fp_v1_0()
                    .get_device_proc_addr;
                let proc = match gpo {
                    sk::gpu::vk::GetProcOf::Instance(instance, name) => {
                        let instance = ash::vk::Instance::from_raw(instance as _);
                        entry.get_instance_proc_addr(instance, name)
                    }
                    sk::gpu::vk::GetProcOf::Device(raw_device, name) => {
                        let raw_device = ash::vk::Device::from_raw(raw_device as _);
                        get_device_proc_addr(raw_device, name)
                    }
                };
                #[allow(
                    clippy::fn_to_numeric_cast_any,
                    reason = "Skia expects Vulkan proc addresses as opaque void pointers"
                )]
                let proc = proc.map(|f| f as _);
                proc.unwrap_or(core::ptr::null())
            },
        )?;

        Ok(Self {
            context,
            queue_family_index: device.queue_family_index(),
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
                .as_hal::<wgpu::hal::api::Vulkan>()
                .ok_or(Error::CreateGpuSurface)?
        };
        let image_info = unsafe {
            sk::gpu::vk::ImageInfo::new(
                hal_texture.raw_handle().as_raw() as _,
                sk::gpu::vk::Alloc::default(),
                sk::gpu::vk::ImageTiling::OPTIMAL,
                sk::gpu::vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                vk_format_for_wgpu_texture_format(format)?,
                1,
                self.queue_family_index,
                None::<sk::gpu::vk::YcbcrConversionInfo>,
                None::<sk::gpu::Protected>,
                None::<sk::gpu::vk::SharingMode>,
            )
        };
        let backend_texture = unsafe {
            sk::gpu::backend_textures::make_vk(
                (width, height),
                &image_info,
                "imaging_skia vulkan texture",
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
}

pub(crate) fn supports_texture_format(texture_format: wgpu::TextureFormat) -> Result<(), Error> {
    let _ = vk_format_for_wgpu_texture_format(texture_format)?;
    Ok(())
}

fn create_gr_context(
    _entry: &ash::Entry,
    instance: ash::vk::Instance,
    physical_device: ash::vk::PhysicalDevice,
    device: ash::vk::Device,
    queue: ash::vk::Queue,
    queue_family_index: u32,
    get_proc: impl Fn(sk::gpu::vk::GetProcOf) -> sk::gpu::vk::GetProcResult,
) -> Result<sk::gpu::DirectContext, Error> {
    let mut backend_context = unsafe {
        sk::gpu::vk::BackendContext::new(
            instance.as_raw() as _,
            physical_device.as_raw() as _,
            device.as_raw() as _,
            (queue.as_raw() as _, queue_family_index as usize),
            &get_proc,
        )
    };
    backend_context.set_max_api_version(sk::gpu::vk::Version::new(1, 1, 0));
    sk::gpu::direct_contexts::make_vulkan(&backend_context, &sk::gpu::ContextOptions::default())
        .ok_or(Error::CreateGpuContext(
            "unable to create Skia Vulkan context",
        ))
}

fn vk_format_for_wgpu_texture_format(
    texture_format: wgpu::TextureFormat,
) -> Result<sk::gpu::vk::Format, Error> {
    Ok(match texture_format {
        wgpu::TextureFormat::Rgba8Unorm => sk::gpu::vk::Format::R8G8B8A8_UNORM,
        wgpu::TextureFormat::Rgba8UnormSrgb => sk::gpu::vk::Format::R8G8B8A8_SRGB,
        wgpu::TextureFormat::Bgra8Unorm => sk::gpu::vk::Format::B8G8R8A8_UNORM,
        wgpu::TextureFormat::Bgra8UnormSrgb => sk::gpu::vk::Format::B8G8R8A8_SRGB,
        wgpu::TextureFormat::Rgb10a2Unorm => sk::gpu::vk::Format::A2B10G10R10_UNORM_PACK32,
        wgpu::TextureFormat::Rgba16Unorm => sk::gpu::vk::Format::R16G16B16A16_UNORM,
        wgpu::TextureFormat::Rgba16Float => sk::gpu::vk::Format::R16G16B16A16_SFLOAT,
        _ => return Err(Error::UnsupportedGpuTextureFormat),
    })
}
