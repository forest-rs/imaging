// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Owned semantic recording types.
//!
//! This module contains the retained representation for imaging command streams. Use [`Scene`] when
//! you need an owned semantic recording you can retain, validate, and replay.

use alloc::vec::Vec;

use kurbo::{Affine, BezPath, Rect, RoundedRect, Shape as _};
use peniko::FontData;

use crate::{
    BlurredRoundedRect, ClipRef, Composite, FillRef, FillRule, GlyphRunRef, GlyphStyle, GroupRef,
    NormalizedCoord, Paint, PaintSink, StrokeRef, StrokeStyle,
};

/// A geometry payload stored in a recording.
#[derive(Clone, Debug, PartialEq)]
pub enum Geometry {
    /// Axis-aligned rectangle.
    Rect(Rect),
    /// Axis-aligned rounded rectangle.
    RoundedRect(RoundedRect),
    /// General path.
    Path(BezPath),
}

impl Geometry {
    /// Convert a geometry payload into a path.
    ///
    /// This is intended for backends that need explicit path data.
    pub fn to_path(&self, tolerance: f64) -> BezPath {
        match self {
            Self::Rect(r) => r.to_path(tolerance),
            Self::RoundedRect(rr) => rr.to_path(tolerance),
            Self::Path(p) => p.clone(),
        }
    }
}

/// A low-level clip payload stored in a [`Scene`] recording.
///
/// Prefer using [`crate::Painter`] or borrowed [`crate::ClipRef`] values for normal command
/// authoring.
#[derive(Clone, Debug, PartialEq)]
pub enum Clip {
    /// Clip to the fill region of a shape.
    Fill {
        /// Transform applied to the clip shape.
        ///
        /// This does not affect subsequent draws; it only affects how the clip shape is interpreted.
        transform: Affine,
        /// Shape used to define the clip region.
        shape: Geometry,
        /// Fill rule used to determine the interior for path clips.
        fill_rule: FillRule,
    },
    /// Clip to the filled outline of a stroked shape.
    Stroke {
        /// Transform applied to the clip shape.
        ///
        /// This does not affect subsequent draws; it only affects how the clip shape is interpreted.
        transform: Affine,
        /// Shape whose stroked outline defines the clip region.
        shape: Geometry,
        /// Stroke style used to compute the outline (including dashes).
        stroke: StrokeStyle,
    },
}

/// Identifier for a clip payload stored in a [`Scene`].
#[repr(transparent)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ClipId(pub(crate) u32);

/// Identifier for a group payload stored in a [`Scene`].
#[repr(transparent)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct GroupId(pub(crate) u32);

/// Identifier for a draw payload stored in a [`Scene`].
#[repr(transparent)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct DrawId(pub(crate) u32);

/// Parameters for an isolated group.
///
/// The group is rendered into an isolated buffer, then composited into its parent using
/// [`Group::composite`]. If `clip` is present, it is applied to the group's result at
/// composite time (isolated clip).
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// Optional isolated clip applied to the group result.
    pub clip: Option<Clip>,
    /// Optional filter chain applied to the group result before compositing.
    pub filters: Vec<crate::Filter>,
    /// Compositing parameters used when merging the group into its parent.
    pub composite: Composite,
}

impl Default for Group {
    #[inline]
    fn default() -> Self {
        Self {
            clip: None,
            filters: Vec::new(),
            composite: Composite::default(),
        }
    }
}

/// Positioned glyph in a glyph run.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Glyph {
    /// Font-specific glyph identifier.
    ///
    /// This is a glyph index in the selected font, not a Unicode scalar value.
    pub id: u32,
    /// Horizontal offset within the run, relative to [`GlyphRun::transform`].
    pub x: f32,
    /// Vertical offset within the run, relative to [`GlyphRun::transform`].
    pub y: f32,
}

/// A positioned glyph run.
///
/// This records the final glyph IDs and positions. Shaping and line layout are intentionally out of
/// scope for `imaging`; callers are expected to provide already-shaped glyph runs.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphRun {
    /// Font for all glyphs in the run.
    pub font: FontData,
    /// Global run transform.
    pub transform: Affine,
    /// Optional per-glyph transform applied before the glyph offset translation.
    pub glyph_transform: Option<Affine>,
    /// Font size in pixels per em.
    pub font_size: f32,
    /// Whether glyph hinting is enabled.
    pub hint: bool,
    /// Normalized variation coordinates for variable fonts.
    pub normalized_coords: Vec<NormalizedCoord>,
    /// Fill or stroke style for the glyphs.
    pub style: GlyphStyle,
    /// Positioned glyphs in the run.
    pub glyphs: Vec<Glyph>,
    /// Paint used for the run.
    pub paint: Paint,
    /// Per-draw compositing.
    pub composite: Composite,
}

