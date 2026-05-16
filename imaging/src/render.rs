// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Small backend-facing rendering traits.
//!
//! This module provides a minimal integration seam for higher layers that want to render semantic
//! `imaging` content without committing to a specific backend API up front.
//!
//! The core ideas are:
//! - [`RenderSource`] abstracts "something that can paint into a sink".
//! - [`ImageRenderer`] renders a source into caller-owned image buffers.

use crate::{PaintSink, RgbaImage, record};
use alloc::boxed::Box;
use alloc::vec::Vec;

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

/// Shared source/content failures surfaced by renderer traits.
#[derive(Debug)]
pub enum RenderContentError {
    /// The source failed shared retained-scene validation before rendering.
    InvalidScene(record::ValidateError),
    /// The backend could not decode or use the supplied font data.
    InvalidFontData,
    /// A glyph identifier could not be represented by the backend.
    InvalidGlyphId,
}

impl core::fmt::Display for RenderContentError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidScene(error) => core::fmt::Display::fmt(error, f),
            Self::InvalidFontData => f.write_str("backend could not decode font data"),
            Self::InvalidGlyphId => f.write_str("backend could not represent a glyph identifier"),
        }
    }
}

impl core::error::Error for RenderContentError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::InvalidScene(error) => Some(error),
            Self::InvalidFontData | Self::InvalidGlyphId => None,
        }
    }
}

/// Shared GPU readback failures surfaced by image renderers.
#[derive(Debug)]
pub enum GpuReadbackError {
    /// A GPU readback required for image output failed while polling the device.
    DevicePoll,
    /// A GPU readback callback was dropped before completing.
    CallbackDropped,
    /// A GPU readback buffer could not be mapped.
    BufferMap,
}

impl core::fmt::Display for GpuReadbackError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::DevicePoll => f.write_str("wgpu device poll failed during readback"),
            Self::CallbackDropped => {
                f.write_str("wgpu readback callback dropped before completion")
            }
            Self::BufferMap => f.write_str("wgpu readback buffer map failed"),
        }
    }
}

impl core::error::Error for GpuReadbackError {}

/// Image-target-specific failures surfaced by image renderers.
#[derive(Debug)]
pub enum ImageTargetError {
    /// The supplied image target is not compatible with the renderer output.
    InvalidTarget(&'static str),
    /// The requested render dimensions exceed backend limits.
    DimensionsTooLarge,
    /// The backend could not map the caller target format to its native image output.
    UnsupportedTargetFormat,
    /// The caller-provided image buffer is too small or otherwise invalid.
    InvalidTargetBuffer,
}

impl core::fmt::Display for ImageTargetError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidTarget(message) => f.write_str(message),
            Self::DimensionsTooLarge => f.write_str("render dimensions exceed backend limits"),
            Self::UnsupportedTargetFormat => {
                f.write_str("backend does not support the requested target format")
            }
            Self::InvalidTargetBuffer => {
                f.write_str("target buffer is too small or otherwise invalid")
            }
        }
    }
}

impl core::error::Error for ImageTargetError {}

/// Pixel format for caller-owned image buffers consumed by [`ImageRenderer`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ImageBufferFormat {
    /// Unpremultiplied RGBA8 in linear space.
    Rgba8Unorm,
    /// Unpremultiplied RGBA8 in sRGB space.
    Rgba8UnormSrgb,
    /// Unpremultiplied BGRA8 in linear space.
    Bgra8Unorm,
    /// Unpremultiplied BGRA8 in sRGB space.
    Bgra8UnormSrgb,
    /// Packed 10/10/10/2 RGBA in linear space.
    Rgb10a2Unorm,
    /// Unpremultiplied RGBA16 normalized unsigned integer.
    Rgba16Unorm,
    /// Unpremultiplied RGBA16 floating point.
    Rgba16Float,
}

impl ImageBufferFormat {
    /// Return the size in bytes for one pixel in this format.
    #[must_use]
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgba8Unorm
            | Self::Rgba8UnormSrgb
            | Self::Bgra8Unorm
            | Self::Bgra8UnormSrgb
            | Self::Rgb10a2Unorm => 4,
            Self::Rgba16Unorm | Self::Rgba16Float => 8,
        }
    }
}

