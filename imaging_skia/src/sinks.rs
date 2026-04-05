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
    record::{self, replay, replay_transformed},
};
use kurbo::{Affine, Rect, Shape as _};
use skia_safe as sk;
use std::{cell::RefCell, collections::VecDeque, rc::Rc};

#[derive(Clone, Debug)]
struct CachedMask {
    scene: record::Scene,
    mode: MaskMode,
    transform: Affine,
    width: i32,
    height: i32,
    tolerance: f64,
    bytes: Vec<u8>,
}

impl CachedMask {
    fn matches(
        &self,
        scene: &record::Scene,
        mode: MaskMode,
        transform: Affine,
        width: i32,
        height: i32,
        tolerance: f64,
    ) -> bool {
        self.scene == *scene
            && self.mode == mode
            && self.transform == transform
            && self.width == width
            && self.height == height
            && self.tolerance == tolerance
    }
}

#[derive(Debug)]
pub(crate) struct MaskCache {
    bytes_used: usize,
    max_bytes: usize,
    entries: VecDeque<CachedMask>,
}

impl MaskCache {
    pub(crate) fn new(max_bytes: usize) -> Self {
        Self {
            bytes_used: 0,
            max_bytes,
            entries: VecDeque::new(),
        }
    }

    pub(crate) fn clear(&mut self) {
        self.bytes_used = 0;
        self.entries.clear();
    }

    pub(crate) fn set_max_bytes(&mut self, max_bytes: usize) {
        self.max_bytes = max_bytes;
        self.evict_to_budget();
    }

    fn touch(&mut self, index: usize) {
        if index + 1 == self.entries.len() {
            return;
        }
        if let Some(entry) = self.entries.remove(index) {
            self.entries.push_back(entry);
        }
    }

    fn evict_to_budget(&mut self) {
        while self.bytes_used > self.max_bytes {
            let Some(oldest) = self.entries.pop_front() else {
                break;
            };
            self.bytes_used = self.bytes_used.saturating_sub(oldest.bytes.len());
        }
    }

    fn get(
        &mut self,
        scene: &record::Scene,
        mode: MaskMode,
        transform: Affine,
        width: i32,
        height: i32,
        tolerance: f64,
    ) -> Option<Vec<u8>> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.matches(scene, mode, transform, width, height, tolerance))?;
        let bytes = self.entries.get(index)?.bytes.clone();
        self.touch(index);
        Some(bytes)
    }

    fn insert(
        &mut self,
        scene: &record::Scene,
        mode: MaskMode,
        transform: Affine,
        width: i32,
        height: i32,
        tolerance: f64,
        bytes: &[u8],
    ) {
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.matches(scene, mode, transform, width, height, tolerance))
            && let Some(existing) = self.entries.remove(index)
        {
            self.bytes_used = self.bytes_used.saturating_sub(existing.bytes.len());
        }

        // If more backends end up wanting realized-mask caches, add a portable scene/cache key at
        // the imaging layer instead of retaining full scenes in backend-local caches.
        let entry = CachedMask {
            scene: scene.clone(),
            mode,
            transform,
            width,
            height,
            tolerance,
            bytes: bytes.to_vec(),
        };
        self.bytes_used = self.bytes_used.saturating_add(entry.bytes.len());
        self.entries.push_back(entry);
        self.evict_to_budget();
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

impl Default for MaskCache {
    fn default() -> Self {
        Self::new(64 * 1024 * 1024)
    }
}

#[derive(Debug)]
struct StreamState {
    tolerance: f64,
    error: Option<Error>,
    clip_depth: u32,
    group_stack: Vec<GroupFrame>,
}

#[derive(Debug)]
enum GroupFrame {
    Direct { restores: u8 },
    Masked(Box<MaskedGroupFrame>),
}

#[derive(Debug)]
struct MaskedGroupFrame {
    clip: Option<record::Clip>,
    filters: Vec<imaging::Filter>,
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
    let filter = if group.filters.is_empty() {
        None
    } else {
        build_filter_chain(group.filters)
    };
    if !group.filters.is_empty() && filter.is_none() {
        state.set_error_once(Error::UnsupportedFilter);
    }

    let clip_path = group.clip.and_then(|clip| clip_path(canvas, state, clip));
    let mut restores = 0_u8;

    let mut paint = sk::Paint::default();
    let mut needs_layer = false;

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

    restores
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

