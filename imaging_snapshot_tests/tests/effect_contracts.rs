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
    let mut isolated = Scene::new();
    let mut p = Painter::new(&mut isolated);
    p.fill_rect(SURFACE, Color::from_rgb8(0, 0, 255));
    p.with_group(GroupRef::new(), |p| {
        p.fill(SURFACE, Color::from_rgb8(255, 0, 0))
            .composite(Composite::new(peniko::Mix::Multiply.into(), 1.0))
            .draw();
    });
    let image = renderer.render_source(&mut &isolated, 16, 8).unwrap();
    assert!(
        image
            .data
            .chunks_exact(4)
            .all(|pixel| pixel == [255, 0, 0, 255]),
        "default groups must isolate child blending from the parent backdrop: {:?}",
        &image.data[..4]
    );

    let mut empty = Scene::new();
    let mut p = Painter::new(&mut empty);
    p.with_group(GroupRef::new(), |p| {
        p.with_group(
            GroupRef::new().with_clip(imaging::ClipRef::fill(Rect::ZERO)),
            |p| {
                p.fill_rect(SURFACE, Color::BLACK);
                p.with_group(GroupRef::new(), |p| p.fill_rect(SURFACE, Color::BLACK));
            },
        );
        p.fill_rect(Rect::new(0.0, 0.0, 4.0, 8.0), Color::WHITE);
    });
    let image = renderer.render_source(&mut &empty, 16, 8).unwrap();
    assert_eq!(
        image.data[(4 * 16 + 2) * 4 + 3],
        255,
        "empty nested groups must preserve the parent layer"
    );
    assert_eq!(
        image.data[(4 * 16 + 12) * 4 + 3],
        0,
        "empty group clips must discard their content"
    );

    // A cropped glyph mask must preserve the image brush's run and brush transforms.
    use skrifa::MetadataProvider as _;
    let bytes = include_bytes!("../../test_assets/fonts/NotoSans-Regular.ttf");
    let glyph = skrifa::FontRef::new(bytes)
        .unwrap()
        .charmap()
        .map('M')
        .unwrap();
    let font = peniko::FontData::new(Blob::new(Arc::new(bytes)), 0);
    let image = imaging::ImageBrush::new(ImageData {
        data: Blob::new(Arc::new([255_u8, 0, 0, 255, 0, 0, 255, 255])),
        width: 2,
        height: 1,
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
    })
    .with_sampler(peniko::ImageSampler::default().with_quality(peniko::ImageQuality::Low));
    let mut text = Scene::new();
    Painter::new(&mut text).with_group(
        GroupRef::new().with_clip(imaging::ClipRef::fill(Rect::new(2.0, 1.0, 14.0, 8.0))),
        |p| {
            p.glyphs(&font, &image)
                .font_size(8.0)
                .transform(Affine::translate((3.0, 7.0)))
                .brush_transform(Some(Affine::scale(4.0)))
                .draw(
                    &peniko::Style::Fill(peniko::Fill::NonZero),
                    [imaging::record::Glyph {
                        id: glyph.to_u32(),
                        x: 0.0,
                        y: 0.0,
                    }],
                );
        },
    );
    let image = renderer.render_source(&mut &text, 16, 8).unwrap();
    let mut painted = 0;
    for (index, pixel) in image.data.chunks_exact(4).enumerate() {
        if pixel[3] < 32 {
            continue;
        }
        painted += 1;
        let x = index % 16;
        let expected = if x < 7 { [255, 0, 0] } else { [0, 0, 255] };
        assert!(
            pixel[..3]
                .iter()
                .zip(expected)
                .all(|(a, b)| a.abs_diff(b) <= 2),
            "glyph brush at x={x}: {pixel:?}, expected {expected:?}"
        );
    }
    assert!(painted > 0, "glyphs inside an offset group must render");

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

    let mask = Painter::<Scene>::record_mask(MaskMode::Alpha, |p| {
        p.fill_rect(Rect::new(4.0, 4.0, 12.0, 6.0), Color::WHITE);
    });
    let mut nested = Scene::new();
    Painter::new(&mut nested).with_group(
        GroupRef::new().with_clip(imaging::ClipRef::fill(Rect::new(2.0, 2.0, 14.0, 8.0))),
        |p| {
            p.with_group(GroupRef::new().with_mask(mask.as_ref()), |p| {
                p.fill_rect(SURFACE, Color::WHITE);
            });
        },
    );
    let image = renderer.render_source(&mut &nested, 16, 8).unwrap();
    assert_eq!(
        image.data[(5 * 16 + 6) * 4 + 3],
        255,
        "localized masks stay aligned inside an offset parent layer"
    );
    assert_eq!(
        image.data[(5 * 16 + 13) * 4 + 3],
        0,
        "localized mask bounds stay in scene coordinates"
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

#[cfg(any(
    feature = "tiny_skia",
    feature = "vello_cpu",
    feature = "skia",
    feature = "vello_hybrid"
))]
fn check_filter_contract(renderer: &mut dyn ImageRenderer) {
    check_filter_coordinates(renderer);
    // Finite core geometry can exceed a backend's numeric or kernel limits.
    for filter in [
        imaging::Filter::blur(f64::from(f32::MAX) * 2.0),
        imaging::Filter::offset(f64::from(f32::MAX) * 2.0, 0.0),
    ] {
        let mut unsupported = Scene::new();
        Painter::new(&mut unsupported).with_group(GroupRef::new().with_filters(&[filter]), |p| {
            p.fill_rect(SURFACE, Color::WHITE);
        });
        unsupported.validate().unwrap();
        assert!(
            matches!(
                renderer.render_source(&mut &unsupported, 16, 8),
                Err(imaging::ImageRendererError::Unsupported(
                    imaging::render::RenderUnsupportedError::Filter
                ))
            ),
            "backend limits must be reported separately from invalid source parameters"
        );
    }
    let mut invalid = Scene::new();
    Painter::new(&mut invalid).with_group(
        GroupRef::new()
            .with_filters(&[imaging::Filter::blur(f64::MAX)])
            .with_filter_transform(Affine::scale(2.0)),
        |p| p.fill_rect(SURFACE, Color::WHITE),
    );
    assert!(
        matches!(
            renderer.render_source(&mut &invalid, 16, 8),
            Err(imaging::ImageRendererError::Content(
                imaging::render::RenderContentError::InvalidScene(_)
            ))
        ),
        "invalid transformed parameters must fail source validation"
    );

    let mask = Painter::<Scene>::record_mask(MaskMode::Alpha, |p| {
        p.fill_rect(Rect::new(0.0, 0.0, 4.0, 8.0), Color::WHITE);
    });
    let mut scene = Scene::new();
    Painter::new(&mut scene).with_group(
        GroupRef::new()
            .with_mask(mask.as_ref())
            .with_filters(&[imaging::Filter::offset(4.0, 0.0)]),
        |p| p.fill_rect(Rect::new(0.0, 0.0, 4.0, 8.0), Color::WHITE),
    );
    let image = renderer.render_source(&mut &scene, 16, 8).unwrap();
    assert!(
        image.data.chunks_exact(4).all(|pixel| pixel[3] == 0),
        "filters must run before masks: the offset moves all content outside the mask"
    );

    let mut scene = Scene::new();
    Painter::new(&mut scene).with_group(
        GroupRef::new()
            .with_filters(&[
                imaging::Filter::offset(2.0, 0.0),
                imaging::Filter::offset(2.0, 0.0),
            ])
            .with_composite(Composite::new(Compose::SrcOver.into(), 0.5)),
        |p| p.fill_rect(Rect::new(0.0, 0.0, 4.0, 8.0), Color::WHITE),
    );
    let image = renderer.render_source(&mut &scene, 16, 8).unwrap();
    for y in 0..8 {
        for x in 0..16 {
            let expected = if (4..8).contains(&x) { 128 } else { 0 };
            let alpha = image.data[(y * 16 + x) * 4 + 3];
            assert!(
                alpha.abs_diff(expected) <= 1,
                "filter chain at ({x}, {y}): {alpha}, expected {expected}; group alpha applies once after both offsets"
            );
        }
    }
    // The isolated clip constrains the filtered result, not the filter's input.
    let mut scene = Scene::new();
    Painter::new(&mut scene).with_group(
        GroupRef::new()
            .with_clip(imaging::ClipRef::fill(Rect::new(4.0, 0.0, 8.0, 8.0)))
            .with_filters(&[imaging::Filter::offset(4.0, 0.0)]),
        |p| p.fill_rect(Rect::new(0.0, 0.0, 4.0, 8.0), Color::WHITE),
    );
    let image = renderer.render_source(&mut &scene, 16, 8).unwrap();
    assert_eq!(
        image.data[(4 * 16 + 6) * 4 + 3],
        255,
        "source outside the isolated clip can contribute to its filtered result"
    );
    assert_eq!(
        image.data[(4 * 16 + 2) * 4 + 3],
        0,
        "the isolated clip constrains the result"
    );

    // Flood replaces the source even when there are no draws to provide content bounds.
    let mut scene = Scene::new();
    Painter::new(&mut scene).with_group(
        GroupRef::new().with_filters(&[imaging::Filter::flood(Color::from_rgba8(255, 0, 0, 128))]),
        |_| {},
    );
    let image = renderer.render_source(&mut &scene, 16, 8).unwrap();
    for pixel in image.data.chunks_exact(4) {
        assert_eq!(
            pixel,
            [255, 0, 0, 128],
            "flood must not depend on source draw bounds"
        );
    }

    // These filters do not commute: the second flood must replace the first.
    let mut scene = Scene::new();
    Painter::new(&mut scene).with_group(
        GroupRef::new().with_filters(&[
            imaging::Filter::flood(Color::from_rgb8(255, 0, 0)),
            imaging::Filter::flood(Color::from_rgba8(0, 0, 255, 128)),
        ]),
        |_| {},
    );
    let image = renderer.render_source(&mut &scene, 16, 8).unwrap();
    for pixel in image.data.chunks_exact(4) {
        assert_eq!(pixel, [0, 0, 255, 128], "filters must run in slice order");
    }
}

