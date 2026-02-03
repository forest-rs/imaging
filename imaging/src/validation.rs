// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Defensive validation helpers for `imaging`.
//!
//! This module provides [`ValidatingSink`], a wrapper around a [`Sink`] that checks inputs for
//! common validity issues (NaNs/infinities, invalid rects, out-of-range alpha, etc.) before
//! forwarding commands to the wrapped sink.
//!
//! ## Examples
//!
//! Abort on the first invalid value (the default):
//!
//! ```rust
//! use imaging::{Composite, Draw, FillRule, Geometry, Paint, Scene, Sink};
//! use imaging::validation::ValidatingSink;
//! use kurbo::{Affine, Rect};
//!
//! let inner = Scene::new();
//! let mut sink = ValidatingSink::new(inner);
//!
//! sink.draw(Draw::Fill {
//!     transform: Affine::translate((f64::NAN, 0.0)),
//!     fill_rule: FillRule::NonZero,
//!     paint: Paint::default(),
//!     paint_transform: None,
//!     shape: Geometry::Rect(Rect::new(0.0, 0.0, 1.0, 1.0)),
//!     composite: Composite::default(),
//! });
//!
//! assert!(sink.first_error().is_some());
//! // Default behavior aborts and stops forwarding commands.
//! assert!(sink.inner().commands().is_empty());
//! ```
//!
//! Continue forwarding after a validation error (custom hook):
//!
//! ```rust
//! use imaging::{Composite, Draw, FillRule, Geometry, Paint, Scene, Sink};
//! use imaging::validation::{ValidationDecision, ValidatingSink};
//! use kurbo::{Affine, Rect};
//!
//! let inner = Scene::new();
//! let mut sink = ValidatingSink::with_hook(inner, |_err| ValidationDecision::Continue);
//!
//! sink.draw(Draw::Fill {
//!     transform: Affine::translate((f64::NAN, 0.0)),
//!     fill_rule: FillRule::NonZero,
//!     paint: Paint::default(),
//!     paint_transform: None,
//!     shape: Geometry::Rect(Rect::new(0.0, 0.0, 1.0, 1.0)),
//!     composite: Composite::default(),
//! });
//!
//! assert!(sink.first_error().is_some());
//! assert_eq!(sink.inner().commands().len(), 1);
//! ```

use crate::{Clip, Composite, Draw, Filter, Geometry, Group, Sink, StrokeStyle};
use kurbo::{Affine, BezPath, Rect, RoundedRect};

/// Decision returned by a [`ValidatingSink`] violation hook.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ValidationDecision {
    /// Continue forwarding commands to the wrapped sink.
    Continue,
    /// Abort: stop forwarding commands and record the first validation error.
    Abort,
}

/// A validation error reported by [`ValidatingSink`].
#[derive(Clone, Debug, PartialEq)]
pub enum ValidationError {
    /// A value that must be finite was NaN or infinite.
    NonFinite {
        /// Short label describing what was invalid.
        what: &'static str,
    },
    /// A rectangle had invalid bounds (e.g. `x0 > x1` or `y0 > y1`).
    InvalidRect {
        /// Short label describing what was invalid.
        what: &'static str,
    },
    /// A rounded rectangle had invalid radii (e.g. negative).
    InvalidRoundedRect {
        /// Short label describing what was invalid.
        what: &'static str,
    },
    /// A composite alpha was not finite or outside `0..=1`.
    InvalidAlpha,
    /// A stroke style had invalid parameters (e.g. negative width or non-finite dash values).
    InvalidStroke,
    /// A filter had invalid parameters (e.g. negative blur sigma or NaN offsets).
    InvalidFilter,
    /// A stack pop occurred without a corresponding push.
    StackUnderflow {
        /// Which stack underflowed.
        what: &'static str,
    },
    /// The command stream ended with open clips.
    UnclosedClips {
        /// Remaining clip depth.
        depth: u32,
    },
    /// The command stream ended with open groups.
    UnclosedGroups {
        /// Remaining group depth.
        depth: u32,
    },
}

