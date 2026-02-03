// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Vello backend for `imaging`.
//!
//! This crate translates the `imaging` command stream into a `vello::Scene`.
//!
//! Rendering a `vello::Scene` to pixels requires `wgpu` device/queue setup; for CI snapshot
//! testing we keep that logic in `imaging_snapshot_tests` to better control lifetimes and reduce
//! platform-specific flakiness.

#![deny(unsafe_code)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use imaging::{Clip, Composite, Draw, Geometry, Group, Scene, Sink, replay};
use kurbo::{Affine, Rect};
use peniko::{Brush, Fill};

/// Errors that can occur when recording into a `vello::Scene`.
#[derive(Debug)]
pub enum Error {
    /// The scene is invalid (unbalanced stacks).
    InvalidScene(imaging::ValidateError),
    /// An image brush was encountered; this backend does not support images yet.
    UnsupportedImageBrush,
    /// A filter configuration could not be translated.
    UnsupportedFilter,
    /// The clip/group stack was not well-nested for this backend.
    ///
    /// Vello uses a single layer stack for both clipping and blending; `imaging` tracks these as
    /// separate stacks, so scenes that interleave them (e.g. `push_clip`, `push_group`, `pop_clip`)
    /// cannot be represented directly.
    UnbalancedLayerStack,
    /// An internal invariant was violated.
    Internal(&'static str),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum LayerKind {
    Clip,
    Group,
}

/// Recorder that translates `imaging` commands into a `vello::Scene`.
pub struct VelloRecorder {
    scene: vello::Scene,
    width: u16,
    height: u16,
    tolerance: f64,
    error: Option<Error>,
    layer_stack: Vec<LayerKind>,
}

impl core::fmt::Debug for VelloRecorder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VelloRecorder")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("tolerance", &self.tolerance)
            .field("error", &self.error)
            .field("layer_stack_depth", &self.layer_stack.len())
            .finish_non_exhaustive()
    }
}

impl VelloRecorder {
    /// Create a recorder for a fixed-size target.
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            scene: vello::Scene::new(),
            width,
            height,
            tolerance: 0.1,
            error: None,
            layer_stack: Vec::new(),
        }
    }

    /// Set the curve flattening tolerance used when converting rounded rects to paths.
    pub fn set_tolerance(&mut self, tolerance: f64) {
        self.tolerance = tolerance;
    }

    /// Reset the internal Vello scene and state.
    pub fn reset(&mut self) {
        self.scene.reset();
        self.error = None;
        self.layer_stack.clear();
    }

    /// Record an `imaging::Scene` into a `vello::Scene`.
    pub fn record(mut self, scene: &Scene) -> Result<vello::Scene, Error> {
        scene.validate().map_err(Error::InvalidScene)?;
        self.reset();
        replay(scene, &mut self);
        self.finish()
    }

    /// Finish recording and return the produced `vello::Scene`.
    pub fn finish(mut self) -> Result<vello::Scene, Error> {
        if let Some(err) = self.error.take() {
            return Err(err);
        }
        if !self.layer_stack.is_empty() {
            return Err(Error::Internal("unbalanced layer stack"));
        }
        Ok(self.scene)
    }

    fn set_error_once(&mut self, err: Error) {
        if self.error.is_none() {
            self.error = Some(err);
        }
    }

    fn brush_to_brush(&mut self, brush: Brush, composite: Composite) -> Option<Brush> {
        let brush = brush.multiply_alpha(composite.alpha);
        match brush {
            Brush::Solid(_) | Brush::Gradient(_) => Some(brush),
            Brush::Image(_) => {
                self.set_error_once(Error::UnsupportedImageBrush);
                None
            }
        }
    }

    fn surface_clip(&self) -> Rect {
        Rect::new(0.0, 0.0, f64::from(self.width), f64::from(self.height))
    }

    fn push_layer_kind(&mut self, kind: LayerKind) {
        self.layer_stack.push(kind);
    }

    fn pop_layer_kind(&mut self, expected: LayerKind) -> bool {
        match self.layer_stack.pop() {
            Some(kind) if kind == expected => true,
            _ => {
                self.set_error_once(Error::UnbalancedLayerStack);
                false
            }
        }
    }
}