#[cfg(any(
    feature = "tiny_skia",
    feature = "vello_cpu",
    feature = "skia",
    feature = "vello_hybrid"
))]
fn check_filter_coordinates(renderer: &mut dyn ImageRenderer) {
    // Prefixing a recorded scene must transform the effect along with its source and mask.
    for masked in [false, true] {
        let mask = Painter::<Scene>::record_mask(MaskMode::Alpha, |p| {
            p.fill_rect(Rect::new(2.0, 0.0, 4.0, 4.0), Color::WHITE);
        });
        let mut source = Scene::new();
        let filters = [imaging::Filter::offset(1.0, 0.0)];
        let mut group = GroupRef::new()
            .with_filters(&filters)
            .with_filter_transform(Affine::translate((50.0, 50.0)) * Affine::scale(2.0));
        if masked {
            group = group.with_mask(mask.as_ref());
        }
        Painter::new(&mut source).with_group(group, |p| {
            p.fill_rect(Rect::new(0.0, 0.0, 2.0, 4.0), Color::WHITE);
        });
        let mut scene = Scene::new();
        imaging::record::replay_transformed(&source, &mut scene, Affine::scale(2.0));
        let image = renderer.render_source(&mut &scene, 16, 8).unwrap();
        for (index, pixel) in image.data.chunks_exact(4).enumerate() {
            let expected = if (4..8).contains(&(index % 16)) {
                255
            } else {
                0
            };
            assert_eq!(
                pixel[3], expected,
                "transformed filter at pixel {index}, masked={masked}"
            );
        }
    }

    // A singular filter transform collapses the offset, without hiding the source.
    let mut scene = Scene::new();
    Painter::new(&mut scene).with_group(
        GroupRef::new()
            .with_filters(&[imaging::Filter::offset(4.0, 0.0)])
            .with_filter_transform(Affine::scale(0.0)),
        |p| p.fill_rect(Rect::new(0.0, 0.0, 2.0, 8.0), Color::WHITE),
    );
    let image = renderer.render_source(&mut &scene, 16, 8).unwrap();
    assert_eq!(
        image.data[(4 * 16 + 1) * 4 + 3],
        255,
        "collapsed filter coordinates preserve the source"
    );
    assert_eq!(
        image.data[(4 * 16 + 5) * 4 + 3],
        0,
        "a collapsed offset must not move the source"
    );

    // Scaling an effect's coordinates must agree with specifying its device-space parameters.
    for shadow in [false, true] {
        let effect = |sigma, dx| {
            if shadow {
                imaging::Filter::DropShadow {
                    dx,
                    dy: 0.0,
                    std_deviation_x: sigma,
                    std_deviation_y: sigma,
                    color: Color::from_rgba8(255, 0, 0, 128),
                }
            } else {
                imaging::Filter::blur(sigma)
            }
        };
        let mut images = Vec::new();
        for (sigma, dx, transform, replay) in [
            (1.0, 1.0, Affine::scale(2.0), false),
            (2.0, 2.0, Affine::IDENTITY, false),
            (1.0, 1.0, Affine::IDENTITY, true),
        ] {
            let mut scene = Scene::new();
            let rect = if replay {
                Rect::new(3.0, 1.0, 5.0, 3.0)
            } else {
                Rect::new(6.0, 2.0, 10.0, 6.0)
            };
            Painter::new(&mut scene).with_group(
                GroupRef::new()
                    .with_filters(&[effect(sigma, dx)])
                    .with_filter_transform(transform),
                |p| p.fill_rect(rect, Color::WHITE),
            );
            if replay {
                let mut transformed = Scene::new();
                imaging::record::replay_transformed(&scene, &mut transformed, Affine::scale(2.0));
                scene = transformed;
            }
            images.push(renderer.render_source(&mut &scene, 16, 8).unwrap());
        }
        for image in &images[1..] {
            for (a, b) in images[0].data.iter().zip(&image.data) {
                assert!(
                    a.abs_diff(*b) <= 1,
                    "scaled filter differs: {a} vs {b}, shadow={shadow}"
                );
            }
        }
    }
}

