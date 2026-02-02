// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Snapshot cases for `imaging` backends.
//!
//! These are intentionally “wow” / Skia GM–inspired visuals, translated into the `imaging` command API.

mod blends;
mod clips;
mod filters;
mod gradients;
mod strokes;
mod util;

use imaging::{Scene, Sink};

pub use self::util::{DEFAULT_HEIGHT, DEFAULT_WIDTH};

/// A single snapshot test case.
pub trait SnapshotCase: Sync {
    /// Stable identifier used for snapshot filenames.
    fn name(&self) -> &'static str;
    /// Emit commands into the given sink.
    fn run(&self, sink: &mut dyn Sink, width: f64, height: f64);
}

static CASES_VELLO_CPU: &[&dyn SnapshotCase] = &[
    &gradients::GmGradientsLinear,
    &gradients::GmGradientsSweep,
    &gradients::GmGradientsTwoPointRadial,
    &clips::GmClipNonIsolated,
    &filters::GmGroupBlurFilter,
    &filters::GmGroupDropShadow,
    &blends::GmBlendGrid,
    &strokes::GmStrokes,
];

static CASES_SKIA: &[&dyn SnapshotCase] = CASES_VELLO_CPU;

static CASES_VELLO_HYBRID: &[&dyn SnapshotCase] = &[
    &gradients::GmGradientsLinear,
    &gradients::GmGradientsSweep,
    &gradients::GmGradientsTwoPointRadial,
    &clips::GmClipNonIsolated,
    &blends::GmBlendGrid,
    &strokes::GmStrokes,
];

static CASES_VELLO: &[&dyn SnapshotCase] = CASES_VELLO_HYBRID;

/// List of cases to run for a given backend.
pub fn selected_cases_for_backend(backend: &str) -> &'static [&'static dyn SnapshotCase] {
    match backend {
        "vello_cpu" => CASES_VELLO_CPU,
        "skia" => CASES_SKIA,
        "vello_hybrid" => CASES_VELLO_HYBRID,
        "vello" => CASES_VELLO,
        _ => &[],
    }
}

/// Build a complete `Scene` for the given case.
pub fn build_scene(case: &dyn SnapshotCase, width: f64, height: f64) -> Scene {
    let mut scene = Scene::new();
    case.run(&mut scene, width, height);
    scene
}