impl GlyphRun {
    /// Create a glyph run with default rendering parameters.
    #[must_use]
    pub fn new(font: FontData) -> Self {
        Self {
            font,
            transform: Affine::IDENTITY,
            glyph_transform: None,
            font_size: 16.0,
            hint: false,
            normalized_coords: Vec::new(),
            style: GlyphStyle::Fill(FillRule::NonZero),
            glyphs: Vec::new(),
            paint: Paint::Solid(peniko::Color::BLACK),
            composite: Composite::default(),
        }
    }
}

/// A low-level draw payload stored in a [`Scene`] recording.
///
/// Prefer using [`crate::Painter`] or borrowed draw payloads like [`crate::FillRef`] and
/// [`crate::StrokeRef`] for normal command authoring.
#[derive(Clone, Debug, PartialEq)]
pub enum Draw {
    /// Fill a shape.
    Fill {
        /// Geometry transform.
        transform: Affine,
        /// Fill rule used to determine inside/outside for paths.
        fill_rule: FillRule,
        /// Paint used by this draw.
        paint: Paint,
        /// Optional paint-space transform (for gradients/images).
        paint_transform: Option<Affine>,
        /// Geometry to fill.
        shape: Geometry,
        /// Per-draw compositing.
        composite: Composite,
    },
    /// Stroke a shape.
    Stroke {
        /// Geometry transform.
        transform: Affine,
        /// Stroke style.
        stroke: StrokeStyle,
        /// Paint used by this draw.
        paint: Paint,
        /// Optional paint-space transform (for gradients/images).
        paint_transform: Option<Affine>,
        /// Geometry to stroke.
        shape: Geometry,
        /// Per-draw compositing.
        composite: Composite,
    },
    /// Draw a positioned glyph run.
    GlyphRun(GlyphRun),
    /// Draw a solid-color rounded rectangle blurred with a gaussian filter.
    BlurredRoundedRect(BlurredRoundedRect),
}

/// A single command in a [`Scene`].
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Push a non-isolated clip onto the clip stack.
    PushClip(ClipId),
    /// Pop the most recently pushed non-isolated clip.
    PopClip,
    /// Push an isolated group onto the group stack.
    PushGroup(GroupId),
    /// Pop the most recently pushed isolated group.
    PopGroup,
    /// Emit a draw command.
    Draw(DrawId),
}

/// An owned, backend-agnostic semantic recording.
///
/// [`Scene`] is the retained form of an imaging command stream. Use [`crate::Painter`] and
/// [`crate::PaintSink`] to author commands, then record them into a scene when you need
/// validation, replay, testing, or backend-independent storage.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    commands: Vec<Command>,
    clips: Vec<Clip>,
    groups: Vec<Group>,
    draws: Vec<Draw>,
}

impl Scene {
    /// Create an empty scene.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear all recorded commands.
    #[inline]
    pub fn clear(&mut self) {
        self.commands.clear();
        self.clips.clear();
        self.groups.clear();
        self.draws.clear();
    }

    /// Borrow the recorded command stream.
    #[inline]
    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    /// Resolve a clip payload by ID.
    #[inline]
    pub fn clip(&self, id: ClipId) -> &Clip {
        &self.clips[id.0 as usize]
    }

    /// Resolve a group payload by ID.
    #[inline]
    pub fn group(&self, id: GroupId) -> &Group {
        &self.groups[id.0 as usize]
    }

    /// Resolve a draw payload by ID.
    #[inline]
    pub fn draw_op(&self, id: DrawId) -> &Draw {
        &self.draws[id.0 as usize]
    }

    /// Push a non-isolated clip.
    #[inline]
    pub fn push_clip(&mut self, clip: Clip) -> ClipId {
        let idx = u32::try_from(self.clips.len()).expect("scene clip table overflow");
        let id = ClipId(idx);
        self.clips.push(clip);
        self.commands.push(Command::PushClip(id));
        id
    }

    /// Pop a non-isolated clip.
    #[inline]
    pub fn pop_clip(&mut self) {
        self.commands.push(Command::PopClip);
    }

    /// Push an isolated group.
    #[inline]
    pub fn push_group(&mut self, group: Group) -> GroupId {
        let idx = u32::try_from(self.groups.len()).expect("scene group table overflow");
        let id = GroupId(idx);
        self.groups.push(group);
        self.commands.push(Command::PushGroup(id));
        id
    }

    /// Pop an isolated group.
    #[inline]
    pub fn pop_group(&mut self) {
        self.commands.push(Command::PopGroup);
    }

    /// Record a draw command.
    #[inline]
    pub fn draw(&mut self, draw: Draw) -> DrawId {
        let idx = u32::try_from(self.draws.len()).expect("scene draw table overflow");
        let id = DrawId(idx);
        self.draws.push(draw);
        self.commands.push(Command::Draw(id));
        id
    }

