// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Unsupported-feature reporting for SVG rendering.

use alloc::{string::String, vec::Vec};

/// Kind of SVG feature that `svg_imaging` could not represent faithfully.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnsupportedFeatureKind {
    /// Clip paths that require unsupported semantics.
    ClipPath,
    /// A mask feature could not be lowered.
    Mask,
    /// SVG filter graphs are not lowered yet.
    Filter,
    /// Raster image nodes that failed to decode.
    Image,
    /// Text nodes that remain after `usvg` normalization are not lowered yet.
    Text,
    /// Pattern paints are not lowered yet.
    PatternPaint,
}

/// A single unsupported SVG feature encountered while lowering a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsupportedFeature {
    /// The unsupported feature kind.
    pub kind: UnsupportedFeatureKind,
    /// SVG node id, when the source node had one.
    pub node_id: Option<String>,
}

/// Diagnostics collected while rendering an SVG document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenderReport {
    /// Unsupported or approximated features encountered while lowering.
    pub unsupported_features: Vec<UnsupportedFeature>,
}

impl RenderReport {
    /// Returns `true` when the document lowered without any unsupported features.
    #[must_use]
    pub fn is_fully_supported(&self) -> bool {
        self.unsupported_features.is_empty()
    }

    pub(crate) fn push(
        &mut self,
        kind: UnsupportedFeatureKind,
        node_id: Option<impl Into<String>>,
    ) {
        self.unsupported_features.push(UnsupportedFeature {
            kind,
            node_id: node_id.map(Into::into),
        });
    }
}