    let Some(mut sk_paint) = brush_to_paint(
        glyph_run.brush,
        glyph_run.composite.alpha,
        Affine::IDENTITY,
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

fn canvas_dimensions(canvas: &sk::Canvas) -> Option<(i32, i32)> {
    let dims = canvas.image_info().dimensions();
    (dims.width > 0 && dims.height > 0).then_some((dims.width, dims.height))
}

fn raster_surface_for_canvas(canvas: &sk::Canvas) -> Option<sk::Surface> {
    let (width, height) = canvas_dimensions(canvas)?;
    sk::surfaces::raster_n32_premul((width, height))
}

fn read_surface_rgba_premul(surface: &mut sk::Surface) -> Option<Vec<u8>> {
    let image = surface.image_snapshot();
    let dims = image.dimensions();
    let row_bytes = (dims.width as usize) * 4;
    let info = sk::ImageInfo::new(
        (dims.width, dims.height),
        sk::ColorType::RGBA8888,
        sk::AlphaType::Premul,
        None,
    );
    let mut bytes = vec![0_u8; row_bytes * (dims.height as usize)];
    image
        .read_pixels(
            &info,
            bytes.as_mut_slice(),
            row_bytes,
            (0, 0),
            sk::image::CachingHint::Disallow,
        )
        .then_some(bytes)
}

fn mask_value(mask: &[u8], mode: MaskMode) -> u8 {
    match mode {
        MaskMode::Alpha => mask[3],
        MaskMode::Luminance => {
            let value = (54_u32 * u32::from(mask[0])
                + 183_u32 * u32::from(mask[1])
                + 19_u32 * u32::from(mask[2])
                + 128)
                >> 8;
            u8::try_from(value).expect("luminance mask value stays within u8 range")
        }
    }
}

fn apply_mask_to_premul_rgba(content: &mut [u8], mask: &[u8], mode: MaskMode) {
    for (content_px, mask_px) in content.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
        let mask = u32::from(mask_value(mask_px, mode));
        for channel in content_px.iter_mut() {
            let value = (u32::from(*channel) * mask + 127) / 255;
            *channel = u8::try_from(value).expect("masked premul channel stays within u8 range");
        }
    }
}

fn draw_masked_group(
    canvas: &sk::Canvas,
    state: &mut StreamState,
    masked: MaskedGroupFrame,
    image_cache: Option<&Rc<RefCell<ImageCache>>>,
    mask_cache: Option<&Rc<RefCell<MaskCache>>>,
    font_cache: Option<&SkiaFontCache>,
) {
    let Some((width, height)) = canvas_dimensions(canvas) else {
        state.set_error_once(Error::Internal(
            "masked layer requires raster canvas dimensions",
        ));
        return;
    };

    let mask_bytes = if let Some(cache) = mask_cache
        && let Some(bytes) = cache.borrow_mut().get(
            &masked.mask,
            masked.mode,
            masked.transform,
            width,
            height,
            state.tolerance,
        ) {
        bytes
    } else {
        let Some(mut mask_surface) = raster_surface_for_canvas(canvas) else {
            state.set_error_once(Error::Internal(
                "masked layer requires raster canvas dimensions",
            ));
            return;
        };
        {
            let mut sink = match (mask_cache, font_cache) {
                (Some(cache), Some(font_cache)) => SkCanvasSink::new_with_caches(
                    mask_surface.canvas(),
                    image_cache.cloned(),
                    cache.clone(),
                    font_cache.clone(),
                ),
                (Some(_), None) => {
                    state.set_error_once(Error::Internal("mask cache missing paired font cache"));
                    return;
                }
                (None, _) => SkCanvasSink::new(mask_surface.canvas()),
            };
            sink.set_tolerance(state.tolerance);
            replay_transformed(&masked.mask, &mut sink, masked.transform);
            if let Err(err) = sink.finish() {
                state.set_error_once(err);
                return;
            }
        }
        let Some(bytes) = read_surface_rgba_premul(&mut mask_surface) else {
            state.set_error_once(Error::Internal("read masked layer surface"));
            return;
        };
        if let Some(cache) = mask_cache {
            cache.borrow_mut().insert(
                &masked.mask,
                masked.mode,
                masked.transform,
                width,
                height,
                state.tolerance,
                &bytes,
            );
        }
        bytes
    };

    let Some(mut content_surface) = raster_surface_for_canvas(canvas) else {
        state.set_error_once(Error::Internal(
            "masked layer requires raster canvas dimensions",
        ));
        return;
    };

    {
        let mut sink = match (mask_cache, font_cache) {
            (Some(cache), Some(font_cache)) => SkCanvasSink::new_with_caches(
                content_surface.canvas(),
                image_cache.cloned(),
                cache.clone(),
                font_cache.clone(),
            ),
            (Some(_), None) => {
                state.set_error_once(Error::Internal("mask cache missing paired font cache"));
                return;
            }
            (None, _) => SkCanvasSink::new(content_surface.canvas()),
        };
        sink.set_tolerance(state.tolerance);
        replay(&masked.content, &mut sink);
        if let Err(err) = sink.finish() {
            state.set_error_once(err);
            return;
        }
    }
    let Some(mut content_bytes) = read_surface_rgba_premul(&mut content_surface) else {
        state.set_error_once(Error::Internal("read masked content surface"));
        return;
    };
    apply_mask_to_premul_rgba(&mut content_bytes, &mask_bytes, masked.mode);
    let info = sk::ImageInfo::new(
        (width, height),
        sk::ColorType::RGBA8888,
        sk::AlphaType::Premul,
        None,
    );
    let row_bytes = (width as usize) * 4;
    let Some(image) =
        sk::images::raster_from_data(&info, sk::Data::new_copy(&content_bytes), row_bytes)
    else {
        state.set_error_once(Error::Internal("create masked layer image"));
        return;
    };

    let mut paint = sk::Paint::default();
    paint.set_anti_alias(true);
    paint.set_blend_mode(map_blend_mode(&masked.composite.blend));
    paint.set_alpha_f(masked.composite.alpha);
    if !masked.filters.is_empty() {
        if let Some(filter) = build_filter_chain(&masked.filters) {
            paint.set_image_filter(filter);
        } else {
            state.set_error_once(Error::UnsupportedFilter);
            return;
        }
    }

    let clip_path = masked
        .clip
        .as_ref()
        .and_then(|clip| clip_path(canvas, state, clip.as_ref()));
    if let Some(path) = clip_path.as_ref() {
        canvas.save();
        canvas.clip_path(path, None, true);
    }

    set_matrix(canvas, Affine::IDENTITY);
    canvas.draw_image(&image, (0, 0), Some(&paint));

    if clip_path.is_some() {
        canvas.restore();
    }
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
                composite: group.composite,
                mode: mask.mask.mode,
                transform: mask.transform,
                mask: mask.mask.scene.clone(),
                content: record::Scene::new(),
                nested_group_depth: 0,
            })));
        return;
    }
    let restores = push_group_impl(canvas, state, group);
    state.group_stack.push(GroupFrame::Direct { restores });
}

