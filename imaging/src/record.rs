// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Owned semantic recording types.
//!
//! This module contains the retained representation for imaging command streams. Use [`Scene`] when
//! you need an owned semantic recording you can retain, validate, and replay.

use alloc::{boxed::Box, string::String, vec::Vec};

use kurbo::{Affine, BezPath, Rect, RoundedRect, Shape as _, Stroke};
use peniko::{Brush, Fill, FontData, Style};

use crate::{
    BlurredRoundedRect, ClipRef, Composite, ContextRef, FillRef, GlyphRunRef, GroupRef, MaskMode,
    NormalizedCoord, PaintSink, SourceLocationRef, StrokeRef,
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
        fill_rule: Fill,
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
        stroke: Stroke,
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

/// Identifier for a mask payload stored in a [`Scene`].
#[repr(transparent)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct MaskId(pub(crate) u32);

/// Identifier for a draw payload stored in a [`Scene`].
#[repr(transparent)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct DrawId(pub(crate) u32);

/// Identifier for an interned context label stored in a [`Scene`].
#[repr(transparent)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct LabelId(pub(crate) u32);

/// Identifier for an interned source file stored in a [`Scene`].
#[repr(transparent)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct FileId(pub(crate) u32);

/// Identifier for a context annotation stored in a [`Scene`].
#[repr(transparent)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ContextId(pub(crate) u32);

/// Interned source location stored in a [`Scene`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceLocation {
    /// Interned file name.
    pub file: FileId,
    /// 1-based source line.
    pub line: u32,
    /// 1-based source column.
    pub column: u32,
}

/// A context annotation stored in a [`Scene`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    /// Human-readable label.
    pub label: LabelId,
    /// Optional source location associated with the context.
    pub source: Option<SourceLocation>,
}

impl Context {
    /// Borrow this context as a [`crate::ContextRef`].
    #[must_use]
    pub fn as_ref<'a>(&'a self, scene: &'a Scene) -> ContextRef<'a> {
        ContextRef {
            label: scene.label(self.label),
            source: self.source.as_ref().map(|source| SourceLocationRef {
                file: scene.file(source.file),
                line: source.line,
                column: source.column,
            }),
        }
    }
}

/// Human-readable context note attached to validation errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextNote {
    /// Human-readable label for the enclosing context.
    pub label: String,
    /// Optional source location.
    pub source: Option<ResolvedSourceLocation>,
}

/// Resolved source location attached to validation errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedSourceLocation {
    /// Source file name.
    pub file: String,
    /// 1-based source line.
    pub line: u32,
    /// 1-based source column.
    pub column: u32,
}

/// A retained mask definition.
#[derive(Clone, Debug, PartialEq)]
pub struct Mask {
    /// How this mask scene modulates masked content.
    pub mode: MaskMode,
    /// Scene that produces mask coverage values.
    pub scene: Scene,
}

impl Mask {
    /// Create a mask definition from a retained scene and interpretation mode.
    #[must_use]
    pub fn new(mode: MaskMode, scene: Scene) -> Self {
        Self { mode, scene }
    }
}

/// A use of a retained mask within an isolated group.
#[derive(Clone, Debug, PartialEq)]
pub struct AppliedMask {
    /// Referenced mask definition.
    pub mask: MaskId,
    /// Transform applied when realizing the mask scene.
    pub transform: Affine,
}

impl AppliedMask {
    /// Create a mask use with an identity transform.
    #[must_use]
    pub fn new(mask: MaskId) -> Self {
        Self {
            mask,
            transform: Affine::IDENTITY,
        }
    }
}

