// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::{
    Error, ImageCache, SkiaFontCache, affine_to_matrix, apply_stroke_style, bez_to_sk_path,
    brush_to_paint, build_filter_chain, f64_to_f32, geometry_to_bez_path, geometry_to_sk_path,
    map_blend_mode, path_with_fill_rule, skia_font_from_glyph_run,
};
use imaging::{
    BlurredRoundedRect, ClipRef, FillRef, GeometryRef, GlyphRunRef, GroupRef, MaskMode, PaintSink,
    StrokeRef,
    record::{self, replay_transformed},
};
use kurbo::{Affine, Rect, Shape as _};
use skia_safe as sk;
use std::{cell::RefCell, rc::Rc};

#[derive(Debug)]
struct StreamState {
    tolerance: f64,
    error: Option<Error>,
    clip_depth: u32,
    group_stack: Vec<GroupFrame>,
}

#[derive(Debug)]
enum GroupFrame {
    Direct {
        restores: u8,
        clip: Option<record::Clip>,
    },
    Masked(Box<MaskedGroupFrame>),
}

#[derive(Debug)]
struct MaskedGroupFrame {
    clip: Option<record::Clip>,
    filters: Vec<imaging::Filter>,
    filter_transform: Affine,
    composite: imaging::Composite,
    mode: MaskMode,
    transform: Affine,
    mask: record::Scene,
    content: record::Scene,
    nested_group_depth: u32,
}

impl StreamState {
    fn new() -> Self {
        Self {
            tolerance: 0.1,
            error: None,
            clip_depth: 0,
            group_stack: Vec::new(),
        }
    }

    fn set_error_once(&mut self, err: Error) {
        if self.error.is_none() {
            self.error = Some(err);
        }
    }

    fn finish(&mut self) -> Result<(), Error> {
        if let Some(err) = self.error.take() {
            return Err(err);
        }
        if self.clip_depth != 0 {
            return Err(Error::Internal("unbalanced clip stack"));
        }
        if !self.group_stack.is_empty() {
            return Err(Error::Internal("unbalanced group stack"));
        }
        Ok(())
    }
}

fn active_masked_group_mut(state: &mut StreamState) -> Option<&mut MaskedGroupFrame> {
    match state.group_stack.last_mut() {
        Some(GroupFrame::Masked(frame)) => Some(frame.as_mut()),
        _ => None,
    }
}

fn set_matrix(canvas: &sk::Canvas, xf: Affine) {
    canvas.reset_matrix();
    canvas.concat(&affine_to_matrix(xf));
}

fn clip_path(canvas: &sk::Canvas, state: &mut StreamState, clip: ClipRef<'_>) -> Option<sk::Path> {
    match clip {
        ClipRef::Fill {
            transform,
            shape,
            fill_rule,
        } => {
            let mut path = geometry_to_sk_path(shape, state.tolerance)?;
            set_matrix(canvas, transform);
            path = path_with_fill_rule(&path, fill_rule);
            Some(path)
        }
        ClipRef::Stroke {
            transform,
            shape,
            stroke,
        } => {
            let src = geometry_to_bez_path(shape, state.tolerance)?;
            let outline = kurbo::stroke(
                src.iter(),
                stroke,
                &kurbo::StrokeOpts::default(),
                state.tolerance,
            );
            set_matrix(canvas, transform);
            bez_to_sk_path(&outline)
        }
    }
}

