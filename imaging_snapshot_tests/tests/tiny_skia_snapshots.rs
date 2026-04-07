// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Image snapshot tests for `imaging_tiny_skia` using `kompari`.

#![cfg(feature = "tiny_skia")]

use imaging_snapshot_tests::cases::{DEFAULT_HEIGHT, DEFAULT_WIDTH, build_scene};
use imaging_tiny_skia::TinySkiaRenderer;

mod common;

fn render_case(case: &dyn imaging_snapshot_tests::cases::SnapshotCase) -> imaging::RgbaImage {
    let width = DEFAULT_WIDTH;
    let height = DEFAULT_HEIGHT;
    let width_u32 = u32::from(width);
    let height_u32 = u32::from(height);
    let w = f64::from(width);
    let h = f64::from(height);

    let scene = build_scene(case, w, h);
    let mut renderer = TinySkiaRenderer::new();
    renderer
        .render_scene(&scene, width_u32, height_u32)
        .expect("render image")
}

#[test]
fn snapshots() {
    let mut errors = Vec::new();
    common::run_cases_with(
        "tiny_skia",
        render_case,
        |case| case.skia_max_diff_pixels(),
        &mut errors,
    );
    common::assert_no_snapshot_errors(errors);
}
