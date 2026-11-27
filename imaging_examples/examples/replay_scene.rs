// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Replay a retained scene into another retained scene.

use imaging::{Painter, kurbo::Rect, peniko::Color, record};

fn main() -> Result<(), record::ValidateError> {
    let mut source = record::Scene::new();

    {
        let mut painter = Painter::new(&mut source);
        painter.fill_rect(Rect::new(0.0, 0.0, 96.0, 96.0), Color::WHITE);
        imaging::with_context!(painter, "replay-demo/card", |painter| {
            painter.fill_rect(
                Rect::new(16.0, 16.0, 80.0, 80.0),
                Color::from_rgb8(0xd9, 0x77, 0x06),
            );
        });
    }

    source.validate()?;

    let mut replayed = record::Scene::new();
    record::replay(&source, &mut replayed);

    assert_eq!(source, replayed, "replay should preserve this recording");
    println!("replayed {} commands", replayed.commands().len());

    Ok(())
}