fn push_group_impl(canvas: &sk::Canvas, state: &mut StreamState, group: GroupRef<'_>) -> u8 {
    if group
        .mask
        .as_ref()
        .is_some_and(|mask| mask.mask.scene.has_external_images())
    {
        state.set_error_once(Error::UnsupportedExternalImage);
    }
    let (filter, filter_transform) =
        build_transformed_filter_chain(group.filters, group.filter_transform);
    if !group.filters.is_empty() && filter.is_none() {
        state.set_error_once(Error::UnsupportedFilter);
    }

    let mut paint = sk::Paint::default();
    paint.set_blend_mode(map_blend_mode(&group.composite.blend));
    paint.set_alpha_f(group.composite.alpha.clamp(0.0, 1.0));
    set_matrix(canvas, Affine::IDENTITY);
    canvas.save_layer(&sk::canvas::SaveLayerRec::default().paint(&paint));
    let mut restores = 1_u8;
    if let Some(filter) = filter {
        set_matrix(canvas, filter_transform);
        let mut filter_paint = sk::Paint::default();
        filter_paint.set_image_filter(filter);
        canvas.save_layer(&sk::canvas::SaveLayerRec::default().paint(&filter_paint));
        set_matrix(canvas, Affine::IDENTITY);
        restores += 1;
    }

    restores
}

fn build_transformed_filter_chain(
    filters: &[imaging::Filter],
    transform: Affine,
) -> (Option<sk::ImageFilter>, Affine) {
    // Prefer device-space parameters, including for singular transforms that Skia's CTM
    // would otherwise treat as invisible. Preserve native affine kernels when needed.
    if let Some(filters) = filters
        .iter()
        .map(|filter| filter.in_device_space(transform))
        .collect::<Option<Vec<_>>>()
    {
        (build_filter_chain(&filters), Affine::IDENTITY)
    } else if !transform.is_finite() || transform.determinant() == 0.0 {
        (None, Affine::IDENTITY)
    } else {
        // Filter parameters use only the linear part. Check Skia's matrix range here,
        // at the native boundary, rather than constraining the shared geometry model.
        let transform = transform.with_translation(kurbo::Vec2::ZERO);
        if !transform
            .as_coeffs()
            .into_iter()
            .map(f64_to_f32)
            .all(f32::is_finite)
        {
            return (None, Affine::IDENTITY);
        }
        (build_filter_chain(filters), transform)
    }
}

fn apply_isolated_clip(canvas: &sk::Canvas, state: &mut StreamState, clip: ClipRef<'_>) {
    let Some(path) = clip_path(canvas, state, clip) else {
        return;
    };
    // A full isolated source clears the destination outside the clip path as well.
    let transform = canvas.local_to_device();
    set_matrix(canvas, Affine::IDENTITY);
    let mut mask_paint = sk::Paint::default();
    mask_paint.set_blend_mode(sk::BlendMode::DstIn);
    canvas.save_layer(&sk::canvas::SaveLayerRec::default().paint(&mask_paint));
    canvas.set_matrix(&transform);
    let mut paint = sk::Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(sk::Color::WHITE);
    canvas.draw_path(&path, &paint);
    canvas.restore();
}

fn draw_glyph_run(
    canvas: &sk::Canvas,
    state: &mut StreamState,
    image_cache: Option<&Rc<RefCell<ImageCache>>>,
    font_cache: Option<&SkiaFontCache>,
    glyph_run: GlyphRunRef<'_>,
    glyphs: &mut dyn Iterator<Item = record::Glyph>,
) {
    let Some(mut font) = skia_font_from_glyph_run(font_cache, &glyph_run) else {
        state.set_error_once(Error::InvalidFontData);
        return;
    };

    set_matrix(canvas, glyph_run.transform);

    if glyph_run.brush.as_raster().is_none() {
        state.set_error_once(Error::UnsupportedExternalImage);
        return;
    }
    let Some(mut sk_paint) = brush_to_paint(
        glyph_run.brush,
        glyph_run.composite.alpha,
        glyph_run.brush_transform.unwrap_or(Affine::IDENTITY),
        image_cache,
    ) else {
        state.set_error_once(Error::Internal("invalid image brush"));
        return;
    };
    sk_paint.set_blend_mode(map_blend_mode(&glyph_run.composite.blend));

    match glyph_run.style {
        peniko::Style::Fill(_) => {
            sk_paint.set_style(sk::PaintStyle::Fill);
        }
        peniko::Style::Stroke(stroke) => apply_stroke_style(&mut sk_paint, stroke),
    }

    let mut glyph_ids = Vec::new();
    let mut positions = Vec::new();
    for glyph in glyphs {
        let Ok(glyph_id) = sk::GlyphId::try_from(glyph.id) else {
            state.set_error_once(Error::InvalidGlyphId);
            return;
        };
        glyph_ids.push(glyph_id);
        positions.push(sk::Point::new(glyph.x, glyph.y));
    }

    font.set_subpixel(true);
    canvas.draw_glyphs_at(
        &glyph_ids,
        positions.as_slice(),
        sk::Point::new(0.0, 0.0),
        &font,
        &sk_paint,
    );
}

