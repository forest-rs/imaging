// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Count commands to show that `PaintSink` is only a command target.

use imaging::{
    BlurredRoundedRect, ClipRef, FillRef, GlyphRunRef, GroupRef, PaintSink, Painter, StrokeRef,
    kurbo::Rect, peniko::Color, record::Glyph,
};

#[derive(Default)]
struct CountingSink {
    clips: usize,
    fills: usize,
}

impl PaintSink for CountingSink {
    fn push_clip(&mut self, _clip: ClipRef<'_>) {
        self.clips += 1;
    }

    fn pop_clip(&mut self) {}

    fn push_group(&mut self, _group: GroupRef<'_>) {}

    fn pop_group(&mut self) {}

    fn fill(&mut self, _draw: FillRef<'_>) {
        self.fills += 1;
    }

    fn stroke(&mut self, _draw: StrokeRef<'_>) {}

    fn glyph_run(&mut self, _draw: GlyphRunRef<'_>, _glyphs: &mut dyn Iterator<Item = Glyph>) {}

    fn blurred_rounded_rect(&mut self, _draw: BlurredRoundedRect) {}
}

fn main() {
    let mut sink = CountingSink::default();

    {
        let mut painter = Painter::new(&mut sink);
        painter.fill_rect(Rect::new(0.0, 0.0, 128.0, 80.0), Color::WHITE);
        painter.with_fill_clip(Rect::new(16.0, 16.0, 112.0, 64.0), |painter| {
            painter.fill_rect(
                Rect::new(28.0, 24.0, 100.0, 56.0),
                Color::from_rgb8(0x2a, 0x6f, 0xdb),
            );
        });
    }

    assert_eq!(sink.fills, 2, "each rectangle should emit one fill");
    assert_eq!(sink.clips, 1, "the scoped clip should emit one clip");
    println!("counted {} fills and {} clip", sink.fills, sink.clips);
}