/// Parameters for an isolated group.
///
/// The group is rendered into an isolated buffer, then composited into its parent using
/// [`Group::composite`]. If `clip` is present, it is applied to the group's result at
/// composite time (isolated clip).
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// Optional isolated clip applied to the group result.
    pub clip: Option<Clip>,
    /// Optional retained mask applied to the group result before compositing.
    pub mask: Option<AppliedMask>,
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
            mask: None,
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
    pub style: Style,
    /// Positioned glyphs in the run.
    pub glyphs: Vec<Glyph>,
    /// Brush used for the run.
    pub brush: Brush,
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
            style: Style::Fill(Fill::NonZero),
            glyphs: Vec::new(),
            brush: Brush::Solid(peniko::Color::BLACK),
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
        fill_rule: Fill,
        /// Brush used by this draw.
        brush: Brush,
        /// Optional brush-space transform (for gradients/images).
        brush_transform: Option<Affine>,
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
        stroke: Stroke,
        /// Brush used by this draw.
        brush: Brush,
        /// Optional brush-space transform (for gradients/images).
        brush_transform: Option<Affine>,
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
    /// Push a context annotation onto the context stack.
    PushContext(ContextId),
    /// Pop the most recently pushed context annotation.
    PopContext,
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
    labels: Vec<Box<str>>,
    files: Vec<Box<str>>,
    contexts: Vec<Context>,
    clips: Vec<Clip>,
    masks: Vec<Mask>,
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
        self.labels.clear();
        self.files.clear();
        self.contexts.clear();
        self.clips.clear();
        self.masks.clear();
        self.groups.clear();
        self.draws.clear();
    }

    /// Reserve space for additional recorded payloads.
    ///
    /// This is useful when composing many retained scenes into one destination scene.
    #[inline]
    pub fn reserve_additional(
        &mut self,
        commands: usize,
        labels: usize,
        files: usize,
        contexts: usize,
        clips: usize,
        groups: usize,
        draws: usize,
        masks: usize,
    ) {
        self.commands.reserve(commands);
        self.labels.reserve(labels);
        self.files.reserve(files);
        self.contexts.reserve(contexts);
        self.clips.reserve(clips);
        self.masks.reserve(masks);
        self.groups.reserve(groups);
        self.draws.reserve(draws);
    }

    /// Reserve enough additional space to append another scene's current contents.
    #[inline]
    pub fn reserve_like(&mut self, other: &Self) {
        self.reserve_additional(
            other.commands.len(),
            other.labels.len(),
            other.files.len(),
            other.contexts.len(),
            other.clips.len(),
            other.groups.len(),
            other.draws.len(),
            other.masks.len(),
        );
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

    /// Resolve a context annotation by ID.
    #[inline]
    pub fn context(&self, id: ContextId) -> &Context {
        &self.contexts[id.0 as usize]
    }

    /// Resolve an interned label string by ID.
    #[inline]
    pub fn label(&self, id: LabelId) -> &str {
        &self.labels[id.0 as usize]
    }

    /// Resolve an interned file string by ID.
    #[inline]
    pub fn file(&self, id: FileId) -> &str {
        &self.files[id.0 as usize]
    }

    /// Resolve a mask payload by ID.
    #[inline]
    pub fn mask(&self, id: MaskId) -> &Mask {
        &self.masks[id.0 as usize]
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

    /// Push a context annotation.
    #[inline]
    pub fn push_context(
        &mut self,
        label: &str,
        source: Option<SourceLocationRef<'_>>,
    ) -> ContextId {
        let label = self.define_label(label);
        let source = source.map(|source| SourceLocation {
            file: self.define_file(source.file),
            line: source.line,
            column: source.column,
        });
        let idx = u32::try_from(self.contexts.len()).expect("scene context table overflow");
        let id = ContextId(idx);
        self.contexts.push(Context { label, source });
        self.commands.push(Command::PushContext(id));
        id
    }

    /// Pop the most recently pushed context annotation.
    #[inline]
    pub fn pop_context(&mut self) {
        self.commands.push(Command::PopContext);
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

    /// Define a reusable retained mask.
    #[inline]
    pub fn define_mask(&mut self, mask: Mask) -> MaskId {
        if let Some(idx) = self.masks.iter().position(|existing| existing == &mask) {
            return MaskId(u32::try_from(idx).expect("scene mask table overflow"));
        }
        let idx = u32::try_from(self.masks.len()).expect("scene mask table overflow");
        let id = MaskId(idx);
        self.masks.push(mask);
        id
    }

    fn define_label(&mut self, label: &str) -> LabelId {
        if let Some(idx) = self
            .labels
            .iter()
            .position(|existing| existing.as_ref() == label)
        {
            return LabelId(u32::try_from(idx).expect("scene label table overflow"));
        }
        let idx = u32::try_from(self.labels.len()).expect("scene label table overflow");
        self.labels.push(label.into());
        LabelId(idx)
    }

    fn define_file(&mut self, file: &str) -> FileId {
        if let Some(idx) = self
            .files
            .iter()
            .position(|existing| existing.as_ref() == file)
        {
            return FileId(u32::try_from(idx).expect("scene file table overflow"));
        }
        let idx = u32::try_from(self.files.len()).expect("scene file table overflow");
        self.files.push(file.into());
        FileId(idx)
    }

    pub(crate) fn context_notes_for_stack(&self, stack: &[ContextId]) -> Vec<ContextNote> {
        stack
            .iter()
            .map(|id| {
                let context = self.context(*id);
                ContextNote {
                    label: self.label(context.label).into(),
                    source: context
                        .source
                        .as_ref()
                        .map(|source| ResolvedSourceLocation {
                            file: self.file(source.file).into(),
                            line: source.line,
                            column: source.column,
                        }),
                }
            })
            .collect()
    }

    /// Validate well-nested stacks.
    pub fn validate(&self) -> Result<(), ValidateError> {
        let mut context_depth = 0_u32;
        let mut context_stack = Vec::new();
        let mut clip_depth = 0_u32;
        let mut group_depth = 0_u32;

        for mask in &self.masks {
            mask.scene
                .validate()
                .map_err(|err| ValidateError::InvalidMask(Box::new(err)))?;
        }

        for cmd in &self.commands {
            match cmd {
                Command::PushContext(id) => {
                    context_depth += 1;
                    context_stack.push(*id);
                }
                Command::PopContext => {
                    context_depth = context_depth.checked_sub(1).ok_or_else(|| {
                        ValidateError::UnbalancedPopContext {
                            contexts: self.context_notes_for_stack(&context_stack),
                        }
                    })?;
                    context_stack.pop();
                }
                Command::PushClip(_) => clip_depth += 1,
                Command::PopClip => {
                    clip_depth = clip_depth.checked_sub(1).ok_or_else(|| {
                        ValidateError::UnbalancedPopClip {
                            contexts: self.context_notes_for_stack(&context_stack),
                        }
                    })?;
                }
                Command::PushGroup(_) => group_depth += 1,
                Command::PopGroup => {
                    group_depth = group_depth.checked_sub(1).ok_or_else(|| {
                        ValidateError::UnbalancedPopGroup {
                            contexts: self.context_notes_for_stack(&context_stack),
                        }
                    })?;
                }
                Command::Draw(_) => {}
            }
        }

        if context_depth != 0 {
            return Err(ValidateError::UnclosedContexts {
                contexts: self.context_notes_for_stack(&context_stack),
            });
        }
        if clip_depth != 0 {
            return Err(ValidateError::UnclosedClips {
                contexts: self.context_notes_for_stack(&context_stack),
            });
        }
        if group_depth != 0 {
            return Err(ValidateError::UnclosedGroups {
                contexts: self.context_notes_for_stack(&context_stack),
            });
        }
        Ok(())
    }

    /// Append another retained scene with an extra transform applied to its commands.
    ///
    /// The extra transform prefixes clip transforms, draw transforms, group clip transforms, and
    /// Existing brush transforms are preserved as-is.
    pub fn append_transformed(&mut self, other: &Self, transform: Affine) {
        self.reserve_like(other);
        replay_transformed(other, self, transform);
    }
}

impl PaintSink for Scene {
    #[inline]
    fn push_context(&mut self, context: ContextRef<'_>) {
        let _ = Self::push_context(self, context.label, context.source);
    }

    #[inline]
    fn pop_context(&mut self) {
        Self::pop_context(self);
    }

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
        let group = group.into_owned_with(&mut |mask| self.define_mask(mask.to_owned()));
        let _ = Self::push_group(self, group);
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
    fn glyph_run(&mut self, draw: GlyphRunRef<'_>, glyphs: &mut dyn Iterator<Item = Glyph>) {
        let _ = Self::draw(self, Draw::GlyphRun(draw.to_owned(glyphs)));
    }

    #[inline]
    fn blurred_rounded_rect(&mut self, draw: BlurredRoundedRect) {
        let _ = Self::draw(self, Draw::BlurredRoundedRect(draw));
    }
}

/// Replay a recorded [`Scene`] into a [`crate::PaintSink`].
pub fn replay<S>(scene: &Scene, sink: &mut S)
where
    S: PaintSink + ?Sized,
{
    crate::paint::replay(scene, sink);
}

/// Replay a recorded [`Scene`] into a [`crate::PaintSink`] with an extra transform.
///
/// The extra transform prefixes clip transforms, draw transforms, group clip transforms, and
/// Existing brush transforms are preserved as-is.
pub fn replay_transformed<S>(scene: &Scene, sink: &mut S, transform: Affine)
where
    S: PaintSink + ?Sized,
{
    crate::paint::replay_transformed(scene, sink, transform);
}

/// Errors returned by [`Scene::validate`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidateError {
    /// A `PopClip` occurred without a matching prior `PushClip`.
    UnbalancedPopClip {
        /// Enclosing context stack active at the point of failure.
        contexts: Vec<ContextNote>,
    },
    /// A `PopGroup` occurred without a matching prior `PushGroup`.
    UnbalancedPopGroup {
        /// Enclosing context stack active at the point of failure.
        contexts: Vec<ContextNote>,
    },
    /// A `PopContext` occurred without a matching prior `PushContext`.
    UnbalancedPopContext {
        /// Enclosing context stack active at the point of failure.
        contexts: Vec<ContextNote>,
    },
    /// The command stream ended with open clips.
    UnclosedClips {
        /// Enclosing context stack still open at the end of validation.
        contexts: Vec<ContextNote>,
    },
    /// The command stream ended with open groups.
    UnclosedGroups {
        /// Enclosing context stack still open at the end of validation.
        contexts: Vec<ContextNote>,
    },
    /// The command stream ended with open contexts.
    UnclosedContexts {
        /// Enclosing context stack still open at the end of validation.
        contexts: Vec<ContextNote>,
    },
    /// A retained mask definition was invalid.
    InvalidMask(Box<Self>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MaskRef;
    use alloc::sync::Arc;
    use alloc::vec;

    #[test]
    fn validate_balanced() {
        let mut s = Scene::new();
        let _clip = s.push_clip(Clip::Fill {
            transform: Affine::IDENTITY,
            shape: Geometry::Rect(Rect::new(0.0, 0.0, 10.0, 10.0)),
            fill_rule: Fill::NonZero,
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
        assert_eq!(
            s.validate(),
            Err(ValidateError::UnbalancedPopClip {
                contexts: Vec::new()
            })
        );
    }

    #[test]
    fn replay_round_trip() {
        let mut a = Scene::new();
        let _ = a.push_clip(Clip::Fill {
            transform: Affine::IDENTITY,
            shape: Geometry::Rect(Rect::new(0.0, 0.0, 1.0, 1.0)),
            fill_rule: Fill::NonZero,
        });
        let _ = a.push_group(Group::default());
        a.draw(Draw::Fill {
            transform: Affine::IDENTITY,
            fill_rule: Fill::NonZero,
            brush: Brush::Solid(peniko::Color::WHITE),
            brush_transform: None,
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
            style: Style::Fill(Fill::NonZero),
            glyphs: vec![Glyph {
                id: 7,
                x: 0.0,
                y: 0.0,
            }],
            brush: Brush::Solid(peniko::Color::BLACK),
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

    #[test]
    fn append_transformed_reserves_and_appends() {
        let mut source = Scene::new();
        source.draw(Draw::Fill {
            transform: Affine::translate((1.0, 2.0)),
            fill_rule: Fill::NonZero,
            brush: Brush::Solid(peniko::Color::WHITE),
            brush_transform: None,
            shape: Geometry::Rect(Rect::new(0.0, 0.0, 3.0, 4.0)),
            composite: Composite::default(),
        });

        let mut dest = Scene::new();
        dest.reserve_like(&source);
        dest.append_transformed(&source, Affine::translate((5.0, 6.0)));

        assert_eq!(dest.commands().len(), 1);
        assert_eq!(
            dest.draw_op(DrawId(0)),
            &Draw::Fill {
                transform: Affine::translate((6.0, 8.0)),
                fill_rule: Fill::NonZero,
                brush: Brush::Solid(peniko::Color::WHITE),
                brush_transform: None,
                shape: Geometry::Rect(Rect::new(0.0, 0.0, 3.0, 4.0)),
                composite: Composite::default(),
            }
        );
    }

    #[test]
    fn append_transformed_preserves_context_annotations() {
        let mut source = Scene::new();
        source.push_context(
            "toolbar/button",
            Some(SourceLocationRef::new("widgets.rs", 9, 2)),
        );
        source.draw(Draw::Fill {
            transform: Affine::translate((1.0, 2.0)),
            fill_rule: Fill::NonZero,
            brush: Brush::Solid(peniko::Color::WHITE),
            brush_transform: None,
            shape: Geometry::Rect(Rect::new(0.0, 0.0, 3.0, 4.0)),
            composite: Composite::default(),
        });
        source.pop_context();

        let mut dest = Scene::new();
        dest.reserve_like(&source);
        dest.append_transformed(&source, Affine::translate((5.0, 6.0)));

        assert_eq!(
            dest.commands(),
            &[
                Command::PushContext(ContextId(0)),
                Command::Draw(DrawId(0)),
                Command::PopContext,
            ]
        );
        assert_eq!(
            dest.label(dest.context(ContextId(0)).label),
            "toolbar/button"
        );
        let source = dest
            .context(ContextId(0))
            .source
            .as_ref()
            .expect("expected source location");
        assert_eq!(dest.file(source.file), "widgets.rs");
        assert_eq!(
            dest.draw_op(DrawId(0)),
            &Draw::Fill {
                transform: Affine::translate((6.0, 8.0)),
                fill_rule: Fill::NonZero,
                brush: Brush::Solid(peniko::Color::WHITE),
                brush_transform: None,
                shape: Geometry::Rect(Rect::new(0.0, 0.0, 3.0, 4.0)),
                composite: Composite::default(),
            }
        );
    }

    #[test]
    fn validate_rejects_invalid_nested_mask_scene() {
        let mut invalid_mask = Scene::new();
        invalid_mask.pop_clip();

        let mut scene = Scene::new();
        scene.define_mask(Mask::new(MaskMode::Alpha, invalid_mask));

        assert_eq!(
            scene.validate(),
            Err(ValidateError::InvalidMask(Box::new(
                ValidateError::UnbalancedPopClip {
                    contexts: Vec::new()
                }
            )))
        );
    }

    #[test]
    fn replay_round_trip_preserves_context_annotations() {
        let mut source = Scene::new();
        source.push_context(
            "toolbar/button",
            Some(SourceLocationRef::new("widgets.rs", 12, 8)),
        );
        source.draw(Draw::Fill {
            transform: Affine::IDENTITY,
            fill_rule: Fill::NonZero,
            brush: Brush::Solid(peniko::Color::WHITE),
            brush_transform: None,
            shape: Geometry::Rect(Rect::new(0.0, 0.0, 1.0, 1.0)),
            composite: Composite::default(),
        });
        source.pop_context();

        let mut replayed = Scene::new();
        replay(&source, &mut replayed);

        assert_eq!(source, replayed);
    }

    #[test]
    fn validate_reports_active_context_stack() {
        let mut scene = Scene::new();
        scene.push_context("window", None);
        scene.push_context(
            "toolbar/button",
            Some(SourceLocationRef::new("widgets.rs", 42, 9)),
        );
        scene.pop_clip();

        assert_eq!(
            scene.validate(),
            Err(ValidateError::UnbalancedPopClip {
                contexts: vec![
                    ContextNote {
                        label: "window".into(),
                        source: None,
                    },
                    ContextNote {
                        label: "toolbar/button".into(),
                        source: Some(ResolvedSourceLocation {
                            file: "widgets.rs".into(),
                            line: 42,
                            column: 9,
                        }),
                    },
                ],
            })
        );
    }

    #[test]
    fn recording_groups_with_same_mask_ref_shares_one_mask_definition() {
        let mut mask_scene = Scene::new();
        mask_scene.draw(Draw::Fill {
            transform: Affine::IDENTITY,
            fill_rule: Fill::NonZero,
            brush: Brush::Solid(peniko::Color::WHITE),
            brush_transform: None,
            shape: Geometry::Rect(Rect::new(0.0, 0.0, 4.0, 4.0)),
            composite: Composite::default(),
        });

        let mut scene = Scene::new();
        let mask = MaskRef::new(MaskMode::Alpha, &mask_scene);
        let group = GroupRef::new().with_mask(mask);

        PaintSink::push_group(&mut scene, group.clone());
        scene.pop_group();
        PaintSink::push_group(&mut scene, group);
        scene.pop_group();

        assert_eq!(scene.masks.len(), 1);
        let first = scene
            .group(GroupId(0))
            .mask
            .as_ref()
            .expect("expected mask")
            .mask;
        let second = scene
            .group(GroupId(1))
            .mask
            .as_ref()
            .expect("expected mask")
            .mask;
        assert_eq!(first, second);
    }

    #[test]
    fn append_transformed_prefixes_group_mask_transform() {
        let mut mask = Scene::new();
        mask.draw(Draw::Fill {
            transform: Affine::IDENTITY,
            fill_rule: Fill::NonZero,
            brush: Brush::Solid(peniko::Color::WHITE),
            brush_transform: None,
            shape: Geometry::Rect(Rect::new(0.0, 0.0, 4.0, 4.0)),
            composite: Composite::default(),
        });
        let mut source = Scene::new();
        let mask_id = source.define_mask(Mask::new(MaskMode::Luminance, mask));
        let group = Group {
            mask: Some(AppliedMask {
                mask: mask_id,
                transform: Affine::translate((1.0, 2.0)),
            }),
            ..Group::default()
        };
        source.push_group(group);
        source.draw(Draw::Fill {
            transform: Affine::IDENTITY,
            fill_rule: Fill::NonZero,
            brush: Brush::Solid(peniko::Color::BLACK),
            brush_transform: None,
            shape: Geometry::Rect(Rect::new(1.0, 1.0, 3.0, 3.0)),
            composite: Composite::default(),
        });
        source.pop_group();

        let mut dest = Scene::new();
        dest.reserve_like(&source);
        dest.append_transformed(&source, Affine::translate((5.0, 6.0)));

        let group = dest.group(GroupId(0));
        let mask = group.mask.as_ref().expect("expected group mask");
        assert_eq!(mask.transform, Affine::translate((6.0, 8.0)));
        assert_eq!(dest.masks.len(), 1);
        assert_eq!(dest.mask(mask.mask).mode, MaskMode::Luminance);
    }
}
