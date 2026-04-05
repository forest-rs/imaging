// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Advisory diagnostics for retained `imaging` scenes.
//!
//! Unlike [`crate::record::Scene::validate`], diagnostics report suspicious or wasteful patterns
//! that are still structurally valid.

use alloc::vec::Vec;

use peniko::Brush;

use crate::{
    Composite,
    record::{Clip, Command, ContextNote, Draw, DrawId, Geometry, Scene},
};

/// Severity of a retained-scene diagnostic.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DiagnosticLevel {
    /// Suspicious or wasteful content that is still structurally valid.
    Warning,
}

/// Kind of retained-scene diagnostic reported by [`Scene::diagnose`].
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

/// A non-fatal retained-scene finding reported by [`Scene::diagnose`].
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
struct ScopeDiagnosticFrame {
    command_index: u32,
    draw_count: u32,
}

#[derive(Copy, Clone, Debug)]
struct GroupDiagnosticFrame {
    command_index: u32,
    draw_count: u32,
    is_identity: bool,
}

impl Scene {
    /// Analyze the retained command stream for suspicious but still structurally valid patterns.
    #[must_use]
    pub fn diagnose(&self) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let mut context_stack = Vec::new();
        let mut context_frames = Vec::new();
        let mut clip_frames = Vec::new();
        let mut group_frames = Vec::new();
        let mut draw_count = 0_u32;

        for (command_index, cmd) in self.commands().iter().enumerate() {
            let command_index =
                u32::try_from(command_index).expect("scene command stream overflow");
            match *cmd {
                Command::PushContext(id) => {
                    context_stack.push(id);
                    context_frames.push(ScopeDiagnosticFrame {
                        command_index,
                        draw_count,
                    });
                }
                Command::PopContext => {
                    if let Some(frame) = context_frames.pop()
                        && frame.draw_count == draw_count
                    {
                        diagnostics.push(Diagnostic {
                            level: DiagnosticLevel::Warning,
                            kind: DiagnosticKind::EmptyContext,
                            command_index: frame.command_index,
                            contexts: self.context_notes_for_stack(&context_stack),
                        });
                    }
                    context_stack.pop();
                }
                Command::PushClip(id) => {
                    if clip_uses_empty_path(self.clip(id)) {
                        diagnostics.push(Diagnostic {
                            level: DiagnosticLevel::Warning,
                            kind: DiagnosticKind::EmptyPath,
                            command_index,
                            contexts: self.context_notes_for_stack(&context_stack),
                        });
                    }
                    if clip_uses_zero_width_stroke(self.clip(id)) {
                        diagnostics.push(Diagnostic {
                            level: DiagnosticLevel::Warning,
                            kind: DiagnosticKind::ZeroWidthStroke,
                            command_index,
                            contexts: self.context_notes_for_stack(&context_stack),
                        });
                    }
                    clip_frames.push(ScopeDiagnosticFrame {
                        command_index,
                        draw_count,
                    });
                }
                Command::PopClip => {
                    if let Some(frame) = clip_frames.pop()
                        && frame.draw_count == draw_count
                    {
                        diagnostics.push(Diagnostic {
                            level: DiagnosticLevel::Warning,
                            kind: DiagnosticKind::EmptyClip,
                            command_index: frame.command_index,
                            contexts: self.context_notes_for_stack(&context_stack),
                        });
                    }
                }
                Command::PushGroup(id) => {
                    let group = self.group(id);
                    if let Some(mask) = &group.mask
                        && !scene_has_any_draw(&self.mask(mask.mask).scene)
                    {
                        diagnostics.push(Diagnostic {
                            level: DiagnosticLevel::Warning,
                            kind: DiagnosticKind::EmptyMaskScene,
                            command_index,
                            contexts: self.context_notes_for_stack(&context_stack),
                        });
                    }
                    group_frames.push(GroupDiagnosticFrame {
                        command_index,
                        draw_count,
                        is_identity: group_is_identity(group),
                    });
                }
                Command::PopGroup => {
                    if let Some(frame) = group_frames.pop() {
                        if frame.draw_count == draw_count {
                            diagnostics.push(Diagnostic {
                                level: DiagnosticLevel::Warning,
                                kind: DiagnosticKind::EmptyGroup,
                                command_index: frame.command_index,
                                contexts: self.context_notes_for_stack(&context_stack),
                            });
                        } else if frame.is_identity {
                            diagnostics.push(Diagnostic {
                                level: DiagnosticLevel::Warning,
                                kind: DiagnosticKind::IdentityGroup,
                                command_index: frame.command_index,
                                contexts: self.context_notes_for_stack(&context_stack),
                            });
                        }
                    }
                }
                Command::Draw(id) => {
                    draw_count += 1;
                    if draw_uses_empty_path(self.draw_op(id)) {
                        diagnostics.push(Diagnostic {
                            level: DiagnosticLevel::Warning,
                            kind: DiagnosticKind::EmptyPath,
                            command_index,
                            contexts: self.context_notes_for_stack(&context_stack),
                        });
                    }
                    if draw_uses_zero_width_stroke(self.draw_op(id)) {
                        diagnostics.push(Diagnostic {
                            level: DiagnosticLevel::Warning,
                            kind: DiagnosticKind::ZeroWidthStroke,
                            command_index,
                            contexts: self.context_notes_for_stack(&context_stack),
                        });
                    }
                    if draw_uses_zero_blur(self.draw_op(id)) {
                        diagnostics.push(Diagnostic {
                            level: DiagnosticLevel::Warning,
                            kind: DiagnosticKind::ZeroBlurredRoundedRect,
                            command_index,
                            contexts: self.context_notes_for_stack(&context_stack),
                        });
                    }
                    if draw_is_fully_transparent(self, id) {
                        diagnostics.push(Diagnostic {
                            level: DiagnosticLevel::Warning,
                            kind: DiagnosticKind::TransparentDraw,
                            command_index,
                            contexts: self.context_notes_for_stack(&context_stack),
                        });
                    }
                }
            }
        }

