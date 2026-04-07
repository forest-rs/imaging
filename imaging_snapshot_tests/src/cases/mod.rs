// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Snapshot cases for `imaging` backends.
//!
//! These are intentionally “wow” / Skia GM–inspired visuals, translated into the `imaging` command API.

mod blends;
mod clips;
mod filters;
mod gradients;
mod images;
mod masks;
mod strokes;
mod svg;
mod text;
mod util;

use imaging::{PaintSink, record::Scene};

pub use self::util::{DEFAULT_HEIGHT, DEFAULT_WIDTH};

fn case_filter_patterns() -> Vec<String> {
    let Ok(value) = std::env::var("IMAGING_CASE") else {
        return Vec::new();
    };
    value
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn matches_case_pattern(pattern: &str, name: &str) -> bool {
    if !pattern.contains('*') {
        return pattern == name;
    }

    let mut remaining = name;
    let mut parts = pattern.split('*').peekable();
    let starts_with_star = pattern.starts_with('*');
    let ends_with_star = pattern.ends_with('*');

    if let Some(first) = parts.next() {
        if !starts_with_star && !remaining.starts_with(first) {
            return false;
        }
        if !first.is_empty() {
            remaining = &remaining[first.len()..];
        }
    }

    while let Some(part) = parts.next() {
        if part.is_empty() {
            continue;
        }
        if parts.peek().is_none() && !ends_with_star {
            return remaining.ends_with(part);
        }
        let Some(idx) = remaining.find(part) else {
            return false;
        };
        remaining = &remaining[idx + part.len()..];
    }

    true
}

/// A single snapshot test case.
pub trait SnapshotCase: Sync {
    /// Stable identifier used for snapshot filenames.
    fn name(&self) -> &'static str;

    /// Maximum number of pixels allowed to differ for Skia snapshots.
    fn skia_max_diff_pixels(&self) -> u64 {
        0
    }

    /// Maximum number of pixels allowed to differ for Vello GPU snapshots.
    fn vello_max_diff_pixels(&self) -> u64 {
        0
    }

    /// Maximum number of pixels allowed to differ for Vello hybrid snapshots.
    fn vello_hybrid_max_diff_pixels(&self) -> u64 {
        0
    }

    /// Whether this case should run on the given backend.
    fn supports_backend(&self, _backend: &str) -> bool {
        true
    }

    /// Emit commands into the given sink.
    fn run(&self, sink: &mut dyn PaintSink, width: f64, height: f64);
}

/// All snapshot cases.
pub const CASES: &[&dyn SnapshotCase] = &[
    &gradients::GmGradientsLinear,
    &gradients::GmGradientsSweep,
    &gradients::GmGradientsTwoPointRadial,
    &images::GmImageBrushes,
    &masks::GmMaskAlpha,
    &masks::GmMaskLuminance,
    &clips::GmClipNonIsolated,
    &clips::GmClipStrokeNested,
    &filters::GmGroupBlurFilter,
    &filters::GmGroupDropShadow,
    &filters::GmBlurredRoundedRect,
    &filters::GmBlurredRoundedRectVariants,
    &blends::GmBlendGrid,
    &strokes::GmStrokes,
    &text::GmGlyphRuns,
    &text::GmGlyphRunsGradientFill,
    &text::GmGlyphRunsGradientStroke,
    &text::GmGlyphRunsImageFill,
    &text::GmGlyphRunsImageStroke,
    &svg::GmSvgLayeredCard,
    &svg::GmSvgLuminanceMask,
    &svg::GmSvgTextBanner,
];

/// List of cases to run for a given backend.
pub fn selected_cases_for_backend(backend: &str) -> Vec<&'static dyn SnapshotCase> {
    let available_for_backend: Vec<&'static dyn SnapshotCase> = CASES
        .iter()
        .copied()
        .filter(|case| case.supports_backend(backend))
        .collect();

    let patterns = case_filter_patterns();
    if patterns.is_empty() {
        return available_for_backend;
    }

    let selected: Vec<&'static dyn SnapshotCase> = available_for_backend
        .iter()
        .copied()
        .filter(|case| {
            patterns
                .iter()
                .any(|pattern| matches_case_pattern(pattern, case.name()))
        })
        .collect();

    if selected.is_empty() {
        let available_names: Vec<&str> = available_for_backend
            .iter()
            .map(|case| case.name())
            .collect();
        panic!(
            "IMAGING_CASE matched no snapshot cases for backend `{backend}`.\n  filter: {patterns:?}\n  available: {available_names:?}"
        );
    }

    selected
}

/// Build a complete `Scene` for the given case.
pub fn build_scene(case: &dyn SnapshotCase, width: f64, height: f64) -> Scene {
    let mut scene = Scene::new();
    case.run(&mut scene, width, height);
    scene
}
