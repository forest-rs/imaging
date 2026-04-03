// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Image snapshot tests for `imaging_vello_cpu` using `kompari`.

#![cfg(feature = "vello_cpu")]

use imaging_snapshot_tests::cases::{DEFAULT_HEIGHT, DEFAULT_WIDTH, build_scene};
use imaging_vello_cpu::VelloCpuRenderer;

mod common;

fn render_case(case: &dyn imaging_snapshot_tests::cases::SnapshotCase) -> imaging::RgbaImage {
    let width = DEFAULT_WIDTH;
    let height = DEFAULT_HEIGHT;
    let w = f64::from(width);
    let h = f64::from(height);

    let scene = build_scene(case, w, h);
    let mut renderer = VelloCpuRenderer::new(width, height);
    renderer
        .render_scene(&scene, width, height)
        .expect("render image")
}

#[test]
fn snapshots() {
    let mut errors = Vec::new();
    common::run_cases("vello_cpu", render_case, &mut errors);
    common::assert_no_snapshot_errors(errors);
}
