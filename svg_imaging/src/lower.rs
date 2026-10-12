// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Lower `usvg` trees into a crate-local render plan.

use alloc::{vec, vec::Vec};
use hashbrown::HashMap;

use imaging::MaskMode;
use kurbo::{Affine, BezPath, Cap, Join, Point, Shape as _, Stroke};
use peniko::{
    BlendMode, Blob, Brush, ColorStop, Extend, Fill, Gradient, ImageAlphaType, ImageBrush,
    ImageData, ImageFormat, ImageQuality, Mix,
};
use usvg::{Node, PaintOrder};

use crate::{
    RenderOptions,
    diagnostics::{RenderReport, UnsupportedFeatureKind},
    plan::{
        PlanAppliedMask, PlanClip, PlanFill, PlanGroup, PlanImage, PlanMask, PlanMaskId, PlanNode,
        PlanStroke,
    },
};

pub(crate) fn lower_tree(tree: &usvg::Tree, options: &RenderOptions) -> crate::plan::RenderPlan {
    let mut cx = LowerCx {
        masks: Vec::new(),
        lowered_masks: HashMap::new(),
        report: RenderReport::default(),
    };
    let nodes = cx.lower_group(tree.root(), options.transform);
    crate::plan::RenderPlan {
        masks: cx.masks,
        nodes,
        report: cx.report,
    }
}

struct LowerCx {
    masks: Vec<PlanMask>,
    lowered_masks: HashMap<usize, PlanMaskId>,
    report: RenderReport,
}

impl LowerCx {
    fn lower_group(&mut self, group: &usvg::Group, root_transform: Affine) -> Vec<PlanNode> {
        if !group.filters().is_empty() {
            self.unsupported(UnsupportedFeatureKind::Filter, group.id());
        }
        let transform = root_transform * convert_transform(group.abs_transform());
        let clips = group
            .clip_path()
            .and_then(|clip_path| self.lower_clip_path_chain(clip_path, transform, group.id()))
            .unwrap_or_default();
        let mask = group.mask().map(|mask| PlanAppliedMask {
            mask: self.lower_mask(mask),
            transform,
        });

        let mut children = Vec::new();
        for child in group.children() {
            children.extend(self.lower_node(child, root_transform));
        }

        let composite =
            imaging::Composite::new(map_blend_mode(group.blend_mode()), group.opacity().get());
        let needs_group = group.isolate()
            || composite != imaging::Composite::default()
            || !clips.is_empty()
            || mask.is_some();
        if needs_group {
            vec![PlanNode::Group(PlanGroup {
                clips,
                mask,
                composite,
                children,
            })]
        } else {
            children
        }
    }

    fn lower_node(&mut self, node: &Node, root_transform: Affine) -> Vec<PlanNode> {
        match node {
            Node::Group(group) => self.lower_group(group, root_transform),
            Node::Path(path) => self.lower_path(path, root_transform),
            Node::Image(image) => self.lower_image(image, root_transform),
            Node::Text(text) => self.lower_group(text.flattened(), root_transform),
        }
    }

    fn lower_path(&mut self, path: &usvg::Path, root_transform: Affine) -> Vec<PlanNode> {
        if !path.is_visible() {
            return Vec::new();
        }

        let geometry = convert_path(path.data());
        let transform = root_transform * convert_transform(path.abs_transform());
        let mut nodes = Vec::new();

        let push_fill =
            |nodes: &mut Vec<PlanNode>, fill: PlanFill| nodes.push(PlanNode::Fill(fill));
        let push_stroke =
            |nodes: &mut Vec<PlanNode>, stroke: PlanStroke| nodes.push(PlanNode::Stroke(stroke));

        match path.paint_order() {
            PaintOrder::FillAndStroke => {
                if let Some(fill) = self.lower_fill(path.fill(), &geometry, transform, path.id()) {
                    push_fill(&mut nodes, fill);
                }
                if let Some(stroke) =
                    self.lower_stroke(path.stroke(), &geometry, transform, path.id())
                {
                    push_stroke(&mut nodes, stroke);
                }
            }
            PaintOrder::StrokeAndFill => {
                if let Some(stroke) =
                    self.lower_stroke(path.stroke(), &geometry, transform, path.id())
                {
                    push_stroke(&mut nodes, stroke);
                }
                if let Some(fill) = self.lower_fill(path.fill(), &geometry, transform, path.id()) {
                    push_fill(&mut nodes, fill);
                }
            }
        }

        nodes
    }

