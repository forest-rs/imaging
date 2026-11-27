// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::{SkPictureRecorderSink, SkiaCpuRenderer};
use imaging::{
    ClipRef, GroupRef, MaskMode, Painter, RgbaImage,
    filter::{ColorMatrix, FilterPrimitive, TransferFunction},
    record::replay,
};
use kurbo::Affine;
use peniko::Color;

const REGION: Rect = Rect::new(0.0, 0.0, 16.0, 16.0);

fn srgb(kind: FilterPrimitiveKind) -> FilterPrimitive {
    FilterPrimitive {
        subregion: None,
        color_space: FilterColorSpace::Srgb,
        kind,
    }
}

fn result(index: usize) -> FilterInput {
    FilterInput::Result(FilterId(index))
}

fn filtered_scene(graph: &FilterGraph, draw: impl FnOnce(&mut Painter<'_, Scene>)) -> Scene {
    let mut scene = Scene::new();
    Painter::new(&mut scene).with_group(GroupRef::new().with_filter_graph(graph), draw);
    scene
}

fn pixel(image: &RgbaImage, x: usize, y: usize) -> [u8; 4] {
    let width = usize::try_from(image.width).expect("render width fits usize");
    let offset = (y * width + x) * 4;
    image.data[offset..offset + 4]
        .try_into()
        .expect("slice holds one pixel")
}

fn assert_pixel(image: &RgbaImage, x: usize, y: usize, expected: [u8; 4], tolerance: u8) {
    let actual = pixel(image, x, y);
    assert!(
        actual
            .into_iter()
            .zip(expected)
            .all(|(actual, expected)| actual.abs_diff(expected) <= tolerance),
        "pixel ({x}, {y}): expected {expected:?}, got {actual:?}",
    );
}

fn branching_scene() -> Scene {
    let region = Rect::new(2.0, 2.0, 12.0, 10.0);
    let graph = FilterGraph {
        region,
        primitives: vec![
            srgb(FilterPrimitiveKind::Flood {
                color: Color::from_rgb8(0, 0, 255),
            }),
            srgb(FilterPrimitiveKind::Offset {
                input: FilterInput::SourceAlpha,
                offset: Vec2::new(3.0, 0.0),
            }),
            FilterPrimitive {
                subregion: Some(Rect::new(4.0, 2.0, 10.0, 8.0)),
                ..srgb(FilterPrimitiveKind::Composite {
                    source: result(0),
                    backdrop: result(1),
                    operator: CompositeOperator::In,
                })
            },
            srgb(FilterPrimitiveKind::Merge {
                inputs: vec![FilterInput::SourceGraphic, result(2)],
            }),
            // The seed exceeds native precision, but this output is unreachable.
            srgb(FilterPrimitiveKind::Turbulence {
                base_frequency: Vec2::splat(0.5),
                num_octaves: 1,
                seed: 16_777_217,
                stitch_tiles: false,
                kind: TurbulenceKind::Turbulence,
            }),
        ],
        output: result(3),
    };
    filtered_scene(&graph, |painter| {
        painter.fill_rect(Rect::new(0.0, 3.0, 8.0, 7.0), Color::from_rgb8(255, 0, 0));
    })
}

fn assert_branching_scene(image: &RgbaImage) {
    assert_pixel(image, 1, 5, [0, 0, 0, 0], 0);
    assert_pixel(image, 4, 5, [255, 0, 0, 255], 0);
    assert_pixel(image, 5, 5, [0, 0, 255, 255], 0);
    assert_pixel(image, 9, 5, [0, 0, 255, 255], 0);
    assert_pixel(image, 10, 5, [0, 0, 0, 0], 0);
    assert_pixel(image, 6, 7, [0, 0, 0, 0], 0);
}

#[test]
fn source_branches_are_cropped_and_the_selected_result_is_rendered() {
    let scene = branching_scene();
    let mut renderer = SkiaCpuRenderer::new();
    assert_branching_scene(&renderer.render_scene(&scene, 16, 16).unwrap());

    let mut sink = SkPictureRecorderSink::new(REGION);
    replay(&scene, &mut sink);
    let picture = sink.finish_picture().unwrap();
    assert_branching_scene(&renderer.render_picture(&picture, 16, 16).unwrap());
}

#[test]
fn selecting_source_alpha_keeps_only_the_original_alpha() {
    let scene = filtered_scene(
        &FilterGraph {
            region: REGION,
            primitives: Vec::new(),
            output: FilterInput::SourceAlpha,
        },
        |painter| painter.fill_rect(REGION, Color::from_rgba8(255, 96, 32, 127)),
    );
    let image = SkiaCpuRenderer::new().render_scene(&scene, 16, 16).unwrap();
    assert_pixel(&image, 8, 8, [0, 0, 0, 127], 1);
}

#[test]
fn arithmetic_composite_interpolates_in_the_primitive_color_space() {
    let mut renderer = SkiaCpuRenderer::new();
    for (color_space, expected) in [
        (FilterColorSpace::Srgb, 128),
        (FilterColorSpace::LinearRgb, 188),
    ] {
        let scene = filtered_scene(
            &FilterGraph {
                region: REGION,
                primitives: vec![
                    srgb(FilterPrimitiveKind::Flood {
                        color: Color::BLACK,
                    }),
                    srgb(FilterPrimitiveKind::Flood {
                        color: Color::WHITE,
                    }),
                    FilterPrimitive {
                        color_space,
                        ..srgb(FilterPrimitiveKind::Composite {
                            source: result(1),
                            backdrop: result(0),
                            operator: CompositeOperator::Arithmetic {
                                k1: 0.0,
                                k2: 0.5,
                                k3: 0.5,
                                k4: 0.0,
                            },
                        })
                    },
                ],
                output: result(2),
            },
            |_| {},
        );
        let image = renderer.render_scene(&scene, 16, 16).unwrap();
        assert_pixel(&image, 8, 8, [expected, expected, expected, 255], 1);
    }
}

#[test]
fn component_transfer_preserves_straight_color_and_maps_alpha() {
    let graph = FilterGraph {
        region: REGION,
        primitives: vec![srgb(FilterPrimitiveKind::ComponentTransfer {
            input: FilterInput::SourceGraphic,
            functions: [
                TransferFunction::Table(vec![0.0, 1.0, 0.0]),
                TransferFunction::Discrete(vec![0.0, 1.0]),
                TransferFunction::Gamma {
                    amplitude: 0.5,
                    exponent: 2.0,
                    offset: 0.25,
                },
                TransferFunction::Linear {
                    slope: 0.75,
                    intercept: 0.0,
                },
            ],
        })],
        output: result(0),
    };
    let scene = filtered_scene(&graph, |painter| {
        painter.fill_rect(REGION, Color::from_rgb8(64, 128, 192));
    });
    let image = SkiaCpuRenderer::new().render_scene(&scene, 16, 16).unwrap();
    assert_pixel(&image, 8, 8, [128, 255, 136, 191], 2);
}

#[test]
fn color_matrix_can_replace_alpha_with_luminance() {
    let graph = FilterGraph {
        region: REGION,
        primitives: vec![srgb(FilterPrimitiveKind::ColorMatrix {
            input: FilterInput::SourceGraphic,
            matrix: ColorMatrix::LuminanceToAlpha,
        })],
        output: result(0),
    };
    let scene = filtered_scene(&graph, |painter| {
        painter.fill_rect(REGION, Color::from_rgb8(255, 0, 0));
    });
    let image = SkiaCpuRenderer::new().render_scene(&scene, 16, 16).unwrap();
    assert_pixel(&image, 8, 8, [0, 0, 0, 54], 1);
}

#[test]
fn transformed_embedded_scene_is_filtered_before_mask_and_clip() {
    let region = Rect::new(0.0, 0.0, 8.0, 6.0);
    let mut embedded = Scene::new();
    Painter::new(&mut embedded)
        .fill_rect(Rect::new(1.0, 1.0, 7.0, 5.0), Color::from_rgb8(0, 0, 255));
    let graph = FilterGraph {
        region,
        primitives: vec![srgb(FilterPrimitiveKind::Image {
            scene: Box::new(embedded),
        })],
        output: result(0),
    };
    let mask = Painter::<Scene>::record_mask(MaskMode::Alpha, |painter| {
        painter.fill_rect(Rect::new(12.0, 4.0, 22.0, 10.0), Color::WHITE);
    });
    let transform = Affine::translate((10.0, 4.0)) * Affine::scale_non_uniform(2.0, 1.0);
    let mut scene = Scene::new();
    Painter::new(&mut scene).with_group(
        GroupRef::new()
            .with_filter_graph(&graph)
            .with_filter_transform(transform)
            .with_clip(ClipRef::fill(Rect::new(0.0, 0.0, 20.0, 16.0)))
            .with_mask(mask.as_ref()),
        |_| {},
    );
    let mut renderer = SkiaCpuRenderer::new();
    let assert_pixels = |image: &RgbaImage| {
        assert_pixel(image, 11, 6, [0, 0, 0, 0], 0);
        assert_pixel(image, 16, 6, [0, 0, 255, 255], 0);
        assert_pixel(image, 20, 6, [0, 0, 0, 0], 0);
        assert_pixel(image, 16, 9, [0, 0, 0, 0], 0);
    };
    assert_pixels(&renderer.render_scene(&scene, 32, 16).unwrap());
    let mut sink = SkPictureRecorderSink::new(Rect::new(0.0, 0.0, 32.0, 16.0));
    replay(&scene, &mut sink);
    let picture = sink.finish_picture().unwrap();
    assert_pixels(&renderer.render_picture(&picture, 32, 16).unwrap());
}

#[test]
fn convolution_reverses_the_svg_kernel_and_wraps_at_the_input_edge() {
    let graph = FilterGraph {
        region: Rect::new(0.0, 0.0, 8.0, 8.0),
        primitives: vec![srgb(FilterPrimitiveKind::ConvolveMatrix {
            input: FilterInput::SourceGraphic,
            order: [2, 1],
            target: [0, 0],
            kernel: vec![1.0, 0.0],
            divisor: 1.0,
            bias: 0.1,
            edge_mode: EdgeMode::Wrap,
            preserve_alpha: true,
        })],
        output: result(0),
    };
    let scene = filtered_scene(&graph, |painter| {
        painter.fill_rect(graph.region, Color::BLACK);
        painter.fill_rect(Rect::new(0.0, 0.0, 1.0, 8.0), Color::from_rgb8(50, 0, 0));
        painter.fill_rect(Rect::new(4.0, 0.0, 5.0, 8.0), Color::from_rgb8(100, 0, 0));
    });
    let image = SkiaCpuRenderer::new().render_scene(&scene, 8, 8).unwrap();
    assert_pixel(&image, 3, 4, [126, 26, 26, 255], 1);
    assert_pixel(&image, 7, 4, [76, 26, 26, 255], 1);
}

#[test]
fn offset_uses_the_rotated_graph_coordinate_system() {
    let graph = FilterGraph {
        region: Rect::new(0.0, 0.0, 8.0, 8.0),
        primitives: vec![srgb(FilterPrimitiveKind::Offset {
            input: FilterInput::SourceGraphic,
            offset: Vec2::new(2.0, 0.0),
        })],
        output: result(0),
    };
    let transform = Affine::translate((12.0, 2.0)) * Affine::rotate(core::f64::consts::FRAC_PI_2);
    let mut scene = Scene::new();
    Painter::new(&mut scene).with_group(
        GroupRef::new()
            .with_filter_graph(&graph)
            .with_filter_transform(transform),
        |painter| painter.fill_rect(Rect::new(8.0, 3.0, 11.0, 5.0), Color::WHITE),
    );
    let image = SkiaCpuRenderer::new().render_scene(&scene, 16, 16).unwrap();
    assert_pixel(&image, 9, 4, [0, 0, 0, 0], 0);
    assert_pixel(&image, 9, 5, [255, 255, 255, 255], 0);
    assert_pixel(&image, 9, 7, [0, 0, 0, 0], 0);
    assert_pixel(&image, 12, 5, [0, 0, 0, 0], 0);
}

#[test]
fn tile_repeats_the_input_subregion() {
    let graph = FilterGraph {
        region: REGION,
        primitives: vec![
            FilterPrimitive {
                subregion: Some(Rect::new(1.0, 1.0, 5.0, 5.0)),
                ..srgb(FilterPrimitiveKind::Offset {
                    input: FilterInput::SourceGraphic,
                    offset: Vec2::ZERO,
                })
            },
            FilterPrimitive {
                subregion: Some(Rect::new(1.0, 1.0, 13.0, 9.0)),
                ..srgb(FilterPrimitiveKind::Tile { input: result(0) })
            },
        ],
        output: result(1),
    };
    let red = Color::from_rgb8(255, 0, 0);
    let blue = Color::from_rgb8(0, 0, 255);
    let scene = filtered_scene(&graph, |painter| {
        painter.fill_rect(Rect::new(1.0, 1.0, 3.0, 5.0), red);
        painter.fill_rect(Rect::new(3.0, 1.0, 5.0, 5.0), blue);
    });
    let image = SkiaCpuRenderer::new().render_scene(&scene, 16, 16).unwrap();
    for offset in [0, 4, 8] {
        assert_pixel(&image, 1 + offset, 7, [255, 0, 0, 255], 0);
        assert_pixel(&image, 3 + offset, 7, [0, 0, 255, 255], 0);
    }
    assert_pixel(&image, 0, 7, [0, 0, 0, 0], 0);
    assert_pixel(&image, 13, 7, [0, 0, 0, 0], 0);
}

#[test]
#[cfg(feature = "gpu")]
fn graph_renders_to_gpu_through_a_scene_and_native_picture() {
    let Some(mut renderer) = crate::tests::try_init_gpu_renderer() else {
        return;
    };
    let scene = branching_scene();
    let image = imaging::ImageRenderer::render_source(&mut renderer, &mut &scene, 16, 16).unwrap();
    assert_branching_scene(&image);

    let mut sink = SkPictureRecorderSink::new(REGION);
    replay(&scene, &mut sink);
    let picture = sink.finish_picture().unwrap();
    assert_branching_scene(&renderer.render_picture(&picture, 16, 16).unwrap());
}
