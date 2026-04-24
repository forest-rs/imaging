// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use imaging::RgbaImage;
use std::sync::mpsc;

#[derive(Debug)]
pub(crate) enum ReadbackError {
    DevicePoll,
    CallbackDropped,
    BufferMap,
}

#[derive(Debug)]
pub(crate) struct ScratchTexture {
    texture: wgpu::Texture,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    label: &'static str,
}

impl ScratchTexture {
    pub(crate) fn new(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        label: &'static str,
    ) -> Self {
        let texture = create_texture(device, width, height, format, label);
        Self {
            texture,
            width,
            height,
            format,
            label,
        }
    }

    pub(crate) fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return;
        }

        self.texture = create_texture(device, width, height, self.format, self.label);
        self.width = width;
        self.height = height;
    }

    pub(crate) fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    pub(crate) const fn format(&self) -> wgpu::TextureFormat {
        self.format
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
    image.resize(width, height);
    read_texture_into_target(
        device,
        queue,
        texture,
        width,
        height,
        image.data.as_mut_slice(),
        usize::try_from(width).expect("image width should fit in usize") * 4,
    )
}

pub(crate) fn read_texture_into_target(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
    data: &mut [u8],
    bytes_per_row_out: usize,
) -> Result<(), ReadbackError> {
    let width_bytes = width * 4;
    let width_bytes_usize = width_bytes as usize;
    let bytes_per_row = width_bytes.div_ceil(256) * 256;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("imaging_skia readback"),
        size: u64::from(bytes_per_row) * u64::from(height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("imaging_skia readback"),
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
    for (row, out_row) in mapped
        .chunks_exact(bytes_per_row as usize)
        .zip(data.chunks_exact_mut(bytes_per_row_out))
    {
        out_row[..width_bytes_usize].copy_from_slice(&row[..width_bytes_usize]);
    }
    drop(mapped);
    readback.unmap();
    Ok(())
}

pub(crate) fn create_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    label: &'static str,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}
