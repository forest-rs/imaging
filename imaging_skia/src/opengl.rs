// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// Portions of this file are derived from `anyrender_skia` in the AnyRender project:
// <https://github.com/dioxuslabs/anyrender>
// Original source: `crates/anyrender_skia/src/opengl.rs`
// Adapted here for imaging's offscreen Ganesh renderer.

#![allow(unsafe_code, reason = "OpenGL and Skia FFI setup requires raw handles")]

use core::ffi::c_void;
use std::{ffi::CString, num::NonZeroU32};

use glutin::config::{ConfigTemplateBuilder, GlConfig};
use glutin::context::{ContextApi, ContextAttributesBuilder, PossiblyCurrentContext};
use glutin::display::{Display, DisplayApiPreference, GetGlDisplay};
use glutin::prelude::{GlDisplay, NotCurrentGlContext, PossiblyCurrentGlContext};
use glutin::surface::{PbufferSurface, Surface, SurfaceAttributesBuilder};
use raw_window_handle::{RawDisplayHandle, WindowsDisplayHandle, XlibDisplayHandle};
use skia_safe as sk;

use crate::{
    Error, SkiaRenderer, color_space_for_wgpu_texture_format, color_type_for_wgpu_texture_format,
    ganesh::GaneshBackend,
};

#[derive(Debug)]
pub(crate) struct OpenGlBackend {
    gl_surface: Surface<PbufferSurface>,
    gl_context: PossiblyCurrentContext,
    context: sk::gpu::DirectContext,
}

impl OpenGlBackend {
    /// Create the internal headless OpenGL backend used by the default Ganesh renderer path.
    ///
    /// This allocates a tiny pbuffer context so `imaging_skia` can own a self-contained GL-backed
    /// Skia context when the caller is not supplying one.
    pub(crate) fn new() -> Result<Self, Error> {
        let raw_display_handle = default_display_handle();
        let gl_display = unsafe {
            Display::new(raw_display_handle, DisplayApiPreference::Egl)
                .map_err(|_| Error::CreateGpuContext("unable to create EGL display"))?
        };

        let template = ConfigTemplateBuilder::new()
            .with_alpha_size(8)
            .with_transparency(false)
            .with_surface_type(glutin::config::ConfigSurfaceTypes::PBUFFER)
            .build();

        let gl_config = unsafe {
            gl_display
                .find_configs(template)
                .map_err(|_| Error::CreateGpuContext("unable to enumerate EGL configs"))?
                .reduce(|best, candidate| {
                    if candidate.num_samples() < best.num_samples() {
                        candidate
                    } else {
                        best
                    }
                })
                .ok_or(Error::CreateGpuContext("no suitable EGL config found"))?
        };

        let context_attributes = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::OpenGl(None))
            .build(None);
        let not_current = unsafe {
            gl_display
                .create_context(&gl_config, &context_attributes)
                .map_err(|_| Error::CreateGpuContext("unable to create OpenGL context"))?
        };

        let width = NonZeroU32::new(1).expect("non-zero");
        let height = NonZeroU32::new(1).expect("non-zero");
        let surface_attributes =
            SurfaceAttributesBuilder::<PbufferSurface>::new().build(width, height);
        let gl_surface = unsafe {
            gl_display
                .create_pbuffer_surface(&gl_config, &surface_attributes)
                .map_err(|_| Error::CreateGpuContext("unable to create OpenGL pbuffer"))?
        };

        let gl_context = not_current
            .make_current(&gl_surface)
            .map_err(|_| Error::CreateGpuContext("unable to make OpenGL context current"))?;

        let interface = sk::gpu::gl::Interface::new_load_with(|name| {
            gl_display.get_proc_address(CString::new(name).expect("GL symbol").as_c_str())
        })
        .ok_or(Error::CreateGpuContext("unable to load OpenGL interface"))?;
        let context = sk::gpu::direct_contexts::make_gl(interface, None).ok_or(
            Error::CreateGpuContext("unable to create Skia OpenGL context"),
        )?;

        Ok(Self {
            gl_surface,
            gl_context,
            context,
        })
    }

    /// Borrow the underlying Skia direct context.
    pub(crate) fn direct_context(&mut self) -> &mut sk::gpu::DirectContext {
        &mut self.context
    }

    /// Ensure the internal pbuffer GL context is current before issuing Skia work.
    pub(crate) fn ensure_current(&mut self) -> Result<(), Error> {
        if !self.gl_context.is_current() || !self.gl_surface.is_current(&self.gl_context) {
            self.gl_context
                .make_current(&self.gl_surface)
                .map_err(|_| Error::CreateGpuContext("unable to make OpenGL context current"))?;
        }
        Ok(())
    }
}

