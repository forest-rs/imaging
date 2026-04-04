// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Small backend-facing rendering traits.
//!
//! This module provides a minimal integration seam for higher layers that want to render semantic
//! `imaging` content without committing to a specific backend API up front.
//!
//! The core ideas are:
//! - [`RenderSource`] abstracts "something that can paint into a sink".
//! - [`ImageRenderer`] renders a source into an [`crate::RgbaImage`].
//! - [`TextureRenderer`] renders a source into a backend-owned GPU target type.
//!
//! `imaging` intentionally does not define concrete GPU target wrappers here. Each backend keeps
//! ownership of its own target types and host API choices.
//!
//! These traits are intended for compile-time-selected backends. They are not a promise of runtime
//! backend erasure or object-safe renderer polymorphism.

use crate::{PaintSink, RgbaImage, record};

/// A source of drawing commands that can paint into any [`PaintSink`].
///
/// This abstracts over both retained recordings like [`record::Scene`] and immediate command
/// producers like closures.
pub trait RenderSource {
    /// Validate this source before rendering, when validation is meaningful.
    ///
    /// This is primarily a retained-scene preflight hook. Streaming/immediate sources typically
    /// use the default implementation, which assumes no up-front validation step is available.
    fn validate(&self) -> Result<(), record::ValidateError> {
        Ok(())
    }

    /// Emit drawing commands into the provided sink.
    fn paint_into(&mut self, sink: &mut dyn PaintSink);
}

impl RenderSource for record::Scene {
    fn validate(&self) -> Result<(), record::ValidateError> {
        self.validate()
    }

    fn paint_into(&mut self, sink: &mut dyn PaintSink) {
        record::replay(self, sink);
    }
}

impl RenderSource for &record::Scene {
    fn validate(&self) -> Result<(), record::ValidateError> {
        record::Scene::validate(self)
    }

    fn paint_into(&mut self, sink: &mut dyn PaintSink) {
        record::replay(self, sink);
    }
}

impl<F> RenderSource for F
where
    F: FnMut(&mut dyn PaintSink),
{
    fn paint_into(&mut self, sink: &mut dyn PaintSink) {
        self(sink);
    }
}

/// Renderer capability for producing RGBA8 image results from a [`RenderSource`].
pub trait ImageRenderer {
    /// Error type returned by this renderer.
    type Error;

    /// Render a source into a caller-provided image buffer.
    fn render_source_into<S: RenderSource + ?Sized>(
        &mut self,
        source: &mut S,
        width: u32,
        height: u32,
        image: &mut RgbaImage,
    ) -> Result<(), Self::Error>;

    /// Render a source and return a newly allocated RGBA8 image.
    fn render_source<S: RenderSource + ?Sized>(
        &mut self,
        source: &mut S,
        width: u32,
        height: u32,
    ) -> Result<RgbaImage, Self::Error> {
        let mut image = RgbaImage::new(width, height);
        self.render_source_into(source, width, height, &mut image)?;
        Ok(image)
    }
}

/// Renderer capability for drawing a [`RenderSource`] into a backend-owned texture target.
///
/// The concrete target type is backend-specific and remains owned by the backend crate.
pub trait TextureRenderer {
    /// Error type returned by this renderer.
    type Error;

    /// Backend-owned texture target type.
    type TextureTarget<'a>
    where
        Self: 'a;

    /// Render a source into a caller-provided texture target.
    fn render_source_to_texture<'a, S: RenderSource + ?Sized>(
        &mut self,
        source: &mut S,
        target: Self::TextureTarget<'a>,
    ) -> Result<(), Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FillRef, GroupRef, Painter, StrokeRef, record::Scene};
    use kurbo::Rect;
    use peniko::Color;

    #[derive(Default)]
    struct CountingSink {
        fills: usize,
    }

    impl PaintSink for CountingSink {
        fn push_clip(&mut self, _clip: crate::ClipRef<'_>) {}

        fn pop_clip(&mut self) {}

        fn push_group(&mut self, _group: GroupRef<'_>) {}

        fn pop_group(&mut self) {}

        fn fill(&mut self, _draw: FillRef<'_>) {
            self.fills += 1;
        }

        fn stroke(&mut self, _draw: StrokeRef<'_>) {}

        fn glyph_run(
            &mut self,
            _draw: crate::GlyphRunRef<'_>,
            _glyphs: &mut dyn Iterator<Item = record::Glyph>,
        ) {
        }

        fn blurred_rounded_rect(&mut self, _draw: crate::BlurredRoundedRect) {}
    }

    #[test]
    fn scene_render_source_replays_commands() {
        let mut scene = Scene::new();
        {
            let mut painter = Painter::new(&mut scene);
            painter
                .fill(
                    Rect::new(0.0, 0.0, 16.0, 16.0),
                    Color::from_rgb8(0x2a, 0x6f, 0xdb),
                )
                .draw();
        }

        let mut sink = CountingSink::default();
        let mut source = &scene;
        source.paint_into(&mut sink);
        assert_eq!(sink.fills, 1);
    }

    #[test]
    fn closure_render_source_paints_into_sink() {
        let mut sink = CountingSink::default();
        let mut source = |sink: &mut dyn PaintSink| {
            let mut painter = Painter::new(sink);
            painter
                .fill(
                    Rect::new(0.0, 0.0, 12.0, 12.0),
                    Color::from_rgb8(0xd9, 0x77, 0x06),
                )
                .draw();
        };

        source.paint_into(&mut sink);
        assert_eq!(sink.fills, 1);
    }
}
