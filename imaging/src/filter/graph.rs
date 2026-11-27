// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{boxed::Box, vec::Vec};
use kurbo::{Rect, Vec2};
use peniko::{Color, Mix};

use crate::record::Scene;

use super::Filter;

/// Filters for a borrowed isolated group.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FilterEffectsRef<'a> {
    /// Simple effects applied sequentially to the group content.
    Chain(&'a [Filter]),
    /// A graph evaluated against the group's content.
    Graph(&'a FilterGraph),
}

impl FilterEffectsRef<'_> {
    /// Returns whether there are no effects to apply.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        matches!(self, Self::Chain(filters) if filters.is_empty())
    }

    /// Retain these effects in an owned group.
    #[must_use]
    pub fn to_owned(self) -> FilterEffects {
        match self {
            Self::Chain(filters) => FilterEffects::Chain(filters.to_vec()),
            Self::Graph(graph) => FilterEffects::Graph(Box::new(graph.clone())),
        }
    }
}

/// Filters retained by an isolated group.
#[derive(Clone, Debug, PartialEq)]
pub enum FilterEffects {
    /// Simple effects applied sequentially to the group content.
    Chain(Vec<Filter>),
    /// A graph evaluated against the group's content.
    Graph(Box<FilterGraph>),
}

impl FilterEffects {
    /// Borrow these effects for streaming or replay.
    #[must_use]
    pub fn as_ref(&self) -> FilterEffectsRef<'_> {
        match self {
            Self::Chain(filters) => FilterEffectsRef::Chain(filters),
            Self::Graph(graph) => FilterEffectsRef::Graph(graph),
        }
    }
}

impl Default for FilterEffects {
    fn default() -> Self {
        Self::Chain(Vec::new())
    }
}

/// An ordered filter graph with an explicit output.
///
/// The group supplies an isolated source image, clipped to the graph region. Primitives can
/// read that image, its alpha, or results earlier in the graph. The graph output is clipped
/// to the region before the group mask, isolated clip, and compositing are applied.
///
/// Rectangles, geometric parameters, and embedded scenes use the same filter-local coordinates.
/// [`crate::GroupRef::filter_transform`] maps that coordinate system to the destination.
#[derive(Clone, Debug, PartialEq)]
pub struct FilterGraph {
    /// Overall filter region, in filter-local coordinates.
    pub region: Rect,
    /// Primitives in dependency order. The index of each primitive defines its result ID.
    pub primitives: Vec<FilterPrimitive>,
    /// Selected output of the graph.
    pub output: FilterInput,
}

/// The graph-local index of a primitive's result.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FilterId(pub usize);

/// An image available to a primitive or selectable as the graph output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FilterInput {
    /// The unfiltered, isolated group image.
    SourceGraphic,
    /// The alpha channel of the isolated group image, with black RGB channels.
    SourceAlpha,
    /// A primitive result from this graph.
    Result(FilterId),
}

/// Color space used for operations on RGB channels within a primitive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FilterColorSpace {
    /// Nonlinear sRGB.
    Srgb,
    /// Linear-light sRGB.
    #[default]
    LinearRgb,
}

/// One filter operation and the bounds and color space of its result.
#[derive(Clone, Debug, PartialEq)]
pub struct FilterPrimitive {
    /// Optional output bounds, in filter-local coordinates.
    ///
    /// `None` uses the primitive's natural bounds derived from its inputs and parameters,
    /// bounded by the overall filter region. Primitives without finite natural bounds use the
    /// filter region. A tile repeats its input's own subregion across the tile's output.
    pub subregion: Option<Rect>,
    /// Working color space. Inputs from another color space are converted for the operation.
    pub color_space: FilterColorSpace,
    /// Operation and its input connections.
    pub kind: FilterPrimitiveKind,
}

