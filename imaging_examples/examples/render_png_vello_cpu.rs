// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Render a retained scene to a PNG with the Vello CPU backend.

use std::{fs::File, path::PathBuf};

use image::{ColorType, ImageEncoder, codecs::png::PngEncoder};
use imaging::{Painter, record::Scene};
use imaging_vello_cpu::VelloCpuRenderer;
use kurbo::{Rect, RoundedRect};
use peniko::Color;

const WIDTH: u16 = 320;
const HEIGHT: u16 = 180;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("out.png"));

    let mut scene = Scene::new();
    {
        let mut painter = Painter::new(&mut scene);
        painter.fill_rect(
            Rect::new(0.0, 0.0, f64::from(WIDTH), f64::from(HEIGHT)),
            Color::from_rgb8(0xf6, 0xf7, 0xfb),
        );
        painter
            .fill(
                RoundedRect::new(48.0, 42.0, 272.0, 138.0, 18.0),
                Color::from_rgb8(0x1d, 0x4e, 0x89),
            )
            .draw();
        painter
            .fill(
                RoundedRect::new(72.0, 66.0, 248.0, 114.0, 10.0),
                Color::from_rgba8(0xff, 0xff, 0xff, 0x60),
            )
            .draw();
    }

    scene.validate()?;

    let mut renderer = VelloCpuRenderer::new(WIDTH, HEIGHT);
    let image = renderer.render_scene(&scene, WIDTH, HEIGHT)?;

    let file = File::create(&output)?;
    PngEncoder::new(file).write_image(
        &image.data,
        image.width,
        image.height,
        ColorType::Rgba8.into(),
    )?;
    println!("wrote {}", output.display());

    Ok(())
}
