// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Vector text effects demo.
//!
//! Takes a string of text and renders a sheet of purely vector/imaging-based text effects
//! through the backend-agnostic [`imaging`] API, rasterized with the `vello_cpu` backend.
//!
//! Every effect here is resolution-independent: effects are expressed as glyph runs, glyph
//! outline paths, clips, masks, gradients, and group filters — never as pixel or character
//! tricks. The same scene could be replayed into any other `imaging` backend.
//!
//! Run with:
//!
//! ```sh
//! cargo run -p imaging_snapshot_tests --features vello_cpu --example text_effects -- "Imaging"
//! ```
//!
//! The sheet is written to `target/text_effects.png` (override with a second argument).

use std::sync::Arc;

use imaging::{Filter, GroupRef, PaintSink, Painter, record};
use kurbo::{Affine, BezPath, PathEl, Point, Rect, Shape as _, Stroke, Vec2};
use peniko::{Blob, Brush, Color, FontData, Style};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{FontRef, GlyphId, MetadataProvider};

const ROBOTO_FONT_BYTES: &[u8] = include_bytes!("../src/assets/roboto/Roboto-Regular.ttf");

const SHEET_WIDTH: f64 = 900.0;
const BAND_HEIGHT: f64 = 150.0;
const MARGIN_X: f64 = 40.0;
const DISPLAY_SIZE: f32 = 64.0;
const LABEL_SIZE: f32 = 13.0;

fn main() {
    let mut args = std::env::args().skip(1);
    let text = args.next().unwrap_or_else(|| "Imaging".to_string());
    let out_path = args
        .next()
        .unwrap_or_else(|| "target/text_effects.png".to_string());

    let font = FontData::new(Blob::new(Arc::new(ROBOTO_FONT_BYTES)), 0);

    let bands: Vec<(&str, BandFn)> = vec![
        ("gradient fill", draw_gradient_fill),
        ("dashed outline", draw_dashed_outline),
        ("glow + drop shadow", draw_glow_shadow),
        ("stripe clip", draw_stripe_clip),
        ("per-glyph wave", draw_wave),
        ("outline warp (flag)", draw_flag_warp),
        ("text on an arc", draw_arc_text),
        ("extrude", draw_extrude),
        ("knockout", draw_knockout),
    ];

    #[allow(
        clippy::cast_possible_truncation,
        reason = "Sheet dimensions are small compile-time constants."
    )]
    let (width, height) = (
        SHEET_WIDTH as u16,
        (BAND_HEIGHT * bands.len() as f64) as u16,
    );

    let mut scene = record::Scene::new();
    {
        let mut base_painter = Painter::new(&mut scene);
        let mut painter = base_painter.as_dyn();
        painter.fill_rect(
            Rect::new(0.0, 0.0, f64::from(width), f64::from(height)),
            Color::from_rgb8(24, 26, 32),
        );

        for (index, (label, draw)) in bands.iter().enumerate() {
            let band = Band {
                top: BAND_HEIGHT * index as f64,
                baseline: BAND_HEIGHT * index as f64 + 104.0,
            };
            draw_label(&mut painter, &font, band.top, label);
            draw(&mut painter, &font, &text, band);
        }
    }

    scene
        .validate()
        .expect("text effects scene should validate");

    let mut renderer = imaging_vello_cpu::VelloCpuRenderer::new(width, height);
    let image = renderer
        .render_scene(&scene, width, height)
        .expect("render text effects scene");

    let png = kompari::image_to_png(
        &kompari::image::ImageBuffer::from_raw(image.width, image.height, image.data)
            .expect("RGBA buffer size should match image dimensions"),
        kompari::SizeOptimizationLevel::Fast,
    );
    if let Some(parent) = std::path::Path::new(&out_path).parent() {
        std::fs::create_dir_all(parent).expect("create output directory");
    }
    std::fs::write(&out_path, png).expect("write output png");
    println!("wrote {out_path} ({width}x{height})");
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "Inputs are clamped to the u8 channel range first."
)]
fn channel(value: f64) -> u8 {
    value.clamp(0.0, 255.0) as u8
}

/// Vertical placement of one effect band on the sheet.
#[derive(Clone, Copy)]
struct Band {
    top: f64,
    baseline: f64,
}

