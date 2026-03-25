// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// Portions of this file are derived from `anyrender_skia` in the AnyRender project:
// <https://github.com/dioxuslabs/anyrender>
// Original source: `crates/anyrender_skia/src/vulkan.rs`
// Adapted here for imaging's offscreen Ganesh renderer.

#![allow(unsafe_code, reason = "Vulkan and Skia FFI setup requires raw handles")]

use ash::{
    Device, Entry, Instance,
    vk::{
        API_VERSION_1_1, ApplicationInfo, DeviceCreateInfo, DeviceQueueCreateInfo, Handle,
        InstanceCreateInfo, PhysicalDevice, PhysicalDeviceFeatures, Queue, QueueFlags,
        make_api_version,
    },
};
use skia_safe as sk;
use std::ffi::CString;
use std::sync::Arc;

use crate::{
    Error, SkiaRenderer, color_space_for_wgpu_texture_format, color_type_for_wgpu_texture_format,
    ganesh::GaneshBackend,
};

#[derive(Debug)]
pub(crate) struct VulkanBackend {
    context: sk::gpu::DirectContext,
    handles: VulkanHandles,
}

#[derive(Debug)]
enum VulkanHandles {
    Owned {
        _entry: Entry,
        instance: Instance,
        _physical_device: PhysicalDevice,
        queue_family_index: u32,
        device: Arc<Device>,
        _queue: Queue,
    },
    Borrowed {
        _entry: Entry,
        instance: Instance,
        _physical_device: PhysicalDevice,
        queue_family_index: u32,
        device: Arc<Device>,
        _queue: Queue,
    },
}

impl VulkanBackend {
    /// Create the internal Vulkan backend used by the default Ganesh renderer path.
    ///
    /// This path is used when `imaging_skia` owns the Vulkan instance, device, queue, and Skia
    /// context outright.
    pub(crate) fn new() -> Result<Self, Error> {
        let entry = unsafe {
            Entry::load().map_err(|_| Error::CreateGpuContext("unable to load Vulkan entry"))?
        };
        let instance = create_instance(&entry)?;
        let (physical_device, queue_family_index) = pick_physical_device(&instance)?;
        let (device, queue) =
            create_logical_device(&instance, physical_device, queue_family_index)?;
        let device = Arc::new(device);
        let context = create_gr_context(
            &entry,
            &instance,
            physical_device,
            device.clone(),
            queue,
            queue_family_index,
        )?;

        Ok(Self {
            context,
            handles: VulkanHandles::Owned {
                _entry: entry,
                instance,
                _physical_device: physical_device,
                queue_family_index,
                device,
                _queue: queue,
            },
        })
    }

    /// Rebuild a Skia Vulkan backend from caller-owned raw Vulkan handles.
    ///
    /// This is the bridge used by explicit Vulkan interop paths where the embedding application
    /// already controls Vulkan lifetime and resource management.
    pub(crate) fn from_raw_handles(
        instance: ash::vk::Instance,
        physical_device: ash::vk::PhysicalDevice,
        device: ash::vk::Device,
        queue: ash::vk::Queue,
        queue_family_index: u32,
    ) -> Result<Self, Error> {
        let entry = unsafe {
            Entry::load().map_err(|_| Error::CreateGpuContext("unable to load Vulkan entry"))?
        };
        let instance = unsafe { Instance::load(entry.static_fn(), instance) };
        let device = Arc::new(unsafe { Device::load(instance.fp_v1_0(), device) });
        let context = create_gr_context(
            &entry,
            &instance,
            physical_device,
            device.clone(),
            queue,
            queue_family_index,
        )?;

        Ok(Self {
            context,
            handles: VulkanHandles::Borrowed {
                _entry: entry,
                instance,
                _physical_device: physical_device,
                queue_family_index,
                device,
                _queue: queue,
            },
        })
    }

    /// Borrow the underlying Skia direct context.
    pub(crate) fn direct_context(&mut self) -> &mut sk::gpu::DirectContext {
        &mut self.context
    }

