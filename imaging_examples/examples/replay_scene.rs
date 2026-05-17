// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Replay a retained scene into another retained scene.

use imaging::{Painter, record};
use kurbo::Rect;
use peniko::Color;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut source = record::Scene::new();

    {
        let mut painter = Painter::new(&mut source);
        painter.fill_rect(Rect::new(0.0, 0.0, 96.0, 96.0), Color::WHITE);
        painter.with_context("replay-demo/card", None, |painter| {
            painter.fill_rect(
                Rect::new(16.0, 16.0, 80.0, 80.0),
                Color::from_rgb8(0xd9, 0x77, 0x06),
            );
        });
    }

    source.validate()?;

    let mut replayed = record::Scene::new();
    record::replay(&source, &mut replayed);

    assert_eq!(
        source, replayed,
        "replay should preserve the recorded scene"
    );
    println!("replayed {} commands", replayed.commands().len());

    Ok(())
}