type BandFn = fn(&mut Painter<'_, dyn PaintSink + '_>, &FontData, &str, Band);

/// Map characters to positioned glyphs with metrics-based advances.
///
/// This is deliberately simple positioning (no shaping/kerning); a real application would use
/// Parley to produce the same `(glyph id, x, y)` stream with full shaping, and everything below
/// would work unchanged.
fn layout_glyphs(font: &FontData, font_size: f32, text: &str) -> Vec<record::Glyph> {
    let font_ref = FontRef::from_index(font.data.as_ref(), font.index).expect("load demo font");
    let charmap = font_ref.charmap();
    let coords: &[skrifa::instance::NormalizedCoord] = &[];
    let glyph_metrics = font_ref.glyph_metrics(Size::new(font_size), coords);
    let mut pen_x = 0.0_f32;

    text.chars()
        .map(|ch| {
            let gid = charmap.map(ch).unwrap_or_default();
            let glyph = record::Glyph {
                id: gid.to_u32(),
                x: pen_x,
                y: 0.0,
            };
            pen_x += glyph_metrics.advance_width(gid).unwrap_or_default();
            glyph
        })
        .collect()
}

fn text_advance(font: &FontData, font_size: f32, text: &str) -> f64 {
    let font_ref = FontRef::from_index(font.data.as_ref(), font.index).expect("load demo font");
    let charmap = font_ref.charmap();
    let coords: &[skrifa::instance::NormalizedCoord] = &[];
    let glyph_metrics = font_ref.glyph_metrics(Size::new(font_size), coords);
    text.chars()
        .map(|ch| {
            f64::from(
                glyph_metrics
                    .advance_width(charmap.map(ch).unwrap_or_default())
                    .unwrap_or_default(),
            )
        })
        .sum()
}

/// Collects skrifa outline callbacks into a y-down [`BezPath`].
struct BezPathPen {
    path: BezPath,
    offset: Vec2,
}

impl BezPathPen {
    fn point(&self, x: f32, y: f32) -> Point {
        // Font outlines are y-up; the canvas is y-down.
        Point::new(self.offset.x + f64::from(x), self.offset.y - f64::from(y))
    }
}

impl OutlinePen for BezPathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.point(x, y);
        self.path.move_to(p);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.point(x, y);
        self.path.line_to(p);
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let c0 = self.point(cx0, cy0);
        let p = self.point(x, y);
        self.path.quad_to(c0, p);
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let c0 = self.point(cx0, cy0);
        let c1 = self.point(cx1, cy1);
        let p = self.point(x, y);
        self.path.curve_to(c0, c1, p);
    }

    fn close(&mut self) {
        self.path.close_path();
    }
}

/// Extract the whole string as one vector outline path, positioned at `origin` (baseline-left).
fn text_outline_path(font: &FontData, font_size: f32, text: &str, origin: Point) -> BezPath {
    let font_ref = FontRef::from_index(font.data.as_ref(), font.index).expect("load demo font");
    let outlines = font_ref.outline_glyphs();
    let glyphs = layout_glyphs(font, font_size, text);

    let mut pen = BezPathPen {
        path: BezPath::new(),
        offset: Vec2::ZERO,
    };
    for glyph in glyphs {
        let Some(outline) = outlines.get(GlyphId::new(glyph.id)) else {
            continue;
        };
        pen.offset = Vec2::new(origin.x + f64::from(glyph.x), origin.y + f64::from(glyph.y));
        let settings = DrawSettings::unhinted(Size::new(font_size), LocationRef::default());
        outline
            .draw(settings, &mut pen)
            .expect("draw glyph outline");
    }
    pen.path
}

fn draw_label(
    painter: &mut Painter<'_, dyn PaintSink + '_>,
    font: &FontData,
    top: f64,
    label: &str,
) {
    let glyphs = layout_glyphs(font, LABEL_SIZE, label);
    let brush = Brush::Solid(Color::from_rgba8(148, 158, 178, 255));
    painter
        .glyphs(font, &brush)
        .transform(Affine::translate((16.0, top + 24.0)))
        .font_size(LABEL_SIZE)
        .hint(true)
        .draw(&Style::Fill(peniko::Fill::NonZero), &glyphs);
}

