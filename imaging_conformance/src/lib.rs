// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Conformance tests for the `imaging` command semantics.
//!
//! This crate is intentionally `std`-using and `publish = false`. It exists to keep test-only
//! dependencies out of the core `imaging` crate.

#[cfg(test)]
mod tests {
    use imaging::{Clip, FillRule, Geometry, Group, Scene, ValidateError};
    use kurbo::Rect;

    #[test]
    fn smoke() {
        let mut s = Scene::new();
        s.push_clip(Clip::Fill {
            transform: kurbo::Affine::IDENTITY,
            shape: Geometry::Rect(Rect::new(0.0, 0.0, 10.0, 10.0)),
            fill_rule: FillRule::NonZero,
        });
        s.push_group(Group::default());
        s.pop_group();
        s.pop_clip();
        assert_eq!(s.validate(), Ok(()));

        let mut bad = Scene::new();
        bad.pop_clip();
        assert_eq!(bad.validate(), Err(ValidateError::UnbalancedPopClip));
    }
}
