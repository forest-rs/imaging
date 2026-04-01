// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Vello backend for `imaging`.
//!
//! This crate provides a headless CPU/GPU renderer that consumes `imaging::record::Scene` or a
//! native [`vello::Scene`] and produces an RGBA8 image buffer using `vello` + `wgpu`.
//!
//! Enable exactly one backend compatibility feature:
//!
//! - `vello-0-8` (default)
//! - `vello-0-7`
//!
//! # Render A Recorded Scene
//!
//! Record commands into [`imaging::record::Scene`], then render them with [`VelloRenderer`].
//!
//! ```no_run
//! use imaging::{Painter, record};
//! use imaging_vello::VelloRenderer;
//! use kurbo::Rect;
//! use peniko::{Brush, Color};
//!
//! fn main() -> Result<(), imaging_vello::Error> {
//!     let paint = Brush::Solid(Color::from_rgb8(0x2a, 0x6f, 0xdb));
//!     let mut scene = record::Scene::new();
//!
//!     {
//!         let mut painter = Painter::new(&mut scene);
//!         painter.fill_rect(Rect::new(0.0, 0.0, 128.0, 128.0), &paint);
//!     }
//!
//!     let mut renderer = VelloRenderer::try_new(128, 128)?;
//!     let rgba = renderer.render_scene_rgba8(&scene)?;
//!     assert_eq!(rgba.len(), 128 * 128 * 4);
//!     Ok(())
//! }
//! ```
//!
//! # Record Into `vello::Scene`
//!
//! If you want a backend-native retained scene without going through [`VelloRenderer`], wrap a
//! mutable [`vello::Scene`] with [`VelloSceneSink`].
//!
//! ```no_run
//! use imaging::Painter;
//! use imaging_vello::{VelloSceneSink, vello};
//! use kurbo::Rect;
//! use peniko::{Brush, Color};
//!
//! fn main() -> Result<(), imaging_vello::Error> {
//!     let paint = Brush::Solid(Color::from_rgb8(0x1d, 0x4e, 0x89));
//!     let mut scene = vello::Scene::new();
//!
//!     {
//!         let bounds = Rect::new(0.0, 0.0, 128.0, 128.0);
//!         let mut sink = VelloSceneSink::new(&mut scene, bounds);
//!         let mut painter = Painter::new(&mut sink);
//!         painter.fill_rect(bounds, &paint);
//!         sink.finish()?;
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! # Render A Native `vello::Scene`
//!
//! If you already have a native Vello scene, hand it directly to [`VelloRenderer`].
//!
//! ```no_run
//! use imaging::Painter;
//! use imaging_vello::{VelloRenderer, VelloSceneSink, vello};
//! use kurbo::Rect;
//! use peniko::{Brush, Color};
//!
//! fn main() -> Result<(), imaging_vello::Error> {
//!     let paint = Brush::Solid(Color::from_rgb8(0xd9, 0x77, 0x06));
//!     let mut scene = vello::Scene::new();
//!
//!     {
//!         let bounds = Rect::new(0.0, 0.0, 128.0, 128.0);
//!         let mut sink = VelloSceneSink::new(&mut scene, bounds);
//!         let mut painter = Painter::new(&mut sink);
//!         painter.fill_rect(bounds, &paint);
//!         sink.finish()?;
//!     }
//!
//!     let mut renderer = VelloRenderer::try_new(128, 128)?;
//!     let rgba = renderer.render_vello_scene_rgba8(&scene)?;
//!     assert_eq!(rgba.len(), 128 * 128 * 4);
//!     Ok(())
//! }
//! ```
//!
//! Note: Vello uses a single layer stack for clipping and blending. Scenes that interleave clips
//! and groups in ways Vello cannot represent may return [`Error::UnbalancedLayerStack`].

#![deny(unsafe_code)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod scene_sink;

#[cfg(all(feature = "vello-0-7", feature = "vello-0-8"))]
compile_error!("Enable exactly one of `vello-0-7` or `vello-0-8`.");

#[cfg(not(any(feature = "vello-0-7", feature = "vello-0-8")))]
compile_error!("Enable one of `vello-0-7` or `vello-0-8`.");

use imaging::record::{Scene, ValidateError, replay};
use kurbo::Rect;
use std::sync::mpsc;

#[cfg(feature = "vello-0-7")]
pub use vello_07 as vello;
#[cfg(all(not(feature = "vello-0-7"), feature = "vello-0-8"))]
pub use vello_08 as vello;

use crate::vello::wgpu;
use crate::vello::{AaConfig, RenderParams};

pub use scene_sink::VelloSceneSink;