/// Linear gradient swept across the run, expressed as a glyph-run brush.
fn draw_gradient_fill(
    painter: &mut Painter<'_, dyn PaintSink + '_>,
    font: &FontData,
    text: &str,
    band: Band,
) {
    let glyphs = layout_glyphs(font, DISPLAY_SIZE, text);
    let advance = text_advance(font, DISPLAY_SIZE, text);
    let brush = Brush::Gradient(
        peniko::Gradient::new_linear((0.0, 0.0), (advance, 0.0)).with_stops([
            (0.0, Color::from_rgb8(255, 94, 87)),
            (0.5, Color::from_rgb8(255, 195, 60)),
            (1.0, Color::from_rgb8(80, 200, 255)),
        ]),
    );
    painter
        .glyphs(font, &brush)
        .transform(Affine::translate((MARGIN_X, band.baseline)))
        .font_size(DISPLAY_SIZE)
        .draw(&Style::Fill(peniko::Fill::NonZero), &glyphs);
}

/// Dashed stroke over the glyph outlines — a marching-ants outline style.
fn draw_dashed_outline(
    painter: &mut Painter<'_, dyn PaintSink + '_>,
    font: &FontData,
    text: &str,
    band: Band,
) {
    let glyphs = layout_glyphs(font, DISPLAY_SIZE, text);

    // Faint solid fill underneath so the letterforms stay readable.
    let under = Brush::Solid(Color::from_rgba8(88, 96, 112, 90));
    painter
        .glyphs(font, &under)
        .transform(Affine::translate((MARGIN_X, band.baseline)))
        .font_size(DISPLAY_SIZE)
        .draw(&Style::Fill(peniko::Fill::NonZero), &glyphs);

    let brush = Brush::Solid(Color::from_rgb8(122, 226, 255));
    let stroke = Stroke::new(1.6).with_dashes(0.0, [6.0, 4.0]);
    painter
        .glyphs(font, &brush)
        .transform(Affine::translate((MARGIN_X, band.baseline)))
        .font_size(DISPLAY_SIZE)
        .draw(&Style::Stroke(stroke), &glyphs);
}

/// Group filters: a blurred bright copy behind (glow) plus a drop shadow on the sharp copy.
fn draw_glow_shadow(
    painter: &mut Painter<'_, dyn PaintSink + '_>,
    font: &FontData,
    text: &str,
    band: Band,
) {
    let glyphs = layout_glyphs(font, DISPLAY_SIZE, text);
    let style = Style::Fill(peniko::Fill::NonZero);

    let glow_filters = [Filter::Blur {
        std_deviation_x: 7.0,
        std_deviation_y: 7.0,
    }];
    let glow_brush = Brush::Solid(Color::from_rgb8(120, 240, 200));
    painter.with_group(GroupRef::new().with_filters(&glow_filters), |painter| {
        painter
            .glyphs(font, &glow_brush)
            .transform(Affine::translate((MARGIN_X, band.baseline)))
            .font_size(DISPLAY_SIZE)
            .draw(&style, &glyphs);
    });

    let shadow_filters = [Filter::DropShadow {
        dx: 3.0,
        dy: 5.0,
        std_deviation_x: 2.5,
        std_deviation_y: 2.5,
        color: Color::from_rgba8(0, 0, 0, 160),
    }];
    let top_brush = Brush::Solid(Color::from_rgb8(235, 255, 248));
    painter.with_group(GroupRef::new().with_filters(&shadow_filters), |painter| {
        painter
            .glyphs(font, &top_brush)
            .transform(Affine::translate((MARGIN_X, band.baseline)))
            .font_size(DISPLAY_SIZE)
            .draw(&style, &glyphs);
    });
}

/// The string's outline path used as a clip; anything can be painted "inside" the text.
fn draw_stripe_clip(
    painter: &mut Painter<'_, dyn PaintSink + '_>,
    font: &FontData,
    text: &str,
    band: Band,
) {
    let outline = text_outline_path(
        font,
        DISPLAY_SIZE,
        text,
        Point::new(MARGIN_X, band.baseline),
    );
    let bounds = outline.bounding_box();
    painter.with_fill_clip(&outline, |painter| {
        // Diagonal stripes painted through the text-shaped clip.
        let stripe = Brush::Solid(Color::from_rgb8(255, 170, 60));
        let backdrop = Brush::Solid(Color::from_rgb8(120, 70, 190));
        painter.fill_rect(bounds.inflate(4.0, 4.0), &backdrop);
        let mut x = bounds.min_x() - bounds.height();
        while x < bounds.max_x() + bounds.height() {
            let mut path = BezPath::new();
            path.move_to((x, bounds.max_y() + 4.0));
            path.line_to((x + 9.0, bounds.max_y() + 4.0));
            path.line_to((x + 9.0 + bounds.height(), bounds.min_y() - 4.0));
            path.line_to((x + bounds.height(), bounds.min_y() - 4.0));
            path.close_path();
            painter.fill(&path, &stripe).draw();
            x += 18.0;
        }
    });
}

