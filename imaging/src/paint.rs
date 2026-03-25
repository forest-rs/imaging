// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Borrowed paint/streaming API.
//!
//! These types let callers stream imaging commands as borrowed data into a [`PaintSink`] without
//! first constructing owned recording payloads. [`crate::record::Scene`] remains the owned
//! semantic recording format.

use kurbo::{Affine, BezPath, Rect, RoundedRect, Shape as _, Stroke};
use peniko::{BrushRef, Fill, Style};

use crate::{
    BlurredRoundedRect, Composite, Filter, MaskMode, NormalizedCoord,
    record::{
        AppliedMask, Clip, ClipId, Command, Draw, DrawId, Geometry, Glyph, GlyphRun, Group,
        GroupId, Mask, MaskId, Scene,
    },
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
        fill_rule: Fill,
    },
    /// Clip to the filled outline of a stroked shape.
    Stroke {
        /// Transform applied to the clip shape.
        transform: Affine,
        /// Shape whose stroked outline defines the clip region.
        shape: GeometryRef<'a>,
        /// Stroke style used to compute the outline.
        stroke: &'a Stroke,
    },
}

impl<'a> ClipRef<'a> {
    /// Create a fill-style clip.
    ///
    /// Defaults:
    /// - clip transform: [`Affine::IDENTITY`]
    /// - fill rule: [`Fill::NonZero`]
    #[must_use]
    pub fn fill(shape: impl Into<GeometryRef<'a>>) -> Self {
        Self::Fill {
            transform: Affine::IDENTITY,
            shape: shape.into(),
            fill_rule: Fill::NonZero,
        }
    }

    /// Create a fill-style clip with an explicit fill rule.
    ///
    /// Default:
    /// - clip transform: [`Affine::IDENTITY`]
    #[must_use]
    pub fn fill_with_rule(shape: impl Into<GeometryRef<'a>>, fill_rule: Fill) -> Self {
        Self::Fill {
            transform: Affine::IDENTITY,
            shape: shape.into(),
            fill_rule,
        }
    }

    /// Create a stroke-style clip.
    ///
    /// Default:
    /// - clip transform: [`Affine::IDENTITY`]
    #[must_use]
    pub fn stroke(shape: impl Into<GeometryRef<'a>>, stroke: &'a Stroke) -> Self {
        Self::Stroke {
            transform: Affine::IDENTITY,
            shape: shape.into(),
            stroke,
        }
    }

    /// Set the transform applied to the clip shape.
    #[must_use]
    pub fn with_transform(self, transform: Affine) -> Self {
        match self {
            Self::Fill {
                shape, fill_rule, ..
            } => Self::Fill {
                transform,
                shape,
                fill_rule,
            },
            Self::Stroke { shape, stroke, .. } => Self::Stroke {
                transform,
                shape,
                stroke,
            },
        }
    }