impl Sink for VelloRecorder {
    fn push_clip(&mut self, clip: Clip) {
        if self.error.is_some() {
            return;
        }

        match clip {
            Clip::Fill {
                transform,
                shape,
                fill_rule,
            } => match shape {
                Geometry::Rect(r) => self.scene.push_clip_layer(fill_rule, transform, &r),
                Geometry::RoundedRect(rr) => self.scene.push_clip_layer(fill_rule, transform, &rr),
                Geometry::Path(p) => self.scene.push_clip_layer(fill_rule, transform, &p),
            },
            Clip::Stroke {
                transform,
                shape,
                stroke,
            } => match shape {
                Geometry::Rect(r) => self.scene.push_clip_layer(&stroke, transform, &r),
                Geometry::RoundedRect(rr) => self.scene.push_clip_layer(&stroke, transform, &rr),
                Geometry::Path(p) => self.scene.push_clip_layer(&stroke, transform, &p),
            },
        }
        self.push_layer_kind(LayerKind::Clip);
    }

    fn pop_clip(&mut self) {
        if self.error.is_some() {
            return;
        }
        if !self.pop_layer_kind(LayerKind::Clip) {
            return;
        }
        self.scene.pop_layer();
    }

    fn push_group(&mut self, group: Group) {
        if self.error.is_some() {
            return;
        }
        if !group.filters.is_empty() {
            self.set_error_once(Error::UnsupportedFilter);
            return;
        }

        if let Some(clip) = group.clip {
            match clip {
                Clip::Fill {
                    transform,
                    shape,
                    fill_rule,
                } => match shape {
                    Geometry::Rect(r) => self.scene.push_layer(
                        fill_rule,
                        group.composite.blend,
                        group.composite.alpha,
                        transform,
                        &r,
                    ),
                    Geometry::RoundedRect(rr) => self.scene.push_layer(
                        fill_rule,
                        group.composite.blend,
                        group.composite.alpha,
                        transform,
                        &rr,
                    ),
                    Geometry::Path(p) => self.scene.push_layer(
                        fill_rule,
                        group.composite.blend,
                        group.composite.alpha,
                        transform,
                        &p,
                    ),
                },
                Clip::Stroke {
                    transform,
                    shape,
                    stroke,
                } => match shape {
                    Geometry::Rect(r) => self.scene.push_layer(
                        &stroke,
                        group.composite.blend,
                        group.composite.alpha,
                        transform,
                        &r,
                    ),
                    Geometry::RoundedRect(rr) => self.scene.push_layer(
                        &stroke,
                        group.composite.blend,
                        group.composite.alpha,
                        transform,
                        &rr,
                    ),
                    Geometry::Path(p) => self.scene.push_layer(
                        &stroke,
                        group.composite.blend,
                        group.composite.alpha,
                        transform,
                        &p,
                    ),
                },
            }
        } else {
            let clip_box = self.surface_clip();
            self.scene.push_layer(
                Fill::NonZero,
                group.composite.blend,
                group.composite.alpha,
                Affine::IDENTITY,
                &clip_box,
            );
        }
        self.push_layer_kind(LayerKind::Group);
    }

    fn pop_group(&mut self) {
        if self.error.is_some() {
            return;
        }
        if !self.pop_layer_kind(LayerKind::Group) {
            return;
        }
        self.scene.pop_layer();
    }