    /// Return the queue family index associated with the wrapped graphics queue.
    pub(crate) fn queue_family_index(&self) -> u32 {
        match &self.handles {
            VulkanHandles::Owned {
                queue_family_index, ..
            }
            | VulkanHandles::Borrowed {
                queue_family_index, ..
            } => *queue_family_index,
        }
    }
}

impl Drop for VulkanBackend {
    fn drop(&mut self) {
        match &self.handles {
            VulkanHandles::Owned {
                instance, device, ..
            } => {
                let _ = unsafe { device.device_wait_idle() };
                unsafe {
                    device.destroy_device(None);
                    instance.destroy_instance(None);
                }
            }
            VulkanHandles::Borrowed { device, .. } => {
                let _ = unsafe { device.device_wait_idle() };
            }
        }
    }
}

#[cfg(feature = "wgpu")]
#[allow(
    clippy::too_many_arguments,
    reason = "Raw Vulkan interop needs the full image description."
)]
/// Wrap a caller-owned Vulkan image in a Skia surface for direct rendering.
fn create_wrapped_vulkan_surface(
    context: &mut sk::gpu::DirectContext,
    width: i32,
    height: i32,
    texture_format: wgpu::TextureFormat,
    image: ash::vk::Image,
    image_layout: ash::vk::ImageLayout,
    image_usage_flags: sk::gpu::vk::ImageUsageFlags,
    level_count: u32,
    queue_family_index: u32,
) -> Result<sk::Surface, Error> {
    use ash::vk::Handle as _;

    let vk_info = unsafe {
        sk::gpu::vk::ImageInfo::new(
            image.as_raw() as _,
            sk::gpu::vk::Alloc::default(),
            ash::vk::ImageTiling::OPTIMAL.as_raw() as _,
            image_layout.as_raw() as _,
            vk_format_for_wgpu_texture_format(texture_format)?,
            level_count,
            queue_family_index,
            None::<sk::gpu::vk::YcbcrConversionInfo>,
            None::<sk::gpu::Protected>,
            None::<sk::gpu::vk::SharingMode>,
        )
    };
    let mut backend_texture =
        unsafe { sk::gpu::backend_textures::make_vk((width, height), &vk_info, "") };
    sk::gpu::backend_textures::set_vk_image_layout(
        &mut backend_texture,
        image_layout.as_raw() as _,
    );
    let _ = image_usage_flags;
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

#[cfg(feature = "wgpu")]
/// Map supported wrapped `wgpu` formats to Vulkan image formats for Skia interop.
fn vk_format_for_wgpu_texture_format(
    texture_format: wgpu::TextureFormat,
) -> Result<sk::gpu::vk::Format, Error> {
    Ok(match texture_format {
        wgpu::TextureFormat::Rgba8Unorm => ash::vk::Format::R8G8B8A8_UNORM.as_raw() as _,
        wgpu::TextureFormat::Rgba8UnormSrgb => ash::vk::Format::R8G8B8A8_SRGB.as_raw() as _,
        wgpu::TextureFormat::Bgra8Unorm => ash::vk::Format::B8G8R8A8_UNORM.as_raw() as _,
        wgpu::TextureFormat::Bgra8UnormSrgb => ash::vk::Format::B8G8R8A8_SRGB.as_raw() as _,
        wgpu::TextureFormat::Rgb10a2Unorm => {
            ash::vk::Format::A2B10G10R10_UNORM_PACK32.as_raw() as _
        }
        wgpu::TextureFormat::Rgba16Unorm => ash::vk::Format::R16G16B16A16_UNORM.as_raw() as _,
        wgpu::TextureFormat::Rgba16Float => ash::vk::Format::R16G16B16A16_SFLOAT.as_raw() as _,
        _ => return Err(Error::Internal("unsupported Vulkan wgpu texture format")),
    })
}

#[cfg(feature = "wgpu")]
impl SkiaRenderer {
    /// Create an offscreen renderer backed by caller-owned Vulkan handles.
    ///
    /// Use this when `wgpu` or another embedding layer already owns the Vulkan instance, device,
    /// and graphics queue, but you want `imaging_skia` to allocate and own its render target.
    ///
    /// # Safety
    ///
    /// All Vulkan handles must be valid, belong together, and remain alive for the renderer
    /// lifetime.
    pub unsafe fn try_new_vulkan_from_raw_handles(
        width: u16,
        height: u16,
        instance: ash::vk::Instance,
        physical_device: ash::vk::PhysicalDevice,
        device: ash::vk::Device,
        queue: ash::vk::Queue,
        queue_family_index: u32,
    ) -> Result<Self, Error> {
        let width = i32::from(width);
        let height = i32::from(height);
        let mut backend = GaneshBackend::Vulkan(VulkanBackend::from_raw_handles(
            instance,
            physical_device,
            device,
            queue,
            queue_family_index,
        )?);
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

    /// Create a renderer that draws directly into a caller-owned Vulkan image.
    ///
    /// This is the explicit Vulkan interop entry point for applications that already manage their
    /// own Vulkan instance, device, queue, and image lifecycle and need Skia to render into one of
    /// those images without introducing another graphics stack.
    ///
    /// # Safety
    ///
    /// All Vulkan handles must be valid, belong together, remain alive for the renderer lifetime,
    /// and refer to a renderable 2D image compatible with the supplied layout and usage flags.
    #[allow(
        clippy::too_many_arguments,
        reason = "Raw Vulkan interop needs the full handle set."
    )]
    pub unsafe fn try_new_vulkan_from_raw_handles_and_texture(
        width: u16,
        height: u16,
        texture_format: wgpu::TextureFormat,
        instance: ash::vk::Instance,
        physical_device: ash::vk::PhysicalDevice,
        device: ash::vk::Device,
        queue: ash::vk::Queue,
        queue_family_index: u32,
        image: ash::vk::Image,
        image_layout: ash::vk::ImageLayout,
        image_usage_flags: ash::vk::ImageUsageFlags,
        level_count: u32,
    ) -> Result<Self, Error> {
        use ash::vk::Handle as _;

        let width = i32::from(width);
        let height = i32::from(height);
        let mut backend = GaneshBackend::Vulkan(VulkanBackend::from_raw_handles(
            instance,
            physical_device,
            device,
            queue,
            queue_family_index,
        )?);
        let surface = create_wrapped_vulkan_surface(
            backend.direct_context(),
            width,
            height,
            texture_format,
            image,
            image_layout,
            image_usage_flags.as_raw() as _,
            level_count,
            queue_family_index,
        )?;
        Ok(Self::from_backend_surface(backend, surface))
    }

    /// Retarget the renderer to a different caller-owned Vulkan image.
    ///
    /// This keeps the existing Skia/Vulkan context alive while swapping the image that subsequent
    /// draws render into.
    ///
    /// # Safety
    ///
    /// The supplied image metadata must remain valid and match the provided image.
    pub unsafe fn replace_vulkan_texture(
        &mut self,
        width: u16,
        height: u16,
        texture_format: wgpu::TextureFormat,
        image: ash::vk::Image,
        image_layout: ash::vk::ImageLayout,
        image_usage_flags: ash::vk::ImageUsageFlags,
        level_count: u32,
        queue_family_index: u32,
    ) -> Result<(), Error> {
        let width = i32::from(width);
        let height = i32::from(height);
        self.backend.ensure_current()?;
        self.backend.flush_surface(&mut self.surface);
        let surface = create_wrapped_vulkan_surface(
            self.backend.direct_context(),
            width,
            height,
            texture_format,
            image,
            image_layout,
            image_usage_flags.as_raw() as _,
            level_count,
            queue_family_index,
        )?;
        self.surface = surface;
        Ok(())
    }
}