fn draw_blurred_rounded_rect(
    canvas: &sk::Canvas,
    state: &mut StreamState,
    draw: BlurredRoundedRect,
) {
    set_matrix(canvas, draw.transform);

    let mut paint = sk::Paint::default();
    paint.set_anti_alias(true);
    paint.set_style(sk::PaintStyle::Fill);
    let color = draw.color.multiply_alpha(draw.composite.alpha);
    let comps = color.components;
    paint.set_color4f(
        sk::Color4f::new(comps[0], comps[1], comps[2], comps[3]),
        None,
    );
    paint.set_blend_mode(map_blend_mode(&draw.composite.blend));
    let Some(mask_filter) =
        sk::MaskFilter::blur(sk::BlurStyle::Normal, f64_to_f32(draw.std_dev), Some(true))
    else {
        state.set_error_once(Error::Internal("create blur mask filter"));
        return;
    };
    paint.set_mask_filter(mask_filter);

    let rect = sk::Rect::new(
        f64_to_f32(draw.rect.x0),
        f64_to_f32(draw.rect.y0),
        f64_to_f32(draw.rect.x1),
        f64_to_f32(draw.rect.y1),
    );
    let rrect = sk::RRect::new_rect_xy(rect, f64_to_f32(draw.radius), f64_to_f32(draw.radius));
    canvas.draw_rrect(rrect, &paint);
}

fn replay_scene_into_canvas(
    canvas: &sk::Canvas,
    state: &mut StreamState,
    image_cache: Option<&Rc<RefCell<ImageCache>>>,
    font_cache: Option<&SkiaFontCache>,
    scene: &record::Scene,
    transform: Affine,
) -> bool {
    let mut sink = match font_cache {
        Some(font_cache) => {
            SkCanvasSink::new_with_caches(canvas, image_cache.cloned(), font_cache.clone())
        }
        None => SkCanvasSink::new(canvas),
    };
    sink.set_tolerance(state.tolerance);
    replay_transformed(scene, &mut sink, transform);
    if let Err(err) = sink.finish() {
        state.set_error_once(err);
        return false;
    }
    true
}

