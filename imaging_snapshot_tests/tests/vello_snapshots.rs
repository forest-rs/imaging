// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Image snapshot tests for `imaging_vello` using `kompari`.

#![cfg(feature = "vello")]

use kompari::Image;

use imaging_snapshot_tests::cases::{DEFAULT_HEIGHT, DEFAULT_WIDTH, build_scene};
use imaging_vello::VelloRenderer;

mod common;

fn render_case(case: &dyn imaging_snapshot_tests::cases::SnapshotCase) -> Image {
    let width = DEFAULT_WIDTH;
    let height = DEFAULT_HEIGHT;
    let w = f64::from(width);
    let h = f64::from(height);

    let scene = build_scene(case, w, h);
    let mut renderer = VelloRenderer::try_new(width, height).expect("create vello");
    let bytes = renderer
        .render_scene_rgba8(&scene)
        .expect("render vello scene");

    kompari::image::ImageBuffer::from_raw(u32::from(width), u32::from(height), bytes)
        .expect("RGBA buffer size should match image dimensions")
}

#[test]
fn snapshots() {
    // In some sandboxed/headless environments, `wgpu` can't create a usable device.
    // Treat that as a skip rather than a snapshot failure.
    if VelloRenderer::try_new(DEFAULT_WIDTH, DEFAULT_HEIGHT).is_err() {
        eprintln!("[vello] no adapter/device available; skipping snapshots");
        return;
    }
    let mut errors = Vec::new();
    common::run_cases("vello", render_case, &mut errors);
    common::assert_no_snapshot_errors(errors);
}
