// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Skia backend for `imaging`.
//!
//! This crate connects the semantic `imaging` command stream to Skia.
//!
//! At a high level, there are three ways to use it:
//!
//! - [`SkiaRenderer`] renders through Skia Ganesh into a GPU-backed surface.
//! - [`SkiaCpuRenderer`] renders through Skia's raster backend into CPU memory.
//! - [`SkCanvasSink`] and [`SkPictureRecorderSink`] let you stream `imaging` commands directly into
//!   native Skia targets instead of going through the owned renderers.
//!
//! # What The Crate Supports
//!
//! `imaging_skia` is useful when you want one of the following:
//!
//! - replay an [`imaging::record::Scene`] through Skia and read the result back as RGBA8 pixels
//! - render `imaging` scenes with Skia's GPU path when a Ganesh backend is available
//! - keep rendering CPU-only while still using Skia's paint, path, gradient, text, and filter
//!   implementations
//! - stream `imaging` commands directly into an existing `skia_safe::Canvas`
//! - record `imaging` commands into a native `skia_safe::Picture`
//! - attach Skia rendering to an existing backend-owned texture or image via Metal, Vulkan, or GL
//!   interop methods
//!
//! # CPU vs GPU
//!
//! [`SkiaCpuRenderer`] is the simpler choice when you just need pixels and do not need graphics API
//! interop. It allocates a raster surface internally and returns RGBA8 output after replay.
//!
//! [`SkiaRenderer`] is the GPU path. It owns a Ganesh context and an offscreen GPU render surface
//! by default, but it can also be pointed at caller-owned backend textures through the backend-
//! specific constructors. Use it when you want GPU rendering, Skia backend texture access, or
//! integration with an existing graphics stack.
//!
//! # Backend Layout
//!
//! The GPU renderer chooses or exposes different backends depending on platform and enabled
//! features:
//!
//! - Apple platforms use Metal for the default GPU path.
//! - Non-Apple platforms use the internal OpenGL backend unless the `vulkan` feature is enabled,
//!   in which case the default GPU path is Vulkan.
//! - With the `wgpu` feature enabled, the renderer can also attach to existing backend resources:
//!   Metal on Apple, Vulkan on non-Apple builds with `vulkan`, and explicit GL/GLES textures via
//!   raw-context interop methods.
//!
//! # Which API To Reach For
//!
//! Use [`SkiaCpuRenderer`] when:
//!
//! - you want the lowest-friction offscreen renderer
//! - you do not need GPU interop
//! - deterministic CPU-side rendering is more important than backend integration
//!
//! Use [`SkiaRenderer::new`] or [`SkiaRenderer::try_new`] when:
//!
//! - you want an owned offscreen GPU renderer
//! - `imaging_skia` should choose and own the underlying Ganesh backend
//! - you want to render scenes or pictures and optionally inspect the GPU surface afterward
//!
//! Use the backend-specific `SkiaRenderer` constructors when:
//!
//! - your application already owns the render target
//! - Skia must draw into an existing Metal texture, Vulkan image, or GL texture
//! - you need the renderer to follow an embedding application's backend/resource lifetime
//!
//! Use [`SkCanvasSink`] when:
//!
//! - you already have a Skia canvas
//! - you want to stream `imaging` commands directly into it
//! - you do not need `imaging_skia` to own any renderer state
//!
//! Use [`SkPictureRecorderSink`] when:
//!
//! - you want a retained native Skia recording
//! - downstream code already consumes `skia_safe::Picture`
//! - you want to separate authoring from later rendering
//!
//! # Render A Recorded Scene
//!
//! Record commands into [`imaging::record::Scene`], then hand the scene to [`SkiaRenderer`].
//!
//! ```no_run
//! use imaging::{Painter, record};
//! use imaging_skia::SkiaRenderer;
//! use kurbo::Rect;
//! use peniko::{Brush, Color};
//!
//! fn main() -> Result<(), imaging_skia::Error> {
//!     let paint = Brush::Solid(Color::from_rgb8(0x2a, 0x6f, 0xdb));
//!     let mut scene = record::Scene::new();
//!
//!     {
//!         let mut painter = Painter::new(&mut scene);
//!         painter.fill_rect(Rect::new(0.0, 0.0, 128.0, 128.0), &paint);
//!     }
//!
//!     let mut renderer = SkiaRenderer::new(128, 128);
//!     let rgba = renderer.render_scene_rgba8(&scene)?;
//!     assert_eq!(rgba.len(), 128 * 128 * 4);
//!     Ok(())
//! }
//! ```
//!
//! # Draw Into An Existing `Canvas`
//!
//! If you already have a Skia canvas, wrap it with [`SkCanvasSink`] and stream commands directly.
//!
//! ```no_run
//! use imaging::Painter;
//! use imaging_skia::SkCanvasSink;
//! use kurbo::Rect;
//! use peniko::{Brush, Color};
//! use skia_safe::surfaces;
//!
//! fn main() -> Result<(), imaging_skia::Error> {
//!     let paint = Brush::Solid(Color::from_rgb8(0x1d, 0x4e, 0x89));
//!     let mut surface = surfaces::raster_n32_premul((128, 128)).unwrap();
//!
//!     {
//!         let mut sink = SkCanvasSink::new(surface.canvas());
//!         let mut painter = Painter::new(&mut sink);
//!         painter.fill_rect(Rect::new(0.0, 0.0, 128.0, 128.0), &paint);
//!         sink.finish()?;
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! # Record A `SkPicture`
//!
//! Use [`SkPictureRecorderSink`] when you want Skia's native retained recording format.
//!
//! ```no_run
//! use imaging::Painter;
//! use imaging_skia::SkPictureRecorderSink;
//! use kurbo::Rect;
//! use peniko::{Brush, Color};
//!
//! fn main() -> Result<(), imaging_skia::Error> {
//!     let paint = Brush::Solid(Color::from_rgb8(0x7c, 0x3a, 0xed));
//!     let mut sink = SkPictureRecorderSink::new(Rect::new(0.0, 0.0, 128.0, 128.0));
//!
//!     {
//!         let mut painter = Painter::new(&mut sink);
//!         painter.fill_rect(Rect::new(16.0, 16.0, 112.0, 112.0), &paint);
//!     }
//!
//!     let picture = sink.finish_picture()?;
//!     assert_eq!(picture.cull_rect().right, 128.0);
//!     Ok(())
//! }
//! ```
//!
//! # Render A Native `SkPicture`
//!
//! If you already have a recorded picture, hand it directly to [`SkiaRenderer`].
//!
//! ```no_run
//! use imaging::Painter;
//! use imaging_skia::{SkPictureRecorderSink, SkiaRenderer};
//! use kurbo::Rect;
//! use peniko::{Brush, Color};
//!
//! fn main() -> Result<(), imaging_skia::Error> {
//!     let paint = Brush::Solid(Color::from_rgb8(0xd9, 0x77, 0x06));
//!     let mut sink = SkPictureRecorderSink::new(Rect::new(0.0, 0.0, 128.0, 128.0));
//!
//!     {
//!         let mut painter = Painter::new(&mut sink);
//!         painter.fill_rect(Rect::new(16.0, 16.0, 112.0, 112.0), &paint);
//!     }
//!
//!     let picture = sink.finish_picture()?;
//!     let mut renderer = SkiaRenderer::new(128, 128);
//!     let rgba = renderer.render_picture_rgba8(&picture)?;
//!     assert_eq!(rgba.len(), 128 * 128 * 4);
//!     Ok(())
//! }
//! ```

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

