// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Crate-local SVG render plan.

use alloc::vec::Vec;

use imaging::{Composite, MaskMode};
use kurbo::{Affine, BezPath, Stroke};
use peniko::{Brush, Fill, ImageBrush};

use crate::RenderReport;

/// Lowered render plan for an SVG document.
#[derive(Clone, Debug)]
pub(crate) struct RenderPlan {
    /// Reusable lowered mask definitions referenced from groups.
    pub(crate) masks: Vec<PlanMask>,
    /// Lowered nodes ready for emission through `imaging`.
    pub(crate) nodes: Vec<PlanNode>,
    /// Unsupported-feature diagnostics collected during lowering.
    pub(crate) report: RenderReport,
}

/// A lowered SVG node.
#[derive(Clone, Debug)]
pub(crate) enum PlanNode {
    /// Isolated group with explicit compositing.
    Group(PlanGroup),
    /// Filled path draw.
    Fill(PlanFill),
    /// Raster image draw.
    Image(PlanImage),
    /// Stroked path draw.
    Stroke(PlanStroke),
}

/// Lowered isolated group.
#[derive(Clone, Debug)]
pub(crate) struct PlanGroup {
    /// Isolated clip chain applied to the group result.
    ///
    /// Multiple entries represent intersected isolated clips, including nested `clip-path`
    /// references and mask regions.
    pub(crate) clips: Vec<PlanClip>,
    /// Optional retained mask applied to the group result before compositing.
    pub(crate) mask: Option<PlanAppliedMask>,
    /// Group compositing state.
    pub(crate) composite: Composite,
    /// Child nodes rendered inside the isolated group.
    pub(crate) children: Vec<PlanNode>,
}

/// Lowered group clip.
#[derive(Clone, Debug)]
pub(crate) struct PlanClip {
    /// Clip path geometry in final user-space coordinates.
    pub(crate) path: BezPath,
    /// Fill rule used for the clip.
    pub(crate) fill_rule: Fill,
}

/// Identifier for a lowered mask definition.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) struct PlanMaskId(pub(crate) usize);

/// Lowered retained mask definition.
#[derive(Clone, Debug)]
pub(crate) struct PlanMask {
    /// How the mask scene modulates masked content.
    pub(crate) mode: MaskMode,
    /// Child nodes rendered into the mask scene.
    pub(crate) nodes: Vec<PlanNode>,
}

/// Lowered application of a retained mask definition.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PlanAppliedMask {
    /// Referenced mask definition.
    pub(crate) mask: PlanMaskId,
}

/// Lowered fill draw.
#[derive(Clone, Debug)]
pub(crate) struct PlanFill {
    /// Path geometry in local coordinates.
    pub(crate) path: BezPath,
    /// Geometry transform.
    pub(crate) transform: Affine,
    /// SVG fill rule.
    pub(crate) fill_rule: Fill,
    /// Fill brush.
    pub(crate) brush: Brush,
    /// Optional brush-space transform.
    pub(crate) brush_transform: Option<Affine>,
}

/// Lowered raster image draw.
#[derive(Clone, Debug)]
pub(crate) struct PlanImage {
    /// Decoded image brush and sampling parameters.
    pub(crate) image: ImageBrush,
    /// Geometry transform applied when drawing at the image's natural size.
    pub(crate) transform: Affine,
}

/// Lowered stroke draw.
#[derive(Clone, Debug)]
pub(crate) struct PlanStroke {
    /// Path geometry in local coordinates.
    pub(crate) path: BezPath,
    /// Geometry transform.
    pub(crate) transform: Affine,
    /// Stroke style.
    pub(crate) stroke: Stroke,
    /// Stroke brush.
    pub(crate) brush: Brush,
    /// Optional brush-space transform.
    pub(crate) brush_transform: Option<Affine>,
}
