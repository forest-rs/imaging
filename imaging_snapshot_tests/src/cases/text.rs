// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use imaging::{PaintSink, Painter, record::Glyph};
use kurbo::{Affine, Stroke};
use peniko::{Brush, Color, Fill, Style};
use skrifa::{FontRef, MetadataProvider};

use crate::cases::SnapshotCase;

use super::util::{background, test_font, test_image};

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

pub(super) struct GmGlyphRunsGradientFill;

impl SnapshotCase for GmGlyphRunsGradientFill {
    fn name(&self) -> &'static str {
        "gm_glyph_runs_gradient_fill"
    }

    fn run(&self, sink: &mut dyn PaintSink, width: f64, height: f64) {
        background(sink, width, height, Color::from_rgba8(247, 241, 232, 255));
        let mut painter = Painter::new(sink);
        let font = test_font();
        let glyphs = glyphs_for_text(&font, 44.0, "gradient");
        let brush = Brush::Gradient(
            peniko::Gradient::new_linear((0.0, 0.0), (width, 0.0)).with_stops([
                (0.0, Color::from_rgba8(190, 44, 44, 255)),
                (0.5, Color::from_rgba8(245, 165, 36, 255)),
                (1.0, Color::from_rgba8(52, 88, 160, 255)),
            ]),
        );
        let style = Style::Fill(Fill::NonZero);
        painter
            .glyphs(&font, &brush)
            .transform(Affine::translate((18.0, 112.0)))
            .font_size(44.0)
            .hint(true)
            .draw(&style, &glyphs);
    }
}

pub(super) struct GmGlyphRunsGradientStroke;

impl SnapshotCase for GmGlyphRunsGradientStroke {
    fn name(&self) -> &'static str {
        "gm_glyph_runs_gradient_stroke"
    }

    fn run(&self, sink: &mut dyn PaintSink, width: f64, height: f64) {
        background(sink, width, height, Color::from_rgba8(241, 243, 248, 255));
        let mut painter = Painter::new(sink);
        let font = test_font();
        let glyphs = glyphs_for_text(&font, 42.0, "outline");
        let brush = Brush::Gradient(
            peniko::Gradient::new_linear((0.0, 0.0), (0.0, height)).with_stops([
                (0.0, Color::from_rgba8(29, 39, 72, 255)),
                (1.0, Color::from_rgba8(74, 120, 216, 255)),
            ]),
        );
        let style = Style::Stroke(Stroke::new(2.0));
        painter
            .glyphs(&font, &brush)
            .transform(Affine::translate((18.0, 120.0)))
            .glyph_transform(Some(Affine::skew(0.22, 0.0)))
            .font_size(42.0)
            .draw(&style, &glyphs);
    }
}

pub(super) struct GmGlyphRunsImageFill;

impl SnapshotCase for GmGlyphRunsImageFill {
    fn name(&self) -> &'static str {
        "gm_glyph_runs_image_fill"
    }

    fn run(&self, sink: &mut dyn PaintSink, width: f64, height: f64) {
        background(sink, width, height, Color::from_rgba8(247, 243, 236, 255));
        let mut painter = Painter::new(sink);
        let font = test_font();
        let glyphs = glyphs_for_text(&font, 42.0, "texture");
        let brush = Brush::Image(
            peniko::ImageBrush::new(test_image())
                .with_quality(peniko::ImageQuality::Medium)
                .with_extend(peniko::Extend::Repeat),
        );
        let style = Style::Fill(Fill::NonZero);
        painter
            .glyphs(&font, &brush)
            .transform(Affine::translate((14.0, 116.0)))
            .font_size(42.0)
            .hint(true)
            .draw(&style, &glyphs);
    }
}

pub(super) struct GmGlyphRunsImageStroke;

impl SnapshotCase for GmGlyphRunsImageStroke {
    fn name(&self) -> &'static str {
        "gm_glyph_runs_image_stroke"
    }

    fn run(&self, sink: &mut dyn PaintSink, width: f64, height: f64) {
        background(sink, width, height, Color::from_rgba8(240, 242, 246, 255));
        let mut painter = Painter::new(sink);
        let font = test_font();
        let glyphs = glyphs_for_text(&font, 40.0, "repeat");
        let brush = Brush::Image(
            peniko::ImageBrush::new(test_image())
                .with_quality(peniko::ImageQuality::Medium)
                .with_extend(peniko::Extend::Repeat),
        );
        let style = Style::Stroke(Stroke::new(1.75));
        painter
            .glyphs(&font, &brush)
            .transform(Affine::translate((18.0, 118.0)))
            .glyph_transform(Some(Affine::skew(0.18, 0.0)))
            .font_size(40.0)
            .draw(&style, &glyphs);
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
