// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `imaging`: backend-agnostic 2D imaging recording + streaming API.
//!
//! This crate has two primary public layers:
//! - [`PaintSink`] and [`Painter`] for borrowed command authoring and streaming.
//! - [`record`] for owned semantic recordings you can retain, validate, and replay.
//!
//! The root of the crate is intentionally focused on the streaming surface and the shared drawing
//! vocabulary. Retained scene data and low-level recording payloads live under [`record`].
//!
//! Use [`Painter`] with any [`PaintSink`] when authoring commands. Record into
//! [`record::Scene`] when you need backend-independent retention, validation, testing, or replay.
//!
//! Migration note: retained types that previously lived at the crate root now live under
//! [`record`], for example [`record::Scene`] and [`record::Draw`].
//!
//! The API is intentionally small and experimental; expect breaking changes while we iterate.

#![no_std]

extern crate alloc;

use kurbo::{Affine, Rect, Stroke};
use peniko::{BlendMode, Brush, Fill, Style};

mod paint;
mod painter;
pub mod record;
pub mod validation;

pub use paint::{
    ClipRef, DrawRef, FillRef, GeometryRef, GlyphRunRef, GroupRef, PaintSink, StrokeRef,
};
pub use painter::{FillBuilder, GlyphRunBuilder, Painter, StrokeBuilder};

/// Fill rule used by fills and fill-style clips.
pub type FillRule = Fill;

/// Stroke style used by strokes and stroke-style clips.
pub type StrokeStyle = Stroke;

/// Brush/paint used for fills and strokes.
///
/// This is currently a direct re-export of Peniko's brush type.
pub type Paint = Brush;

/// Glyph drawing style used by [`record::GlyphRun`].
pub type GlyphStyle = Style;

/// Normalized variable-font coordinate value.
pub type NormalizedCoord = i16;

/// Description of a filter effect applied to an isolated group.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Filter {
    /// Fill the group output with a solid color (aka `feFlood`).
    ///
    /// This ignores the group's source content. It is still affected by the group's isolated clip.
    Flood {
        /// Flood color.
        color: peniko::Color,
    },
    /// Gaussian blur with separate X/Y standard deviation values in user space.
    ///
    /// Backends should scale these values using the current transform when the filter is applied.
    Blur {
        /// Standard deviation along the X axis (in user space units).
        std_deviation_x: f32,
        /// Standard deviation along the Y axis (in user space units).
        std_deviation_y: f32,
    },
    /// Drop shadow under the source content.
    DropShadow {
        /// Shadow offset along the X axis (in user space units).
        dx: f32,
        /// Shadow offset along the Y axis (in user space units).
        dy: f32,
        /// Blur standard deviation along the X axis (in user space units).
        std_deviation_x: f32,
        /// Blur standard deviation along the Y axis (in user space units).
        std_deviation_y: f32,
        /// Shadow color.
        color: peniko::Color,
    },
    /// Translate the group output by a vector (aka `feOffset`).
    ///
    /// Offsets are specified in user space; backends should transform this vector using the current
    /// linear transform when the filter is applied.
    Offset {
        /// Offset along the X axis (in user space units).
        dx: f32,
        /// Offset along the Y axis (in user space units).
        dy: f32,
    },
}

impl Filter {
    /// Create a flood filter.
    #[inline]
    pub const fn flood(color: peniko::Color) -> Self {
        Self::Flood { color }
    }

    /// Create a uniform Gaussian blur filter.
    #[inline]
    pub const fn blur(sigma: f32) -> Self {
        Self::Blur {
            std_deviation_x: sigma,
            std_deviation_y: sigma,
        }
    }

    /// Create a Gaussian blur filter with separate X/Y sigma values.
    #[inline]
    pub const fn blur_xy(std_deviation_x: f32, std_deviation_y: f32) -> Self {
        Self::Blur {
            std_deviation_x,
            std_deviation_y,
        }
    }

    /// Create an offset/translation filter.
    #[inline]
    pub const fn offset(dx: f32, dy: f32) -> Self {
        Self::Offset { dx, dy }
    }
}

/// A solid-color rounded rectangle blurred with a gaussian filter.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BlurredRoundedRect {
    /// Geometry transform.
    pub transform: Affine,
    /// Unblurred rectangle bounds.
    pub rect: Rect,
    /// Solid color used by the blurred rectangle.
    pub color: peniko::Color,
    /// Uniform corner radius in user-space units.
    pub radius: f64,
    /// Gaussian standard deviation in user-space units.
    pub std_dev: f64,
    /// Per-draw compositing.
    pub composite: Composite,
}

/// Canvas-style compositing state.
///
/// This corresponds to HTML Canvas 2D's `globalCompositeOperation` (blend) plus `globalAlpha`
/// (alpha), applied per draw.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Composite {
    /// Blend mode used by the draw.
    pub blend: BlendMode,
    /// Alpha multiplier in `0..=1`.
    pub alpha: f32,
}

impl Composite {
    /// Create compositing state with a blend mode and alpha multiplier.
    #[inline]
    pub fn new(blend: BlendMode, alpha: f32) -> Self {
        Self {
            blend,
            alpha: alpha.clamp(0.0, 1.0),
        }
    }
}

impl Default for Composite {
    #[inline]
    fn default() -> Self {
        Self::new(BlendMode::default(), 1.0)
    }
}
