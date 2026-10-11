// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Rendered contracts for isolated groups, masks, and filters.

#![cfg(any(
    feature = "tiny_skia",
    feature = "vello_cpu",
    feature = "skia",
    feature = "vello",
    feature = "vello_hybrid"
))]

use imaging::{Composite, GroupRef, ImageRenderer, MaskMode, MaskRef, Painter, record::Scene};
use kurbo::{Affine, Rect};
use peniko::{Blob, Color, Compose, ImageAlphaType, ImageData, ImageFormat};
use std::sync::Arc;

const SURFACE: Rect = Rect::new(0.0, 0.0, 16.0, 8.0);

fn check_alpha_mask_contract(renderer: &mut dyn ImageRenderer) {
    let mut mask = Scene::new();
    let mut painter = Painter::new(&mut mask);
    painter.fill_rect(
        Rect::new(0.0, 0.0, 8.0, 8.0),
        Color::from_rgba8(0, 0, 255, 128),
    );
    painter.fill_rect(
        Rect::new(4.0, 0.0, 12.0, 8.0),
        Color::from_rgba8(255, 0, 0, 128),
    );
    // The alpha of the isolated mask must include destructive composition.
    painter
        .fill(Rect::new(6.0, 0.0, 8.0, 8.0), Color::WHITE)
        .composite(Composite::new(Compose::DestOut.into(), 1.0))
        .draw();

    let mut scene = Scene::new();
    Painter::new(&mut scene).with_group(
        GroupRef::new()
            .with_mask_transformed(
                MaskRef::new(MaskMode::Alpha, &mask),
                Affine::translate((2.0, 0.0)),
            )
            .with_composite(Composite::new(Compose::SrcOver.into(), 0.5)),
        |content| content.fill_rect(SURFACE, Color::WHITE),
    );
    let image = renderer.render_source(&mut &scene, 16, 8).unwrap();
    for y in 0..8 {
        for x in 0..16 {
            let expected: u8 = match x {
                2..=5 | 10..=13 => 64,
                6..=7 => 96,
                _ => 0,
            };
            let pixel = &image.data[(y * 16 + x) * 4..][..4];
            assert!(
                pixel[3].abs_diff(expected) <= 1,
                "mask alpha at ({x}, {y}): {pixel:?}, expected {expected}"
            );
            if expected != 0 {
                assert_eq!(
                    &pixel[..3],
                    &[255; 3],
                    "mask RGB must not color the content"
                );
            }
        }
    }

    // A premultiplied BGRA image inside a nested mask uses the same image path as normal paint.
    let image = ImageData {
        data: Blob::new(Arc::new([128_u8, 0, 0, 128].repeat(4))),
        width: 2,
        height: 2,
        format: ImageFormat::Bgra8,
        alpha_type: ImageAlphaType::AlphaPremultiplied,
    };
    let inner = Painter::<Scene>::record_mask(MaskMode::Alpha, |p| {
        p.draw_image(&image, Affine::scale(4.0));
    });
    let outer = Painter::<Scene>::record_mask(MaskMode::Alpha, |p| {
        p.with_group(GroupRef::new().with_mask(inner.as_ref()), |p| {
            p.fill_rect(SURFACE, Color::BLACK);
        });
    });
    let mut scene = Scene::new();
    Painter::new(&mut scene).with_group(GroupRef::new().with_mask(outer.as_ref()), |p| {
        p.fill_rect(SURFACE, Color::WHITE);
    });
    let image = renderer.render_source(&mut &scene, 16, 8).unwrap();
    assert!(
        image.data[(4 * 16 + 4) * 4 + 3].abs_diff(128) <= 1,
        "nested image mask preserves alpha"
    );
    assert_eq!(
        image.data[(4 * 16 + 12) * 4 + 3],
        0,
        "outside nested mask is transparent"
    );

    let empty = Scene::new();
    let mut scene = Scene::new();
    Painter::new(&mut scene).with_group(
        GroupRef::new().with_mask(MaskRef::new(MaskMode::Alpha, &empty)),
        |p| p.fill_rect(SURFACE, Color::WHITE),
    );
    let image = renderer.render_source(&mut &scene, 16, 8).unwrap();
    assert!(
        image.data.chunks_exact(4).all(|pixel| pixel[3] == 0),
        "an empty mask must clear the entire group"
    );
}

#[test]
#[cfg(feature = "tiny_skia")]
fn tiny_skia_alpha_masks() {
    check_alpha_mask_contract(&mut imaging_tiny_skia::TinySkiaRenderer::new());
}

#[test]
#[cfg(feature = "vello_cpu")]
fn vello_cpu_alpha_masks() {
    check_alpha_mask_contract(&mut imaging_vello_cpu::VelloCpuRenderer::new(16, 8));
}

#[test]
#[cfg(feature = "skia")]
fn skia_alpha_masks() {
    check_alpha_mask_contract(&mut imaging_skia::SkiaCpuRenderer::new());
}

#[cfg(any(feature = "vello", feature = "vello_hybrid"))]
fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
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

#[cfg(feature = "vello")]
use imaging_vello::wgpu;
#[cfg(all(not(feature = "vello"), feature = "vello_hybrid"))]
use imaging_vello_hybrid::wgpu;

#[test]
#[cfg(feature = "vello")]
fn vello_alpha_masks() {
    let Some((device, queue)) = device() else {
        return;
    };
    check_alpha_mask_contract(&mut imaging_vello::VelloRenderer::new(device, queue).unwrap());
}

#[test]
#[cfg(feature = "vello_hybrid")]
fn hybrid_alpha_masks() {
    let Some((device, queue)) = device() else {
        return;
    };
    check_alpha_mask_contract(&mut imaging_vello_hybrid::VelloHybridRenderer::new(
        device, queue,
    ));
}
