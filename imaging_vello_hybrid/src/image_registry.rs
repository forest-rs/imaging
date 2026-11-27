// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::Error;
use peniko::ImageData;
use std::collections::VecDeque;
use std::sync::Arc;
use vello_common::paint::ImageSource;
use vello_common::pixmap::Pixmap;

#[derive(Debug)]
pub(crate) struct ImageTexture {
    pub(crate) view: wgpu::TextureView,
    pub(crate) may_have_transparency: bool,
}

#[derive(Debug)]
pub(crate) struct HybridImageRegistry {
    live: VecDeque<RegisteredImage>,
    bytes_used: u64,
    max_bytes: u64,
}

impl Default for HybridImageRegistry {
    fn default() -> Self {
        Self::new(64 * 1024 * 1024)
    }
}

impl HybridImageRegistry {
    pub(crate) fn new(max_bytes: u64) -> Self {
        Self {
            live: VecDeque::new(),
            bytes_used: 0,
            max_bytes,
        }
    }

    pub(crate) fn begin_upload_session<'a>(
        &'a mut self,
        device: &'a wgpu::Device,
        queue: &'a wgpu::Queue,
    ) -> HybridImageUploadSession<'a> {
        // Retained scenes pin their textures even when the cache exceeds its budget.
        self.evict_unused(self.max_bytes);
        HybridImageUploadSession {
            registry: self,
            device,
            queue,
            pending: Vec::new(),
        }
    }

    fn evict_unused(&mut self, target_bytes: u64) {
        self.live.retain(|image| {
            if self.bytes_used > target_bytes && Arc::strong_count(&image.texture) == 1 {
                self.bytes_used -= image.bytes;
                false
            } else {
                true
            }
        });
    }

    pub(crate) fn clear(&mut self) {
        self.evict_unused(0);
    }
}

pub(crate) struct HybridImageUploadSession<'a> {
    registry: &'a mut HybridImageRegistry,
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    pending: Vec<(RegisteredImage, Arc<Pixmap>)>,
}

impl HybridImageUploadSession<'_> {
    pub(crate) fn resolve_image(&mut self, data: &ImageData) -> Result<Arc<ImageTexture>, Error> {
        let key = ImageKey::derive(data);
        if let Some((image, _)) = self.pending.iter().find(|(image, _)| image.key == key) {
            return Ok(Arc::clone(&image.texture));
        }
        if let Some(index) = self.registry.live.iter().position(|image| image.key == key) {
            let image = self.registry.live.remove(index).unwrap();
            let texture = Arc::clone(&image.texture);
            self.registry.live.push_back(image);
            return Ok(texture);
        }

        let limit = self
            .device
            .limits()
            .max_texture_dimension_2d
            .min(u32::from(u16::MAX));
        if data.width == 0 || data.height == 0 || data.width > limit || data.height > limit {
            return Err(Error::UnsupportedImageBrush);
        }
        if !matches!(
            data.format,
            peniko::ImageFormat::Rgba8 | peniko::ImageFormat::Bgra8
        ) || data.format.size_in_bytes(data.width, data.height) != Some(data.data.len())
        {
            return Err(Error::UnsupportedImageBrush);
        }
        let ImageSource::Pixmap(pixmap) = ImageSource::from_peniko_image_data(data) else {
            unreachable!("peniko image conversion always produces a pixmap");
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("imaging_vello_hybrid inline image"),
            size: wgpu::Extent3d {
                width: data.width,
                height: data.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let texture = Arc::new(ImageTexture {
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            may_have_transparency: pixmap.may_have_transparency(),
        });
        self.pending.push((
            RegisteredImage {
                key,
                texture: Arc::clone(&texture),
                bytes: u64::from(data.width) * u64::from(data.height) * 4,
            },
            pixmap,
        ));
        Ok(texture)
    }

    /// Submit uploads and publish cache entries only after recording succeeds.
    pub(crate) fn finish(self) {
        if self.pending.is_empty() {
            return;
        }
        for (image, pixmap) in self.pending {
            let texture = image.texture.view.texture();
            self.queue.write_texture(
                texture.as_image_copy(),
                pixmap.data_as_u8_slice(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(texture.width() * 4),
                    rows_per_image: None,
                },
                texture.size(),
            );
            self.registry.bytes_used += image.bytes;
            self.registry.live.push_back(image);
        }
        self.queue.submit([]);
    }
}

#[derive(Debug)]
struct RegisteredImage {
    key: ImageKey,
    texture: Arc<ImageTexture>,
    bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ImageKey {
    format: core::mem::Discriminant<peniko::ImageFormat>,
    alpha_type: core::mem::Discriminant<peniko::ImageAlphaType>,
    width: u32,
    height: u32,
    blob_id: u64,
}

impl ImageKey {
    fn derive(image: &ImageData) -> Self {
        Self {
            format: core::mem::discriminant(&image.format),
            alpha_type: core::mem::discriminant(&image.alpha_type),
            width: image.width,
            height: image.height,
            blob_id: image.data.id(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ImageKey;
    use peniko::{Blob, ImageAlphaType, ImageData, ImageFormat};
    use std::sync::Arc;

    fn image(bytes: [u8; 16]) -> ImageData {
        ImageData {
            data: Blob::new(Arc::new(bytes)),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: 2,
            height: 2,
        }
    }

    #[test]
    fn image_key_reuses_cloned_assets() {
        let a = image([1, 2, 3, 4, 9, 8, 7, 6, 5, 4, 3, 2, 10, 11, 12, 13]);
        let b = image([1, 2, 3, 4, 9, 8, 7, 6, 5, 4, 3, 2, 10, 11, 12, 13]);
        assert_eq!(ImageKey::derive(&a), ImageKey::derive(&a.clone()));
        assert_ne!(ImageKey::derive(&a), ImageKey::derive(&b));
    }

    #[test]
    fn image_key_distinguishes_metadata_changes() {
        let mut a = image([1, 2, 3, 4, 9, 8, 7, 6, 5, 4, 3, 2, 10, 11, 12, 13]);
        let mut b = a.clone();
        b.alpha_type = ImageAlphaType::AlphaPremultiplied;
        assert_ne!(ImageKey::derive(&a), ImageKey::derive(&b));

        a.format = ImageFormat::Bgra8;
        assert_ne!(ImageKey::derive(&a), ImageKey::derive(&b));
    }
}
