// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Skia backend for `imaging`.
//!
//! This crate provides a CPU raster renderer that consumes `imaging::Scene` (or accepts commands
//! directly via `imaging::Sink`) and produces an RGBA8 image buffer using Skia.

#![deny(unsafe_code)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use imaging::{Clip, Draw, Filter, Geometry, Group, Scene, Sink, replay};
use kurbo::{Affine, Shape as _};
use peniko::Brush;
use peniko::InterpolationAlphaSpace;
use peniko::color::{ColorSpaceTag, HueDirection};
use skia_safe as sk;

/// Errors that can occur when rendering via Skia.
#[derive(Debug)]
pub enum Error {
    /// The scene is invalid (unbalanced stacks).
    InvalidScene(imaging::ValidateError),
    /// An image brush was encountered; this backend does not support images yet.
    UnsupportedImageBrush,
    /// A filter configuration could not be translated.
    UnsupportedFilter,
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
    error: Option<Error>,
    clip_depth: u32,
    group_stack: Vec<u8>,
}

impl SkiaRenderer {
    /// Create a renderer for a fixed-size target.
    pub fn new(width: u16, height: u16) -> Self {
        let width = i32::from(width);
        let height = i32::from(height);
        // Use an explicit RGBA8888 premultiplied raster surface. Many blend modes are defined in
        // premultiplied space, and it also matches Skia's typical raster backend behavior.
        //
        // Note: we still *export* unpremultiplied RGBA8 in `finish_rgba8()`.
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
            error: None,
            clip_depth: 0,
            group_stack: Vec::new(),
        }
    }

    /// Set the tolerance used when converting shapes to paths.
    pub fn set_tolerance(&mut self, tolerance: f64) {
        self.tolerance = tolerance;
    }

    /// Reset the internal surface and local error state.
    pub fn reset(&mut self) {
        self.error = None;
        self.clip_depth = 0;
        self.group_stack.clear();
        let canvas = self.surface.canvas();
        canvas.restore_to_count(1);
        canvas.reset_matrix();
        canvas.clear(sk::Color::TRANSPARENT);
    }

    /// Render a recorded scene and return an RGBA8 buffer (unpremultiplied).
    pub fn render_scene_rgba8(&mut self, scene: &Scene) -> Result<Vec<u8>, Error> {
        scene.validate().map_err(Error::InvalidScene)?;
        self.reset();
        replay(scene, self);
        self.finish_rgba8()
    }

    /// Finish rendering the current command stream and return an RGBA8 buffer (unpremultiplied).
    pub fn finish_rgba8(&mut self) -> Result<Vec<u8>, Error> {
        if let Some(err) = self.error.take() {
            return Err(err);
        }
        if self.clip_depth != 0 {
            return Err(Error::Internal("unbalanced clip stack"));
        }
        if !self.group_stack.is_empty() {
            return Err(Error::Internal("unbalanced group stack"));
        }

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

    fn set_error_once(&mut self, err: Error) {
        if self.error.is_none() {
            self.error = Some(err);
        }
    }

    fn set_matrix(&mut self, xf: Affine) {
        let canvas = self.surface.canvas();
        canvas.reset_matrix();
        canvas.concat(&affine_to_matrix(xf));
    }

    fn clip_path(&mut self, clip: &Clip) -> Option<sk::Path> {
        match clip {
            Clip::Fill {
                transform,
                shape,
                fill_rule,
            } => {
                let mut path = geometry_to_sk_path(shape, self.tolerance)?;
                self.set_matrix(*transform);
                path = path_with_fill_rule(&path, *fill_rule);
                Some(path)
            }
            Clip::Stroke {
                transform,
                shape,
                stroke,
            } => {
                // Convert to a filled outline path using kurbo stroking.
                let src = geometry_to_bez_path(shape, self.tolerance)?;
                // Stroke is in local coordinates; apply clip transform at clip time via canvas matrix.
                let outline = kurbo::stroke(
                    src.iter(),
                    stroke,
                    &kurbo::StrokeOpts::default(),
                    self.tolerance,
                );
                self.set_matrix(*transform);
                bez_to_sk_path(&outline)
            }
        }
    }

    fn push_group_impl(&mut self, group: &Group) -> u8 {
        let filter = if group.filters.is_empty() {
            None
        } else {
            build_filter_chain(&group.filters)
        };
        if !group.filters.is_empty() && filter.is_none() {
            self.set_error_once(Error::UnsupportedFilter);
        }

        let clip_path = group.clip.as_ref().and_then(|clip| self.clip_path(clip));
        let mut restores = 0_u8;

        let mut paint = sk::Paint::default();
        let mut needs_layer = false;

        // Group composite (blend + opacity) is applied at compositing time via saveLayer paint.
        let blend = group.composite.blend;
        let alpha = group.composite.alpha.clamp(0.0, 1.0);
        if blend != peniko::BlendMode::default() || alpha != 1.0 {
            paint.set_blend_mode(map_blend_mode(&blend));
            paint.set_alpha_f(alpha);
            needs_layer = true;
        }

        if let Some(filter) = filter {
            paint.set_image_filter(filter);
            needs_layer = true;
        }

        {
            let canvas = self.surface.canvas();

            if let Some(path) = clip_path.as_ref() {
                canvas.save();
                canvas.clip_path(path, None, true);
                restores += 1;
            }

            if needs_layer {
                let bounds = sk::Rect::new(-10_000.0, -10_000.0, 10_000.0, 10_000.0);
                let mut rec = sk::canvas::SaveLayerRec::default();
                rec = rec.bounds(&bounds);
                rec = rec.paint(&paint);
                canvas.save_layer(&rec);
                restores += 1;
            }
        }

        restores
    }
}

