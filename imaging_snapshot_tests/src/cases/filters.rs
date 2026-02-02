// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use imaging::{Composite, Draw, Filter, Geometry, Group, Sink};
use kurbo::{Affine, Rect};
use peniko::{Brush, Color, Fill};

use super::SnapshotCase;
use super::util::{background, circle_geometry};

pub(crate) struct GmGroupBlurFilter;
impl SnapshotCase for GmGroupBlurFilter {
    fn name(&self) -> &'static str {
        "gm_group_blur_filter"
    }

    fn run(&self, sink: &mut dyn Sink, width: f64, height: f64) {
        background(sink, width, height, Color::WHITE);

        sink.push_group(Group {
            clip: None,
            filters: vec![Filter::blur(6.0)],
            composite: Composite::default(),
        });
        sink.draw(Draw::Fill {
            transform: Affine::IDENTITY,
            fill_rule: Fill::NonZero,
            paint: Brush::Solid(Color::from_rgb8(0, 0, 0)),
            paint_transform: None,
            shape: Geometry::Rect(Rect::new(
                width * 0.35,
                height * 0.35,
                width * 0.65,
                height * 0.65,
            )),
            composite: Composite::default(),
        });
        sink.pop_group();
    }
}

pub(crate) struct GmGroupDropShadow;
impl SnapshotCase for GmGroupDropShadow {
    fn name(&self) -> &'static str {
        "gm_group_drop_shadow"
    }

    fn run(&self, sink: &mut dyn Sink, width: f64, height: f64) {
        background(sink, width, height, Color::from_rgb8(240, 240, 245));

        sink.push_group(Group {
            clip: None,
            filters: vec![Filter::DropShadow {
                dx: 8.0,
                dy: 10.0,
                std_deviation_x: 6.0,
                std_deviation_y: 6.0,
                color: Color::from_rgba8(0, 0, 0, 130),
            }],
            composite: Composite::default(),
        });

        sink.draw(Draw::Fill {
            transform: Affine::IDENTITY,
            fill_rule: Fill::NonZero,
            paint: Brush::Solid(Color::from_rgb8(0, 140, 255)),
            paint_transform: None,
            shape: circle_geometry((width * 0.45, height * 0.45), width.min(height) * 0.22, 0.1),
            composite: Composite::default(),
        });
        sink.pop_group();
    }
}
