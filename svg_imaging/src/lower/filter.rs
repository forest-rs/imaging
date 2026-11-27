// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Lower normalized SVG filter definitions into shared imaging graphs.

use alloc::{boxed::Box, vec::Vec};

use hashbrown::HashMap;
use imaging::{
    Painter,
    filter::{
        ColorChannel, ColorMatrix, CompositeOperator, EdgeMode, FilterColorSpace, FilterGraph,
        FilterId, FilterInput, FilterPrimitive, FilterPrimitiveKind, LightSource,
        MorphologyOperator, TransferFunction, TurbulenceKind,
    },
    record::Scene,
};
use kurbo::{Affine, Rect, Vec2};
use usvg::filter::{ColorInterpolation, Input, Kind};

use super::{convert_color, convert_rect, lower_root, map_blend_mode};

pub(super) fn lower_graphs(group: &usvg::Group) -> Option<Vec<FilterGraph>> {
    group
        .filters()
        .iter()
        .map(|filter| lower_graph(filter))
        .collect()
}

fn lower_graph(filter: &usvg::filter::Filter) -> Option<FilterGraph> {
    let region = convert_rect(filter.rect());
    let mut primitives: Vec<FilterPrimitive> = Vec::with_capacity(filter.primitives().len());
    let mut results = HashMap::new();
    let mut output = FilterInput::SourceGraphic;

    for primitive in filter.primitives() {
        let resolve = |input: &Input| match input {
            Input::SourceGraphic => Some(FilterInput::SourceGraphic),
            Input::SourceAlpha => Some(FilterInput::SourceAlpha),
            Input::Reference(result) => Some(FilterInput::Result(*results.get(result.as_str())?)),
        };
        let subregion = convert_rect(primitive.rect());
        let kind = lower_kind(primitive.kind(), subregion, resolve)?;
        let subregion = match kind {
            // Match the offset subregion used by resvg and AnyRender.
            FilterPrimitiveKind::Offset { input, .. } => match input {
                FilterInput::Result(id) => primitives[id.0].subregion.unwrap_or(region),
                FilterInput::SourceGraphic | FilterInput::SourceAlpha => region,
            },
            _ => subregion.intersect(region),
        };
        let color_space = match primitive.color_interpolation() {
            ColorInterpolation::SRGB => FilterColorSpace::Srgb,
            ColorInterpolation::LinearRGB => FilterColorSpace::LinearRgb,
        };
        let id = FilterId(primitives.len());
        primitives.push(FilterPrimitive {
            subregion: Some(subregion),
            color_space,
            kind,
        });
        results.insert(primitive.result(), id);
        output = FilterInput::Result(id);
    }

    let graph = FilterGraph {
        region,
        primitives,
        output,
    };
    graph.validate().ok()?;
    Some(graph)
}

