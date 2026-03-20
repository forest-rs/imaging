// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use imaging::{Composite, PaintSink, Painter, record::Geometry};
use kurbo::RoundedRect;
use peniko::{BlendMode, Brush, Color, Compose, Gradient, Mix};

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

    fn vello_max_diff_pixels(&self) -> u64 {
        32
    }

    fn run(&self, sink: &mut dyn PaintSink, width: f64, height: f64) {
        background(sink, width, height, Color::from_rgb8(12, 12, 14));
        let mut painter = Painter::new(sink);

        let cols = 4_usize;
        let cell_w = width / cols as f64;
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
            let col = (i % cols) as f64;
            let row = (i / cols) as f64;
            let x0 = col * cell_w;
            let y0 = row * cell_h;
            let x1 = x0 + cell_w;
            let y1 = y0 + cell_h;

            let linear = linear_rainbow((f32p(x0), f32p(y0)), (f32p(x1), f32p(y1)));
            painter
                .fill(
                    Geometry::RoundedRect(RoundedRect::new(
                        x0 + pad,
                        y0 + pad,
                        x1 - pad,
                        y1 - pad,
                        14.0,
                    )),
                    &linear,
                )
                .draw();

            let sweep = sweep_rainbow((f32p((x0 + x1) * 0.5), f32p((y0 + y1) * 0.5)));
            painter
                .fill(
                    circle_geometry(
                        ((x0 + x1) * 0.5, (y0 + y1) * 0.5),
                        cell_w.min(cell_h) * 0.24,
                        tol,
                    ),
                    &sweep,
                )
                .composite(Composite::new(*mode, *alpha))
                .draw();
        }
    }
}