#[cfg(all(feature = "wgpu", any(target_os = "macos", target_os = "ios")))]
use core::ffi::c_void;
#[cfg(all(feature = "wgpu", any(target_os = "macos", target_os = "ios")))]
use foreign_types_shared::ForeignType;

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod metal;
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
mod opengl;
mod sinks;
#[cfg(all(feature = "vulkan", not(any(target_os = "macos", target_os = "ios"))))]
mod vulkan;

mod font_cache;
mod ganesh;
use imaging::{
    Filter, GeometryRef,
    record::{Scene, ValidateError, replay},
};
use kurbo::{Affine, Shape as _};
use peniko::color::{ColorSpaceTag, HueDirection, Srgb};
use peniko::{BrushRef, ImageAlphaType, ImageFormat, ImageQuality, InterpolationAlphaSpace};
use skia_safe as sk;

use crate::font_cache::skia_font_from_glyph_run;
use crate::ganesh::{GaneshBackend, create_surface as create_ganesh_surface};
use std::{cell::RefCell, rc::Rc};

use sinks::MaskCache;
pub use sinks::{SkCanvasSink, SkPictureRecorderSink};

/// Errors that can occur when rendering via Skia.
#[derive(Debug)]
pub enum Error {
    /// The scene is invalid (unbalanced stacks).
    InvalidScene(ValidateError),
    /// No supported Ganesh backend was available on this platform.
    UnsupportedGpuBackend,
    /// A Ganesh backend could not be initialized.
    CreateGpuContext(&'static str),
    /// A Ganesh render target surface could not be created.
    CreateGpuSurface,
    /// An image brush was encountered; this backend does not support it.
    UnsupportedImageBrush,
    /// A filter configuration could not be translated.
    UnsupportedFilter,
    /// A glyph run used variable-font coordinates unsupported by this backend.
    UnsupportedGlyphVariations,
    /// A glyph run used a per-glyph transform unsupported by this backend.
    UnsupportedGlyphTransform,
    /// Font bytes could not be loaded by Skia.
    InvalidFontData,
    /// A glyph identifier could not be represented by Skia's glyph type.
    InvalidGlyphId,
    /// An internal invariant was violated.
    Internal(&'static str),
}

/// Ganesh renderer that executes `imaging` commands into a GPU-backed Skia surface.
#[derive(Debug)]
pub struct SkiaRenderer {
    backend: GaneshBackend,
    surface: sk::Surface,
    tolerance: f64,
}

impl SkiaRenderer {
    /// Create an offscreen GPU renderer for a fixed output size.
    ///
    /// This is the convenience entry point for the common case where `imaging_skia` owns the GPU
    /// backend and temporary render target for you.
    pub fn new(width: u16, height: u16) -> Self {
        Self::try_new(width, height).expect("create imaging_skia renderer")
    }

    /// Create an offscreen GPU renderer for a fixed output size.
    ///
    /// Use this when renderer construction may legitimately fail at runtime, such as when no
    /// compatible Ganesh backend can be created on the current machine.
    pub fn try_new(width: u16, height: u16) -> Result<Self, Error> {
        let width = i32::from(width);
        let height = i32::from(height);
        let mut backend = GaneshBackend::new()?;
        let surface = create_ganesh_surface(backend.direct_context(), width, height)?;
        Ok(Self::from_backend_surface(backend, surface))
    }

    /// Build a renderer from an already-initialized Ganesh backend and wrapped target surface.
    ///
    /// Backend modules use this to share the same renderer initialization path without reaching
    /// into `SkiaRenderer`'s private fields directly.
    pub(crate) fn from_backend_surface(backend: GaneshBackend, surface: sk::Surface) -> Self {
        Self {
            backend,
            surface,
            tolerance: 0.1,
        }
    }

    /// Set the geometric flattening tolerance used for path conversion.
    ///
    /// Lower values preserve curve fidelity more aggressively; higher values can reduce path
    /// complexity when rendering highly curved geometry.
    pub fn set_tolerance(&mut self, tolerance: f64) {
        self.tolerance = tolerance;
    }
}

#[cfg(feature = "wgpu")]
impl SkiaRenderer {
    /// Create an offscreen renderer that shares the caller's `wgpu` device and queue.
    ///
    /// Unlike [`Self::try_new_from_wgpu_texture`], this path does not wrap a caller-owned texture.
    /// It rebuilds the platform backend from `wgpu` and then allocates an internal Skia render
    /// target, so it is the right choice when you care about backend sharing but not about drawing
    /// into a specific external texture.
    pub fn try_new_from_wgpu_device(
        width: u16,
        height: u16,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<Self, Error> {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        {
            let device = unsafe {
                device
                    .as_hal::<wgpu::hal::api::Metal>()
                    .ok_or(Error::CreateGpuContext("missing Metal device"))?
            };
            let queue = unsafe {
                queue
                    .as_hal::<wgpu::hal::api::Metal>()
                    .ok_or(Error::CreateGpuContext("missing Metal queue"))?
            };
            let device = device.raw_device().as_ptr() as *mut c_void;
            let command_queue = queue.as_raw().lock().as_ptr() as *mut c_void;
            return unsafe {
                Self::try_new_metal_from_raw_pointers_without_texture(
                    width,
                    height,
                    device,
                    command_queue,
                )
            };
        }

        #[cfg(all(feature = "vulkan", not(any(target_os = "macos", target_os = "ios"))))]
        {
            use ash::vk::Handle as _;

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
            let instance = device.shared_instance().raw_instance().handle();
            let physical_device = device.raw_physical_device();
            let raw_device = device.raw_device().handle();
            let raw_queue = queue.as_raw();
            let queue_family_index = device.queue_family_index();
            return unsafe {
                Self::try_new_vulkan_from_raw_handles(
                    width,
                    height,
                    instance,
                    physical_device,
                    raw_device,
                    raw_queue,
                    queue_family_index,
                )
            };
        }

        #[allow(
            unreachable_code,
            unused_variables,
            reason = "Platform and feature cfgs intentionally leave unsupported backend paths empty."
        )]
        Err(Error::UnsupportedGpuBackend)
    }

    /// Create a renderer that targets an owned `wgpu` texture.
    ///
    /// This is the high-level interop entry point when the caller already works in `wgpu` terms and
    /// wants `imaging_skia` to attach to that device/queue/texture tuple. Use
    /// [`Self::try_new_from_wgpu_device`] instead when you only want to share the backend and let
    /// `imaging_skia` allocate its own offscreen target surface.
    pub fn try_new_from_wgpu_texture(
        texture_format: wgpu::TextureFormat,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: wgpu::Texture,
    ) -> Result<Self, Error> {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        {
            let hal_device = unsafe {
                device
                    .as_hal::<wgpu::hal::api::Metal>()
                    .ok_or(Error::CreateGpuContext("missing Metal device"))?
            };
            let hal_queue = unsafe {
                queue
                    .as_hal::<wgpu::hal::api::Metal>()
                    .ok_or(Error::CreateGpuContext("missing Metal queue"))?
            };
            let texture = unsafe {
                texture_keepalive
                    .texture
                    .as_hal::<wgpu::hal::api::Metal>()
                    .ok_or(Error::CreateGpuSurface)?
            };
            let raw_device = hal_device.raw_device().as_ptr() as *mut c_void;
            let raw_command_queue = hal_queue.as_raw().lock().as_ptr() as *mut c_void;
            let texture = unsafe { texture.raw_handle() }.as_ptr() as *mut c_void;
            return unsafe {
                Self::try_new_metal_from_raw_pointers(
                    width,
                    height,
                    texture_format,
                    raw_device,
                    raw_command_queue,
                    texture,
                )
            };
        }

        #[cfg(all(feature = "vulkan", not(any(target_os = "macos", target_os = "ios"))))]
        {
            use ash::vk::Handle as _;

            let hal_device = unsafe {
                device
                    .as_hal::<wgpu::hal::api::Vulkan>()
                    .ok_or(Error::CreateGpuContext("missing Vulkan device"))?
            };
            let hal_queue = unsafe {
                queue
                    .as_hal::<wgpu::hal::api::Vulkan>()
                    .ok_or(Error::CreateGpuContext("missing Vulkan queue"))?
            };
            let texture = unsafe {
                texture_keepalive
                    .texture
                    .as_hal::<wgpu::hal::api::Vulkan>()
                    .ok_or(Error::CreateGpuSurface)?
            };
            let instance = hal_device.shared_instance().raw_instance().handle();
            let physical_device = hal_device.raw_physical_device();
            let raw_device = hal_device.raw_device().handle();
            let raw_queue = hal_queue.as_raw();
            let queue_family_index = hal_device.queue_family_index();
            let raw_image = unsafe { texture.raw_handle() };
            let image_layout = ash::vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL;
            let image_usage_flags = ash::vk::ImageUsageFlags::COLOR_ATTACHMENT;
            return unsafe {
                Self::try_new_vulkan_from_raw_handles_and_texture(
                    width,
                    height,
                    texture_format,
                    instance,
                    physical_device,
                    raw_device,
                    raw_queue,
                    queue_family_index,
                    raw_image,
                    image_layout,
                    image_usage_flags,
                    1,
                )
            };
        }

        #[allow(
            unreachable_code,
            unused_variables,
            reason = "Platform and feature cfgs intentionally leave unsupported backend paths empty."
        )]
        Err(Error::UnsupportedGpuBackend)
    }
}

