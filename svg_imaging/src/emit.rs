// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Emit a lowered SVG render plan through `imaging`.

use alloc::{vec, vec::Vec};

use imaging::{ClipRef, GroupRef, PaintSink, Painter, record};

use crate::plan::{PlanFill, PlanGroup, PlanImage, PlanMask, PlanMaskId, PlanNode, PlanStroke};

/// Emit a lowered render plan into a painter.
pub(crate) fn emit_plan<S>(plan: &crate::plan::RenderPlan, painter: &mut Painter<'_, S>)
where
    S: PaintSink + ?Sized,
{
    let mut cx = EmitCx::new(&plan.masks);
    cx.emit_nodes(&plan.nodes, painter);
}

struct EmitCx<'a> {
    masks: &'a [PlanMask],
    materialized_masks: Vec<Option<record::Mask>>,
}

impl<'a> EmitCx<'a> {
    fn new(masks: &'a [PlanMask]) -> Self {
        Self {
            masks,
            materialized_masks: vec![None; masks.len()],
        }
    }

    fn emit_nodes<S>(&mut self, nodes: &[PlanNode], painter: &mut Painter<'_, S>)
    where
        S: PaintSink + ?Sized,
    {
        for node in nodes {
            self.emit_node(node, painter);
        }
    }

    fn emit_node<S>(&mut self, node: &PlanNode, painter: &mut Painter<'_, S>)
    where
        S: PaintSink + ?Sized,
    {
        match node {
            PlanNode::Group(group) => self.emit_group(group, painter),
            PlanNode::Fill(fill) => emit_fill(fill, painter),
            PlanNode::Image(image) => emit_image(image, painter),
            PlanNode::Stroke(stroke) => emit_stroke(stroke, painter),
        }
    }

    fn emit_group<S>(&mut self, group: &PlanGroup, painter: &mut Painter<'_, S>)
    where
        S: PaintSink + ?Sized,
    {
        self.emit_group_layers(&group.clips, group, painter);
    }

    fn emit_group_layers<S>(
        &mut self,
        clips: &[crate::plan::PlanClip],
        group: &PlanGroup,
        painter: &mut Painter<'_, S>,
    ) where
        S: PaintSink + ?Sized,
    {
        if let Some((clip, rest)) = clips.split_first() {
            let group_ref = GroupRef::new()
                .with_clip(ClipRef::fill_with_rule(&clip.path, clip.fill_rule))
                .with_composite(if rest.is_empty() && group.mask.is_none() {
                    group.composite
                } else {
                    imaging::Composite::default()
                });
            painter.with_group(group_ref, |painter| {
                if rest.is_empty() {
                    self.emit_masked_children(group, painter);
                } else {
                    self.emit_group_layers(rest, group, painter);
                }
            });
        } else {
            self.emit_masked_children(group, painter);
        }
    }

    fn emit_masked_children<S>(&mut self, group: &PlanGroup, painter: &mut Painter<'_, S>)
    where
        S: PaintSink + ?Sized,
    {
        if let Some(mask) = group.mask {
            let mask = self.materialize_mask(mask.mask);
            let group_ref = GroupRef::new()
                .with_mask(mask.as_ref())
                .with_composite(group.composite);
            painter.with_group(group_ref, |painter| {
                self.emit_nodes(&group.children, painter);
            });
        } else if group.composite == imaging::Composite::default() {
            self.emit_nodes(&group.children, painter);
        } else {
            painter.with_group(GroupRef::new().with_composite(group.composite), |painter| {
                self.emit_nodes(&group.children, painter);
            });
        }
    }

    fn materialize_mask(&mut self, mask: PlanMaskId) -> record::Mask {
        if let Some(mask_def) = self.materialized_masks[mask.0].as_ref() {
            return mask_def.clone();
        }

        let plan_mask = &self.masks[mask.0];
        let mut scene = record::Scene::new();
        {
            let mut painter = Painter::new(&mut scene);
            self.emit_nodes(&plan_mask.nodes, &mut painter);
        }
        let materialized = record::Mask::new(plan_mask.mode, scene);
        self.materialized_masks[mask.0] = Some(materialized.clone());
        materialized
    }
}

fn emit_fill<S>(fill: &PlanFill, painter: &mut Painter<'_, S>)
where
    S: PaintSink + ?Sized,
{
    painter
        .fill(&fill.path, &fill.brush)
        .transform(fill.transform)
        .fill_rule(fill.fill_rule)
        .brush_transform(fill.brush_transform)
        .draw();
}

fn emit_image<S>(image: &PlanImage, painter: &mut Painter<'_, S>)
where
    S: PaintSink + ?Sized,
{
    painter.draw_image(&image.image, image.transform);
}

fn emit_stroke<S>(stroke: &PlanStroke, painter: &mut Painter<'_, S>)
where
    S: PaintSink + ?Sized,
{
    painter
        .stroke(&stroke.path, &stroke.stroke, &stroke.brush)
        .transform(stroke.transform)
        .brush_transform(stroke.brush_transform)
        .draw();
}