/// Strongly typed filter operations and their input connections.
#[derive(Clone, Debug, PartialEq)]
pub enum FilterPrimitiveKind {
    /// Blend a source against a backdrop (`feBlend`).
    Blend {
        /// Upper image.
        source: FilterInput,
        /// Lower image.
        backdrop: FilterInput,
        /// Blend function, with source-over compositing.
        mode: Mix,
    },
    /// Transform the unpremultiplied color channels (`feColorMatrix`).
    ColorMatrix {
        /// Image to transform.
        input: FilterInput,
        /// Color transform.
        matrix: ColorMatrix,
    },
    /// Remap the unpremultiplied color and alpha components (`feComponentTransfer`).
    ComponentTransfer {
        /// Image to remap.
        input: FilterInput,
        /// Transfer functions in red, green, blue, alpha order.
        functions: [TransferFunction; 4],
    },
    /// Composite a source against a backdrop (`feComposite`).
    Composite {
        /// Upper image.
        source: FilterInput,
        /// Lower image.
        backdrop: FilterInput,
        /// Compositing operator.
        operator: CompositeOperator,
    },
    /// Convolve an image with a kernel (`feConvolveMatrix`).
    ConvolveMatrix {
        /// Image to sample.
        input: FilterInput,
        /// Kernel columns and rows.
        order: [u32; 2],
        /// Zero-based kernel column and row aligned with the output pixel.
        target: [u32; 2],
        /// Row-major kernel weights.
        kernel: Vec<f64>,
        /// Nonzero divisor applied to the weighted sum.
        divisor: f64,
        /// Value added after division.
        bias: f64,
        /// Sampling outside the input bounds.
        edge_mode: EdgeMode,
        /// Copy alpha from the source instead of convolving it.
        preserve_alpha: bool,
    },
    /// Shade the input's alpha as a height map with diffuse lighting (`feDiffuseLighting`).
    DiffuseLighting {
        /// Input alpha used to compute the surface normals.
        input: FilterInput,
        /// Scale from normalized alpha to surface height.
        surface_scale: f64,
        /// Diffuse reflection coefficient.
        diffuse_constant: f64,
        /// Light color.
        lighting_color: Color,
        /// Light configuration.
        light: LightSource,
    },
    /// Displace one image by channels of another (`feDisplacementMap`).
    DisplacementMap {
        /// Image to sample.
        input: FilterInput,
        /// Image providing the displacement channels.
        displacement: FilterInput,
        /// Scale factor for the displacement.
        scale: f64,
        /// Displacement channel for the X axis.
        x_channel: ColorChannel,
        /// Displacement channel for the Y axis.
        y_channel: ColorChannel,
    },
    /// Put a blurred, colored copy of the input alpha behind the input (`feDropShadow`).
    DropShadow {
        /// Image casting the shadow.
        input: FilterInput,
        /// Shadow offset in filter-local coordinates.
        offset: Vec2,
        /// Gaussian standard deviations along the X and Y axes.
        std_deviation: Vec2,
        /// Shadow color, including opacity.
        color: Color,
    },
    /// Fill the output with a solid color (`feFlood`).
    Flood {
        /// Fill color, including opacity.
        color: Color,
    },
    /// Blur an image with a Gaussian kernel (`feGaussianBlur`).
    GaussianBlur {
        /// Image to blur.
        input: FilterInput,
        /// Gaussian standard deviations along the X and Y axes.
        std_deviation: Vec2,
    },
    /// Draw retained vector or raster content into a primitive (`feImage`).
    Image {
        /// Scene authored in filter-local coordinates, using the same transform as the graph.
        scene: Box<Scene>,
    },
    /// Composite images source-over in the supplied bottom-to-top order (`feMerge`).
    Merge {
        /// Images to merge. An empty list has a transparent output.
        inputs: Vec<FilterInput>,
    },
    /// Erode or dilate an image (`feMorphology`).
    Morphology {
        /// Image to sample.
        input: FilterInput,
        /// Sampling operation.
        operator: MorphologyOperator,
        /// Sampling radii along the X and Y axes.
        radius: Vec2,
    },
    /// Translate an image (`feOffset`).
    Offset {
        /// Image to translate.
        input: FilterInput,
        /// Translation vector in filter-local coordinates.
        offset: Vec2,
    },
    /// Shade the input's alpha as a height map with specular lighting (`feSpecularLighting`).
    SpecularLighting {
        /// Input alpha used to compute the surface normals.
        input: FilterInput,
        /// Scale from normalized alpha to surface height.
        surface_scale: f64,
        /// Specular reflection coefficient.
        specular_constant: f64,
        /// Highlight exponent, between 1 and 128 inclusive.
        specular_exponent: f64,
        /// Light color.
        lighting_color: Color,
        /// Light configuration.
        light: LightSource,
    },
    /// Repeat the input subregion to fill the output (`feTile`).
    Tile {
        /// Image to repeat.
        input: FilterInput,
    },
    /// Generate procedural Perlin noise (`feTurbulence`).
    Turbulence {
        /// Base frequencies along the X and Y axes.
        base_frequency: Vec2,
        /// Number of noise octaves.
        num_octaves: u32,
        /// Seed of the noise generator.
        seed: i32,
        /// Adjust the frequencies to make the result seamless at the output bounds.
        stitch_tiles: bool,
        /// Form of the generated noise.
        kind: TurbulenceKind,
    },
}