#[cfg(feature = "wgpu")]
impl SkiaRenderer {
    /// Retarget the renderer to a different owned `wgpu` texture on the same backend bridge.
    ///
    /// This is the `wgpu`-level companion to the explicit Metal and Vulkan replacement methods and
    /// is intended for integrations that manage resize or swapchain churn entirely through `wgpu`.
    pub fn replace_wgpu_texture(
        &mut self,
        texture_format: wgpu::TextureFormat,
        texture: wgpu::Texture,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<(), Error> {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        {
            let texture = unsafe {
                texture_keepalive
                    .texture
                    .as_hal::<wgpu::hal::api::Metal>()
                    .ok_or(Error::CreateGpuSurface)?
            };
            let texture = unsafe { texture.raw_handle() }.as_ptr() as *mut c_void;
            return unsafe {
                self.replace_metal_texture_raw(width, height, texture_format, texture)
            };
        }

        #[cfg(all(feature = "vulkan", not(any(target_os = "macos", target_os = "ios"))))]
        {
            let texture = unsafe {
                texture_keepalive
                    .texture
                    .as_hal::<wgpu::hal::api::Vulkan>()
                    .ok_or(Error::CreateGpuSurface)?
            };
            let raw_image = unsafe { texture.raw_handle() };
            let queue_family_index = match &self.backend {
                GaneshBackend::Vulkan(backend) => backend.queue_family_index(),
                _ => sk::gpu::vk::QUEUE_FAMILY_IGNORED,
            };
            return unsafe {
                self.replace_vulkan_texture(
                    width,
                    height,
                    texture_format,
                    raw_image,
                    ash::vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                    ash::vk::ImageUsageFlags::COLOR_ATTACHMENT,
                    1,
                    queue_family_index,
                )
            };
        }

        #[allow(
            unreachable_code,
            unused_variables,
            reason = "Platform and feature cfgs intentionally leave unsupported backend paths empty."
        )]
        Err(Error::UnsupportedGpuBackend)
    }
}

impl SkiaRenderer {
    /// Reset canvas state before starting a new frame on the wrapped GPU surface.
    ///
    /// Rendering methods call this internally so each frame starts from a known transform, clip,
    /// and clear state regardless of what the previous frame left behind.
    fn reset(&mut self) {
        let canvas = self.surface.canvas();
        canvas.restore_to_count(1);
        canvas.reset_matrix();
        canvas.clear(sk::Color::TRANSPARENT);
    }

