// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

#![allow(
    missing_docs,
    reason = "Integration-test crate; these conformance specs are not public API docs."
)]

use imaging::{
    Painter, SourceLocationRef,
    record::{ContextNote, ResolvedSourceLocation, Scene, ValidateError},
};
use imaging_conformance::assert_validate_ok;

#[test]
fn unclosed_context_reports_label_and_source() {
    let mut scene = Scene::new();
    scene.push_context(
        "toolbar/button",
        Some(SourceLocationRef::new("widgets.rs", 7, 3)),
    );

    assert_eq!(
        scene.validate(),
        Err(ValidateError::UnclosedContexts {
            contexts: vec![ContextNote {
                label: "toolbar/button".into(),
                source: Some(ResolvedSourceLocation {
                    file: "widgets.rs".into(),
                    line: 7,
                    column: 3,
                }),
            }],
        }),
    );
}

#[test]
fn with_context_macro_produces_balanced_scene() {
    let mut scene = Scene::new();
    let mut painter = Painter::new(&mut scene);

    imaging::with_context!(painter, "toolbar/button", |p| {
        let _ = p;
    });

    assert_validate_ok(&scene);
}