/// Create a minimal Vulkan instance for the internal headless renderer path.
fn create_instance(entry: &Entry) -> Result<Instance, Error> {
    let app_name = CString::new("imaging_skia").expect("static string");
    let engine_name = CString::new("imaging").expect("static string");
    let app_info = ApplicationInfo::default()
        .application_name(&app_name)
        .application_version(make_api_version(0, 1, 0, 0))
        .engine_name(&engine_name)
        .engine_version(make_api_version(0, 1, 0, 0))
        .api_version(API_VERSION_1_1);
    let create_info = InstanceCreateInfo::default().application_info(&app_info);
    unsafe {
        entry
            .create_instance(&create_info, None)
            .map_err(|_| Error::CreateGpuContext("unable to create Vulkan instance"))
    }
}

/// Choose a graphics-capable physical device and queue family for headless rendering.
fn pick_physical_device(instance: &Instance) -> Result<(PhysicalDevice, u32), Error> {
    let devices = unsafe {
        instance
            .enumerate_physical_devices()
            .map_err(|_| Error::CreateGpuContext("unable to enumerate Vulkan devices"))?
    };
    devices
        .into_iter()
        .find_map(|physical_device| {
            let queue_family_index = unsafe {
                instance
                    .get_physical_device_queue_family_properties(physical_device)
                    .iter()
                    .enumerate()
                    .find_map(|(index, props)| {
                        props
                            .queue_flags
                            .contains(QueueFlags::GRAPHICS)
                            .then_some(index as u32)
                    })
            }?;
            Some((physical_device, queue_family_index))
        })
        .ok_or(Error::CreateGpuContext(
            "no suitable Vulkan graphics queue found",
        ))
}

