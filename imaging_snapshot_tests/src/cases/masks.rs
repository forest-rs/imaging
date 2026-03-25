// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use imaging::{MaskMode, PaintSink, Painter, record::Scene};
use kurbo::{Circle, Rect, RoundedRect, Stroke};
use peniko::{Color, Extend, Gradient};

use super::SnapshotCase;
use super::util::{background, f32p};

fn build_mask_scene(width: f64, height: f64) -> Scene {
    let mut scene = Scene::new();
    let mut painter = Painter::new(&mut scene);

    painter
        .fill(
            Circle::new((width * 0.33, height * 0.48), 40.0),
            Color::from_rgb8(24, 84, 232),
        )
        .draw();
    painter
        .fill(
            Circle::new((width * 0.5, height * 0.48), 40.0),
            Color::from_rgb8(34, 210, 120),
        )
        .draw();
    painter
        .fill(
            Circle::new((width * 0.67, height * 0.48), 40.0),
            Color::from_rgb8(255, 84, 76),
        )
        .draw();
    painter
        .fill(
            RoundedRect::new(
                width * 0.22,
                height * 0.62,
                width * 0.78,
                height * 0.8,
                20.0,
            ),
            Color::from_rgba8(255, 255, 255, 180),
        )
        .draw();

    scene
}

fn draw_content<S>(painter: &mut Painter<'_, S>, width: f64, height: f64)
where
    S: PaintSink + ?Sized,
{
    let gradient = Gradient::new_linear(
        (f32p(width * 0.2), f32p(height * 0.2)),
        (f32p(width * 0.8), f32p(height * 0.82)),
    )
    .with_extend(Extend::Pad)
    .with_stops([
        (0.0, Color::from_rgb8(255, 242, 128)),
        (0.5, Color::from_rgb8(255, 132, 54)),
        (1.0, Color::from_rgb8(255, 58, 96)),
    ]);

    painter
        .fill(
            RoundedRect::new(
                width * 0.18,
                height * 0.18,
                width * 0.82,
                height * 0.82,
                26.0,
            ),
            &gradient,
        )
        .draw();
    painter
        .stroke(
            RoundedRect::new(
                width * 0.18,
                height * 0.18,
                width * 0.82,
                height * 0.82,
                26.0,
            ),
            &Stroke::new(3.0),
            Color::from_rgba8(255, 255, 255, 224),
        )
        .draw();
    painter
        .fill(
            Rect::new(width * 0.22, height * 0.3, width * 0.78, height * 0.38),
            Color::from_rgba8(255, 255, 255, 80),
        )
        .draw();
}

fn run_mask_case(sink: &mut dyn PaintSink, width: f64, height: f64, mode: MaskMode) {
    background(sink, width, height, Color::from_rgb8(22, 24, 30));
    let mut painter = Painter::new(sink);

    let mask = Painter::<Scene>::record_mask(mode, |mask| {
        mask.replay(&build_mask_scene(width, height));
    });
    painter.with_group(
        imaging::GroupRef::new().with_mask(mask.as_ref()),
        |painter| {
            draw_content(painter, width, height);
        },
    );
    painter
        .fill(
            Rect::new(width * 0.08, height * 0.86, width * 0.92, height * 0.9),
            Color::from_rgba8(255, 255, 255, 22),
        )
        .draw();
}

pub(crate) struct GmMaskAlpha;
impl SnapshotCase for GmMaskAlpha {
    fn name(&self) -> &'static str {
        "gm_mask_alpha"
    }

    fn supports_backend(&self, backend: &str) -> bool {
        matches!(backend, "skia" | "vello_cpu")
    }

    fn run(&self, sink: &mut dyn PaintSink, width: f64, height: f64) {
        run_mask_case(sink, width, height, MaskMode::Alpha);
    }
}

pub(crate) struct GmMaskLuminance;
impl SnapshotCase for GmMaskLuminance {
    fn name(&self) -> &'static str {
        "gm_mask_luminance"
    }

    fn supports_backend(&self, backend: &str) -> bool {
        matches!(backend, "skia" | "vello_cpu" | "vello")
    }

    fn run(&self, sink: &mut dyn PaintSink, width: f64, height: f64) {
        run_mask_case(sink, width, height, MaskMode::Luminance);
    }
}
