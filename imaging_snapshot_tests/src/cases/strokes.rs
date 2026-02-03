// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use imaging::{Composite, Draw, Geometry, Sink, StrokeStyle};
use kurbo::{Affine, BezPath, Cap, Join, Point};
use peniko::{Brush, Color, Fill};

use super::SnapshotCase;
use super::util::background;

pub(crate) struct GmStrokes;
impl SnapshotCase for GmStrokes {
    fn name(&self) -> &'static str {
        "gm_strokes"
    }

    fn skia_max_diff_pixels(&self) -> u64 {
        // Minor variation in stroke rasterization/AA across platforms/toolchains.
        2_500
    }

    fn vello_hybrid_max_diff_pixels(&self) -> u64 {
        3_000
    }

    fn run(&self, sink: &mut dyn Sink, width: f64, height: f64) {
        background(sink, width, height, Color::from_rgb8(245, 245, 250));

        let mut path = BezPath::new();
        path.move_to(Point::new(width * 0.15, height * 0.75));
        path.line_to(Point::new(width * 0.35, height * 0.25));
        path.line_to(Point::new(width * 0.55, height * 0.75));
        path.line_to(Point::new(width * 0.75, height * 0.25));

        let styles: &[(Join, Cap, Cap, Option<&[f64]>)] = &[
            (Join::Miter, Cap::Butt, Cap::Butt, None),
            (Join::Bevel, Cap::Square, Cap::Square, None),
            (Join::Round, Cap::Round, Cap::Round, None),
            (Join::Miter, Cap::Round, Cap::Square, Some(&[10.0, 6.0])),
        ];

        for (i, (join, start_cap, end_cap, dash)) in styles.iter().enumerate() {
            let y = (i as f64) * (height * 0.18);
            let transform = Affine::translate((0.0, y));
            let mut stroke = StrokeStyle::new(14.0)
                .with_join(*join)
                .with_start_cap(*start_cap)
                .with_end_cap(*end_cap);
            if let Some(dashes) = dash {
                stroke.dash_pattern = kurbo::Dashes::from_slice(dashes);
                stroke.dash_offset = 0.0;
            }

            sink.draw(Draw::Stroke {
                transform,
                stroke,
                paint: Brush::Solid(Color::from_rgb8(20, 80, 200)),
                paint_transform: None,
                shape: Geometry::Path(path.clone()),
                composite: Composite::default(),
            });

            // Underlay to show caps clearly.
            sink.draw(Draw::Fill {
                transform,
                fill_rule: Fill::NonZero,
                paint: Brush::Solid(Color::from_rgba8(0, 0, 0, 18)),
                paint_transform: None,
                shape: Geometry::Rect(kurbo::Rect::new(
                    width * 0.1,
                    height * 0.78,
                    width * 0.8,
                    height * 0.82,
                )),
                composite: Composite::default(),
            });
        }
    }
}
