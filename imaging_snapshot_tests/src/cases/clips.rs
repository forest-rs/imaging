// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use imaging::{Clip, Composite, Draw, Geometry, Sink};
use kurbo::{Affine, Rect, RoundedRect};
use peniko::{BlendMode, Brush, Color, Compose, Fill};

use super::SnapshotCase;
use super::util::{background, circle_geometry};

pub(crate) struct GmClipNonIsolated;
impl SnapshotCase for GmClipNonIsolated {
    fn name(&self) -> &'static str {
        "gm_clip_non_isolated"
    }

    fn run(&self, sink: &mut dyn Sink, width: f64, height: f64) {
        background(sink, width, height, Color::from_rgb8(30, 30, 34));
        sink.draw(Draw::Fill {
            transform: Affine::IDENTITY,
            fill_rule: Fill::NonZero,
            paint: Brush::Solid(Color::from_rgb8(46, 46, 52)),
            paint_transform: None,
            shape: Geometry::Rect(Rect::new(0.0, 0.0, width * 0.5, height * 0.5)),
            composite: Composite::default(),
        });
        sink.draw(Draw::Fill {
            transform: Affine::IDENTITY,
            fill_rule: Fill::NonZero,
            paint: Brush::Solid(Color::from_rgb8(46, 46, 52)),
            paint_transform: None,
            shape: Geometry::Rect(Rect::new(width * 0.5, height * 0.5, width, height)),
            composite: Composite::default(),
        });

        sink.push_clip(Clip::Fill {
            transform: Affine::IDENTITY,
            shape: Geometry::RoundedRect(RoundedRect::new(
                width * 0.15,
                height * 0.2,
                width * 0.85,
                height * 0.8,
                26.0,
            )),
            fill_rule: Fill::NonZero,
        });
        sink.draw(Draw::Fill {
            transform: Affine::IDENTITY,
            fill_rule: Fill::NonZero,
            paint: Brush::Solid(Color::from_rgba8(255, 80, 0, 255)),
            paint_transform: None,
            shape: circle_geometry((width * 0.48, height * 0.52), width.min(height) * 0.26, 0.1),
            composite: Composite::new(BlendMode::from(Compose::Xor), 0.85),
        });
        sink.pop_clip();
    }
}