/// Shared image-rendering error type for image renderers.
#[derive(Debug)]
pub enum ImageRendererError {
    /// Source/content-related failure.
    Content(RenderContentError),
    /// Caller target-related failure.
    Target(ImageTargetError),
    /// GPU readback failure.
    Readback(GpuReadbackError),
    /// Backend-specific rendering error.
    Backend(Box<dyn core::error::Error + Send + Sync + 'static>),
}

impl ImageRendererError {
    /// Box a backend-specific error value.
    #[must_use]
    pub fn backend(error: impl core::error::Error + Send + Sync + 'static) -> Self {
        Self::Backend(Box::new(error))
    }
}

impl core::fmt::Display for ImageRendererError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Content(error) => core::fmt::Display::fmt(error, f),
            Self::Target(error) => core::fmt::Display::fmt(error, f),
            Self::Readback(error) => core::fmt::Display::fmt(error, f),
            Self::Backend(error) => core::fmt::Display::fmt(error, f),
        }
    }
}

impl core::error::Error for ImageRendererError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Content(error) => Some(error),
            Self::Target(error) => Some(error),
            Self::Readback(error) => Some(error),
            Self::Backend(error) => Some(error.as_ref()),
        }
    }
}

/// Borrowed caller-owned image target.
#[derive(Debug)]
pub struct ImageBufferTarget<'a> {
    /// Pixel bytes in the target format's native packing.
    pub data: &'a mut [u8],
    /// Target width in pixels.
    pub width: u32,
    /// Target height in pixels.
    pub height: u32,
    /// Target row stride in bytes.
    pub bytes_per_row: usize,
    /// Target pixel format.
    pub format: ImageBufferFormat,
}

impl<'a> ImageBufferTarget<'a> {
    /// Wrap a caller-owned [`RgbaImage`] as an image target.
    #[must_use]
    pub fn from_rgba_image(image: &'a mut RgbaImage) -> Self {
        let width = image.width;
        Self {
            data: image.data.as_mut_slice(),
            width,
            height: image.height,
            bytes_per_row: usize::try_from(width)
                .expect("image width should fit in usize")
                .checked_mul(4)
                .expect("image row bytes should fit in usize"),
            format: ImageBufferFormat::Rgba8Unorm,
        }
    }
}

/// Copy a tightly packed [`RgbaImage`] into a caller-owned target buffer.
pub fn copy_rgba_image(
    image: &RgbaImage,
    target: ImageBufferTarget<'_>,
) -> Result<(), ImageRendererError> {
    let width_bytes = usize::try_from(image.width)
        .expect("image width should fit in usize")
        .checked_mul(4)
        .expect("image row bytes should fit in usize");
    if target.width != image.width || target.height != image.height {
        return Err(ImageRendererError::Target(ImageTargetError::InvalidTarget(
            "image target dimensions do not match renderer output",
        )));
    }
    if target.bytes_per_row < width_bytes {
        return Err(ImageRendererError::Target(ImageTargetError::InvalidTarget(
            "image target row stride is smaller than the rendered width",
        )));
    }
    if target.format != ImageBufferFormat::Rgba8Unorm {
        return Err(ImageRendererError::Target(
            ImageTargetError::UnsupportedTargetFormat,
        ));
    }
    let required_len = target
        .bytes_per_row
        .checked_mul(usize::try_from(target.height).expect("image height should fit in usize"))
        .expect("image target byte length should fit in usize");
    if target.data.len() < required_len {
        return Err(ImageRendererError::Target(ImageTargetError::InvalidTarget(
            "image target buffer is too small for the requested dimensions",
        )));
    }

    for (src, dst) in image
        .data
        .chunks_exact(width_bytes)
        .zip(target.data.chunks_exact_mut(target.bytes_per_row))
    {
        dst[..width_bytes].copy_from_slice(src);
    }
    Ok(())
}

/// Renderer capability for producing RGBA8 image results from a [`RenderSource`].
///
/// The source is erased behind `&mut dyn RenderSource`, and backends map their concrete failures
/// into the shared [`ImageRendererError`] type.
pub trait ImageRenderer {
    /// Return the image buffer formats this renderer can write directly.
    fn supported_image_formats(&self) -> Vec<ImageBufferFormat>;

    /// Render a source into a caller-provided image buffer.
    ///
    /// Renderers should treat the target as a fresh output and may clear or overwrite any
    /// existing contents before drawing.
    fn render_source_into(
        &mut self,
        source: &mut dyn RenderSource,
        target: ImageBufferTarget<'_>,
    ) -> Result<(), ImageRendererError>;

    /// Render a source and return a newly allocated RGBA8 image.
    fn render_source(
        &mut self,
        source: &mut dyn RenderSource,
        width: u32,
        height: u32,
    ) -> Result<RgbaImage, ImageRendererError> {
        let mut image = RgbaImage::new(width, height);
        self.render_source_into(source, ImageBufferTarget::from_rgba_image(&mut image))?;
        Ok(image)
    }
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
