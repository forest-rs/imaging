// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Borrowed paint/streaming API.
//!
//! These types let callers stream imaging commands as borrowed data into a [`PaintSink`] without
//! first constructing owned IR payloads. [`Scene`] remains the owned semantic recording format.

use kurbo::{Affine, BezPath, Rect, RoundedRect, Shape as _};

use crate::{
    BlurredRoundedRect, Clip, ClipId, Command, Composite, Draw, DrawId, FillRule, Filter, Geometry,
    Glyph, GlyphRun, GlyphStyle, Group, GroupId, NormalizedCoord, Paint, Scene, StrokeStyle,
};

/// Borrowed geometry payload.
#[derive(Clone, Debug, PartialEq)]
pub enum GeometryRef<'a> {
    /// Axis-aligned rectangle.
    Rect(Rect),
    /// Axis-aligned rounded rectangle.
    RoundedRect(RoundedRect),
    /// General path.
    Path(&'a BezPath),
    /// Owned general path.
    OwnedPath(BezPath),
}

impl<'a> GeometryRef<'a> {
    /// Convert a borrowed geometry payload into an owned [`Geometry`].
    #[must_use]
    pub fn to_owned(self) -> Geometry {
        match self {
            Self::Rect(rect) => Geometry::Rect(rect),
            Self::RoundedRect(rounded_rect) => Geometry::RoundedRect(rounded_rect),
            Self::Path(path) => Geometry::Path(path.clone()),
            Self::OwnedPath(path) => Geometry::Path(path),
        }
    }

    /// Convert a geometry payload into a path.
    #[must_use]
    pub fn to_path(self, tolerance: f64) -> BezPath {
        match self {
            Self::Rect(rect) => rect.to_path(tolerance),
            Self::RoundedRect(rounded_rect) => rounded_rect.to_path(tolerance),
            Self::Path(path) => path.clone(),
            Self::OwnedPath(path) => path,
        }
    }
}

impl<'a> From<Rect> for GeometryRef<'a> {
    fn from(rect: Rect) -> Self {
        Self::Rect(rect)
    }
}

impl<'a> From<RoundedRect> for GeometryRef<'a> {
    fn from(rounded_rect: RoundedRect) -> Self {
        Self::RoundedRect(rounded_rect)
    }
}

impl<'a> From<&'a BezPath> for GeometryRef<'a> {
    fn from(path: &'a BezPath) -> Self {
        Self::Path(path)
    }
}

impl<'a> From<BezPath> for GeometryRef<'a> {
    fn from(path: BezPath) -> Self {
        Self::OwnedPath(path)
    }
}

impl<'a> From<Geometry> for GeometryRef<'a> {
    fn from(geometry: Geometry) -> Self {
        match geometry {
            Geometry::Rect(rect) => Self::Rect(rect),
            Geometry::RoundedRect(rounded_rect) => Self::RoundedRect(rounded_rect),
            Geometry::Path(path) => Self::OwnedPath(path),
        }
    }
}

impl<'a> From<&'a Geometry> for GeometryRef<'a> {
    fn from(geometry: &'a Geometry) -> Self {
        geometry.as_ref()
    }
}

/// Borrowed clip payload.
#[derive(Clone, Debug, PartialEq)]
pub enum ClipRef<'a> {
    /// Clip to the fill region of a shape.
    Fill {
        /// Transform applied to the clip shape.
        transform: Affine,
        /// Shape used to define the clip region.
        shape: GeometryRef<'a>,
        /// Fill rule used to determine the interior for path clips.
        fill_rule: FillRule,
    },
    /// Clip to the filled outline of a stroked shape.
    Stroke {
        /// Transform applied to the clip shape.
        transform: Affine,
        /// Shape whose stroked outline defines the clip region.
        shape: GeometryRef<'a>,
        /// Stroke style used to compute the outline.
        stroke: &'a StrokeStyle,
    },
}