impl Sink for SkiaRenderer {
    fn push_clip(&mut self, clip: Clip) {
        if self.error.is_some() {
            return;
        }
        let Some(path) = self.clip_path(&clip) else {
            return;
        };
        let canvas = self.surface.canvas();
        canvas.save();
        canvas.clip_path(&path, None, true);
        self.clip_depth += 1;
    }

    fn pop_clip(&mut self) {
        if self.error.is_some() {
            return;
        }
        if self.clip_depth == 0 {
            self.set_error_once(Error::Internal("pop_clip underflow"));
            return;
        }
        self.surface.canvas().restore();
        self.clip_depth -= 1;
    }

    fn push_group(&mut self, group: Group) {
        if self.error.is_some() {
            return;
        }
        let restores = self.push_group_impl(&group);
        self.group_stack.push(restores);
    }

    fn pop_group(&mut self) {
        if self.error.is_some() {
            return;
        }
        let Some(restores) = self.group_stack.pop() else {
            self.set_error_once(Error::Internal("pop_group underflow"));
            return;
        };
        for _ in 0..restores {
            self.surface.canvas().restore();
        }
    }

    fn draw(&mut self, draw: Draw) {
        if self.error.is_some() {
            return;
        }

        match draw {
            Draw::Fill {
                transform,
                fill_rule,
                paint,
                paint_transform,
                shape,
                composite,
            } => {
                if matches!(paint, Brush::Image(_)) {
                    self.set_error_once(Error::UnsupportedImageBrush);
                    return;
                }
                self.set_matrix(transform);
                let mut sk_paint = brush_to_paint(
                    &paint,
                    composite.alpha,
                    paint_transform.unwrap_or(Affine::IDENTITY),
                );
                sk_paint.set_blend_mode(map_blend_mode(&composite.blend));
                sk_paint.set_style(sk::PaintStyle::Fill);

                match shape {
                    Geometry::Rect(r) => {
                        let rect = sk::Rect::new(
                            f64_to_f32(r.x0),
                            f64_to_f32(r.y0),
                            f64_to_f32(r.x1),
                            f64_to_f32(r.y1),
                        );
                        self.surface.canvas().draw_rect(rect, &sk_paint);
                    }
                    Geometry::RoundedRect(rr) => {
                        let path = rr.to_path(self.tolerance);
                        let sk_path = bez_to_sk_path(&path).expect("rounded rect to sk path");
                        let sk_path = path_with_fill_rule(&sk_path, fill_rule);
                        self.surface.canvas().draw_path(&sk_path, &sk_paint);
                    }
                    Geometry::Path(p) => {
                        let sk_path = bez_to_sk_path(&p).expect("path to sk path");
                        let sk_path = path_with_fill_rule(&sk_path, fill_rule);
                        self.surface.canvas().draw_path(&sk_path, &sk_paint);
                    }
                }
            }
            Draw::Stroke {
                transform,
                stroke,
                paint,
                paint_transform,
                shape,
                composite,
            } => {
                if matches!(paint, Brush::Image(_)) {
                    self.set_error_once(Error::UnsupportedImageBrush);
                    return;
                }
                self.set_matrix(transform);
                let mut sk_paint = brush_to_paint(
                    &paint,
                    composite.alpha,
                    paint_transform.unwrap_or(Affine::IDENTITY),
                );
                sk_paint.set_blend_mode(map_blend_mode(&composite.blend));
                apply_stroke_style(&mut sk_paint, &stroke);

                match shape {
                    Geometry::Rect(r) => {
                        let rect = sk::Rect::new(
                            f64_to_f32(r.x0),
                            f64_to_f32(r.y0),
                            f64_to_f32(r.x1),
                            f64_to_f32(r.y1),
                        );
                        self.surface.canvas().draw_rect(rect, &sk_paint);
                    }
                    Geometry::RoundedRect(rr) => {
                        let path = rr.to_path(self.tolerance);
                        let sk_path = bez_to_sk_path(&path).expect("rounded rect to sk path");
                        self.surface.canvas().draw_path(&sk_path, &sk_paint);
                    }
                    Geometry::Path(p) => {
                        let sk_path = bez_to_sk_path(&p).expect("path to sk path");
                        self.surface.canvas().draw_path(&sk_path, &sk_paint);
                    }
                }
            }
        }
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

fn geometry_to_bez_path(geom: &Geometry, tolerance: f64) -> Option<kurbo::BezPath> {
    Some(match geom {
        Geometry::Rect(r) => r.to_path(tolerance),
        Geometry::RoundedRect(rr) => rr.to_path(tolerance),
        Geometry::Path(p) => p.clone(),
    })
}

fn geometry_to_sk_path(geom: &Geometry, tolerance: f64) -> Option<sk::Path> {
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

fn brush_to_paint(brush: &Brush, opacity: f32, paint_xf: Affine) -> sk::Paint {
    let mut paint = sk::Paint::default();
    paint.set_anti_alias(true);
    let alpha_scale = opacity.clamp(0.0, 1.0);

    match brush {
        Brush::Solid(color) => {
            // Use float color to avoid quantizing alpha (important for Porter-Duff ops like XOR).
            let comps = color.components;
            let c = sk::Color4f::new(comps[0], comps[1], comps[2], comps[3] * alpha_scale);
            paint.set_color4f(c, None);
        }
        Brush::Gradient(grad) => {
            let stops = grad.stops.as_ref();
            if stops.is_empty() {
                paint.set_color(sk::Color::TRANSPARENT);
                return paint;
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
        Brush::Image(_) => {
            // Image brushes are not yet mapped by this crate. Callers should handle this earlier.
            paint.set_color(sk::Color::TRANSPARENT);
        }
    }

    paint
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
