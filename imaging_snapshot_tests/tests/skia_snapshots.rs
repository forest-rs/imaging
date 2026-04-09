// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Image snapshot tests for `imaging_skia` using `kompari`.

#![cfg(feature = "skia")]

use imaging_skia::SkiaCpuRenderer;
use imaging_snapshot_tests::cases::{DEFAULT_HEIGHT, DEFAULT_WIDTH, build_scene};

mod common;

fn render_case(case: &dyn imaging_snapshot_tests::cases::SnapshotCase) -> imaging::RgbaImage {
    let width = DEFAULT_WIDTH;
    let height = DEFAULT_HEIGHT;
    let w = f64::from(width);
    let h = f64::from(height);

    let scene = build_scene(case, w, h);
    let mut renderer = SkiaCpuRenderer::new();
    renderer
        .render_scene(&scene, width, height)
        .expect("render image")
}

#[test]
fn snapshots() {
    let mut errors = Vec::new();
    common::run_cases_with(
        "skia",
        render_case,
        |case| case.skia_max_diff_pixels(),
        &mut errors,
    );
    common::assert_no_snapshot_errors(errors);
}