    fn lower_fill(
        &mut self,
        fill: Option<&usvg::Fill>,
        geometry: &BezPath,
        transform: Affine,
        node_id: &str,
    ) -> Option<PlanFill> {
        let fill = fill?;
        let (brush, brush_transform) =
            self.lower_brush(fill.paint(), fill.opacity().get(), node_id)?;
        Some(PlanFill {
            path: geometry.clone(),
            transform,
            fill_rule: match fill.rule() {
                usvg::FillRule::NonZero => Fill::NonZero,
                usvg::FillRule::EvenOdd => Fill::EvenOdd,
            },
            brush,
            brush_transform,
        })
    }

    fn lower_stroke(
        &mut self,
        stroke: Option<&usvg::Stroke>,
        geometry: &BezPath,
        transform: Affine,
        node_id: &str,
    ) -> Option<PlanStroke> {
        let stroke = stroke?;
        let (brush, brush_transform) =
            self.lower_brush(stroke.paint(), stroke.opacity().get(), node_id)?;
        Some(PlanStroke {
            path: geometry.clone(),
            transform,
            stroke: convert_stroke(stroke),
            brush,
            brush_transform,
        })
    }

    fn lower_brush(
        &mut self,
        paint: &usvg::Paint,
        opacity: f32,
        node_id: &str,
    ) -> Option<(Brush, Option<Affine>)> {
        match paint {
            usvg::Paint::Color(color) => Some((convert_color(*color, opacity), None)),
            usvg::Paint::LinearGradient(gradient) => Some((
                Brush::Gradient(convert_linear_gradient(gradient).multiply_alpha(opacity)),
                affine_if_non_identity(gradient.transform()),
            )),
            usvg::Paint::RadialGradient(gradient) => Some((
                Brush::Gradient(convert_radial_gradient(gradient).multiply_alpha(opacity)),
                affine_if_non_identity(gradient.transform()),
            )),
            usvg::Paint::Pattern(_) => {
                self.unsupported(UnsupportedFeatureKind::PatternPaint, node_id);
                None
            }
        }
    }

    fn lower_image(&mut self, image: &usvg::Image, root_transform: Affine) -> Vec<PlanNode> {
        if !image.is_visible() {
            return Vec::new();
        }
        match image.kind() {
            usvg::ImageKind::SVG(tree) => self.lower_group(
                tree.root(),
                root_transform * convert_transform(image.abs_transform()),
            ),
            usvg::ImageKind::PNG(data) => self.lower_raster_image(
                data.as_slice(),
                image.rendering_mode(),
                root_transform * convert_transform(image.abs_transform()),
                image.id(),
                decode_png,
            ),
            usvg::ImageKind::JPEG(data) => self.lower_raster_image(
                data.as_slice(),
                image.rendering_mode(),
                root_transform * convert_transform(image.abs_transform()),
                image.id(),
                decode_jpeg,
            ),
            usvg::ImageKind::GIF(data) => self.lower_raster_image(
                data.as_slice(),
                image.rendering_mode(),
                root_transform * convert_transform(image.abs_transform()),
                image.id(),
                decode_gif,
            ),
            usvg::ImageKind::WEBP(data) => self.lower_raster_image(
                data.as_slice(),
                image.rendering_mode(),
                root_transform * convert_transform(image.abs_transform()),
                image.id(),
                decode_webp,
            ),
        }
    }

    fn lower_raster_image(
        &mut self,
        data: &[u8],
        rendering_mode: usvg::ImageRendering,
        transform: Affine,
        node_id: &str,
        decode: fn(&[u8]) -> Option<ImageData>,
    ) -> Vec<PlanNode> {
        let Some(image) = decode(data) else {
            self.unsupported(UnsupportedFeatureKind::Image, node_id);
            return Vec::new();
        };
        let image = PlanImage {
            image: with_rendering_quality(ImageBrush::new(image), rendering_mode),
            transform,
        };
        vec![PlanNode::Image(image)]
    }

