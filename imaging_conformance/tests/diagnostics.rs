// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

#![allow(
    missing_docs,
    reason = "Integration-test crate; these conformance specs are not public API docs."
)]

use imaging::{ClipRef, diagnostics::DiagnosticKind, record::Geometry, record::Scene};
use imaging_conformance::diagnostic_kinds;
use kurbo::{BezPath, Rect};

#[test]
fn empty_clip_is_reported() {
    let mut scene = Scene::new();
    scene.push_clip(ClipRef::fill(Geometry::Rect(Rect::new(0.0, 0.0, 8.0, 8.0))).to_owned());
    scene.pop_clip();

    assert_eq!(diagnostic_kinds(&scene), vec![DiagnosticKind::EmptyClip]);
}

#[test]
fn empty_path_clip_is_reported() {
    let mut scene = Scene::new();
    scene.push_clip(ClipRef::fill(Geometry::Path(BezPath::new())).to_owned());
    scene.pop_clip();

    assert_eq!(
        diagnostic_kinds(&scene),
        vec![DiagnosticKind::EmptyPath, DiagnosticKind::EmptyClip]
    );
}
