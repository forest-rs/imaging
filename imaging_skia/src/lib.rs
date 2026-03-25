// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Skia backend for `imaging`.
//!
//! This crate provides a CPU raster renderer that consumes `imaging::record::Scene` or native
//! Skia draw targets and produces an RGBA8 image buffer using Skia.
//!
//! # Render A Recorded Scene
//!
//! Record commands into [`imaging::record::Scene`], then hand the scene to [`SkiaRenderer`].
//!
//! ```no_run
//! use imaging::{Painter, record};
//! use imaging_skia::SkiaRenderer;
//! use kurbo::Rect;
//! use peniko::{Brush, Color};
//!
//! fn main() -> Result<(), imaging_skia::Error> {
//!     let paint = Brush::Solid(Color::from_rgb8(0x2a, 0x6f, 0xdb));
//!     let mut scene = record::Scene::new();
//!
//!     {
//!         let mut painter = Painter::new(&mut scene);
//!         painter.fill_rect(Rect::new(0.0, 0.0, 128.0, 128.0), &paint);
//!     }
//!
//!     let mut renderer = SkiaRenderer::new(128, 128);
//!     let rgba = renderer.render_scene_rgba8(&scene)?;
//!     assert_eq!(rgba.len(), 128 * 128 * 4);
//!     Ok(())
//! }
//! ```
//!
//! # Draw Into An Existing `Canvas`
//!
//! If you already have a Skia canvas, wrap it with [`SkCanvasSink`] and stream commands directly.
//!
//! ```no_run
//! use imaging::Painter;
//! use imaging_skia::SkCanvasSink;
//! use kurbo::Rect;
//! use peniko::{Brush, Color};
//! use skia_safe::surfaces;
//!
//! fn main() -> Result<(), imaging_skia::Error> {
//!     let paint = Brush::Solid(Color::from_rgb8(0x1d, 0x4e, 0x89));
//!     let mut surface = surfaces::raster_n32_premul((128, 128)).unwrap();
//!
//!     {
//!         let mut sink = SkCanvasSink::new(surface.canvas());
//!         let mut painter = Painter::new(&mut sink);
//!         painter.fill_rect(Rect::new(0.0, 0.0, 128.0, 128.0), &paint);
//!         sink.finish()?;
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! # Record A `SkPicture`
//!
//! Use [`SkPictureRecorderSink`] when you want Skia's native retained recording format.
//!
//! ```no_run
//! use imaging::Painter;
//! use imaging_skia::SkPictureRecorderSink;
//! use kurbo::Rect;
//! use peniko::{Brush, Color};
//!
//! fn main() -> Result<(), imaging_skia::Error> {
//!     let paint = Brush::Solid(Color::from_rgb8(0x7c, 0x3a, 0xed));
//!     let mut sink = SkPictureRecorderSink::new(Rect::new(0.0, 0.0, 128.0, 128.0));
//!
//!     {
//!         let mut painter = Painter::new(&mut sink);
//!         painter.fill_rect(Rect::new(16.0, 16.0, 112.0, 112.0), &paint);
//!     }
//!
//!     let picture = sink.finish_picture()?;
//!     assert_eq!(picture.cull_rect().right, 128.0);
//!     Ok(())
//! }
//! ```
//!
//! # Render A Native `SkPicture`
//!
//! If you already have a recorded picture, hand it directly to [`SkiaRenderer`].
//!
//! ```no_run
//! use imaging::Painter;
//! use imaging_skia::{SkPictureRecorderSink, SkiaRenderer};
//! use kurbo::Rect;
//! use peniko::{Brush, Color};
//!
//! fn main() -> Result<(), imaging_skia::Error> {
//!     let paint = Brush::Solid(Color::from_rgb8(0xd9, 0x77, 0x06));
//!     let mut sink = SkPictureRecorderSink::new(Rect::new(0.0, 0.0, 128.0, 128.0));
//!
//!     {
//!         let mut painter = Painter::new(&mut sink);
//!         painter.fill_rect(Rect::new(16.0, 16.0, 112.0, 112.0), &paint);
//!     }
//!
//!     let picture = sink.finish_picture()?;
//!     let mut renderer = SkiaRenderer::new(128, 128);
//!     let rgba = renderer.render_picture_rgba8(&picture)?;
//!     assert_eq!(rgba.len(), 128 * 128 * 4);
//!     Ok(())
//! }
//! ```

