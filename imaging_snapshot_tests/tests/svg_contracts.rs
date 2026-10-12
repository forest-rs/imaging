// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Pixel contracts for SVG clipping, masking, and group composition.

#![cfg(any(
    feature = "tiny_skia",
    feature = "vello_cpu",
    feature = "skia",
    feature = "vello",
    feature = "vello_hybrid"
))]

use imaging::{ImageRenderer, Painter, RgbaImage, record::Scene};
use kurbo::Affine;
use svg_imaging::{ParseOptions, RenderOptions, SvgDocument};

fn render(renderer: &mut dyn ImageRenderer, body: &str, transform: Affine) -> RgbaImage {
    let svg =
        format!("<svg xmlns='http://www.w3.org/2000/svg' width='32' height='16'>{body}</svg>");
    let document = SvgDocument::from_str(&svg, &ParseOptions::default()).unwrap();
    let mut scene = Scene::new();
    let report = document
        .render(&mut Painter::new(&mut scene), &RenderOptions { transform })
        .unwrap();
    assert!(report.is_fully_supported(), "{report:?}");
    scene.validate().unwrap();
    renderer.render_source(&mut &scene, 32, 16).unwrap()
}

fn assert_alpha(image: &RgbaImage, expected: impl Fn(usize, usize) -> u8) {
    for (index, pixel) in image.data.chunks_exact(4).enumerate() {
        let (x, y) = (index % 32, index / 32);
        let alpha = expected(x, y);
        assert!(
            pixel[3].abs_diff(alpha) <= 1,
            "alpha at ({x}, {y}): {pixel:?}, expected {alpha}"
        );
    }
}

fn transformed_clip(renderer: &mut dyn ImageRenderer) {
    let image = render(
        renderer,
        "<defs><clipPath id='c' transform='translate(1 1)'>
            <rect width='3' height='2'/>
         </clipPath></defs>
         <g transform='translate(3 1)' clip-path='url(#c)'>
            <rect width='12' height='8' fill='white'/>
         </g>",
        Affine::translate((4.0, 2.0)) * Affine::scale(2.0),
    );
    assert_alpha(&image, |x, y| {
        if (12..18).contains(&x) && (6..10).contains(&y) {
            255
        } else {
            0
        }
    });
}

fn shared_mask_transforms(renderer: &mut dyn ImageRenderer) {
    let image = render(
        renderer,
        "<defs><mask id='m' mask-type='alpha' maskUnits='userSpaceOnUse'
             x='0' y='0' width='6' height='6'>
            <rect width='10' height='10' fill='white'/>
         </mask></defs>
         <g transform='translate(4 2)' mask='url(#m)'>
            <rect width='10' height='10' fill='white'/>
         </g>
         <g transform='translate(20 2)' mask='url(#m)'>
            <rect width='10' height='10' fill='white'/>
         </g>",
        Affine::translate((1.0, 1.0)),
    );
    assert_alpha(&image, |x, y| {
        if ((5..11).contains(&x) || (21..27).contains(&x)) && (3..9).contains(&y) {
            255
        } else {
            0
        }
    });
}

fn object_bounding_box_effects(renderer: &mut dyn ImageRenderer) {
    let image = render(
        renderer,
        "<defs>
            <clipPath id='c' clipPathUnits='objectBoundingBox'>
                <rect x='0.25' width='0.5' height='1'/>
            </clipPath>
            <mask id='m' mask-type='alpha' maskContentUnits='objectBoundingBox'
                x='25%' y='25%' width='50%' height='50%'>
                <rect width='0.75' height='1' fill='white'/>
            </mask>
         </defs>
         <g transform='translate(4 2) scale(2)' mask='url(#m)' clip-path='url(#c)'>
            <rect x='2' y='1' width='8' height='4' fill='white'/>
         </g>",
        Affine::IDENTITY,
    );
    assert_alpha(&image, |x, y| {
        if (12..20).contains(&x) && (6..10).contains(&y) {
            255
        } else {
            0
        }
    });
}

fn nested_masks(renderer: &mut dyn ImageRenderer) {
    let image = render(
        renderer,
        "<defs>
            <mask id='inner' mask-type='alpha' maskUnits='userSpaceOnUse'
                x='0' y='0' width='16' height='16'>
                <rect x='4' y='4' width='8' height='8' fill='white'/>
            </mask>
            <mask id='outer' mask-type='alpha' maskUnits='userSpaceOnUse'
                x='0' y='0' width='16' height='16'>
                <g mask='url(#inner)'>
                    <rect x='2' y='2' width='6' height='12' fill='white'/>
                </g>
            </mask>
         </defs>
         <rect width='16' height='16' fill='white' mask='url(#outer)'/>",
        Affine::IDENTITY,
    );
    assert_alpha(&image, |x, y| {
        if (4..8).contains(&x) && (4..12).contains(&y) {
            255
        } else {
            0
        }
    });
}

