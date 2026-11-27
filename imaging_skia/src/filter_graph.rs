// Copyright 2026 the Imaging Authors and the AnyRender contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// Skia factory mappings adapted from the AnyRender v0.15.1 Skia scene painter.
// https://github.com/DioxusLabs/anyrender/blob/5d59eaea58a09ed854512d9949c790df8489c29b/crates/anyrender_skia/src/scene.rs

mod color;
mod lighting;

use crate::{Error, color_to_sk_color4f, f64_to_f32, map_blend_mode};
use imaging::{
    filter::{
        ColorChannel, CompositeOperator, EdgeMode, FilterColorSpace, FilterGraph, FilterId,
        FilterInput, FilterPrimitiveKind, MorphologyOperator, TurbulenceKind,
    },
    record::Scene,
};
use kurbo::{Rect, Vec2};
use peniko::{BlendMode, Compose};
use skia_safe::{self as sk, image_filters};

use color::{color_in_space, color_matrix, convert_color_space, source_alpha, transfer};
use lighting::{Lighting, lighting_filter};

type PictureRecorder<'a> = dyn FnMut(&Scene, Rect) -> Result<sk::Picture, Error> + 'a;

#[derive(Clone)]
struct Image {
    filter: sk::ImageFilter,
    bounds: sk::Rect,
    color_space: FilterColorSpace,
}

struct Inputs<'a> {
    source: &'a Image,
    alpha: &'a Image,
    results: &'a [Option<Image>],
}

impl Inputs<'_> {
    fn get(&self, input: FilterInput) -> Result<&Image, Error> {
        match input {
            FilterInput::SourceGraphic => Ok(self.source),
            FilterInput::SourceAlpha => Ok(self.alpha),
            FilterInput::Result(FilterId(index)) => self
                .results
                .get(index)
                .and_then(Option::as_ref)
                .ok_or(Error::UnsupportedFilter),
        }
    }
}

pub(super) fn build(
    graph: &FilterGraph,
    record_picture: &mut PictureRecorder<'_>,
) -> Result<sk::ImageFilter, Error> {
    let region = sk_rect(graph.region)?;
    if region.is_empty() {
        return Ok(image_filters::empty());
    }
    let source = Image {
        filter: native(image_filters::crop(region, None, None))?,
        bounds: region,
        color_space: FilterColorSpace::Srgb,
    };
    let alpha = Image {
        filter: source_alpha(source.filter.clone())?,
        ..source.clone()
    };

    let mut needed = vec![false; graph.primitives.len()];
    if let FilterInput::Result(FilterId(index)) = graph.output {
        *needed.get_mut(index).ok_or(Error::UnsupportedFilter)? = true;
    }
    for (index, primitive) in graph.primitives.iter().enumerate().rev() {
        if needed[index] {
            for input in primitive.kind.inputs() {
                if let FilterInput::Result(FilterId(dependency)) = input {
                    if dependency >= index {
                        return Err(Error::UnsupportedFilter);
                    }
                    needed[dependency] = true;
                }
            }
        }
    }

    let mut results = vec![None; graph.primitives.len()];
    for (index, primitive) in graph.primitives.iter().enumerate() {
        if !needed[index] {
            continue;
        }
        let inputs = Inputs {
            source: &source,
            alpha: &alpha,
            results: &results,
        };
        let output = primitive
            .subregion
            .unwrap_or(graph.region)
            .intersect(graph.region);
        let output_bounds = sk_rect(output)?;
        let color_space = match primitive.kind {
            FilterPrimitiveKind::DisplacementMap { input, .. }
            | FilterPrimitiveKind::Offset { input, .. }
            | FilterPrimitiveKind::Tile { input } => inputs.get(input)?.color_space,
            FilterPrimitiveKind::Flood { .. } | FilterPrimitiveKind::Image { .. } => {
                FilterColorSpace::Srgb
            }
            _ => primitive.color_space,
        };
        let result = if output_bounds.is_empty() {
            image_filters::empty()
        } else {
            primitive_filter(
                &primitive.kind,
                primitive.color_space,
                output,
                output_bounds,
                &inputs,
                record_picture,
            )?
        };
        let bounds = if primitive.subregion.is_some() || !result.can_compute_fast_bounds() {
            output_bounds
        } else {
            let mut natural = result.compute_fast_bounds(region);
            if natural.intersect(output_bounds) {
                natural
            } else {
                sk::Rect::new_empty()
            }
        };
        results[index] = Some(Image {
            filter: native(image_filters::crop(output_bounds, None, result))?,
            bounds,
            color_space,
        });
    }

    let inputs = Inputs {
        source: &source,
        alpha: &alpha,
        results: &results,
    };
    let output = inputs.get(graph.output)?;
    convert_color_space(
        output.filter.clone(),
        output.color_space,
        FilterColorSpace::Srgb,
    )
}