impl<'a> ClipRef<'a> {
    /// Create a fill-style clip with the default non-zero fill rule.
    #[must_use]
    pub fn fill(shape: impl Into<GeometryRef<'a>>) -> Self {
        Self::Fill {
            transform: Affine::IDENTITY,
            shape: shape.into(),
            fill_rule: FillRule::NonZero,
        }
    }

    /// Create a fill-style clip with an explicit fill rule.
    #[must_use]
    pub fn fill_with_rule(shape: impl Into<GeometryRef<'a>>, fill_rule: FillRule) -> Self {
        Self::Fill {
            transform: Affine::IDENTITY,
            shape: shape.into(),
            fill_rule,
        }
    }

    /// Create a stroke-style clip.
    #[must_use]
    pub fn stroke(shape: impl Into<GeometryRef<'a>>, stroke: &'a StrokeStyle) -> Self {
        Self::Stroke {
            transform: Affine::IDENTITY,
            shape: shape.into(),
            stroke,
        }
    }

    /// Convert a borrowed clip payload into an owned [`Clip`].
    #[must_use]
    pub fn to_owned(self) -> Clip {
        match self {
            Self::Fill {
                transform,
                shape,
                fill_rule,
            } => Clip::Fill {
                transform,
                shape: shape.to_owned(),
                fill_rule,
            },
            Self::Stroke {
                transform,
                shape,
                stroke,
            } => Clip::Stroke {
                transform,
                shape: shape.to_owned(),
                stroke: stroke.clone(),
            },
        }
    }
}

/// Borrowed isolated group payload.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupRef<'a> {
    /// Optional isolated clip applied to the group result.
    pub clip: Option<ClipRef<'a>>,
    /// Optional filter chain applied to the group result before compositing.
    pub filters: &'a [Filter],
    /// Compositing parameters used when merging the group into its parent.
    pub composite: Composite,
}

impl<'a> GroupRef<'a> {
    /// Create a group with default compositing and no isolated clip or filters.
    #[must_use]
    pub fn new() -> Self {
        Self {
            clip: None,
            filters: &[],
            composite: Composite::default(),
        }
    }

    /// Set the isolated clip applied to the group result.
    #[must_use]
    pub fn with_clip(mut self, clip: ClipRef<'a>) -> Self {
        self.clip = Some(clip);
        self
    }

    /// Set the filter chain applied to the group result.
    #[must_use]
    pub fn with_filters(mut self, filters: &'a [Filter]) -> Self {
        self.filters = filters;
        self
    }

    /// Set the compositing parameters used when merging the group into its parent.
    #[must_use]
    pub fn with_composite(mut self, composite: Composite) -> Self {
        self.composite = composite;
        self
    }

    /// Convert a borrowed group payload into an owned [`Group`].
    #[must_use]
    pub fn to_owned(self) -> Group {
        Group {
            clip: self.clip.map(ClipRef::to_owned),
            filters: self.filters.to_vec(),
            composite: self.composite,
        }
    }
}

impl<'a> Default for GroupRef<'a> {
    fn default() -> Self {
        Self::new()
    }
}

/// Borrowed fill draw payload.
#[derive(Clone, Debug, PartialEq)]
pub struct FillRef<'a> {
    /// Geometry transform.
    pub transform: Affine,
    /// Fill rule used to determine inside/outside for paths.
    pub fill_rule: FillRule,
    /// Paint used by this draw.
    pub paint: &'a Paint,
    /// Optional paint-space transform (for gradients/images).
    pub paint_transform: Option<Affine>,
    /// Geometry to fill.
    pub shape: GeometryRef<'a>,
    /// Per-draw compositing.
    pub composite: Composite,
}