fn draw_masked_group(
    canvas: &sk::Canvas,
    state: &mut StreamState,
    masked: MaskedGroupFrame,
    image_cache: Option<&Rc<RefCell<ImageCache>>>,
    font_cache: Option<&SkiaFontCache>,
) {
    let mut paint = sk::Paint::default();
    paint.set_anti_alias(true);
    paint.set_blend_mode(map_blend_mode(&masked.composite.blend));
    paint.set_alpha_f(masked.composite.alpha.clamp(0.0, 1.0));
    let (filter, filter_transform) =
        build_transformed_filter_chain(&masked.filters, masked.filter_transform);
    if !masked.filters.is_empty() && filter.is_none() {
        state.set_error_once(Error::UnsupportedFilter);
        return;
    }

    set_matrix(canvas, Affine::IDENTITY);
    let bounds = canvas.local_clip_bounds();
    let mut content_layer = sk::canvas::SaveLayerRec::default();
    if let Some(bounds) = bounds.as_ref() {
        content_layer = content_layer.bounds(bounds);
    }
    content_layer = content_layer.paint(&paint);
    canvas.save_layer(&content_layer);

    // Filter the content before it is modulated by the fully rendered mask.
    if let Some(filter) = filter.as_ref() {
        set_matrix(canvas, filter_transform);
        let mut filter_paint = sk::Paint::default();
        filter_paint.set_image_filter(filter.clone());
        let rec = sk::canvas::SaveLayerRec::default().paint(&filter_paint);
        canvas.save_layer(&rec);
        set_matrix(canvas, Affine::IDENTITY);
    }

    if !replay_scene_into_canvas(
        canvas,
        state,
        image_cache,
        font_cache,
        &masked.content,
        Affine::IDENTITY,
    ) {
        if filter.is_some() {
            canvas.restore();
        }
        canvas.restore();
        return;
    }
    if filter.is_some() {
        canvas.restore();
    }

    set_matrix(canvas, Affine::IDENTITY);
    let mut mask_paint = sk::Paint::default();
    mask_paint.set_anti_alias(true);
    mask_paint.set_blend_mode(sk::BlendMode::DstIn);
    if masked.mode == MaskMode::Luminance {
        mask_paint.set_color_filter(sk::ColorFilter::luma());
    }
    let mut mask_layer = sk::canvas::SaveLayerRec::default();
    if let Some(bounds) = bounds.as_ref() {
        mask_layer = mask_layer.bounds(bounds);
    }
    mask_layer = mask_layer.paint(&mask_paint);
    canvas.save_layer(&mask_layer);

    if !replay_scene_into_canvas(
        canvas,
        state,
        image_cache,
        font_cache,
        &masked.mask,
        masked.transform,
    ) {
        canvas.restore();
        canvas.restore();
        return;
    }

    canvas.restore();
    if let Some(clip) = masked.clip.as_ref() {
        apply_isolated_clip(canvas, state, clip.as_ref());
    }
    canvas.restore();
}

fn paint_sink_push_clip(canvas: &sk::Canvas, state: &mut StreamState, clip: ClipRef<'_>) {
    if state.error.is_some() {
        return;
    }
    if let Some(frame) = active_masked_group_mut(state) {
        PaintSink::push_clip(&mut frame.content, clip);
        return;
    }
    let Some(path) = clip_path(canvas, state, clip) else {
        return;
    };
    canvas.save();
    canvas.clip_path(&path, None, true);
    state.clip_depth += 1;
}

fn paint_sink_pop_clip(canvas: &sk::Canvas, state: &mut StreamState) {
    if state.error.is_some() {
        return;
    }
    if let Some(frame) = active_masked_group_mut(state) {
        frame.content.pop_clip();
        return;
    }
    if state.clip_depth == 0 {
        state.set_error_once(Error::Internal("pop_clip underflow"));
        return;
    }
    canvas.restore();
    state.clip_depth -= 1;
}

fn paint_sink_push_group(canvas: &sk::Canvas, state: &mut StreamState, group: GroupRef<'_>) {
    if state.error.is_some() {
        return;
    }
    if let Some(frame) = active_masked_group_mut(state) {
        PaintSink::push_group(&mut frame.content, group);
        frame.nested_group_depth += 1;
        return;
    }
    if let Some(mask) = group.mask {
        state
            .group_stack
            .push(GroupFrame::Masked(Box::new(MaskedGroupFrame {
                clip: group.clip.map(ClipRef::to_owned),
                filters: group.filters.to_vec(),
                filter_transform: group.filter_transform,
                composite: group.composite,
                mode: mask.mask.mode,
                transform: mask.transform,
                mask: mask.mask.scene.clone(),
                content: record::Scene::new(),
                nested_group_depth: 0,
            })));
        return;
    }
    let clip = group.clip.as_ref().map(|clip| clip.clone().to_owned());
    let restores = push_group_impl(canvas, state, group);
    state
        .group_stack
        .push(GroupFrame::Direct { restores, clip });
}

