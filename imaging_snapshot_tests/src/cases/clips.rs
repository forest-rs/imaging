// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use imaging::{ClipRef, Composite, Geometry, GroupRef, PaintSink, Painter};
use kurbo::{Affine, Rect, RoundedRect, Stroke};
use peniko::{BlendMode, Brush, Color, Compose, Mix};

use super::SnapshotCase;
use super::util::{background, circle_geometry};

pub(crate) struct GmClipNonIsolated;
impl SnapshotCase for GmClipNonIsolated {
    fn name(&self) -> &'static str {
        "gm_clip_non_isolated"
    }

    fn vello_max_diff_pixels(&self) -> u64 {
        4
    }

    fn run(&self, sink: &mut dyn PaintSink, width: f64, height: f64) {
        background(sink, width, height, Color::from_rgb8(30, 30, 34));
        let mut painter = Painter::new(sink);
        let tile = Brush::Solid(Color::from_rgb8(46, 46, 52));
        painter
            .fill(
                Geometry::Rect(Rect::new(0.0, 0.0, width * 0.5, height * 0.5)),
                &tile,
            )
            .draw();
        painter
            .fill(
                Geometry::Rect(Rect::new(width * 0.5, height * 0.5, width, height)),
                &tile,
            )
            .draw();

        let clip = ClipRef::fill(Geometry::RoundedRect(RoundedRect::new(
            width * 0.15,
            height * 0.2,
            width * 0.85,
            height * 0.8,
            26.0,
        )));
        painter.with_clip(clip, |painter| {
            let paint = Brush::Solid(Color::from_rgba8(255, 80, 0, 255));
            painter
                .fill(
                    circle_geometry((width * 0.48, height * 0.52), width.min(height) * 0.26, 0.1),
                    &paint,
                )
                .composite(Composite::new(BlendMode::from(Compose::Xor), 0.85))
                .draw();
        });
    }
}

pub(crate) struct GmClipStrokeNested;
impl SnapshotCase for GmClipStrokeNested {
    fn name(&self) -> &'static str {
        "gm_clip_stroke_nested"
    }

    fn run(&self, sink: &mut dyn PaintSink, width: f64, height: f64) {
        background(sink, width, height, Color::from_rgb8(16, 18, 24));
        let mut painter = Painter::new(sink);

        let card = Brush::Solid(Color::from_rgb8(28, 32, 42));
        painter
            .fill(
                Geometry::Rect(Rect::new(
                    width * 0.08,
                    height * 0.08,
                    width * 0.92,
                    height * 0.92,
                )),
                &card,
            )
            .draw();

        let stroke_clip = Stroke::new(width * 0.18);
        painter.with_stroke_clip(
            Geometry::RoundedRect(RoundedRect::new(
                width * 0.16,
                height * 0.16,
                width * 0.84,
                height * 0.84,
                42.0,
            )),
            &stroke_clip,
            |painter| {
                let slat = Brush::Solid(Color::from_rgb8(255, 119, 48));
                painter
                    .fill(
                        Geometry::Rect(Rect::new(0.0, 0.0, width * 0.88, height * 0.34)),
                        &slat,
                    )
                    .transform(
                        Affine::rotate(0.18)
                            .then_translate((width * 0.12, -(height * 0.06)).into()),
                    )
                    .draw();

                painter.with_group(GroupRef::new(), |painter| {
                    let cyan = Brush::Solid(Color::from_rgb8(35, 181, 255));
                    painter
                        .fill(
                            circle_geometry(
                                (width * 0.33, height * 0.6),
                                width.min(height) * 0.24,
                                0.1,
                            ),
                            &cyan,
                        )
                        .composite(Composite::new(BlendMode::from(Mix::Screen), 0.88))
                        .draw();

                    painter.with_group(
                        GroupRef::new()
                            .with_composite(Composite::new(BlendMode::from(Mix::Multiply), 0.92)),
                        |painter| {
                            let violet = Brush::Solid(Color::from_rgb8(122, 74, 255));
                            painter
                                .fill(
                                    Geometry::Rect(Rect::new(
                                        width * 0.42,
                                        height * 0.26,
                                        width * 0.88,
                                        height * 0.82,
                                    )),
                                    &violet,
                                )
                                .draw();

                            let glow = Brush::Solid(Color::from_rgba8(255, 246, 186, 255));
                            painter
                                .fill(
                                    circle_geometry(
                                        (width * 0.7, height * 0.36),
                                        width.min(height) * 0.18,
                                        0.1,
                                    ),
                                    &glow,
                                )
                                .composite(Composite::new(BlendMode::from(Compose::Plus), 0.7))
                                .draw();
                        },
                    );
                });
            },
        );
    }
}
