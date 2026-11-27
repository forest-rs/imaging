// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::boxed::Box;
use core::fmt;
use kurbo::{Affine, Rect};

use crate::{record::ValidateError, validation::filter_is_valid};

use super::{
    ColorMatrix, CompositeOperator, Filter, FilterGraph, FilterId, FilterInput,
    FilterPrimitiveKind, LightSource, TransferFunction,
};

/// Invalid data in a filter graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilterGraphError {
    /// The overall filter region was invalid.
    InvalidRegion,
    /// The bounds of a primitive were invalid.
    InvalidSubregion(FilterId),
    /// A primitive referred to its own result or to a result that is unavailable.
    InvalidInput {
        /// Primitive reading the result.
        primitive: FilterId,
        /// Unavailable result.
        result: FilterId,
    },
    /// The selected output was unavailable.
    InvalidOutput(FilterId),
    /// A primitive's parameters were invalid.
    InvalidParameters(FilterId),
    /// The scene retained by an image primitive was invalid.
    InvalidImage {
        /// Image primitive with an invalid scene.
        primitive: FilterId,
        /// Error in the retained scene.
        error: Box<ValidateError>,
    },
}

impl fmt::Display for FilterGraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRegion => f.write_str("invalid filter region"),
            Self::InvalidSubregion(id) => write!(f, "invalid subregion for primitive {}", id.0),
            Self::InvalidInput { primitive, result } => {
                write!(
                    f,
                    "primitive {} references unavailable result {}",
                    primitive.0, result.0
                )
            }
            Self::InvalidOutput(id) => write!(f, "unavailable graph output {}", id.0),
            Self::InvalidParameters(id) => write!(f, "invalid parameters for primitive {}", id.0),
            Self::InvalidImage { primitive, .. } => {
                write!(
                    f,
                    "invalid retained image scene for primitive {}",
                    primitive.0
                )
            }
        }
    }
}

impl core::error::Error for FilterGraphError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::InvalidImage { error, .. } => Some(error.as_ref()),
            _ => None,
        }
    }
}

impl FilterGraph {
    /// Check regions, dependencies, primitive parameters, and retained image scenes.
    pub fn validate(&self) -> Result<(), FilterGraphError> {
        self.validate_transformed(Affine::IDENTITY)
    }

    pub(crate) fn validate_transformed(&self, transform: Affine) -> Result<(), FilterGraphError> {
        let rect_is_valid =
            |rect: Rect| rect_is_valid(rect) && rect_is_valid(transform.transform_rect_bbox(rect));
        if !rect_is_valid(self.region) {
            return Err(FilterGraphError::InvalidRegion);
        }
        for (index, primitive) in self.primitives.iter().enumerate() {
            let id = FilterId(index);
            if primitive.subregion.is_some_and(|rect| !rect_is_valid(rect)) {
                return Err(FilterGraphError::InvalidSubregion(id));
            }
            for input in primitive.kind.inputs() {
                if let FilterInput::Result(result) = input
                    && result.0 >= index
                {
                    return Err(FilterGraphError::InvalidInput {
                        primitive: id,
                        result,
                    });
                }
            }
            if !parameters_are_valid(&primitive.kind, transform) {
                return Err(FilterGraphError::InvalidParameters(id));
            }
            if let FilterPrimitiveKind::Image { scene } = &primitive.kind {
                scene
                    .validate()
                    .map_err(|error| FilterGraphError::InvalidImage {
                        primitive: id,
                        error: Box::new(error),
                    })?;
            }
        }
        if let FilterInput::Result(result) = self.output
            && result.0 >= self.primitives.len()
        {
            return Err(FilterGraphError::InvalidOutput(result));
        }
        Ok(())
    }
}

fn rect_is_valid(rect: Rect) -> bool {
    rect.is_finite()
        && rect.x0 <= rect.x1
        && rect.y0 <= rect.y1
        && rect.width().is_finite()
        && rect.height().is_finite()
}

