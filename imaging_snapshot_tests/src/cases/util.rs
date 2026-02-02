// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use imaging::{Draw, Geometry, Sink};
use kurbo::{Affine, Circle, Point, Rect, Shape as _};
use peniko::{Brush, Color, Fill};

/// Default snapshot width in pixels.
pub const DEFAULT_WIDTH: u16 = 256;
/// Default snapshot height in pixels.
pub const DEFAULT_HEIGHT: u16 = 256;

pub(crate) fn background(sink: &mut dyn Sink, width: f64, height: f64, color: Color) {
    sink.draw(Draw::Fill {
        transform: Affine::IDENTITY,
        fill_rule: Fill::NonZero,
        paint: Brush::Solid(color),
        paint_transform: None,
        shape: Geometry::Rect(Rect::new(0.0, 0.0, width, height)),
        composite: imaging::Composite::default(),
    });
}

pub(crate) fn circle_geometry(center: (f64, f64), radius: f64, tolerance: f64) -> Geometry {
    let circle = Circle::new(Point::new(center.0, center.1), radius);
    Geometry::Path(circle.to_path(tolerance))
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
