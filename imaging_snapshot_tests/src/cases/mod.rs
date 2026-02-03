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

fn case_filter_patterns() -> Vec<String> {
    let Ok(v) = std::env::var("IMAGING_CASE") else {
        return Vec::new();
    };
    v.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn matches_case_pattern(pattern: &str, name: &str) -> bool {
    if !pattern.contains('*') {
        return pattern == name;
    }

    // Simple `*` glob: all chunks must appear in-order.
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
    ///
    /// Skia output can vary slightly across platforms/toolchains for certain effects (notably
    /// filters). This provides a small per-case tolerance to keep CI stable.
    fn skia_max_diff_pixels(&self) -> u64 {
        0
    }

    /// Maximum number of pixels allowed to differ for Vello (GPU) snapshots.
    ///
    /// GPU output can vary slightly across platforms/drivers for AA-heavy cases. This provides a
    /// small per-case tolerance to keep CI stable while still catching real regressions.
    fn vello_max_diff_pixels(&self) -> u64 {
        0
    }

    /// Maximum number of pixels allowed to differ for Vello hybrid snapshots.
    ///
    /// Hybrid output can vary slightly across platforms/drivers for AA-heavy cases. This provides
    /// a small per-case tolerance to keep CI stable while still catching real regressions.
    fn vello_hybrid_max_diff_pixels(&self) -> u64 {
        0
    }

    /// Whether this case supports a specific backend.
    ///
    /// This allows sharing a single case list across all backends while skipping cases where a
    /// backend is known to be unsupported or unstable.
    fn supports_backend(&self, _backend: &str) -> bool {
        true
    }

    /// Emit commands into the given sink.
    fn run(&self, sink: &mut dyn Sink, width: f64, height: f64);
}

/// All snapshot cases.
pub const CASES: &[&dyn SnapshotCase] = &[
    &gradients::GmGradientsLinear,
    &gradients::GmGradientsSweep,
    &gradients::GmGradientsTwoPointRadial,
    &clips::GmClipNonIsolated,
    &filters::GmGroupBlurFilter,
    &filters::GmGroupDropShadow,
    &blends::GmBlendGrid,
    &strokes::GmStrokes,
];

/// List of cases to run for a given backend.
///
/// If `IMAGING_CASE` is set, this filters cases using `*` globs. If the filter matches no cases,
/// this panics and prints the available case names to avoid silently passing.
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
                .any(|p| matches_case_pattern(p, case.name()))
        })
        .collect();

    if selected.is_empty() {
        let available_names: Vec<&str> = available_for_backend.iter().map(|c| c.name()).collect();
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