fn paint_sink_pop_group(
    canvas: &sk::Canvas,
    state: &mut StreamState,
    image_cache: Option<&Rc<RefCell<ImageCache>>>,
    font_cache: Option<&SkiaFontCache>,
) {
    if state.error.is_some() {
        return;
    }
    let Some(frame) = state.group_stack.pop() else {
        state.set_error_once(Error::Internal("pop_group underflow"));
        return;
    };
    match frame {
        GroupFrame::Direct { restores, clip } => {
            for _ in 1..restores {
                canvas.restore();
            }
            if let Some(clip) = clip.as_ref() {
                apply_isolated_clip(canvas, state, clip.as_ref());
            }
            canvas.restore();
        }
        GroupFrame::Masked(mut frame) => {
            if frame.nested_group_depth != 0 {
                frame.content.pop_group();
                frame.nested_group_depth -= 1;
                state.group_stack.push(GroupFrame::Masked(frame));
                return;
            }
            draw_masked_group(canvas, state, *frame, image_cache, font_cache);
        }
    }
}

fn paint_sink_fill(
    canvas: &sk::Canvas,
    state: &mut StreamState,
    image_cache: Option<&Rc<RefCell<ImageCache>>>,
    draw: FillRef<'_>,
) {
    if state.error.is_some() {
        return;
    }
    if let Some(frame) = active_masked_group_mut(state) {
        frame.content.fill(draw);
        return;
    }

    set_matrix(canvas, draw.transform);
    if draw.brush.as_raster().is_none() {
        state.set_error_once(Error::UnsupportedExternalImage);
        return;
    }
    let Some(mut sk_paint) = brush_to_paint(
        draw.brush,
        draw.composite.alpha,
        draw.brush_transform.unwrap_or(Affine::IDENTITY),
        image_cache,
    ) else {
        state.set_error_once(Error::Internal("invalid image brush"));
        return;
    };
    sk_paint.set_blend_mode(map_blend_mode(&draw.composite.blend));
    sk_paint.set_style(sk::PaintStyle::Fill);

    match draw.shape {
        GeometryRef::Rect(r) => {
            let rect = sk::Rect::new(
                f64_to_f32(r.x0),
                f64_to_f32(r.y0),
                f64_to_f32(r.x1),
                f64_to_f32(r.y1),
            );
            canvas.draw_rect(rect, &sk_paint);
        }
        GeometryRef::RoundedRect(rr) => {
            let path = rr.to_path(state.tolerance);
            let sk_path = bez_to_sk_path(&path).expect("rounded rect to sk path");
            let sk_path = path_with_fill_rule(&sk_path, draw.fill_rule);
            canvas.draw_path(&sk_path, &sk_paint);
        }
        GeometryRef::Path(p) => {
            let sk_path = bez_to_sk_path(p).expect("path to sk path");
            let sk_path = path_with_fill_rule(&sk_path, draw.fill_rule);
            canvas.draw_path(&sk_path, &sk_paint);
        }
        GeometryRef::OwnedPath(p) => {
            let sk_path = bez_to_sk_path(&p).expect("path to sk path");
            let sk_path = path_with_fill_rule(&sk_path, draw.fill_rule);
            canvas.draw_path(&sk_path, &sk_paint);
        }
    }
}

fn paint_sink_stroke(
    canvas: &sk::Canvas,
    state: &mut StreamState,
    image_cache: Option<&Rc<RefCell<ImageCache>>>,
    draw: StrokeRef<'_>,
) {
    if state.error.is_some() {
        return;
    }
    if let Some(frame) = active_masked_group_mut(state) {
        frame.content.stroke(draw);
        return;
    }

    set_matrix(canvas, draw.transform);
    if draw.brush.as_raster().is_none() {
        state.set_error_once(Error::UnsupportedExternalImage);
        return;
    }
    let Some(mut sk_paint) = brush_to_paint(
        draw.brush,
        draw.composite.alpha,
        draw.brush_transform.unwrap_or(Affine::IDENTITY),
        image_cache,
    ) else {
        state.set_error_once(Error::Internal("invalid image brush"));
        return;
    };
    sk_paint.set_blend_mode(map_blend_mode(&draw.composite.blend));
    apply_stroke_style(&mut sk_paint, draw.stroke);

    match draw.shape {
        GeometryRef::Rect(r) => {
            let rect = sk::Rect::new(
                f64_to_f32(r.x0),
                f64_to_f32(r.y0),
                f64_to_f32(r.x1),
                f64_to_f32(r.y1),
            );
            canvas.draw_rect(rect, &sk_paint);
        }
        GeometryRef::RoundedRect(rr) => {
            let path = rr.to_path(state.tolerance);
            let sk_path = bez_to_sk_path(&path).expect("rounded rect to sk path");
            canvas.draw_path(&sk_path, &sk_paint);
        }
        GeometryRef::Path(p) => {
            let sk_path = bez_to_sk_path(p).expect("path to sk path");
            canvas.draw_path(&sk_path, &sk_paint);
        }
        GeometryRef::OwnedPath(p) => {
            let sk_path = bez_to_sk_path(&p).expect("path to sk path");
            canvas.draw_path(&sk_path, &sk_paint);
        }
    }
}