    fn lower_clip_path_chain(
        &mut self,
        clip_path: &usvg::ClipPath,
        transform: Affine,
        node_id: &str,
    ) -> Option<Vec<PlanClip>> {
        let mut clips = if let Some(parent) = clip_path.clip_path() {
            self.lower_clip_path_chain(parent, transform, node_id)?
        } else {
            Vec::new()
        };
        clips.push(self.lower_single_clip_path(clip_path, transform, node_id)?);
        Some(clips)
    }

    fn lower_single_clip_path(
        &mut self,
        clip_path: &usvg::ClipPath,
        transform: Affine,
        node_id: &str,
    ) -> Option<PlanClip> {
        let nodes = self.lower_clip_group(
            clip_path.root(),
            convert_transform(clip_path.transform()),
            node_id,
        )?;
        // Keep the ordinary single-path case as a geometric clip. Multiple children must
        // be filled independently: concatenating their paths can cancel overlapping coverage.
        if nodes.len() == 1
            && let PlanNode::Fill(fill) = &nodes[0]
        {
            let mut path = fill.path.clone();
            path.apply_affine(transform * fill.transform);
            return Some(PlanClip::Path {
                path,
                fill_rule: fill.fill_rule,
            });
        }
        Some(PlanClip::Mask(PlanAppliedMask {
            mask: self.store_mask(MaskMode::Alpha, nodes),
            transform,
        }))
    }

    fn lower_mask_region(mask: &usvg::Mask) -> PlanClip {
        let rect = mask.rect().to_rect();
        let rect = kurbo::Rect::new(
            f64::from(rect.left()),
            f64::from(rect.top()),
            f64::from(rect.right()),
            f64::from(rect.bottom()),
        );
        PlanClip::Path {
            path: rect.to_path(0.1),
            fill_rule: Fill::NonZero,
        }
    }

    fn lower_mask(&mut self, mask: &usvg::Mask) -> PlanMaskId {
        let key = (mask as *const usvg::Mask) as usize;
        if let Some(mask_id) = self.lowered_masks.get(&key).copied() {
            return mask_id;
        }

        // Definitions stay in mask-local coordinates and can be placed by multiple groups.
        // Finish dependencies before assigning an ID so nested definitions cannot share a slot.
        let children = self.lower_group(mask.root(), Affine::IDENTITY);
        let linked = mask.mask().map(|linked| PlanAppliedMask {
            mask: self.lower_mask(linked),
            transform: Affine::IDENTITY,
        });
        let nodes = vec![PlanNode::Group(PlanGroup {
            clips: vec![Self::lower_mask_region(mask)],
            mask: linked,
            composite: imaging::Composite::default(),
            children,
        })];
        let mask_id = self.store_mask(lower_mask_mode(mask.kind()), nodes);
        self.lowered_masks.insert(key, mask_id);
        mask_id
    }

    fn store_mask(&mut self, mode: MaskMode, nodes: Vec<PlanNode>) -> PlanMaskId {
        let id = PlanMaskId(self.masks.len());
        self.masks.push(PlanMask { mode, nodes });
        id
    }

    fn lower_clip_group(
        &mut self,
        group: &usvg::Group,
        transform: Affine,
        node_id: &str,
    ) -> Option<Vec<PlanNode>> {
        if group.mask().is_some() || !group.filters().is_empty() {
            self.unsupported(UnsupportedFeatureKind::ClipPath, node_id);
            return None;
        }

        let transform = transform * convert_transform(group.transform());
        let mut children = Vec::new();
        for child in group.children() {
            children.extend(self.lower_clip_node(child, transform, node_id)?);
        }
        if let Some(clip) = group.clip_path() {
            Some(vec![PlanNode::Group(PlanGroup {
                clips: self.lower_clip_path_chain(clip, transform, node_id)?,
                mask: None,
                composite: imaging::Composite::default(),
                children,
            })])
        } else {
            Some(children)
        }
    }

