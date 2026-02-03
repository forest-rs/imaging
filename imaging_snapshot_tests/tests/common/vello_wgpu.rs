// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Minimal `wgpu` harness for rendering `vello::Scene` to an RGBA8 buffer.
//!
//! This is based on the approach used by Vello's own test utilities to avoid lifetime/teardown
//! issues on some platforms (notably D3D12 on Windows): allocate per-frame textures and readback
//! buffers and keep orchestration in the test harness rather than backend crates.

use std::sync::mpsc;

use peniko::Color;
use vello::wgpu;

#[derive(Debug)]
pub(crate) enum Error {
    NoAdapter,
    RequestDevice,
    Render(vello::Error),
    Internal(&'static str),
}

pub(crate) struct Context {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl Context {
    pub(crate) fn try_new() -> Result<Self, Error> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            force_fallback_adapter: false,
            compatible_surface: None,
        }))
        .map_err(|_| Error::NoAdapter)?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("imaging_snapshot_tests vello device"),
            required_features: wgpu::Features::empty(),
            ..Default::default()
        }))
        .map_err(|_| Error::RequestDevice)?;

        Ok(Self { device, queue })
    }

    pub(crate) fn render_rgba8(
        &self,
        scene: &vello::Scene,
        width: u16,
        height: u16,
    ) -> Result<Vec<u8>, Error> {
        let mut renderer = vello::Renderer::new(&self.device, vello::RendererOptions::default())
            .map_err(Error::Render)?;

        let size = wgpu::Extent3d {
            width: u32::from(width),
            height: u32::from(height),
            depth_or_array_layers: 1,
        };

        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("imaging_snapshot_tests vello target"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());

        let params = vello::RenderParams {
            base_color: Color::from_rgba8(0, 0, 0, 0),
            width: u32::from(width),
            height: u32::from(height),
            antialiasing_method: vello::AaConfig::Area,
        };

        renderer
            .render_to_texture(&self.device, &self.queue, scene, &view, &params)
            .map_err(Error::Render)?;

        let padded_bytes_per_row = (u32::from(width) * 4).next_multiple_of(256);
        let buffer_size = u64::from(padded_bytes_per_row) * u64::from(height);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("imaging_snapshot_tests vello readback"),
            size: buffer_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("imaging_snapshot_tests vello copy out buffer"),
            });
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: None,
                },
            },
            size,
        );
        self.queue.submit([encoder.finish()]);

        let buf_slice = buffer.slice(..);
        let (tx, rx) = mpsc::channel();
        buf_slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|_| Error::Internal("device poll failed"))?;
        rx.recv()
            .map_err(|_| Error::Internal("map_async callback dropped"))?
            .map_err(|_| Error::Internal("buffer map failed"))?;

        let data = buf_slice.get_mapped_range();
        let mut out = Vec::with_capacity(usize::from(width) * usize::from(height) * 4);
        let width_bytes = usize::from(width) * 4;
        for row in 0..u32::from(height) {
            let start = (row * padded_bytes_per_row) as usize;
            out.extend_from_slice(&data[start..start + width_bytes]);
        }
        drop(data);
        buffer.unmap();

        Ok(out)
    }
}
