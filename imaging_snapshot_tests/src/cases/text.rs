// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use imaging::{PaintSink, Painter, record::Glyph};
use kurbo::{Affine, Rect, Stroke};
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

pub(super) struct GmGlyphRunsEmbolden;

impl SnapshotCase for GmGlyphRunsEmbolden {
    fn name(&self) -> &'static str {
        "gm_glyph_runs_embolden"
    }

    fn supports_backend(&self, backend: &str) -> bool {
        backend == "tiny_skia"
    }

    fn run(&self, sink: &mut dyn PaintSink, width: f64, height: f64) {
        background(sink, width, height, Color::from_rgba8(244, 240, 233, 255));
        let mut painter = Painter::new(sink);

        let font = test_font();
        let base_glyphs = glyphs_for_text(&font, 36.0, "regular");
        let emboldened_glyphs = glyphs_for_text(&font, 36.0, "embolden");
        let fill_style = Style::Fill(Fill::NonZero);

        painter
            .glyphs(&font, &Brush::Solid(Color::from_rgba8(39, 44, 52, 255)))
            .transform(Affine::translate((20.0, height * 0.38)))
            .font_size(36.0)
            .hint(true)
            .draw(&fill_style, &base_glyphs);

        painter
            .glyphs(&font, &Brush::Solid(Color::from_rgba8(171, 76, 33, 255)))
            .transform(Affine::translate((20.0, height * 0.7)))
            .font_size(36.0)
            .font_embolden(kurbo::Vec2::new(1.5, 0.5))
            .hint(true)
            .draw(&fill_style, &emboldened_glyphs);
    }
}

pub(super) struct GmGlyphRunsGradientStroke;

