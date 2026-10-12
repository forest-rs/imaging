// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Rendered contracts for synthetic glyph emboldening.

#![cfg(any(
    feature = "tiny_skia",
    feature = "vello_cpu",
    feature = "skia",
    feature = "vello",
    feature = "vello_hybrid"
))]

use imaging::{ImageRenderer, Painter, RgbaImage, record::Scene};
use kurbo::{Affine, Vec2};
use peniko::{Blob, Color, Fill, FontData, Style};
use skrifa::MetadataProvider as _;
use std::sync::{Arc, OnceLock};

fn font() -> &'static FontData {
    static FONT: OnceLock<FontData> = OnceLock::new();
    FONT.get_or_init(|| {
        FontData::new(
            Blob::new(Arc::new(include_bytes!(
                "../../test_assets/fonts/NotoSans-Regular.ttf"
            ))),
            0,
        )
    })
}

fn render(
    renderer: &mut dyn ImageRenderer,
    amount: Vec2,
    transform: Affine,
    glyph_transform: Option<Affine>,
    hint: bool,
) -> RgbaImage {
    let bytes = include_bytes!("../../test_assets/fonts/NotoSans-Regular.ttf");
    let id = skrifa::FontRef::new(bytes)
        .unwrap()
        .charmap()
        .map('H')
        .unwrap()
        .to_u32();
    let font = font();
    let mut scene = Scene::new();
    Painter::new(&mut scene)
        .glyphs(font, Color::WHITE)
        .font_size(32.0)
        .hint(hint)
        .font_embolden(amount)
        .transform(transform)
        .glyph_transform(glyph_transform)
        .draw(
            &Style::Fill(Fill::NonZero),
            [imaging::record::Glyph { id, x: 0.0, y: 0.0 }],
        );
    renderer.render_source(&mut &scene, 128, 128).unwrap()
}

fn bounds(image: &RgbaImage) -> [i32; 4] {
    let mut bounds = [128, 128, -1, -1];
    for (index, pixel) in image.data.chunks_exact(4).enumerate() {
        if pixel[3] < 128 {
            continue;
        }
        let x = i32::try_from(index % 128).unwrap();
        let y = i32::try_from(index / 128).unwrap();
        bounds[0] = bounds[0].min(x);
        bounds[1] = bounds[1].min(y);
        bounds[2] = bounds[2].max(x);
        bounds[3] = bounds[3].max(y);
    }
    assert!(bounds[2] >= bounds[0], "glyph must render");
    bounds
}

fn expansion(
    renderer: &mut dyn ImageRenderer,
    transform: Affine,
    glyph_transform: Option<Affine>,
    amount: Vec2,
    expected: [i32; 2],
    hint: bool,
) {
    let base = bounds(&render(
        renderer,
        Vec2::ZERO,
        transform,
        glyph_transform,
        hint,
    ));
    let bold = bounds(&render(renderer, amount, transform, glyph_transform, hint));
    let actual = [
        base[0] - bold[0],
        base[1] - bold[1],
        bold[2] - base[2],
        bold[3] - base[3],
    ];
    for (actual, expected) in
        actual
            .into_iter()
            .zip([expected[0], expected[1], expected[0], expected[1]])
    {
        assert!(
            (actual - expected).abs() <= 1,
            "expansion: base={base:?}, bold={bold:?}, expected={expected}"
        );
    }
    // Reusing a renderer must not confuse plain and expanded outline caches.
    assert_eq!(
        base,
        bounds(&render(
            renderer,
            Vec2::ZERO,
            transform,
            glyph_transform,
            hint
        )),
        "plain outline cache entries must survive emboldened draws"
    );
}

fn independent_axes(renderer: &mut dyn ImageRenderer, hint: bool) {
    let transform = Affine::translate((24.0, 70.0));
    expansion(renderer, transform, None, Vec2::new(3.0, 0.0), [3, 0], hint);
    expansion(renderer, transform, None, Vec2::new(0.0, 3.0), [0, 3], hint);
    expansion(renderer, transform, None, Vec2::new(3.0, 2.0), [3, 2], hint);
}

fn transformed_expansion(renderer: &mut dyn ImageRenderer) {
    for hint in [false, true] {
        expansion(
            renderer,
            Affine::translate((24.0, 90.0)) * Affine::scale(2.0),
            None,
            Vec2::new(3.0, 2.0),
            [6, 4],
            hint,
        );
        expansion(
            renderer,
            Affine::translate((24.0, 90.0)),
            Some(Affine::scale_non_uniform(2.0, 1.5)),
            Vec2::new(3.0, 2.0),
            [6, 3],
            hint,
        );
        expansion(
            renderer,
            Affine::translate((24.0, 30.0)) * Affine::rotate(core::f64::consts::FRAC_PI_2),
            None,
            Vec2::new(3.0, 2.0),
            [2, 3],
            hint,
        );
    }
}

