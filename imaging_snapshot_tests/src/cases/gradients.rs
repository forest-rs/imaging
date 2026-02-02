// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use imaging::{Clip, Composite, Draw, Geometry, Sink};
use kurbo::{Affine, RoundedRect};
use peniko::{BlendMode, Brush, Color, Compose, Extend, Fill, Gradient};

use super::SnapshotCase;
use super::util::{background, circle_geometry, f32p};

fn linear_rainbow(start: (f32, f32), end: (f32, f32)) -> Brush {
    let stops = [
        (0.00, Color::from_rgb8(255, 0, 0)),
        (0.16, Color::from_rgb8(255, 160, 0)),
        (0.33, Color::from_rgb8(255, 255, 0)),
        (0.50, Color::from_rgb8(0, 255, 0)),
        (0.66, Color::from_rgb8(0, 200, 255)),
        (0.83, Color::from_rgb8(0, 0, 255)),
        (1.00, Color::from_rgb8(180, 0, 255)),
    ];
    Brush::Gradient(
        Gradient::new_linear(start, end)
            .with_extend(Extend::Pad)
            .with_stops(stops),
    )
}

fn sweep_rainbow(center: (f32, f32), start_angle: f32, end_angle: f32) -> Brush {
    let stops = [
        (0.00, Color::from_rgb8(255, 0, 0)),
        (0.16, Color::from_rgb8(255, 160, 0)),
        (0.33, Color::from_rgb8(255, 255, 0)),
        (0.50, Color::from_rgb8(0, 255, 0)),
        (0.66, Color::from_rgb8(0, 200, 255)),
        (0.83, Color::from_rgb8(0, 0, 255)),
        (1.00, Color::from_rgb8(255, 0, 0)),
    ];
    Brush::Gradient(
        Gradient::new_sweep(center, start_angle, end_angle)
            .with_extend(Extend::Pad)
            .with_stops(stops),
    )
}

pub(crate) struct GmGradientsLinear;
impl SnapshotCase for GmGradientsLinear {
    fn name(&self) -> &'static str {
        "gm_gradients_linear"
    }

    fn run(&self, sink: &mut dyn Sink, width: f64, height: f64) {
        background(sink, width, height, Color::from_rgb8(18, 18, 22));

        let band = RoundedRect::new(24.0, 32.0, width - 24.0, height - 32.0, 28.0);
        let g = linear_rainbow((20.0, 20.0), (f32p(width - 20.0), f32p(height - 20.0)));
        sink.draw(Draw::Fill {
            transform: Affine::IDENTITY,
            fill_rule: Fill::NonZero,
            paint: g,
            paint_transform: None,
            shape: Geometry::RoundedRect(band),
            composite: Composite::default(),
        });

        // Punch a transparent hole using a nontrivial compose mode (Copy).
        sink.draw(Draw::Fill {
            transform: Affine::IDENTITY,
            fill_rule: Fill::NonZero,
            paint: Brush::Solid(Color::TRANSPARENT),
            paint_transform: None,
            shape: circle_geometry((width * 0.5, height * 0.5), width.min(height) * 0.14, 0.1),
            composite: Composite::new(BlendMode::from(Compose::Copy), 1.0),
        });
    }
}

pub(crate) struct GmGradientsSweep;
impl SnapshotCase for GmGradientsSweep {
    fn name(&self) -> &'static str {
        "gm_gradients_sweep"
    }

    fn run(&self, sink: &mut dyn Sink, width: f64, height: f64) {
        background(sink, width, height, Color::from_rgb8(10, 10, 12));

        sink.push_clip(Clip::Fill {
            transform: Affine::IDENTITY,
            shape: Geometry::RoundedRect(RoundedRect::new(
                16.0,
                16.0,
                width - 16.0,
                height - 16.0,
                24.0,
            )),
            fill_rule: Fill::NonZero,
        });
        sink.draw(Draw::Fill {
            transform: Affine::IDENTITY,
            fill_rule: Fill::NonZero,
            paint: sweep_rainbow(
                (f32p(width * 0.5), f32p(height * 0.5)),
                0.0,
                std::f32::consts::TAU,
            ),
            paint_transform: None,
            shape: circle_geometry((width * 0.5, height * 0.5), width.min(height) * 0.35, 0.1),
            composite: Composite::default(),
        });
        sink.pop_clip();
    }
}

pub(crate) struct GmGradientsTwoPointRadial;
impl SnapshotCase for GmGradientsTwoPointRadial {
    fn name(&self) -> &'static str {
        "gm_gradients_two_point_radial"
    }

    fn run(&self, sink: &mut dyn Sink, width: f64, height: f64) {
        background(sink, width, height, Color::from_rgb8(16, 16, 18));

        // Two-point radial gradient "spotlight".
        let stops = [
            (0.00, Color::from_rgba8(255, 255, 255, 255)),
            (0.35, Color::from_rgba8(255, 220, 40, 255)),
            (1.00, Color::from_rgba8(255, 80, 0, 0)),
        ];
        let g = Brush::Gradient(
            Gradient::new_two_point_radial(
                (f32p(width * 0.35), f32p(height * 0.35)),
                f32p(width.min(height) * 0.05),
                (f32p(width * 0.55), f32p(height * 0.55)),
                f32p(width.min(height) * 0.42),
            )
            .with_extend(Extend::Pad)
            .with_stops(stops),
        );
        sink.draw(Draw::Fill {
            transform: Affine::IDENTITY,
            fill_rule: Fill::NonZero,
            paint: g,
            paint_transform: None,
            shape: Geometry::RoundedRect(RoundedRect::new(
                24.0,
                24.0,
                width - 24.0,
                height - 24.0,
                30.0,
            )),
            composite: Composite::default(),
        });
    }
}