fn lower_kind(
    kind: &Kind,
    subregion: Rect,
    resolve: impl Fn(&Input) -> Option<FilterInput>,
) -> Option<FilterPrimitiveKind> {
    Some(match kind {
        Kind::Blend(fe) => FilterPrimitiveKind::Blend {
            source: resolve(fe.input1())?,
            backdrop: resolve(fe.input2())?,
            mode: map_blend_mode(fe.mode()).mix,
        },
        Kind::ColorMatrix(fe) => FilterPrimitiveKind::ColorMatrix {
            input: resolve(fe.input())?,
            matrix: match fe.kind() {
                usvg::filter::ColorMatrixKind::Matrix(samples) => {
                    let values: &[f32; 20] = samples.as_slice().try_into().ok()?;
                    ColorMatrix::Matrix(values.map(f64::from))
                }
                usvg::filter::ColorMatrixKind::Saturate(amount) => {
                    ColorMatrix::Saturate(f64::from(amount.get()))
                }
                usvg::filter::ColorMatrixKind::HueRotate(degrees) => {
                    ColorMatrix::HueRotate(f64::from(*degrees))
                }
                usvg::filter::ColorMatrixKind::LuminanceToAlpha => ColorMatrix::LuminanceToAlpha,
            },
        },
        Kind::ComponentTransfer(fe) => FilterPrimitiveKind::ComponentTransfer {
            input: resolve(fe.input())?,
            functions: [fe.func_r(), fe.func_g(), fe.func_b(), fe.func_a()].map(transfer_function),
        },
        Kind::Composite(fe) => FilterPrimitiveKind::Composite {
            source: resolve(fe.input1())?,
            backdrop: resolve(fe.input2())?,
            operator: composite_operator(fe.operator()),
        },
        Kind::ConvolveMatrix(fe) => {
            let matrix = fe.matrix();
            FilterPrimitiveKind::ConvolveMatrix {
                input: resolve(fe.input())?,
                order: [matrix.columns(), matrix.rows()],
                target: [matrix.target_x(), matrix.target_y()],
                kernel: matrix.data().iter().copied().map(f64::from).collect(),
                divisor: f64::from(fe.divisor().get()),
                bias: f64::from(fe.bias()),
                edge_mode: match fe.edge_mode() {
                    usvg::filter::EdgeMode::None => EdgeMode::None,
                    usvg::filter::EdgeMode::Duplicate => EdgeMode::Duplicate,
                    usvg::filter::EdgeMode::Wrap => EdgeMode::Wrap,
                },
                preserve_alpha: fe.preserve_alpha(),
            }
        }
        Kind::DiffuseLighting(fe) => FilterPrimitiveKind::DiffuseLighting {
            input: resolve(fe.input())?,
            surface_scale: f64::from(fe.surface_scale()),
            diffuse_constant: f64::from(fe.diffuse_constant()),
            lighting_color: convert_color(fe.lighting_color(), 1.0),
            light: light_source(fe.light_source()),
        },
        Kind::DisplacementMap(fe) => FilterPrimitiveKind::DisplacementMap {
            input: resolve(fe.input1())?,
            displacement: resolve(fe.input2())?,
            scale: f64::from(fe.scale()),
            x_channel: color_channel(fe.x_channel_selector()),
            y_channel: color_channel(fe.y_channel_selector()),
        },
        Kind::DropShadow(fe) => FilterPrimitiveKind::DropShadow {
            input: resolve(fe.input())?,
            offset: vector(fe.dx(), fe.dy()),
            std_deviation: vector(fe.std_dev_x().get(), fe.std_dev_y().get()),
            color: convert_color(fe.color(), fe.opacity().get()),
        },
        Kind::Flood(fe) => FilterPrimitiveKind::Flood {
            color: convert_color(fe.color(), fe.opacity().get()),
        },
        Kind::GaussianBlur(fe) => FilterPrimitiveKind::GaussianBlur {
            input: resolve(fe.input())?,
            std_deviation: vector(fe.std_dev_x().get(), fe.std_dev_y().get()),
        },
        Kind::Image(fe) => FilterPrimitiveKind::Image {
            scene: lower_image(fe, subregion.origin().to_vec2())?,
        },
        Kind::Merge(fe) => FilterPrimitiveKind::Merge {
            inputs: fe.inputs().iter().map(resolve).collect::<Option<_>>()?,
        },
        Kind::Morphology(fe) => FilterPrimitiveKind::Morphology {
            input: resolve(fe.input())?,
            operator: match fe.operator() {
                usvg::filter::MorphologyOperator::Erode => MorphologyOperator::Erode,
                usvg::filter::MorphologyOperator::Dilate => MorphologyOperator::Dilate,
            },
            radius: vector(fe.radius_x().get(), fe.radius_y().get()),
        },
        Kind::Offset(fe) => FilterPrimitiveKind::Offset {
            input: resolve(fe.input())?,
            offset: vector(fe.dx(), fe.dy()),
        },
        Kind::SpecularLighting(fe) => FilterPrimitiveKind::SpecularLighting {
            input: resolve(fe.input())?,
            surface_scale: f64::from(fe.surface_scale()),
            specular_constant: f64::from(fe.specular_constant()),
            specular_exponent: f64::from(fe.specular_exponent()),
            lighting_color: convert_color(fe.lighting_color(), 1.0),
            light: light_source(fe.light_source()),
        },
        Kind::Tile(fe) => FilterPrimitiveKind::Tile {
            input: resolve(fe.input())?,
        },
        Kind::Turbulence(fe) => FilterPrimitiveKind::Turbulence {
            base_frequency: vector(fe.base_frequency_x().get(), fe.base_frequency_y().get()),
            num_octaves: fe.num_octaves(),
            seed: fe.seed(),
            stitch_tiles: fe.stitch_tiles(),
            kind: match fe.kind() {
                usvg::filter::TurbulenceKind::FractalNoise => TurbulenceKind::FractalNoise,
                usvg::filter::TurbulenceKind::Turbulence => TurbulenceKind::Turbulence,
            },
        },
    })
}

