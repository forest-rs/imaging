// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Repeated render harness for CPU profiling snapshot backends.

use std::time::Instant;

use imaging_snapshot_tests::cases::{
    DEFAULT_HEIGHT, DEFAULT_WIDTH, build_scene, selected_cases_for_backend,
};

#[cfg(feature = "skia")]
use imaging_skia::SkiaRenderer;
#[cfg(feature = "tiny_skia")]
use imaging_tiny_skia::TinySkiaRenderer;
#[cfg(feature = "vello_cpu")]
use imaging_vello_cpu::VelloCpuRenderer;

fn parse_loops() -> usize {
    std::env::var("IMAGING_PROFILE_LOOPS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|&value| value > 0)
        .unwrap_or(400)
}

fn main() {
    let backend = std::env::var("IMAGING_PROFILE_BACKEND")
        .expect("set IMAGING_PROFILE_BACKEND to `skia`, `tiny_skia`, or `vello_cpu`");
    let loops = parse_loops();
    let cases = selected_cases_for_backend(&backend);
    assert_eq!(
        cases.len(),
        1,
        "set IMAGING_CASE to exactly one snapshot case for profiling"
    );
    let case = cases[0];
    let scene = build_scene(case, f64::from(DEFAULT_WIDTH), f64::from(DEFAULT_HEIGHT));

    let start = Instant::now();
    let mut checksum = 0_u64;

    match backend.as_str() {
        "skia" => {
            #[cfg(feature = "skia")]
            {
                let mut renderer = SkiaRenderer::new();
                for _ in 0..loops {
                    let image = renderer
                        .render_scene(&scene, DEFAULT_WIDTH, DEFAULT_HEIGHT)
                        .expect("render image");
                    checksum = checksum.wrapping_add(
                        image
                            .data
                            .iter()
                            .fold(0_u64, |acc, &byte| acc.wrapping_add(u64::from(byte))),
                    );
                }
            }
            #[cfg(not(feature = "skia"))]
            {
                let _ = &scene;
                panic!("profile_render built without `skia` feature");
            }
        }
        "tiny_skia" => {
            #[cfg(feature = "tiny_skia")]
            {
                let mut renderer = TinySkiaRenderer::new();
                for _ in 0..loops {
                    let image = renderer
                        .render_scene(&scene, u32::from(DEFAULT_WIDTH), u32::from(DEFAULT_HEIGHT))
                        .expect("render image");
                    checksum = checksum.wrapping_add(
                        image
                            .data
                            .iter()
                            .fold(0_u64, |acc, &byte| acc.wrapping_add(u64::from(byte))),
                    );
                }
            }
            #[cfg(not(feature = "tiny_skia"))]
            {
                let _ = &scene;
                panic!("profile_render built without `tiny_skia` feature");
            }
        }
        "vello_cpu" => {
            #[cfg(feature = "vello_cpu")]
            {
                let mut renderer = VelloCpuRenderer::new(DEFAULT_WIDTH, DEFAULT_HEIGHT);
                for _ in 0..loops {
                    let image = renderer
                        .render_scene(&scene, DEFAULT_WIDTH, DEFAULT_HEIGHT)
                        .expect("render image");
                    checksum = checksum.wrapping_add(
                        image
                            .data
                            .iter()
                            .fold(0_u64, |acc, &byte| acc.wrapping_add(u64::from(byte))),
                    );
                }
            }
            #[cfg(not(feature = "vello_cpu"))]
            {
                let _ = &scene;
                panic!("profile_render built without `vello_cpu` feature");
            }
        }
        other => panic!("unsupported profiling backend `{other}`"),
    }

    let elapsed = start.elapsed();
    eprintln!(
        "backend={backend} case={} loops={loops} elapsed_ms={} checksum={checksum}",
        case.name(),
        elapsed.as_millis(),
    );
}