#[test]
#[cfg(feature = "skia")]
fn skia_rotated_anisotropic_blur() {
    let mut renderer = imaging_skia::SkiaCpuRenderer::new();
    for masked in [false, true] {
        let mask = Painter::<Scene>::record_mask(MaskMode::Alpha, |p| {
            p.fill_rect(Rect::new(0.0, 0.0, 32.0, 32.0), Color::WHITE);
        });
        let filters = [imaging::Filter::blur_xy(4.0, 0.5)];
        let mut group = GroupRef::new()
            .with_filters(&filters)
            .with_filter_transform(Affine::rotate(core::f64::consts::FRAC_PI_4));
        if masked {
            group = group.with_mask(mask.as_ref());
        }
        let mut scene = Scene::new();
        Painter::new(&mut scene).with_group(group, |p| {
            p.fill_rect(Rect::new(15.0, 15.0, 17.0, 17.0), Color::WHITE);
        });
        let image = renderer.render_source(&mut &scene, 32, 32).unwrap();
        let along = image.data[(19 * 32 + 19) * 4 + 3];
        let across = image.data[(12 * 32 + 19) * 4 + 3];
        assert!(
            along > across + 5,
            "anisotropic blur must rotate its kernel: {along} vs {across}, masked={masked}"
        );
    }
}

#[test]
#[cfg(feature = "tiny_skia")]
fn tiny_skia_alpha_masks() {
    let mut renderer = imaging_tiny_skia::TinySkiaRenderer::new();
    check_alpha_mask_contract(&mut renderer);
    check_filter_contract(&mut renderer);
}

#[test]
#[cfg(feature = "vello_cpu")]
fn vello_cpu_alpha_masks() {
    let mut renderer = imaging_vello_cpu::VelloCpuRenderer::new(16, 8);
    check_alpha_mask_contract(&mut renderer);
    check_filter_contract(&mut renderer);
}

#[test]
#[cfg(feature = "skia")]
fn skia_alpha_masks() {
    let mut renderer = imaging_skia::SkiaCpuRenderer::new();
    check_alpha_mask_contract(&mut renderer);
    check_filter_contract(&mut renderer);
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
    let mut renderer = imaging_vello_hybrid::VelloHybridRenderer::new(device, queue);
    check_alpha_mask_contract(&mut renderer);
    check_filter_contract(&mut renderer);
}
