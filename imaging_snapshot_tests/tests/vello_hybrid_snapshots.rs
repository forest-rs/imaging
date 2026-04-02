// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Image snapshot tests for `imaging_vello_hybrid` using `kompari`.

#![cfg(feature = "vello_hybrid")]

use std::sync::Arc;

use imaging::Painter;
use imaging_snapshot_tests::cases::{DEFAULT_HEIGHT, DEFAULT_WIDTH, build_scene};
use imaging_vello_hybrid::{VelloHybridRenderer, VelloHybridSceneSink};
use kurbo::Rect;
use peniko::{Blob, Brush, ImageAlphaType, ImageBrush, ImageData, ImageFormat};

mod common;

#[test]
fn snapshots() {
    let width = DEFAULT_WIDTH;
    let height = DEFAULT_HEIGHT;
    let w = f64::from(width);
    let h = f64::from(height);

    let Some(mut renderer) = common::try_init_or_skip("vello_hybrid", || {
        VelloHybridRenderer::try_new(width, height)
    }) else {
        return;
    };

    let mut errors = Vec::new();
    common::run_cases_with(
        "vello_hybrid",
        |case| {
            let scene = build_scene(case, w, h);
            let image = renderer
                .render_scene_rgba8(&scene)
                .expect("render vello_hybrid scene");
            let bytes = image.data.as_ref().to_vec();

            kompari::image::ImageBuffer::from_raw(u32::from(width), u32::from(height), bytes)
                .expect("RGBA buffer size should match image dimensions")
        },
        |case| case.vello_hybrid_max_diff_pixels(),
        &mut errors,
    );
    common::assert_no_snapshot_errors(errors);
}

#[test]
fn native_scene_sink_supports_image_brushes_with_renderer() {
    let Some(mut renderer) =
        common::try_init_or_skip("vello_hybrid", || VelloHybridRenderer::try_new(32, 32))
    else {
        return;
    };

    let mut scene = vello_hybrid::Scene::new(32, 32);
    scene.reset();
    {
        let brush = Brush::Image(ImageBrush::new(ImageData {
            data: Blob::new(Arc::new([
                0xff, 0x20, 0x20, 0xff, 0x20, 0xff, 0x20, 0xff, 0x20, 0x20, 0xff, 0xff, 0xff, 0xff,
                0x20, 0xff,
            ])),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: 2,
            height: 2,
        }));
        let mut sink = VelloHybridSceneSink::with_renderer(&mut scene, &mut renderer);
        let mut painter = Painter::new(&mut sink);
        painter.fill_rect(Rect::new(0.0, 0.0, 32.0, 32.0), &brush);
        sink.finish().expect("finish native scene sink");
    }

    let image = renderer
        .render_vello_hybrid_scene_rgba8(&scene)
        .expect("render native hybrid scene");
    let bytes = image.data.as_ref();
    assert_eq!(bytes.len(), 32 * 32 * 4);
    assert!(bytes.iter().any(|&channel| channel != 0));
}
