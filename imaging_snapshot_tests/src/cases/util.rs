// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::sync::{Arc, OnceLock};

use imaging::{Geometry, PaintSink, Painter};
use kurbo::{Circle, Point, Rect, Shape as _};
use peniko::{Blob, Brush, Color, FontData, ImageAlphaType, ImageData, ImageFormat};

/// Default snapshot width in pixels.
pub const DEFAULT_WIDTH: u16 = 256;
/// Default snapshot height in pixels.
pub const DEFAULT_HEIGHT: u16 = 256;

const ROBOTO_FONT_BYTES: &[u8] = include_bytes!("../assets/roboto/Roboto-Regular.ttf");

pub(crate) fn background(sink: &mut dyn PaintSink, width: f64, height: f64, color: Color) {
    let paint = Brush::Solid(color);
    Painter::new(sink).fill_rect(Rect::new(0.0, 0.0, width, height), &paint);
}

pub(crate) fn circle_geometry(center: (f64, f64), radius: f64, tolerance: f64) -> Geometry {
    let circle = Circle::new(Point::new(center.0, center.1), radius);
    Geometry::Path(circle.to_path(tolerance))
}

pub(crate) fn test_font() -> FontData {
    static FONT: OnceLock<FontData> = OnceLock::new();
    FONT.get_or_init(|| FontData::new(Blob::new(Arc::new(ROBOTO_FONT_BYTES)), 0))
        .clone()
}

pub(crate) fn test_image() -> ImageData {
    static IMAGE: OnceLock<ImageData> = OnceLock::new();
    IMAGE
        .get_or_init(|| {
            let width = 96_u32;
            let height = 96_u32;
            let mut bytes = Vec::with_capacity((width * height * 4) as usize);
            #[allow(
                clippy::cast_possible_truncation,
                reason = "Generated test-image channels are explicitly bounded to u8 ranges."
            )]
            for y in 0..height {
                for x in 0..width {
                    let r = 32 + ((223 * x) / (width - 1)) as u8;
                    let g = 36 + ((170 * y) / (height - 1)) as u8;
                    let stripe = if ((x / 8) + (y / 8)) % 2 == 0 { 34 } else { 0 };
                    let b = 82 + stripe;
                    let a = 255;
                    bytes.extend_from_slice(&[r, g, b, a]);
                }
            }
            ImageData {
                data: Blob::from(bytes),
                format: ImageFormat::Rgba8,
                alpha_type: ImageAlphaType::Alpha,
                width,
                height,
            }
        })
        .clone()
}

#[inline]
#[allow(
    clippy::cast_possible_truncation,
    reason = "Snapshot scenes use small, finite coordinates."
)]
pub(crate) fn f32p(x: f64) -> f32 {
    debug_assert!(x.is_finite(), "snapshot coordinates must be finite");
    x as f32
}