#![deny(unsafe_code)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod sinks;

use imaging::{
    Filter, GeometryRef, GlyphRunRef,
    record::{Scene, ValidateError, replay},
};
use kurbo::{Affine, Shape as _};
use peniko::color::{ColorSpaceTag, HueDirection};
use peniko::{BrushRef, ImageAlphaType, ImageFormat, ImageQuality, InterpolationAlphaSpace};
use skia_safe as sk;
use std::{cell::RefCell, rc::Rc};

use sinks::MaskCache;
pub use sinks::{SkCanvasSink, SkPictureRecorderSink};

/// Errors that can occur when rendering via Skia.
#[derive(Debug)]
pub enum Error {
    /// The scene is invalid (unbalanced stacks).
    InvalidScene(ValidateError),
    /// An image brush was encountered; this backend does not support it.
    UnsupportedImageBrush,
    /// A filter configuration could not be translated.
    UnsupportedFilter,
    /// A glyph run used variable-font coordinates unsupported by this backend.
    UnsupportedGlyphVariations,
    /// A glyph run used a per-glyph transform unsupported by this backend.
    UnsupportedGlyphTransform,
    /// Font bytes could not be loaded by Skia.
    InvalidFontData,
    /// A glyph identifier could not be represented by Skia's glyph type.
    InvalidGlyphId,
    /// An internal invariant was violated.
    Internal(&'static str),
}

/// Renderer that executes `imaging` commands using a Skia raster surface.
#[derive(Debug)]
pub struct SkiaRenderer {
    surface: sk::Surface,
    width: i32,
    height: i32,
    tolerance: f64,
    mask_cache: Rc<RefCell<MaskCache>>,
}

impl SkiaRenderer {
    /// Create a renderer for a fixed-size target.
    pub fn new(width: u16, height: u16) -> Self {
        let width = i32::from(width);
        let height = i32::from(height);
        // Use an explicit RGBA8888 premultiplied raster surface. Many blend modes are defined in
        // premultiplied space, and it also matches Skia's typical raster backend behavior.
        //
        // Note: we still export unpremultiplied RGBA8 from `read_rgba8()`.
        let info = sk::ImageInfo::new(
            (width, height),
            sk::ColorType::RGBA8888,
            sk::AlphaType::Premul,
            None,
        );
        let surface = sk::surfaces::raster(&info, None, None)
            .expect("create skia raster RGBA8888/premul surface");
        Self {
            surface,
            width,
            height,
            tolerance: 0.1,
            mask_cache: Rc::new(RefCell::new(MaskCache::default())),
        }
    }

    /// Set the tolerance used when converting shapes to paths.
    pub fn set_tolerance(&mut self, tolerance: f64) {
        if self.tolerance != tolerance {
            self.mask_cache.borrow_mut().clear();
        }
        self.tolerance = tolerance;
    }

    /// Drop any realized mask artifacts cached by the renderer.
    ///
    /// The cache is renderer-scoped so unchanged masked subscenes can be reused across renders.
    /// Call this if you need to release memory aggressively or after changing assumptions that
    /// affect mask realization outside the recorded scene itself.
    pub fn clear_cached_masks(&mut self) {
        self.mask_cache.borrow_mut().clear();
    }

    fn reset(&mut self) {
        let canvas = self.surface.canvas();
        canvas.restore_to_count(1);
        canvas.reset_matrix();
        canvas.clear(sk::Color::TRANSPARENT);
    }

    /// Render a recorded scene and return an RGBA8 buffer (unpremultiplied).
    pub fn render_scene_rgba8(&mut self, scene: &Scene) -> Result<Vec<u8>, Error> {
        scene.validate().map_err(Error::InvalidScene)?;
        self.reset();
        let mut sink =
            SkCanvasSink::new_with_mask_cache(self.surface.canvas(), Rc::clone(&self.mask_cache));
        sink.set_tolerance(self.tolerance);
        replay(scene, &mut sink);
        sink.finish()?;
        self.read_rgba8()
    }