impl<'a> FillRef<'a> {
    /// Create a fill draw with default transform, paint transform, fill rule, and composite state.
    #[must_use]
    pub fn new(shape: impl Into<GeometryRef<'a>>, paint: &'a Paint) -> Self {
        Self {
            transform: Affine::IDENTITY,
            fill_rule: FillRule::NonZero,
            paint,
            paint_transform: None,
            shape: shape.into(),
            composite: Composite::default(),
        }
    }

    /// Set the geometry transform.
    #[must_use]
    pub fn transform(mut self, transform: Affine) -> Self {
        self.transform = transform;
        self
    }

    /// Set the fill rule.
    #[must_use]
    pub fn fill_rule(mut self, fill_rule: FillRule) -> Self {
        self.fill_rule = fill_rule;
        self
    }

    /// Set the optional paint-space transform.
    #[must_use]
    pub fn paint_transform(mut self, paint_transform: Option<Affine>) -> Self {
        self.paint_transform = paint_transform;
        self
    }

    /// Set the per-draw compositing state.
    #[must_use]
    pub fn composite(mut self, composite: Composite) -> Self {
        self.composite = composite;
        self
    }

    /// Convert a borrowed fill payload into an owned [`Draw`].
    #[must_use]
    pub fn to_owned(self) -> Draw {
        Draw::Fill {
            transform: self.transform,
            fill_rule: self.fill_rule,
            paint: self.paint.clone(),
            paint_transform: self.paint_transform,
            shape: self.shape.to_owned(),
            composite: self.composite,
        }
    }
}

/// Borrowed stroke draw payload.
#[derive(Clone, Debug, PartialEq)]
pub struct StrokeRef<'a> {
    /// Geometry transform.
    pub transform: Affine,
    /// Stroke style.
    pub stroke: &'a StrokeStyle,
    /// Paint used by this draw.
    pub paint: &'a Paint,
    /// Optional paint-space transform (for gradients/images).
    pub paint_transform: Option<Affine>,
    /// Geometry to stroke.
    pub shape: GeometryRef<'a>,
    /// Per-draw compositing.
    pub composite: Composite,
}

impl<'a> StrokeRef<'a> {
    /// Create a stroke draw with default transform, paint transform, and composite state.
    #[must_use]
    pub fn new(
        shape: impl Into<GeometryRef<'a>>,
        stroke: &'a StrokeStyle,
        paint: &'a Paint,
    ) -> Self {
        Self {
            transform: Affine::IDENTITY,
            stroke,
            paint,
            paint_transform: None,
            shape: shape.into(),
            composite: Composite::default(),
        }
    }

    /// Set the geometry transform.
    #[must_use]
    pub fn transform(mut self, transform: Affine) -> Self {
        self.transform = transform;
        self
    }

    /// Set the optional paint-space transform.
    #[must_use]
    pub fn paint_transform(mut self, paint_transform: Option<Affine>) -> Self {
        self.paint_transform = paint_transform;
        self
    }

    /// Set the per-draw compositing state.
    #[must_use]
    pub fn composite(mut self, composite: Composite) -> Self {
        self.composite = composite;
        self
    }

    /// Convert a borrowed stroke payload into an owned [`Draw`].
    #[must_use]
    pub fn to_owned(self) -> Draw {
        Draw::Stroke {
            transform: self.transform,
            stroke: self.stroke.clone(),
            paint: self.paint.clone(),
            paint_transform: self.paint_transform,
            shape: self.shape.to_owned(),
            composite: self.composite,
        }
    }
}

/// Borrowed glyph run payload.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphRunRef<'a> {
    /// Font for all glyphs in the run.
    pub font: &'a peniko::FontData,
    /// Global run transform.
    pub transform: Affine,
    /// Optional per-glyph transform applied before the glyph offset translation.
    pub glyph_transform: Option<Affine>,
    /// Font size in pixels per em.
    pub font_size: f32,
    /// Whether glyph hinting is enabled.
    pub hint: bool,
    /// Normalized variation coordinates for a variable font instance.
    pub normalized_coords: &'a [NormalizedCoord],
    /// Fill or stroke style for the glyphs.
    pub style: &'a GlyphStyle,
    /// Positioned glyphs in the run.
    pub glyphs: &'a [Glyph],
    /// Paint used for the run.
    pub paint: &'a Paint,
    /// Per-draw compositing.
    pub composite: Composite,
}