/// Default violation hook for [`ValidatingSink`].
///
/// This is intentionally I/O-free to support `no_std`: it aborts on the first violation.
#[inline]
pub fn default_validation_hook(_: &ValidationError) -> ValidationDecision {
    ValidationDecision::Abort
}

/// A wrapper around a [`Sink`] that validates inputs before forwarding them.
///
/// This is intended as a defensive layer to catch invalid values (NaNs, infinities, etc.) early,
/// before they reach a backend.
///
/// The wrapper can be configured with a *violation hook* (`hook`) that decides whether to keep
/// forwarding commands after a validation error.
#[derive(Debug)]
pub struct ValidatingSink<S, H = fn(&ValidationError) -> ValidationDecision> {
    inner: S,
    hook: H,
    first_error: Option<ValidationError>,
    aborted: bool,
    clip_depth: u32,
    group_depth: u32,
}

impl<S> ValidatingSink<S> {
    /// Wrap a sink using the [`default_validation_hook`] (abort on first error).
    #[inline]
    pub fn new(inner: S) -> Self {
        Self::with_hook(inner, default_validation_hook)
    }
}

impl<S, H> ValidatingSink<S, H>
where
    H: FnMut(&ValidationError) -> ValidationDecision,
{
    /// Wrap a sink with a custom validation hook.
    #[inline]
    pub fn with_hook(inner: S, hook: H) -> Self {
        Self {
            inner,
            hook,
            first_error: None,
            aborted: false,
            clip_depth: 0,
            group_depth: 0,
        }
    }

    /// Borrow the wrapped sink.
    #[inline]
    pub fn inner(&self) -> &S {
        &self.inner
    }

    /// Mutably borrow the wrapped sink.
    #[inline]
    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.inner
    }

    /// Return the first validation error, if any.
    #[inline]
    pub fn first_error(&self) -> Option<&ValidationError> {
        self.first_error.as_ref()
    }

    /// Unwrap the sink, returning the inner sink and the first validation error (if any).
    #[inline]
    pub fn into_inner(self) -> (S, Option<ValidationError>) {
        (self.inner, self.first_error)
    }

    /// Validate that all push/pop stacks are balanced at end-of-stream.
    ///
    /// This is useful when streaming commands into a backend or recorder: unlike [`Scene::validate`],
    /// a [`ValidatingSink`] cannot know when you are "done" unless you call this explicitly.
    ///
    /// Returns `Err` if there are unclosed clips or groups.
    pub fn finish(&mut self) -> Result<(), ValidationError> {
        if self.clip_depth != 0 {
            let err = ValidationError::UnclosedClips {
                depth: self.clip_depth,
            };
            self.note_error(err.clone());
            return Err(err);
        }
        if self.group_depth != 0 {
            let err = ValidationError::UnclosedGroups {
                depth: self.group_depth,
            };
            self.note_error(err.clone());
            return Err(err);
        }
        Ok(())
    }

    fn note_error(&mut self, err: ValidationError) {
        if self.first_error.is_none() {
            self.first_error = Some(err);
        }
    }

    fn violate(&mut self, err: ValidationError) -> bool {
        self.note_error(err.clone());
        match (self.hook)(&err) {
            ValidationDecision::Continue => false,
            ValidationDecision::Abort => {
                self.aborted = true;
                true
            }
        }
    }

    fn validate_affine(&mut self, what: &'static str, xf: &Affine) -> bool {
        if xf.is_finite() {
            true
        } else {
            !self.violate(ValidationError::NonFinite { what })
        }
    }

    fn validate_rect(&mut self, what: &'static str, r: &Rect) -> bool {
        if !r.is_finite() {
            return !self.violate(ValidationError::NonFinite { what });
        }
        if r.x0 <= r.x1 && r.y0 <= r.y1 {
            true
        } else {
            !self.violate(ValidationError::InvalidRect { what })
        }
    }

    fn validate_rounded_rect(&mut self, what: &'static str, rr: &RoundedRect) -> bool {
        if !rr.is_finite() {
            return !self.violate(ValidationError::NonFinite { what });
        }
        let radii = rr.radii();
        if radii.top_left >= 0.0
            && radii.top_right >= 0.0
            && radii.bottom_right >= 0.0
            && radii.bottom_left >= 0.0
        {
            true
        } else {
            !self.violate(ValidationError::InvalidRoundedRect { what })
        }
    }

    fn validate_path(&mut self, what: &'static str, p: &BezPath) -> bool {
        if p.is_finite() {
            true
        } else {
            !self.violate(ValidationError::NonFinite { what })
        }
    }

    fn validate_geometry(&mut self, geom: &Geometry) -> bool {
        match geom {
            Geometry::Rect(r) => self.validate_rect("Geometry::Rect", r),
            Geometry::RoundedRect(rr) => self.validate_rounded_rect("Geometry::RoundedRect", rr),
            Geometry::Path(p) => self.validate_path("Geometry::Path", p),
        }
    }

    fn validate_stroke(&mut self, stroke: &StrokeStyle) -> bool {
        let ok = stroke.width.is_finite()
            && stroke.width >= 0.0
            && stroke.miter_limit.is_finite()
            && stroke.dash_offset.is_finite()
            && stroke
                .dash_pattern
                .iter()
                .all(|v| v.is_finite() && *v >= 0.0);
        if ok {
            true
        } else {
            !self.violate(ValidationError::InvalidStroke)
        }
    }

    fn validate_composite(&mut self, composite: &Composite) -> bool {
        let a = composite.alpha;
        let ok = a.is_finite() && (0.0..=1.0).contains(&a);
        if ok {
            true
        } else {
            !self.violate(ValidationError::InvalidAlpha)
        }
    }

    fn validate_filter(&mut self, f: &Filter) -> bool {
        let ok = match *f {
            Filter::Flood { .. } => true,
            Filter::Blur {
                std_deviation_x,
                std_deviation_y,
            } => {
                std_deviation_x.is_finite()
                    && std_deviation_y.is_finite()
                    && std_deviation_x >= 0.0
                    && std_deviation_y >= 0.0
            }
            Filter::DropShadow {
                dx,
                dy,
                std_deviation_x,
                std_deviation_y,
                ..
            } => {
                dx.is_finite()
                    && dy.is_finite()
                    && std_deviation_x.is_finite()
                    && std_deviation_y.is_finite()
                    && std_deviation_x >= 0.0
                    && std_deviation_y >= 0.0
            }
            Filter::Offset { dx, dy } => dx.is_finite() && dy.is_finite(),
        };
        if ok {
            true
        } else {
            !self.violate(ValidationError::InvalidFilter)
        }
    }
}

