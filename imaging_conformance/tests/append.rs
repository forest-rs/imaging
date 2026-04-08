// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

#![allow(
    missing_docs,
    reason = "Integration-test crate; these conformance specs are not public API docs."
)]

use imaging::{
    Painter,
    record::{Command, Scene},
};
use imaging_conformance::assert_validate_ok;
use kurbo::Affine;

#[test]
fn append_transformed_preserves_context_annotations() {
    let mut source = Scene::new();
    {
        let mut painter = Painter::new(&mut source);
        painter.with_context("source/button", None, |p| {
            let _ = p;
        });
    }

    let mut dest = Scene::new();
    dest.append_transformed(&source, Affine::translate((5.0, 6.0)));

    assert_validate_ok(&dest);

    assert_eq!(dest.commands().len(), 2);

    let context_id = match &dest.commands()[0] {
        Command::PushContext(id) => *id,
        other => panic!("expected leading PushContext, got {other:?}"),
    };
    assert_eq!(dest.label(dest.context(context_id).label), "source/button");
    assert!(matches!(dest.commands()[1], Command::PopContext));
}
