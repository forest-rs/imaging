// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use imaging::{Composite, Draw, Geometry, Sink};
use kurbo::{Affine, RoundedRect};
use peniko::{BlendMode, Brush, Color, Compose, Fill, Gradient, Mix};

use super::SnapshotCase;
use super::util::{background, circle_geometry, f32p};

fn linear_rainbow(start: (f32, f32), end: (f32, f32)) -> Brush {
    let stops = [
        (0.00, Color::from_rgb8(255, 0, 0)),
        (0.20, Color::from_rgb8(255, 180, 0)),
        (0.40, Color::from_rgb8(255, 255, 0)),
        (0.60, Color::from_rgb8(0, 255, 0)),
        (0.80, Color::from_rgb8(0, 180, 255)),
        (1.00, Color::from_rgb8(180, 0, 255)),
    ];
    Brush::Gradient(Gradient::new_linear(start, end).with_stops(stops))
}

fn sweep_rainbow(center: (f32, f32)) -> Brush {
    let stops = [
        (0.00, Color::from_rgb8(255, 0, 0)),
        (0.25, Color::from_rgb8(255, 255, 0)),
        (0.50, Color::from_rgb8(0, 255, 0)),
        (0.75, Color::from_rgb8(0, 0, 255)),
        (1.00, Color::from_rgb8(255, 0, 0)),
    ];
    Brush::Gradient(Gradient::new_sweep(center, 0.0, std::f32::consts::TAU).with_stops(stops))
}

pub(crate) struct GmBlendGrid;
impl SnapshotCase for GmBlendGrid {
    fn name(&self) -> &'static str {
        "gm_blend_grid"
    }

    fn skia_max_diff_pixels(&self) -> u64 {
        // Skia's blend output can vary slightly across platforms/toolchains.
        3_000
    }

    fn vello_max_diff_pixels(&self) -> u64 {
        64
    }

    fn vello_hybrid_max_diff_pixels(&self) -> u64 {
        2_000
    }

    fn run(&self, sink: &mut dyn Sink, width: f64, height: f64) {
        background(sink, width, height, Color::from_rgb8(12, 12, 14));

        let cols = 4.0;
        let cell_w = width / cols;
        let cell_h = height / 3.0;
        let pad = 6.0;
        let tol = 0.1;

        let modes: &[(BlendMode, f32)] = &[
            (BlendMode::from(Compose::SrcOver), 1.0),
            (BlendMode::from(Compose::Copy), 1.0),
            (BlendMode::from(Compose::Xor), 0.9),
            (BlendMode::from(Compose::Plus), 0.8),
            (BlendMode::from(Mix::Multiply), 0.9),
            (BlendMode::from(Mix::Screen), 0.9),
            (BlendMode::from(Mix::Overlay), 0.9),
            (BlendMode::from(Mix::Darken), 0.9),
            (BlendMode::from(Mix::Lighten), 0.9),
            (BlendMode::from(Mix::ColorDodge), 0.9),
            (BlendMode::from(Mix::ColorBurn), 0.9),
            (BlendMode::from(Mix::HardLight), 0.9),
        ];

        for (i, (mode, alpha)) in modes.iter().enumerate() {
            let col = (i as f64) % cols;
            let row = ((i as f64) / cols).floor();
            let x0 = col * cell_w;
            let y0 = row * cell_h;
            let x1 = x0 + cell_w;
            let y1 = y0 + cell_h;

            sink.draw(Draw::Fill {
                transform: Affine::IDENTITY,
                fill_rule: Fill::NonZero,
                paint: linear_rainbow((f32p(x0), f32p(y0)), (f32p(x1), f32p(y1))),
                paint_transform: None,
                shape: Geometry::RoundedRect(RoundedRect::new(
                    x0 + pad,
                    y0 + pad,
                    x1 - pad,
                    y1 - pad,
                    14.0,
                )),
                composite: Composite::default(),
            });

            sink.draw(Draw::Fill {
                transform: Affine::IDENTITY,
                fill_rule: Fill::NonZero,
                paint: sweep_rainbow((f32p((x0 + x1) * 0.5), f32p((y0 + y1) * 0.5))),
                paint_transform: None,
                shape: circle_geometry(
                    ((x0 + x1) * 0.5, (y0 + y1) * 0.5),
                    cell_w.min(cell_h) * 0.24,
                    tol,
                ),
                composite: Composite::new(*mode, *alpha),
            });
        }
    }
}
