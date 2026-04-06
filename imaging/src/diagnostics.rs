// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Advisory diagnostics for `imaging` command streams and retained scenes.
//!
//! Unlike [`crate::record::Scene::validate`], diagnostics report suspicious or wasteful patterns
//! that are still structurally valid.
//!
//! Use [`DiagnosingSink`] when you want stream-local findings while painting into another sink.
//! [`crate::record::Scene::diagnose`] is the retained convenience entry point; it replays the
//! scene into a `DiagnosingSink` and may add retained-only findings in the future.

use alloc::vec::Vec;

use peniko::BrushRef;

use crate::{
    BlurredRoundedRect, ClipRef, Composite, ContextRef, FillRef, GlyphRunRef, GroupRef, PaintSink,
    SourceLocationRef, StrokeRef,
    record::{ContextNote, ResolvedSourceLocation, Scene},
};

/// Scope of a diagnostic kind.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DiagnosticScope {
    /// The finding can be produced while streaming commands through a [`PaintSink`].
    StreamLocal,
    /// The finding requires retained-scene inspection and only appears through
    /// [`crate::record::Scene::diagnose`].
    RetainedScene,
}

/// Severity of a diagnostic.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DiagnosticLevel {
    /// Suspicious or wasteful content that is still structurally valid.
    Warning,
}

/// Kind of diagnostic reported by [`DiagnosingSink`] or [`Scene::diagnose`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DiagnosticKind {
    /// A context scope contains no draw commands.
    EmptyContext,
    /// A clip scope contains no draw commands.
    EmptyClip,
    /// An isolated group contains no draw commands.
    EmptyGroup,
    /// A draw is fully transparent and will not contribute visible output.
    TransparentDraw,
    /// An isolated group has no clip, no mask, no filters, and default compositing.
    IdentityGroup,
    /// A group references a retained mask scene that draws nothing.
    EmptyMaskScene,
    /// A path-backed clip or draw contains no path elements.
    EmptyPath,
    /// A stroke-backed clip or draw has zero width.
    ZeroWidthStroke,
    /// A blurred rounded rectangle uses a zero standard deviation.
    ZeroBlurredRoundedRect,
}

impl DiagnosticKind {
    /// Return whether this finding can be produced during streaming or requires retained-scene
    /// inspection.
    #[must_use]
    pub const fn scope(self) -> DiagnosticScope {
        match self {
            Self::EmptyContext
            | Self::EmptyClip
            | Self::EmptyGroup
            | Self::TransparentDraw
            | Self::IdentityGroup
            | Self::EmptyMaskScene
            | Self::EmptyPath
            | Self::ZeroWidthStroke
            | Self::ZeroBlurredRoundedRect => DiagnosticScope::StreamLocal,
        }
    }
}

/// A non-fatal finding reported by [`DiagnosingSink`] or [`Scene::diagnose`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Severity of the finding.
    pub level: DiagnosticLevel,
    /// Machine-readable kind.
    pub kind: DiagnosticKind,
    /// Command index associated with the finding.
    pub command_index: u32,
    /// Active context stack at the point of the finding.
    pub contexts: Vec<ContextNote>,
}

#[derive(Copy, Clone, Debug)]
struct ScopeFrame {
    command_index: u32,
    draw_count: u32,
}

#[derive(Copy, Clone, Debug)]
struct GroupFrame {
    command_index: u32,
    draw_count: u32,
    is_identity: bool,
}

/// A [`PaintSink`] wrapper that collects advisory diagnostics while forwarding commands.
///
/// This is the stream-local counterpart to [`Scene::diagnose`]. It observes the same borrowed
/// command stream a backend sink would receive and records non-fatal findings such as empty scopes,
/// transparent draws, and other no-op or suspicious constructs.
#[derive(Debug)]
pub struct DiagnosingSink<S> {
    inner: S,
    diagnostics: Vec<Diagnostic>,
    command_index: u32,
    draw_count: u32,
    context_stack: Vec<ContextNote>,
    context_frames: Vec<ScopeFrame>,
    clip_frames: Vec<ScopeFrame>,
    group_frames: Vec<GroupFrame>,
}