    fn lower_clip_node(
        &mut self,
        node: &Node,
        transform: Affine,
        node_id: &str,
    ) -> Option<Vec<PlanNode>> {
        match node {
            Node::Path(path) => {
                if !path.is_visible() {
                    return Some(Vec::new());
                }
                let node_fill_rule = path
                    .fill()
                    .map(|fill| match fill.rule() {
                        usvg::FillRule::NonZero => Fill::NonZero,
                        usvg::FillRule::EvenOdd => Fill::EvenOdd,
                    })
                    .unwrap_or(Fill::NonZero);
                Some(vec![PlanNode::Fill(PlanFill {
                    path: convert_path(path.data()),
                    transform,
                    fill_rule: node_fill_rule,
                    brush: Brush::Solid(peniko::Color::WHITE),
                    brush_transform: None,
                })])
            }
            Node::Text(text) => self.lower_clip_group(text.flattened(), transform, node_id),
            Node::Group(group) => self.lower_clip_group(group, transform, node_id),
            Node::Image(_) => {
                self.unsupported(UnsupportedFeatureKind::ClipPath, node_id);
                None
            }
        }
    }

    fn unsupported(&mut self, kind: UnsupportedFeatureKind, node_id: &str) {
        self.report.push(
            kind,
            if node_id.is_empty() {
                None
            } else {
                Some(node_id)
            },
        );
    }
}

fn convert_path(path: &usvg::tiny_skia_path::Path) -> BezPath {
    let mut bez = BezPath::new();
    for segment in path.segments() {
        match segment {
            usvg::tiny_skia_path::PathSegment::MoveTo(point) => {
                bez.move_to(Point::new(f64::from(point.x), f64::from(point.y)));
            }
            usvg::tiny_skia_path::PathSegment::LineTo(point) => {
                bez.line_to(Point::new(f64::from(point.x), f64::from(point.y)));
            }
            usvg::tiny_skia_path::PathSegment::QuadTo(p1, p2) => {
                bez.quad_to(
                    Point::new(f64::from(p1.x), f64::from(p1.y)),
                    Point::new(f64::from(p2.x), f64::from(p2.y)),
                );
            }
            usvg::tiny_skia_path::PathSegment::CubicTo(p1, p2, p3) => {
                bez.curve_to(
                    Point::new(f64::from(p1.x), f64::from(p1.y)),
                    Point::new(f64::from(p2.x), f64::from(p2.y)),
                    Point::new(f64::from(p3.x), f64::from(p3.y)),
                );
            }
            usvg::tiny_skia_path::PathSegment::Close => bez.close_path(),
        }
    }
    bez
}

fn convert_transform(transform: usvg::Transform) -> Affine {
    Affine::new([
        f64::from(transform.sx),
        f64::from(transform.ky),
        f64::from(transform.kx),
        f64::from(transform.sy),
        f64::from(transform.tx),
        f64::from(transform.ty),
    ])
}

fn affine_if_non_identity(transform: usvg::Transform) -> Option<Affine> {
    (!transform.is_identity()).then(|| convert_transform(transform))
}

fn lower_mask_mode(kind: usvg::MaskType) -> MaskMode {
    match kind {
        usvg::MaskType::Luminance => MaskMode::Luminance,
        usvg::MaskType::Alpha => MaskMode::Alpha,
    }
}

fn convert_color(color: usvg::Color, opacity: f32) -> Brush {
    let alpha = opacity_to_u8(opacity);
    Brush::Solid(peniko::Color::from_rgba8(
        color.red,
        color.green,
        color.blue,
        alpha,
    ))
}

fn with_rendering_quality(image: ImageBrush, rendering_mode: usvg::ImageRendering) -> ImageBrush {
    let quality = match rendering_mode {
        usvg::ImageRendering::OptimizeSpeed
        | usvg::ImageRendering::CrispEdges
        | usvg::ImageRendering::Pixelated => ImageQuality::Low,
        usvg::ImageRendering::OptimizeQuality | usvg::ImageRendering::HighQuality => {
            ImageQuality::High
        }
        usvg::ImageRendering::Smooth => ImageQuality::Medium,
    };
    image.with_quality(quality)
}

fn decode_png(data: &[u8]) -> Option<ImageData> {
    decode_image(data, image::ImageFormat::Png)
}

fn decode_jpeg(data: &[u8]) -> Option<ImageData> {
    decode_image(data, image::ImageFormat::Jpeg)
}

fn decode_gif(data: &[u8]) -> Option<ImageData> {
    decode_image(data, image::ImageFormat::Gif)
}

fn decode_webp(data: &[u8]) -> Option<ImageData> {
    decode_image(data, image::ImageFormat::WebP)
}

