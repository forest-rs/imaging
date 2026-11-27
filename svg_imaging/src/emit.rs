// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Emit a lowered SVG render plan through `imaging`.

use alloc::{vec, vec::Vec};

use imaging::{ClipRef, GroupRef, PaintSink, Painter, record};

use crate::plan::{
    PlanClip, PlanFill, PlanGroup, PlanImage, PlanMask, PlanMaskId, PlanNode, PlanStroke,
};

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
        let mask = group
            .mask
            .map(|applied| self.materialize_mask(applied.mask));
        let mut group_ref = GroupRef::new().with_composite(group.composite);
        if let (Some(mask), Some(applied)) = (&mask, group.mask) {
            group_ref = group_ref.with_mask_transformed(mask.as_ref(), applied.transform);
        }
        // The outer group owns compositing, even when its opacity and blend are defaults:
        // an explicitly isolated SVG group must still isolate child blends from the backdrop.
        let clips =
            if let Some((PlanClip::Path { path, fill_rule }, rest)) = group.clips.split_first() {
                group_ref = group_ref.with_clip(ClipRef::fill_with_rule(path, *fill_rule));
                rest
            } else {
                &group.clips
            };
        painter.with_group(group_ref, |painter| {
            self.emit_clip_layers(clips, &group.children, painter);
        });
    }

    fn emit_clip_layers<S>(
        &mut self,
        clips: &[PlanClip],
        children: &[PlanNode],
        painter: &mut Painter<'_, S>,
    ) where
        S: PaintSink + ?Sized,
    {
        let Some((clip, rest)) = clips.split_first() else {
            self.emit_nodes(children, painter);
            return;
        };
        match clip {
            PlanClip::Path { path, fill_rule } => {
                painter.with_group(
                    GroupRef::new().with_clip(ClipRef::fill_with_rule(path, *fill_rule)),
                    |painter| self.emit_clip_layers(rest, children, painter),
                );
            }
            PlanClip::Mask(applied) => {
                let mask = self.materialize_mask(applied.mask);
                painter.with_group(
                    GroupRef::new().with_mask_transformed(mask.as_ref(), applied.transform),
                    |painter| self.emit_clip_layers(rest, children, painter),
                );
            }
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