fn referenced_masks(renderer: &mut dyn ImageRenderer) {
    let image = render(
        renderer,
        "<defs>
            <mask id='inner' mask-type='alpha' maskUnits='userSpaceOnUse'
                x='0' y='0' width='16' height='16'>
                <rect x='4' y='4' width='8' height='8' fill='white' opacity='0.5'/>
            </mask>
            <mask id='outer' mask='url(#inner)' mask-type='alpha'
                maskUnits='userSpaceOnUse' x='0' y='0' width='16' height='16'>
                <rect x='2' y='2' width='6' height='12' fill='white' opacity='0.5'/>
            </mask>
         </defs>
         <rect width='16' height='16' fill='white' mask='url(#outer)'/>",
        Affine::IDENTITY,
    );
    assert_alpha(&image, |x, y| {
        if (4..8).contains(&x) && (4..12).contains(&y) {
            64
        } else {
            0
        }
    });
}

fn nested_svg_masks(renderer: &mut dyn ImageRenderer) {
    use std::fmt::Write as _;
    let nested = "<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'>
        <defs><mask id='m' mask-type='alpha' maskUnits='userSpaceOnUse'
            x='0' y='0' width='8' height='8'>
            <rect x='2' width='4' height='8' fill='white'/>
        </mask></defs><rect width='8' height='8' fill='white' mask='url(#m)'/>
        </svg>";
    let mut encoded = String::new();
    for byte in nested.bytes() {
        write!(encoded, "%{byte:02X}").unwrap();
    }
    let body = format!(
        "<defs><mask id='m' mask-type='alpha' maskUnits='userSpaceOnUse'
            x='0' y='0' width='32' height='16'>
            <rect width='32' height='16' fill='white'/>
         </mask></defs><g mask='url(#m)'>
            <image x='4' y='2' width='8' height='8' href='data:image/svg+xml,{encoded}'/>
            <image x='20' y='2' width='8' height='8' href='data:image/svg+xml,{encoded}'/>
         </g>"
    );
    let image = render(renderer, &body, Affine::IDENTITY);
    assert_alpha(&image, |x, y| {
        if ((6..10).contains(&x) || (22..26).contains(&x)) && (2..10).contains(&y) {
            255
        } else {
            0
        }
    });
}

fn compound_clip(renderer: &mut dyn ImageRenderer) {
    let image = render(
        renderer,
        "<defs><clipPath id='c'>
            <path clip-rule='evenodd' d='M2 2H12V14H2Z M4 4H10V12H4Z'/>
            <path clip-rule='nonzero' d='M6 6H16V10H6Z'/>
         </clipPath></defs>
         <rect width='32' height='16' fill='white' clip-path='url(#c)'/>",
        Affine::IDENTITY,
    );
    assert_alpha(&image, |x, y| {
        let ring = (2..12).contains(&x)
            && (2..14).contains(&y)
            && !((4..10).contains(&x) && (4..12).contains(&y));
        let bar = (6..16).contains(&x) && (6..10).contains(&y);
        if ring || bar { 255 } else { 0 }
    });
}

fn overlapping_clip_children(renderer: &mut dyn ImageRenderer) {
    let image = render(
        renderer,
        "<defs><clipPath id='c'>
            <path d='M2 2H12V14H2Z'/>
            <path d='M6 4V12H16V4Z'/>
         </clipPath></defs>
         <rect width='32' height='16' fill='white' clip-path='url(#c)'/>",
        Affine::IDENTITY,
    );
    assert_alpha(&image, |x, y| {
        if ((2..12).contains(&x) && (2..14).contains(&y))
            || ((6..16).contains(&x) && (4..12).contains(&y))
        {
            255
        } else {
            0
        }
    });
}

fn clipped_clip_child(renderer: &mut dyn ImageRenderer) {
    let image = render(
        renderer,
        "<defs>
            <clipPath id='inner'><rect x='4' y='2' width='8' height='8'/></clipPath>
            <clipPath id='outer'>
                <rect x='2' y='2' width='6' height='12' clip-path='url(#inner)'/>
                <rect x='14' y='4' width='4' height='4'/>
            </clipPath>
         </defs>
         <g transform='translate(2 0)'><rect width='24' height='16' fill='white'
             clip-path='url(#outer)'/></g>",
        Affine::IDENTITY,
    );
    assert_alpha(&image, |x, y| {
        if ((6..10).contains(&x) && (2..10).contains(&y))
            || ((16..20).contains(&x) && (4..8).contains(&y))
        {
            255
        } else {
            0
        }
    });
}