    /// Replay an `imaging` scene into the current GPU surface.
    ///
    /// This is the main path for rendering the semantic `imaging::record::Scene` representation
    /// through Skia/Ganesh.
    pub fn render_scene(&mut self, scene: &Scene) -> Result<(), Error> {
        scene.validate().map_err(Error::InvalidScene)?;
        self.backend.ensure_current()?;
        self.reset();
        let mut sink = SkCanvasSink::new(self.surface.canvas());
        sink.set_tolerance(self.tolerance);
        replay(scene, &mut sink);
        sink.finish()?;
        self.backend.flush_surface(&mut self.surface);
        Ok(())
    }

    /// Draw an existing native Skia picture into the current GPU surface.
    ///
    /// This is useful when higher layers already hold a recorded `SkPicture` and want to reuse the
    /// same renderer and readback path as scene-based rendering.
    pub fn render_picture(&mut self, picture: &sk::Picture) -> Result<(), Error> {
        self.backend.ensure_current()?;
        self.reset();
        self.surface.canvas().draw_picture(picture, None, None);
        self.backend.flush_surface(&mut self.surface);
        Ok(())
    }

    /// Borrow the live GPU-backed `skia_safe::Surface`.
    ///
    /// Use this when you need direct Skia drawing access instead of going through scene replay.
    pub fn surface(&mut self) -> &mut sk::Surface {
        &mut self.surface
    }

    /// Snapshot the current GPU surface as a Skia image.
    ///
    /// This flushes pending work first so the returned image reflects the renderer's latest output.
    pub fn image_snapshot(&mut self) -> sk::Image {
        let _ = self.backend.ensure_current();
        self.backend.flush_surface(&mut self.surface);
        self.surface.image_snapshot()
    }

    /// Expose Skia's backend texture for the current surface when the backend supports it.
    ///
    /// This is primarily for advanced interop or inspection code that needs the underlying Ganesh
    /// texture handle after rendering.
    pub fn backend_texture(&mut self) -> Option<sk::gpu::BackendTexture> {
        let _ = self.backend.ensure_current();
        self.backend.flush_surface(&mut self.surface);
        sk::gpu::surfaces::get_backend_texture(
            &mut self.surface,
            sk::surface::BackendHandleAccess::FlushRead,
        )
    }

    /// Render a scene and immediately read it back as unpremultiplied RGBA8 bytes.
    ///
    /// This is the simplest end-to-end API when the caller only needs pixel output.
    pub fn render_scene_rgba8(&mut self, scene: &Scene) -> Result<Vec<u8>, Error> {
        self.render_scene(scene)?;
        self.read_rgba8()
    }

    /// Render a native Skia picture and immediately read it back as RGBA8 bytes.
    pub fn render_picture_rgba8(&mut self, picture: &sk::Picture) -> Result<Vec<u8>, Error> {
        self.render_picture(picture)?;
        self.read_rgba8()
    }

    /// Read back the current GPU surface into an unpremultiplied RGBA8 buffer.
    ///
    /// Rendering methods funnel through this helper after flushing work to the active backend.
    fn read_rgba8(&mut self) -> Result<Vec<u8>, Error> {
        self.backend.ensure_current()?;
        self.backend.flush_surface(&mut self.surface);
        let info = sk::ImageInfo::new(
            (self.surface.width(), self.surface.height()),
            sk::ColorType::RGBA8888,
            sk::AlphaType::Unpremul,
            None,
        );
        let mut bytes =
            vec![0_u8; (self.surface.width() as usize) * (self.surface.height() as usize) * 4];
        let ok = self.surface.read_pixels(
            &info,
            bytes.as_mut_slice(),
            (4 * self.surface.width()) as usize,
            (0, 0),
        );
        if !ok {
            return Err(Error::Internal("read_pixels failed"));
        }
        Ok(bytes)
    }
}

/// CPU raster renderer that executes `imaging` commands using a Skia raster surface.
#[derive(Debug)]
pub struct SkiaCpuRenderer {
    surface: sk::Surface,
    width: i32,
    height: i32,
    tolerance: f64,
    mask_cache: Rc<RefCell<MaskCache>>,
}

impl SkiaCpuRenderer {
    /// Create a CPU raster renderer for a fixed output size.
    ///
    /// This is the fallback path when callers want Skia rendering without any GPU backend or
    /// external graphics interop.
    pub fn new(width: u16, height: u16) -> Self {
        let width = i32::from(width);
        let height = i32::from(height);
        // Use an explicit RGBA8888 premultiplied raster surface. Many blend modes are defined in
        // premultiplied space, and it also matches Skia's typical raster backend behavior.
        //
        // Note: we still export unpremultiplied RGBA8 from `read_rgba8()`.
        let info = sk::ImageInfo::new(
            (width, height),
            sk::ColorType::RGBA8888,
            sk::AlphaType::Premul,
            None,
        );
        let surface = sk::surfaces::raster(&info, None, None)
            .expect("create skia raster RGBA8888/premul surface");
        Self {
            surface,
            width,
            height,
            tolerance: 0.1,
            mask_cache: Rc::new(RefCell::new(MaskCache::default())),
        }
    }

    /// Set the geometric flattening tolerance used for path conversion.
    pub fn set_tolerance(&mut self, tolerance: f64) {
        if self.tolerance != tolerance {
            self.mask_cache.borrow_mut().clear();
        }
        self.tolerance = tolerance;
    }

    /// Drop any realized mask artifacts cached by the renderer.
    ///
    /// The cache is renderer-scoped so unchanged masked subscenes can be reused across renders.
    /// Call this if you need to release memory aggressively or after changing assumptions that
    /// affect mask realization outside the recorded scene itself.
    pub fn clear_cached_masks(&mut self) {
        self.mask_cache.borrow_mut().clear();
    }


    /// Reset canvas state before rendering a new frame into the raster surface.
    ///
    /// Rendering methods call this internally so each frame starts from a known transform, clip,
    /// and clear state.
    fn reset(&mut self) {
        let canvas = self.surface.canvas();
        canvas.restore_to_count(1);
        canvas.reset_matrix();
        canvas.clear(sk::Color::TRANSPARENT);
    }

    /// Replay an `imaging` scene through the raster backend and read back RGBA8 bytes.
    pub fn render_scene_rgba8(&mut self, scene: &Scene) -> Result<Vec<u8>, Error> {
        scene.validate().map_err(Error::InvalidScene)?;
        self.reset();
        let mut sink =
            SkCanvasSink::new_with_mask_cache(self.surface.canvas(), Rc::clone(&self.mask_cache));
        sink.set_tolerance(self.tolerance);
        replay(scene, &mut sink);
        sink.finish()?;
        self.read_rgba8()
    }

