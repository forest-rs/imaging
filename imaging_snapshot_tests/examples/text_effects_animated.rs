// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Animated vector text effects demo, inspired by [TerminalTextEffects].
//!
//! [TerminalTextEffects] animates text by treating every character as a particle with a motion
//! path, easing, staggered timing, and color gradients — but it is bound to terminal cells. This
//! example applies the same animation model to real vector glyphs through the backend-agnostic
//! [`imaging`] API: glyphs get sub-pixel motion, true rotation and squash-and-stretch, gradient
//! brushes, blur/glow group filters, and outline-path clips.
//!
//! Each effect is a pure function of time `t in [0, 1]`, recorded as an [`imaging`] scene per
//! frame and rasterized with the `vello_cpu` backend into a looping APNG (plus a static preview
//! frame).
//!
//! Run with:
//!
//! ```sh
//! cargo run --release -p imaging_snapshot_tests --features vello_cpu \
//!     --example text_effects_animated -- "Imaging"
//! ```
//!
//! Output goes to `target/text_effects_animated/` (override with a second argument).
//!
//! [TerminalTextEffects]: https://github.com/ChrisBuilds/terminaltexteffects

use std::io::BufWriter;
use std::sync::Arc;

use imaging::{Filter, GroupRef, PaintSink, Painter, record};
use kurbo::{Affine, BezPath, Circle, Point, Rect, Shape as _, Vec2};
use peniko::{Blob, Brush, Color, FontData, Style};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{FontRef, GlyphId, MetadataProvider};

const ROBOTO_FONT_BYTES: &[u8] = include_bytes!("../src/assets/roboto/Roboto-Regular.ttf");

const WIDTH: u16 = 900;
const HEIGHT: u16 = 150;
const BASELINE: f64 = 104.0;
const MARGIN_X: f64 = 40.0;
const DISPLAY_SIZE: f32 = 64.0;
const FRAMES: u32 = 84;
const FPS: u16 = 24;

fn main() {
    let mut args = std::env::args().skip(1);
    let text = args.next().unwrap_or_else(|| "Imaging".to_string());
    let out_dir = args
        .next()
        .unwrap_or_else(|| "target/text_effects_animated".to_string());
    std::fs::create_dir_all(&out_dir).expect("create output directory");

    let font = FontData::new(Blob::new(Arc::new(ROBOTO_FONT_BYTES)), 0);

    let effects: Vec<(&str, EffectFn)> = vec![
        ("scatter_assemble", draw_scatter_assemble),
        ("pour_bounce", draw_pour_bounce),
        ("decrypt", draw_decrypt),
        ("beam_sweep", draw_beam_sweep),
        ("burn_reveal", draw_burn_reveal),
    ];

    let mut renderer = imaging_vello_cpu::VelloCpuRenderer::new(WIDTH, HEIGHT);

    for (name, effect) in effects {
        let mut frames = Vec::with_capacity(FRAMES as usize);
        for frame in 0..FRAMES {
            let t = f64::from(frame) / f64::from(FRAMES - 1);
            let mut scene = record::Scene::new();
            {
                let mut base_painter = Painter::new(&mut scene);
                let mut painter = base_painter.as_dyn();
                painter.fill_rect(
                    Rect::new(0.0, 0.0, f64::from(WIDTH), f64::from(HEIGHT)),
                    Color::from_rgb8(24, 26, 32),
                );
                effect(&mut painter, &font, &text, t, frame);
            }
            scene.validate().expect("effect scene should validate");
            let image = renderer
                .render_scene(&scene, WIDTH, HEIGHT)
                .expect("render effect frame");
            frames.push(image.data);
        }

        let apng_path = format!("{out_dir}/{name}.png");
        write_apng(&apng_path, &frames);
        let preview = &frames[(FRAMES as usize * 3) / 5];
        let preview_png = kompari::image_to_png(
            &kompari::image::ImageBuffer::from_raw(
                u32::from(WIDTH),
                u32::from(HEIGHT),
                preview.clone(),
            )
            .expect("RGBA buffer size should match image dimensions"),
            kompari::SizeOptimizationLevel::Fast,
        );
        std::fs::write(format!("{out_dir}/{name}_preview.png"), preview_png)
            .expect("write preview png");
        println!("wrote {apng_path} ({FRAMES} frames @ {FPS} fps)");
    }
}

