// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Pixel contracts shared by raster image consumers.

#![cfg(any(
    feature = "tiny_skia",
    feature = "vello_cpu",
    feature = "skia",
    feature = "vello",
    feature = "vello_hybrid"
))]

use imaging::{Painter, RgbaImage, record::Scene};
use kurbo::Affine;
use peniko::{Blob, ImageAlphaType, ImageData, ImageFormat};
use std::sync::Arc;

fn image_scene(data: ImageData) -> Scene {
    let mut scene = Scene::new();
    Painter::new(&mut scene).draw_image(&data, Affine::IDENTITY);
    scene
}

fn assert_pixels(image: RgbaImage, expected: [u8; 4]) {
    for pixel in image.data.chunks_exact(4) {
        assert_eq!(
            pixel, expected,
            "image pixels must honor channel order and alpha metadata"
        );
    }
}

fn check_raster_contract(mut render: impl FnMut(&Scene) -> RgbaImage) {
    for (format, alpha_type, pixel) in [
        (ImageFormat::Rgba8, ImageAlphaType::Alpha, [255, 0, 0, 128]),
        (ImageFormat::Bgra8, ImageAlphaType::Alpha, [0, 0, 255, 128]),
        (
            ImageFormat::Rgba8,
            ImageAlphaType::AlphaPremultiplied,
            [128, 0, 0, 128],
        ),
        (
            ImageFormat::Bgra8,
            ImageAlphaType::AlphaPremultiplied,
            [0, 0, 128, 128],
        ),
    ] {
        let scene = image_scene(ImageData {
            data: Blob::new(Arc::new(pixel.repeat(4))),
            format,
            alpha_type,
            width: 2,
            height: 2,
        });
        assert_pixels(render(&scene), [255, 0, 0, 128]);
    }
    // Metadata participates in cache identity even when the pixel blob is shared.
    let blob = Blob::new(Arc::new([255_u8, 0, 0, 255].repeat(4)));
    for (format, expected) in [
        (ImageFormat::Rgba8, [255, 0, 0, 255]),
        (ImageFormat::Bgra8, [0, 0, 255, 255]),
    ] {
        let scene = image_scene(ImageData {
            data: blob.clone(),
            format,
            alpha_type: ImageAlphaType::Alpha,
            width: 2,
            height: 2,
        });
        assert_pixels(render(&scene), expected);
    }
}

#[cfg(any(feature = "tiny_skia", feature = "vello_cpu", feature = "skia"))]
fn check_external_rejection(renderer: &mut dyn imaging::ImageRenderer) {
    use imaging::render::{ImageRendererError, RenderUnsupportedError};
    let image = imaging::ExternalImage::new(2, 2, ImageAlphaType::AlphaPremultiplied);
    let mut scene = Scene::new();
    Painter::new(&mut scene).draw_image(&image, Affine::IDENTITY);
    let mut source = &scene;
    assert!(
        matches!(
            renderer.render_source(&mut source, 2, 2),
            Err(ImageRendererError::Unsupported(
                RenderUnsupportedError::ExternalImage
            ))
        ),
        "external images must fail explicitly on raster backends"
    );
    let mut masked = Scene::new();
    Painter::new(&mut masked).with_masked_group(
        imaging::MaskMode::Alpha,
        |mask| mask.draw_image(&image, Affine::IDENTITY),
        |content| content.fill_rect(kurbo::Rect::new(0.0, 0.0, 2.0, 2.0), peniko::Color::WHITE),
    );
    assert!(
        masked.has_external_images(),
        "external images inside masks must be discoverable"
    );
    let mut source = &masked;
    assert!(
        matches!(
            renderer.render_source(&mut source, 2, 2),
            Err(ImageRendererError::Unsupported(
                RenderUnsupportedError::ExternalImage
            ))
        ),
        "mask images must fail explicitly on raster backends"
    );
}

#[test]
#[cfg(feature = "tiny_skia")]
fn tiny_skia_raster_images() {
    let mut renderer = imaging_tiny_skia::TinySkiaRenderer::new();
    check_raster_contract(|scene| renderer.render_scene(scene, 2, 2).unwrap());
    check_external_rejection(&mut renderer);
}

#[test]
#[cfg(feature = "vello_cpu")]
fn vello_cpu_raster_images() {
    let mut renderer = imaging_vello_cpu::VelloCpuRenderer::new(2, 2);
    check_raster_contract(|scene| renderer.render_scene(scene, 2, 2).unwrap());
    check_external_rejection(&mut renderer);
}

#[test]
#[cfg(feature = "skia")]
fn skia_raster_images() {
    let mut renderer = imaging_skia::SkiaCpuRenderer::new();
    check_raster_contract(|scene| renderer.render_scene(scene, 2, 2).unwrap());
    check_external_rejection(&mut renderer);
}

#[cfg(any(feature = "vello", feature = "vello_hybrid"))]
mod gpu {
    #[cfg(feature = "vello")]
    pub(super) use imaging_vello::wgpu;
    #[cfg(all(not(feature = "vello"), feature = "vello_hybrid"))]
    pub(super) use imaging_vello_hybrid::wgpu;

    pub(super) fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        if std::env::var("IMAGING_CI_GPU_SUPPORT").as_deref() == Ok("no") {
            return None;
        }
        let result = pollster::block_on(async {
            let instance = wgpu::Instance::default();
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .ok()?;
            adapter
                .request_device(&wgpu::DeviceDescriptor::default())
                .await
                .ok()
        });
        if std::env::var("IMAGING_CI_GPU_SUPPORT").as_deref() == Ok("yes") {
            assert!(result.is_some(), "GPU tests require a device");
        }
        result
    }
}

#[test]
#[cfg(feature = "vello")]
fn vello_raster_images() {
    let Some((device, queue)) = gpu::device() else {
        return;
    };
    let mut renderer = imaging_vello::VelloRenderer::new(device, queue).unwrap();
    check_raster_contract(|scene| {
        let encoded = renderer.encode_scene(scene, 2, 2).unwrap();
        renderer.render(&encoded, 2, 2).unwrap()
    });
}

#[test]
#[cfg(feature = "vello_hybrid")]
fn hybrid_raster_images() {
    let Some((device, queue)) = gpu::device() else {
        return;
    };
    let mut renderer = imaging_vello_hybrid::VelloHybridRenderer::new(device, queue);
    check_raster_contract(|scene| {
        let encoded = renderer.encode_scene(scene, 2, 2).unwrap();
        renderer.render(&encoded, 2, 2).unwrap()
    });
}