    #[must_use]
    pub(crate) fn prepend_transform(self, prefix: Affine) -> Self {
        match self {
            Self::Fill {
                transform,
                shape,
                fill_rule,
            } => Self::Fill {
                transform: prefix * transform,
                shape,
                fill_rule,
            },
            Self::Stroke {
                transform,
                shape,
                stroke,
            } => Self::Stroke {
                transform: prefix * transform,
                shape,
                stroke,
            },
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
    /// Optional retained mask applied to the group result before compositing.
    pub mask: Option<AppliedMaskRef<'a>>,
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
            mask: None,
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

    /// Set the retained mask applied to the group result.
    ///
    /// The mask's interpretation comes from the [`MaskRef`] itself.
    #[must_use]
    pub fn with_mask(mut self, mask: MaskRef<'a>) -> Self {
        self.mask = Some(AppliedMaskRef::new(mask));
        self
    }

    /// Set the retained mask applied to the group result with an explicit mask transform.
    ///
    /// The mask's interpretation comes from the [`MaskRef`] itself.
    #[must_use]
    pub fn with_mask_transformed(mut self, mask: MaskRef<'a>, transform: Affine) -> Self {
        self.mask = Some(AppliedMaskRef::new(mask).transform(transform));
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

    pub(crate) fn into_owned_with(
        self,
        define_mask: &mut impl FnMut(MaskRef<'_>) -> MaskId,
    ) -> Group {
        Group {
            clip: self.clip.map(ClipRef::to_owned),
            mask: self.mask.map(|mask| mask.into_owned_with(define_mask)),
            filters: self.filters.to_vec(),
            composite: self.composite,
        }
    }

    #[must_use]
    pub(crate) fn prepend_transform(self, prefix: Affine) -> Self {
        Self {
            clip: self.clip.map(|clip| clip.prepend_transform(prefix)),
            mask: self.mask.map(|mask| mask.prepend_transform(prefix)),
            ..self
        }
    }
}

impl<'a> Default for GroupRef<'a> {
    fn default() -> Self {
        Self::new()
    }
}

/// Borrowed retained mask definition.
#[derive(Clone, Debug, PartialEq)]
pub struct MaskRef<'a> {
    /// How this mask scene modulates masked content.
    pub mode: MaskMode,
    /// Scene that produces the mask values.
    pub scene: &'a Scene,
}

impl<'a> MaskRef<'a> {
    /// Create a borrowed mask definition.
    #[must_use]
    pub fn new(mode: MaskMode, scene: &'a Scene) -> Self {
        Self { mode, scene }
    }

    /// Convert a borrowed mask definition into an owned [`Mask`].
    #[must_use]
    pub fn to_owned(self) -> Mask {
        Mask {
            mode: self.mode,
            scene: self.scene.clone(),
        }
    }
}

/// Borrowed use of a retained mask within an isolated group.
#[derive(Clone, Debug, PartialEq)]
pub struct AppliedMaskRef<'a> {
    /// Referenced mask definition.
    pub mask: MaskRef<'a>,
    /// Transform applied when realizing the mask scene.
    pub transform: Affine,
}

impl<'a> AppliedMaskRef<'a> {
    /// Create a mask use with an identity transform.
    #[must_use]
    pub fn new(mask: MaskRef<'a>) -> Self {
        Self {
            mask,
            transform: Affine::IDENTITY,
        }
    }

    /// Set the transform applied when realizing the mask scene.
    #[must_use]
    pub fn transform(mut self, transform: Affine) -> Self {
        self.transform = transform;
        self
    }

    pub(crate) fn into_owned_with(
        self,
        define_mask: &mut impl FnMut(MaskRef<'_>) -> MaskId,
    ) -> AppliedMask {
        AppliedMask {
            mask: define_mask(self.mask),
            transform: self.transform,
        }
    }

    #[must_use]
    pub(crate) fn prepend_transform(self, prefix: Affine) -> Self {
        Self {
            transform: prefix * self.transform,
            ..self
        }
    }
}

/// Borrowed fill draw payload.
#[derive(Clone, Debug, PartialEq)]
pub struct FillRef<'a> {
    /// Geometry transform.
    pub transform: Affine,
    /// Fill rule used to determine inside/outside for paths.
    pub fill_rule: Fill,
    /// Brush used by this draw.
    pub brush: BrushRef<'a>,
    /// Optional brush-space transform (for gradients/images).
    pub brush_transform: Option<Affine>,
    /// Geometry to fill.
    pub shape: GeometryRef<'a>,
    /// Per-draw compositing.
    pub composite: Composite,
}

impl<'a> FillRef<'a> {
    /// Create a fill draw with default state.
    ///
    /// Defaults:
    /// - geometry transform: [`Affine::IDENTITY`]
    /// - fill rule: [`Fill::NonZero`]
    /// - brush transform: `None`
    /// - compositing: [`Composite::default()`]
    #[must_use]
    pub fn new(shape: impl Into<GeometryRef<'a>>, brush: impl Into<BrushRef<'a>>) -> Self {
        Self {
            transform: Affine::IDENTITY,
            fill_rule: Fill::NonZero,
            brush: brush.into(),
            brush_transform: None,
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
    pub fn fill_rule(mut self, fill_rule: Fill) -> Self {
        self.fill_rule = fill_rule;
        self
    }

    /// Set the optional brush-space transform.
    #[must_use]
    pub fn brush_transform(mut self, brush_transform: Option<Affine>) -> Self {
        self.brush_transform = brush_transform;
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
            brush: self.brush.to_owned(),
            brush_transform: self.brush_transform,
            shape: self.shape.to_owned(),
            composite: self.composite,
        }
    }

    #[must_use]
    pub(crate) fn prepend_transform(self, prefix: Affine) -> Self {
        Self {
            transform: prefix * self.transform,
            ..self
        }
    }
}

/// Borrowed stroke draw payload.
#[derive(Clone, Debug, PartialEq)]
pub struct StrokeRef<'a> {
    /// Geometry transform.
    pub transform: Affine,
    /// Stroke style.
    pub stroke: &'a Stroke,
    /// Brush used by this draw.
    pub brush: BrushRef<'a>,
    /// Optional brush-space transform (for gradients/images).
    pub brush_transform: Option<Affine>,
    /// Geometry to stroke.
    pub shape: GeometryRef<'a>,
    /// Per-draw compositing.
    pub composite: Composite,
}

impl<'a> StrokeRef<'a> {
    /// Create a stroke draw with default state.
    ///
    /// Defaults:
    /// - geometry transform: [`Affine::IDENTITY`]
    /// - brush transform: `None`
    /// - compositing: [`Composite::default()`]
    #[must_use]
    pub fn new(
        shape: impl Into<GeometryRef<'a>>,
        stroke: &'a Stroke,
        brush: impl Into<BrushRef<'a>>,
    ) -> Self {
        Self {
            transform: Affine::IDENTITY,
            stroke,
            brush: brush.into(),
            brush_transform: None,
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

    /// Set the optional brush-space transform.
    #[must_use]
    pub fn brush_transform(mut self, brush_transform: Option<Affine>) -> Self {
        self.brush_transform = brush_transform;
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
            brush: self.brush.to_owned(),
            brush_transform: self.brush_transform,
            shape: self.shape.to_owned(),
            composite: self.composite,
        }
    }

    #[must_use]
    pub(crate) fn prepend_transform(self, prefix: Affine) -> Self {
        Self {
            transform: prefix * self.transform,
            ..self
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
    pub style: &'a Style,
    /// Brush used for the run.
    pub brush: BrushRef<'a>,
    /// Per-draw compositing.
    pub composite: Composite,
}

impl<'a> GlyphRunRef<'a> {
    /// Create a glyph run with default state.
    ///
    /// Defaults:
    /// - run transform: [`Affine::IDENTITY`]
    /// - per-glyph transform: `None`
    /// - font size: `16.0`
    /// - hinting: `false`
    /// - normalized variation coordinates: `&[]`
    /// - compositing: [`Composite::default()`]
    #[must_use]
    pub fn new(
        font: &'a peniko::FontData,
        style: &'a Style,
        brush: impl Into<BrushRef<'a>>,
    ) -> Self {
        Self {
            font,
            transform: Affine::IDENTITY,
            glyph_transform: None,
            font_size: 16.0,
            hint: false,
            normalized_coords: &[],
            style,
            brush: brush.into(),
            composite: Composite::default(),
        }
    }

    /// Convert a borrowed glyph run into an owned [`GlyphRun`].
    #[must_use]
    pub fn to_owned(self, glyphs: impl IntoIterator<Item = Glyph>) -> GlyphRun {
        GlyphRun {
            font: self.font.clone(),
            transform: self.transform,
            glyph_transform: self.glyph_transform,
            font_size: self.font_size,
            hint: self.hint,
            normalized_coords: self.normalized_coords.to_vec(),
            style: self.style.clone(),
            glyphs: glyphs.into_iter().collect(),
            brush: self.brush.to_owned(),
            composite: self.composite,
        }
    }

    #[must_use]
    pub(crate) fn prepend_transform(self, prefix: Affine) -> Self {
        Self {
            transform: prefix * self.transform,
            ..self
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
    pub fn to_owned(self, glyphs: impl IntoIterator<Item = Glyph>) -> Draw {
        match self {
            Self::Fill(draw) => draw.to_owned(),
            Self::Stroke(draw) => draw.to_owned(),
            Self::GlyphRun(draw) => Draw::GlyphRun(draw.to_owned(glyphs)),
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
    fn glyph_run(&mut self, draw: GlyphRunRef<'_>, glyphs: &mut dyn Iterator<Item = Glyph>);
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
    pub fn as_ref_with<'a>(&'a self, scene: &'a Scene) -> GroupRef<'a> {
        GroupRef {
            clip: self.clip.as_ref().map(Clip::as_ref),
            mask: self
                .mask
                .as_ref()
                .map(|mask| mask.as_ref(scene.mask(mask.mask))),
            filters: &self.filters,
            composite: self.composite,
        }
    }
}

impl Mask {
    /// Borrow this mask as a [`MaskRef`].
    #[must_use]
    pub fn as_ref(&self) -> MaskRef<'_> {
        MaskRef {
            mode: self.mode,
            scene: &self.scene,
        }
    }
}

impl AppliedMask {
    /// Borrow this mask use as an [`AppliedMaskRef`].
    #[must_use]
    pub fn as_ref<'a>(&self, mask: &'a Mask) -> AppliedMaskRef<'a> {
        AppliedMaskRef {
            mask: mask.as_ref(),
            transform: self.transform,
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
            brush: (&self.brush).into(),
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
                brush,
                brush_transform,
                shape,
                composite,
            } => DrawRef::Fill(FillRef {
                transform: *transform,
                fill_rule: *fill_rule,
                brush: brush.into(),
                brush_transform: *brush_transform,
                shape: shape.as_ref(),
                composite: *composite,
            }),
            Self::Stroke {
                transform,
                stroke,
                brush,
                brush_transform,
                shape,
                composite,
            } => DrawRef::Stroke(StrokeRef {
                transform: *transform,
                stroke,
                brush: brush.into(),
                brush_transform: *brush_transform,
                shape: shape.as_ref(),
                composite: *composite,
            }),
            Self::GlyphRun(glyph_run) => DrawRef::GlyphRun(glyph_run.as_ref()),
            Self::BlurredRoundedRect(draw) => DrawRef::BlurredRoundedRect(*draw),
        }
    }
}

struct TransformingSink<'a, S: ?Sized> {
    inner: &'a mut S,
    transform: Affine,
}

impl<S> PaintSink for TransformingSink<'_, S>
where
    S: PaintSink + ?Sized,
{
    fn push_clip(&mut self, clip: ClipRef<'_>) {
        self.inner.push_clip(clip.prepend_transform(self.transform));
    }

    fn pop_clip(&mut self) {
        self.inner.pop_clip();
    }

    fn push_group(&mut self, group: GroupRef<'_>) {
        self.inner
            .push_group(group.prepend_transform(self.transform));
    }

    fn pop_group(&mut self) {
        self.inner.pop_group();
    }

    fn fill(&mut self, draw: FillRef<'_>) {
        self.inner.fill(draw.prepend_transform(self.transform));
    }

    fn stroke(&mut self, draw: StrokeRef<'_>) {
        self.inner.stroke(draw.prepend_transform(self.transform));
    }

    fn glyph_run(&mut self, draw: GlyphRunRef<'_>, glyphs: &mut dyn Iterator<Item = Glyph>) {
        self.inner
            .glyph_run(draw.prepend_transform(self.transform), glyphs);
    }

    fn blurred_rounded_rect(&mut self, draw: BlurredRoundedRect) {
        self.inner
            .blurred_rounded_rect(draw.prepend_transform(self.transform));
    }
}

fn replay_clip<S>(scene: &Scene, id: ClipId, sink: &mut S)
where
    S: PaintSink + ?Sized,
{
    sink.push_clip(scene.clip(id).as_ref());
}

fn replay_group<S>(scene: &Scene, id: GroupId, sink: &mut S)
where
    S: PaintSink + ?Sized,
{
    sink.push_group(scene.group(id).as_ref_with(scene));
}

fn replay_draw<S>(scene: &Scene, id: DrawId, sink: &mut S)
where
    S: PaintSink + ?Sized,
{
    match scene.draw_op(id) {
        Draw::GlyphRun(glyph_run) => {
            let mut glyphs = glyph_run.glyphs.iter().copied();
            sink.glyph_run(glyph_run.as_ref(), &mut glyphs);
        }
        draw => match draw.as_ref() {
            DrawRef::Fill(draw) => sink.fill(draw),
            DrawRef::Stroke(draw) => sink.stroke(draw),
            DrawRef::BlurredRoundedRect(draw) => sink.blurred_rounded_rect(draw),
            DrawRef::GlyphRun(_) => {
                unreachable!("glyph runs are handled using the owned glyph slice")
            }
        },
    }
}

/// Replay a recorded [`crate::record::Scene`] into a [`PaintSink`].
pub(crate) fn replay<S>(scene: &Scene, sink: &mut S)
where
    S: PaintSink + ?Sized,
{
    replay_transformed(scene, sink, Affine::IDENTITY);
}

/// Replay a recorded [`crate::record::Scene`] into a [`PaintSink`] with an extra transform.
pub(crate) fn replay_transformed<S>(scene: &Scene, sink: &mut S, transform: Affine)
where
    S: PaintSink + ?Sized,
{
    let mut sink = TransformingSink {
        inner: sink,
        transform,
    };
    for cmd in scene.commands() {
        match *cmd {
            Command::PushClip(id) => replay_clip(scene, id, &mut sink),
            Command::PopClip => sink.pop_clip(),
            Command::PushGroup(id) => replay_group(scene, id, &mut sink),
            Command::PopGroup => sink.pop_group(),
            Command::Draw(id) => replay_draw(scene, id, &mut sink),
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::sync::Arc;
    use alloc::vec;

    use super::*;
    use crate::{Composite, record::Geometry};
    use peniko::{Brush, FontData};

    #[test]
    fn clip_ref_prepend_transform_prefixes_clip_transform() {
        let clip = ClipRef::fill(Rect::new(0.0, 0.0, 3.0, 4.0))
            .with_transform(Affine::translate((1.0, 2.0)));

        assert_eq!(
            clip.prepend_transform(Affine::translate((5.0, 6.0))),
            ClipRef::Fill {
                transform: Affine::translate((6.0, 8.0)),
                shape: GeometryRef::Rect(Rect::new(0.0, 0.0, 3.0, 4.0)),
                fill_rule: Fill::NonZero,
            }
        );
    }

    #[test]
    fn group_ref_prepend_transform_prefixes_isolated_clip() {
        let stroke = Stroke::new(2.0);
        let group = GroupRef::new().with_clip(
            ClipRef::stroke(Rect::new(0.0, 0.0, 3.0, 4.0), &stroke)
                .with_transform(Affine::translate((1.0, 2.0))),
        );

        assert_eq!(
            group.prepend_transform(Affine::translate((5.0, 6.0))),
            GroupRef {
                clip: Some(ClipRef::Stroke {
                    transform: Affine::translate((6.0, 8.0)),
                    shape: GeometryRef::Rect(Rect::new(0.0, 0.0, 3.0, 4.0)),
                    stroke: &stroke,
                }),
                mask: None,
                filters: &[],
                composite: Composite::default(),
            }
        );
    }

    #[test]
    fn group_ref_mask_helpers_set_transform() {
        let mask_scene = Scene::new();
        let group = GroupRef::new().with_mask_transformed(
            MaskRef::new(MaskMode::Luminance, &mask_scene),
            Affine::translate((3.0, 4.0)),
        );

        let mask = group.mask.expect("expected mask");
        assert_eq!(mask.mask.mode, MaskMode::Luminance);
        assert_eq!(mask.transform, Affine::translate((3.0, 4.0)));
        assert_eq!(mask.mask, MaskRef::new(MaskMode::Luminance, &mask_scene));
    }

    #[test]
    fn fill_ref_prepend_transform_prefixes_draw_transform_only() {
        let draw = FillRef::new(
            Rect::new(0.0, 0.0, 3.0, 4.0),
            Brush::Solid(peniko::Color::WHITE),
        )
        .transform(Affine::translate((1.0, 2.0)))
        .brush_transform(Some(Affine::translate((3.0, 4.0))));

        assert_eq!(
            draw.prepend_transform(Affine::translate((5.0, 6.0))),
            FillRef {
                transform: Affine::translate((6.0, 8.0)),
                fill_rule: Fill::NonZero,
                brush: BrushRef::Solid(peniko::Color::WHITE),
                brush_transform: Some(Affine::translate((3.0, 4.0))),
                shape: GeometryRef::Rect(Rect::new(0.0, 0.0, 3.0, 4.0)),
                composite: Composite::default(),
            }
        );
    }

    #[test]
    fn fill_ref_prepend_transform_preserves_missing_brush_transform() {
        let draw = FillRef::new(
            Rect::new(0.0, 0.0, 3.0, 4.0),
            Brush::Solid(peniko::Color::WHITE),
        );

        assert_eq!(
            draw.prepend_transform(Affine::translate((5.0, 6.0)))
                .brush_transform,
            None
        );
    }

    #[test]
    fn glyph_run_ref_prepend_transform_only_prefixes_run_transform() {
        let font = FontData::new(peniko::Blob::new(Arc::new([0_u8, 1_u8, 2_u8, 3_u8])), 0);
        let style = Style::Fill(Fill::NonZero);
        let draw = GlyphRunRef::new(&font, &style, Brush::Solid(peniko::Color::BLACK));
        let draw = GlyphRunRef {
            transform: Affine::translate((1.0, 2.0)),
            glyph_transform: Some(Affine::translate((3.0, 4.0))),
            font_size: 12.0,
            ..draw
        };

        let transformed = draw.prepend_transform(Affine::translate((5.0, 6.0)));
        assert_eq!(transformed.transform, Affine::translate((6.0, 8.0)));
        assert_eq!(
            transformed.glyph_transform,
            Some(Affine::translate((3.0, 4.0)))
        );
    }

    #[test]
    fn replay_transformed_prefixes_recorded_transforms() {
        let mut source = Scene::new();
        let clip_id = source.push_clip(Clip::Fill {
            transform: Affine::translate((1.0, 2.0)),
            shape: Geometry::Rect(Rect::new(0.0, 0.0, 3.0, 4.0)),
            fill_rule: Fill::NonZero,
        });
        let group_id = source.push_group(Group {
            clip: Some(Clip::Stroke {
                transform: Affine::translate((5.0, 6.0)),
                shape: Geometry::Rect(Rect::new(0.0, 0.0, 7.0, 8.0)),
                stroke: Stroke::new(2.0),
            }),
            mask: None,
            filters: vec![],
            composite: Composite::default(),
        });
        let fill_id = source.draw(Draw::Fill {
            transform: Affine::translate((9.0, 10.0)),
            fill_rule: Fill::NonZero,
            brush: Brush::Solid(peniko::Color::WHITE),
            brush_transform: Some(Affine::translate((11.0, 12.0))),
            shape: Geometry::Rect(Rect::new(0.0, 0.0, 13.0, 14.0)),
            composite: Composite::default(),
        });
        let stroke_id = source.draw(Draw::Stroke {
            transform: Affine::translate((15.0, 16.0)),
            stroke: Stroke::new(3.0),
            brush: Brush::Solid(peniko::Color::BLACK),
            brush_transform: None,
            shape: Geometry::Rect(Rect::new(0.0, 0.0, 17.0, 18.0)),
            composite: Composite::default(),
        });
        let font = FontData::new(peniko::Blob::new(Arc::new([0_u8, 1_u8, 2_u8, 3_u8])), 0);
        let glyph_id = source.draw(Draw::GlyphRun(GlyphRun {
            font,
            transform: Affine::translate((19.0, 20.0)),
            glyph_transform: Some(Affine::translate((21.0, 22.0))),
            font_size: 12.0,
            hint: false,
            normalized_coords: vec![],
            style: Style::Fill(Fill::NonZero),
            glyphs: vec![Glyph {
                id: 7,
                x: 0.0,
                y: 0.0,
            }],
            brush: Brush::Solid(peniko::Color::BLACK),
            composite: Composite::default(),
        }));
        let blur_id = source.draw(Draw::BlurredRoundedRect(BlurredRoundedRect {
            transform: Affine::translate((23.0, 24.0)),
            rect: Rect::new(0.0, 0.0, 4.0, 3.0),
            color: peniko::Color::BLACK,
            radius: 1.0,
            std_dev: 2.0,
            composite: Composite::default(),
        }));
        source.pop_group();
        source.pop_clip();

        let transform = Affine::translate((100.0, 200.0));
        let mut replayed = Scene::new();
        replay_transformed(&source, &mut replayed, transform);

        assert_eq!(replayed.commands(), source.commands());
        assert_eq!(
            replayed.clip(clip_id),
            &Clip::Fill {
                transform: transform * Affine::translate((1.0, 2.0)),
                shape: Geometry::Rect(Rect::new(0.0, 0.0, 3.0, 4.0)),
                fill_rule: Fill::NonZero,
            }
        );
        assert_eq!(
            replayed.group(group_id),
            &Group {
                clip: Some(Clip::Stroke {
                    transform: transform * Affine::translate((5.0, 6.0)),
                    shape: Geometry::Rect(Rect::new(0.0, 0.0, 7.0, 8.0)),
                    stroke: Stroke::new(2.0),
                }),
                mask: None,
                filters: vec![],
                composite: Composite::default(),
            }
        );
        assert_eq!(
            replayed.draw_op(fill_id),
            &Draw::Fill {
                transform: transform * Affine::translate((9.0, 10.0)),
                fill_rule: Fill::NonZero,
                brush: Brush::Solid(peniko::Color::WHITE),
                brush_transform: Some(Affine::translate((11.0, 12.0))),
                shape: Geometry::Rect(Rect::new(0.0, 0.0, 13.0, 14.0)),
                composite: Composite::default(),
            }
        );
        assert_eq!(
            replayed.draw_op(stroke_id),
            &Draw::Stroke {
                transform: transform * Affine::translate((15.0, 16.0)),
                stroke: Stroke::new(3.0),
                brush: Brush::Solid(peniko::Color::BLACK),
                brush_transform: None,
                shape: Geometry::Rect(Rect::new(0.0, 0.0, 17.0, 18.0)),
                composite: Composite::default(),
            }
        );
        match replayed.draw_op(glyph_id) {
            Draw::GlyphRun(glyph_run) => {
                assert_eq!(
                    glyph_run.transform,
                    transform * Affine::translate((19.0, 20.0))
                );
                assert_eq!(
                    glyph_run.glyph_transform,
                    Some(Affine::translate((21.0, 22.0)))
                );
                assert_eq!(glyph_run.brush, Brush::Solid(peniko::Color::BLACK));
            }
            other => panic!("expected glyph run draw, got {other:?}"),
        }
        assert_eq!(
            replayed.draw_op(blur_id),
            &Draw::BlurredRoundedRect(BlurredRoundedRect {
                transform: transform * Affine::translate((23.0, 24.0)),
                rect: Rect::new(0.0, 0.0, 4.0, 3.0),
                color: peniko::Color::BLACK,
                radius: 1.0,
                std_dev: 2.0,
                composite: Composite::default(),
            })
        );
    }
}
