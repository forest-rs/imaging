// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Render Velato animations through `imaging`.
//!
//! This crate adapts Velato's [`velato::RenderSink`] abstraction to [`imaging::PaintSink`].
//! Use [`ImagingSink`] when you want to stream Velato output directly into an imaging backend or
//! recorder, or use [`RendererExt`] when you want convenience helpers on [`velato::Renderer`].
//!
//! ```rust
//! use imaging::record::Scene;
//! use kurbo::Affine;
//! use velato::Composition;
//! use velato_imaging::{RendererExt, velato};
//!
//! let composition = Composition::default();
//! let mut renderer = velato::Renderer::new();
//! let mut scene = Scene::new();
//!
//! renderer.append_to_imaging(&composition, 0.0, Affine::IDENTITY, 1.0, &mut scene)?;
//! assert_eq!(scene.validate(), Ok(()));
//! # Ok::<(), velato_imaging::Error>(())
//! ```

#![deny(unsafe_code)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use imaging::{ClipRef, Composite, FillRef, GeometryRef, GroupRef, PaintSink, StrokeRef, record};
use kurbo::{Affine, BezPath};

pub use velato;

const DEFAULT_PATH_TOLERANCE: f64 = 0.1;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum LayerKind {
    Clip,
    Group,
}

/// Errors returned by the Velato-to-imaging adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The Velato layer stack did not balance correctly when translated to imaging.
    UnbalancedLayerStack,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnbalancedLayerStack => write!(f, "unbalanced Velato layer stack"),
        }
    }
}

impl core::error::Error for Error {}

/// Adapter that streams Velato rendering commands into an [`imaging::PaintSink`].
///
/// Velato exposes a single layer stack through [`velato::RenderSink`], while `imaging`
/// distinguishes isolated groups from non-isolated clips. `ImagingSink` tracks that stack shape
/// and translates it into the corresponding imaging commands.
#[derive(Debug)]
pub struct ImagingSink<'a, S: PaintSink + ?Sized = dyn PaintSink + 'a> {
    sink: &'a mut S,
    tolerance: f64,
    layer_stack: Vec<LayerKind>,
    error: Option<Error>,
}

impl<'a, S> ImagingSink<'a, S>
where
    S: PaintSink + ?Sized,
{
    /// Create an adapter around an imaging sink using the default path tolerance.
    #[must_use]
    pub fn new(sink: &'a mut S) -> Self {
        Self {
            sink,
            tolerance: DEFAULT_PATH_TOLERANCE,
            layer_stack: Vec::new(),
            error: None,
        }
    }

    /// Set the tolerance used when flattening Velato shapes into retained paths.
    #[must_use]
    pub fn with_tolerance(mut self, tolerance: f64) -> Self {
        self.tolerance = tolerance;
        self
    }

    /// Borrow the wrapped sink.
    #[must_use]
    pub fn inner(&self) -> &S {
        self.sink
    }

    /// Mutably borrow the wrapped sink.
    #[must_use]
    pub fn inner_mut(&mut self) -> &mut S {
        self.sink
    }

    /// Finish the adapter and return any deferred stack error.
    pub fn finish(&mut self) -> Result<(), Error> {
        if let Some(err) = self.error.take() {
            return Err(err);
        }
        if !self.layer_stack.is_empty() {
            return Err(Error::UnbalancedLayerStack);
        }
        Ok(())
    }

    fn set_error_once(&mut self, err: Error) {
        if self.error.is_none() {
            self.error = Some(err);
        }
    }

    fn shape_to_path(&self, shape: &impl kurbo::Shape) -> BezPath {
        shape.to_path(self.tolerance)
    }
}

impl<S> velato::RenderSink for ImagingSink<'_, S>
where
    S: PaintSink + ?Sized,
{
    fn push_layer(
        &mut self,
        blend: impl Into<peniko::BlendMode>,
        alpha: f32,
        transform: Affine,
        shape: &impl kurbo::Shape,
    ) {
        if self.error.is_some() {
            return;
        }
        let clip = ClipRef::fill(GeometryRef::OwnedPath(self.shape_to_path(shape)))
            .with_transform(transform);
        let group = GroupRef::new()
            .with_clip(clip)
            .with_composite(Composite::new(blend.into(), alpha));
        self.sink.push_group(group);
        self.layer_stack.push(LayerKind::Group);
    }

    fn push_clip_layer(&mut self, transform: Affine, shape: &impl kurbo::Shape) {
        if self.error.is_some() {
            return;
        }
        let clip = ClipRef::fill(GeometryRef::OwnedPath(self.shape_to_path(shape)))
            .with_transform(transform);
        self.sink.push_clip(clip);
        self.layer_stack.push(LayerKind::Clip);
    }

    fn pop_layer(&mut self) {
        if self.error.is_some() {
            return;
        }
        match self.layer_stack.pop() {
            Some(LayerKind::Clip) => self.sink.pop_clip(),
            Some(LayerKind::Group) => self.sink.pop_group(),
            None => self.set_error_once(Error::UnbalancedLayerStack),
        }
    }

    fn draw(
        &mut self,
        stroke: Option<&velato::model::fixed::Stroke>,
        transform: Affine,
        brush: &velato::model::fixed::Brush,
        shape: &impl kurbo::Shape,
    ) {
        if self.error.is_some() {
            return;
        }
        let geometry = GeometryRef::OwnedPath(self.shape_to_path(shape));
        match stroke {
            Some(stroke) => self
                .sink
                .stroke(StrokeRef::new(geometry, stroke, brush).transform(transform)),
            None => self
                .sink
                .fill(FillRef::new(geometry, brush).transform(transform)),
        }
    }
}