/// Errors that can occur when rendering via Vello.
#[derive(Debug)]
pub enum Error {
    /// The scene is invalid (unbalanced stacks).
    InvalidScene(ValidateError),
    /// An image brush was encountered; this backend does not support it.
    UnsupportedImageBrush,
    /// A filter configuration could not be translated.
    UnsupportedFilter,
    /// A mask mode or masking primitive is not supported by this backend.
    UnsupportedMask,
    /// Glyph draws with non-default blend modes are not supported by this backend yet.
    UnsupportedGlyphBlend,
    /// Blurred rounded rect draws with non-default blend modes are not supported by this backend yet.
    UnsupportedBlurredRoundedRectBlend,
    /// The clip/group stack was not well-nested for this backend.
    ///
    /// Vello uses a single layer stack for both clipping and blending; `imaging` tracks these as
    /// separate stacks, so scenes that interleave them (e.g. `push_clip`, `push_group`, `pop_clip`)
    /// cannot be represented directly.
    UnbalancedLayerStack,
    /// No suitable GPU adapter was found.
    NoAdapter,
    /// A GPU device could not be created.
    RequestDevice,
    /// Vello returned a render error.
    Render(vello::Error),
    /// An internal invariant was violated.
    Internal(&'static str),
}

/// Renderer that executes `imaging` commands using `vello` + `wgpu`.
pub struct VelloRenderer {
    renderer: vello::Renderer,
    device: wgpu::Device,
    queue: wgpu::Queue,
    texture: wgpu::Texture,
    texture_view: wgpu::TextureView,
    readback: wgpu::Buffer,
    bytes_per_row: u32,
    width: u16,
    height: u16,
}

impl core::fmt::Debug for VelloRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VelloRenderer")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl VelloRenderer {
    /// Create a renderer for a fixed-size target.
    pub fn new(width: u16, height: u16) -> Self {
        Self::try_new(width, height).expect("create imaging_vello renderer")
    }

    /// Create a renderer for a fixed-size target.
    ///
    /// This is fallible because `wgpu` may not be able to find a compatible adapter/device
    /// in some sandboxed or headless environments.
    pub fn try_new(width: u16, height: u16) -> Result<Self, Error> {
        let (device, queue) = pollster::block_on(init_device_and_queue())?;
        let (texture, texture_view, readback, bytes_per_row) =
            create_targets(&device, width, height);

        let renderer = vello::Renderer::new(&device, vello::RendererOptions::default())
            .map_err(Error::Render)?;

        Ok(Self {
            renderer,
            device,
            queue,
            texture,
            texture_view,
            readback,
            bytes_per_row,
            width,
            height,
        })
    }

    /// Render a recorded scene and return an RGBA8 buffer (unpremultiplied).
    pub fn render_scene_rgba8(&mut self, scene: &Scene) -> Result<Vec<u8>, Error> {
        scene.validate().map_err(Error::InvalidScene)?;
        let mut native = vello::Scene::new();
        let bounds = Rect::new(0.0, 0.0, f64::from(self.width), f64::from(self.height));
        let mut sink = VelloSceneSink::new(&mut native, bounds);
        replay(scene, &mut sink);
        sink.finish()?;
        self.render_vello_scene_rgba8(&native)
    }

    /// Render a native [`crate::vello::Scene`] and return an RGBA8 buffer (unpremultiplied).
    pub fn render_vello_scene_rgba8(&mut self, scene: &vello::Scene) -> Result<Vec<u8>, Error> {
        let params = RenderParams {
            base_color: peniko::Color::from_rgba8(0, 0, 0, 0),
            width: u32::from(self.width),
            height: u32::from(self.height),
            antialiasing_method: AaConfig::Area,
        };

        self.renderer
            .render_to_texture(
                &self.device,
                &self.queue,
                scene,
                &self.texture_view,
                &params,
            )
            .map_err(Error::Render)?;

        readback_rgba8(
            &self.device,
            &self.queue,
            &self.texture,
            &self.readback,
            self.bytes_per_row,
            self.width,
            self.height,
        )
    }
}

async fn init_device_and_queue() -> Result<(wgpu::Device, wgpu::Queue), Error> {
    let instance = wgpu::Instance::default();
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            force_fallback_adapter: false,
            compatible_surface: None,
        })
        .await
        .map_err(|_| Error::NoAdapter)?;

    adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("imaging_vello device"),
            required_features: wgpu::Features::empty(),
            ..Default::default()
        })
        .await
        .map_err(|_| Error::RequestDevice)
}

fn create_targets(
    device: &wgpu::Device,
    width: u16,
    height: u16,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::Buffer, u32) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("imaging_vello render target"),
        size: wgpu::Extent3d {
            width: u32::from(width),
            height: u32::from(height),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let bytes_per_row = u32::from(width) * 4;
    let padded_bytes_per_row = bytes_per_row.div_ceil(256) * 256;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("imaging_vello readback"),
        size: u64::from(padded_bytes_per_row) * u64::from(height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    (texture, texture_view, readback, padded_bytes_per_row)
}

fn readback_rgba8(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    readback: &wgpu::Buffer,
    bytes_per_row: u32,
    width: u16,
    height: u16,
) -> Result<Vec<u8>, Error> {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("imaging_vello readback"),
    });

    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: u32::from(width),
            height: u32::from(height),
            depth_or_array_layers: 1,
        },
    );

    queue.submit([encoder.finish()]);

    let slice = readback.slice(..);
    let (tx, rx) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|_| Error::Internal("device poll failed"))?;
    rx.recv()
        .map_err(|_| Error::Internal("map_async callback dropped"))?
        .map_err(|_| Error::Internal("buffer map failed"))?;

    let mapped = slice.get_mapped_range();
    let width_bytes = usize::from(width) * 4;

    let mut out = Vec::with_capacity(usize::from(width) * usize::from(height) * 4);
    for row in mapped.chunks_exact(bytes_per_row as usize) {
        out.extend_from_slice(&row[..width_bytes]);
    }
    drop(mapped);
    readback.unmap();
    Ok(out)
}