    /// Draw a native Skia picture through the raster backend and read back RGBA8 bytes.
    pub fn render_picture_rgba8(&mut self, picture: &sk::Picture) -> Result<Vec<u8>, Error> {
        self.reset();
        self.surface.canvas().draw_picture(picture, None, None);
        self.read_rgba8()
    }

    /// Read back the current raster surface into an unpremultiplied RGBA8 buffer.
    ///
    /// This is the raster counterpart to the GPU renderer's readback helper.
    fn read_rgba8(&mut self) -> Result<Vec<u8>, Error> {
        let image = self.surface.image_snapshot();
        let info = sk::ImageInfo::new(
            (self.width, self.height),
            sk::ColorType::RGBA8888,
            sk::AlphaType::Unpremul,
            None,
        );
        let mut bytes = vec![0_u8; (self.width as usize) * (self.height as usize) * 4];
        let ok = image.read_pixels(
            &info,
            bytes.as_mut_slice(),
            (4 * self.width) as usize,
            (0, 0),
            sk::image::CachingHint::Disallow,
        );
        if !ok {
            return Err(Error::Internal("read_pixels failed"));
        }
        Ok(bytes)
    }
}

#[cfg(feature = "wgpu")]
/// Map supported `wgpu` texture formats to the Skia color types used for wrapped surfaces.
pub(crate) fn color_type_for_wgpu_texture_format(
    texture_format: wgpu::TextureFormat,
) -> Result<sk::ColorType, Error> {
    match texture_format {
        wgpu::TextureFormat::Rgba8Unorm => Ok(sk::ColorType::RGBA8888),
        wgpu::TextureFormat::Rgba8UnormSrgb => Ok(sk::ColorType::SRGBA8888),
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => {
            Ok(sk::ColorType::BGRA8888)
        }
        wgpu::TextureFormat::Rgb10a2Unorm => Ok(sk::ColorType::RGBA1010102),
        wgpu::TextureFormat::Rgba16Unorm => Ok(sk::ColorType::R16G16B16A16UNorm),
        wgpu::TextureFormat::Rgba16Float => Ok(sk::ColorType::RGBAF16),
        _ => Err(Error::Internal("unsupported wgpu texture format")),
    }
}

#[cfg(feature = "wgpu")]
/// Attach an explicit Skia color space when the wrapped `wgpu` texture is sRGB encoded.
pub(crate) fn color_space_for_wgpu_texture_format(
    texture_format: wgpu::TextureFormat,
) -> Option<sk::ColorSpace> {
    match texture_format {
        wgpu::TextureFormat::Rgba8UnormSrgb | wgpu::TextureFormat::Bgra8UnormSrgb => {
            Some(sk::ColorSpace::new_srgb())
        }
        _ => None,
    }
}

/// Initializes wgpu's internal resource tracking state for a texture before
/// handing its raw Metal handle to Skia.
///
/// On Metal, wgpu lazily initializes textures the first time they are used
/// through wgpu itself. If Skia renders into the raw `MTLTexture` before wgpu
/// has touched it, wgpu will later insert its own clear pass to "initialize"
/// the texture — overwriting whatever Skia drew and producing a black frame.
///
/// By submitting a render pass that clears to transparent here, we force wgpu
/// to mark the texture as initialized before Skia takes ownership of it, so
/// wgpu's deferred clear never fires. The clear value doesn't matter since
/// Skia will overwrite the entire texture, but transparent is the least
/// surprising default if anything goes wrong.
#[cfg(feature = "wgpu")]
fn initialize_texture_for_wgpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) {
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("texture_init"),
    });
    let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("texture_init"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
            depth_slice: None,
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    drop(_pass);
    queue.submit([encoder.finish()]);
}

/// Narrow an `f64` value to `f32` for Skia APIs that operate in single precision.
#[allow(
    clippy::cast_possible_truncation,
    reason = "Skia APIs consume f32; truncation from f64 geometry is acceptable"
)]
fn f64_to_f32(v: f64) -> f32 {
    v as f32
}

/// Convert a `kurbo` affine transform into Skia's row-major matrix representation.
fn affine_to_matrix(xf: Affine) -> sk::Matrix {
    let a = xf.as_coeffs();
    sk::Matrix::new_all(
        f64_to_f32(a[0]),
        f64_to_f32(a[2]),
        f64_to_f32(a[4]),
        f64_to_f32(a[1]),
        f64_to_f32(a[3]),
        f64_to_f32(a[5]),
        0.0,
        0.0,
        1.0,
    )
}

/// Translate `peniko` fill rules to the equivalent Skia path fill type.
fn sk_path_fill_type_from_fill_rule(rule: peniko::Fill) -> sk::PathFillType {
    match rule {
        peniko::Fill::NonZero => sk::PathFillType::Winding,
        peniko::Fill::EvenOdd => sk::PathFillType::EvenOdd,
    }
}

/// Return a path with the requested fill rule, cloning only when the rule must change.
fn path_with_fill_rule(path: &sk::Path, rule: peniko::Fill) -> sk::Path {
    let fill = sk_path_fill_type_from_fill_rule(rule);
    if path.fill_type() == fill {
        path.clone()
    } else {
        path.with_fill_type(fill)
    }
}

/// Normalize supported geometry inputs to a `kurbo::BezPath` for downstream conversion.
fn geometry_to_bez_path(geom: GeometryRef<'_>, tolerance: f64) -> Option<kurbo::BezPath> {
    Some(match geom {
        GeometryRef::Rect(r) => r.to_path(tolerance),
        GeometryRef::RoundedRect(rr) => rr.to_path(tolerance),
        GeometryRef::Path(p) => p.clone(),
        GeometryRef::OwnedPath(p) => p,
    })
}

/// Convert supported geometry inputs to a Skia path using the configured tolerance.
fn geometry_to_sk_path(geom: GeometryRef<'_>, tolerance: f64) -> Option<sk::Path> {
    let bez = geometry_to_bez_path(geom, tolerance)?;
    bez_to_sk_path(&bez)
}