    /// Render a native [`skia_safe::Picture`] and return an RGBA8 buffer (unpremultiplied).
    pub fn render_picture_rgba8(&mut self, picture: &sk::Picture) -> Result<Vec<u8>, Error> {
        self.reset();
        self.surface.canvas().draw_picture(picture, None, None);
        self.read_rgba8()
    }

    fn read_rgba8(&mut self) -> Result<Vec<u8>, Error> {
        let image = self.surface.image_snapshot();
        let info = sk::ImageInfo::new(
            (self.width, self.height),
            sk::ColorType::RGBA8888,
            sk::AlphaType::Unpremul,
            None,
        );
        let mut bytes = vec![0_u8; (self.width as usize) * (self.height as usize) * 4];
        let ok = image.read_pixels(
            &info,
            bytes.as_mut_slice(),
            (4 * self.width) as usize,
            (0, 0),
            sk::image::CachingHint::Disallow,
        );
        if !ok {
            return Err(Error::Internal("read_pixels failed"));
        }
        Ok(bytes)
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "Skia APIs consume f32; truncation from f64 geometry is acceptable"
)]
fn f64_to_f32(v: f64) -> f32 {
    v as f32
}

fn rad_to_deg(rad: f32) -> f32 {
    rad * (180.0 / core::f32::consts::PI)
}

fn affine_to_matrix(xf: Affine) -> sk::Matrix {
    let a = xf.as_coeffs();
    sk::Matrix::new_all(
        f64_to_f32(a[0]),
        f64_to_f32(a[2]),
        f64_to_f32(a[4]),
        f64_to_f32(a[1]),
        f64_to_f32(a[3]),
        f64_to_f32(a[5]),
        0.0,
        0.0,
        1.0,
    )
}

fn skia_font_from_glyph_run(glyph_run: &GlyphRunRef<'_>) -> Option<sk::Font> {
    let typeface = sk::FontMgr::default()
        .new_from_data(glyph_run.font.data.as_ref(), glyph_run.font.index as usize)?;

    let mut font = sk::Font::from_typeface(typeface, glyph_run.font_size);
    font.set_hinting(if glyph_run.hint {
        sk::FontHinting::Slight
    } else {
        sk::FontHinting::None
    });

    if let Some(transform) = glyph_run.glyph_transform {
        let [a, b, c, d, e, f] = transform.as_coeffs();
        if b != 0.0 || e != 0.0 || f != 0.0 || d <= 0.0 {
            return None;
        }
        let y_scale = f64_to_f32(d);
        font.set_size(f64_to_f32(glyph_run.font_size as f64 * d));
        font.set_scale_x(f64_to_f32(a / d));
        font.set_skew_x(f64_to_f32(c / d));
        if y_scale <= 0.0 {
            return None;
        }
    }

    Some(font)
}

fn sk_path_fill_type_from_fill_rule(rule: peniko::Fill) -> sk::PathFillType {
    match rule {
        peniko::Fill::NonZero => sk::PathFillType::Winding,
        peniko::Fill::EvenOdd => sk::PathFillType::EvenOdd,
    }
}

fn path_with_fill_rule(path: &sk::Path, rule: peniko::Fill) -> sk::Path {
    let fill = sk_path_fill_type_from_fill_rule(rule);
    if path.fill_type() == fill {
        path.clone()
    } else {
        path.with_fill_type(fill)
    }
}

fn geometry_to_bez_path(geom: GeometryRef<'_>, tolerance: f64) -> Option<kurbo::BezPath> {
    Some(match geom {
        GeometryRef::Rect(r) => r.to_path(tolerance),
        GeometryRef::RoundedRect(rr) => rr.to_path(tolerance),
        GeometryRef::Path(p) => p.clone(),
        GeometryRef::OwnedPath(p) => p,
    })
}

fn geometry_to_sk_path(geom: GeometryRef<'_>, tolerance: f64) -> Option<sk::Path> {
    let bez = geometry_to_bez_path(geom, tolerance)?;
    bez_to_sk_path(&bez)
}

fn bez_to_sk_path(bez: &kurbo::BezPath) -> Option<sk::Path> {
    let mut path = sk::Path::new();
    for el in bez.elements() {
        match el {
            kurbo::PathEl::MoveTo(p) => {
                path.move_to((f64_to_f32(p.x), f64_to_f32(p.y)));
            }
            kurbo::PathEl::LineTo(p) => {
                path.line_to((f64_to_f32(p.x), f64_to_f32(p.y)));
            }
            kurbo::PathEl::QuadTo(p1, p2) => {
                path.quad_to(
                    (f64_to_f32(p1.x), f64_to_f32(p1.y)),
                    (f64_to_f32(p2.x), f64_to_f32(p2.y)),
                );
            }
            kurbo::PathEl::CurveTo(p1, p2, p3) => {
                path.cubic_to(
                    (f64_to_f32(p1.x), f64_to_f32(p1.y)),
                    (f64_to_f32(p2.x), f64_to_f32(p2.y)),
                    (f64_to_f32(p3.x), f64_to_f32(p3.y)),
                );
            }
            kurbo::PathEl::ClosePath => {
                path.close();
            }
        }
    }
    Some(path)
}

fn tile_mode_from_extend(extend: peniko::Extend) -> sk::TileMode {
    match extend {
        peniko::Extend::Pad => sk::TileMode::Clamp,
        peniko::Extend::Repeat => sk::TileMode::Repeat,
        peniko::Extend::Reflect => sk::TileMode::Mirror,
    }
}

fn gradient_shader_cs_from_cs_tag(
    color_space: ColorSpaceTag,
) -> sk::gradient_shader::interpolation::ColorSpace {
    use sk::gradient_shader::interpolation::ColorSpace as SkCs;

    match color_space {
        ColorSpaceTag::Srgb => SkCs::SRGB,
        ColorSpaceTag::LinearSrgb => SkCs::SRGBLinear,
        ColorSpaceTag::Lab => SkCs::Lab,
        ColorSpaceTag::Lch => SkCs::LCH,
        ColorSpaceTag::Hsl => SkCs::HSL,
        ColorSpaceTag::Hwb => SkCs::HWB,
        ColorSpaceTag::Oklab => SkCs::OKLab,
        ColorSpaceTag::Oklch => SkCs::OKLCH,
        ColorSpaceTag::DisplayP3 => SkCs::DisplayP3,
        ColorSpaceTag::A98Rgb => SkCs::A98RGB,
        ColorSpaceTag::ProphotoRgb => SkCs::ProphotoRGB,
        ColorSpaceTag::Rec2020 => SkCs::Rec2020,
        _ => SkCs::SRGB,
    }
}

fn gradient_shader_hue_method_from_hue_direction(
    direction: HueDirection,
) -> sk::gradient_shader::interpolation::HueMethod {
    use sk::gradient_shader::interpolation::HueMethod as SkHue;

    match direction {
        HueDirection::Shorter => SkHue::Shorter,
        HueDirection::Longer => SkHue::Longer,
        HueDirection::Increasing => SkHue::Increasing,
        HueDirection::Decreasing => SkHue::Decreasing,
        _ => SkHue::Shorter,
    }
}

fn color_to_sk_color(color: peniko::Color) -> sk::Color {
    let rgba = color.to_rgba8();
    sk::Color::from_argb(rgba.a, rgba.r, rgba.g, rgba.b)
}

fn color_to_sk_color4f(color: peniko::Color) -> sk::Color4f {
    let comps = color.components;
    sk::Color4f::new(comps[0], comps[1], comps[2], comps[3])
}

fn brush_to_paint(brush: BrushRef<'_>, opacity: f32, paint_xf: Affine) -> Option<sk::Paint> {
    let mut paint = sk::Paint::default();
    paint.set_anti_alias(true);
    let alpha_scale = opacity.clamp(0.0, 1.0);

    match brush {
        BrushRef::Solid(color) => {
            // Use float color to avoid quantizing alpha (important for Porter-Duff ops like XOR).
            let comps = color.components;
            let c = sk::Color4f::new(comps[0], comps[1], comps[2], comps[3] * alpha_scale);
            paint.set_color4f(c, None);
        }
        BrushRef::Gradient(grad) => {
            let stops = grad.stops.as_ref();
            if stops.is_empty() {
                paint.set_color(sk::Color::TRANSPARENT);
                return Some(paint);
            }

            let mut colors: Vec<sk::Color4f> = Vec::with_capacity(stops.len());
            let mut pos: Vec<f32> = Vec::with_capacity(stops.len());

            for s in stops {
                let comps = s.color.components;
                let a = comps[3] * alpha_scale;
                colors.push(sk::Color4f::new(comps[0], comps[1], comps[2], a));
                pos.push(s.offset.clamp(0.0, 1.0));
            }

            let tile_mode = tile_mode_from_extend(grad.extend);
            let local = affine_to_matrix(paint_xf);

            let interpolation = sk::gradient_shader::Interpolation {
                color_space: gradient_shader_cs_from_cs_tag(grad.interpolation_cs),
                in_premul: match grad.interpolation_alpha_space {
                    InterpolationAlphaSpace::Premultiplied => {
                        sk::gradient_shader::interpolation::InPremul::Yes
                    }
                    InterpolationAlphaSpace::Unpremultiplied => {
                        sk::gradient_shader::interpolation::InPremul::No
                    }
                },
                hue_method: gradient_shader_hue_method_from_hue_direction(grad.hue_direction),
            };

            match &grad.kind {
                peniko::GradientKind::Linear(line) => {
                    let p0 = sk::Point::new(f64_to_f32(line.start.x), f64_to_f32(line.start.y));
                    let p1 = sk::Point::new(f64_to_f32(line.end.x), f64_to_f32(line.end.y));
                    if let Some(shader) = sk::Shader::linear_gradient_with_interpolation(
                        (p0, p1),
                        (&colors[..], None),
                        &pos[..],
                        tile_mode,
                        interpolation,
                        Some(&local),
                    ) {
                        paint.set_shader(shader);
                    }
                }
                peniko::GradientKind::Radial(rad) => {
                    let start_center = sk::Point::new(
                        f64_to_f32(rad.start_center.x),
                        f64_to_f32(rad.start_center.y),
                    );
                    let start_radius = rad.start_radius;
                    let end_center =
                        sk::Point::new(f64_to_f32(rad.end_center.x), f64_to_f32(rad.end_center.y));
                    let end_radius = rad.end_radius;

                    if let Some(shader) = sk::Shader::two_point_conical_gradient_with_interpolation(
                        (start_center, start_radius),
                        (end_center, end_radius),
                        (&colors[..], None),
                        &pos[..],
                        tile_mode,
                        interpolation,
                        Some(&local),
                    ) {
                        paint.set_shader(shader);
                    }
                }
                peniko::GradientKind::Sweep(sweep) => {
                    let center =
                        sk::Point::new(f64_to_f32(sweep.center.x), f64_to_f32(sweep.center.y));
                    // `peniko` uses radians; Skia uses degrees for sweep gradient angles.
                    let start = rad_to_deg(sweep.start_angle);
                    let end = rad_to_deg(sweep.end_angle);
                    if let Some(shader) = sk::Shader::sweep_gradient_with_interpolation(
                        center,
                        (&colors[..], None),
                        Some(&pos[..]),
                        tile_mode,
                        Some((start, end)),
                        interpolation,
                        Some(&local),
                    ) {
                        paint.set_shader(shader);
                    }
                }
            }

            if paint.shader().is_none()
                && let Some(last_stop) = stops.last()
            {
                let color = last_stop
                    .color
                    .to_alpha_color::<peniko::color::Srgb>()
                    .multiply_alpha(alpha_scale);
                paint.set_color(color_to_sk_color(color));
            }
        }
        BrushRef::Image(image_brush) => {
            let image = skia_image_from_peniko(image_brush.image)?;
            let shader = image.to_shader(
                Some((
                    tile_mode_from_extend(image_brush.sampler.x_extend),
                    tile_mode_from_extend(image_brush.sampler.y_extend),
                )),
                sampling_options_from_quality(image_brush.sampler.quality),
                Some(&affine_to_matrix(paint_xf)),
            )?;
            paint.set_shader(shader);
            paint.set_alpha_f((image_brush.sampler.alpha * alpha_scale).clamp(0.0, 1.0));
        }
    }

    Some(paint)
}

fn skia_image_from_peniko(image: &peniko::ImageData) -> Option<sk::Image> {
    let color_type = match image.format {
        ImageFormat::Rgba8 => sk::ColorType::RGBA8888,
        ImageFormat::Bgra8 => sk::ColorType::BGRA8888,
        _ => return None,
    };
    let alpha_type = match image.alpha_type {
        ImageAlphaType::Alpha => sk::AlphaType::Unpremul,
        ImageAlphaType::AlphaPremultiplied => sk::AlphaType::Premul,
    };
    let info = sk::ImageInfo::new(
        (
            i32::try_from(image.width).ok()?,
            i32::try_from(image.height).ok()?,
        ),
        color_type,
        alpha_type,
        None,
    );
    let row_bytes = image.format.size_in_bytes(image.width, 1)?;
    sk::images::raster_from_data(&info, sk::Data::new_copy(image.data.data()), row_bytes)
}

fn sampling_options_from_quality(quality: ImageQuality) -> sk::SamplingOptions {
    match quality {
        ImageQuality::Low => sk::SamplingOptions::from(sk::FilterMode::Nearest),
        ImageQuality::Medium => sk::SamplingOptions::from(sk::FilterMode::Linear),
        ImageQuality::High => sk::SamplingOptions::from(sk::CubicResampler::mitchell()),
    }
}

fn apply_stroke_style(paint: &mut sk::Paint, style: &kurbo::Stroke) {
    paint.set_style(sk::PaintStyle::Stroke);
    paint.set_stroke_width(f64_to_f32(style.width));
    paint.set_stroke_miter(f64_to_f32(style.miter_limit));
    paint.set_stroke_join(match style.join {
        kurbo::Join::Bevel => sk::PaintJoin::Bevel,
        kurbo::Join::Miter => sk::PaintJoin::Miter,
        kurbo::Join::Round => sk::PaintJoin::Round,
    });
    let cap = match style.start_cap {
        kurbo::Cap::Butt => sk::PaintCap::Butt,
        kurbo::Cap::Square => sk::PaintCap::Square,
        kurbo::Cap::Round => sk::PaintCap::Round,
    };
    paint.set_stroke_cap(cap);
    if !style.dash_pattern.is_empty() {
        let intervals: Vec<f32> = style.dash_pattern.iter().map(|v| f64_to_f32(*v)).collect();
        if let Some(effect) =
            sk::PathEffect::dash(intervals.as_slice(), f64_to_f32(style.dash_offset))
        {
            paint.set_path_effect(effect);
        }
    }
}

fn map_blend_mode(mode: &peniko::BlendMode) -> sk::BlendMode {
    use peniko::{Compose, Mix};

    match (mode.mix, mode.compose) {
        (_, Compose::Clear) => sk::BlendMode::Clear,
        (_, Compose::Copy) => sk::BlendMode::Src,
        (_, Compose::Dest) => sk::BlendMode::Dst,
        (_, Compose::SrcOver) => match mode.mix {
            Mix::Normal => sk::BlendMode::SrcOver,
            Mix::Multiply => sk::BlendMode::Multiply,
            Mix::Screen => sk::BlendMode::Screen,
            Mix::Overlay => sk::BlendMode::Overlay,
            Mix::Darken => sk::BlendMode::Darken,
            Mix::Lighten => sk::BlendMode::Lighten,
            Mix::ColorDodge => sk::BlendMode::ColorDodge,
            Mix::ColorBurn => sk::BlendMode::ColorBurn,
            Mix::HardLight => sk::BlendMode::HardLight,
            Mix::SoftLight => sk::BlendMode::SoftLight,
            Mix::Difference => sk::BlendMode::Difference,
            Mix::Exclusion => sk::BlendMode::Exclusion,
            Mix::Hue => sk::BlendMode::Hue,
            Mix::Saturation => sk::BlendMode::Saturation,
            Mix::Color => sk::BlendMode::Color,
            Mix::Luminosity => sk::BlendMode::Luminosity,
        },
        (_, Compose::DestOver) => sk::BlendMode::DstOver,
        (_, Compose::SrcIn) => sk::BlendMode::SrcIn,
        (_, Compose::DestIn) => sk::BlendMode::DstIn,
        (_, Compose::SrcOut) => sk::BlendMode::SrcOut,
        (_, Compose::DestOut) => sk::BlendMode::DstOut,
        (_, Compose::SrcAtop) => sk::BlendMode::SrcATop,
        (_, Compose::DestAtop) => sk::BlendMode::DstATop,
        (_, Compose::Xor) => sk::BlendMode::Xor,
        (_, Compose::Plus) => sk::BlendMode::Plus,
        (_, Compose::PlusLighter) => sk::BlendMode::Plus,
    }
}

fn build_filter_chain(filters: &[Filter]) -> Option<sk::ImageFilter> {
    use sk::image_filters;

    let mut current: Option<sk::ImageFilter> = None;
    for f in filters {
        current = Some(match *f {
            Filter::Flood { color } => {
                let shader = sk::shaders::color(color_to_sk_color(color));
                // Leaf filter: ignores any existing input chain.
                image_filters::shader(shader, None)?
            }
            Filter::Blur {
                std_deviation_x,
                std_deviation_y,
            } => image_filters::blur((std_deviation_x, std_deviation_y), None, current, None)?,
            Filter::DropShadow {
                dx,
                dy,
                std_deviation_x,
                std_deviation_y,
                color,
            } => image_filters::drop_shadow(
                (dx, dy),
                (std_deviation_x, std_deviation_y),
                color_to_sk_color4f(color),
                None,
                current,
                None,
            )?,
            Filter::Offset { dx, dy } => image_filters::offset((dx, dy), current, None)?,
        });
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;
    use imaging::{GroupRef, MaskMode, Painter};
    use kurbo::Rect;
    use peniko::{Brush, Color};

    fn masked_scene(mode: MaskMode) -> Scene {
        let mask = Painter::<Scene>::record_mask(mode, |mask| {
            mask.fill(
                Rect::new(8.0, 8.0, 56.0, 56.0),
                Color::from_rgba8(255, 255, 255, 160),
            )
            .draw();
        });

        let mut scene = Scene::new();
        {
            let mut painter = Painter::new(&mut scene);
            painter.with_group(GroupRef::new().with_mask(mask.as_ref()), |content| {
                content
                    .fill(
                        Rect::new(0.0, 0.0, 64.0, 64.0),
                        Color::from_rgb8(0x2a, 0x6f, 0xdb),
                    )
                    .draw();
            });
        }

        scene
    }

    #[test]
    fn render_picture_rgba8_reads_native_picture() {
        let mut sink = SkPictureRecorderSink::new(Rect::new(0.0, 0.0, 32.0, 32.0));
        let paint = Brush::Solid(Color::from_rgb8(0x22, 0x66, 0xaa));
        {
            let mut painter = Painter::new(&mut sink);
            painter.fill_rect(Rect::new(0.0, 0.0, 32.0, 32.0), &paint);
        }

        let picture = sink.finish_picture().unwrap();
        let mut renderer = SkiaRenderer::new(32, 32);
        let rgba = renderer.render_picture_rgba8(&picture).unwrap();

        assert_eq!(rgba.len(), 32 * 32 * 4);
        assert_eq!(&rgba[..4], &[0x22, 0x66, 0xaa, 0xff]);
    }

    #[test]
    fn render_scene_reuses_cached_masks_for_identical_scenes() {
        let scene = masked_scene(MaskMode::Alpha);
        let mut renderer = SkiaRenderer::new(64, 64);

        renderer.render_scene_rgba8(&scene).unwrap();
        assert_eq!(renderer.mask_cache.borrow().len(), 1);

        renderer.render_scene_rgba8(&scene).unwrap();
        assert_eq!(renderer.mask_cache.borrow().len(), 1);
    }

    #[test]
    fn clear_cached_masks_drops_realized_masks() {
        let scene = masked_scene(MaskMode::Luminance);
        let mut renderer = SkiaRenderer::new(64, 64);

        renderer.render_scene_rgba8(&scene).unwrap();
        assert_eq!(renderer.mask_cache.borrow().len(), 1);

        renderer.clear_cached_masks();
        assert_eq!(renderer.mask_cache.borrow().len(), 0);

        renderer.render_scene_rgba8(&scene).unwrap();
        assert_eq!(renderer.mask_cache.borrow().len(), 1);
    }

    #[test]
    fn changing_tolerance_clears_cached_masks() {
        let scene = masked_scene(MaskMode::Alpha);
        let mut renderer = SkiaRenderer::new(64, 64);

        renderer.render_scene_rgba8(&scene).unwrap();
        assert_eq!(renderer.mask_cache.borrow().len(), 1);

        renderer.set_tolerance(0.25);
        assert_eq!(renderer.mask_cache.borrow().len(), 0);
    }
}