fn lower_image(image: &usvg::filter::Image, position: Vec2) -> Option<Box<Scene>> {
    // The parser places the image at the origin of its primitive subregion.
    let plan = lower_root(image.root(), Affine::translate(position));
    if !plan.report.is_fully_supported() {
        return None;
    }
    let mut scene = Scene::new();
    crate::emit::emit_plan(&plan, &mut Painter::new(&mut scene));
    Some(Box::new(scene))
}

fn composite_operator(operator: usvg::filter::CompositeOperator) -> CompositeOperator {
    match operator {
        usvg::filter::CompositeOperator::Over => CompositeOperator::Over,
        usvg::filter::CompositeOperator::In => CompositeOperator::In,
        usvg::filter::CompositeOperator::Out => CompositeOperator::Out,
        usvg::filter::CompositeOperator::Atop => CompositeOperator::Atop,
        usvg::filter::CompositeOperator::Xor => CompositeOperator::Xor,
        usvg::filter::CompositeOperator::Lighter => CompositeOperator::Lighter,
        usvg::filter::CompositeOperator::Arithmetic { k1, k2, k3, k4 } => {
            CompositeOperator::Arithmetic {
                k1: f64::from(k1),
                k2: f64::from(k2),
                k3: f64::from(k3),
                k4: f64::from(k4),
            }
        }
    }
}

fn transfer_function(function: &usvg::filter::TransferFunction) -> TransferFunction {
    match function {
        usvg::filter::TransferFunction::Identity => TransferFunction::Identity,
        usvg::filter::TransferFunction::Table(values) => {
            TransferFunction::Table(values.iter().copied().map(f64::from).collect())
        }
        usvg::filter::TransferFunction::Discrete(values) => {
            TransferFunction::Discrete(values.iter().copied().map(f64::from).collect())
        }
        usvg::filter::TransferFunction::Linear { slope, intercept } => TransferFunction::Linear {
            slope: f64::from(*slope),
            intercept: f64::from(*intercept),
        },
        usvg::filter::TransferFunction::Gamma {
            amplitude,
            exponent,
            offset,
        } => TransferFunction::Gamma {
            amplitude: f64::from(*amplitude),
            exponent: f64::from(*exponent),
            offset: f64::from(*offset),
        },
    }
}

fn color_channel(channel: usvg::filter::ColorChannel) -> ColorChannel {
    match channel {
        usvg::filter::ColorChannel::R => ColorChannel::Red,
        usvg::filter::ColorChannel::G => ColorChannel::Green,
        usvg::filter::ColorChannel::B => ColorChannel::Blue,
        usvg::filter::ColorChannel::A => ColorChannel::Alpha,
    }
}

fn light_source(light: usvg::filter::LightSource) -> LightSource {
    match light {
        usvg::filter::LightSource::DistantLight(light) => LightSource::Distant {
            azimuth: f64::from(light.azimuth),
            elevation: f64::from(light.elevation),
        },
        usvg::filter::LightSource::PointLight(light) => LightSource::Point {
            position: [light.x, light.y, light.z].map(f64::from),
        },
        usvg::filter::LightSource::SpotLight(light) => LightSource::Spot {
            position: [light.x, light.y, light.z].map(f64::from),
            points_at: [light.points_at_x, light.points_at_y, light.points_at_z].map(f64::from),
            specular_exponent: f64::from(light.specular_exponent.get()),
            limiting_cone_angle: light.limiting_cone_angle.map(f64::from),
        },
    }
}

fn vector(x: f32, y: f32) -> Vec2 {
    Vec2::new(f64::from(x), f64::from(y))
}
