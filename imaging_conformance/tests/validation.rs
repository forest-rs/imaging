// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

#![allow(
    missing_docs,
    reason = "Integration-test crate; these conformance specs are not public API docs."
)]

use imaging::{
    ClipRef,
    record::{Geometry, Group, Scene, ValidateError},
};
use imaging_conformance::{assert_validate_err, assert_validate_ok};
use kurbo::Rect;

#[test]
fn balanced_clip_group_scene_validates() {
    let mut scene = Scene::new();
    scene.push_clip(ClipRef::fill(Geometry::Rect(Rect::new(0.0, 0.0, 10.0, 10.0))).to_owned());
    scene.push_group(Group::default());
    scene.pop_group();
    scene.pop_clip();

    assert_validate_ok(&scene);
}

#[test]
fn unbalanced_pop_clip_reports_empty_context_stack() {
    let mut scene = Scene::new();
    scene.pop_clip();

    assert_validate_err(
        &scene,
        ValidateError::UnbalancedPopClip { contexts: vec![] },
    );
}