impl<S> DiagnosingSink<S> {
    /// Wrap a sink and collect diagnostics while forwarding commands to it.
    #[must_use]
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            diagnostics: Vec::new(),
            command_index: 0,
            draw_count: 0,
            context_stack: Vec::new(),
            context_frames: Vec::new(),
            clip_frames: Vec::new(),
            group_frames: Vec::new(),
        }
    }

    /// Borrow the wrapped sink.
    #[must_use]
    pub fn inner(&self) -> &S {
        &self.inner
    }

    /// Mutably borrow the wrapped sink.
    #[must_use]
    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.inner
    }

    /// Borrow the diagnostics accumulated so far.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Unwrap the sink, returning the wrapped sink and all collected diagnostics.
    #[must_use]
    pub fn into_inner(self) -> (S, Vec<Diagnostic>) {
        (self.inner, self.diagnostics)
    }

    fn push_diagnostic(&mut self, kind: DiagnosticKind) {
        self.diagnostics.push(Diagnostic {
            level: DiagnosticLevel::Warning,
            kind,
            command_index: self.command_index,
            contexts: self.context_stack.clone(),
        });
    }

    fn finish_command(&mut self) {
        self.command_index = self
            .command_index
            .checked_add(1)
            .expect("diagnostic command stream overflow");
    }
}

impl<S> PaintSink for DiagnosingSink<S>
where
    S: PaintSink,
{
    fn push_context(&mut self, context: ContextRef<'_>) {
        self.context_stack.push(context_note(context));
        self.context_frames.push(ScopeFrame {
            command_index: self.command_index,
            draw_count: self.draw_count,
        });
        self.inner.push_context(context);
        self.finish_command();
    }

    fn pop_context(&mut self) {
        if let Some(frame) = self.context_frames.pop()
            && frame.draw_count == self.draw_count
        {
            self.push_diagnostic(DiagnosticKind::EmptyContext);
            self.diagnostics
                .last_mut()
                .expect("just pushed diagnostic")
                .command_index = frame.command_index;
        }
        self.inner.pop_context();
        self.context_stack.pop();
        self.finish_command();
    }

    fn push_clip(&mut self, clip: ClipRef<'_>) {
        if clip_uses_empty_path(&clip) {
            self.push_diagnostic(DiagnosticKind::EmptyPath);
        }
        if clip_uses_zero_width_stroke(&clip) {
            self.push_diagnostic(DiagnosticKind::ZeroWidthStroke);
        }
        self.clip_frames.push(ScopeFrame {
            command_index: self.command_index,
            draw_count: self.draw_count,
        });
        self.inner.push_clip(clip);
        self.finish_command();
    }

    fn pop_clip(&mut self) {
        if let Some(frame) = self.clip_frames.pop()
            && frame.draw_count == self.draw_count
        {
            self.push_diagnostic(DiagnosticKind::EmptyClip);
            self.diagnostics
                .last_mut()
                .expect("just pushed diagnostic")
                .command_index = frame.command_index;
        }
        self.inner.pop_clip();
        self.finish_command();
    }

    fn push_group(&mut self, group: GroupRef<'_>) {
        if let Some(mask) = &group.mask
            && !scene_has_any_draw(mask.mask.scene)
        {
            self.push_diagnostic(DiagnosticKind::EmptyMaskScene);
        }
        self.group_frames.push(GroupFrame {
            command_index: self.command_index,
            draw_count: self.draw_count,
            is_identity: group_is_identity(&group),
        });
        self.inner.push_group(group);
        self.finish_command();
    }

    fn pop_group(&mut self) {
        if let Some(frame) = self.group_frames.pop() {
            if frame.draw_count == self.draw_count {
                self.push_diagnostic(DiagnosticKind::EmptyGroup);
                self.diagnostics
                    .last_mut()
                    .expect("just pushed diagnostic")
                    .command_index = frame.command_index;
            } else if frame.is_identity {
                self.push_diagnostic(DiagnosticKind::IdentityGroup);
                self.diagnostics
                    .last_mut()
                    .expect("just pushed diagnostic")
                    .command_index = frame.command_index;
            }
        }
        self.inner.pop_group();
        self.finish_command();
    }

    fn fill(&mut self, draw: FillRef<'_>) {
        self.draw_count += 1;
        if draw_uses_empty_path_fill(&draw) {
            self.push_diagnostic(DiagnosticKind::EmptyPath);
        }
        if draw_is_fully_transparent_fill(&draw) {
            self.push_diagnostic(DiagnosticKind::TransparentDraw);
        }
        self.inner.fill(draw);
        self.finish_command();
    }

    fn stroke(&mut self, draw: StrokeRef<'_>) {
        self.draw_count += 1;
        if draw_uses_empty_path_stroke(&draw) {
            self.push_diagnostic(DiagnosticKind::EmptyPath);
        }
        if draw.stroke.width <= 0.0 {
            self.push_diagnostic(DiagnosticKind::ZeroWidthStroke);
        }
        if draw_is_fully_transparent_stroke(&draw) {
            self.push_diagnostic(DiagnosticKind::TransparentDraw);
        }
        self.inner.stroke(draw);
        self.finish_command();
    }

    fn glyph_run(
        &mut self,
        draw: GlyphRunRef<'_>,
        glyphs: &mut dyn Iterator<Item = crate::record::Glyph>,
    ) {
        self.draw_count += 1;
        if draw_is_fully_transparent_glyph_run(&draw) {
            self.push_diagnostic(DiagnosticKind::TransparentDraw);
        }
        self.inner.glyph_run(draw, glyphs);
        self.finish_command();
    }

    fn blurred_rounded_rect(&mut self, draw: BlurredRoundedRect) {
        self.draw_count += 1;
        if draw.std_dev == 0.0 {
            self.push_diagnostic(DiagnosticKind::ZeroBlurredRoundedRect);
        }
        if draw.composite.alpha <= 0.0 || draw.color.components[3] <= 0.0 {
            self.push_diagnostic(DiagnosticKind::TransparentDraw);
        }
        self.inner.blurred_rounded_rect(draw);
        self.finish_command();
    }
}