#[cfg(feature = "wgpu")]
impl SkiaRenderer {
    /// Create a renderer that draws into a caller-owned OpenGL or GLES texture.
    ///
    /// This is the explicit GL interop path for integrations where the application already owns the
    /// current context and texture object and only needs Skia to attach to them.
    ///
    /// `load_fn` must resolve GL function pointers for the current context, and that context must
    /// stay current whenever the renderer is used.
    ///
    /// # Safety
    ///
    /// The current GL context and texture must remain valid for the renderer lifetime.
    pub unsafe fn try_new_gl_from_texture_with_load_fn<F>(
        width: u16,
        height: u16,
        texture_format: wgpu::TextureFormat,
        texture_target: sk::gpu::gl::Enum,
        texture_id: sk::gpu::gl::Enum,
        load_fn: F,
    ) -> Result<Self, Error>
    where
        F: FnMut(&str) -> *const c_void,
    {
        let width = i32::from(width);
        let height = i32::from(height);
        let interface = sk::gpu::gl::Interface::new_load_with(load_fn).ok_or(
            Error::CreateGpuContext("unable to create Skia OpenGL interface"),
        )?;
        let context = sk::gpu::direct_contexts::make_gl(interface, None).ok_or(
            Error::CreateGpuContext("unable to create Skia OpenGL context"),
        )?;
        let mut backend = GaneshBackend::ExternalGl(context);
        let surface = create_wrapped_gl_surface(
            backend.direct_context(),
            width,
            height,
            texture_format,
            texture_target,
            texture_id,
        )?;
        Ok(Self::from_backend_surface(backend, surface))
    }

    /// Retarget the renderer to a different caller-owned OpenGL or GLES texture.
    ///
    /// Use this when the surrounding GL application recreates textures but wants to preserve the
    /// current Skia context and renderer state.
    ///
    /// # Safety
    ///
    /// The caller-managed GL context must be current, and the texture must remain valid.
    pub unsafe fn replace_gl_texture(
        &mut self,
        width: u16,
        height: u16,
        texture_format: wgpu::TextureFormat,
        texture_target: sk::gpu::gl::Enum,
        texture_id: sk::gpu::gl::Enum,
    ) -> Result<(), Error> {
        let width = i32::from(width);
        let height = i32::from(height);
        self.backend.ensure_current()?;
        self.backend.flush_surface(&mut self.surface);
        let surface = create_wrapped_gl_surface(
            self.backend.direct_context(),
            width,
            height,
            texture_format,
            texture_target,
            texture_id,
        )?;
        self.surface = surface;
        Ok(())
    }
}

#[cfg(feature = "wgpu")]
/// Wrap a caller-owned GL texture in a Skia surface for direct rendering.
fn create_wrapped_gl_surface(
    context: &mut sk::gpu::DirectContext,
    width: i32,
    height: i32,
    texture_format: wgpu::TextureFormat,
    texture_target: sk::gpu::gl::Enum,
    texture_id: sk::gpu::gl::Enum,
) -> Result<sk::Surface, Error> {
    let backend_texture = unsafe {
        sk::gpu::backend_textures::make_gl(
            (width, height),
            sk::gpu::Mipmapped::No,
            sk::gpu::gl::TextureInfo {
                target: texture_target,
                id: texture_id,
                format: gl_format_for_wgpu_texture_format(texture_format)?,
                protected: sk::gpu::Protected::No,
            },
            "",
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

#[cfg(feature = "wgpu")]
/// Map supported wrapped `wgpu` formats to GL internal formats for Skia interop.
fn gl_format_for_wgpu_texture_format(
    texture_format: wgpu::TextureFormat,
) -> Result<sk::gpu::gl::Enum, Error> {
    Ok(match texture_format {
        wgpu::TextureFormat::Rgba8Unorm => sk::gpu::gl::Format::RGBA8.into(),
        wgpu::TextureFormat::Rgba8UnormSrgb => sk::gpu::gl::Format::SRGB8_ALPHA8.into(),
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => {
            sk::gpu::gl::Format::BGRA8.into()
        }
        wgpu::TextureFormat::Rgb10a2Unorm => sk::gpu::gl::Format::RGB10_A2.into(),
        wgpu::TextureFormat::Rgba16Unorm => sk::gpu::gl::Format::RGBA16.into(),
        wgpu::TextureFormat::Rgba16Float => sk::gpu::gl::Format::RGBA16F.into(),
        _ => return Err(Error::Internal("unsupported OpenGL wgpu texture format")),
    })
}

/// Build a placeholder raw display handle for headless GL context creation on the host platform.
fn default_display_handle() -> RawDisplayHandle {
    #[cfg(target_os = "windows")]
    {
        RawDisplayHandle::Windows(WindowsDisplayHandle::new())
    }
    #[cfg(not(target_os = "windows"))]
    {
        RawDisplayHandle::Xlib(XlibDisplayHandle::new(None, 0))
    }
}