/// Per-glyph placement: each glyph rides a sine wave with a matching rotation.
fn draw_wave(
    painter: &mut Painter<'_, dyn PaintSink + '_>,
    font: &FontData,
    text: &str,
    band: Band,
) {
    let glyphs = layout_glyphs(font, DISPLAY_SIZE, text);
    let advance = text_advance(font, DISPLAY_SIZE, text);
    let style = Style::Fill(peniko::Fill::NonZero);
    let amplitude = 12.0;
    let wavelength = 190.0;

    for glyph in &glyphs {
        let x = MARGIN_X + f64::from(glyph.x);
        let phase = f64::from(glyph.x) / wavelength * std::f64::consts::TAU;
        let y = band.baseline - 6.0 + amplitude * phase.sin();
        // Slope of the wave gives each glyph its rotation.
        let angle = (amplitude * phase.cos() * std::f64::consts::TAU / wavelength).atan();
        let t = f64::from(glyph.x) / advance;
        let hue_color =
            Color::from_rgb8(channel(140.0 + 100.0 * t), channel(220.0 - 120.0 * t), 255);
        let brush = Brush::Solid(hue_color);
        let single = [record::Glyph {
            id: glyph.id,
            x: 0.0,
            y: 0.0,
        }];
        painter
            .glyphs(font, &brush)
            .transform(Affine::translate((x, y)) * Affine::rotate(angle))
            .font_size(DISPLAY_SIZE)
            .draw(&style, single);
    }
}

/// True outline warping: glyph outlines are flattened and every point is displaced.
///
/// This is the effect class that needs real vector data — it cannot be expressed as a glyph run
/// plus affine transforms.
fn draw_flag_warp(
    painter: &mut Painter<'_, dyn PaintSink + '_>,
    font: &FontData,
    text: &str,
    band: Band,
) {
    let outline = text_outline_path(
        font,
        DISPLAY_SIZE,
        text,
        Point::new(MARGIN_X, band.baseline - 8.0),
    );
    let bounds = outline.bounding_box();
    let amplitude = 10.0;
    let wavelength = 160.0;
    let warp = |p: Point| -> Point {
        // Flag ripple: sine displacement that grows toward the right edge.
        let strength = ((p.x - bounds.min_x()) / bounds.width()).clamp(0.0, 1.0);
        let phase = (p.x - bounds.min_x()) / wavelength * std::f64::consts::TAU;
        Point::new(p.x, p.y + amplitude * strength * phase.sin())
    };

    let mut warped = BezPath::new();
    kurbo::flatten(outline.path_elements(0.15), 0.15, |el| match el {
        PathEl::MoveTo(p) => warped.move_to(warp(p)),
        PathEl::LineTo(p) => warped.line_to(warp(p)),
        PathEl::ClosePath => warped.close_path(),
        _ => unreachable!("flatten emits only move/line/close"),
    });

    let brush = Brush::Gradient(
        peniko::Gradient::new_linear((bounds.min_x(), 0.0), (bounds.max_x(), 0.0)).with_stops([
            (0.0, Color::from_rgb8(255, 120, 120)),
            (1.0, Color::from_rgb8(255, 220, 120)),
        ]),
    );
    painter.fill(&warped, &brush).draw();
}