fn parameters_are_valid(kind: &FilterPrimitiveKind, transform: Affine) -> bool {
    use FilterPrimitiveKind as Kind;

    let color_is_finite = |color: &peniko::Color| color.components.into_iter().all(f32::is_finite);
    match kind {
        Kind::Blend { .. } | Kind::Image { .. } | Kind::Merge { .. } | Kind::Tile { .. } => true,
        Kind::ColorMatrix { matrix, .. } => match matrix {
            ColorMatrix::Matrix(values) => values.iter().copied().all(f64::is_finite),
            ColorMatrix::Saturate(value) => value.is_finite() && *value >= 0.0,
            ColorMatrix::HueRotate(value) => value.is_finite(),
            ColorMatrix::LuminanceToAlpha => true,
        },
        Kind::ComponentTransfer { functions, .. } => functions.iter().all(transfer_is_valid),
        Kind::Composite { operator, .. } => match *operator {
            CompositeOperator::Arithmetic { k1, k2, k3, k4 } => {
                [k1, k2, k3, k4].into_iter().all(f64::is_finite)
            }
            _ => true,
        },
        Kind::ConvolveMatrix {
            order: [columns, rows],
            target: [x, y],
            kernel,
            divisor,
            bias,
            ..
        } => {
            *x < *columns
                && *y < *rows
                && usize::try_from(u64::from(*columns) * u64::from(*rows)) == Ok(kernel.len())
                && kernel.iter().copied().all(f64::is_finite)
                && divisor.is_finite()
                && *divisor != 0.0
                && bias.is_finite()
        }
        Kind::DiffuseLighting {
            surface_scale,
            diffuse_constant,
            lighting_color,
            light,
            ..
        } => {
            surface_scale.is_finite()
                && diffuse_constant.is_finite()
                && color_is_finite(lighting_color)
                && light_is_valid(light)
        }
        Kind::DisplacementMap { scale, .. } => {
            filter_is_valid(&Filter::blur(scale.abs()), transform)
        }
        Kind::DropShadow {
            offset,
            std_deviation,
            color,
            ..
        } => {
            color_is_finite(color)
                && filter_is_valid(
                    &Filter::DropShadow {
                        dx: offset.x,
                        dy: offset.y,
                        std_deviation_x: std_deviation.x,
                        std_deviation_y: std_deviation.y,
                        color: *color,
                    },
                    transform,
                )
        }
        Kind::Flood { color } => color_is_finite(color),
        Kind::GaussianBlur { std_deviation, .. }
        | Kind::Morphology {
            radius: std_deviation,
            ..
        } => filter_is_valid(
            &Filter::blur_xy(std_deviation.x, std_deviation.y),
            transform,
        ),
        Kind::Offset { offset, .. } => {
            filter_is_valid(&Filter::offset(offset.x, offset.y), transform)
        }
        Kind::SpecularLighting {
            surface_scale,
            specular_constant,
            specular_exponent,
            lighting_color,
            light,
            ..
        } => {
            surface_scale.is_finite()
                && specular_constant.is_finite()
                && (1.0..=128.0).contains(specular_exponent)
                && color_is_finite(lighting_color)
                && light_is_valid(light)
        }
        Kind::Turbulence { base_frequency, .. } => {
            base_frequency.is_finite() && base_frequency.x >= 0.0 && base_frequency.y >= 0.0
        }
    }
}

fn transfer_is_valid(function: &TransferFunction) -> bool {
    match function {
        TransferFunction::Identity => true,
        TransferFunction::Table(values) | TransferFunction::Discrete(values) => {
            values.iter().copied().all(f64::is_finite)
        }
        TransferFunction::Linear { slope, intercept } => slope.is_finite() && intercept.is_finite(),
        TransferFunction::Gamma {
            amplitude,
            exponent,
            offset,
        } => amplitude.is_finite() && exponent.is_finite() && offset.is_finite(),
    }
}

fn light_is_valid(light: &LightSource) -> bool {
    match *light {
        LightSource::Distant { azimuth, elevation } => azimuth.is_finite() && elevation.is_finite(),
        LightSource::Point { position } => position.into_iter().all(f64::is_finite),
        LightSource::Spot {
            position,
            points_at,
            specular_exponent,
            limiting_cone_angle,
        } => {
            position.into_iter().chain(points_at).all(f64::is_finite)
                && specular_exponent.is_finite()
                && specular_exponent >= 0.0
                && limiting_cone_angle.is_none_or(f64::is_finite)
        }
    }
}