type EffectFn = fn(&mut Painter<'_, dyn PaintSink + '_>, &FontData, &str, f64, u32);

fn write_apng(path: &str, frames: &[Vec<u8>]) {
    let file = std::fs::File::create(path).expect("create apng file");
    let mut encoder = png::Encoder::new(BufWriter::new(file), u32::from(WIDTH), u32::from(HEIGHT));
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    #[allow(
        clippy::cast_possible_truncation,
        reason = "Frame count is a small compile-time constant."
    )]
    encoder
        .set_animated(frames.len() as u32, 0)
        .expect("enable apng");
    encoder.set_frame_delay(1, FPS).expect("set frame delay");
    let mut writer = encoder.write_header().expect("write apng header");
    for frame in frames {
        writer.write_image_data(frame).expect("write apng frame");
    }
    writer.finish().expect("finish apng");
}

// --- deterministic pseudo-randomness -------------------------------------------------------

/// Tiny deterministic xorshift PRNG so every run produces identical animations.
struct Rng(u64);

impl Rng {
    fn for_glyph(seed: u64, index: usize) -> Self {
        Self((seed ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)) | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Uniform value in `[0, 1)`.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "Intentional reduction of PRNG output to mantissa bits."
    )]
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1_u64 << 53) as f64
    }

    fn range(&mut self, min: f64, max: f64) -> f64 {
        min + (max - min) * self.unit()
    }
}

// --- easing ---------------------------------------------------------------------------------

fn ease_out_cubic(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}

fn ease_in_out_cubic(t: f64) -> f64 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

fn ease_out_bounce(t: f64) -> f64 {
    const N1: f64 = 7.5625;
    const D1: f64 = 2.75;
    if t < 1.0 / D1 {
        N1 * t * t
    } else if t < 2.0 / D1 {
        let t = t - 1.5 / D1;
        N1 * t * t + 0.75
    } else if t < 2.5 / D1 {
        let t = t - 2.25 / D1;
        N1 * t * t + 0.9375
    } else {
        let t = t - 2.625 / D1;
        N1 * t * t + 0.984375
    }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "Inputs are clamped to the u8 channel range first."
)]
fn channel(value: f64) -> u8 {
    value.clamp(0.0, 255.0) as u8
}

/// Clamp a unit alpha value into `f32`.
#[allow(
    clippy::cast_possible_truncation,
    reason = "Alpha values are clamped to the unit range first."
)]
fn alpha32(value: f64) -> f32 {
    value.clamp(0.0, 1.0) as f32
}

fn lerp_color(a: Color, b: Color, t: f64) -> Color {
    let [ar, ag, ab, aa] = a.to_rgba8().to_u8_array();
    let [br, bg, bb, ba] = b.to_rgba8().to_u8_array();
    Color::from_rgba8(
        channel(lerp(f64::from(ar), f64::from(br), t)),
        channel(lerp(f64::from(ag), f64::from(bg), t)),
        channel(lerp(f64::from(ab), f64::from(bb), t)),
        channel(lerp(f64::from(aa), f64::from(ba), t)),
    )
}

// --- shared text plumbing -------------------------------------------------------------------

/// Map characters to positioned glyphs with metrics-based advances.
///
/// Simple advance-based positioning; a real application would use Parley for full shaping and
/// feed the same `(glyph id, x, y)` stream into everything below.
fn layout_glyphs(font: &FontData, font_size: f32, text: &str) -> Vec<(record::Glyph, f64)> {
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
            let advance = glyph_metrics.advance_width(gid).unwrap_or_default();
            pen_x += advance;
            (glyph, f64::from(advance))
        })
        .collect()
}