/// Borrowed adapter that streams `imaging` commands into an existing [`skia_safe::Canvas`].
pub struct SkCanvasSink<'a> {
    canvas: &'a sk::Canvas,
    image_cache: Option<Rc<RefCell<ImageCache>>>,
    font_cache: Option<SkiaFontCache>,
    state: StreamState,
}

impl core::fmt::Debug for SkCanvasSink<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SkCanvasSink")
            .field("tolerance", &self.state.tolerance)
            .field("error", &self.state.error)
            .field("clip_depth", &self.state.clip_depth)
            .field("group_depth", &self.state.group_stack.len())
            .finish_non_exhaustive()
    }
}

impl<'a> SkCanvasSink<'a> {
    /// Wrap an existing [`skia_safe::Canvas`].
    pub fn new(canvas: &'a sk::Canvas) -> Self {
        Self {
            canvas,
            image_cache: None,
            font_cache: None,
            state: StreamState::new(),
        }
    }

    pub(crate) fn new_with_caches(
        canvas: &'a sk::Canvas,
        image_cache: Option<Rc<RefCell<ImageCache>>>,
        font_cache: SkiaFontCache,
    ) -> Self {
        Self {
            canvas,
            image_cache,
            font_cache: Some(font_cache),
            state: StreamState::new(),
        }
    }

    /// Set the tolerance used when converting rounded rectangles to paths.
    pub fn set_tolerance(&mut self, tolerance: f64) {
        self.state.tolerance = tolerance;
    }

    /// Return the first deferred translation error, if any, and ensure clip/group stacks are balanced.
    pub fn finish(&mut self) -> Result<(), Error> {
        self.state.finish()
    }
}

impl PaintSink for SkCanvasSink<'_> {
    fn push_clip(&mut self, clip: ClipRef<'_>) {
        paint_sink_push_clip(self.canvas, &mut self.state, clip);
    }

    fn pop_clip(&mut self) {
        paint_sink_pop_clip(self.canvas, &mut self.state);
    }

    fn push_group(&mut self, group: GroupRef<'_>) {
        paint_sink_push_group(self.canvas, &mut self.state, group);
    }

    fn pop_group(&mut self) {
        paint_sink_pop_group(
            self.canvas,
            &mut self.state,
            self.image_cache.as_ref(),
            self.font_cache.as_ref(),
        );
    }

    fn fill(&mut self, draw: FillRef<'_>) {
        paint_sink_fill(
            self.canvas,
            &mut self.state,
            self.image_cache.as_ref(),
            draw,
        );
    }

    fn stroke(&mut self, draw: StrokeRef<'_>) {
        paint_sink_stroke(
            self.canvas,
            &mut self.state,
            self.image_cache.as_ref(),
            draw,
        );
    }

    fn glyph_run(
        &mut self,
        draw: GlyphRunRef<'_>,
        glyphs: &mut dyn Iterator<Item = record::Glyph>,
    ) {
        if self.state.error.is_some() {
            return;
        }
        if let Some(frame) = active_masked_group_mut(&mut self.state) {
            frame.content.glyph_run(draw, glyphs);
            return;
        }
        draw_glyph_run(
            self.canvas,
            &mut self.state,
            self.image_cache.as_ref(),
            self.font_cache.as_ref(),
            draw,
            glyphs,
        );
    }

    fn blurred_rounded_rect(&mut self, draw: BlurredRoundedRect) {
        if self.state.error.is_some() {
            return;
        }
        if let Some(frame) = active_masked_group_mut(&mut self.state) {
            frame.content.blurred_rounded_rect(draw);
            return;
        }
        draw_blurred_rounded_rect(self.canvas, &mut self.state, draw);
    }
}

