// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Image snapshot tests for `imaging_vello` using `kompari`.

#![cfg(feature = "vello")]

use imaging_snapshot_tests::cases::{DEFAULT_HEIGHT, DEFAULT_WIDTH, build_scene};
use imaging_vello::VelloRecorder;

mod common;

#[test]
fn snapshots() {
    let width = DEFAULT_WIDTH;
    let height = DEFAULT_HEIGHT;
    let w = f64::from(width);
    let h = f64::from(height);

    // In some sandboxed/headless environments, `wgpu` can't create a usable device.
    // Treat that as a skip rather than a snapshot failure.
    let Some(wgpu) = common::try_init_or_skip("vello", common::vello_wgpu::Context::try_new) else {
        return;
    };

    let mut errors = Vec::new();
    common::run_cases_with(
        "vello",
        |case| {
            let scene = build_scene(case, w, h);
            let vello_scene = VelloRecorder::new(width, height)
                .record(&scene)
                .expect("record vello scene");
            let bytes = wgpu
                .render_rgba8(&vello_scene, width, height)
                .expect("render vello scene via wgpu");

            kompari::image::ImageBuffer::from_raw(u32::from(width), u32::from(height), bytes)
                .expect("RGBA buffer size should match image dimensions")
        },
        |case| case.vello_max_diff_pixels(),
        &mut errors,
    );
    common::assert_no_snapshot_errors(errors);
}