/// Glyphs positioned and rotated along a circular arc.
fn draw_arc_text(
    painter: &mut Painter<'_, dyn PaintSink + '_>,
    font: &FontData,
    text: &str,
    band: Band,
) {
    let font_ref = FontRef::from_index(font.data.as_ref(), font.index).expect("load demo font");
    let charmap = font_ref.charmap();
    let coords: &[skrifa::instance::NormalizedCoord] = &[];
    let size = DISPLAY_SIZE * 0.82;
    let glyph_metrics = font_ref.glyph_metrics(Size::new(size), coords);

    let advance = text_advance(font, size, text);
    let radius = 420.0;
    let center = Point::new(SHEET_WIDTH / 2.0, band.baseline + radius - 24.0);
    let style = Style::Fill(peniko::Fill::NonZero);
    let brush = Brush::Solid(Color::from_rgb8(170, 200, 255));

    let mut pen_x = 0.0_f64;
    for ch in text.chars() {
        let gid = charmap.map(ch).unwrap_or_default();
        let glyph_advance = f64::from(glyph_metrics.advance_width(gid).unwrap_or_default());
        // Center of this glyph along the arc, angle 0 at the top of the circle.
        let angle = (pen_x + glyph_advance / 2.0 - advance / 2.0) / radius;
        let transform = Affine::translate(center.to_vec2())
            * Affine::rotate(angle)
            * Affine::translate((-glyph_advance / 2.0, -radius));
        let single = [record::Glyph {
            id: gid.to_u32(),
            x: 0.0,
            y: 0.0,
        }];
        painter
            .glyphs(font, &brush)
            .transform(transform)
            .font_size(size)
            .draw(&style, single);
        pen_x += glyph_advance;
    }
}

/// Fake-3D extrusion: the run is restamped along a depth vector, darkest at the back.
fn draw_extrude(
    painter: &mut Painter<'_, dyn PaintSink + '_>,
    font: &FontData,
    text: &str,
    band: Band,
) {
    let glyphs = layout_glyphs(font, DISPLAY_SIZE, text);
    let style = Style::Fill(peniko::Fill::NonZero);
    let depth = 10;

    for i in (1..=depth).rev() {
        let t = f64::from(i) / f64::from(depth);
        let shade = Color::from_rgb8(
            channel(30.0 + 60.0 * (1.0 - t)),
            channel(20.0 + 40.0 * (1.0 - t)),
            channel(70.0 + 60.0 * (1.0 - t)),
        );
        let brush = Brush::Solid(shade);
        painter
            .glyphs(font, &brush)
            .transform(Affine::translate((
                MARGIN_X + f64::from(i) * 1.4,
                band.baseline + f64::from(i) * 1.4,
            )))
            .font_size(DISPLAY_SIZE)
            .draw(&style, &glyphs);
    }

    let brush = Brush::Gradient(
        peniko::Gradient::new_linear((0.0, -f64::from(DISPLAY_SIZE)), (0.0, 0.0)).with_stops([
            (0.0, Color::from_rgb8(255, 235, 170)),
            (1.0, Color::from_rgb8(255, 150, 90)),
        ]),
    );
    painter
        .glyphs(font, &brush)
        .transform(Affine::translate((MARGIN_X, band.baseline)))
        .font_size(DISPLAY_SIZE)
        .draw(&style, &glyphs);
}

/// Even-odd knockout: the text outline is punched out of a card in a single vector fill.
fn draw_knockout(
    painter: &mut Painter<'_, dyn PaintSink + '_>,
    font: &FontData,
    text: &str,
    band: Band,
) {
    // Gradient beneath, visible only through the punched-out letters.
    let card = Rect::new(24.0, band.top + 36.0, SHEET_WIDTH - 24.0, band.top + 138.0);
    let backdrop = Brush::Gradient(
        peniko::Gradient::new_linear((card.min_x(), 0.0), (card.max_x(), 0.0)).with_stops([
            (0.0, Color::from_rgb8(90, 220, 255)),
            (0.5, Color::from_rgb8(255, 120, 220)),
            (1.0, Color::from_rgb8(255, 220, 100)),
        ]),
    );
    painter.fill_rect(card, &backdrop);

    // One even-odd fill of card-rect + glyph outlines leaves letter-shaped holes.
    let mut punched = card.to_path(0.1);
    punched.extend(text_outline_path(
        font,
        DISPLAY_SIZE,
        text,
        Point::new(MARGIN_X, band.baseline),
    ));
    let card_brush = Brush::Solid(Color::from_rgb8(38, 42, 54));
    painter
        .fill(&punched, &card_brush)
        .fill_rule(peniko::Fill::EvenOdd)
        .draw();
}
