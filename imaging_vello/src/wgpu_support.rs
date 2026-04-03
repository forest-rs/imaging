// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::wgpu;
use imaging::RgbaImage;
use std::sync::mpsc;

#[derive(Debug)]
pub(crate) enum ReadbackError {
    DevicePoll,
    CallbackDropped,
    BufferMap,
}

#[derive(Debug)]
pub(crate) struct OffscreenTarget {
    texture: wgpu::Texture,
    texture_view: wgpu::TextureView,
    width: u32,
    height: u32,
}

impl OffscreenTarget {
    pub(crate) fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let texture = create_texture(device, width, height);
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            texture_view,
            width,
            height,
        }
    }

    pub(crate) fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    pub(crate) fn texture_view(&self) -> &wgpu::TextureView {
        &self.texture_view
    }

    pub(crate) fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return;
        }

        self.texture = create_texture(device, width, height);
        self.texture_view = self
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.width = width;
        self.height = height;
    }

    pub(crate) const fn width(&self) -> u32 {
        self.width
    }

    pub(crate) const fn height(&self) -> u32 {
        self.height
    }
}

pub(crate) fn read_texture_into(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
    image: &mut RgbaImage,
) -> Result<(), ReadbackError> {
    let width_bytes = width * 4;
    let bytes_per_row = width_bytes.div_ceil(256) * 256;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("imaging_vello readback"),
        size: u64::from(bytes_per_row) * u64::from(height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

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
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);

    let slice = readback.slice(..);
    let (tx, rx) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|_| ReadbackError::DevicePoll)?;
    rx.recv()
        .map_err(|_| ReadbackError::CallbackDropped)?
        .map_err(|_| ReadbackError::BufferMap)?;

    let mapped = slice.get_mapped_range();
    let width_bytes = width_bytes as usize;
    image.resize(width, height);
    for (row, out_row) in mapped
        .chunks_exact(bytes_per_row as usize)
        .zip(image.data.chunks_exact_mut(width_bytes))
    {
        out_row.copy_from_slice(&row[..width_bytes]);
    }
    drop(mapped);
    readback.unmap();
    Ok(())
}

fn create_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("imaging_vello target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}