/// Convenience methods for rendering Velato compositions into imaging sinks.
pub trait RendererExt {
    /// Render and append a composition into an imaging sink.
    fn append_to_imaging<S>(
        &mut self,
        animation: &velato::Composition,
        frame: f64,
        transform: Affine,
        alpha: f64,
        sink: &mut S,
    ) -> Result<(), Error>
    where
        S: PaintSink + ?Sized;

    /// Render a composition into a freshly allocated [`imaging::record::Scene`].
    fn render_to_imaging_scene(
        &mut self,
        animation: &velato::Composition,
        frame: f64,
        transform: Affine,
        alpha: f64,
    ) -> Result<record::Scene, Error>;
}

impl RendererExt for velato::Renderer {
    fn append_to_imaging<S>(
        &mut self,
        animation: &velato::Composition,
        frame: f64,
        transform: Affine,
        alpha: f64,
        sink: &mut S,
    ) -> Result<(), Error>
    where
        S: PaintSink + ?Sized,
    {
        let mut sink = ImagingSink::new(sink);
        self.append(animation, frame, transform, alpha, &mut sink);
        sink.finish()
    }

    fn render_to_imaging_scene(
        &mut self,
        animation: &velato::Composition,
        frame: f64,
        transform: Affine,
        alpha: f64,
    ) -> Result<record::Scene, Error> {
        let mut scene = record::Scene::new();
        self.append_to_imaging(animation, frame, transform, alpha, &mut scene)?;
        Ok(scene)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imaging::record::{Command, Scene};
    use kurbo::Rect;
    use peniko::{BlendMode, Brush, Color, Mix};

    #[test]
    fn imaging_sink_translates_clip_layers_and_draws() {
        let mut scene = Scene::new();
        {
            let mut sink = ImagingSink::new(&mut scene);
            velato::RenderSink::push_clip_layer(
                &mut sink,
                Affine::IDENTITY,
                &Rect::new(0.0, 0.0, 10.0, 10.0),
            );
            velato::RenderSink::draw(
                &mut sink,
                None,
                Affine::translate((2.0, 3.0)),
                &Brush::Solid(Color::WHITE),
                &Rect::new(0.0, 0.0, 4.0, 5.0),
            );
            velato::RenderSink::pop_layer(&mut sink);
            sink.finish().expect("balanced sink");
        }

        assert_eq!(scene.validate(), Ok(()));
        assert_eq!(scene.commands().len(), 3);
        assert!(matches!(scene.commands()[0], Command::PushClip(_)));
        assert!(matches!(scene.commands()[1], Command::Draw(_)));
        assert!(matches!(scene.commands()[2], Command::PopClip));
    }

    #[test]
    fn imaging_sink_translates_isolated_groups() {
        let mut scene = Scene::new();
        {
            let mut sink = ImagingSink::new(&mut scene);
            velato::RenderSink::push_layer(
                &mut sink,
                BlendMode::from(Mix::Multiply),
                0.5,
                Affine::translate((4.0, 6.0)),
                &Rect::new(0.0, 0.0, 8.0, 9.0),
            );
            velato::RenderSink::pop_layer(&mut sink);
            sink.finish().expect("balanced sink");
        }

        assert_eq!(scene.validate(), Ok(()));
        let Command::PushGroup(group_id) = scene.commands()[0] else {
            panic!("expected push group");
        };
        let group = scene.group(group_id);
        assert_eq!(
            group.composite,
            Composite::new(BlendMode::from(Mix::Multiply), 0.5)
        );
        assert!(group.clip.is_some());
        assert!(matches!(scene.commands()[1], Command::PopGroup));
    }

    #[test]
    fn imaging_sink_reports_unbalanced_layer_stack() {
        let mut scene = Scene::new();
        let mut sink = ImagingSink::new(&mut scene);
        velato::RenderSink::pop_layer(&mut sink);
        assert_eq!(sink.finish(), Err(Error::UnbalancedLayerStack));
    }

    #[test]
    fn renderer_ext_renders_default_composition_to_scene() {
        let mut renderer = velato::Renderer::new();
        let composition = velato::Composition::default();
        let scene = renderer
            .render_to_imaging_scene(&composition, 0.0, Affine::IDENTITY, 1.0)
            .expect("render scene");
        assert_eq!(scene.validate(), Ok(()));
        assert_eq!(scene.commands().len(), 2);
        assert!(matches!(scene.commands()[0], Command::PushClip(_)));
        assert!(matches!(scene.commands()[1], Command::PopClip));
    }
}
