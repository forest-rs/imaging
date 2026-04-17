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
use pollster::block_on;

mod common;

fn try_init_device_and_queue() -> Result<
    (
        imaging_vello_hybrid::wgpu::Device,
        imaging_vello_hybrid::wgpu::Queue,
    ),
    (),
> {
    block_on(async {
        let instance = imaging_vello_hybrid::wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&imaging_vello_hybrid::wgpu::RequestAdapterOptions {
                power_preference: imaging_vello_hybrid::wgpu::PowerPreference::default(),
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await
            .map_err(|_| ())?;
        adapter
            .request_device(&imaging_vello_hybrid::wgpu::DeviceDescriptor {
                label: Some("imaging_snapshot_tests vello_hybrid device"),
                required_features: imaging_vello_hybrid::wgpu::Features::empty(),
                ..Default::default()
            })
            .await
            .map_err(|_| ())
    })
}

#[test]
fn snapshots() {
    let width = DEFAULT_WIDTH;
    let height = DEFAULT_HEIGHT;
    let w = f64::from(width);
    let h = f64::from(height);

    let Some(mut renderer) = common::try_init_or_skip(
        "vello_hybrid",
        || -> Result<_, imaging_vello_hybrid::Error> {
            let (device, queue) = try_init_device_and_queue()
                .map_err(|_| imaging_vello_hybrid::Error::Internal("initialize wgpu"))?;
            Ok(VelloHybridRenderer::new(device, queue))
        },
    ) else {
        return;
    };

    let mut errors = Vec::new();
    common::run_cases_with(
        "vello_hybrid",
        |case| {
            let scene = build_scene(case, w, h);
            let native = renderer
                .encode_scene(&scene, width, height)
                .expect("encode scene");
            renderer
                .render(&native, width, height)
                .expect("render image")
        },
        |case| case.vello_hybrid_max_diff_pixels(),
        &mut errors,
    );
    common::assert_no_snapshot_errors(errors);
}

#[test]
fn native_scene_sink_supports_image_brushes_with_renderer() {
    let Some(mut renderer) = common::try_init_or_skip(
        "vello_hybrid",
        || -> Result<_, imaging_vello_hybrid::Error> {
            let (device, queue) = try_init_device_and_queue()
                .map_err(|_| imaging_vello_hybrid::Error::Internal("initialize wgpu"))?;
            Ok(VelloHybridRenderer::new(device, queue))
        },
    ) else {
        return;
    };

    let mut scene = vello_hybrid::Scene::new(32, 32);
    scene.reset();
    {
        let brush = Brush::Image(ImageBrush::from(ImageData {
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
        .render(&scene, 32, 32)
        .expect("render native hybrid scene");
    let bytes = image.data.as_slice();
    assert_eq!(bytes.len(), 32 * 32 * 4);
    assert!(bytes.iter().any(|&channel| channel != 0));
}