impl Scene {
    /// Analyze the retained command stream for suspicious but still structurally valid patterns.
    ///
    /// Diagnostics whose [`DiagnosticKind::scope`] is [`DiagnosticScope::StreamLocal`] are
    /// produced by replaying this scene into a [`DiagnosingSink`]. Future
    /// [`DiagnosticScope::RetainedScene`] findings may be appended here without affecting the
    /// streaming API.
    #[must_use]
    pub fn diagnose(&self) -> Vec<Diagnostic> {
        let mut sink = DiagnosingSink::new(NullSink);
        crate::record::replay(self, &mut sink);
        let (_, diagnostics) = sink.into_inner();
        diagnostics
    }
}

fn context_note(context: ContextRef<'_>) -> ContextNote {
    ContextNote {
        label: context.label.into(),
        source: context.source.map(resolved_source_location),
    }
}

fn resolved_source_location(source: SourceLocationRef<'_>) -> ResolvedSourceLocation {
    ResolvedSourceLocation {
        file: source.file.into(),
        line: source.line,
        column: source.column,
    }
}

fn scene_has_any_draw(scene: &Scene) -> bool {
    scene
        .commands()
        .iter()
        .any(|command| matches!(command, crate::record::Command::Draw(_)))
}

fn group_is_identity(group: &GroupRef<'_>) -> bool {
    group.clip.is_none()
        && group.mask.is_none()
        && group.filters.is_empty()
        && group.composite == Composite::default()
}

fn clip_uses_empty_path(clip: &ClipRef<'_>) -> bool {
    match clip {
        ClipRef::Fill { shape, .. } | ClipRef::Stroke { shape, .. } => {
            geometry_ref_is_empty_path(shape)
        }
    }
}

fn clip_uses_zero_width_stroke(clip: &ClipRef<'_>) -> bool {
    match clip {
        ClipRef::Fill { .. } => false,
        ClipRef::Stroke { stroke, .. } => stroke.width <= 0.0,
    }
}

fn draw_uses_empty_path_fill(draw: &FillRef<'_>) -> bool {
    geometry_ref_is_empty_path(&draw.shape)
}

fn draw_uses_empty_path_stroke(draw: &StrokeRef<'_>) -> bool {
    geometry_ref_is_empty_path(&draw.shape)
}

fn geometry_ref_is_empty_path(geometry: &crate::GeometryRef<'_>) -> bool {
    match geometry {
        crate::GeometryRef::Path(path) => path.is_empty(),
        crate::GeometryRef::OwnedPath(path) => path.is_empty(),
        crate::GeometryRef::Rect(_) | crate::GeometryRef::RoundedRect(_) => false,
    }
}

fn draw_is_fully_transparent_fill(draw: &FillRef<'_>) -> bool {
    draw.composite.alpha <= 0.0 || brush_ref_is_fully_transparent(draw.brush)
}

