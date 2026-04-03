// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Image snapshot tests for `imaging_vello` using `kompari`.

#![cfg(feature = "vello")]

use imaging_snapshot_tests::cases::{DEFAULT_HEIGHT, DEFAULT_WIDTH, build_scene};
use imaging_vello::VelloRenderer;
use pollster::block_on;

mod common;

fn try_init_device_and_queue()
-> Result<(imaging_vello::wgpu::Device, imaging_vello::wgpu::Queue), ()> {
    block_on(async {
        let instance = imaging_vello::wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&imaging_vello::wgpu::RequestAdapterOptions {
                power_preference: imaging_vello::wgpu::PowerPreference::default(),
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await
            .map_err(|_| ())?;
        adapter
            .request_device(&imaging_vello::wgpu::DeviceDescriptor {
                label: Some("imaging_snapshot_tests vello device"),
                required_features: imaging_vello::wgpu::Features::empty(),
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

    let Some(mut renderer) = common::try_init_or_skip("vello", || {
        let (device, queue) = try_init_device_and_queue()
            .map_err(|_| imaging_vello::Error::Internal("initialize wgpu"))?;
        VelloRenderer::new(device, queue)
    }) else {
        return;
    };

    let mut errors = Vec::new();
    common::run_cases_with(
        "vello",
        |case| {
            let scene = build_scene(case, w, h);
            let native = renderer
                .encode_scene(&scene, u32::from(width), u32::from(height))
                .expect("encode scene");
            renderer
                .render(&native, width, height)
                .expect("render image")
        },
        |case| case.vello_max_diff_pixels(),
        &mut errors,
    );
    common::assert_no_snapshot_errors(errors);
}