fn primitive_filter(
    kind: &FilterPrimitiveKind,
    space: FilterColorSpace,
    output: Rect,
    bounds: sk::Rect,
    inputs: &Inputs<'_>,
    record_picture: &mut PictureRecorder<'_>,
) -> Result<sk::ImageFilter, Error> {
    let get = |input| {
        let image = inputs.get(input)?;
        convert_color_space(image.filter.clone(), image.color_space, space)
    };
    let result = match kind {
        FilterPrimitiveKind::Blend {
            source,
            backdrop,
            mode,
        } => {
            let blend = BlendMode {
                mix: *mode,
                compose: Compose::SrcOver,
            };
            image_filters::blend(map_blend_mode(&blend), get(*backdrop)?, get(*source)?, None)
        }
        FilterPrimitiveKind::ColorMatrix { input, matrix } => {
            return color_matrix(matrix, get(*input)?);
        }
        FilterPrimitiveKind::ComponentTransfer { input, functions } => {
            return transfer(functions, get(*input)?);
        }
        FilterPrimitiveKind::Composite {
            source,
            backdrop,
            operator,
        } => {
            return composite(*operator, get(*source)?, get(*backdrop)?);
        }
        FilterPrimitiveKind::ConvolveMatrix {
            input,
            order,
            target,
            kernel,
            divisor,
            bias,
            edge_mode,
            preserve_alpha,
        } => {
            let src = inputs.get(*input)?;
            return convolve(
                get(*input)?,
                src.bounds,
                *order,
                *target,
                kernel,
                *divisor,
                *bias,
                *edge_mode,
                *preserve_alpha,
            );
        }
        FilterPrimitiveKind::DiffuseLighting {
            input,
            surface_scale,
            diffuse_constant,
            lighting_color,
            light,
        } => {
            return lighting_filter(
                get(*input)?,
                *surface_scale,
                color_in_space(*lighting_color, space),
                light,
                Lighting::Diffuse(*diffuse_constant),
            );
        }
        FilterPrimitiveKind::DisplacementMap {
            input,
            displacement,
            scale,
            x_channel,
            y_channel,
        } => image_filters::displacement_map(
            (channel(*x_channel), channel(*y_channel)),
            scalar(*scale)?,
            get(*displacement)?,
            inputs.get(*input)?.filter.clone(),
            None,
        ),
        FilterPrimitiveKind::DropShadow {
            input,
            offset,
            std_deviation,
            color,
        } => image_filters::drop_shadow(
            vector(*offset)?,
            vector(*std_deviation)?,
            color_to_sk_color4f(color_in_space(*color, space)),
            None,
            get(*input)?,
            None,
        ),
        FilterPrimitiveKind::Flood { color } => {
            let color = color_to_sk_color4f(*color);
            let shader = sk::shaders::color_in_space(color, sk::ColorSpace::new_srgb());
            image_filters::shader(shader, bounds)
        }
        FilterPrimitiveKind::GaussianBlur {
            input,
            std_deviation,
        } => image_filters::blur(vector(*std_deviation)?, None, get(*input)?, None),
        FilterPrimitiveKind::Image { scene } => {
            let scene = record_picture(scene, output)?;
            image_filters::picture(scene, Some(&bounds))
        }
        FilterPrimitiveKind::Merge { inputs } => {
            if inputs.is_empty() {
                return Ok(image_filters::empty());
            }
            let layers = inputs
                .iter()
                .copied()
                .map(|input| get(input).map(Some))
                .collect::<Result<Vec<_>, _>>()?;
            image_filters::merge(layers, None)
        }
        FilterPrimitiveKind::Morphology {
            input,
            operator,
            radius,
        } => {
            let radius = vector(*radius)?;
            let input = get(*input)?;
            match operator {
                MorphologyOperator::Erode => image_filters::erode(radius, input, None),
                MorphologyOperator::Dilate => image_filters::dilate(radius, input, None),
            }
        }
        FilterPrimitiveKind::Offset { input, offset } => {
            image_filters::offset(vector(*offset)?, inputs.get(*input)?.filter.clone(), None)
        }
        FilterPrimitiveKind::SpecularLighting {
            input,
            surface_scale,
            specular_constant,
            specular_exponent,
            lighting_color,
            light,
        } => {
            return lighting_filter(
                get(*input)?,
                *surface_scale,
                color_in_space(*lighting_color, space),
                light,
                Lighting::Specular {
                    coefficient: *specular_constant,
                    exponent: *specular_exponent,
                },
            );
        }
        FilterPrimitiveKind::Tile { input } => {
            let src = inputs.get(*input)?.bounds;
            if src.is_empty() {
                return Ok(image_filters::empty());
            }
            image_filters::tile(src, bounds, inputs.get(*input)?.filter.clone())
        }
        FilterPrimitiveKind::Turbulence {
            base_frequency,
            num_octaves,
            seed,
            stitch_tiles,
            kind,
        } => {
            let frequencies = vector(*base_frequency)?;
            let seed_value = scalar(f64::from(*seed))?;
            if f64::from(seed_value) != f64::from(*seed) || *num_octaves > 255 {
                return Err(Error::UnsupportedFilter);
            }
            let tile_size = if *stitch_tiles {
                Some(sk::ISize::new(
                    integer_size(output.width())?,
                    integer_size(output.height())?,
                ))
            } else {
                None
            };
            let octaves = usize::try_from(*num_octaves).map_err(|_| Error::UnsupportedFilter)?;
            let noise = match kind {
                TurbulenceKind::FractalNoise => {
                    sk::shaders::fractal_noise(frequencies, octaves, seed_value, tile_size)
                }
                TurbulenceKind::Turbulence => {
                    sk::shaders::turbulence(frequencies, octaves, seed_value, tile_size)
                }
            }
            .ok_or(Error::UnsupportedFilter)?;
            image_filters::shader(noise, bounds)
        }
    };
    native(result)
}

