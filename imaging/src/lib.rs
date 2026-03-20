// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `imaging`: backend-agnostic 2D imaging recording + streaming API.
//!
//! `imaging` has two primary workflows:
//! - Painting: stream borrowed commands into any [`PaintSink`] with [`Painter`].
//! - Recording: retain an owned command stream in [`record::Scene`] for validation and replay.
//!
//! The root of the crate is intentionally focused on the streaming surface and the shared drawing
//! vocabulary. Retained scene data and low-level recording payloads live under [`record`].
//!
//! # Painting
//!
//! Use [`Painter`] when you want to stream commands directly into a renderer, backend-native
//! recorder, validator, or any other [`PaintSink`] implementation without first constructing owned
//! retained payloads.
//!
//! ```rust
//! use imaging::{
//!     BlurredRoundedRect, ClipRef, FillRef, GlyphRunRef, GroupRef, PaintSink, Painter, StrokeRef,
//! };
//! use kurbo::Rect;
//! use peniko::{Brush, Color};
//!
//! #[derive(Default)]
//! struct CountingSink {
//!     fills: usize,
//!     clips: usize,
//! }
//!
//! impl PaintSink for CountingSink {
//!     fn push_clip(&mut self, _clip: ClipRef<'_>) {
//!         self.clips += 1;
//!     }
//!
//!     fn pop_clip(&mut self) {}
//!
//!     fn push_group(&mut self, _group: GroupRef<'_>) {}
//!
//!     fn pop_group(&mut self) {}
//!
//!     fn fill(&mut self, _draw: FillRef<'_>) {
//!         self.fills += 1;
//!     }
//!
//!     fn stroke(&mut self, _draw: StrokeRef<'_>) {}
//!
//!     fn glyph_run(&mut self, _draw: GlyphRunRef<'_>) {}
//!
//!     fn blurred_rounded_rect(&mut self, _draw: BlurredRoundedRect) {}
//! }
//!
//! let paint = Brush::Solid(Color::from_rgb8(0x2a, 0x6f, 0xdb));
//! let mut sink = CountingSink::default();
//!
//! {
//!     let mut painter = Painter::new(&mut sink);
//!     painter.fill_rect(Rect::new(0.0, 0.0, 64.0, 64.0), &paint);
//!     painter.with_fill_clip(Rect::new(8.0, 8.0, 56.0, 56.0), |p| {
//!         p.fill_rect(Rect::new(16.0, 16.0, 48.0, 48.0), &paint);
//!     });
//! }
//!
//! assert_eq!(sink.fills, 2);
//! assert_eq!(sink.clips, 1);
//! ```
//!
//! # Recording
//!
//! Use [`record::Scene`] when you want an owned, backend-agnostic recording you can retain,
//! validate, compare in tests, and replay into another sink later.
//!
//! ```rust
//! use imaging::{record, Painter};
//! use kurbo::Rect;
//! use peniko::{Brush, Color};
//!
//! let paint = Brush::Solid(Color::from_rgb8(0x12, 0x34, 0x56));
//! let mut scene = record::Scene::new();
//!
//! {
//!     let mut painter = Painter::new(&mut scene);
//!     painter.fill_rect(Rect::new(0.0, 0.0, 64.0, 64.0), &paint);
//! }
//!
//! scene.validate()?;
//! assert_eq!(scene.commands().len(), 1);
//!
//! let mut replayed = record::Scene::new();
//! record::replay(&scene, &mut replayed);
//! assert_eq!(scene, replayed);
//! # Ok::<(), record::ValidateError>(())
//! ```
//!
//! Low-level retained payloads like [`record::Draw`], [`record::Clip`], and [`record::Group`] are
//! also public under [`record`] when you need exact control over the recorded representation.
//!
//! The API is intentionally small and experimental; expect breaking changes while we iterate.

#![no_std]

extern crate alloc;

use kurbo::{Affine, Rect};
use peniko::BlendMode;

mod paint;
mod painter;
pub mod record;
pub mod validation;

pub use paint::{
    ClipRef, DrawRef, FillRef, GeometryRef, GlyphRunRef, GroupRef, PaintSink, StrokeRef,
};
pub use painter::{FillBuilder, GlyphRunBuilder, Painter, StrokeBuilder};

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