impl<'a> GlyphRunRef<'a> {
    /// Create a glyph run with default transform, hinting, variations, and compositing state.
    #[must_use]
    pub fn new(
        font: &'a peniko::FontData,
        style: &'a GlyphStyle,
        glyphs: &'a [Glyph],
        paint: &'a Paint,
    ) -> Self {
        Self {
            font,
            transform: Affine::IDENTITY,
            glyph_transform: None,
            font_size: 16.0,
            hint: false,
            normalized_coords: &[],
            style,
            glyphs,
            paint,
            composite: Composite::default(),
        }
    }

    /// Convert a borrowed glyph run into an owned [`GlyphRun`].
    #[must_use]
    pub fn to_owned(self) -> GlyphRun {
        GlyphRun {
            font: self.font.clone(),
            transform: self.transform,
            glyph_transform: self.glyph_transform,
            font_size: self.font_size,
            hint: self.hint,
            normalized_coords: self.normalized_coords.to_vec(),
            style: self.style.clone(),
            glyphs: self.glyphs.to_vec(),
            paint: self.paint.clone(),
            composite: self.composite,
        }
    }
}

/// Borrowed draw payload.
#[derive(Clone, Debug, PartialEq)]
pub enum DrawRef<'a> {
    /// Fill a shape.
    Fill(FillRef<'a>),
    /// Stroke a shape.
    Stroke(StrokeRef<'a>),
    /// Draw a positioned glyph run.
    GlyphRun(GlyphRunRef<'a>),
    /// Draw a solid-color rounded rectangle blurred with a gaussian filter.
    BlurredRoundedRect(BlurredRoundedRect),
}

impl<'a> DrawRef<'a> {
    /// Convert a borrowed draw payload into an owned [`Draw`].
    #[must_use]
    pub fn to_owned(self) -> Draw {
        match self {
            Self::Fill(draw) => draw.to_owned(),
            Self::Stroke(draw) => draw.to_owned(),
            Self::GlyphRun(draw) => Draw::GlyphRun(draw.to_owned()),
            Self::BlurredRoundedRect(draw) => Draw::BlurredRoundedRect(draw),
        }
    }
}

/// A backend that can accept borrowed imaging commands.
///
/// This trait is intended for streaming authoring APIs and backend/native recorders that can
/// consume borrowed input directly.
pub trait PaintSink {
    /// Push a non-isolated clip onto the clip stack.
    fn push_clip(&mut self, clip: ClipRef<'_>);
    /// Pop the most recently pushed non-isolated clip.
    fn pop_clip(&mut self);
    /// Push an isolated group onto the group stack.
    fn push_group(&mut self, group: GroupRef<'_>);
    /// Pop the most recently pushed isolated group.
    fn pop_group(&mut self);
    /// Emit a fill draw.
    fn fill(&mut self, draw: FillRef<'_>);
    /// Emit a stroke draw.
    fn stroke(&mut self, draw: StrokeRef<'_>);
    /// Emit a glyph run draw.
    fn glyph_run(&mut self, draw: GlyphRunRef<'_>);
    /// Emit a blurred rounded rect draw.
    fn blurred_rounded_rect(&mut self, draw: BlurredRoundedRect);
}

impl Geometry {
    /// Borrow this geometry as a [`GeometryRef`].
    #[must_use]
    pub fn as_ref(&self) -> GeometryRef<'_> {
        match self {
            Self::Rect(rect) => GeometryRef::Rect(*rect),
            Self::RoundedRect(rounded_rect) => GeometryRef::RoundedRect(*rounded_rect),
            Self::Path(path) => GeometryRef::Path(path),
        }
    }
}

impl Clip {
    /// Borrow this clip as a [`ClipRef`].
    #[must_use]
    pub fn as_ref(&self) -> ClipRef<'_> {
        match self {
            Self::Fill {
                transform,
                shape,
                fill_rule,
            } => ClipRef::Fill {
                transform: *transform,
                shape: shape.as_ref(),
                fill_rule: *fill_rule,
            },
            Self::Stroke {
                transform,
                shape,
                stroke,
            } => ClipRef::Stroke {
                transform: *transform,
                shape: shape.as_ref(),
                stroke,
            },
        }
    }
}

impl Group {
    /// Borrow this group as a [`GroupRef`].
    #[must_use]
    pub fn as_ref(&self) -> GroupRef<'_> {
        GroupRef {
            clip: self.clip.as_ref().map(Clip::as_ref),
            filters: &self.filters,
            composite: self.composite,
        }
    }
}

impl GlyphRun {
    /// Borrow this glyph run as a [`GlyphRunRef`].
    #[must_use]
    pub fn as_ref(&self) -> GlyphRunRef<'_> {
        GlyphRunRef {
            font: &self.font,
            transform: self.transform,
            glyph_transform: self.glyph_transform,
            font_size: self.font_size,
            hint: self.hint,
            normalized_coords: &self.normalized_coords,
            style: &self.style,
            glyphs: &self.glyphs,
            paint: &self.paint,
            composite: self.composite,
        }
    }
}

impl Draw {
    /// Borrow this draw as a [`DrawRef`].
    #[must_use]
    pub fn as_ref(&self) -> DrawRef<'_> {
        match self {
            Self::Fill {
                transform,
                fill_rule,
                paint,
                paint_transform,
                shape,
                composite,
            } => DrawRef::Fill(FillRef {
                transform: *transform,
                fill_rule: *fill_rule,
                paint,
                paint_transform: *paint_transform,
                shape: shape.as_ref(),
                composite: *composite,
            }),
            Self::Stroke {
                transform,
                stroke,
                paint,
                paint_transform,
                shape,
                composite,
            } => DrawRef::Stroke(StrokeRef {
                transform: *transform,
                stroke,
                paint,
                paint_transform: *paint_transform,
                shape: shape.as_ref(),
                composite: *composite,
            }),
            Self::GlyphRun(glyph_run) => DrawRef::GlyphRun(glyph_run.as_ref()),
            Self::BlurredRoundedRect(draw) => DrawRef::BlurredRoundedRect(*draw),
        }
    }
}

fn replay_clip(scene: &Scene, id: ClipId, sink: &mut impl PaintSink) {
    sink.push_clip(scene.clip(id).as_ref());
}

fn replay_group(scene: &Scene, id: GroupId, sink: &mut impl PaintSink) {
    sink.push_group(scene.group(id).as_ref());
}

fn replay_draw(scene: &Scene, id: DrawId, sink: &mut impl PaintSink) {
    match scene.draw_op(id).as_ref() {
        DrawRef::Fill(draw) => sink.fill(draw),
        DrawRef::Stroke(draw) => sink.stroke(draw),
        DrawRef::GlyphRun(draw) => sink.glyph_run(draw),
        DrawRef::BlurredRoundedRect(draw) => sink.blurred_rounded_rect(draw),
    }
}

/// Replay a recorded [`Scene`] into a [`PaintSink`].
pub fn replay(scene: &Scene, sink: &mut impl PaintSink) {
    for cmd in scene.commands() {
        match *cmd {
            Command::PushClip(id) => replay_clip(scene, id, sink),
            Command::PopClip => sink.pop_clip(),
            Command::PushGroup(id) => replay_group(scene, id, sink),
            Command::PopGroup => sink.pop_group(),
            Command::Draw(id) => replay_draw(scene, id, sink),
        }
    }
}
