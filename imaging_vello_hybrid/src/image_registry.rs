// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::Error;
use peniko::{ImageBrush, ImageData};
use std::collections::VecDeque;
use std::sync::Arc;
use vello_common::paint::{Image as VelloImage, ImageId, ImageSource};

#[derive(Debug)]
pub(crate) struct HybridImageRegistry {
    live: VecDeque<Arc<RegisteredImage>>,
    bytes_used: usize,
    max_bytes: usize,
}

impl Default for HybridImageRegistry {
    fn default() -> Self {
        Self::new(64 * 1024 * 1024)
    }
}

impl HybridImageRegistry {
    pub(crate) fn new(max_bytes: usize) -> Self {
        Self {
            live: VecDeque::new(),
            bytes_used: 0,
            max_bytes,
        }
    }

    pub(crate) fn begin_upload_session<'a>(
        &'a mut self,
        resources: &mut vello_gpu::Resources,
        renderer: &'a mut vello_gpu::Renderer,
        device: &'a wgpu::Device,
        queue: &'a wgpu::Queue,
        mut encoder: wgpu::CommandEncoder,
    ) -> HybridImageUploadSession<'a> {
        // The budget is soft: retained scenes pin their image allocations. Evict only
        // unused images, before uploads for the next recording begin.
        self.evict_to_budget(resources, renderer, &mut encoder);

        HybridImageUploadSession {
            registry: self,
            renderer,
            device,
            queue,
            encoder: Some(encoder),
            pending: Vec::new(),
        }
    }

    /// Returns the new index of the touched entry.
    fn touch(&mut self, index: usize) -> usize {
        if index + 1 == self.live.len() {
            return index;
        }
        if let Some(image) = self.live.remove(index) {
            self.live.push_back(image);
            return self.live.len() - 1;
        }
        index
    }

    fn evict_to_budget(
        &mut self,
        resources: &mut vello_gpu::Resources,
        renderer: &mut vello_gpu::Renderer,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        self.evict_unused(resources, renderer, encoder, self.max_bytes);
    }

    fn evict_unused(
        &mut self,
        resources: &mut vello_gpu::Resources,
        renderer: &mut vello_gpu::Renderer,
        encoder: &mut wgpu::CommandEncoder,
        target_bytes: usize,
    ) {
        let mut bytes_used = self.bytes_used;
        self.live.retain(|image| {
            if bytes_used > target_bytes && Arc::strong_count(image) == 1 {
                bytes_used = bytes_used.saturating_sub(image.bytes);
                renderer.destroy_image(resources, encoder, image.id);
                false
            } else {
                true
            }
        });
        self.bytes_used = bytes_used;
    }

    pub(crate) fn clear(
        &mut self,
        resources: &mut vello_gpu::Resources,
        renderer: &mut vello_gpu::Renderer,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        self.evict_unused(resources, renderer, encoder, 0);
    }
}

pub(crate) struct HybridImageUploadSession<'a> {
    registry: &'a mut HybridImageRegistry,
    renderer: &'a mut vello_gpu::Renderer,
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    encoder: Option<wgpu::CommandEncoder>,
    pending: Vec<Arc<RegisteredImage>>,
}

impl HybridImageUploadSession<'_> {
    pub(crate) fn resolve_image_brush(
        &mut self,
        resources: &mut vello_gpu::Resources,
        brush: &ImageBrush,
    ) -> Result<(VelloImage, Arc<RegisteredImage>), Error> {
        let key = ImageKey::derive(&brush.image);
        let image = if let Some(image) = self.pending.iter().find(|ri| ri.key == key).cloned() {
            image
        } else if let Some(index) = self.registry.live.iter().position(|ri| ri.key == key) {
            let index = self.registry.touch(index);
            Arc::clone(self.registry.live.get(index).unwrap())
        } else {
            let image_source = ImageSource::from_peniko_image_data(&brush.image);
            let ImageSource::Pixmap(pixmap) = image_source else {
                return Err(Error::Internal(
                    "peniko image conversion did not produce a pixmap",
                ));
            };
            let id = self.renderer.upload_image(
                resources,
                self.device,
                self.queue,
                self.encoder
                    .as_mut()
                    .expect("hybrid image upload session should own an encoder"),
                &pixmap,
            );
            let image = Arc::new(RegisteredImage {
                key,
                id,
                may_have_transparency: pixmap.may_have_transparency(),
                bytes: brush
                    .image
                    .format
                    .size_in_bytes(brush.image.width, brush.image.height)
                    .unwrap_or_else(|| brush.image.data.data().len()),
            });
            self.pending.push(Arc::clone(&image));
            image
        };

        Ok((
            VelloImage {
                image: ImageSource::opaque_id_with_transparency_hint(
                    image.id,
                    image.may_have_transparency,
                ),
                sampler: brush.sampler,
            },
            image,
        ))
    }

    pub(crate) fn finish(&mut self, resources: &mut vello_gpu::Resources, success: bool) {
        if success {
            for image in self.pending.drain(..) {
                self.registry.bytes_used = self.registry.bytes_used.saturating_add(image.bytes);
                self.registry.live.push_back(image);
            }
        } else {
            for image in self.pending.drain(..) {
                self.renderer.destroy_image(
                    resources,
                    self.encoder
                        .as_mut()
                        .expect("hybrid image upload session should own an encoder"),
                    image.id,
                );
            }
        }

        if let Some(encoder) = self.encoder.take() {
            self.queue.submit([encoder.finish()]);
        }
    }
}

#[derive(Debug)]
pub(crate) struct RegisteredImage {
    key: ImageKey,
    id: ImageId,
    may_have_transparency: bool,
    bytes: usize,
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
    use crate::image_registry::HybridImageRegistry;

    use super::{ImageKey, RegisteredImage};
    use peniko::{Blob, ImageAlphaType, ImageData, ImageFormat};
    use std::collections::VecDeque;
    use std::sync::Arc;
    use vello_common::paint::ImageId;

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

    #[test]
    fn image_touch() {
        let a = image([1, 2, 3, 4, 9, 8, 7, 6, 5, 4, 3, 2, 10, 11, 12, 13]);
        let b = image([13, 12, 11, 10, 2, 3, 4, 5, 6, 7, 8, 9, 4, 3, 2, 1]);

        let bytes_used = a.data.len() + b.data.len();

        let a_key = ImageKey::derive(&a);
        let b_key = ImageKey::derive(&b);

        let a_ri = RegisteredImage {
            key: a_key,
            id: ImageId::new(0),
            may_have_transparency: true,
            bytes: a.data.len(),
        };
        let b_ri = RegisteredImage {
            key: b_key,
            id: ImageId::new(1),
            may_have_transparency: true,
            bytes: b.data.len(),
        };

        let mut live = VecDeque::new();
        live.push_back(Arc::new(a_ri));
        live.push_back(Arc::new(b_ri));

        let mut registry = HybridImageRegistry {
            live,
            max_bytes: 1000 * 1000 * 1000,
            bytes_used,
        };

        // Touching the last entry is a no-op
        assert_eq!(registry.touch(1), 1);
        assert_eq!(registry.live.get(1).unwrap().id, ImageId::new(1));

        // Touching the first entry moves it to the end
        assert_eq!(registry.touch(0), 1);
        assert_eq!(registry.live.get(1).unwrap().id, ImageId::new(0));
    }
}