/// Translate a `kurbo` bezier path into the equivalent Skia path commands.
fn bez_to_sk_path(bez: &kurbo::BezPath) -> Option<sk::Path> {
    let mut path = sk::Path::new();
    for el in bez.elements() {
        match el {
            kurbo::PathEl::MoveTo(p) => {
                path.move_to((f64_to_f32(p.x), f64_to_f32(p.y)));
            }
            kurbo::PathEl::LineTo(p) => {
                path.line_to((f64_to_f32(p.x), f64_to_f32(p.y)));
            }
            kurbo::PathEl::QuadTo(p1, p2) => {
                path.quad_to(
                    (f64_to_f32(p1.x), f64_to_f32(p1.y)),
                    (f64_to_f32(p2.x), f64_to_f32(p2.y)),
                );
            }
            kurbo::PathEl::CurveTo(p1, p2, p3) => {
                path.cubic_to(
                    (f64_to_f32(p1.x), f64_to_f32(p1.y)),
                    (f64_to_f32(p2.x), f64_to_f32(p2.y)),
                    (f64_to_f32(p3.x), f64_to_f32(p3.y)),
                );
            }
            kurbo::PathEl::ClosePath => {
                path.close();
            }
        }
    }
    Some(path)
}

/// Map `peniko` extend behavior to Skia tile modes for gradients and image shaders.
fn tile_mode_from_extend(extend: peniko::Extend) -> sk::TileMode {
    match extend {
        peniko::Extend::Pad => sk::TileMode::Clamp,
        peniko::Extend::Repeat => sk::TileMode::Repeat,
        peniko::Extend::Reflect => sk::TileMode::Mirror,
    }
}

/// Map `peniko` interpolation color spaces to Skia's gradient interpolation settings.
fn gradient_shader_cs_from_cs_tag(
    color_space: ColorSpaceTag,
) -> sk::gradient_shader::interpolation::ColorSpace {
    use sk::gradient_shader::interpolation::ColorSpace as SkCs;

    match color_space {
        ColorSpaceTag::Srgb => SkCs::SRGB,
        ColorSpaceTag::LinearSrgb => SkCs::SRGBLinear,
        ColorSpaceTag::Lab => SkCs::Lab,
        ColorSpaceTag::Lch => SkCs::LCH,
        ColorSpaceTag::Hsl => SkCs::HSL,
        ColorSpaceTag::Hwb => SkCs::HWB,
        ColorSpaceTag::Oklab => SkCs::OKLab,
        ColorSpaceTag::Oklch => SkCs::OKLCH,
        ColorSpaceTag::DisplayP3 => SkCs::DisplayP3,
        ColorSpaceTag::A98Rgb => SkCs::A98RGB,
        ColorSpaceTag::ProphotoRgb => SkCs::ProphotoRGB,
        ColorSpaceTag::Rec2020 => SkCs::Rec2020,
        _ => SkCs::SRGB,
    }
}

/// Map `peniko` hue interpolation direction to Skia's gradient hue method.
fn gradient_shader_hue_method_from_hue_direction(
    direction: HueDirection,
) -> sk::gradient_shader::interpolation::HueMethod {
    use sk::gradient_shader::interpolation::HueMethod as SkHue;

    match direction {
        HueDirection::Shorter => SkHue::Shorter,
        HueDirection::Longer => SkHue::Longer,
        HueDirection::Increasing => SkHue::Increasing,
        HueDirection::Decreasing => SkHue::Decreasing,
        _ => SkHue::Shorter,
    }
}

/// Convert a `peniko` color to Skia's packed 8-bit color representation.
fn color_to_sk_color(color: peniko::Color) -> sk::Color {
    let rgba = color.to_rgba8();
    sk::Color::from_argb(rgba.a, rgba.r, rgba.g, rgba.b)
}

/// Convert a `peniko` color to Skia's floating-point color representation.
fn color_to_sk_color4f(color: peniko::Color) -> sk::Color4f {
    let comps = color.components;
    sk::Color4f::new(comps[0], comps[1], comps[2], comps[3])
}

/// Build a configured Skia paint from a semantic `peniko` brush description.
///
/// This is the main translation point from `imaging` brush semantics into Skia shaders, colors,
/// image sampling, and opacity handling.
fn brush_to_paint(brush: BrushRef<'_>, opacity: f32, paint_xf: Affine) -> Option<sk::Paint> {
    let mut paint = sk::Paint::default();
    paint.set_anti_alias(true);
    let alpha_scale = opacity.clamp(0.0, 1.0);

    match brush {
        BrushRef::Solid(color) => {
            // Use float color to avoid quantizing alpha (important for Porter-Duff ops like XOR).
            let comps = color.components;
            let c = sk::Color4f::new(comps[0], comps[1], comps[2], comps[3] * alpha_scale);
            paint.set_color4f(c, None);
        }
        BrushRef::Gradient(grad) => {
            let stops = grad.stops.as_ref();
            if stops.is_empty() {
                paint.set_color(sk::Color::TRANSPARENT);
                return Some(paint);
            }

            let mut colors: Vec<sk::Color4f> = Vec::with_capacity(stops.len());
            let mut pos: Vec<f32> = Vec::with_capacity(stops.len());

            for s in stops {
                let color = s.color.to_alpha_color::<Srgb>().multiply_alpha(alpha_scale);
                colors.push(sk::Color4f::new(
                    color.components[0],
                    color.components[1],
                    color.components[2],
                    color.components[3],
                ));
                pos.push(s.offset.clamp(0.0, 1.0));
            }

            let tile_mode = tile_mode_from_extend(grad.extend);
            let local = affine_to_matrix(paint_xf);

            let interpolation = sk::gradient_shader::Interpolation {
                color_space: gradient_shader_cs_from_cs_tag(grad.interpolation_cs),
                in_premul: match grad.interpolation_alpha_space {
                    InterpolationAlphaSpace::Premultiplied => {
                        sk::gradient_shader::interpolation::InPremul::Yes
                    }
                    InterpolationAlphaSpace::Unpremultiplied => {
                        sk::gradient_shader::interpolation::InPremul::No
                    }
                },
                hue_method: gradient_shader_hue_method_from_hue_direction(grad.hue_direction),
            };

            match &grad.kind {
                peniko::GradientKind::Linear(line) => {
                    let p0 = sk::Point::new(f64_to_f32(line.start.x), f64_to_f32(line.start.y));
                    let p1 = sk::Point::new(f64_to_f32(line.end.x), f64_to_f32(line.end.y));
                    if let Some(shader) = sk::Shader::linear_gradient_with_interpolation(
                        (p0, p1),
                        (&colors[..], None),
                        &pos[..],
                        tile_mode,
                        interpolation,
                        Some(&local),
                    ) {
                        paint.set_shader(shader);
                    }
                }
                peniko::GradientKind::Radial(rad) => {
                    let start_center = sk::Point::new(
                        f64_to_f32(rad.start_center.x),
                        f64_to_f32(rad.start_center.y),
                    );
                    let start_radius = rad.start_radius;
                    let end_center =
                        sk::Point::new(f64_to_f32(rad.end_center.x), f64_to_f32(rad.end_center.y));
                    let end_radius = rad.end_radius;

                    if let Some(shader) = sk::Shader::two_point_conical_gradient_with_interpolation(
                        (start_center, start_radius),
                        (end_center, end_radius),
                        (&colors[..], None),
                        &pos[..],
                        tile_mode,
                        interpolation,
                        Some(&local),
                    ) {
                        paint.set_shader(shader);
                    }
                }
                peniko::GradientKind::Sweep(sweep) => {
                    let center =
                        sk::Point::new(f64_to_f32(sweep.center.x), f64_to_f32(sweep.center.y));
                    // `peniko` uses radians; Skia uses degrees for sweep gradient angles.
                    let start = {
                        let rad = sweep.start_angle;
                        rad.to_degrees()
                    };
                    let end = {
                        let rad = sweep.end_angle;
                        rad.to_degrees()
                    };
                    if let Some(shader) = sk::Shader::sweep_gradient_with_interpolation(
                        center,
                        (&colors[..], None),
                        Some(&pos[..]),
                        tile_mode,
                        Some((start, end)),
                        interpolation,
                        Some(&local),
                    ) {
                        paint.set_shader(shader);
                    }
                }
            }

            if paint.shader().is_none()
                && let Some(last_stop) = stops.last()
            {
                let color = last_stop
                    .color
                    .to_alpha_color::<Srgb>()
                    .multiply_alpha(alpha_scale);
                paint.set_color(color_to_sk_color(color));
            }
        }
        BrushRef::Image(image_brush) => {
            let image = skia_image_from_peniko(image_brush.image)?;
            let shader = image.to_shader(
                Some((
                    tile_mode_from_extend(image_brush.sampler.x_extend),
                    tile_mode_from_extend(image_brush.sampler.y_extend),
                )),
                sampling_options_from_quality(image_brush.sampler.quality),
                Some(&affine_to_matrix(paint_xf)),
            )?;
            paint.set_shader(shader);
            paint.set_alpha_f((image_brush.sampler.alpha * alpha_scale).clamp(0.0, 1.0));
        }
    }

    Some(paint)
}