/// Owned sink that records `imaging` commands into a native [`skia_safe::Picture`].
pub struct SkPictureRecorderSink {
    recorder: sk::PictureRecorder,
    image_cache: Option<Rc<RefCell<ImageCache>>>,
    font_cache: Option<SkiaFontCache>,
    state: StreamState,
}

impl core::fmt::Debug for SkPictureRecorderSink {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SkPictureRecorderSink")
            .field("tolerance", &self.state.tolerance)
            .field("error", &self.state.error)
            .field("clip_depth", &self.state.clip_depth)
            .field("group_depth", &self.state.group_stack.len())
            .finish_non_exhaustive()
    }
}

impl SkPictureRecorderSink {
    /// Start recording a Skia picture with the given cull bounds.
    pub fn new(bounds: Rect) -> Self {
        Self::new_with_bbh(bounds, false)
    }

    #[cfg(feature = "gpu")]
    pub(crate) fn new_with_caches(
        bounds: Rect,
        image_cache: Option<Rc<RefCell<ImageCache>>>,
        font_cache: SkiaFontCache,
    ) -> Self {
        Self::new_with_options(bounds, false, image_cache, Some(font_cache))
    }

    /// Start recording a Skia picture with optional bounding-box hierarchy acceleration.
    pub fn new_with_bbh(bounds: Rect, use_bbh: bool) -> Self {
        Self::new_with_options(bounds, use_bbh, None, None)
    }

    fn new_with_options(
        bounds: Rect,
        use_bbh: bool,
        image_cache: Option<Rc<RefCell<ImageCache>>>,
        font_cache: Option<SkiaFontCache>,
    ) -> Self {
        let mut recorder = sk::PictureRecorder::new();
        let bounds = sk::Rect::new(
            f64_to_f32(bounds.x0),
            f64_to_f32(bounds.y0),
            f64_to_f32(bounds.x1),
            f64_to_f32(bounds.y1),
        );
        let _ = recorder.begin_recording(bounds, use_bbh);
        Self {
            recorder,
            image_cache,
            font_cache,
            state: StreamState::new(),
        }
    }

    /// Set the tolerance used when converting rounded rectangles to paths.
    pub fn set_tolerance(&mut self, tolerance: f64) {
        self.state.tolerance = tolerance;
    }

    /// Finish recording and return the resulting [`skia_safe::Picture`].
    pub fn finish_picture(mut self) -> Result<sk::Picture, Error> {
        self.state.finish()?;
        self.recorder
            .finish_recording_as_picture(None)
            .ok_or(Error::Internal("finish_recording_as_picture failed"))
    }
}

impl PaintSink for SkPictureRecorderSink {
    fn push_clip(&mut self, clip: ClipRef<'_>) {
        let recorder = &mut self.recorder;
        let state = &mut self.state;
        let Some(canvas) = recorder.recording_canvas() else {
            state.set_error_once(Error::Internal("picture recorder not recording"));
            return;
        };
        paint_sink_push_clip(canvas, state, clip);
    }

    fn pop_clip(&mut self) {
        let recorder = &mut self.recorder;
        let state = &mut self.state;
        let Some(canvas) = recorder.recording_canvas() else {
            state.set_error_once(Error::Internal("picture recorder not recording"));
            return;
        };
        paint_sink_pop_clip(canvas, state);
    }

    fn push_group(&mut self, group: GroupRef<'_>) {
        let recorder = &mut self.recorder;
        let state = &mut self.state;
        let Some(canvas) = recorder.recording_canvas() else {
            state.set_error_once(Error::Internal("picture recorder not recording"));
            return;
        };
        paint_sink_push_group(canvas, state, group);
    }

