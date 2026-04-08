// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Backend-neutral conformance helpers for `imaging`.
//!
//! This crate is intentionally for a small, curated set of semantic contract tests:
//! behaviors we want to preserve even as internal representations and helper APIs are refactored.
//! Examples include stack balancing, append/replay preservation, and context/diagnostic semantics.
//!
//! It is **not** intended to absorb every test that could be written against `imaging`.
//! Implementation-adjacent regressions, helper edge cases, and module-local invariants should
//! usually stay in the owning crate next to the code they exercise.
//!
//! The actual conformance specifications live in the integration tests under `tests/`, while this
//! library module stays intentionally tiny and only provides shared assertions/helpers.

use imaging::{
    diagnostics::DiagnosticKind,
    record::{Scene, ValidateError},
};

/// Assert that a recorded scene is structurally valid.
pub fn assert_validate_ok(scene: &Scene) {
    assert_eq!(
        scene.validate(),
        Ok(()),
        "expected scene to validate successfully"
    );
}

/// Assert that a recorded scene fails validation with the expected error.
pub fn assert_validate_err(scene: &Scene, expected: ValidateError) {
    assert_eq!(
        scene.validate(),
        Err(expected),
        "expected scene to fail validation with the specified error"
    );
}

/// Collect just the diagnostic kinds produced for a scene.
#[must_use]
pub fn diagnostic_kinds(scene: &Scene) -> Vec<DiagnosticKind> {
    scene
        .diagnose()
        .into_iter()
        .map(|diagnostic| diagnostic.kind)
        .collect()
}
