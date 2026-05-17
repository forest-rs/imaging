// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Build the smallest useful retained scene.

use imaging::{Painter, record::Scene};
use kurbo::{Rect, RoundedRect};
use peniko::Color;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut scene = Scene::new();

    {
        let mut painter = Painter::new(&mut scene);
        painter.fill_rect(Rect::new(0.0, 0.0, 160.0, 96.0), Color::WHITE);
        painter
            .fill(
                RoundedRect::new(24.0, 20.0, 136.0, 76.0, 10.0),
                Color::from_rgb8(0x2a, 0x6f, 0xdb),
            )
            .draw();
    }

    scene.validate()?;
    let diagnostics = scene.diagnose();

    println!("commands: {}", scene.commands().len());
    println!("diagnostics: {}", diagnostics.len());
    if !diagnostics.is_empty() {
        println!("{diagnostics:#?}");
    }

    Ok(())
}