impl<S, H> Sink for ValidatingSink<S, H>
where
    S: Sink,
    H: FnMut(&ValidationError) -> ValidationDecision,
{
    fn push_clip(&mut self, clip: Clip) {
        if self.aborted {
            return;
        }

        let ok = match &clip {
            Clip::Fill {
                transform, shape, ..
            } => {
                self.validate_affine("Clip::Fill::transform", transform)
                    && self.validate_geometry(shape)
            }
            Clip::Stroke {
                transform,
                shape,
                stroke,
            } => {
                self.validate_affine("Clip::Stroke::transform", transform)
                    && self.validate_geometry(shape)
                    && self.validate_stroke(stroke)
            }
        };
        if !ok {
            return;
        }

        self.clip_depth += 1;
        self.inner.push_clip(clip);
    }

    fn pop_clip(&mut self) {
        if self.aborted {
            return;
        }
        if self.clip_depth == 0 {
            let _ = self.violate(ValidationError::StackUnderflow { what: "clip" });
            return;
        }
        self.clip_depth -= 1;
        self.inner.pop_clip();
    }

    fn push_group(&mut self, group: Group) {
        if self.aborted {
            return;
        }

        let mut ok = self.validate_composite(&group.composite);
        if let Some(clip) = group.clip.as_ref() {
            ok &= match clip {
                Clip::Fill {
                    transform, shape, ..
                } => {
                    self.validate_affine("Group::clip::Fill::transform", transform)
                        && self.validate_geometry(shape)
                }
                Clip::Stroke {
                    transform,
                    shape,
                    stroke,
                } => {
                    self.validate_affine("Group::clip::Stroke::transform", transform)
                        && self.validate_geometry(shape)
                        && self.validate_stroke(stroke)
                }
            };
        }
        for f in &group.filters {
            ok &= self.validate_filter(f);
        }
        if !ok {
            return;
        }

        self.group_depth += 1;
        self.inner.push_group(group);
    }

    fn pop_group(&mut self) {
        if self.aborted {
            return;
        }
        if self.group_depth == 0 {
            let _ = self.violate(ValidationError::StackUnderflow { what: "group" });
            return;
        }
        self.group_depth -= 1;
        self.inner.pop_group();
    }

    fn draw(&mut self, draw: Draw) {
        if self.aborted {
            return;
        }

        let ok = match &draw {
            Draw::Fill {
                transform,
                paint_transform,
                shape,
                composite,
                ..
            } => {
                self.validate_affine("Draw::Fill::transform", transform)
                    && paint_transform
                        .as_ref()
                        .is_none_or(|xf| self.validate_affine("Draw::Fill::paint_transform", xf))
                    && self.validate_geometry(shape)
                    && self.validate_composite(composite)
            }
            Draw::Stroke {
                transform,
                paint_transform,
                stroke,
                shape,
                composite,
                ..
            } => {
                self.validate_affine("Draw::Stroke::transform", transform)
                    && paint_transform
                        .as_ref()
                        .is_none_or(|xf| self.validate_affine("Draw::Stroke::paint_transform", xf))
                    && self.validate_stroke(stroke)
                    && self.validate_geometry(shape)
                    && self.validate_composite(composite)
            }
        };
        if !ok {
            return;
        }

        self.inner.draw(draw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FillRule, Paint, Scene};

    #[test]
    fn validating_sink_records_nan_and_aborts_by_default() {
        let inner = Scene::new();
        let mut v = ValidatingSink::new(inner);
        v.draw(Draw::Fill {
            transform: Affine::translate((f64::NAN, 0.0)),
            fill_rule: FillRule::NonZero,
            paint: Paint::default(),
            paint_transform: None,
            shape: Geometry::Rect(Rect::new(0.0, 0.0, 1.0, 1.0)),
            composite: Composite::default(),
        });
        assert!(matches!(
            v.first_error(),
            Some(ValidationError::NonFinite { .. })
        ));
        // Default policy aborts and stops forwarding.
        assert!(v.inner().commands().is_empty());
    }

    #[test]
    fn validating_sink_hook_can_continue() {
        let inner = Scene::new();
        let mut v = ValidatingSink::with_hook(inner, |_err| ValidationDecision::Continue);
        v.draw(Draw::Fill {
            transform: Affine::translate((f64::NAN, 0.0)),
            fill_rule: FillRule::NonZero,
            paint: Paint::default(),
            paint_transform: None,
            shape: Geometry::Rect(Rect::new(0.0, 0.0, 1.0, 1.0)),
            composite: Composite::default(),
        });
        // Error is recorded, but forwarding continues.
        assert!(v.first_error().is_some());
        assert_eq!(v.inner().commands().len(), 1);
    }

    #[test]
    fn finish_catches_unclosed_stacks() {
        let inner = Scene::new();
        let mut v = ValidatingSink::new(inner);

        v.push_clip(Clip::Fill {
            transform: Affine::IDENTITY,
            shape: Geometry::Rect(Rect::new(0.0, 0.0, 1.0, 1.0)),
            fill_rule: FillRule::NonZero,
        });

        assert_eq!(v.finish(), Err(ValidationError::UnclosedClips { depth: 1 }));
        assert_eq!(
            v.first_error(),
            Some(&ValidationError::UnclosedClips { depth: 1 })
        );
    }
}