    /// Validate well-nested stacks.
    pub fn validate(&self) -> Result<(), ValidateError> {
        let mut clip_depth = 0_u32;
        let mut group_depth = 0_u32;

        for cmd in &self.commands {
            match cmd {
                Command::PushClip(_) => clip_depth += 1,
                Command::PopClip => {
                    clip_depth = clip_depth
                        .checked_sub(1)
                        .ok_or(ValidateError::UnbalancedPopClip)?;
                }
                Command::PushGroup(_) => group_depth += 1,
                Command::PopGroup => {
                    group_depth = group_depth
                        .checked_sub(1)
                        .ok_or(ValidateError::UnbalancedPopGroup)?;
                }
                Command::Draw(_) => {}
            }
        }

        if clip_depth != 0 {
            return Err(ValidateError::UnclosedClips);
        }
        if group_depth != 0 {
            return Err(ValidateError::UnclosedGroups);
        }
        Ok(())
    }
}

impl PaintSink for Scene {
    #[inline]
    fn push_clip(&mut self, clip: ClipRef<'_>) {
        let _ = Self::push_clip(self, clip.to_owned());
    }

    #[inline]
    fn pop_clip(&mut self) {
        Self::pop_clip(self);
    }

    #[inline]
    fn push_group(&mut self, group: GroupRef<'_>) {
        let _ = Self::push_group(self, group.to_owned());
    }

    #[inline]
    fn pop_group(&mut self) {
        Self::pop_group(self);
    }

    #[inline]
    fn fill(&mut self, draw: FillRef<'_>) {
        let _ = Self::draw(self, draw.to_owned());
    }

    #[inline]
    fn stroke(&mut self, draw: StrokeRef<'_>) {
        let _ = Self::draw(self, draw.to_owned());
    }

    #[inline]
    fn glyph_run(&mut self, draw: GlyphRunRef<'_>) {
        let _ = Self::draw(self, Draw::GlyphRun(draw.to_owned()));
    }

    #[inline]
    fn blurred_rounded_rect(&mut self, draw: BlurredRoundedRect) {
        let _ = Self::draw(self, Draw::BlurredRoundedRect(draw));
    }
}

/// Replay a recorded [`Scene`] into a [`crate::PaintSink`].
pub fn replay(scene: &Scene, sink: &mut impl PaintSink) {
    crate::paint::replay(scene, sink);
}

/// Errors returned by [`Scene::validate`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ValidateError {
    /// A `PopClip` occurred without a matching prior `PushClip`.
    UnbalancedPopClip,
    /// A `PopGroup` occurred without a matching prior `PushGroup`.
    UnbalancedPopGroup,
    /// The command stream ended with open clips.
    UnclosedClips,
    /// The command stream ended with open groups.
    UnclosedGroups,
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::sync::Arc;
    use alloc::vec;

    #[test]
    fn validate_balanced() {
        let mut s = Scene::new();
        let _clip = s.push_clip(Clip::Fill {
            transform: Affine::IDENTITY,
            shape: Geometry::Rect(Rect::new(0.0, 0.0, 10.0, 10.0)),
            fill_rule: FillRule::NonZero,
        });
        let _group = s.push_group(Group::default());
        s.pop_group();
        s.pop_clip();
        assert_eq!(s.validate(), Ok(()));
    }

    #[test]
    fn validate_catches_pop_underflow() {
        let mut s = Scene::new();
        s.pop_clip();
        assert_eq!(s.validate(), Err(ValidateError::UnbalancedPopClip));
    }

    #[test]
    fn replay_round_trip() {
        let mut a = Scene::new();
        let _ = a.push_clip(Clip::Fill {
            transform: Affine::IDENTITY,
            shape: Geometry::Rect(Rect::new(0.0, 0.0, 1.0, 1.0)),
            fill_rule: FillRule::NonZero,
        });
        let _ = a.push_group(Group::default());
        a.draw(Draw::Fill {
            transform: Affine::IDENTITY,
            fill_rule: FillRule::NonZero,
            paint: Paint::Solid(peniko::Color::WHITE),
            paint_transform: None,
            shape: Geometry::Rect(Rect::new(0.0, 0.0, 1.0, 1.0)),
            composite: Composite::default(),
        });
        let font = FontData::new(peniko::Blob::new(Arc::new([0_u8, 1_u8, 2_u8, 3_u8])), 0);
        a.draw(Draw::GlyphRun(GlyphRun {
            font,
            transform: Affine::IDENTITY,
            glyph_transform: None,
            font_size: 12.0,
            hint: false,
            normalized_coords: Vec::new(),
            style: GlyphStyle::Fill(FillRule::NonZero),
            glyphs: vec![Glyph {
                id: 7,
                x: 0.0,
                y: 0.0,
            }],
            paint: Paint::Solid(peniko::Color::BLACK),
            composite: Composite::default(),
        }));
        a.draw(Draw::BlurredRoundedRect(BlurredRoundedRect {
            transform: Affine::IDENTITY,
            rect: Rect::new(0.0, 0.0, 4.0, 3.0),
            color: peniko::Color::BLACK,
            radius: 1.0,
            std_dev: 2.0,
            composite: Composite::default(),
        }));
        a.pop_group();
        a.pop_clip();
        assert_eq!(a.validate(), Ok(()));

        let mut b = Scene::new();
        replay(&a, &mut b);
        assert_eq!(a, b);
    }
}
