// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Image snapshot tests for `imaging_vello` using `kompari`.

#![cfg(feature = "vello")]

use imaging_snapshot_tests::cases::{DEFAULT_HEIGHT, DEFAULT_WIDTH, build_scene};
use imaging_vello::VelloRenderer;

mod common;

mod gpu;

#[test]
fn snapshots() {
    let width = DEFAULT_WIDTH;
    let height = DEFAULT_HEIGHT;
    let w = f64::from(width);
    let h = f64::from(height);

    let Some((device, queue)) = gpu::device() else {
        return;
    };
    let mut renderer = VelloRenderer::new(device, queue).expect("initialize Vello renderer");

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