impl SnapshotCase for GmGlyphRunsGradientStroke {
    fn name(&self) -> &'static str {
        "gm_glyph_runs_gradient_stroke"
    }

    fn vello_max_diff_pixels(&self) -> u64 {
        4
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

    fn vello_max_diff_pixels(&self) -> u64 {
        4
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

pub(super) struct GmTextEditorLorem;

impl SnapshotCase for GmTextEditorLorem {
    fn name(&self) -> &'static str {
        "gm_text_editor_lorem"
    }

    fn supports_backend(&self, backend: &str) -> bool {
        matches!(backend, "tiny_skia" | "vello_cpu" | "skia")
    }

    fn run(&self, sink: &mut dyn PaintSink, width: f64, height: f64) {
        const TAB_TEXT: &str = "lorem_notes.md";
        const STATUS_TEXT: &str = "UTF-8  Ln 18, Col 24  Spaces: 4";
        const LINE_TEXTS: &[&str] = &[
            "lorem ipsum dolor sit amet, consectetur adipiscing elit;",
            "sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.",
            "ut enim ad minim veniam, quis nostrud exercitation ullamco laboris.",
            "nisi ut aliquip ex ea commodo consequat duis aute irure dolor.",
            "in reprehenderit in voluptate velit esse cillum dolore eu fugiat.",
            "nulla pariatur excepteur sint occaecat cupidatat non proident.",
            "sunt in culpa qui officia deserunt mollit anim id est laborum.",
            "lorem ipsum dolor sit amet, sed ut perspiciatis unde omnis iste.",
            "natus error sit voluptatem accusantium doloremque laudantium totam.",
            "rem aperiam eaque ipsa quae ab illo inventore veritatis et quasi.",
            "architecto beatae vitae dicta sunt explicabo nemo enim ipsam.",
            "voluptatem quia voluptas sit aspernatur aut odit aut fugit sed.",
            "quia consequuntur magni dolores eos qui ratione voluptatem sequi.",
            "nesciunt neque porro quisquam est qui dolorem ipsum quia dolor.",
            "sit amet consectetur adipisci velit sed quia non numquam eius.",
            "modi tempora incidunt ut labore et dolore magnam aliquam quaerat.",
            "voluptatem ut enim ad minima veniam quis nostrum exercitationem.",
            "ullam corporis suscipit laboriosam nisi ut aliquid ex ea commodi.",
        ];

        background(sink, width, height, Color::from_rgba8(24, 28, 34, 255));
        let mut painter = Painter::new(sink);
        let font = test_font();

        let tab_bar_height = 21.0;
        let status_bar_height = 15.0;
        let gutter_width = 24.0;
        let text_top = tab_bar_height + 9.0;
        let line_height = 10.6;
        let tab_rect = Rect::new(10.0, 3.0, 118.0, tab_bar_height + 1.0);
        let editor_rect = Rect::new(8.0, tab_bar_height, width - 8.0, height - status_bar_height);
        let current_line_rect = Rect::new(
            gutter_width + 9.0,
            text_top + line_height * 4.0 - 7.0,
            width - 12.0,
            text_top + line_height * 4.0 + 2.5,
        );

        painter.fill_rect(
            Rect::new(0.0, 0.0, width, tab_bar_height),
            Color::from_rgba8(33, 38, 45, 255),
        );
        painter.fill_rect(tab_rect, Color::from_rgba8(43, 49, 58, 255));
        painter.fill_rect(editor_rect, Color::from_rgba8(30, 34, 41, 255));
        painter.fill_rect(
            Rect::new(
                8.0,
                tab_bar_height,
                gutter_width + 10.0,
                height - status_bar_height,
            ),
            Color::from_rgba8(27, 31, 37, 255),
        );
        painter.fill_rect(current_line_rect, Color::from_rgba8(39, 45, 54, 255));
        painter.fill_rect(
            Rect::new(0.0, height - status_bar_height, width, height),
            Color::from_rgba8(19, 111, 99, 255),
        );
        painter.fill_rect(
            Rect::new(
                gutter_width + 8.0,
                tab_bar_height,
                gutter_width + 9.0,
                height - status_bar_height,
            ),
            Color::from_rgba8(49, 55, 64, 255),
        );
        painter.fill_rect(
            Rect::new(
                gutter_width + 44.0,
                text_top + line_height * 4.0 - 6.0,
                gutter_width + 108.0,
                text_top + line_height * 4.0 + 2.0,
            ),
            Color::from_rgba8(74, 97, 138, 255),
        );
        painter.fill_rect(
            Rect::new(
                gutter_width + 110.0,
                text_top + line_height * 11.0 - 6.0,
                gutter_width + 176.0,
                text_top + line_height * 11.0 + 2.0,
            ),
            Color::from_rgba8(87, 73, 120, 255),
        );
        painter.fill_rect(
            Rect::new(
                gutter_width + 76.0,
                text_top + line_height * 16.0 - 6.0,
                gutter_width + 118.0,
                text_top + line_height * 16.0 + 2.0,
            ),
            Color::from_rgba8(135, 74, 52, 255),
        );

        let ui_brush = Brush::Solid(Color::from_rgba8(205, 212, 220, 255));
        let line_no_brush = Brush::Solid(Color::from_rgba8(106, 116, 128, 255));
        let text_brush = Brush::Solid(Color::from_rgba8(214, 220, 229, 255));
        let active_no_brush = Brush::Solid(Color::from_rgba8(158, 195, 255, 255));
        let status_brush = Brush::Solid(Color::from_rgba8(233, 247, 244, 255));
        let fill_style = Style::Fill(Fill::NonZero);

        let tab_glyphs = glyphs_for_text(&font, 8.0, TAB_TEXT);
        painter
            .glyphs(&font, &ui_brush)
            .transform(Affine::translate((14.0, 14.0)))
            .font_size(8.0)
            .hint(true)
            .draw(&fill_style, &tab_glyphs);

        let status_glyphs = glyphs_for_text(&font, 7.5, STATUS_TEXT);
        painter
            .glyphs(&font, &status_brush)
            .transform(Affine::translate((11.0, height - 4.6)))
            .font_size(7.5)
            .hint(true)
            .draw(&fill_style, &status_glyphs);

        for (index, text) in LINE_TEXTS.iter().enumerate() {
            let baseline_y = text_top + line_height * (index as f64);
            let line_number = (index + 1).to_string();
            let line_no_glyphs = glyphs_for_text(&font, 7.5, &line_number);
            let line_no_paint = if index == 4 {
                &active_no_brush
            } else {
                &line_no_brush
            };
            painter
                .glyphs(&font, line_no_paint)
                .transform(Affine::translate((11.0, baseline_y)))
                .font_size(7.5)
                .hint(true)
                .draw(&fill_style, &line_no_glyphs);

            let text_glyphs = glyphs_for_text(&font, 8.5, text);
            painter
                .glyphs(&font, &text_brush)
                .transform(Affine::translate((gutter_width + 13.0, baseline_y)))
                .font_size(8.5)
                .hint(true)
                .draw(&fill_style, &text_glyphs);
        }
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