    fn draw(&mut self, draw: Draw) {
        if self.error.is_some() {
            return;
        }

        match draw {
            Draw::Fill {
                transform,
                fill_rule,
                paint,
                paint_transform,
                shape,
                composite,
            } => {
                let Some(paint) = self.brush_to_brush(paint, composite) else {
                    return;
                };

                // Vello layers don't behave well if the layer content is entirely transparent and
                // the compose mode is destructive (notably `Copy`), because the raster coverage can
                // be effectively optimized away. We special-case “copy transparent” as “clear by
                // destination-out with an opaque source”, which preserves coverage/AA.
                let (blend, paint) = match (&paint, composite.blend.compose) {
                    (Brush::Solid(c), peniko::Compose::Copy) if c.components[3] == 0.0 => (
                        peniko::BlendMode::new(peniko::Mix::Normal, peniko::Compose::DestOut),
                        Brush::Solid(peniko::Color::from_rgba8(0, 0, 0, 255)),
                    ),
                    _ => (composite.blend, paint),
                };

                if blend != peniko::BlendMode::default() {
                    // Emulate per-draw blending using a layer. The layer clip must match the draw's
                    // geometry; otherwise destructive compose modes like `Copy` would affect the
                    // whole surface.
                    match &shape {
                        Geometry::Rect(r) => {
                            self.scene.push_layer(fill_rule, blend, 1.0, transform, r);
                        }
                        Geometry::RoundedRect(rr) => {
                            self.scene.push_layer(fill_rule, blend, 1.0, transform, rr);
                        }
                        Geometry::Path(p) => {
                            self.scene.push_layer(fill_rule, blend, 1.0, transform, p);
                        }
                    }
                    self.push_layer_kind(LayerKind::Group);
                }

                match shape {
                    Geometry::Rect(r) => {
                        self.scene
                            .fill(fill_rule, transform, &paint, paint_transform, &r);
                    }
                    Geometry::RoundedRect(rr) => {
                        self.scene
                            .fill(fill_rule, transform, &paint, paint_transform, &rr);
                    }
                    Geometry::Path(p) => {
                        self.scene
                            .fill(fill_rule, transform, &paint, paint_transform, &p);
                    }
                }

                if blend != peniko::BlendMode::default() {
                    if !self.pop_layer_kind(LayerKind::Group) {
                        return;
                    }
                    self.scene.pop_layer();
                }
            }
            Draw::Stroke {
                transform,
                stroke,
                paint,
                paint_transform,
                shape,
                composite,
            } => {
                let Some(paint) = self.brush_to_brush(paint, composite) else {
                    return;
                };

                let (blend, paint) = match (&paint, composite.blend.compose) {
                    (Brush::Solid(c), peniko::Compose::Copy) if c.components[3] == 0.0 => (
                        peniko::BlendMode::new(peniko::Mix::Normal, peniko::Compose::DestOut),
                        Brush::Solid(peniko::Color::from_rgba8(0, 0, 0, 255)),
                    ),
                    _ => (composite.blend, paint),
                };

                if blend != peniko::BlendMode::default() {
                    // Emulate per-draw blending using a layer. The layer clip must match the draw's
                    // geometry; otherwise destructive compose modes like `Copy` would affect the
                    // whole surface.
                    match &shape {
                        Geometry::Rect(r) => {
                            self.scene.push_layer(&stroke, blend, 1.0, transform, r);
                        }
                        Geometry::RoundedRect(rr) => {
                            self.scene.push_layer(&stroke, blend, 1.0, transform, rr);
                        }
                        Geometry::Path(p) => {
                            self.scene.push_layer(&stroke, blend, 1.0, transform, p);
                        }
                    }
                    self.push_layer_kind(LayerKind::Group);
                }

                match shape {
                    Geometry::Rect(r) => {
                        self.scene
                            .stroke(&stroke, transform, &paint, paint_transform, &r);
                    }
                    Geometry::RoundedRect(rr) => {
                        self.scene
                            .stroke(&stroke, transform, &paint, paint_transform, &rr);
                    }
                    Geometry::Path(p) => {
                        self.scene
                            .stroke(&stroke, transform, &paint, paint_transform, &p);
                    }
                }

                if blend != peniko::BlendMode::default() {
                    if !self.pop_layer_kind(LayerKind::Group) {
                        return;
                    }
                    self.scene.pop_layer();
                }
            }
        }
    }
}
