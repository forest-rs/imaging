// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `svg_imaging`: parse SVG with `usvg` and render it through `imaging`.
//!
//! The crate intentionally sits between SVG parsing and backend rendering:
//! - [`usvg`] parses and normalizes SVG input.
//! - `svg_imaging` lowers supported SVG semantics into a crate-local render plan.
//! - [`imaging`] executes that plan through [`imaging::Painter`].
//!
//! This crate is explicit about gaps. Unsupported features are reported in [`RenderReport`]
//! instead of being silently discarded.
//!
//! Current support includes path fills and strokes, gradients, paint order, isolated group
//! compositing, clip paths including referenced clip-path chains, masks lowered through reusable
//! `imaging` mask definitions, text lowered through `usvg`'s flattened vector output, nested SVG
//! `<image>` nodes, and raster PNG/JPEG/GIF/WebP `<image>` nodes. Filters and pattern paints are
//! still reported as unsupported.
//!
//! Text rendering depends on the parse options' font database. `usvg::Options::default()` starts
//! with an empty font database, so callers that need text should load fonts into
//! [`ParseOptions`] before parsing.
//!
//! `svg_imaging` is a `no_std` plus `alloc` crate. Its optional `std` feature only enables
//! additional `std` integration and dependency features, and the current dependency stack still
//! effectively requires `std` today.
//!
//! ```rust
//! use imaging::{Painter, record::Scene};
//! use svg_imaging::{ParseOptions, RenderOptions, SvgDocument};
//!
//! let svg = br#"
//!     <svg xmlns='http://www.w3.org/2000/svg' width='16' height='16'>
//!         <rect x='1' y='1' width='14' height='14' fill='#3465a4'/>
//!     </svg>
//! "#;
//!
//! let document = SvgDocument::from_data(svg, &ParseOptions::default())?;
//! let mut scene = Scene::new();
//! let mut painter = Painter::new(&mut scene);
//! let report = document.render(&mut painter, &RenderOptions::default())?;
//!
//! assert!(report.unsupported_features.is_empty());
//! assert!(!scene.commands().is_empty());
//! # Ok::<(), svg_imaging::Error>(())
//! ```

#![no_std]
extern crate alloc;
#[cfg(any(test, feature = "std"))]
extern crate std;

mod diagnostics;
mod document;
mod emit;
mod lower;
mod plan;

pub use diagnostics::{RenderReport, UnsupportedFeature, UnsupportedFeatureKind};
pub use document::{ParseOptions, RenderOptions, SvgDocument};

/// Error produced while parsing or rendering SVG content.
#[derive(Debug)]
pub enum Error {
    /// The source SVG could not be parsed by `usvg`.
    Parse(usvg::Error),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Parse(err) => write!(f, "failed to parse SVG: {err}"),
        }
    }
}

impl core::error::Error for Error {}

impl From<usvg::Error> for Error {
    fn from(value: usvg::Error) -> Self {
        Self::Parse(value)
    }
}