fn clipped_group_blend(renderer: &mut dyn ImageRenderer) {
    let image = render(
        renderer,
        "<defs>
            <clipPath id='inner'><rect x='2' y='2' width='12' height='12'/></clipPath>
            <clipPath id='outer' clip-path='url(#inner)'>
                <rect width='10' height='16'/><rect x='6' width='10' height='16'/>
            </clipPath>
            <mask id='m' mask-type='alpha' maskUnits='userSpaceOnUse'
                x='0' y='0' width='16' height='16'>
                <rect width='16' height='16' fill='white'/>
            </mask>
         </defs>
         <rect width='32' height='16' fill='blue'/>
         <g clip-path='url(#outer)' mask='url(#m)' opacity='0.5'
             style='mix-blend-mode:multiply'>
             <rect width='16' height='16' fill='red'/>
         </g>",
        Affine::IDENTITY,
    );
    for (index, pixel) in image.data.chunks_exact(4).enumerate() {
        let (x, y) = (index % 32, index / 32);
        let blue = if (2..14).contains(&x) && (2..14).contains(&y) {
            127
        } else {
            255
        };
        assert_eq!(&pixel[..2], &[0, 0], "group blend at ({x}, {y})");
        assert!(
            pixel[2].abs_diff(blue) <= 1,
            "group blend at ({x}, {y}): {pixel:?}"
        );
        assert_eq!(pixel[3], 255, "opaque backdrop at ({x}, {y})");
    }
}

fn clipped_opacity(renderer: &mut dyn ImageRenderer) {
    let image = render(
        renderer,
        "<defs><clipPath id='c'><rect x='2' y='2' width='12' height='12'/></clipPath></defs>
         <g clip-path='url(#c)' opacity='0.5'><rect width='16' height='16' fill='white'/></g>",
        Affine::IDENTITY,
    );
    assert_alpha(&image, |x, y| {
        if (2..14).contains(&x) && (2..14).contains(&y) {
            128
        } else {
            0
        }
    });
}

fn isolated_blending(renderer: &mut dyn ImageRenderer) {
    let image = render(
        renderer,
        "<rect width='32' height='16' fill='blue'/>
         <g style='isolation:isolate'>
            <rect width='16' height='16' fill='red' style='mix-blend-mode:multiply'/>
         </g>",
        Affine::IDENTITY,
    );
    for (index, pixel) in image.data.chunks_exact(4).enumerate() {
        let expected = if index % 32 < 16 {
            [255, 0, 0, 255]
        } else {
            [0, 0, 255, 255]
        };
        assert_eq!(pixel, expected, "isolated blend at pixel {index}");
    }
}

macro_rules! backend_tests {
    ($backend:ident, $renderer:expr) => {
        mod $backend {
            use super::*;

            fn check(contract: fn(&mut dyn ImageRenderer)) {
                let renderer: Option<Box<dyn ImageRenderer>> = $renderer;
                if let Some(mut renderer) = renderer {
                    contract(renderer.as_mut());
                }
            }

            #[test]
            fn transformed_clip() {
                check(super::transformed_clip);
            }
            #[test]
            fn shared_mask_transforms() {
                check(super::shared_mask_transforms);
            }
            #[test]
            fn object_bounding_box_effects() {
                check(super::object_bounding_box_effects);
            }
            #[test]
            fn nested_masks() {
                check(super::nested_masks);
            }
            #[test]
            fn referenced_masks() {
                check(super::referenced_masks);
            }
            #[test]
            fn nested_svg_masks() {
                check(super::nested_svg_masks);
            }
            #[test]
            fn compound_clip() {
                check(super::compound_clip);
            }
            #[test]
            fn overlapping_clip_children() {
                check(super::overlapping_clip_children);
            }
            #[test]
            fn clipped_clip_child() {
                check(super::clipped_clip_child);
            }
            #[test]
            fn clipped_group_blend() {
                check(super::clipped_group_blend);
            }
            #[test]
            fn clipped_opacity() {
                check(super::clipped_opacity);
            }
            #[test]
            fn isolated_blending() {
                check(super::isolated_blending);
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
    Some(Box::new(imaging_vello_cpu::VelloCpuRenderer::new(32, 16)))
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
    })
);