        diagnostics
    }
}

fn scene_has_any_draw(scene: &Scene) -> bool {
    scene
        .commands()
        .iter()
        .any(|command| matches!(command, Command::Draw(_)))
}

fn group_is_identity(group: &crate::record::Group) -> bool {
    group.clip.is_none()
        && group.mask.is_none()
        && group.filters.is_empty()
        && group.composite == Composite::default()
}

fn clip_uses_empty_path(clip: &Clip) -> bool {
    match clip {
        Clip::Fill { shape, .. } | Clip::Stroke { shape, .. } => geometry_is_empty_path(shape),
    }
}

fn clip_uses_zero_width_stroke(clip: &Clip) -> bool {
    match clip {
        Clip::Fill { .. } => false,
        Clip::Stroke { stroke, .. } => stroke.width <= 0.0,
    }
}

fn draw_uses_empty_path(draw: &Draw) -> bool {
    match draw {
        Draw::Fill { shape, .. } | Draw::Stroke { shape, .. } => geometry_is_empty_path(shape),
        Draw::GlyphRun(_) | Draw::BlurredRoundedRect(_) => false,
    }
}

fn draw_uses_zero_width_stroke(draw: &Draw) -> bool {
    match draw {
        Draw::Stroke { stroke, .. } => stroke.width <= 0.0,
        Draw::Fill { .. } | Draw::GlyphRun(_) | Draw::BlurredRoundedRect(_) => false,
    }
}

fn draw_uses_zero_blur(draw: &Draw) -> bool {
    match draw {
        Draw::BlurredRoundedRect(draw) => draw.std_dev == 0.0,
        Draw::Fill { .. } | Draw::Stroke { .. } | Draw::GlyphRun(_) => false,
    }
}

fn geometry_is_empty_path(geometry: &Geometry) -> bool {
    match geometry {
        Geometry::Path(path) => path.is_empty(),
        Geometry::Rect(_) | Geometry::RoundedRect(_) => false,
    }
}

fn draw_is_fully_transparent(scene: &Scene, id: DrawId) -> bool {
    match scene.draw_op(id) {
        Draw::Fill {
            brush, composite, ..
        }
        | Draw::Stroke {
            brush, composite, ..
        } => composite.alpha <= 0.0 || solid_brush_is_fully_transparent(brush),
        Draw::GlyphRun(glyph_run) => {
            glyph_run.composite.alpha <= 0.0 || solid_brush_is_fully_transparent(&glyph_run.brush)
        }
        Draw::BlurredRoundedRect(draw) => {
            draw.composite.alpha <= 0.0 || draw.color.components[3] <= 0.0
        }
    }
}

fn solid_brush_is_fully_transparent(brush: &Brush) -> bool {
    match brush {
        Brush::Solid(color) => color.components[3] <= 0.0,
        Brush::Gradient(_) | Brush::Image(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use kurbo::{Affine, BezPath, Rect, Stroke};
    use peniko::{Brush, Color, Fill};

    use super::*;
    use crate::Composite;
    use crate::SourceLocationRef;
    use crate::{
        BlurredRoundedRect, MaskMode,
        record::{
            AppliedMask, Clip, ContextId, Draw, Geometry, Group, Mask, ResolvedSourceLocation,
        },
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
}
