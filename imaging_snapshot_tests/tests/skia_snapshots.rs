// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Image snapshot tests for `imaging_skia` using `kompari`.

#![cfg(feature = "skia")]

use imaging_skia::SkiaRenderer;
use imaging_snapshot_tests::cases::{DEFAULT_HEIGHT, DEFAULT_WIDTH, build_scene};

mod common;

#[test]
fn snapshots() {
    let width = DEFAULT_WIDTH;
    let height = DEFAULT_HEIGHT;
    let w = f64::from(width);
    let h = f64::from(height);

    let Some(mut renderer) =
        common::try_init_or_skip("skia", || SkiaRenderer::try_new(width, height))
    else {
        return;
    };

    let mut errors = Vec::new();
    common::run_cases_with(
        "skia",
        |case| {
            let scene = build_scene(case, w, h);
            let bytes = renderer
                .render_scene_rgba8(&scene)
                .expect("render skia scene");

            kompari::image::ImageBuffer::from_raw(u32::from(width), u32::from(height), bytes)
                .expect("RGBA buffer size should match image dimensions")
        },
        |case| case.skia_max_diff_pixels(),
        &mut errors,
    );
    common::assert_no_snapshot_errors(errors);
}