/// Create a logical device and graphics queue for the selected physical device.
fn create_logical_device(
    instance: &Instance,
    physical_device: PhysicalDevice,
    queue_family_index: u32,
) -> Result<(Device, Queue), Error> {
    let queue_priorities = [1.0f32];
    let queue_create_info = DeviceQueueCreateInfo::default()
        .queue_family_index(queue_family_index)
        .queue_priorities(&queue_priorities);
    let features = PhysicalDeviceFeatures::default();
    let create_info = DeviceCreateInfo::default()
        .queue_create_infos(std::slice::from_ref(&queue_create_info))
        .enabled_features(&features);

    let device = unsafe {
        instance
            .create_device(physical_device, &create_info, None)
            .map_err(|_| Error::CreateGpuContext("unable to create Vulkan device"))?
    };
    let queue = unsafe { device.get_device_queue(queue_family_index, 0) };
    Ok((device, queue))
}

/// Create the Skia Ganesh Vulkan context that wraps the selected Vulkan queue and device.
fn create_gr_context(
    entry: &Entry,
    instance: &Instance,
    physical_device: PhysicalDevice,
    device: Arc<Device>,
    queue: Queue,
    queue_family_index: u32,
) -> Result<sk::gpu::DirectContext, Error> {
    let get_proc = unsafe {
        |gpo: sk::gpu::vk::GetProcOf| {
            let get_device_proc_addr = instance.fp_v1_0().get_device_proc_addr;
            match gpo {
                sk::gpu::vk::GetProcOf::Instance(instance, name) => {
                    let vk_instance = ash::vk::Instance::from_raw(instance as _);
                    entry.get_instance_proc_addr(vk_instance, name)
                }
                sk::gpu::vk::GetProcOf::Device(device, name) => {
                    let vk_device = ash::vk::Device::from_raw(device as _);
                    get_device_proc_addr(vk_device, name)
                }
            }
            .map(|f| f as _)
            .unwrap_or(std::ptr::null())
        }
    };

    let mut backend_context = unsafe {
        sk::gpu::vk::BackendContext::new(
            instance.handle().as_raw() as _,
            physical_device.as_raw() as _,
            device.handle().as_raw() as _,
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