    fn pop_group(&mut self) {
        let recorder = &mut self.recorder;
        let state = &mut self.state;
        let Some(canvas) = recorder.recording_canvas() else {
            state.set_error_once(Error::Internal("picture recorder not recording"));
            return;
        };
        paint_sink_pop_group(
            canvas,
            state,
            self.image_cache.as_ref(),
            self.font_cache.as_ref(),
        );
    }

    fn fill(&mut self, draw: FillRef<'_>) {
        let recorder = &mut self.recorder;
        let state = &mut self.state;
        let Some(canvas) = recorder.recording_canvas() else {
            state.set_error_once(Error::Internal("picture recorder not recording"));
            return;
        };
        paint_sink_fill(canvas, state, self.image_cache.as_ref(), draw);
    }

    fn stroke(&mut self, draw: StrokeRef<'_>) {
        let recorder = &mut self.recorder;
        let state = &mut self.state;
        let Some(canvas) = recorder.recording_canvas() else {
            state.set_error_once(Error::Internal("picture recorder not recording"));
            return;
        };
        paint_sink_stroke(canvas, state, self.image_cache.as_ref(), draw);
    }

    fn glyph_run(
        &mut self,
        draw: GlyphRunRef<'_>,
        glyphs: &mut dyn Iterator<Item = record::Glyph>,
    ) {
        let recorder = &mut self.recorder;
        let state = &mut self.state;
        let Some(canvas) = recorder.recording_canvas() else {
            state.set_error_once(Error::Internal("picture recorder not recording"));
            return;
        };
        if state.error.is_some() {
            return;
        }
        if let Some(frame) = active_masked_group_mut(state) {
            frame.content.glyph_run(draw, glyphs);
            return;
        }
        draw_glyph_run(
            canvas,
            state,
            self.image_cache.as_ref(),
            self.font_cache.as_ref(),
            draw,
            glyphs,
        );
    }

    fn blurred_rounded_rect(&mut self, draw: BlurredRoundedRect) {
        let recorder = &mut self.recorder;
        let state = &mut self.state;
        let Some(canvas) = recorder.recording_canvas() else {
            state.set_error_once(Error::Internal("picture recorder not recording"));
            return;
        };
        if state.error.is_some() {
            return;
        }
        if let Some(frame) = active_masked_group_mut(state) {
            frame.content.blurred_rounded_rect(draw);
            return;
        }
        draw_blurred_rounded_rect(canvas, state, draw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imaging::Composite;
    use peniko::{Brush, Color};

    #[test]
    fn sk_canvas_sink_reports_clip_underflow() {
        let mut surface = sk::surfaces::raster_n32_premul((16, 16)).unwrap();
        let mut sink = SkCanvasSink::new(surface.canvas());
        sink.pop_clip();
        assert!(matches!(
            sink.finish(),
            Err(Error::Internal("pop_clip underflow"))
        ));
    }

    #[test]
    fn sk_picture_recorder_sink_finishes_picture() {
        let mut sink = SkPictureRecorderSink::new(Rect::new(0.0, 0.0, 32.0, 32.0));
        sink.fill(FillRef::new(
            Rect::new(0.0, 0.0, 16.0, 16.0),
            &Brush::Solid(Color::from_rgb8(0x11, 0x22, 0x33)),
        ));
        let picture = sink.finish_picture().unwrap();
        let cull = picture.cull_rect();
        assert_eq!(cull.left, 0.0);
        assert_eq!(cull.top, 0.0);
        assert_eq!(cull.right, 32.0);
        assert_eq!(cull.bottom, 32.0);
    }

    #[test]
    fn sk_picture_recorder_sink_rejects_unbalanced_group() {
        let mut sink = SkPictureRecorderSink::new(Rect::new(0.0, 0.0, 32.0, 32.0));
        sink.push_group(GroupRef::new().with_composite(Composite::default()));
        assert!(matches!(
            sink.finish_picture(),
            Err(Error::Internal("unbalanced group stack"))
        ));
    }
}