fn draw_is_fully_transparent_stroke(draw: &StrokeRef<'_>) -> bool {
    draw.composite.alpha <= 0.0 || brush_ref_is_fully_transparent(draw.brush)
}

fn draw_is_fully_transparent_glyph_run(draw: &GlyphRunRef<'_>) -> bool {
    draw.composite.alpha <= 0.0 || brush_ref_is_fully_transparent(draw.brush)
}

fn brush_ref_is_fully_transparent(brush: BrushRef<'_>) -> bool {
    match brush {
        BrushRef::Solid(color) => color.components[3] <= 0.0,
        BrushRef::Gradient(_) | BrushRef::Image(_) => false,
    }
}

#[derive(Copy, Clone, Debug, Default)]
struct NullSink;

impl PaintSink for NullSink {
    fn push_clip(&mut self, _clip: ClipRef<'_>) {}

    fn pop_clip(&mut self) {}

    fn push_group(&mut self, _group: GroupRef<'_>) {}

    fn pop_group(&mut self) {}

    fn fill(&mut self, _draw: FillRef<'_>) {}

    fn stroke(&mut self, _draw: StrokeRef<'_>) {}

    fn glyph_run(
        &mut self,
        _draw: GlyphRunRef<'_>,
        _glyphs: &mut dyn Iterator<Item = crate::record::Glyph>,
    ) {
    }

    fn blurred_rounded_rect(&mut self, _draw: BlurredRoundedRect) {}
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use kurbo::{Affine, BezPath, Rect, Stroke};
    use peniko::{Brush, Color, Fill};

    use super::*;
    use crate::Composite;
    use crate::MaskMode;
    use crate::Painter;
    use crate::record::{
        AppliedMask, Clip, ContextId, Draw, Geometry, Group, Mask, ResolvedSourceLocation,
    };

    #[test]
    fn diagnose_reports_empty_scopes_and_transparent_draws() {
        let mut scene = Scene::new();
        scene.push_context(
            "toolbar/button",
            Some(SourceLocationRef::new("widgets.rs", 3, 1)),
        );
        scene.push_clip(Clip::Fill {
            transform: Affine::IDENTITY,
            shape: Geometry::Rect(Rect::new(0.0, 0.0, 4.0, 4.0)),
            fill_rule: Fill::NonZero,
        });
        scene.pop_clip();
        scene.push_group(Group::default());
        scene.pop_group();
        scene.draw(Draw::Fill {
            transform: Affine::IDENTITY,
            fill_rule: Fill::NonZero,
            brush: Brush::Solid(Color::TRANSPARENT),
            brush_transform: None,
            shape: Geometry::Rect(Rect::new(0.0, 0.0, 4.0, 4.0)),
            composite: Composite::default(),
        });
        scene.pop_context();

        let diagnostics = scene.diagnose();
        assert_eq!(diagnostics.len(), 3);
        assert_eq!(diagnostics[0].kind, DiagnosticKind::EmptyClip);
        assert_eq!(diagnostics[1].kind, DiagnosticKind::EmptyGroup);
        assert_eq!(diagnostics[2].kind, DiagnosticKind::TransparentDraw);
        for diagnostic in diagnostics {
            assert_eq!(diagnostic.kind.scope(), DiagnosticScope::StreamLocal);
            assert_eq!(
                diagnostic.contexts,
                vec![ContextNote {
                    label: "toolbar/button".into(),
                    source: Some(ResolvedSourceLocation {
                        file: "widgets.rs".into(),
                        line: 3,
                        column: 1,
                    }),
                }]
            );
        }
    }

    #[test]
    fn diagnose_reports_empty_context() {
        let mut scene = Scene::new();
        scene.push_context("empty", None);
        scene.pop_context();

        assert_eq!(
            scene.diagnose(),
            vec![Diagnostic {
                level: DiagnosticLevel::Warning,
                kind: DiagnosticKind::EmptyContext,
                command_index: 0,
                contexts: vec![ContextNote {
                    label: "empty".into(),
                    source: None,
                }],
            }]
        );
    }

    #[test]
    fn diagnose_is_empty_for_normal_scene() {
        let mut scene = Scene::new();
        scene.push_context("filled", None);
        scene.draw(Draw::Fill {
            transform: Affine::IDENTITY,
            fill_rule: Fill::NonZero,
            brush: Brush::Solid(Color::WHITE),
            brush_transform: None,
            shape: Geometry::Rect(Rect::new(0.0, 0.0, 4.0, 4.0)),
            composite: Composite::default(),
        });
        scene.pop_context();

        assert!(scene.diagnose().is_empty());
    }

    #[test]
    fn diagnose_preserves_context_ids_from_recording() {
        let mut scene = Scene::new();
        scene.push_context("a", None);
        scene.push_context("b", None);
        scene.pop_context();
        scene.pop_context();

        assert_eq!(scene.context(ContextId(0)).label.0, 0);
        assert_eq!(scene.context(ContextId(1)).label.0, 1);
    }

    #[test]
    fn diagnose_reports_identity_groups_empty_masks_and_no_op_draw_shapes() {
        let mut scene = Scene::new();
        scene.push_context("scoped", None);

        let empty_mask = scene.define_mask(Mask::new(MaskMode::Luminance, Scene::new()));
        scene.push_group(Group {
            mask: Some(AppliedMask::new(empty_mask)),
            ..Group::default()
        });
        scene.draw(Draw::Fill {
            transform: Affine::IDENTITY,
            fill_rule: Fill::NonZero,
            brush: Brush::Solid(Color::WHITE),
            brush_transform: None,
            shape: Geometry::Rect(Rect::new(0.0, 0.0, 4.0, 4.0)),
            composite: Composite::default(),
        });
        scene.pop_group();

        scene.push_group(Group::default());
        scene.draw(Draw::Fill {
            transform: Affine::IDENTITY,
            fill_rule: Fill::NonZero,
            brush: Brush::Solid(Color::WHITE),
            brush_transform: None,
            shape: Geometry::Rect(Rect::new(0.0, 0.0, 4.0, 4.0)),
            composite: Composite::default(),
        });
        scene.pop_group();

        scene.push_clip(Clip::Stroke {
            transform: Affine::IDENTITY,
            shape: Geometry::Path(BezPath::new()),
            stroke: Stroke::new(0.0),
        });
        scene.pop_clip();

        scene.draw(Draw::Stroke {
            transform: Affine::IDENTITY,
            stroke: Stroke::new(0.0),
            brush: Brush::Solid(Color::WHITE),
            brush_transform: None,
            shape: Geometry::Path(BezPath::new()),
            composite: Composite::default(),
        });
        scene.draw(Draw::BlurredRoundedRect(BlurredRoundedRect {
            transform: Affine::IDENTITY,
            rect: Rect::new(0.0, 0.0, 8.0, 8.0),
            color: Color::WHITE,
            radius: 1.0,
            std_dev: 0.0,
            composite: Composite::default(),
        }));
        scene.pop_context();

        let diagnostics = scene.diagnose();
        assert!(
            diagnostics
                .iter()
                .any(|d| d.kind == DiagnosticKind::EmptyMaskScene)
        );
        assert!(
            diagnostics
                .iter()
                .any(|d| d.kind == DiagnosticKind::IdentityGroup)
        );
        assert!(
            diagnostics
                .iter()
                .filter(|d| d.kind == DiagnosticKind::EmptyPath)
                .count()
                >= 2
        );
        assert!(
            diagnostics
                .iter()
                .filter(|d| d.kind == DiagnosticKind::ZeroWidthStroke)
                .count()
                >= 2
        );
        assert!(
            diagnostics
                .iter()
                .any(|d| d.kind == DiagnosticKind::ZeroBlurredRoundedRect)
        );
        assert!(diagnostics.iter().all(|d| {
            d.contexts
                == vec![ContextNote {
                    label: "scoped".into(),
                    source: None,
                }]
        }));
    }

    #[test]
    fn diagnosing_sink_collects_stream_local_findings_while_forwarding() {
        let mut sink = DiagnosingSink::new(Scene::new());

        {
            let mut painter = Painter::new(&mut sink);
            painter.with_context("ctx", None, |p| {
                p.fill(Geometry::Path(BezPath::new()), Color::TRANSPARENT)
                    .draw();
            });
        }

        let (scene, diagnostics) = sink.into_inner();
        assert_eq!(scene.commands().len(), 3);
        assert_eq!(
            diagnostics.iter().map(|d| d.kind).collect::<Vec<_>>(),
            vec![DiagnosticKind::EmptyPath, DiagnosticKind::TransparentDraw,]
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.kind.scope() == DiagnosticScope::StreamLocal)
        );
    }
}