fn brush_and_opacity(renderer: &mut dyn ImageRenderer) {
    let bytes = include_bytes!("../../test_assets/fonts/NotoSans-Regular.ttf");
    let id = skrifa::FontRef::new(bytes)
        .unwrap()
        .charmap()
        .map('H')
        .unwrap()
        .to_u32();
    let font = font();
    let image = imaging::ImageBrush::new(peniko::ImageData {
        data: Blob::new(Arc::new([255_u8, 0, 0, 255, 0, 0, 255, 255])),
        width: 2,
        height: 1,
        format: peniko::ImageFormat::Rgba8,
        alpha_type: peniko::ImageAlphaType::Alpha,
    })
    .with_sampler(peniko::ImageSampler::default().with_quality(peniko::ImageQuality::Low));
    let mut scene = Scene::new();
    Painter::new(&mut scene).with_group(
        imaging::GroupRef::new().with_clip(imaging::ClipRef::fill(kurbo::Rect::new(
            20.0, 20.0, 80.0, 85.0,
        ))),
        |p| {
            p.glyphs(font, &image)
                .font_size(32.0)
                .hint(true)
                .font_embolden(Vec2::new(3.0, 2.0))
                .transform(Affine::translate((24.0, 70.0)))
                .brush_transform(Some(Affine::scale(16.0)))
                .composite(imaging::Composite::new(peniko::BlendMode::default(), 0.5))
                .draw(
                    &Style::Fill(Fill::NonZero),
                    [imaging::record::Glyph {
                        id,
                        x: 10.0,
                        y: 0.0,
                    }],
                );
        },
    );
    let image = renderer.render_source(&mut &scene, 128, 128).unwrap();
    let mut colors = [false; 2];
    for (index, pixel) in image.data.chunks_exact(4).enumerate() {
        assert!(pixel[3] <= 129, "opacity must be applied once: {pixel:?}");
        if pixel[3] < 120 {
            continue;
        }
        let x = index % 128;
        let expected = if x < 40 { [255, 0, 0] } else { [0, 0, 255] };
        assert!(
            pixel[..3]
                .iter()
                .zip(expected)
                .all(|(a, b)| a.abs_diff(b) <= 2),
            "run-space brush at x={x}: {pixel:?}, expected {expected:?}"
        );
        colors[usize::from(x >= 40)] = true;
    }
    assert!(
        colors.into_iter().all(|seen| seen),
        "both image brush colors must render"
    );
}

macro_rules! backend_tests {
    ($backend:ident, $renderer:expr $(, $limitation:literal)?) => {
        mod $backend {
            use super::*;
            fn check(contract: fn(&mut dyn ImageRenderer)) {
                let renderer: Option<Box<dyn ImageRenderer>> = $renderer;
                if let Some(mut renderer) = renderer {
                    contract(renderer.as_mut());
                }
            }
            #[test]
            fn brush_and_opacity() { check(super::brush_and_opacity); }
            #[test]
            fn hinted_axes() {
                check(|renderer| super::independent_axes(renderer, true));
            }
            #[test]
            $(#[ignore = $limitation])?
            fn unhinted_axes() {
                check(|renderer| super::independent_axes(renderer, false));
            }
            #[test]
            $(#[ignore = $limitation])?
            fn transformed_expansion() {
                check(super::transformed_expansion);
            }
        }
    };
}

#[cfg(feature = "tiny_skia")]
backend_tests!(
    tiny_skia,
    Some(Box::new(imaging_tiny_skia::TinySkiaRenderer::new()))
);
#[cfg(feature = "vello_cpu")]
backend_tests!(
    vello_cpu,
    Some(Box::new(imaging_vello_cpu::VelloCpuRenderer::new(128, 128))),
    "glifo 0.4 applies embolden in cache units without correcting font or absorbed transform scales"
);
#[cfg(feature = "skia")]
backend_tests!(skia, Some(Box::new(imaging_skia::SkiaCpuRenderer::new())));

#[cfg(feature = "vello")]
use imaging_vello::wgpu;
#[cfg(all(not(feature = "vello"), feature = "vello_hybrid"))]
use imaging_vello_hybrid::wgpu;

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
backend_tests!(
    vello,
    device().map(|(device, queue)| {
        Box::new(imaging_vello::VelloRenderer::new(device, queue).unwrap())
            as Box<dyn ImageRenderer>
    })
);
#[cfg(feature = "vello_hybrid")]
backend_tests!(
    vello_hybrid,
    device().map(|(device, queue)| {
        Box::new(imaging_vello_hybrid::VelloHybridRenderer::new(
            device, queue,
        )) as Box<dyn ImageRenderer>
    }),
    "glifo 0.4 applies embolden in cache units without correcting font or absorbed transform scales"
);
