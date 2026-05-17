// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Count commands to show that `PaintSink` is only a command target.

use imaging::{
    BlurredRoundedRect, ClipRef, FillRef, GlyphRunRef, GroupRef, PaintSink, Painter, StrokeRef,
};
use kurbo::Rect;
use peniko::Color;

#[derive(Default, Debug)]
struct CountingSink {
    clips: usize,
    fills: usize,
    groups: usize,
    glyph_runs: usize,
    strokes: usize,
    blurred_rounded_rects: usize,
}

impl PaintSink for CountingSink {
    fn push_clip(&mut self, _clip: ClipRef<'_>) {
        self.clips += 1;
    }

    fn pop_clip(&mut self) {}

    fn push_group(&mut self, _group: GroupRef<'_>) {
        self.groups += 1;
    }

    fn pop_group(&mut self) {}

    fn fill(&mut self, _draw: FillRef<'_>) {
        self.fills += 1;
    }

    fn stroke(&mut self, _draw: StrokeRef<'_>) {
        self.strokes += 1;
    }

    fn glyph_run(
        &mut self,
        _draw: GlyphRunRef<'_>,
        _glyphs: &mut dyn Iterator<Item = imaging::record::Glyph>,
    ) {
        self.glyph_runs += 1;
    }

    fn blurred_rounded_rect(&mut self, _draw: BlurredRoundedRect) {
        self.blurred_rounded_rects += 1;
    }
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

    println!("{sink:#?}");
}
