// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use imaging::{PaintSink, Painter, record::Glyph};
use kurbo::{Affine, Stroke};
use peniko::{Brush, Color, Fill, Style};
use skrifa::{FontRef, MetadataProvider};

use crate::cases::SnapshotCase;

use super::util::{background, test_font};

pub(super) struct GmGlyphRuns;

impl SnapshotCase for GmGlyphRuns {
    fn name(&self) -> &'static str {
        "gm_glyph_runs"
    }

    fn vello_max_diff_pixels(&self) -> u64 {
        4
    }

    fn run(&self, sink: &mut dyn PaintSink, width: f64, height: f64) {
        background(sink, width, height, Color::from_rgba8(247, 241, 232, 255));
        let mut painter = Painter::new(sink);

        let font = test_font();
        let fill_glyphs = glyphs_for_text(&font, 42.0, "imaging");
        let fill_paint = Brush::Solid(Color::from_rgba8(28, 32, 36, 255));
        let fill_style = Style::Fill(Fill::NonZero);
        painter
            .glyphs(&font, &fill_paint)
            .transform(Affine::translate((18.0, 88.0)))
            .font_size(42.0)
            .hint(true)
            .draw(&fill_style, &fill_glyphs);

        let stroke_glyphs = glyphs_for_text(&font, 34.0, "glyph run");
        let stroke_paint = Brush::Solid(Color::from_rgba8(178, 74, 30, 255));
        let stroke_style = Style::Stroke(Stroke::new(1.5));
        painter
            .glyphs(&font, &stroke_paint)
            .transform(Affine::translate((22.0, 172.0)))
            .glyph_transform(Some(Affine::skew(0.28, 0.0)))
            .font_size(34.0)
            .draw(&stroke_style, &stroke_glyphs);
    }
}

fn glyphs_for_text(font: &peniko::FontData, font_size: f32, text: &str) -> Vec<Glyph> {
    let font_ref = FontRef::from_index(font.data.as_ref(), font.index).expect("load snapshot font");
    let charmap = font_ref.charmap();
    let coords: &[skrifa::instance::NormalizedCoord] = &[];
    let glyph_metrics = font_ref.glyph_metrics(skrifa::instance::Size::new(font_size), coords);
    let mut pen_x = 0.0_f32;

    text.chars()
        .map(|ch| {
            let gid = charmap.map(ch).unwrap_or_default();
            let glyph = Glyph {
                id: gid.to_u32(),
                x: pen_x,
                y: 0.0,
            };
            pen_x += glyph_metrics.advance_width(gid).unwrap_or_default();
            glyph
        })
        .collect()
}
