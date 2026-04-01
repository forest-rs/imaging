// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::{
    Error, affine_to_matrix, apply_stroke_style, bez_to_sk_path, brush_to_paint,
    build_filter_chain, f64_to_f32, geometry_to_bez_path, geometry_to_sk_path, map_blend_mode,
    path_with_fill_rule, skia_font_from_glyph_run,
};
use imaging::{
    BlurredRoundedRect, ClipRef, FillRef, GeometryRef, GlyphRunRef, GroupRef, MaskMode, PaintSink,
    StrokeRef,
    record::{self, replay, replay_transformed},
};
use kurbo::{Affine, Rect, Shape as _};
use skia_safe as sk;

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
    glyph_run: GlyphRunRef<'_>,
    glyphs: &mut dyn Iterator<Item = record::Glyph>,
) {
    if !glyph_run.normalized_coords.is_empty() {
        state.set_error_once(Error::UnsupportedGlyphVariations);
        return;
    }

    let Some(mut font) = skia_font_from_glyph_run(&glyph_run) else {
        state.set_error_once(Error::InvalidFontData);
        return;
    };

    set_matrix(canvas, glyph_run.transform);

    let Some(mut sk_paint) =
        brush_to_paint(glyph_run.brush, glyph_run.composite.alpha, Affine::IDENTITY)
    else {
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

fn draw_masked_group(canvas: &sk::Canvas, state: &mut StreamState, masked: MaskedGroupFrame) {
    let mut group_paint = sk::Paint::default();
    group_paint.set_anti_alias(true);
    group_paint.set_blend_mode(map_blend_mode(&masked.composite.blend));
    group_paint.set_alpha_f(masked.composite.alpha);
    if !masked.filters.is_empty() {
        if let Some(filter) = build_filter_chain(&masked.filters) {
            group_paint.set_image_filter(filter);
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
    canvas.save_layer(&sk::canvas::SaveLayerRec::default().paint(&group_paint));

    {
        let mut sink = SkCanvasSink::new(canvas);
        sink.set_tolerance(state.tolerance);
        replay(&masked.content, &mut sink);
        if let Err(err) = sink.finish() {
            state.set_error_once(err);
            canvas.restore();
            if clip_path.is_some() {
                canvas.restore();
            }
            return;
        }
    }

    let mut mask_paint = sk::Paint::default();
    mask_paint.set_anti_alias(true);
    mask_paint.set_blend_mode(sk::BlendMode::DstIn);
    if masked.mode == MaskMode::Luminance {
        mask_paint.set_color_filter(sk::ColorFilter::luma());
    }
    canvas.save_layer(&sk::canvas::SaveLayerRec::default().paint(&mask_paint));
    set_matrix(canvas, Affine::IDENTITY);

    {
        let mut sink = SkCanvasSink::new(canvas);
        sink.set_tolerance(state.tolerance);
        replay_transformed(&masked.mask, &mut sink, masked.transform);
        if let Err(err) = sink.finish() {
            state.set_error_once(err);
            canvas.restore();
            canvas.restore();
            if clip_path.is_some() {
                canvas.restore();
            }
            return;
        }
    }

    canvas.restore();
    canvas.restore();

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

fn paint_sink_pop_group(canvas: &sk::Canvas, state: &mut StreamState) {
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
            draw_masked_group(canvas, state, *frame);
        }
    }
}

fn paint_sink_fill(canvas: &sk::Canvas, state: &mut StreamState, draw: FillRef<'_>) {
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

fn paint_sink_stroke(canvas: &sk::Canvas, state: &mut StreamState, draw: StrokeRef<'_>) {
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
        paint_sink_pop_group(self.canvas, &mut self.state);
    }

    fn fill(&mut self, draw: FillRef<'_>) {
        paint_sink_fill(self.canvas, &mut self.state, draw);
    }

    fn stroke(&mut self, draw: StrokeRef<'_>) {
        paint_sink_stroke(self.canvas, &mut self.state, draw);
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
        draw_glyph_run(self.canvas, &mut self.state, draw, glyphs);
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

    /// Start recording a Skia picture with optional bounding-box hierarchy acceleration.
    pub fn new_with_bbh(bounds: Rect, use_bbh: bool) -> Self {
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
        paint_sink_pop_group(canvas, state);
    }

    fn fill(&mut self, draw: FillRef<'_>) {
        let recorder = &mut self.recorder;
        let state = &mut self.state;
        let Some(canvas) = recorder.recording_canvas() else {
            state.set_error_once(Error::Internal("picture recorder not recording"));
            return;
        };
        paint_sink_fill(canvas, state, draw);
    }

    fn stroke(&mut self, draw: StrokeRef<'_>) {
        let recorder = &mut self.recorder;
        let state = &mut self.state;
        let Some(canvas) = recorder.recording_canvas() else {
            state.set_error_once(Error::Internal("picture recorder not recording"));
            return;
        };
        paint_sink_stroke(canvas, state, draw);
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
        draw_glyph_run(canvas, state, draw, glyphs);
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