impl FilterPrimitiveKind {
    /// Iterate over the images read by this operation in argument order.
    pub fn inputs(&self) -> impl Iterator<Item = FilterInput> + '_ {
        let (fixed, variable) = match self {
            Self::Blend {
                source, backdrop, ..
            }
            | Self::Composite {
                source, backdrop, ..
            } => ([Some(*source), Some(*backdrop)], &[][..]),
            Self::DisplacementMap {
                input,
                displacement,
                ..
            } => ([Some(*input), Some(*displacement)], &[][..]),
            Self::ColorMatrix { input, .. }
            | Self::ComponentTransfer { input, .. }
            | Self::ConvolveMatrix { input, .. }
            | Self::DiffuseLighting { input, .. }
            | Self::DropShadow { input, .. }
            | Self::GaussianBlur { input, .. }
            | Self::Morphology { input, .. }
            | Self::Offset { input, .. }
            | Self::SpecularLighting { input, .. }
            | Self::Tile { input } => ([Some(*input), None], &[][..]),
            Self::Merge { inputs } => ([None; 2], inputs.as_slice()),
            Self::Flood { .. } | Self::Image { .. } | Self::Turbulence { .. } => {
                ([None; 2], &[][..])
            }
        };
        fixed.into_iter().flatten().chain(variable.iter().copied())
    }
}

/// Transform applied to a pixel's unpremultiplied color and alpha.
#[derive(Clone, Debug, PartialEq)]
pub enum ColorMatrix {
    /// Row-major 4×5 matrix. Its final column holds the additive RGBA terms.
    Matrix([f64; 20]),
    /// Multiply the color saturation by this factor.
    Saturate(f64),
    /// Rotate the color's hue by this angle in degrees.
    HueRotate(f64),
    /// Transfer RGB luminance to alpha and set RGB to zero.
    LuminanceToAlpha,
}

/// Mapping for one component of an unpremultiplied RGBA pixel.
#[derive(Clone, Debug, PartialEq)]
pub enum TransferFunction {
    /// Keep the input component.
    Identity,
    /// Linearly interpolate the evenly spaced samples. An empty table is the identity.
    Table(Vec<f64>),
    /// Select a sample from equally sized intervals. An empty table is the identity.
    Discrete(Vec<f64>),
    /// Apply a linear function.
    Linear {
        /// Factor multiplying the input.
        slope: f64,
        /// Offset added to the scaled input.
        intercept: f64,
    },
    /// Apply a power function.
    Gamma {
        /// Factor multiplying the exponentiated input.
        amplitude: f64,
        /// Power applied to the input.
        exponent: f64,
        /// Offset added to the scaled result.
        offset: f64,
    },
}

/// Operator for compositing two filter inputs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CompositeOperator {
    /// Source over backdrop.
    Over,
    /// Source where the backdrop is present.
    In,
    /// Source outside the backdrop.
    Out,
    /// Source over the backdrop in the backdrop's coverage.
    Atop,
    /// Source and backdrop outside their intersection.
    Xor,
    /// Add the source and backdrop, clamping the result.
    Lighter,
    /// Compute `k1 * source * backdrop + k2 * source + k3 * backdrop + k4` per component.
    Arithmetic {
        /// Coefficient of the product.
        k1: f64,
        /// Coefficient of the source.
        k2: f64,
        /// Coefficient of the backdrop.
        k3: f64,
        /// Constant offset.
        k4: f64,
    },
}

/// Sampling at coordinates outside the input subregion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeMode {
    /// Sample transparent black.
    None,
    /// Repeat the nearest edge sample.
    Duplicate,
    /// Wrap coordinates into the subregion.
    Wrap,
}

/// A single color or alpha channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorChannel {
    /// Red.
    Red,
    /// Green.
    Green,
    /// Blue.
    Blue,
    /// Alpha.
    Alpha,
}

/// Light geometry for a lighting primitive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightSource {
    /// Light arriving from a fixed direction.
    Distant {
        /// Direction angle on the XY plane, in degrees.
        azimuth: f64,
        /// Direction angle from the XY plane towards the Z axis, in degrees.
        elevation: f64,
    },
    /// Light emitted in all directions from a point.
    Point {
        /// Light origin, in XYZ order.
        position: [f64; 3],
    },
    /// Light directed from a point toward a target.
    Spot {
        /// Light origin, in XYZ order.
        position: [f64; 3],
        /// Aim point, in XYZ order.
        points_at: [f64; 3],
        /// Exponent controlling the focus of the light.
        specular_exponent: f64,
        /// Optional outer cone angle in degrees.
        limiting_cone_angle: Option<f64>,
    },
}

/// Sampling operation used by morphology.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MorphologyOperator {
    /// Take each component's minimum within the sampling window.
    Erode,
    /// Take each component's maximum within the sampling window.
    Dilate,
}

/// Noise function used by a turbulence primitive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurbulenceKind {
    /// Fractal noise.
    FractalNoise,
    /// Turbulence noise.
    Turbulence,
}