fn paint_sink_pop_group(
    canvas: &sk::Canvas,
    state: &mut StreamState,
    image_cache: Option<&Rc<RefCell<ImageCache>>>,
    mask_cache: Option<&Rc<RefCell<MaskCache>>>,
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
        GroupFrame::Direct { restores } => {
            for _ in 0..restores {
                canvas.restore();
            }
        }
        GroupFrame::Masked(mut frame) => {
            if frame.nested_group_depth != 0 {
                frame.content.pop_group();
                frame.nested_group_depth -= 1;
                state.group_stack.push(GroupFrame::Masked(frame));
                return;
            }
            draw_masked_group(canvas, state, *frame, image_cache, mask_cache, font_cache);
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
    mask_cache: Option<Rc<RefCell<MaskCache>>>,
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
            mask_cache: None,
            font_cache: None,
            state: StreamState::new(),
        }
    }

    pub(crate) fn new_with_caches(
        canvas: &'a sk::Canvas,
        image_cache: Option<Rc<RefCell<ImageCache>>>,
        mask_cache: Rc<RefCell<MaskCache>>,
        font_cache: SkiaFontCache,
    ) -> Self {
        Self {
            canvas,
            image_cache,
            mask_cache: Some(mask_cache),
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
            self.mask_cache.as_ref(),
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
            None,
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

    fn scene_with_offset(offset: f64) -> record::Scene {
        let mut scene = record::Scene::new();
        scene.fill(FillRef::new(
            Rect::new(offset, offset, offset + 4.0, offset + 4.0),
            &Brush::Solid(Color::WHITE),
        ));
        scene
    }

    #[test]
    fn mask_cache_evicts_oldest_entries_to_stay_within_budget() {
        let mut cache = MaskCache::new(8);
        let first = scene_with_offset(0.0);
        let second = scene_with_offset(8.0);

        cache.insert(
            &first,
            MaskMode::Alpha,
            Affine::IDENTITY,
            1,
            1,
            0.1,
            &[1, 2, 3, 4],
        );
        assert_eq!(cache.len(), 1);

        cache.insert(
            &second,
            MaskMode::Alpha,
            Affine::IDENTITY,
            1,
            1,
            0.1,
            &[5, 6, 7, 8, 9, 10],
        );

        assert_eq!(cache.len(), 1);
        assert!(
            cache
                .get(&first, MaskMode::Alpha, Affine::IDENTITY, 1, 1, 0.1)
                .is_none()
        );
        assert_eq!(
            cache.get(&second, MaskMode::Alpha, Affine::IDENTITY, 1, 1, 0.1),
            Some(vec![5, 6, 7, 8, 9, 10])
        );
    }

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