/// Collects skrifa outline callbacks into a y-down [`BezPath`].
struct BezPathPen {
    path: BezPath,
    offset: Vec2,
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

impl BezPathPen {
    fn point(&self, x: f32, y: f32) -> Point {
        // Font outlines are y-up; the canvas is y-down.
        Point::new(self.offset.x + f64::from(x), self.offset.y - f64::from(y))
    }
}

/// Extract the whole string as one vector outline path, positioned at `origin` (baseline-left).
fn text_outline_path(font: &FontData, font_size: f32, text: &str, origin: Point) -> BezPath {
    let font_ref = FontRef::from_index(font.data.as_ref(), font.index).expect("load demo font");
    let outlines = font_ref.outline_glyphs();

    let mut pen = BezPathPen {
        path: BezPath::new(),
        offset: Vec2::ZERO,
    };
    for (glyph, _advance) in layout_glyphs(font, font_size, text) {
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

fn draw_single_glyph(
    painter: &mut Painter<'_, dyn PaintSink + '_>,
    font: &FontData,
    glyph_id: u32,
    transform: Affine,
    color: Color,
) {
    let brush = Brush::Solid(color);
    let single = [record::Glyph {
        id: glyph_id,
        x: 0.0,
        y: 0.0,
    }];
    painter
        .glyphs(font, &brush)
        .transform(transform)
        .font_size(DISPLAY_SIZE)
        .draw(&Style::Fill(peniko::Fill::NonZero), single);
}

// --- effects --------------------------------------------------------------------------------

/// TTE "Scattered"/"Unstable": glyphs start strewn across the canvas with random rotation and
/// scale, then fly home on eased paths with motion-blur ghosts.
fn draw_scatter_assemble(
    painter: &mut Painter<'_, dyn PaintSink + '_>,
    font: &FontData,
    text: &str,
    t: f64,
    _frame: u32,
) {
    let glyphs = layout_glyphs(font, DISPLAY_SIZE, text);
    let count = glyphs.len().max(1);
    let stagger = 0.4 / count as f64;
    let duration = 0.55;

    for (index, (glyph, advance)) in glyphs.iter().enumerate() {
        let mut rng = Rng::for_glyph(0xC0FF_EE00, index);
        let start = Point::new(
            rng.range(0.0, f64::from(WIDTH)),
            rng.range(-30.0, f64::from(HEIGHT) + 30.0),
        );
        let start_angle = rng.range(-2.4, 2.4);
        let start_scale = rng.range(0.3, 2.0);
        let home = Point::new(MARGIN_X + f64::from(glyph.x), BASELINE);

        let local = ((t - index as f64 * stagger) / duration).clamp(0.0, 1.0);

        // Motion-blur ghosts trail the eased path at earlier parameter values.
        for ghost in (0..3).rev() {
            let ghost_local = (local - 0.035 * f64::from(ghost)).clamp(0.0, 1.0);
            let eased = ease_out_cubic(ghost_local);
            let pos = Point::new(lerp(start.x, home.x, eased), lerp(start.y, home.y, eased));
            let angle = lerp(start_angle, 0.0, eased);
            let scale = lerp(start_scale, 1.0, eased);
            let alpha = if ghost == 0 {
                0.25 + 0.75 * eased
            } else {
                (1.0 - eased) * 0.16
            };
            let color = lerp_color(
                Color::from_rgb8(120, 200, 255),
                Color::from_rgb8(240, 244, 255),
                eased,
            )
            .with_alpha(alpha32(alpha));
            let center = advance / 2.0;
            let transform = Affine::translate(pos.to_vec2())
                * Affine::translate((center, 0.0))
                * Affine::rotate(angle)
                * Affine::scale(scale)
                * Affine::translate((-center, 0.0));
            draw_single_glyph(painter, font, glyph.id, transform, color);
        }
    }
}

/// TTE "Pour"/"BouncyBalls": glyphs drop in left to right and bounce on the baseline, with
/// velocity-derived squash-and-stretch that terminal cells cannot express.
fn draw_pour_bounce(
    painter: &mut Painter<'_, dyn PaintSink + '_>,
    font: &FontData,
    text: &str,
    t: f64,
    _frame: u32,
) {
    let glyphs = layout_glyphs(font, DISPLAY_SIZE, text);
    let count = glyphs.len().max(1);
    let stagger = 0.5 / count as f64;
    let duration = 0.5;
    let drop_height = BASELINE + 70.0;

    for (index, (glyph, advance)) in glyphs.iter().enumerate() {
        let local = ((t - index as f64 * stagger) / duration).clamp(0.0, 1.0);
        if local <= 0.0 {
            continue;
        }
        let eased = ease_out_bounce(local);
        let y = BASELINE - drop_height * (1.0 - eased);

        // Estimate vertical speed for squash & stretch, anchored at the baseline.
        let dt = 1.0 / f64::from(FRAMES);
        let prev = ease_out_bounce((local - dt / duration).clamp(0.0, 1.0));
        let speed = (eased - prev).abs() * drop_height;
        let stretch = 1.0 + (speed * 0.045).min(0.35);

        let hue = index as f64 / count as f64;
        let color = lerp_color(
            Color::from_rgb8(255, 170, 90),
            Color::from_rgb8(140, 190, 255),
            hue,
        )
        .with_alpha(alpha32(local * 4.0));

        let center = advance / 2.0;
        let transform = Affine::translate((MARGIN_X + f64::from(glyph.x), y))
            * Affine::translate((center, 0.0))
            * Affine::scale_non_uniform(1.0 / stretch, stretch)
            * Affine::translate((-center, 0.0));
        draw_single_glyph(painter, font, glyph.id, transform, color);
    }
}

/// TTE "Decrypt": slots cycle through cipher glyphs in terminal green, then resolve
/// left to right and fade to the final color.
fn draw_decrypt(
    painter: &mut Painter<'_, dyn PaintSink + '_>,
    font: &FontData,
    text: &str,
    t: f64,
    frame: u32,
) {
    let font_ref = FontRef::from_index(font.data.as_ref(), font.index).expect("load demo font");
    let charmap = font_ref.charmap();
    let cipher_pool: Vec<u32> = "#$%&@*+=?/\\<>0123456789ABCDEF"
        .chars()
        .map(|ch| charmap.map(ch).unwrap_or_default().to_u32())
        .collect();

    let glyphs = layout_glyphs(font, DISPLAY_SIZE, text);
    let count = glyphs.len().max(1);

    for (index, (glyph, _advance)) in glyphs.iter().enumerate() {
        let resolve_at = 0.15 + 0.6 * (index as f64 + 0.5) / count as f64;
        let transform = Affine::translate((MARGIN_X + f64::from(glyph.x), BASELINE));

        if t < resolve_at {
            // Unresolved: flicker through cipher glyphs every couple of frames.
            let mut rng = Rng::for_glyph(0xDEC0_DE00 ^ u64::from(frame / 2), index);
            let cipher_index = usize::try_from(rng.next_u64() % cipher_pool.len() as u64)
                .expect("index fits in usize");
            let cipher = cipher_pool[cipher_index];
            let flicker = 0.45 + 0.4 * rng.unit();
            let color = Color::from_rgb8(70, 210, 90).with_alpha(alpha32(flicker));
            draw_single_glyph(painter, font, cipher, transform, color);
        } else {
            // Resolved: flash bright green, then settle to the final color.
            let settle = ((t - resolve_at) / 0.18).clamp(0.0, 1.0);
            let color = lerp_color(
                Color::from_rgb8(170, 255, 170),
                Color::from_rgb8(235, 240, 250),
                ease_out_cubic(settle),
            );
            draw_single_glyph(painter, font, glyph.id, transform, color);
        }
    }
}

/// TTE "Beams"/"Sweep": a glowing beam sweeps across; glyphs it has passed stay lit, glyphs
/// near it flare white-hot.
fn draw_beam_sweep(
    painter: &mut Painter<'_, dyn PaintSink + '_>,
    font: &FontData,
    text: &str,
    t: f64,
    _frame: u32,
) {
    let glyphs = layout_glyphs(font, DISPLAY_SIZE, text);
    let sweep = ease_in_out_cubic(t);
    let beam_x = lerp(-60.0, f64::from(WIDTH) + 60.0, sweep);

    for (glyph, advance) in &glyphs {
        let center_x = MARGIN_X + f64::from(glyph.x) + advance / 2.0;
        let distance = beam_x - center_x;
        let transform = Affine::translate((MARGIN_X + f64::from(glyph.x), BASELINE));

        let color = if distance < 0.0 {
            // Not yet reached: dim placeholder.
            Color::from_rgba8(70, 78, 96, 255)
        } else if distance < 70.0 {
            // Flare near the beam.
            let heat = 1.0 - distance / 70.0;
            lerp_color(
                Color::from_rgb8(255, 200, 120),
                Color::from_rgb8(255, 255, 255),
                heat,
            )
        } else {
            Color::from_rgb8(255, 200, 120)
        };
        draw_single_glyph(painter, font, glyph.id, transform, color);
    }

    // The beam itself: a blurred gradient bar streaking down the canvas.
    let beam_filters = [Filter::Blur {
        std_deviation_x: 4.0,
        std_deviation_y: 0.0,
    }];
    let beam_brush = Brush::Gradient(
        peniko::Gradient::new_linear((beam_x - 26.0, 0.0), (beam_x + 26.0, 0.0)).with_stops([
            (0.0, Color::from_rgba8(255, 240, 200, 0)),
            (0.5, Color::from_rgba8(255, 250, 235, 210)),
            (1.0, Color::from_rgba8(255, 240, 200, 0)),
        ]),
    );
    painter.with_group(GroupRef::new().with_filters(&beam_filters), |painter| {
        painter.fill_rect(
            Rect::new(beam_x - 26.0, 8.0, beam_x + 26.0, f64::from(HEIGHT) - 8.0),
            &beam_brush,
        );
    });
}

/// TTE "Burn"/"LaserEtch": the string's outline path is revealed left to right behind an
/// ember edge with glow and rising sparks — all clips on real vector outlines.
fn draw_burn_reveal(
    painter: &mut Painter<'_, dyn PaintSink + '_>,
    font: &FontData,
    text: &str,
    t: f64,
    _frame: u32,
) {
    let outline = text_outline_path(font, DISPLAY_SIZE, text, Point::new(MARGIN_X, BASELINE));
    let bounds = outline.bounding_box();
    let sweep = ease_in_out_cubic(t);
    let reveal_x = lerp(bounds.min_x() - 12.0, bounds.max_x() + 26.0, sweep);

    // Revealed portion: the outline path clipped to everything left of the burn front.
    let revealed = Rect::new(
        bounds.min_x() - 8.0,
        bounds.min_y() - 8.0,
        reveal_x,
        bounds.max_y() + 8.0,
    );
    let fill_brush = Brush::Gradient(
        peniko::Gradient::new_linear((bounds.min_x(), 0.0), (bounds.max_x(), 0.0)).with_stops([
            (0.0, Color::from_rgb8(255, 120, 90)),
            (1.0, Color::from_rgb8(255, 210, 120)),
        ]),
    );
    painter.with_fill_clip(revealed, |painter| {
        painter.fill(&outline, &fill_brush).draw();
    });

    // Ember edge: a thin strip of the outline at the front, blurred into a glow.
    if t < 0.995 {
        let edge = Rect::new(
            reveal_x - 7.0,
            bounds.min_y() - 10.0,
            reveal_x + 3.0,
            bounds.max_y() + 10.0,
        );
        let ember_filters = [Filter::Blur {
            std_deviation_x: 2.0,
            std_deviation_y: 2.0,
        }];
        let ember_brush = Brush::Solid(Color::from_rgb8(255, 235, 170));
        painter.with_group(GroupRef::new().with_filters(&ember_filters), |painter| {
            painter.with_fill_clip(edge, |painter| {
                painter.fill(&outline, &ember_brush).draw();
            });
        });

        // Sparks rising off the burn front.
        let mut rng = Rng::for_glyph(0xF1AE_0000, _frame as usize);
        for _ in 0..6 {
            let age = rng.unit();
            let spark = Circle::new(
                Point::new(
                    reveal_x + rng.range(-10.0, 6.0),
                    BASELINE - 20.0 - age * rng.range(24.0, 60.0),
                ),
                1.4 + (1.0 - age) * 1.3,
            );
            let color = lerp_color(
                Color::from_rgb8(255, 220, 150),
                Color::from_rgb8(255, 110, 70),
                age,
            )
            .with_alpha(alpha32(1.0 - age));
            painter
                .fill(spark.to_path(0.1), &Brush::Solid(color))
                .draw();
        }
    }
}
