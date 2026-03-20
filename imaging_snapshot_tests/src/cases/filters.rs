// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use imaging::{BlurredRoundedRect, Composite, Filter, Geometry, GroupRef, PaintSink, Painter};
use kurbo::{Affine, Rect};
use peniko::{Brush, Color};

use super::SnapshotCase;
use super::util::{background, circle_geometry};

pub(crate) struct GmGroupBlurFilter;
impl SnapshotCase for GmGroupBlurFilter {
    fn name(&self) -> &'static str {
        "gm_group_blur_filter"
    }

    fn supports_backend(&self, backend: &str) -> bool {
        matches!(backend, "skia" | "vello_cpu")
    }

    fn run(&self, sink: &mut dyn PaintSink, width: f64, height: f64) {
        background(sink, width, height, Color::WHITE);
        let mut painter = Painter::new(sink);
        let filters = [Filter::blur(6.0)];

        painter.with_group(GroupRef::new().with_filters(&filters), |painter| {
            let paint = Brush::Solid(Color::from_rgb8(0, 0, 0));
            painter
                .fill(
                    Geometry::Rect(Rect::new(
                        width * 0.35,
                        height * 0.35,
                        width * 0.65,
                        height * 0.65,
                    )),
                    &paint,
                )
                .draw();
        });
    }
}

pub(crate) struct GmGroupDropShadow;
impl SnapshotCase for GmGroupDropShadow {
    fn name(&self) -> &'static str {
        "gm_group_drop_shadow"
    }

    fn supports_backend(&self, backend: &str) -> bool {
        matches!(backend, "skia" | "vello_cpu")
    }

    fn run(&self, sink: &mut dyn PaintSink, width: f64, height: f64) {
        background(sink, width, height, Color::from_rgb8(240, 240, 245));
        let mut painter = Painter::new(sink);
        let filters = [Filter::DropShadow {
            dx: 8.0,
            dy: 10.0,
            std_deviation_x: 6.0,
            std_deviation_y: 6.0,
            color: Color::from_rgba8(0, 0, 0, 130),
        }];

        painter.with_group(GroupRef::new().with_filters(&filters), |painter| {
            let paint = Brush::Solid(Color::from_rgb8(0, 140, 255));
            painter
                .fill(
                    circle_geometry((width * 0.45, height * 0.45), width.min(height) * 0.22, 0.1),
                    &paint,
                )
                .draw();
        });
    }
}

pub(crate) struct GmBlurredRoundedRect;
impl SnapshotCase for GmBlurredRoundedRect {
    fn name(&self) -> &'static str {
        "gm_blurred_rounded_rect"
    }

    fn supports_backend(&self, backend: &str) -> bool {
        matches!(backend, "skia" | "vello_cpu" | "vello")
    }

    fn run(&self, sink: &mut dyn PaintSink, width: f64, height: f64) {
        background(sink, width, height, Color::from_rgb8(242, 243, 247));
        let mut painter = Painter::new(sink);

        painter.blurred_rounded_rect(BlurredRoundedRect {
            transform: Affine::translate((0.0, 10.0)),
            rect: Rect::new(width * 0.18, height * 0.22, width * 0.82, height * 0.68),
            color: Color::from_rgba8(23, 33, 66, 150),
            radius: 26.0,
            std_dev: 12.0,
            composite: Composite::default(),
        });
        let white = Brush::Solid(Color::WHITE);
        painter
            .fill(
                Geometry::RoundedRect(kurbo::RoundedRect::new(
                    width * 0.18,
                    height * 0.22,
                    width * 0.82,
                    height * 0.68,
                    26.0,
                )),
                &white,
            )
            .draw();
    }
}

pub(crate) struct GmBlurredRoundedRectVariants;
impl SnapshotCase for GmBlurredRoundedRectVariants {
    fn name(&self) -> &'static str {
        "gm_blurred_rounded_rect_variants"
    }

    fn supports_backend(&self, backend: &str) -> bool {
        matches!(backend, "skia" | "vello_cpu" | "vello")
    }

    fn vello_max_diff_pixels(&self) -> u64 {
        4
    }

    fn run(&self, sink: &mut dyn PaintSink, width: f64, height: f64) {
        background(sink, width, height, Color::from_rgb8(236, 238, 244));
        let mut painter = Painter::new(sink);

        let cards = [
            (
                Rect::new(width * 0.08, height * 0.18, width * 0.34, height * 0.78),
                18.0,
                4.0,
                Color::from_rgba8(12, 18, 44, 96),
            ),
            (
                Rect::new(width * 0.37, height * 0.12, width * 0.63, height * 0.84),
                44.0,
                10.0,
                Color::from_rgba8(19, 32, 74, 118),
            ),
            (
                Rect::new(width * 0.66, height * 0.22, width * 0.92, height * 0.74),
                12.0,
                18.0,
                Color::from_rgba8(52, 88, 160, 96),
            ),
        ];

        for (rect, radius, std_dev, color) in cards {
            painter.blurred_rounded_rect(BlurredRoundedRect {
                transform: Affine::translate((0.0, 8.0)),
                rect,
                color,
                radius,
                std_dev,
                composite: Composite::default(),
            });
            let white = Brush::Solid(Color::WHITE);
            painter
                .fill(
                    Geometry::RoundedRect(kurbo::RoundedRect::new(
                        rect.x0, rect.y0, rect.x1, rect.y1, radius,
                    )),
                    &white,
                )
                .draw();
        }
    }
}