/// Convert a `peniko` image payload into a raster Skia image when its format is supported.
fn skia_image_from_peniko(image: &peniko::ImageData) -> Option<sk::Image> {
    let color_type = match image.format {
        ImageFormat::Rgba8 => sk::ColorType::RGBA8888,
        ImageFormat::Bgra8 => sk::ColorType::BGRA8888,
        _ => return None,
    };
    let alpha_type = match image.alpha_type {
        ImageAlphaType::Alpha => sk::AlphaType::Unpremul,
        ImageAlphaType::AlphaPremultiplied => sk::AlphaType::Premul,
    };
    let info = sk::ImageInfo::new(
        (
            i32::try_from(image.width).ok()?,
            i32::try_from(image.height).ok()?,
        ),
        color_type,
        alpha_type,
        None,
    );
    let row_bytes = image.format.size_in_bytes(image.width, 1)?;
    sk::images::raster_from_data(&info, sk::Data::new_copy(image.data.data()), row_bytes)
}

/// Map `peniko` image quality hints to Skia sampling configuration.
fn sampling_options_from_quality(quality: ImageQuality) -> sk::SamplingOptions {
    match quality {
        ImageQuality::Low => sk::SamplingOptions::from(sk::FilterMode::Nearest),
        ImageQuality::Medium => sk::SamplingOptions::from(sk::FilterMode::Linear),
        ImageQuality::High => sk::SamplingOptions::from(sk::CubicResampler::mitchell()),
    }
}

/// Apply `kurbo` stroke settings to a Skia paint before stroke drawing.
fn apply_stroke_style(paint: &mut sk::Paint, style: &kurbo::Stroke) {
    paint.set_style(sk::PaintStyle::Stroke);
    paint.set_stroke_width(f64_to_f32(style.width));
    paint.set_stroke_miter(f64_to_f32(style.miter_limit));
    paint.set_stroke_join(match style.join {
        kurbo::Join::Bevel => sk::PaintJoin::Bevel,
        kurbo::Join::Miter => sk::PaintJoin::Miter,
        kurbo::Join::Round => sk::PaintJoin::Round,
    });
    let cap = match style.start_cap {
        kurbo::Cap::Butt => sk::PaintCap::Butt,
        kurbo::Cap::Square => sk::PaintCap::Square,
        kurbo::Cap::Round => sk::PaintCap::Round,
    };
    paint.set_stroke_cap(cap);
    if !style.dash_pattern.is_empty() {
        let intervals: Vec<f32> = style.dash_pattern.iter().map(|v| f64_to_f32(*v)).collect();
        if let Some(effect) =
            sk::PathEffect::dash(intervals.as_slice(), f64_to_f32(style.dash_offset))
        {
            paint.set_path_effect(effect);
        }
    }
}

/// Map semantic `peniko` blend and composite modes to Skia blend modes.
fn map_blend_mode(mode: &peniko::BlendMode) -> sk::BlendMode {
    use peniko::{Compose, Mix};

    match (mode.mix, mode.compose) {
        (_, Compose::Clear) => sk::BlendMode::Clear,
        (_, Compose::Copy) => sk::BlendMode::Src,
        (_, Compose::Dest) => sk::BlendMode::Dst,
        (_, Compose::SrcOver) => match mode.mix {
            Mix::Normal => sk::BlendMode::SrcOver,
            Mix::Multiply => sk::BlendMode::Multiply,
            Mix::Screen => sk::BlendMode::Screen,
            Mix::Overlay => sk::BlendMode::Overlay,
            Mix::Darken => sk::BlendMode::Darken,
            Mix::Lighten => sk::BlendMode::Lighten,
            Mix::ColorDodge => sk::BlendMode::ColorDodge,
            Mix::ColorBurn => sk::BlendMode::ColorBurn,
            Mix::HardLight => sk::BlendMode::HardLight,
            Mix::SoftLight => sk::BlendMode::SoftLight,
            Mix::Difference => sk::BlendMode::Difference,
            Mix::Exclusion => sk::BlendMode::Exclusion,
            Mix::Hue => sk::BlendMode::Hue,
            Mix::Saturation => sk::BlendMode::Saturation,
            Mix::Color => sk::BlendMode::Color,
            Mix::Luminosity => sk::BlendMode::Luminosity,
        },
        (_, Compose::DestOver) => sk::BlendMode::DstOver,
        (_, Compose::SrcIn) => sk::BlendMode::SrcIn,
        (_, Compose::DestIn) => sk::BlendMode::DstIn,
        (_, Compose::SrcOut) => sk::BlendMode::SrcOut,
        (_, Compose::DestOut) => sk::BlendMode::DstOut,
        (_, Compose::SrcAtop) => sk::BlendMode::SrcATop,
        (_, Compose::DestAtop) => sk::BlendMode::DstATop,
        (_, Compose::Xor) => sk::BlendMode::Xor,
        (_, Compose::Plus) => sk::BlendMode::Plus,
        (_, Compose::PlusLighter) => sk::BlendMode::Plus,
    }
}