fn composite(
    operator: CompositeOperator,
    source: sk::ImageFilter,
    backdrop: sk::ImageFilter,
) -> Result<sk::ImageFilter, Error> {
    let mode = match operator {
        CompositeOperator::Over => sk::BlendMode::SrcOver,
        CompositeOperator::In => sk::BlendMode::SrcIn,
        CompositeOperator::Out => sk::BlendMode::SrcOut,
        CompositeOperator::Atop => sk::BlendMode::SrcATop,
        CompositeOperator::Xor => sk::BlendMode::Xor,
        CompositeOperator::Lighter => sk::BlendMode::Plus,
        CompositeOperator::Arithmetic { k1, k2, k3, k4 } => {
            return native(image_filters::arithmetic(
                scalar(k1)?,
                scalar(k2)?,
                scalar(k3)?,
                scalar(k4)?,
                true,
                backdrop,
                source,
                None,
            ));
        }
    };
    native(image_filters::blend(mode, backdrop, source, None))
}

fn convolve(
    input: sk::ImageFilter,
    bounds: sk::Rect,
    [columns, rows]: [u32; 2],
    [target_x, target_y]: [u32; 2],
    kernel: &[f64],
    divisor: f64,
    bias: f64,
    edge_mode: EdgeMode,
    preserve_alpha: bool,
) -> Result<sk::ImageFilter, Error> {
    let width = i32::try_from(columns).map_err(|_| Error::UnsupportedFilter)?;
    let height = i32::try_from(rows).map_err(|_| Error::UnsupportedFilter)?;
    if width <= 0
        || height <= 0
        || width
            .checked_mul(height)
            .and_then(|area| usize::try_from(area).ok())
            != Some(kernel.len())
        || divisor == 0.0
        || !divisor.is_finite()
        || target_x >= columns
        || target_y >= rows
    {
        return Err(Error::UnsupportedFilter);
    }
    let mode = match (edge_mode, bounds.is_empty()) {
        (EdgeMode::None, _) | (_, true) => None,
        (EdgeMode::Duplicate, false) => Some(sk::TileMode::Clamp),
        (EdgeMode::Wrap, false) => Some(sk::TileMode::Repeat),
    };
    let input = if let Some(mode) = mode {
        native(image_filters::crop(bounds, mode, input))?
    } else {
        input
    };
    let weights = kernel
        .iter()
        .rev()
        .copied()
        .map(scalar)
        .collect::<Result<Vec<_>, _>>()?;
    let target = (
        i32::try_from(target_x).map_err(|_| Error::UnsupportedFilter)?,
        i32::try_from(target_y).map_err(|_| Error::UnsupportedFilter)?,
    );
    let filter = native(image_filters::matrix_convolution(
        (width, height),
        &weights,
        scalar(divisor.recip())?,
        scalar(bias * 255.0)?,
        target,
        sk::TileMode::Decal,
        !preserve_alpha,
        input,
        None,
    ))?;
    if preserve_alpha && mode.is_some() {
        native(image_filters::crop(bounds, None, filter))
    } else {
        Ok(filter)
    }
}

fn channel(channel: ColorChannel) -> sk::ColorChannel {
    match channel {
        ColorChannel::Red => sk::ColorChannel::R,
        ColorChannel::Green => sk::ColorChannel::G,
        ColorChannel::Blue => sk::ColorChannel::B,
        ColorChannel::Alpha => sk::ColorChannel::A,
    }
}

fn sk_rect(rect: Rect) -> Result<sk::Rect, Error> {
    let [x0, y0, x1, y1] = [rect.x0, rect.y0, rect.x1, rect.y1];
    Ok(sk::Rect::new(
        scalar(x0)?,
        scalar(y0)?,
        scalar(x1)?,
        scalar(y1)?,
    ))
}

fn native(filter: Option<sk::ImageFilter>) -> Result<sk::ImageFilter, Error> {
    filter.ok_or(Error::UnsupportedFilter)
}

fn scalar(value: f64) -> Result<f32, Error> {
    let value = f64_to_f32(value);
    if value.is_finite() {
        Ok(value)
    } else {
        Err(Error::UnsupportedFilter)
    }
}

fn vector(vector: Vec2) -> Result<(f32, f32), Error> {
    Ok((scalar(vector.x)?, scalar(vector.y)?))
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "the integer range is checked"
)]
fn integer_size(value: f64) -> Result<i32, Error> {
    if value.fract() == 0.0 && value > 0.0 && value <= f64::from(i32::MAX) {
        Ok(value as i32)
    } else {
        Err(Error::UnsupportedFilter)
    }
}

#[cfg(test)]
mod tests;