fn decode_image(data: &[u8], format: image::ImageFormat) -> Option<ImageData> {
    let rgba = image::load_from_memory_with_format(data, format)
        .ok()?
        .into_rgba8();
    let (width, height) = rgba.dimensions();
    Some(ImageData {
        data: Blob::from(rgba.into_raw()),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width,
        height,
    })
}

fn convert_linear_gradient(gradient: &usvg::LinearGradient) -> Gradient {
    let stops = convert_stops(gradient.stops());
    Gradient::new_linear(
        (gradient.x1(), gradient.y1()),
        (gradient.x2(), gradient.y2()),
    )
    .with_extend(convert_extend(gradient.spread_method()))
    .with_stops(stops.as_slice())
}

fn convert_radial_gradient(gradient: &usvg::RadialGradient) -> Gradient {
    let stops = convert_stops(gradient.stops());
    Gradient::new_two_point_radial(
        (gradient.fx(), gradient.fy()),
        0.0,
        (gradient.cx(), gradient.cy()),
        gradient.r().get(),
    )
    .with_extend(convert_extend(gradient.spread_method()))
    .with_stops(stops.as_slice())
}

fn convert_stops(stops: &[usvg::Stop]) -> Vec<ColorStop> {
    stops
        .iter()
        .map(|stop| ColorStop {
            offset: stop.offset().get(),
            color: peniko::color::DynamicColor::from_alpha_color(peniko::Color::from_rgba8(
                stop.color().red,
                stop.color().green,
                stop.color().blue,
                opacity_to_u8(stop.opacity().get()),
            )),
        })
        .collect()
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "opacity is clamped to 0..=1 and scaled to 0..=255 before conversion"
)]
fn opacity_to_u8(opacity: f32) -> u8 {
    (opacity.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn convert_extend(spread: usvg::SpreadMethod) -> Extend {
    match spread {
        usvg::SpreadMethod::Pad => Extend::Pad,
        usvg::SpreadMethod::Reflect => Extend::Reflect,
        usvg::SpreadMethod::Repeat => Extend::Repeat,
    }
}

fn convert_stroke(stroke: &usvg::Stroke) -> Stroke {
    let mut result = Stroke::new(f64::from(stroke.width().get()))
        .with_join(match stroke.linejoin() {
            usvg::LineJoin::Miter | usvg::LineJoin::MiterClip => Join::Miter,
            usvg::LineJoin::Round => Join::Round,
            usvg::LineJoin::Bevel => Join::Bevel,
        })
        .with_miter_limit(f64::from(stroke.miterlimit().get()))
        .with_caps(match stroke.linecap() {
            usvg::LineCap::Butt => Cap::Butt,
            usvg::LineCap::Square => Cap::Square,
            usvg::LineCap::Round => Cap::Round,
        });
    if let Some(dashes) = stroke.dasharray() {
        result = result.with_dashes(
            f64::from(stroke.dashoffset()),
            dashes.iter().copied().map(f64::from),
        );
    }
    result
}

fn map_blend_mode(mode: usvg::BlendMode) -> BlendMode {
    match mode {
        usvg::BlendMode::Normal => BlendMode::default(),
        usvg::BlendMode::Multiply => BlendMode::from(Mix::Multiply),
        usvg::BlendMode::Screen => BlendMode::from(Mix::Screen),
        usvg::BlendMode::Overlay => BlendMode::from(Mix::Overlay),
        usvg::BlendMode::Darken => BlendMode::from(Mix::Darken),
        usvg::BlendMode::Lighten => BlendMode::from(Mix::Lighten),
        usvg::BlendMode::ColorDodge => BlendMode::from(Mix::ColorDodge),
        usvg::BlendMode::ColorBurn => BlendMode::from(Mix::ColorBurn),
        usvg::BlendMode::HardLight => BlendMode::from(Mix::HardLight),
        usvg::BlendMode::SoftLight => BlendMode::from(Mix::SoftLight),
        usvg::BlendMode::Difference => BlendMode::from(Mix::Difference),
        usvg::BlendMode::Exclusion => BlendMode::from(Mix::Exclusion),
        usvg::BlendMode::Hue => BlendMode::from(Mix::Hue),
        usvg::BlendMode::Saturation => BlendMode::from(Mix::Saturation),
        usvg::BlendMode::Color => BlendMode::from(Mix::Color),
        usvg::BlendMode::Luminosity => BlendMode::from(Mix::Luminosity),
    }
}