/// Build the Skia image-filter chain used for group and layer effects.
fn build_filter_chain(filters: &[Filter]) -> Option<sk::ImageFilter> {
    use sk::image_filters;

    let mut current: Option<sk::ImageFilter> = None;
    for f in filters {
        current = Some(match *f {
            Filter::Flood { color } => {
                let shader = sk::shaders::color(color_to_sk_color(color));
                // Leaf filter: ignores any existing input chain.
                image_filters::shader(shader, None)?
            }
            Filter::Blur {
                std_deviation_x,
                std_deviation_y,
            } => image_filters::blur((std_deviation_x, std_deviation_y), None, current, None)?,
            Filter::DropShadow {
                dx,
                dy,
                std_deviation_x,
                std_deviation_y,
                color,
            } => image_filters::drop_shadow(
                (dx, dy),
                (std_deviation_x, std_deviation_y),
                color_to_sk_color4f(color),
                None,
                current,
                None,
            )?,
            Filter::Offset { dx, dy } => image_filters::offset((dx, dy), current, None)?,
        });
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;
    use imaging::{GroupRef, MaskMode, Painter};
    use kurbo::Rect;
    use peniko::{Brush, Color};

    fn masked_scene(mode: MaskMode) -> Scene {
        let mask = Painter::<Scene>::record_mask(mode, |mask| {
            mask.fill(
                Rect::new(8.0, 8.0, 56.0, 56.0),
                Color::from_rgba8(255, 255, 255, 160),
            )
            .draw();
        });

        let mut scene = Scene::new();
        {
            let mut painter = Painter::new(&mut scene);
            painter.with_group(GroupRef::new().with_mask(mask.as_ref()), |content| {
                content
                    .fill(
                        Rect::new(0.0, 0.0, 64.0, 64.0),
                        Color::from_rgb8(0x2a, 0x6f, 0xdb),
                    )
                    .draw();
            });
        }

        scene
    }

    #[test]
    fn render_picture_rgba8_reads_native_picture() {
        let mut sink = SkPictureRecorderSink::new(Rect::new(0.0, 0.0, 32.0, 32.0));
        let paint = Brush::Solid(Color::from_rgb8(0x22, 0x66, 0xaa));
        {
            let mut painter = Painter::new(&mut sink);
            painter.fill_rect(Rect::new(0.0, 0.0, 32.0, 32.0), &paint);
        }

        let picture = sink.finish_picture().unwrap();
        let mut renderer = SkiaRenderer::new(32, 32);
        let rgba = renderer.render_picture_rgba8(&picture).unwrap();

        assert_eq!(rgba.len(), 32 * 32 * 4);
        assert_eq!(&rgba[..4], &[0x22, 0x66, 0xaa, 0xff]);
    }

    #[test]
    fn render_scene_reuses_cached_masks_for_identical_scenes() {
        let scene = masked_scene(MaskMode::Alpha);
        let mut renderer = SkiaRenderer::new(64, 64);

        renderer.render_scene_rgba8(&scene).unwrap();
        assert_eq!(renderer.mask_cache.borrow().len(), 1);

        renderer.render_scene_rgba8(&scene).unwrap();
        assert_eq!(renderer.mask_cache.borrow().len(), 1);
    }

    #[test]
    fn clear_cached_masks_drops_realized_masks() {
        let scene = masked_scene(MaskMode::Luminance);
        let mut renderer = SkiaRenderer::new(64, 64);

        renderer.render_scene_rgba8(&scene).unwrap();
        assert_eq!(renderer.mask_cache.borrow().len(), 1);

        renderer.clear_cached_masks();
        assert_eq!(renderer.mask_cache.borrow().len(), 0);

        renderer.render_scene_rgba8(&scene).unwrap();
        assert_eq!(renderer.mask_cache.borrow().len(), 1);
    }

    #[test]
    fn changing_tolerance_clears_cached_masks() {
        let scene = masked_scene(MaskMode::Alpha);
        let mut renderer = SkiaRenderer::new(64, 64);

        renderer.render_scene_rgba8(&scene).unwrap();
        assert_eq!(renderer.mask_cache.borrow().len(), 1);

        renderer.set_tolerance(0.25);
        assert_eq!(renderer.mask_cache.borrow().len(), 0);
    }

    #[cfg(feature = "wgpu")]
    #[test]
    fn maps_supported_wgpu_texture_formats_for_wrapped_surfaces() {
        assert_eq!(
            color_type_for_wgpu_texture_format(wgpu::TextureFormat::Rgba8Unorm).unwrap(),
            sk::ColorType::RGBA8888
        );
        assert_eq!(
            color_type_for_wgpu_texture_format(wgpu::TextureFormat::Rgba8UnormSrgb).unwrap(),
            sk::ColorType::SRGBA8888
        );
        assert_eq!(
            color_type_for_wgpu_texture_format(wgpu::TextureFormat::Bgra8Unorm).unwrap(),
            sk::ColorType::BGRA8888
        );
        assert_eq!(
            color_type_for_wgpu_texture_format(wgpu::TextureFormat::Bgra8UnormSrgb).unwrap(),
            sk::ColorType::BGRA8888
        );
        assert_eq!(
            color_type_for_wgpu_texture_format(wgpu::TextureFormat::Rgb10a2Unorm).unwrap(),
            sk::ColorType::RGBA1010102
        );
        assert_eq!(
            color_type_for_wgpu_texture_format(wgpu::TextureFormat::Rgba16Unorm).unwrap(),
            sk::ColorType::R16G16B16A16UNorm
        );
        assert_eq!(
            color_type_for_wgpu_texture_format(wgpu::TextureFormat::Rgba16Float).unwrap(),
            sk::ColorType::RGBAF16
        );
    }
}
