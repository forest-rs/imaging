// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::fmt::Write as _;

use imaging::filter::{ColorMatrix, FilterColorSpace, TransferFunction};
use peniko::{Color, color::LinearSrgb};
use skia_safe::{self as sk, color_filters, image_filters};

use super::{Error, native, scalar};

pub(super) fn color_in_space(color: Color, space: FilterColorSpace) -> Color {
    match space {
        FilterColorSpace::Srgb => color,
        FilterColorSpace::LinearRgb => Color::new(color.convert::<LinearSrgb>().components),
    }
}

pub(super) fn convert_color_space(
    filter: sk::ImageFilter,
    from: FilterColorSpace,
    to: FilterColorSpace,
) -> Result<sk::ImageFilter, Error> {
    let gamma = match (from, to) {
        (FilterColorSpace::Srgb, FilterColorSpace::LinearRgb) => {
            color_filters::srgb_to_linear_gamma()
        }
        (FilterColorSpace::LinearRgb, FilterColorSpace::Srgb) => {
            color_filters::linear_to_srgb_gamma()
        }
        _ => return Ok(filter),
    };
    native(image_filters::color_filter(gamma, filter, None))
}

pub(super) fn source_alpha(source: sk::ImageFilter) -> Result<sk::ImageFilter, Error> {
    let mut alpha_only = [0.0; 20];
    alpha_only[18] = 1.0;
    let alpha = color_filters::matrix_row_major(&alpha_only, color_filters::Clamp::Yes);
    native(image_filters::color_filter(alpha, source, None))
}

pub(super) fn color_matrix(
    matrix: &ColorMatrix,
    input: sk::ImageFilter,
) -> Result<sk::ImageFilter, Error> {
    // Matrices are row-major and operate on straight color.
    #[rustfmt::skip]
    let values = match matrix {
        ColorMatrix::Matrix(values) => *values,
        ColorMatrix::Saturate(saturation) => {
            let [r, g, b] = [0.213, 0.715, 0.072].map(|weight| weight * (1.0 - saturation));
            [
                r + saturation,  g,               b,               0.0, 0.0,
                r,               g + saturation,  b,               0.0, 0.0,
                r,               g,               b + saturation,  0.0, 0.0,
                0.0,             0.0,             0.0,             1.0, 0.0,
            ]
        }
        ColorMatrix::HueRotate(degrees) => {
            let (sin, cos) = degrees.to_radians().sin_cos();
            [
                0.213 + 0.787 * cos - 0.213 * sin, 0.715 - 0.715 * cos - 0.715 * sin, 0.072 - 0.072 * cos + 0.928 * sin, 0.0, 0.0,
                0.213 - 0.213 * cos + 0.143 * sin, 0.715 + 0.285 * cos + 0.140 * sin, 0.072 - 0.072 * cos - 0.283 * sin, 0.0, 0.0,
                0.213 - 0.213 * cos - 0.787 * sin, 0.715 - 0.715 * cos + 0.715 * sin, 0.072 + 0.928 * cos + 0.072 * sin, 0.0, 0.0,
                0.0,                               0.0,                               0.0,                               1.0, 0.0,
            ]
        }
        ColorMatrix::LuminanceToAlpha => [
            0.0,    0.0,    0.0,    0.0, 0.0,
            0.0,    0.0,    0.0,    0.0, 0.0,
            0.0,    0.0,    0.0,    0.0, 0.0,
            0.2125, 0.7154, 0.0721, 0.0, 0.0,
        ],
    };
    let mut native_values = [0.0; 20];
    for (out, value) in native_values.iter_mut().zip(values) {
        *out = scalar(value)?;
    }
    let matrix = color_filters::matrix_row_major(&native_values, color_filters::Clamp::Yes);
    native(image_filters::color_filter(matrix, input, None))
}

pub(super) fn transfer(
    functions: &[TransferFunction; 4],
    input: sk::ImageFilter,
) -> Result<sk::ImageFilter, Error> {
    if functions
        .iter()
        .all(|function| matches!(function, TransferFunction::Identity))
    {
        return Ok(input);
    }

    // Evaluate the SVG curves at the input precision, including discrete table boundaries.
    let mut source = String::new();
    for (channel, function) in ['r', 'g', 'b', 'a'].into_iter().zip(functions) {
        let function = transfer_body(function)?;
        writeln!(source, "float transfer_{channel}(float x) {{ {function} }}")
            .expect("writing to a String is infallible");
    }
    source.push_str(
        "half4 main(half4 pm) {
            float4 c = pm.a > 0.0 ? float4(pm.rgb / pm.a, pm.a) : float4(0.0);
            c = clamp(float4(transfer_r(c.r), transfer_g(c.g), transfer_b(c.b),
                             transfer_a(c.a)), 0.0, 1.0);
            return half4(c.rgb * c.a, c.a);
        }",
    );
    let effect = sk::RuntimeEffect::make_for_color_filter(&source, None)
        .map_err(|_| Error::UnsupportedFilter)?;
    let filter = effect
        .make_color_filter(sk::Data::new_empty(), None)
        .ok_or(Error::UnsupportedFilter)?;
    native(image_filters::color_filter(filter, input, None))
}

fn transfer_body(function: &TransferFunction) -> Result<String, Error> {
    let literal = |value| Ok::<_, Error>(format!("({:e})", scalar(value)?));
    Ok(match function {
        TransferFunction::Identity => "return x;".into(),
        TransferFunction::Table(values) | TransferFunction::Discrete(values) => {
            if values.is_empty() {
                return Ok("return x;".into());
            }
            if let [only] = values.as_slice() {
                return Ok(format!("return {};", literal(*only)?));
            }
            let discrete = matches!(function, TransferFunction::Discrete(_));
            let segments = if discrete {
                values.len()
            } else {
                values.len() - 1
            };
            let values = values
                .iter()
                .copied()
                .map(literal)
                .collect::<Result<Vec<_>, _>>()?;
            // SkSL color filters can use this piecewise expression on both raster and GPU.
            let mut body = format!("float p = clamp(x, 0.0, 1.0) * {segments}.0;");
            for (index, pair) in values.windows(2).enumerate() {
                let end = index + 1;
                let value = if discrete {
                    pair[0].clone()
                } else {
                    format!("mix({}, {}, p - {index}.0)", pair[0], pair[1])
                };
                write!(body, "if (p < {end}.0) {{ return {value}; }}")
                    .expect("writing to a String is infallible");
            }
            write!(
                body,
                "return {};",
                values.last().expect("table is nonempty")
            )
            .expect("writing to a String is infallible");
            body
        }
        TransferFunction::Linear { slope, intercept } => {
            format!(
                "return {} * x + {};",
                literal(*slope)?,
                literal(*intercept)?
            )
        }
        TransferFunction::Gamma {
            amplitude,
            exponent,
            offset,
        } => {
            let a = literal(*amplitude)?;
            let e = literal(*exponent)?;
            let b = literal(*offset)?;
            if *amplitude == 0.0 {
                format!("return {b};")
            } else if *exponent == 0.0 {
                format!("return {a} + {b};")
            } else if *exponent < 0.0 {
                let zero = if *amplitude > 0.0 { "1.0" } else { "0.0" };
                format!("return x <= 0.0 ? {zero} : {a} * pow(x, {e}) + {b};")
            } else {
                format!("return {a} * pow(max(x, 0.0), {e}) + {b};")
            }
        }
    })
}
