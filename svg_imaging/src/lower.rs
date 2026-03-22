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
        root_transform: options.transform,
    };
    let nodes = cx.lower_group(tree.root());
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
    root_transform: Affine,
}

impl LowerCx {
    fn lower_group(&mut self, group: &usvg::Group) -> Vec<PlanNode> {
        if !group.filters().is_empty() {
            self.unsupported(UnsupportedFeatureKind::Filter, group.id());
        }
        let mut clips = group
            .clip_path()
            .and_then(|clip_path| self.lower_clip_path_chain(clip_path, group.id()))
            .unwrap_or_default();
        let mask = group.mask().and_then(|mask| {
            clips.push(self.lower_mask_region(mask));
            self.lower_mask(mask)
        });

        let mut children = Vec::new();
        for child in group.children() {
            children.extend(self.lower_node(child));
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

    fn lower_node(&mut self, node: &Node) -> Vec<PlanNode> {
        match node {
            Node::Group(group) => self.lower_group(group),
            Node::Path(path) => self.lower_path(path),
            Node::Image(image) => self.lower_image(image),
            Node::Text(text) => self.lower_group(text.flattened()),
        }
    }

    fn lower_path(&mut self, path: &usvg::Path) -> Vec<PlanNode> {
        if !path.is_visible() {
            return Vec::new();
        }

        let geometry = convert_path(path.data());
        let transform = self.root_transform * convert_transform(path.abs_transform());
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

    fn lower_image(&mut self, image: &usvg::Image) -> Vec<PlanNode> {
        if !image.is_visible() {
            return Vec::new();
        }
        match image.kind() {
            usvg::ImageKind::SVG(tree) => {
                let nested = lower_tree(
                    tree,
                    &RenderOptions {
                        transform: self.root_transform * convert_transform(image.abs_transform()),
                    },
                );
                self.report
                    .unsupported_features
                    .extend(nested.report.unsupported_features);
                nested.nodes
            }
            usvg::ImageKind::PNG(data) => self.lower_raster_image(
                data.as_slice(),
                image.rendering_mode(),
                image.abs_transform(),
                image.id(),
                decode_png,
            ),
            usvg::ImageKind::JPEG(data) => self.lower_raster_image(
                data.as_slice(),
                image.rendering_mode(),
                image.abs_transform(),
                image.id(),
                decode_jpeg,
            ),
            usvg::ImageKind::GIF(data) => self.lower_raster_image(
                data.as_slice(),
                image.rendering_mode(),
                image.abs_transform(),
                image.id(),
                decode_gif,
            ),
            usvg::ImageKind::WEBP(data) => self.lower_raster_image(
                data.as_slice(),
                image.rendering_mode(),
                image.abs_transform(),
                image.id(),
                decode_webp,
            ),
        }
    }

    fn lower_raster_image(
        &mut self,
        data: &[u8],
        rendering_mode: usvg::ImageRendering,
        abs_transform: usvg::Transform,
        node_id: &str,
        decode: fn(&[u8]) -> Option<ImageData>,
    ) -> Vec<PlanNode> {
        let Some(image) = decode(data) else {
            self.unsupported(UnsupportedFeatureKind::Image, node_id);
            return Vec::new();
        };
        let image = PlanImage {
            image: with_rendering_quality(ImageBrush::new(image), rendering_mode),
            transform: self.root_transform * convert_transform(abs_transform),
        };
        vec![PlanNode::Image(image)]
    }

    fn lower_clip_path_chain(
        &mut self,
        clip_path: &usvg::ClipPath,
        node_id: &str,
    ) -> Option<Vec<PlanClip>> {
        let mut clips = if let Some(parent) = clip_path.clip_path() {
            self.lower_clip_path_chain(parent, node_id)?
        } else {
            Vec::new()
        };
        clips.push(self.lower_single_clip_path(clip_path, node_id)?);
        Some(clips)
    }

    fn lower_single_clip_path(
        &mut self,
        clip_path: &usvg::ClipPath,
        node_id: &str,
    ) -> Option<PlanClip> {
        let mut path = BezPath::new();
        let mut fill_rule = None;
        let transform = convert_transform(clip_path.transform());

        if !self.collect_clip_group(clip_path.root(), transform, &mut path, &mut fill_rule)
            || path.is_empty()
        {
            self.unsupported(UnsupportedFeatureKind::ClipPath, node_id);
            return None;
        }

        Some(PlanClip {
            path,
            fill_rule: fill_rule.unwrap_or(Fill::NonZero),
        })
    }

    fn lower_mask_region(&self, mask: &usvg::Mask) -> PlanClip {
        let rect = mask.rect().to_rect();
        let rect = kurbo::Rect::new(
            f64::from(rect.left()),
            f64::from(rect.top()),
            f64::from(rect.right()),
            f64::from(rect.bottom()),
        );
        let mut path = rect.to_path(0.1);
        path.apply_affine(self.root_transform);
        PlanClip {
            path,
            fill_rule: Fill::NonZero,
        }
    }

    fn lower_mask(&mut self, mask: &usvg::Mask) -> Option<PlanAppliedMask> {
        let key = (mask as *const usvg::Mask) as usize;
        if let Some(mask_id) = self.lowered_masks.get(&key).copied() {
            return Some(PlanAppliedMask { mask: mask_id });
        }

        let mask_id = PlanMaskId(self.masks.len());
        self.lowered_masks.insert(key, mask_id);
        let nodes = self.lower_group(mask.root());
        self.masks.push(PlanMask {
            mode: lower_mask_mode(mask.kind()),
            nodes,
        });
        Some(PlanAppliedMask { mask: mask_id })
    }

    fn collect_clip_group(
        &mut self,
        group: &usvg::Group,
        transform: Affine,
        out: &mut BezPath,
        fill_rule: &mut Option<Fill>,
    ) -> bool {
        if group.clip_path().is_some() || group.mask().is_some() || !group.filters().is_empty() {
            return false;
        }

        let transform = transform * convert_transform(group.transform());
        for child in group.children() {
            if !self.collect_clip_node(child, transform, out, fill_rule) {
                return false;
            }
        }
        true
    }

    fn collect_clip_node(
        &mut self,
        node: &Node,
        transform: Affine,
        out: &mut BezPath,
        fill_rule: &mut Option<Fill>,
    ) -> bool {
        match node {
            Node::Path(path) => {
                if !path.is_visible() {
                    return true;
                }
                let node_fill_rule = path
                    .fill()
                    .map(|fill| match fill.rule() {
                        usvg::FillRule::NonZero => Fill::NonZero,
                        usvg::FillRule::EvenOdd => Fill::EvenOdd,
                    })
                    .unwrap_or(Fill::NonZero);
                if let Some(existing) = fill_rule {
                    if *existing != node_fill_rule {
                        return false;
                    }
                } else {
                    *fill_rule = Some(node_fill_rule);
                }
                let mut clip_path = convert_path(path.data());
                clip_path.apply_affine(transform);
                out.extend(clip_path.elements().iter().copied());
                true
            }
            Node::Text(text) => {
                self.collect_clip_group(text.flattened(), transform, out, fill_rule)
            }
            Node::Group(group) => self.collect_clip_group(group, transform, out, fill_rule),
            Node::Image(_) => false,
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
