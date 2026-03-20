// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `imaging`: backend-agnostic 2D imaging IR + recorder.
//!
//! This crate provides:
//! - A lightweight, backend-agnostic command stream ([`Scene`]).
//! - Explicit semantics for:
//!   - **Non-isolated clipping** via a dedicated clip stack ([`Command::PushClip`]/[`Command::PopClip`]).
//!   - **Isolated compositing** via groups ([`Command::PushGroup`]/[`Command::PopGroup`]).
//!   - **Per-draw compositing** via [`Composite`] (blend mode + global alpha).
//!
//! The API is intentionally small and experimental; expect breaking changes while we iterate.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use kurbo::{Affine, BezPath, Rect, RoundedRect, Shape as _, Stroke};
use peniko::{BlendMode, Brush, Fill, FontData, Style};

mod paint;
mod painter;
pub mod validation;

pub use paint::{
    ClipRef, DrawRef, FillRef, GeometryRef, GlyphRunRef, GroupRef, PaintSink, StrokeRef, replay,
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

/// Glyph drawing style used by [`GlyphRun`].
pub type GlyphStyle = Style;

/// Normalized variable-font coordinate value.
pub type NormalizedCoord = i16;

/// Description of a filter effect applied to an isolated group.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Filter {
    /// Fill the group output with a solid color (aka `feFlood`).
    ///
    /// This ignores the group’s source content. It is still affected by the group’s isolated clip.
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

/// A geometry payload.
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

/// Canvas-style compositing state.
///
/// This corresponds to HTML Canvas 2D's `globalCompositeOperation` (blend) plus `globalAlpha` (alpha),
/// applied **per draw**.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Composite {
    /// Blend mode used by the draw.
    pub blend: BlendMode,
    /// Alpha multiplier in `0..=1`.
    pub alpha: f32,
}

impl Composite {
    /// A convenience constructor.
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

/// A clip operation pushed onto the non-isolated clip stack.
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
pub struct ClipId(u32);

/// Identifier for a group payload stored in a [`Scene`].
#[repr(transparent)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct GroupId(u32);

/// Identifier for a draw payload stored in a [`Scene`].
#[repr(transparent)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct DrawId(u32);

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
    pub filters: Vec<Filter>,
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

/// Builder for recording a glyph run into a [`Scene`].
#[must_use = "Methods on the builder don't do anything until `draw` is called."]
#[derive(Debug)]
pub struct DrawGlyphs<'a> {
    scene: &'a mut Scene,
    glyph_run: GlyphRun,
}

impl<'a> DrawGlyphs<'a> {
    fn new(scene: &'a mut Scene, font: &FontData) -> Self {
        Self {
            scene,
            glyph_run: GlyphRun::new(font.clone()),
        }
    }

    /// Set the global transform applied to the run.
    pub fn transform(mut self, transform: Affine) -> Self {
        self.glyph_run.transform = transform;
        self
    }

    /// Set the per-glyph transform applied before the glyph offset translation.
    pub fn glyph_transform(mut self, transform: Option<Affine>) -> Self {
        self.glyph_run.glyph_transform = transform;
        self
    }

    /// Set the font size in pixels per em.
    pub fn font_size(mut self, size: f32) -> Self {
        self.glyph_run.font_size = size;
        self
    }

    /// Set whether hinting is enabled.
    pub fn hint(mut self, hint: bool) -> Self {
        self.glyph_run.hint = hint;
        self
    }

    /// Set normalized variation coordinates for a variable font instance.
    pub fn normalized_coords(mut self, coords: &[NormalizedCoord]) -> Self {
        self.glyph_run.normalized_coords.clear();
        self.glyph_run.normalized_coords.extend_from_slice(coords);
        self
    }

    /// Set the brush used for the run.
    pub fn brush(mut self, brush: impl Into<Paint>) -> Self {
        self.glyph_run.paint = brush.into();
        self
    }

    /// Set an extra alpha multiplier for the run's brush.
    pub fn brush_alpha(mut self, alpha: f32) -> Self {
        self.glyph_run.composite.alpha = alpha.clamp(0.0, 1.0);
        self
    }

    /// Set the full compositing state for the run.
    pub fn composite(mut self, composite: Composite) -> Self {
        self.glyph_run.composite = composite;
        self
    }

    /// Record the glyph run into the scene.
    pub fn draw(
        mut self,
        style: impl Into<GlyphStyle>,
        glyphs: impl IntoIterator<Item = Glyph>,
    ) -> DrawId {
        self.glyph_run.style = style.into();
        self.glyph_run.glyphs.clear();
        self.glyph_run.glyphs.extend(glyphs);
        self.scene.draw(Draw::GlyphRun(self.glyph_run))
    }
}

/// A drawing command that produces pixels.
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

/// A reusable, backend-agnostic sequence of imaging commands.
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

    /// Returns a builder for recording a glyph run.
    #[inline]
    pub fn draw_glyphs(&mut self, font: &FontData) -> DrawGlyphs<'_> {
        DrawGlyphs::new(self, font)
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
        let _ = a.draw_glyphs(&font).font_size(12.0).draw(
            GlyphStyle::Fill(FillRule::NonZero),
            [Glyph {
                id: 7,
                x: 0.0,
                y: 0.0,
            }],
        );
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
