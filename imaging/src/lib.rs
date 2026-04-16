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
//! Optional scoped context annotations can be attached during painting with
//! [`Painter::with_context`] or the [`with_context!`] macro. Recorded scenes intern context labels
//! and file names only when contexts are actually emitted.
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
//! use peniko::Color;
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
//!     fn glyph_run(
//!         &mut self,
//!         _draw: GlyphRunRef<'_>,
//!         _glyphs: &mut dyn Iterator<Item = imaging::record::Glyph>,
//!     ) {}
//!
//!     fn blurred_rounded_rect(&mut self, _draw: BlurredRoundedRect) {}
//! }
//!
//! let mut sink = CountingSink::default();
//!
//! {
//!     let mut painter = Painter::new(&mut sink);
//!     painter.fill_rect(Rect::new(0.0, 0.0, 64.0, 64.0), Color::from_rgb8(0x2a, 0x6f, 0xdb));
//!     painter.with_fill_clip(Rect::new(8.0, 8.0, 56.0, 56.0), |p| {
//!         p.fill_rect(Rect::new(16.0, 16.0, 48.0, 48.0), Color::from_rgb8(0x2a, 0x6f, 0xdb));
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
//! validate, diagnose, compare in tests, and replay into another sink later.
//!
//! ```rust
//! use imaging::{record, Painter};
//! use kurbo::Rect;
//! use peniko::Color;
//!
//! let mut scene = record::Scene::new();
//!
//! {
//!     let mut painter = Painter::new(&mut scene);
//!     painter.fill_rect(Rect::new(0.0, 0.0, 64.0, 64.0), Color::from_rgb8(0x12, 0x34, 0x56));
//! }
//!
//! scene.validate()?;
//! assert!(scene.diagnose().is_empty());
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

use alloc::vec::Vec;
use kurbo::{Affine, Rect};
use peniko::BlendMode;

pub mod diagnostics;
mod paint;
mod painter;
pub mod record;
pub mod render;
pub mod validation;

pub use paint::{
    AppliedMaskRef, ClipRef, ContextRef, DrawRef, FillRef, GeometryRef, GlyphRunRef, GroupRef,
    MaskRef, PaintSink, SourceLocationRef, StrokeRef,
};
pub use painter::{FillBuilder, GlyphRunBuilder, PaintShape, Painter, StrokeBuilder};
pub use render::{
    ImageBufferFormat, ImageBufferTarget, ImageRenderer, ImageRendererError, RenderSource,
};

pub use kurbo;
pub use peniko;

/// Run a painter closure inside a labeled context scope using the call-site source location.
///
/// This is shorthand for:
///
/// ```rust
/// # use imaging::{Painter, SourceLocationRef, record};
/// # let mut scene = record::Scene::new();
/// # let mut painter = Painter::new(&mut scene);
/// painter.with_context(
///     "toolbar/button",
///     Some(SourceLocationRef::new(file!(), line!(), column!())),
///     |p| {
///         let _ = p;
///     },
/// );
/// ```
///
/// The macro has two forms:
///
/// ```rust
/// # use imaging::{record, Painter};
/// # let mut scene = record::Scene::new();
/// # let mut painter = Painter::new(&mut scene);
/// imaging::with_context!(painter, "toolbar/button", |p| {
///     let _ = p;
/// });
///
/// imaging::with_context!(
///     painter,
///     "toolbar/button",
///     Some(imaging::SourceLocationRef::new("widgets.rs", 7, 3)),
///     |p| {
///         let _ = p;
///     }
/// );
/// ```
#[macro_export]
macro_rules! with_context {
    ($painter:expr, $label:expr, |$p:ident| $body:block) => {{
        $painter.with_context(
            $label,
            Some($crate::SourceLocationRef::new(file!(), line!(), column!())),
            |$p| $body,
        )
    }};
    ($painter:expr, $label:expr, $source:expr, |$p:ident| $body:block) => {{ $painter.with_context($label, $source, |$p| $body) }};
}

/// Owned unpremultiplied RGBA8 image data returned by renderers.
///
/// This is an output type for readback and headless rendering results. It is not the shared image
/// asset type used for image brushes or other `imaging` inputs; those continue to use
/// [`peniko::ImageData`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Tightly packed RGBA8 pixels in row-major order.
    pub data: Vec<u8>,
}

impl RgbaImage {
    /// Create a zero-initialized RGBA8 image with the given dimensions.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: alloc::vec![0; rgba_image_len(width, height)],
        }
    }

    /// Resize the image, preserving existing bytes where possible.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.data.resize(rgba_image_len(width, height), 0);
    }
}

fn rgba_image_len(width: u32, height: u32) -> usize {
    width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .and_then(|bytes| usize::try_from(bytes).ok())
        .expect("RGBA image dimensions should fit in memory")
}

/// Normalized variable-font coordinate value.
pub type NormalizedCoord = i16;

/// How a mask scene modulates a masked content scene.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MaskMode {
    /// Use the mask scene's alpha channel.
    Alpha,
    /// Use the mask scene's luminance.
    ///
    /// This follows SVG/CSS masking behavior where premultiplied RGB contributes to the mask value.
    Luminance,
}

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

impl BlurredRoundedRect {
    pub(crate) fn prepend_transform(self, prefix: Affine) -> Self {
        Self {
            transform: prefix * self.transform,
            ..self
        }
    }
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
