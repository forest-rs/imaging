// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Portions of this file are derived from the Floem tiny-skia renderer,
// from the Floem project at https://github.com/lapce/floem, under the MIT license.

//! tiny-skia backend for `imaging`.
//!
//! This crate provides a CPU renderer that consumes `imaging::record::Scene` values or streaming
//! `imaging::PaintSink` commands and produces RGBA8 image buffers using `tiny-skia`.
//!
//! The implementation was integrated from Floem's tiny-skia renderer and adapted to match the
//! public renderer shape used by the other `imaging_*` backends.

#![deny(unsafe_code)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use imaging::{
    BlurredRoundedRect, ClipRef, FillRef, GlyphRunRef, GroupRef, MaskMode, PaintSink, RgbaImage,
    StrokeRef,
    record::{Scene, ValidateError},
    render::{ImageRenderer, RenderSource},
};
#[cfg(test)]
use kurbo::Vec2;
use kurbo::{Affine, BezPath, Cap, Join, Point, Rect, Shape, Stroke as KurboStroke};
use peniko::{
    BlendMode, BrushRef, Color, Compose, Extend, Gradient, GradientKind, ImageData, ImageQuality,
    Mix, RadialGradientPosition,
    color::{self, ColorSpaceTag, DynamicColor, HueDirection, Srgb},
    kurbo::{PathEl, Size},
};
use rustc_hash::FxHashMap;
use std::{
    borrow::Borrow,
    cell::RefCell,
    sync::Arc,
    time::{Duration, Instant},
};
use swash::{
    FontRef,
    scale::{Render, ScaleContext, Source, StrikeWith, image::Content},
    zeno::Format,
};
use tiny_skia::{
    self, FillRule, FilterQuality, GradientStop, IntRect, LineCap, LineJoin, LinearGradient, Mask,
    MaskType, Paint, Path, PathBuilder, Pattern, Pixmap, PixmapMut, PixmapPaint, PixmapRef,
    PremultipliedColorU8, RadialGradient, Shader, SpreadMode, Stroke as TinyStroke, StrokeDash,
    Transform,
};

type Result<T, E = Error> = core::result::Result<T, E>;

/// Errors that can occur when rendering via tiny-skia.
#[derive(Debug)]
pub enum Error {
    /// The scene is invalid (unbalanced stacks).
    InvalidScene(ValidateError),
    /// The caller-provided buffer format is not supported by this renderer.
    UnsupportedTargetFormat,
    /// The caller-provided buffer shape is not large enough for the target dimensions.
    InvalidTargetBuffer,
    /// An internal invariant was violated.
    Internal(&'static str),
}

/// Byte channel ordering for caller-owned CPU targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuBufferChannelOrder {
    /// RGBA8 byte order.
    Rgba8,
    /// BGRA8 byte order.
    Bgra8,
}

/// Alpha encoding for caller-owned CPU targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuBufferAlphaMode {
    /// Output alpha is forced opaque.
    Opaque,
    /// Output bytes retain premultiplied alpha.
    Premultiplied,
}

/// Pixel format description for caller-owned CPU targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuBufferFormat {
    /// Byte channel ordering.
    pub channel_order: CpuBufferChannelOrder,
    /// Alpha encoding.
    pub alpha_mode: CpuBufferAlphaMode,
}

impl CpuBufferFormat {
    /// Packed opaque `RGBA8`.
    pub const RGBA8_OPAQUE: Self = Self {
        channel_order: CpuBufferChannelOrder::Rgba8,
        alpha_mode: CpuBufferAlphaMode::Opaque,
    };

    /// Packed opaque `BGRA8`.
    pub const BGRA8_OPAQUE: Self = Self {
        channel_order: CpuBufferChannelOrder::Bgra8,
        alpha_mode: CpuBufferAlphaMode::Opaque,
    };
}

/// Metadata used to validate whether a caller-owned CPU target is supported.
#[derive(Clone, Copy, Debug)]
pub struct CpuBufferTargetInfo {
    /// Target width in pixels.
    pub width: u32,
    /// Target height in pixels.
    pub height: u32,
    /// Row stride in bytes.
    pub bytes_per_row: usize,
    /// Target pixel format.
    pub format: CpuBufferFormat,
}

/// Borrowed caller-owned CPU pixel target.
#[derive(Debug)]
pub struct CpuBufferTarget<'a> {
    /// Pixel storage.
    pub buffer: &'a mut [u8],
    /// Target width in pixels.
    pub width: u32,
    /// Target height in pixels.
    pub height: u32,
    /// Row stride in bytes.
    pub bytes_per_row: usize,
    /// Target pixel format.
    pub format: CpuBufferFormat,
}

/// Cache key for rasterized glyphs, replacing cosmic-text's `CacheKey`.
/// Uses Parley's font blob identity + swash-compatible glyph parameters.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphCacheKey {
    font_blob_id: u64,
    font_index: u32,
    glyph_id: u16,
    font_size_bits: u32,
    x_bin: u8,
    y_bin: u8,
    hint: bool,
    embolden: bool,
    skew_bits: u32,
}

struct GlyphKeyInput {
    font_blob_id: u64,
    font_index: u32,
    glyph_id: u16,
    font_size: f32,
    x: f32,
    y: f32,
    hint: bool,
    embolden: bool,
    skew: Option<f32>,
}

impl GlyphCacheKey {
    fn new(input: GlyphKeyInput) -> (Self, f32, f32) {
        let font_size_bits = input.font_size.to_bits();
        let x_floor = input.x.floor();
        let y_floor = input.y.floor();
        let x_fract = input.x - x_floor;
        let y_fract = input.y - y_floor;
        // 4 subpixel bins per axis (matching old SubpixelBin behavior)
        let x_bin = match u8::try_from(f32_to_i32((x_fract * 4.0).min(3.0).round())) {
            Ok(value) => value,
            Err(_) => panic!("x subpixel bin must fit in u8"),
        };
        let y_bin = match u8::try_from(f32_to_i32((y_fract * 4.0).min(3.0).round())) {
            Ok(value) => value,
            Err(_) => panic!("y subpixel bin must fit in u8"),
        };
        let skew_bits = input.skew.unwrap_or(0.0).to_bits();

        (
            Self {
                font_blob_id: input.font_blob_id,
                font_index: input.font_index,
                glyph_id: input.glyph_id,
                font_size_bits,
                x_bin,
                y_bin,
                hint: input.hint,
                embolden: input.embolden,
                skew_bits,
            },
            x_floor + f32::from(x_bin) / 4.0,
            y_floor + f32::from(y_bin) / 4.0,
        )
    }
}

type ImageCacheMap = FxHashMap<Vec<u8>, (CacheColor, Arc<Pixmap>)>;
type ScaledImageCacheMap = FxHashMap<ScaledImageCacheKey, (CacheColor, Arc<Pixmap>)>;
type GlyphCacheMap = FxHashMap<(GlyphCacheKey, u32), GlyphCacheEntry>;

thread_local! {
    static IMAGE_CACHE: RefCell<ImageCacheMap> = RefCell::new(FxHashMap::default());
    static SCALED_IMAGE_CACHE: RefCell<ScaledImageCacheMap> = RefCell::new(FxHashMap::default());
    // The `u32` is a color encoded as a u32 so that it is hashable and eq.
    static GLYPH_CACHE: RefCell<GlyphCacheMap> = RefCell::new(FxHashMap::default());
    static SCALE_CONTEXT: RefCell<ScaleContext> = RefCell::new(ScaleContext::new());
}

const GLYPH_FILTER_PAD: u32 = 1;

struct GlyphRasterRequest<'a> {
    cache_color: CacheColor,
    cache_key: GlyphCacheKey,
    color: Color,
    font_ref: FontRef<'a>,
    font_size: f32,
    hint: bool,
    normalized_coords: &'a [i16],
    embolden_strength: f32,
    skew: Option<f32>,
    offset_x: f32,
    offset_y: f32,
}

fn cache_glyph(request: GlyphRasterRequest<'_>) -> Option<Arc<Glyph>> {
    let c = request.color.to_rgba8();
    let now = Instant::now();

    if let Some(opt_glyph) = GLYPH_CACHE.with_borrow_mut(|gc| {
        if let Some(entry) = gc.get_mut(&(request.cache_key, c.to_u32())) {
            entry.cache_color = request.cache_color;
            entry.last_touched = now;
            Some(entry.glyph.clone())
        } else {
            None
        }
    }) {
        return opt_glyph;
    };

    let image = SCALE_CONTEXT.with_borrow_mut(|context| {
        let mut scaler = context
            .builder(request.font_ref)
            .size(request.font_size)
            .hint(request.hint)
            .normalized_coords(request.normalized_coords)
            .build();

        let mut render = Render::new(&[
            Source::ColorOutline(0),
            Source::ColorBitmap(StrikeWith::BestFit),
            Source::Outline,
        ]);
        render
            .format(Format::Alpha)
            .offset(swash::zeno::Vector::new(
                request.offset_x.fract(),
                request.offset_y.fract(),
            ))
            .embolden(request.embolden_strength);
        if let Some(angle) = request.skew {
            render.transform(Some(swash::zeno::Transform::skew(
                swash::zeno::Angle::from_degrees(angle),
                swash::zeno::Angle::ZERO,
            )));
        }
        render.render(&mut scaler, request.cache_key.glyph_id)
    })?;

    let result = if image.placement.width == 0 || image.placement.height == 0 {
        // We can't create an empty `Pixmap`
        None
    } else {
        let pad = GLYPH_FILTER_PAD;
        let pad_usize = pad as usize;
        let padded_width = image.placement.width.checked_add(pad * 2)?;
        let padded_height = image.placement.height.checked_add(pad * 2)?;
        let mut pixmap = Pixmap::new(padded_width, padded_height)?;

        match image.content {
            Content::Mask => {
                let width = image.placement.width as usize;
                let padded_width = padded_width as usize;
                let pixels = pixmap.pixels_mut();
                for (row_idx, row) in image.data.chunks_exact(width).enumerate() {
                    let dst_row = (row_idx + pad_usize) * padded_width + pad_usize;
                    for (col_idx, &alpha) in row.iter().enumerate() {
                        pixels[dst_row + col_idx] =
                            tiny_skia::Color::from_rgba8(c.r, c.g, c.b, alpha)
                                .premultiply()
                                .to_color_u8();
                    }
                }
            }
            Content::Color => {
                let width = image.placement.width as usize;
                let padded_width = padded_width as usize;
                let pixels = pixmap.pixels_mut();
                for (row_idx, row) in image.data.chunks_exact(width * 4).enumerate() {
                    let dst_row = (row_idx + pad_usize) * padded_width + pad_usize;
                    for (col_idx, b) in row.chunks_exact(4).enumerate() {
                        pixels[dst_row + col_idx] =
                            tiny_skia::Color::from_rgba8(b[0], b[1], b[2], b[3])
                                .premultiply()
                                .to_color_u8();
                    }
                }
            }
            _ => return None,
        }

        Some(Arc::new(Glyph {
            pixmap: Arc::new(pixmap),
            left: image.placement.left as f32 - pad as f32,
            top: image.placement.top as f32 + pad as f32,
        }))
    };

    GLYPH_CACHE.with_borrow_mut(|gc| {
        gc.insert(
            (request.cache_key, c.to_u32()),
            GlyphCacheEntry {
                cache_color: request.cache_color,
                glyph: result.clone(),
                last_touched: now,
            },
        )
    });

    result
}

macro_rules! try_ret {
    ($e:expr) => {
        if let Some(e) = $e {
            e
        } else {
            return;
        }
    };
}

struct Glyph {
    pixmap: Arc<Pixmap>,
    left: f32,
    top: f32,
}

#[derive(Clone)]
pub(crate) struct ClipPath {
    path: Path,
    rect: Rect,
    simple_rect: Option<Rect>,
}

#[derive(PartialEq, Clone, Copy)]
struct CacheColor(bool);

const GLYPH_CACHE_MIN_TTL: Duration = Duration::from_millis(100);

struct GlyphCacheEntry {
    cache_color: CacheColor,
    glyph: Option<Arc<Glyph>>,
    last_touched: Instant,
}

fn should_retain_glyph_entry(
    entry: &GlyphCacheEntry,
    cache_color: CacheColor,
    now: Instant,
) -> bool {
    entry.cache_color == cache_color || now.duration_since(entry.last_touched) < GLYPH_CACHE_MIN_TTL
}

#[derive(Hash, PartialEq, Eq)]
struct ScaledImageCacheKey {
    image_id: u64,
    width: u32,
    height: u32,
    quality: u8,
}

enum LayerPixmap<'a> {
    Owned(Pixmap),
    Borrowed(PixmapMut<'a>),
}

impl LayerPixmap<'_> {
    fn as_ref(&self) -> PixmapRef<'_> {
        match self {
            Self::Owned(pixmap) => pixmap.as_ref(),
            Self::Borrowed(pixmap) => pixmap.as_ref(),
        }
    }

    fn width(&self) -> u32 {
        match self {
            Self::Owned(pixmap) => pixmap.width(),
            Self::Borrowed(pixmap) => pixmap.width(),
        }
    }

    fn height(&self) -> u32 {
        match self {
            Self::Owned(pixmap) => pixmap.height(),
            Self::Borrowed(pixmap) => pixmap.height(),
        }
    }

    fn fill(&mut self, color: tiny_skia::Color) {
        match self {
            Self::Owned(pixmap) => pixmap.fill(color),
            Self::Borrowed(pixmap) => pixmap.fill(color),
        }
    }

    fn pixels_mut(&mut self) -> &mut [PremultipliedColorU8] {
        match self {
            Self::Owned(pixmap) => pixmap.pixels_mut(),
            Self::Borrowed(pixmap) => pixmap.pixels_mut(),
        }
    }

    fn data(&self) -> &[u8] {
        match self {
            Self::Owned(pixmap) => pixmap.data(),
            Self::Borrowed(pixmap) => pixmap.as_ref().data(),
        }
    }

    fn data_mut(&mut self) -> &mut [u8] {
        match self {
            Self::Owned(pixmap) => pixmap.data_mut(),
            Self::Borrowed(pixmap) => pixmap.data_mut(),
        }
    }

    fn fill_rect(
        &mut self,
        rect: tiny_skia::Rect,
        paint: &Paint<'_>,
        transform: Transform,
        mask: Option<&Mask>,
    ) {
        match self {
            Self::Owned(pixmap) => pixmap.fill_rect(rect, paint, transform, mask),
            Self::Borrowed(pixmap) => pixmap.fill_rect(rect, paint, transform, mask),
        }
    }

    fn fill_path(
        &mut self,
        path: &Path,
        paint: &Paint<'_>,
        fill_rule: FillRule,
        transform: Transform,
        mask: Option<&Mask>,
    ) {
        match self {
            Self::Owned(pixmap) => pixmap.fill_path(path, paint, fill_rule, transform, mask),
            Self::Borrowed(pixmap) => pixmap.fill_path(path, paint, fill_rule, transform, mask),
        }
    }

    fn stroke_path(
        &mut self,
        path: &Path,
        paint: &Paint<'_>,
        stroke: &TinyStroke,
        transform: Transform,
        mask: Option<&Mask>,
    ) {
        match self {
            Self::Owned(pixmap) => pixmap.stroke_path(path, paint, stroke, transform, mask),
            Self::Borrowed(pixmap) => pixmap.stroke_path(path, paint, stroke, transform, mask),
        }
    }

    fn draw_pixmap(
        &mut self,
        x: i32,
        y: i32,
        pixmap: PixmapRef<'_>,
        paint: &PixmapPaint,
        transform: Transform,
        mask: Option<&Mask>,
    ) {
        match self {
            Self::Owned(dst) => dst.draw_pixmap(x, y, pixmap, paint, transform, mask),
            Self::Borrowed(dst) => dst.draw_pixmap(x, y, pixmap, paint, transform, mask),
        }
    }

    fn clone_rect(&self, rect: IntRect) -> Option<Pixmap> {
        match self {
            Self::Owned(pixmap) => pixmap.clone_rect(rect),
            Self::Borrowed(pixmap) => pixmap.as_ref().clone_rect(rect),
        }
    }
}

struct Layer<'a> {
    pixmap: LayerPixmap<'a>,
    base_clip: Option<ClipPath>,
    clip_stack: Vec<ClipPath>,
    /// clip is stored with the transform at the time clip is called
    clip: Option<Rect>,
    simple_clip: Option<Rect>,
    draw_bounds: Option<Rect>,
    mask: Mask,
    mask_valid: bool,
    /// this transform should generally only be used when making a draw call to skia
    transform: Affine,
    blend_mode: BlendMode,
    alpha: f32,
    group_mask: Option<GroupMask>,
}

struct GroupMask {
    pixmap: Pixmap,
    mode: MaskMode,
}
impl Layer<'static> {
    fn new_root(width: u32, height: u32) -> Result<Self> {
        Ok(Self {
            pixmap: LayerPixmap::Owned(
                Pixmap::new(width, height).ok_or(Error::Internal("unable to create pixmap"))?,
            ),
            base_clip: None,
            clip_stack: Vec::new(),
            clip: None,
            simple_clip: None,
            draw_bounds: None,
            mask: Mask::new(width, height).ok_or(Error::Internal("unable to create mask"))?,
            mask_valid: false,
            transform: Affine::IDENTITY,
            blend_mode: Mix::Normal.into(),
            alpha: 1.0,
            group_mask: None,
        })
    }

    fn new_with_base_clip(
        blend_mode: BlendMode,
        alpha: f32,
        clip: ClipPath,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let mut layer = Self {
            pixmap: LayerPixmap::Owned(
                Pixmap::new(width, height).ok_or(Error::Internal("unable to create pixmap"))?,
            ),
            base_clip: Some(clip),
            clip_stack: Vec::new(),
            clip: None,
            simple_clip: None,
            draw_bounds: None,
            mask: Mask::new(width, height).ok_or(Error::Internal("unable to create mask"))?,
            mask_valid: false,
            transform: Affine::IDENTITY,
            blend_mode,
            alpha,
            group_mask: None,
        };
        layer.rebuild_clip_mask();
        Ok(layer)
    }
}

impl<'a> Layer<'a> {
    fn active_mask(&self) -> Option<&Mask> {
        (self.clip.is_some() && self.mask_valid).then_some(&self.mask)
    }

    fn new_root_borrowed(data: &'a mut [u8], width: u32, height: u32) -> Result<Self> {
        Ok(Self {
            pixmap: LayerPixmap::Borrowed(
                PixmapMut::from_bytes(data, width, height)
                    .ok_or(Error::Internal("unable to wrap target pixmap"))?,
            ),
            base_clip: None,
            clip_stack: Vec::new(),
            clip: None,
            simple_clip: None,
            draw_bounds: None,
            mask: Mask::new(width, height).ok_or(Error::Internal("unable to create mask"))?,
            mask_valid: false,
            transform: Affine::IDENTITY,
            blend_mode: Mix::Normal.into(),
            alpha: 1.0,
            group_mask: None,
        })
    }

    fn clip_rect_to_mask_bounds_for(
        mask: &Mask,
        rect: Rect,
    ) -> Option<(usize, usize, usize, usize)> {
        let rect = rect_to_int_rect(rect)?;
        let x0 = rect.x().max(0) as usize;
        let y0 = rect.y().max(0) as usize;
        let x1 = (rect.x() + rect.width() as i32).min(mask.width() as i32) as usize;
        let y1 = (rect.y() + rect.height() as i32).min(mask.height() as i32) as usize;
        (x0 < x1 && y0 < y1).then_some((x0, y0, x1, y1))
    }

    fn write_mask_rect(&mut self, rect: Rect) {
        Self::write_mask_rect_to(&mut self.mask, rect);
    }

    fn write_mask_rect_to(mask: &mut Mask, rect: Rect) {
        let Some((x0, y0, x1, y1)) = Self::clip_rect_to_mask_bounds_for(mask, rect) else {
            return;
        };

        let width = mask.width() as usize;
        let data = mask.data_mut();
        for y in y0..y1 {
            let row = y * width;
            data[row + x0..row + x1].fill(255);
        }
    }

    fn fill_mask_rect(&mut self, rect: Rect) {
        self.mask.clear();
        self.write_mask_rect(rect);
        self.mask_valid = true;
    }

    fn intersect_mask_rect(&mut self, rect: Rect) {
        Self::intersect_mask_rect_in(&mut self.mask, rect);
        self.mask_valid = true;
    }

    fn materialize_simple_clip_mask(&mut self) {
        if self.clip.is_some()
            && !self.mask_valid
            && let Some(simple_clip) = self.simple_clip
        {
            self.fill_mask_rect(simple_clip);
        }
    }

    fn intersect_mask_rect_in(mask: &mut Mask, rect: Rect) {
        let Some((x0, y0, x1, y1)) = Self::clip_rect_to_mask_bounds_for(mask, rect) else {
            mask.clear();
            return;
        };

        let width = mask.width() as usize;
        let height = mask.height() as usize;
        let data = mask.data_mut();
        for y in 0..height {
            let row = y * width;
            if y < y0 || y >= y1 {
                data[row..row + width].fill(0);
                continue;
            }

            data[row..row + x0].fill(0);
            data[row + x1..row + width].fill(0);
        }
    }

    fn intersect_clip_path(&mut self, clip: &ClipPath) {
        let prior_simple_clip = self
            .simple_clip
            .or(self.base_clip.as_ref().and_then(|clip| clip.simple_rect));
        let clip_rect = self
            .clip
            .map(|rect| rect.intersect(clip.rect))
            .unwrap_or(clip.rect);
        if clip_rect.is_zero_area() {
            self.clip = None;
            self.simple_clip = None;
            self.mask.clear();
            self.mask_valid = false;
            return;
        }

        let next_simple_clip = match (prior_simple_clip, clip.simple_rect) {
            (Some(current), Some(next)) => {
                let clipped = current.intersect(next);
                (!clipped.is_zero_area()).then_some(clipped)
            }
            (None, Some(next)) if self.base_clip.is_none() && self.clip.is_none() => Some(next),
            _ => None,
        };

        if let Some(simple_clip) = next_simple_clip {
            self.clip = Some(clip_rect);
            self.simple_clip = Some(simple_clip);
            self.mask.clear();
            self.mask_valid = false;
            return;
        }

        if self.active_mask().is_some() {
            if clip.simple_rect.is_some() {
                self.intersect_mask_rect(clip.rect);
            } else {
                self.mask.intersect_path(
                    &clip.path,
                    FillRule::Winding,
                    false,
                    Transform::identity(),
                );
            }
        } else {
            if let Some(simple_clip) = prior_simple_clip {
                self.fill_mask_rect(simple_clip);
                if clip.simple_rect.is_some() {
                    self.intersect_mask_rect(clip.rect);
                } else {
                    self.mask.intersect_path(
                        &clip.path,
                        FillRule::Winding,
                        false,
                        Transform::identity(),
                    );
                }
            } else {
                self.mask.clear();
                self.mask
                    .fill_path(&clip.path, FillRule::Winding, false, Transform::identity());
            }
        }

        self.clip = Some(clip_rect);
        self.simple_clip = None;
        self.mask_valid = true;
    }

    fn rebuild_clip_mask(&mut self) {
        self.rebuild_clip_mask_with_extra_clips(&[]);
    }

    fn rebuild_clip_mask_with_extra_clips(&mut self, extra_clips: &[ClipPath]) {
        let (clip, simple_clip, needs_mask) = {
            let mut clips = self
                .base_clip
                .iter()
                .chain(self.clip_stack.iter())
                .chain(extra_clips.iter());
            let Some(first) = clips.next() else {
                self.clip = None;
                self.simple_clip = None;
                self.mask.clear();
                self.mask_valid = false;
                return;
            };

            let mut clip_rect = first.rect;
            let mut simple_clip = first.simple_rect;
            let mut needs_mask = first.simple_rect.is_none();

            for clip in clips {
                clip_rect = clip_rect.intersect(clip.rect);
                simple_clip = match (simple_clip, clip.simple_rect) {
                    (Some(current), Some(next)) => {
                        let clipped = current.intersect(next);
                        (!clipped.is_zero_area()).then_some(clipped)
                    }
                    _ => None,
                };
                needs_mask |= clip.simple_rect.is_none();
            }

            (
                (!clip_rect.is_zero_area()).then_some(clip_rect),
                simple_clip,
                needs_mask,
            )
        };

        self.clip = clip;
        self.simple_clip = self.clip.and(simple_clip);
        if self.clip.is_none() {
            self.mask.clear();
            self.mask_valid = false;
            return;
        }
        if !needs_mask {
            self.mask.clear();
            self.mask_valid = false;
            return;
        }

        let mut clips = self
            .base_clip
            .iter()
            .chain(self.clip_stack.iter())
            .chain(extra_clips.iter());
        let first = clips.next().expect("checked clip presence");
        self.mask.clear();
        let mask = &mut self.mask;
        if let Some(simple_clip) = self.simple_clip {
            Self::write_mask_rect_to(mask, simple_clip);
        } else if first.simple_rect.is_some() {
            Self::write_mask_rect_to(mask, first.rect);
        } else {
            mask.fill_path(&first.path, FillRule::Winding, false, Transform::identity());
        }

        for clip in clips {
            if clip.simple_rect.is_some() {
                Self::intersect_mask_rect_in(mask, clip.rect);
            } else {
                mask.intersect_path(&clip.path, FillRule::Winding, false, Transform::identity());
            }
        }
        self.mask_valid = true;
    }

    #[cfg(test)]
    fn set_base_clip(&mut self, clip: Option<ClipPath>) {
        self.base_clip = clip;
        self.rebuild_clip_mask();
    }

    #[cfg(test)]
    fn clip_mask_is_empty(&self) -> bool {
        self.mask.data().iter().all(|&value| value == 0)
    }

    fn effective_clips(&self) -> Vec<ClipPath> {
        self.base_clip
            .iter()
            .cloned()
            .chain(self.clip_stack.iter().cloned())
            .collect()
    }

    fn mark_drawn_device_rect(&mut self, rect: Rect) {
        let mut device_rect = rect;
        if let Some(clip) = self.clip {
            device_rect = device_rect.intersect(clip);
        }

        if device_rect.is_zero_area() {
            return;
        }

        self.draw_bounds = Some(
            self.draw_bounds
                .map(|bounds| bounds.union(device_rect))
                .unwrap_or(device_rect),
        );
    }

    fn try_fill_solid_rect_fast(&mut self, rect: Rect, color: Color) -> bool {
        if self.active_mask().is_some() {
            return false;
        }

        let coeffs = self.device_transform().as_coeffs();
        if coeffs[0] != 1.0 || coeffs[1] != 0.0 || coeffs[2] != 0.0 || coeffs[3] != 1.0 {
            return false;
        }

        let c = color.to_rgba8();
        if c.a != 255 {
            return false;
        }

        let Some(device_rect) = rect_to_int_rect(self.device_transform().transform_rect_bbox(rect))
        else {
            return false;
        };

        let width = match i32::try_from(device_rect.width()) {
            Ok(value) => value,
            Err(_) => return false,
        };
        let height = match i32::try_from(device_rect.height()) {
            Ok(value) => value,
            Err(_) => return false,
        };
        let mut device_rect = Rect::new(
            f64::from(device_rect.x()),
            f64::from(device_rect.y()),
            f64::from(device_rect.x() + width),
            f64::from(device_rect.y() + height),
        );
        if let Some(simple_clip) = self.simple_clip {
            device_rect = device_rect.intersect(simple_clip);
            if device_rect.is_zero_area() {
                return true;
            }
        }

        let x0 = f64_to_u32(device_rect.x0.max(0.0));
        let y0 = f64_to_u32(device_rect.y0.max(0.0));
        let x1 = f64_to_u32(device_rect.x1.min(f64::from(self.pixmap.width())));
        let y1 = f64_to_u32(device_rect.y1.min(f64::from(self.pixmap.height())));

        if x0 >= x1 || y0 >= y1 {
            return true;
        }

        self.mark_drawn_device_rect(Rect::new(
            f64::from(x0),
            f64::from(y0),
            f64::from(x1),
            f64::from(y1),
        ));

        let fill = tiny_skia::Color::from_rgba8(c.r, c.g, c.b, c.a)
            .premultiply()
            .to_color_u8();
        let width = match usize::try_from(self.pixmap.width()) {
            Ok(value) => value,
            Err(_) => return false,
        };
        let pixels = self.pixmap.pixels_mut();
        let x0 = match usize::try_from(x0) {
            Ok(value) => value,
            Err(_) => return false,
        };
        let y0 = match usize::try_from(y0) {
            Ok(value) => value,
            Err(_) => return false,
        };
        let x1 = match usize::try_from(x1) {
            Ok(value) => value,
            Err(_) => return false,
        };
        let y1 = match usize::try_from(y1) {
            Ok(value) => value,
            Err(_) => return false,
        };
        for y in y0..y1 {
            let start = y * width + x0;
            let end = y * width + x1;
            pixels[start..end].fill(fill);
        }

        true
    }

    fn device_transform(&self) -> Affine {
        self.transform
    }

    fn intersects_clip(&self, img_rect: Rect, transform: Affine) -> bool {
        let device_rect = transform.transform_rect_bbox(img_rect);
        self.clip
            .map(|clip| to_skia_rect(clip.intersect(device_rect)).is_some())
            .unwrap_or(true)
    }

    fn mark_drawn_rect_inflated(&mut self, rect: Rect, transform: Affine, pad: f64) {
        self.mark_drawn_device_rect(transform.transform_rect_bbox(rect).inset(-pad));
    }

    fn mark_stroke_bounds(&mut self, shape: &impl Shape, stroke: &KurboStroke) {
        if let Some(clip) = self.clip {
            self.mark_drawn_device_rect(clip);
            return;
        }

        let stroke_pad = stroke.width + stroke.miter_limit.max(1.0) + 4.0;
        self.mark_drawn_rect_inflated(
            shape.bounding_box().inset(-stroke_pad),
            self.device_transform(),
            4.0,
        );
    }

    fn try_draw_pixmap_translate_only(
        &mut self,
        pixmap: &Pixmap,
        x: f64,
        y: f64,
        transform: Affine,
        quality: FilterQuality,
    ) -> bool {
        let Some((draw_x, draw_y)) = integer_translation(transform, x, y) else {
            return false;
        };

        let rect = Rect::from_origin_size(
            (f64::from(draw_x), f64::from(draw_y)),
            (f64::from(pixmap.width()), f64::from(pixmap.height())),
        );
        if !self.intersects_clip(rect, Affine::IDENTITY) {
            return true;
        }

        self.mark_drawn_rect_inflated(rect, Affine::IDENTITY, 2.0);
        if quality == FilterQuality::Nearest && self.blit_pixmap_source_over(pixmap, draw_x, draw_y)
        {
            return true;
        }

        let paint = PixmapPaint {
            opacity: 1.0,
            blend_mode: tiny_skia::BlendMode::SourceOver,
            quality,
        };
        self.materialize_simple_clip_mask();
        let clip_mask = self.clip.is_some().then_some(&self.mask);
        self.pixmap.draw_pixmap(
            draw_x,
            draw_y,
            pixmap.as_ref(),
            &paint,
            Transform::identity(),
            clip_mask,
        );
        true
    }

    fn blit_pixmap_source_over(&mut self, pixmap: &Pixmap, draw_x: i32, draw_y: i32) -> bool {
        let Some((x0, y0, x1, y1)) = self.blit_bounds(pixmap, draw_x, draw_y) else {
            return true;
        };

        let src_width = match usize::try_from(pixmap.width()) {
            Ok(value) => value,
            Err(_) => return false,
        };
        let dst_width = match usize::try_from(self.pixmap.width()) {
            Ok(value) => value,
            Err(_) => return false,
        };
        let mask_width = match usize::try_from(self.mask.width()) {
            Ok(value) => value,
            Err(_) => return false,
        };
        let src_pixels = pixmap.pixels();
        let use_mask = self.clip.is_some() && self.simple_clip.is_none();
        let mask = use_mask.then_some(self.mask.data());
        let dst_pixels = self.pixmap.pixels_mut();

        let x0 = match usize::try_from(x0) {
            Ok(value) => value,
            Err(_) => return false,
        };
        let y0 = match usize::try_from(y0) {
            Ok(value) => value,
            Err(_) => return false,
        };
        let x1 = match usize::try_from(x1) {
            Ok(value) => value,
            Err(_) => return false,
        };
        let y1 = match usize::try_from(y1) {
            Ok(value) => value,
            Err(_) => return false,
        };

        for dst_y in y0..y1 {
            let dst_y_i32 = match i32::try_from(dst_y) {
                Ok(value) => value,
                Err(_) => return false,
            };
            let src_y = match usize::try_from(dst_y_i32.saturating_sub(draw_y)) {
                Ok(value) => value,
                Err(_) => return false,
            };
            let dst_row = dst_y * dst_width;
            let src_row = src_y * src_width;
            let mask_row = dst_y * mask_width;

            for dst_x in x0..x1 {
                let dst_x_i32 = match i32::try_from(dst_x) {
                    Ok(value) => value,
                    Err(_) => return false,
                };
                let src_x = match usize::try_from(dst_x_i32.saturating_sub(draw_x)) {
                    Ok(value) => value,
                    Err(_) => return false,
                };
                let src = src_pixels[src_row + src_x];
                let coverage = mask.map_or(255, |mask| mask[mask_row + dst_x]);
                if coverage == 0 || src.alpha() == 0 {
                    continue;
                }

                let src = scale_premultiplied_color(src, coverage);
                let dst = dst_pixels[dst_row + dst_x];
                dst_pixels[dst_row + dst_x] = blend_source_over(src, dst);
            }
        }

        true
    }

    fn blit_bounds(
        &self,
        pixmap: &Pixmap,
        draw_x: i32,
        draw_y: i32,
    ) -> Option<(i32, i32, i32, i32)> {
        let mut x0 = draw_x.max(0);
        let mut y0 = draw_y.max(0);
        let pixmap_width = i32::try_from(pixmap.width()).ok()?;
        let pixmap_height = i32::try_from(pixmap.height()).ok()?;
        let target_width = i32::try_from(self.pixmap.width()).ok()?;
        let target_height = i32::try_from(self.pixmap.height()).ok()?;
        let mut x1 = draw_x.saturating_add(pixmap_width).min(target_width);
        let mut y1 = draw_y.saturating_add(pixmap_height).min(target_height);

        if let Some(simple_clip) = self.simple_clip {
            let clip_rect = rect_to_int_rect(simple_clip)?;
            x0 = x0.max(clip_rect.x());
            y0 = y0.max(clip_rect.y());
            x1 = x1.min(clip_rect.x() + i32::try_from(clip_rect.width()).ok()?);
            y1 = y1.min(clip_rect.y() + i32::try_from(clip_rect.height()).ok()?);
        }

        (x0 < x1 && y0 < y1).then_some((x0, y0, x1, y1))
    }

    fn try_fill_rect_with_paint_fast(
        &mut self,
        rect: Rect,
        paint: &Paint<'_>,
        brush_transform: Option<Affine>,
    ) -> bool {
        if !is_axis_aligned(self.device_transform()) {
            return false;
        }

        let Some(device_rect) = to_skia_rect(self.device_transform().transform_rect_bbox(rect))
        else {
            return false;
        };

        let mut paint = paint.clone();
        paint.shader.transform(affine_to_skia(
            self.device_transform() * brush_transform.unwrap_or(Affine::IDENTITY),
        ));
        self.materialize_simple_clip_mask();
        let clip_mask = self.clip.is_some().then_some(&self.mask);
        self.pixmap
            .fill_rect(device_rect, &paint, Transform::identity(), clip_mask);
        true
    }

    /// Renders the pixmap at the position and transforms it with the given transform.
    /// x and y should have already been scaled by the window scale
    fn render_pixmap_direct(
        &mut self,
        img_pixmap: &Pixmap,
        x: f32,
        y: f32,
        transform: Affine,
        quality: FilterQuality,
    ) {
        if self.try_draw_pixmap_translate_only(img_pixmap, x as f64, y as f64, transform, quality) {
            return;
        }

        let img_rect = Rect::from_origin_size(
            (x, y),
            (img_pixmap.width() as f64, img_pixmap.height() as f64),
        );
        if !self.intersects_clip(img_rect, transform) {
            return;
        }
        self.mark_drawn_rect_inflated(img_rect, transform, 2.0);
        let paint = PixmapPaint {
            opacity: 1.0,
            blend_mode: tiny_skia::BlendMode::SourceOver,
            quality,
        };
        let transform = affine_to_skia(transform * Affine::translate((x as f64, y as f64)));
        self.materialize_simple_clip_mask();
        let clip_mask = self.clip.is_some().then_some(&self.mask);
        self.pixmap
            .draw_pixmap(0, 0, img_pixmap.as_ref(), &paint, transform, clip_mask);
    }

    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "This helper is exercised by tests and kept for future fast paths"
        )
    )]
    fn render_pixmap_rect(
        &mut self,
        pixmap: &Pixmap,
        rect: Rect,
        transform: Affine,
        quality: ImageQuality,
    ) {
        let filter_quality = image_quality_to_filter_quality(quality);
        let local_transform = Affine::translate((rect.x0, rect.y0)).then_scale_non_uniform(
            rect.width() / pixmap.width() as f64,
            rect.height() / pixmap.height() as f64,
        );
        let composite_transform = transform * local_transform;

        if self.try_draw_pixmap_translate_only(
            pixmap,
            0.0,
            0.0,
            composite_transform,
            filter_quality,
        ) {
            return;
        }

        if !self.intersects_clip(rect, transform) {
            return;
        }
        self.mark_drawn_rect_inflated(rect, transform, 2.0);
        let paint = PixmapPaint {
            opacity: 1.0,
            blend_mode: tiny_skia::BlendMode::SourceOver,
            quality: filter_quality,
        };

        self.materialize_simple_clip_mask();
        let clip_mask = self.clip.is_some().then_some(&self.mask);
        self.pixmap.draw_pixmap(
            0,
            0,
            pixmap.as_ref(),
            &paint,
            affine_to_skia(composite_transform),
            clip_mask,
        );
    }

    fn skia_transform(&self) -> Transform {
        skia_transform(self.device_transform())
    }
}
impl Layer<'_> {
    #[cfg(test)]
    fn clip(&mut self, shape: &impl Shape) {
        let path =
            try_ret!(shape_to_path(shape).and_then(|path| path.transform(self.skia_transform())));
        self.set_base_clip(Some(ClipPath {
            path,
            rect: self
                .device_transform()
                .transform_rect_bbox(shape.bounding_box()),
            simple_rect: transformed_axis_aligned_rect(shape, self.device_transform()),
        }));
    }
    fn stroke_with_brush_transform<'b, 's>(
        &mut self,
        shape: &impl Shape,
        brush: impl Into<BrushRef<'b>>,
        stroke: &'s KurboStroke,
        brush_transform: Option<Affine>,
    ) {
        let path = try_ret!(shape_to_path(shape));
        let paint = try_ret!(brush_to_paint(brush, brush_transform));
        self.mark_stroke_bounds(shape, stroke);
        let line_cap = match stroke.end_cap {
            Cap::Butt => LineCap::Butt,
            Cap::Square => LineCap::Square,
            Cap::Round => LineCap::Round,
        };
        let line_join = match stroke.join {
            Join::Bevel => LineJoin::Bevel,
            Join::Miter => LineJoin::Miter,
            Join::Round => LineJoin::Round,
        };
        let stroke = TinyStroke {
            width: f64_to_f32(stroke.width),
            miter_limit: f64_to_f32(stroke.miter_limit),
            line_cap,
            line_join,
            dash: (!stroke.dash_pattern.is_empty())
                .then_some(StrokeDash::new(
                    stroke.dash_pattern.iter().map(|v| f64_to_f32(*v)).collect(),
                    f64_to_f32(stroke.dash_offset),
                ))
                .flatten(),
        };
        self.materialize_simple_clip_mask();
        let clip_mask = self.clip.is_some().then_some(&self.mask);
        self.pixmap
            .stroke_path(&path, &paint, &stroke, self.skia_transform(), clip_mask);
    }

    fn fill<'b>(&mut self, shape: &impl Shape, brush: impl Into<BrushRef<'b>>, _blur_radius: f64) {
        self.fill_with_brush_transform(shape, brush, _blur_radius, None);
    }

    fn fill_with_brush_transform<'b>(
        &mut self,
        shape: &impl Shape,
        brush: impl Into<BrushRef<'b>>,
        _blur_radius: f64,
        brush_transform: Option<Affine>,
    ) {
        // FIXME: Handle _blur_radius

        let brush = brush.into();
        if let BrushRef::Image(image) = brush {
            let image_pixmap = try_ret!(image_brush_pixmap(&image));
            let paint = Paint {
                shader: Pattern::new(
                    image_pixmap.as_ref(),
                    image_brush_spread_mode(&image),
                    image_quality_to_filter_quality(image.sampler.quality),
                    image.sampler.alpha,
                    affine_to_skia(brush_transform.unwrap_or(Affine::IDENTITY)),
                ),
                ..Default::default()
            };
            self.mark_drawn_rect_inflated(shape.bounding_box(), self.device_transform(), 2.0);
            if let Some(rect) = shape.as_rect() {
                if !self.try_fill_rect_with_paint_fast(rect, &paint, brush_transform) {
                    let rect = try_ret!(to_skia_rect(rect));
                    self.materialize_simple_clip_mask();
                    let clip_mask = self.clip.is_some().then_some(&self.mask);
                    self.pixmap
                        .fill_rect(rect, &paint, self.skia_transform(), clip_mask);
                }
            } else {
                let path = try_ret!(shape_to_path(shape));
                self.materialize_simple_clip_mask();
                let clip_mask = self.clip.is_some().then_some(&self.mask);
                self.pixmap.fill_path(
                    &path,
                    &paint,
                    FillRule::Winding,
                    self.skia_transform(),
                    clip_mask,
                );
            }
            return;
        }

        if let Some(rect) = shape.as_rect()
            && let BrushRef::Solid(color) = brush
            && self.try_fill_solid_rect_fast(rect, color)
        {
            return;
        }

        let paint = try_ret!(brush_to_paint(brush, brush_transform));
        self.mark_drawn_rect_inflated(shape.bounding_box(), self.device_transform(), 2.0);
        if let Some(rect) = shape.as_rect() {
            if !self.try_fill_rect_with_paint_fast(rect, &paint, brush_transform) {
                let rect = try_ret!(to_skia_rect(rect));
                self.materialize_simple_clip_mask();
                let clip_mask = self.clip.is_some().then_some(&self.mask);
                self.pixmap
                    .fill_rect(rect, &paint, self.skia_transform(), clip_mask);
            }
        } else {
            let path = try_ret!(shape_to_path(shape));
            self.materialize_simple_clip_mask();
            let clip_mask = self.clip.is_some().then_some(&self.mask);
            self.pixmap.fill_path(
                &path,
                &paint,
                FillRule::Winding,
                self.skia_transform(),
                clip_mask,
            );
        }
    }
}

/// CPU copy renderer for the `imaging` command stream.
pub type TinySkiaRenderer = TinySkiaRendererImpl<'static>;
/// CPU copy renderer alias for [`TinySkiaRenderer`].
pub type TinySkiaCpuCopyRenderer = TinySkiaRenderer;
/// CPU target renderer alias for [`TinySkiaTargetRenderer`].
pub type TinySkiaCpuTargetRenderer<'a> = TinySkiaTargetRenderer<'a>;

/// Core tiny-skia renderer state.
pub struct TinySkiaRendererImpl<'a> {
    cache_color: CacheColor,
    transform: Affine,
    layers: Vec<Layer<'a>>,
}

/// tiny-skia renderer that draws directly into a caller-owned CPU target.
pub struct TinySkiaTargetRenderer<'a> {
    inner: TinySkiaRendererImpl<'a>,
}

impl core::fmt::Debug for TinySkiaRendererImpl<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TinySkiaRendererImpl")
            .finish_non_exhaustive()
    }
}

impl core::fmt::Debug for TinySkiaTargetRenderer<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TinySkiaTargetRenderer")
            .finish_non_exhaustive()
    }
}

impl<'a> TinySkiaTargetRenderer<'a> {
    /// Create a temporary CPU target renderer bound to a caller-provided buffer.
    pub fn new_target(target: CpuBufferTarget<'a>) -> Result<Self> {
        if target.format == CpuBufferFormat::RGBA8_OPAQUE
            && target.bytes_per_row == target.width as usize * 4
        {
            let mut inner = TinySkiaRendererImpl {
                transform: Affine::IDENTITY,
                cache_color: CacheColor(false),
                layers: vec![Layer::new_root_borrowed(
                    target.buffer,
                    target.width,
                    target.height,
                )?],
            };
            inner.clear_root_layer();
            return Ok(Self { inner });
        }

        Err(Error::UnsupportedTargetFormat)
    }
}

impl<'a> TinySkiaRendererImpl<'a> {
    fn clip_path_for_geometry(
        &self,
        shape: imaging::GeometryRef<'_>,
        transform: Affine,
    ) -> Option<ClipPath> {
        let path = match shape {
            imaging::GeometryRef::Rect(rect) => shape_to_path(&rect)?,
            imaging::GeometryRef::RoundedRect(rect) => shape_to_path(&rect)?,
            imaging::GeometryRef::Path(path) => path_to_tiny_skia_path(path)?,
            imaging::GeometryRef::OwnedPath(ref path) => path_to_tiny_skia_path(path)?,
        }
        .transform(affine_to_skia(transform))?;

        let bounds = match shape {
            imaging::GeometryRef::Rect(rect) => rect.bounding_box(),
            imaging::GeometryRef::RoundedRect(rect) => rect.bounding_box(),
            imaging::GeometryRef::Path(path) => path.bounding_box(),
            imaging::GeometryRef::OwnedPath(ref path) => path.bounding_box(),
        };
        let simple_rect = match shape {
            imaging::GeometryRef::Rect(rect) => transformed_axis_aligned_rect(&rect, transform),
            imaging::GeometryRef::RoundedRect(_) => None,
            imaging::GeometryRef::Path(_) => None,
            imaging::GeometryRef::OwnedPath(_) => None,
        };

        Some(ClipPath {
            path,
            rect: transform.transform_rect_bbox(bounds),
            simple_rect,
        })
    }

    fn clear_root_layer(&mut self) {
        let first_layer = &mut self.layers[0];
        first_layer.pixmap.fill(tiny_skia::Color::TRANSPARENT);
        first_layer.base_clip = None;
        first_layer.clip_stack.clear();
        first_layer.clip = None;
        first_layer.simple_clip = None;
        first_layer.draw_bounds = None;
        first_layer.transform = Affine::IDENTITY;
        first_layer.mask.clear();
        first_layer.mask_valid = false;
        first_layer.group_mask = None;
    }

    fn brush_to_owned<'b>(&self, brush: impl Into<BrushRef<'b>>) -> Option<peniko::Brush> {
        match brush.into() {
            BrushRef::Solid(color) => Some(peniko::Brush::Solid(color)),
            BrushRef::Gradient(gradient) => Some(peniko::Brush::Gradient(gradient.clone())),
            BrushRef::Image(image) => Some(peniko::Brush::Image(image.to_owned())),
        }
    }

    fn current_layer_mut(&mut self) -> &mut Layer<'a> {
        self.layers
            .last_mut()
            .expect("TinySkiaRenderer always has a root layer")
    }
}

impl TinySkiaRendererImpl<'static> {
    /// Create the default CPU copy renderer.
    #[must_use]
    pub fn new() -> Self {
        Self::new_with_size(1, 1).expect("1x1 tiny-skia surface should always initialize")
    }

    /// Create the default CPU copy renderer with an explicit initial surface size.
    pub fn new_with_size(width: u32, height: u32) -> Result<Self> {
        let main_layer = Layer::new_root(width, height)?;
        Ok(Self {
            transform: Affine::IDENTITY,
            cache_color: CacheColor(false),
            layers: vec![main_layer],
        })
    }

    /// Reset the renderer for a new frame and resize the internal surface if needed.
    pub fn begin(&mut self, width: u32, height: u32) {
        if width != self.layers[0].pixmap.width() || height != self.layers[0].pixmap.height() {
            self.layers[0] = Layer::new_root(width, height).expect("unable to create layer");
        }
        assert!(
            self.layers.len() == 1,
            "TinySkiaRenderer must contain only the root layer at frame start"
        );
        self.transform = Affine::IDENTITY;
        self.clear_root_layer();
    }
}

fn rasterize_scene_pixmap(
    scene: &Scene,
    width: u32,
    height: u32,
    transform: Affine,
) -> Option<Arc<Pixmap>> {
    let mut renderer = TinySkiaRendererImpl::new_with_size(width, height).ok()?;
    let mut transformed = Scene::new();
    transformed.append_transformed(scene, transform);
    imaging::record::replay(&transformed, &mut renderer);
    let layer = renderer.layers.into_iter().next()?;
    match layer.pixmap {
        LayerPixmap::Owned(pixmap) => Some(Arc::new(pixmap)),
        LayerPixmap::Borrowed(_) => None,
    }
}

fn rasterize_scene_mask(
    scene: &Scene,
    width: u32,
    height: u32,
    transform: Affine,
    mode: MaskMode,
) -> Option<GroupMask> {
    let pixmap = rasterize_scene_pixmap(scene, width, height, transform)?;
    Some(GroupMask {
        pixmap: (*pixmap).clone(),
        mode,
    })
}

impl PaintSink for TinySkiaRendererImpl<'_> {
    fn push_clip(&mut self, clip: ClipRef<'_>) {
        let clip_path = match clip {
            ClipRef::Fill {
                transform, shape, ..
            } => self.clip_path_for_geometry(shape, transform),
            ClipRef::Stroke {
                transform, shape, ..
            } => self.clip_path_for_geometry(shape, transform),
        };
        if let Some(clip_path) = clip_path {
            let layer = self.current_layer_mut();
            layer.clip_stack.push(clip_path.clone());
            layer.intersect_clip_path(&clip_path);
        }
    }

    fn pop_clip(&mut self) {
        let layer = self.current_layer_mut();
        if layer.clip_stack.pop().is_some() {
            layer.rebuild_clip_mask();
        }
    }

    fn push_group(&mut self, group: GroupRef<'_>) {
        let clip = match group.clip {
            Some(ClipRef::Fill {
                transform, shape, ..
            })
            | Some(ClipRef::Stroke {
                transform, shape, ..
            }) => self.clip_path_for_geometry(shape, transform),
            None => {
                let rect = self.canvas_size().to_rect();
                self.clip_path_for_geometry(imaging::GeometryRef::Rect(rect), Affine::IDENTITY)
            }
        };

        if let Some(clip) = clip {
            let width = self.layers[0].pixmap.width();
            let height = self.layers[0].pixmap.height();
            let inherited_clips = self.current_layer_mut().effective_clips();
            let group_mask = group.mask.and_then(|mask| {
                rasterize_scene_mask(
                    mask.mask.scene,
                    width,
                    height,
                    mask.transform,
                    mask.mask.mode,
                )
            });
            let Ok(mut child) = Layer::new_with_base_clip(
                group.composite.blend,
                group.composite.alpha,
                clip,
                width,
                height,
            ) else {
                return;
            };
            child.rebuild_clip_mask_with_extra_clips(&inherited_clips);
            child.group_mask = group_mask;
            self.layers.push(child);
        }
    }

    fn pop_group(&mut self) {
        if self.layers.len() <= 1 {
            return;
        }
        let mut child = self.layers.pop().expect("checked layer depth");
        if let Some(group_mask) = child.group_mask.take() {
            apply_group_mask(&mut child, &group_mask);
        }
        let parent = self.current_layer_mut();
        apply_layer(&child, parent);
    }

    fn fill(&mut self, draw: FillRef<'_>) {
        let Some(brush) = self.brush_to_owned(draw.brush) else {
            return;
        };
        let blur_radius = 0.0;
        match draw.shape {
            imaging::GeometryRef::Rect(rect) => {
                let layer = self.current_layer_mut();
                layer.transform = draw.transform;
                layer.fill_with_brush_transform(&rect, &brush, blur_radius, draw.brush_transform);
            }
            imaging::GeometryRef::RoundedRect(rect) => {
                let layer = self.current_layer_mut();
                layer.transform = draw.transform;
                layer.fill_with_brush_transform(&rect, &brush, blur_radius, draw.brush_transform);
            }
            imaging::GeometryRef::Path(path) => {
                let layer = self.current_layer_mut();
                layer.transform = draw.transform;
                layer.fill_with_brush_transform(path, &brush, blur_radius, draw.brush_transform);
            }
            imaging::GeometryRef::OwnedPath(path) => {
                let layer = self.current_layer_mut();
                layer.transform = draw.transform;
                layer.fill_with_brush_transform(&path, &brush, blur_radius, draw.brush_transform);
            }
        }
    }

    fn stroke(&mut self, draw: StrokeRef<'_>) {
        let Some(brush) = self.brush_to_owned(draw.brush) else {
            return;
        };
        let layer = self.current_layer_mut();
        layer.transform = draw.transform;
        match draw.shape {
            imaging::GeometryRef::Rect(rect) => {
                layer.stroke_with_brush_transform(&rect, &brush, draw.stroke, draw.brush_transform);
            }
            imaging::GeometryRef::RoundedRect(rect) => {
                layer.stroke_with_brush_transform(&rect, &brush, draw.stroke, draw.brush_transform);
            }
            imaging::GeometryRef::Path(path) => {
                layer.stroke_with_brush_transform(path, &brush, draw.stroke, draw.brush_transform);
            }
            imaging::GeometryRef::OwnedPath(ref path) => {
                layer.stroke_with_brush_transform(path, &brush, draw.stroke, draw.brush_transform);
            }
        }
    }

    fn glyph_run(
        &mut self,
        draw: GlyphRunRef<'_>,
        glyphs: &mut dyn Iterator<Item = imaging::record::Glyph>,
    ) {
        self.draw_glyphs(Point::ZERO, &draw, glyphs);
    }

    fn blurred_rounded_rect(&mut self, draw: BlurredRoundedRect) {
        self.transform = draw.transform;
        let shape = draw.rect.to_rounded_rect(draw.radius);
        self.fill(&shape, draw.color, draw.std_dev);
    }
}

impl TinySkiaRendererImpl<'static> {
    fn set_size(&mut self, size: Size) {
        Self::begin(self, f64_to_u32(size.width), f64_to_u32(size.height));
    }

    fn reset_for_frame(&mut self) {}

    /// Render any [`RenderSource`] into a caller-provided image buffer.
    pub fn render_source_into<S: RenderSource + ?Sized>(
        &mut self,
        source: &mut S,
        width: u32,
        height: u32,
        image: &mut RgbaImage,
    ) -> Result<()> {
        source.validate().map_err(Error::InvalidScene)?;
        self.set_size(Size::new(width as f64, height as f64));
        self.reset_for_frame();
        source.paint_into(self);
        image.resize(width, height);
        self.finish_into_rgba8_opaque(image.data.as_mut_slice(), usize_from_u32(width) * 4)
            .ok_or(Error::Internal(
                "tiny-skia image backend did not produce an image",
            ))
    }

    /// Render any [`RenderSource`] and return a newly allocated image.
    pub fn render_source<S: RenderSource + ?Sized>(
        &mut self,
        source: &mut S,
        width: u32,
        height: u32,
    ) -> Result<RgbaImage> {
        let mut image = RgbaImage::new(width, height);
        self.render_source_into(source, width, height, &mut image)?;
        Ok(image)
    }

    /// Render a recorded scene into an RGBA8 image (opaque alpha).
    pub fn render_scene_into(
        &mut self,
        scene: &Scene,
        width: u32,
        height: u32,
        image: &mut RgbaImage,
    ) -> Result<()> {
        let mut source = scene;
        self.render_source_into(&mut source, width, height, image)
    }

    /// Render a recorded scene and return an RGBA8 image (opaque alpha).
    pub fn render_scene(&mut self, scene: &Scene, width: u32, height: u32) -> Result<RgbaImage> {
        let mut source = scene;
        self.render_source(&mut source, width, height)
    }
}

impl Default for TinySkiaRendererImpl<'static> {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageRenderer for TinySkiaRendererImpl<'static> {
    type Error = Error;

    fn render_source_into<S: RenderSource + ?Sized>(
        &mut self,
        source: &mut S,
        width: u32,
        height: u32,
        image: &mut RgbaImage,
    ) -> Result<()> {
        TinySkiaRendererImpl::render_source_into(self, source, width, height, image)
    }
}

impl<'a> TinySkiaTargetRenderer<'a> {
    /// Validate whether the renderer can draw directly into the provided target shape.
    pub fn supports_target_info(target: &CpuBufferTargetInfo) -> Result<()> {
        if target.format == CpuBufferFormat::RGBA8_OPAQUE
            && target.bytes_per_row == usize_from_u32(target.width) * 4
        {
            return Ok(());
        }
        Err(Error::UnsupportedTargetFormat)
    }

    /// Rebind the target renderer to a different caller-owned pixel buffer.
    pub fn set_target(&mut self, target: CpuBufferTarget<'a>) -> Result<()> {
        *self = Self::new_target(target)?;
        Ok(())
    }

    /// Render any [`RenderSource`] into the currently bound caller-owned target.
    pub fn render_source<S: RenderSource + ?Sized>(&mut self, source: &mut S) -> Result<()> {
        source.validate().map_err(Error::InvalidScene)?;
        self.inner.clear_root_layer();
        source.paint_into(&mut self.inner);
        self.inner
            .finish_direct_rgba8_opaque()
            .ok_or(Error::Internal(
                "tiny-skia target renderer did not produce a frame",
            ))
    }

    /// Render a recorded scene into the currently bound caller-owned target.
    pub fn render_scene(&mut self, scene: &Scene) -> Result<()> {
        let mut source = scene;
        self.render_source(&mut source)
    }
}

fn to_color(color: Color) -> tiny_skia::Color {
    let c = color.to_rgba8();
    tiny_skia::Color::from_rgba8(c.r, c.g, c.b, c.a)
}

fn to_point(point: Point) -> tiny_skia::Point {
    tiny_skia::Point::from_xy(f64_to_f32(point.x), f64_to_f32(point.y))
}

fn is_axis_aligned(transform: Affine) -> bool {
    let coeffs = transform.as_coeffs();
    coeffs[1] == 0.0 && coeffs[2] == 0.0
}

fn affine_scale_components(transform: Affine) -> (f64, f64, f64) {
    let coeffs = transform.as_coeffs();
    let scale_x = coeffs[0].hypot(coeffs[1]);
    let scale_y = coeffs[2].hypot(coeffs[3]);
    let uniform = (scale_x + scale_y) * 0.5;
    (scale_x, scale_y, uniform)
}

#[cfg(test)]
fn scaled_embolden_strength(font_embolden: Vec2, raster_scale: f64) -> f32 {
    f64_to_f32(font_embolden.x.abs().max(font_embolden.y.abs()) * raster_scale)
}

const MIN_TEXT_RASTER_SCALE: f64 = 2.0;

fn effective_text_raster_scale(raster_scale: f64) -> f64 {
    if raster_scale <= 0.0 {
        raster_scale
    } else {
        raster_scale.max(MIN_TEXT_RASTER_SCALE)
    }
}

fn usize_from_u32(value: u32) -> usize {
    match usize::try_from(value) {
        Ok(value) => value,
        Err(_) => panic!("u32 value must fit in usize"),
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "This helper is the centralized checked f32-to-i32 narrowing boundary"
)]
fn f32_to_i32(value: f32) -> i32 {
    assert!(
        value.is_finite(),
        "value must be finite before narrowing to i32"
    );
    assert!(
        value >= i32::MIN as f32 && value <= i32::MAX as f32,
        "value must fit in i32 before narrowing"
    );
    value as i32
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "This helper is the centralized checked f64-to-f32 narrowing boundary"
)]
fn f64_to_f32(value: f64) -> f32 {
    assert!(
        value.is_finite(),
        "value must be finite before narrowing to f32"
    );
    assert!(
        value >= f64::from(f32::MIN) && value <= f64::from(f32::MAX),
        "value must fit in f32 before narrowing"
    );
    value as f32
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "This helper is the centralized checked f64-to-i32 narrowing boundary"
)]
fn f64_to_i32(value: f64) -> i32 {
    assert!(
        value.is_finite(),
        "value must be finite before narrowing to i32"
    );
    assert!(
        value >= f64::from(i32::MIN) && value <= f64::from(i32::MAX),
        "value must fit in i32 before narrowing"
    );
    value as i32
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "This helper is the centralized checked f64-to-u32 narrowing boundary"
)]
fn f64_to_u32(value: f64) -> u32 {
    assert!(
        value.is_finite(),
        "value must be finite before narrowing to u32"
    );
    assert!(
        value >= 0.0,
        "value must be non-negative before narrowing to u32"
    );
    assert!(
        value <= f64::from(u32::MAX),
        "value must fit in u32 before narrowing"
    );
    value as u32
}

fn floor_to_i32(value: f64) -> i32 {
    f64_to_i32(value.floor())
}

fn ceil_to_i32(value: f64) -> i32 {
    f64_to_i32(value.ceil())
}

fn round_to_i32(value: f64) -> i32 {
    f64_to_i32(value.round())
}

fn normalize_affine(transform: Affine, include_translation: bool) -> Affine {
    let coeffs = transform.as_coeffs();
    let (scale_x, scale_y, _) = affine_scale_components(transform);
    let tx = if include_translation { coeffs[4] } else { 0.0 };
    let ty = if include_translation { coeffs[5] } else { 0.0 };
    Affine::new([
        if scale_x != 0.0 {
            coeffs[0] / scale_x
        } else {
            0.0
        },
        if scale_x != 0.0 {
            coeffs[1] / scale_x
        } else {
            0.0
        },
        if scale_y != 0.0 {
            coeffs[2] / scale_y
        } else {
            0.0
        },
        if scale_y != 0.0 {
            coeffs[3] / scale_y
        } else {
            0.0
        },
        tx,
        ty,
    ])
}

fn transformed_axis_aligned_rect(shape: &impl Shape, transform: Affine) -> Option<Rect> {
    let rect = shape.as_rect()?;
    is_axis_aligned(transform).then(|| transform.transform_rect_bbox(rect))
}

fn nearly_integral(value: f64) -> Option<i32> {
    let rounded = value.round();
    ((value - rounded).abs() <= 1e-6).then(|| round_to_i32(value))
}

fn integer_translation(transform: Affine, x: f64, y: f64) -> Option<(i32, i32)> {
    let coeffs = transform.as_coeffs();
    (coeffs[0] == 1.0 && coeffs[1] == 0.0 && coeffs[2] == 0.0 && coeffs[3] == 1.0).then_some((
        nearly_integral(x + coeffs[4])?,
        nearly_integral(y + coeffs[5])?,
    ))
}

fn image_quality_to_filter_quality(quality: ImageQuality) -> FilterQuality {
    match quality {
        ImageQuality::Low => FilterQuality::Nearest,
        ImageQuality::Medium | ImageQuality::High => FilterQuality::Bilinear,
    }
}

fn mul_div_255(value: u8, factor: u8) -> u8 {
    match u8::try_from((u16::from(value) * u16::from(factor) + 127) / 255) {
        Ok(value) => value,
        Err(_) => panic!("scaled 8-bit value must fit in u8"),
    }
}

fn scale_premultiplied_color(color: PremultipliedColorU8, alpha: u8) -> PremultipliedColorU8 {
    if alpha == 255 {
        return color;
    }

    PremultipliedColorU8::from_rgba(
        mul_div_255(color.red(), alpha),
        mul_div_255(color.green(), alpha),
        mul_div_255(color.blue(), alpha),
        mul_div_255(color.alpha(), alpha),
    )
    .expect("scaled premultiplied color must remain premultiplied")
}

fn blend_source_over(src: PremultipliedColorU8, dst: PremultipliedColorU8) -> PremultipliedColorU8 {
    if src.alpha() == 255 {
        return src;
    }
    if src.alpha() == 0 {
        return dst;
    }

    let inv_alpha = 255 - src.alpha();
    PremultipliedColorU8::from_rgba(
        src.red().saturating_add(mul_div_255(dst.red(), inv_alpha)),
        src.green()
            .saturating_add(mul_div_255(dst.green(), inv_alpha)),
        src.blue()
            .saturating_add(mul_div_255(dst.blue(), inv_alpha)),
        src.alpha()
            .saturating_add(mul_div_255(dst.alpha(), inv_alpha)),
    )
    .expect("source-over premultiplied blend must remain premultiplied")
}

impl TinySkiaRendererImpl<'_> {
    fn canvas_size(&self) -> Size {
        Size::new(
            self.layers[0].pixmap.width() as f64,
            self.layers[0].pixmap.height() as f64,
        )
    }

    fn fill<'b>(&mut self, shape: &impl Shape, brush: impl Into<BrushRef<'b>>, blur_radius: f64) {
        let Some(brush) = self.brush_to_owned(brush) else {
            return;
        };
        let transform = self.transform;
        let layer = self.current_layer_mut();
        layer.transform = transform;
        layer.fill(shape, &brush, blur_radius);
    }

    fn draw_glyphs<'a>(
        &mut self,
        origin: Point,
        run: &GlyphRunRef<'a>,
        glyphs: impl Iterator<Item = imaging::record::Glyph> + 'a,
    ) {
        let font = run.font;
        let text_transform = run.transform;
        let (_, _, raster_scale) = affine_scale_components(text_transform);
        let effective_raster_scale = effective_text_raster_scale(raster_scale);
        let oversample = if raster_scale > 0.0 {
            effective_raster_scale / raster_scale
        } else {
            1.0
        };
        let transform = normalize_affine(text_transform, false) * Affine::scale(1.0 / oversample);
        let raster_origin = transform.inverse() * (text_transform * origin);
        let brush_color = match &run.brush {
            peniko::Brush::Solid(color) => Color::from(*color),
            _ => return,
        };
        let font_ref = match FontRef::from_index(font.data.data(), font.index as usize) {
            Some(f) => f,
            None => return,
        };
        let font_blob_id = font.data.id();
        let skew = run
            .glyph_transform
            .map(|transform| f64_to_f32(transform.as_coeffs()[0].atan().to_degrees()));

        for glyph in glyphs {
            let glyph_x = f64_to_f32(raster_origin.x + glyph.x as f64 * effective_raster_scale);
            let glyph_y = f64_to_f32(raster_origin.y + glyph.y as f64 * effective_raster_scale);
            let scaled_font_size = run.font_size * f64_to_f32(effective_raster_scale);
            let scaled_embolden = 0.0;
            let glyph_id = match u16::try_from(glyph.id) {
                Ok(value) => value,
                Err(_) => continue,
            };
            let (cache_key, new_x, new_y) = GlyphCacheKey::new(GlyphKeyInput {
                font_blob_id,
                font_index: font.index,
                glyph_id,
                font_size: scaled_font_size,
                x: glyph_x,
                y: glyph_y,
                hint: run.hint,
                embolden: false,
                skew,
            });

            let cached = cache_glyph(GlyphRasterRequest {
                cache_color: self.cache_color,
                cache_key,
                color: brush_color,
                font_ref,
                font_size: scaled_font_size,
                hint: run.hint,
                normalized_coords: run.normalized_coords,
                embolden_strength: scaled_embolden,
                skew,
                offset_x: new_x,
                offset_y: new_y,
            });

            if let Some(cached) = cached {
                self.current_layer_mut().render_pixmap_direct(
                    cached.pixmap.as_ref(),
                    new_x + cached.left,
                    new_y - cached.top,
                    transform,
                    if oversample > 1.0 {
                        FilterQuality::Bilinear
                    } else {
                        FilterQuality::Nearest
                    },
                );
            }
        }
    }

    fn finish_into_rgba8_opaque(&mut self, dst: &mut [u8], bytes_per_row: usize) -> Option<()> {
        self.finish_into_opaque(dst, bytes_per_row, true)
    }

    fn finish_direct_rgba8_opaque(&mut self) -> Option<()> {
        self.finalize_frame();
        for pixel in self.layers[0].pixmap.data_mut().chunks_exact_mut(4) {
            pixel[3] = 0xff;
        }
        Some(())
    }

    fn finalize_frame(&mut self) {
        IMAGE_CACHE.with_borrow_mut(|ic| ic.retain(|_, (c, _)| *c == self.cache_color));
        SCALED_IMAGE_CACHE.with_borrow_mut(|ic| ic.retain(|_, (c, _)| *c == self.cache_color));
        let now = Instant::now();
        GLYPH_CACHE.with_borrow_mut(|gc| {
            gc.retain(|_, entry| should_retain_glyph_entry(entry, self.cache_color, now));
        });
        self.cache_color = CacheColor(!self.cache_color.0);
    }

    fn finish_into_opaque(
        &mut self,
        dst: &mut [u8],
        bytes_per_row: usize,
        rgba: bool,
    ) -> Option<()> {
        self.finalize_frame();

        let pixmap = &self.layers[0].pixmap;
        let width = pixmap.width() as usize;
        let height = pixmap.height() as usize;
        if dst.len() < bytes_per_row.checked_mul(height)? || bytes_per_row < width * 4 {
            return None;
        }

        for (src_row, dst_row) in pixmap
            .data()
            .chunks_exact(width * 4)
            .zip(dst.chunks_exact_mut(bytes_per_row))
        {
            for (src, out) in src_row
                .chunks_exact(4)
                .zip(dst_row[..width * 4].chunks_exact_mut(4))
            {
                if rgba {
                    out.copy_from_slice(&[src[0], src[1], src[2], 0xff]);
                } else {
                    out.copy_from_slice(&[src[2], src[1], src[0], 0xff]);
                }
            }
        }
        Some(())
    }
}

fn shape_to_path(shape: &impl Shape) -> Option<Path> {
    let mut builder = PathBuilder::new();
    for element in shape.path_elements(0.1) {
        match element {
            PathEl::ClosePath => builder.close(),
            PathEl::MoveTo(p) => builder.move_to(f64_to_f32(p.x), f64_to_f32(p.y)),
            PathEl::LineTo(p) => builder.line_to(f64_to_f32(p.x), f64_to_f32(p.y)),
            PathEl::QuadTo(p1, p2) => {
                builder.quad_to(
                    f64_to_f32(p1.x),
                    f64_to_f32(p1.y),
                    f64_to_f32(p2.x),
                    f64_to_f32(p2.y),
                );
            }
            PathEl::CurveTo(p1, p2, p3) => {
                builder.cubic_to(
                    f64_to_f32(p1.x),
                    f64_to_f32(p1.y),
                    f64_to_f32(p2.x),
                    f64_to_f32(p2.y),
                    f64_to_f32(p3.x),
                    f64_to_f32(p3.y),
                );
            }
        }
    }
    builder.finish()
}

fn path_to_tiny_skia_path(path: &BezPath) -> Option<Path> {
    let mut builder = PathBuilder::new();
    for element in path.elements() {
        match element {
            PathEl::ClosePath => builder.close(),
            PathEl::MoveTo(p) => builder.move_to(f64_to_f32(p.x), f64_to_f32(p.y)),
            PathEl::LineTo(p) => builder.line_to(f64_to_f32(p.x), f64_to_f32(p.y)),
            PathEl::QuadTo(p1, p2) => {
                builder.quad_to(
                    f64_to_f32(p1.x),
                    f64_to_f32(p1.y),
                    f64_to_f32(p2.x),
                    f64_to_f32(p2.y),
                );
            }
            PathEl::CurveTo(p1, p2, p3) => {
                builder.cubic_to(
                    f64_to_f32(p1.x),
                    f64_to_f32(p1.y),
                    f64_to_f32(p2.x),
                    f64_to_f32(p2.y),
                    f64_to_f32(p3.x),
                    f64_to_f32(p3.y),
                );
            }
        }
    }
    builder.finish()
}

fn brush_to_paint<'b>(
    brush: impl Into<BrushRef<'b>>,
    brush_transform: Option<Affine>,
) -> Option<Paint<'static>> {
    let shader_transform = affine_to_skia(brush_transform.unwrap_or(Affine::IDENTITY));
    let shader = match brush.into() {
        BrushRef::Solid(c) => Shader::SolidColor(to_color(c)),
        BrushRef::Gradient(g) => {
            let stops = expand_gradient_stops(g);
            let spread_mode = to_spread_mode(g.extend);
            match g.kind {
                GradientKind::Linear(linear) => LinearGradient::new(
                    to_point(linear.start),
                    to_point(linear.end),
                    stops,
                    spread_mode,
                    shader_transform,
                )?,
                GradientKind::Radial(RadialGradientPosition {
                    start_center,
                    start_radius: _,
                    end_center,
                    end_radius,
                }) => {
                    // FIXME: Doesn't use `start_radius`
                    RadialGradient::new(
                        to_point(start_center),
                        to_point(end_center),
                        end_radius,
                        stops,
                        spread_mode,
                        shader_transform,
                    )?
                }
                GradientKind::Sweep { .. } => return None,
            }
        }
        BrushRef::Image(_) => return None,
    };
    Some(Paint {
        shader,
        ..Default::default()
    })
}

fn image_brush_pixmap<T>(image: &peniko::ImageBrush<T>) -> Option<Pixmap>
where
    T: Borrow<ImageData>,
{
    let image_data = image.image.borrow();
    let mut pixmap = Pixmap::new(image_data.width, image_data.height)?;
    for (a, b) in pixmap
        .pixels_mut()
        .iter_mut()
        .zip(image_data.data.data().chunks_exact(4))
    {
        *a = tiny_skia::Color::from_rgba8(b[0], b[1], b[2], b[3])
            .premultiply()
            .to_color_u8();
    }
    Some(pixmap)
}

fn image_brush_spread_mode<T>(image: &peniko::ImageBrush<T>) -> SpreadMode {
    let extend = if image.sampler.x_extend == image.sampler.y_extend {
        image.sampler.x_extend
    } else {
        Extend::Pad
    };
    to_spread_mode(extend)
}

const GRADIENT_TOLERANCE: f32 = 0.01;

fn expand_gradient_stops(gradient: &Gradient) -> Vec<GradientStop> {
    if gradient.stops.is_empty() {
        return Vec::new();
    }

    if gradient.stops.len() == 1 {
        let stop = &gradient.stops[0];
        let color = stop.color.to_alpha_color::<Srgb>().convert::<Srgb>();
        return vec![GradientStop::new(
            stop.offset,
            alpha_color_to_tiny_skia(color),
        )];
    }

    let mut expanded = Vec::new();
    for segment in gradient.stops.windows(2) {
        let start = segment[0];
        let end = segment[1];
        if start.offset == end.offset {
            push_gradient_stop(
                &mut expanded,
                start.offset,
                start.color.to_alpha_color::<Srgb>(),
            );
            push_gradient_stop(
                &mut expanded,
                end.offset,
                end.color.to_alpha_color::<Srgb>(),
            );
            continue;
        }

        expand_gradient_segment(
            &mut expanded,
            start.offset,
            start.color,
            end.offset,
            end.color,
            gradient.interpolation_cs,
            gradient.hue_direction,
        );
    }

    expanded
}

fn expand_gradient_segment(
    expanded: &mut Vec<GradientStop>,
    start_offset: f32,
    start_color: DynamicColor,
    end_offset: f32,
    end_color: DynamicColor,
    interpolation_cs: ColorSpaceTag,
    hue_direction: HueDirection,
) {
    let push_sample = |expanded: &mut Vec<GradientStop>, t: f32, color: color::AlphaColor<Srgb>| {
        let offset = start_offset + (end_offset - start_offset) * t;
        push_gradient_stop(expanded, offset, color);
    };

    for (i, (t, color)) in color::gradient::<Srgb>(
        start_color,
        end_color,
        interpolation_cs,
        hue_direction,
        GRADIENT_TOLERANCE,
    )
    .enumerate()
    {
        if !expanded.is_empty() && i == 0 {
            continue;
        }
        push_sample(expanded, t, color.un_premultiply());
    }
}

fn push_gradient_stop(
    expanded: &mut Vec<GradientStop>,
    offset: f32,
    color: color::AlphaColor<Srgb>,
) {
    let tiny_color = alpha_color_to_tiny_skia(color);
    if let Some(previous) = expanded.last()
        && previous == &GradientStop::new(offset, tiny_color)
    {
        return;
    }
    expanded.push(GradientStop::new(offset, tiny_color));
}

fn alpha_color_to_tiny_skia(color: color::AlphaColor<Srgb>) -> tiny_skia::Color {
    let color = color.to_rgba8();
    tiny_skia::Color::from_rgba8(color.r, color.g, color.b, color.a)
}

fn to_spread_mode(extend: Extend) -> SpreadMode {
    match extend {
        Extend::Pad => SpreadMode::Pad,
        Extend::Repeat => SpreadMode::Repeat,
        Extend::Reflect => SpreadMode::Reflect,
    }
}

fn to_skia_rect(rect: Rect) -> Option<tiny_skia::Rect> {
    tiny_skia::Rect::from_ltrb(
        f64_to_f32(rect.x0),
        f64_to_f32(rect.y0),
        f64_to_f32(rect.x1),
        f64_to_f32(rect.y1),
    )
}

fn rect_to_int_rect(rect: Rect) -> Option<IntRect> {
    IntRect::from_ltrb(
        floor_to_i32(rect.x0),
        floor_to_i32(rect.y0),
        ceil_to_i32(rect.x1),
        ceil_to_i32(rect.y1),
    )
}

type TinyBlendMode = tiny_skia::BlendMode;

enum BlendStrategy {
    /// Can be directly mapped to a tiny-skia blend mode
    SinglePass(TinyBlendMode),
    /// Requires multiple operations
    MultiPass {
        first_pass: TinyBlendMode,
        second_pass: TinyBlendMode,
    },
}

fn determine_blend_strategy(peniko_mode: &BlendMode) -> BlendStrategy {
    match (peniko_mode.mix, peniko_mode.compose) {
        (Mix::Normal, compose) => BlendStrategy::SinglePass(compose_to_tiny_blend_mode(compose)),

        (mix, Compose::SrcOver) => BlendStrategy::SinglePass(mix_to_tiny_blend_mode(mix)),

        (mix, compose) => BlendStrategy::MultiPass {
            first_pass: compose_to_tiny_blend_mode(compose),
            second_pass: mix_to_tiny_blend_mode(mix),
        },
    }
}

fn compose_to_tiny_blend_mode(compose: Compose) -> TinyBlendMode {
    match compose {
        Compose::Clear => TinyBlendMode::Clear,
        Compose::Copy => TinyBlendMode::Source,
        Compose::Dest => TinyBlendMode::Destination,
        Compose::SrcOver => TinyBlendMode::SourceOver,
        Compose::DestOver => TinyBlendMode::DestinationOver,
        Compose::SrcIn => TinyBlendMode::SourceIn,
        Compose::DestIn => TinyBlendMode::DestinationIn,
        Compose::SrcOut => TinyBlendMode::SourceOut,
        Compose::DestOut => TinyBlendMode::DestinationOut,
        Compose::SrcAtop => TinyBlendMode::SourceAtop,
        Compose::DestAtop => TinyBlendMode::DestinationAtop,
        Compose::Xor => TinyBlendMode::Xor,
        Compose::Plus => TinyBlendMode::Plus,
        Compose::PlusLighter => TinyBlendMode::Plus, // ??
    }
}

fn mix_to_tiny_blend_mode(mix: Mix) -> TinyBlendMode {
    match mix {
        Mix::Normal => TinyBlendMode::SourceOver,
        Mix::Multiply => TinyBlendMode::Multiply,
        Mix::Screen => TinyBlendMode::Screen,
        Mix::Overlay => TinyBlendMode::Overlay,
        Mix::Darken => TinyBlendMode::Darken,
        Mix::Lighten => TinyBlendMode::Lighten,
        Mix::ColorDodge => TinyBlendMode::ColorDodge,
        Mix::ColorBurn => TinyBlendMode::ColorBurn,
        Mix::HardLight => TinyBlendMode::HardLight,
        Mix::SoftLight => TinyBlendMode::SoftLight,
        Mix::Difference => TinyBlendMode::Difference,
        Mix::Exclusion => TinyBlendMode::Exclusion,
        Mix::Hue => TinyBlendMode::Hue,
        Mix::Saturation => TinyBlendMode::Saturation,
        Mix::Color => TinyBlendMode::Color,
        Mix::Luminosity => TinyBlendMode::Luminosity,
    }
}

fn layer_composite_rect(layer: &Layer<'_>, parent: &Layer<'_>) -> Option<IntRect> {
    let mut rect = Rect::from_origin_size(
        Point::ZERO,
        Size::new(layer.pixmap.width() as f64, layer.pixmap.height() as f64),
    );

    if let Some(draw_bounds) = layer.draw_bounds {
        rect = rect.intersect(draw_bounds);
    } else {
        return None;
    }

    if let Some(layer_clip) = layer.clip {
        rect = rect.intersect(layer_clip);
    }

    if let Some(parent_clip) = parent.clip {
        rect = rect.intersect(parent_clip);
    }

    if rect.is_zero_area() {
        return None;
    }

    rect_to_int_rect(rect)
}

fn draw_layer_pixmap(
    pixmap: &Pixmap,
    x: i32,
    y: i32,
    parent: &mut Layer<'_>,
    blend_mode: TinyBlendMode,
    alpha: f32,
) {
    parent.mark_drawn_device_rect(Rect::new(
        f64::from(x),
        f64::from(y),
        f64::from(x + i32::try_from(pixmap.width()).expect("pixmap width must fit in i32")),
        f64::from(y + i32::try_from(pixmap.height()).expect("pixmap height must fit in i32")),
    ));

    let paint = PixmapPaint {
        opacity: alpha,
        blend_mode,
        quality: FilterQuality::Nearest,
    };
    parent.materialize_simple_clip_mask();
    let clip_mask = parent.clip.is_some().then_some(&parent.mask);

    parent.pixmap.draw_pixmap(
        x,
        y,
        pixmap.as_ref(),
        &paint,
        Transform::identity(),
        clip_mask,
    );
}

fn draw_layer_region(
    parent: &mut Layer<'_>,
    pixmap: PixmapRef<'_>,
    composite_rect: IntRect,
    blend_mode: TinyBlendMode,
    alpha: f32,
) {
    let Some(cropped) = pixmap.clone_rect(composite_rect) else {
        return;
    };

    draw_layer_pixmap(
        &cropped,
        composite_rect.x(),
        composite_rect.y(),
        parent,
        blend_mode,
        alpha,
    );
}

fn apply_alpha_mask_from_pixmap(target: &mut Pixmap, mask_source: PixmapRef<'_>) {
    let mask = Mask::from_pixmap(mask_source, MaskType::Alpha);
    target.apply_mask(&mask);
}

fn mask_coverage(mask_source: &[u8], idx: usize, mode: MaskMode) -> u8 {
    match mode {
        MaskMode::Alpha => mask_source[idx * 4 + 3],
        MaskMode::Luminance => {
            let r = mask_source[idx * 4];
            let g = mask_source[idx * 4 + 1];
            let b = mask_source[idx * 4 + 2];
            u8::try_from((u16::from(r) * 54 + u16::from(g) * 183 + u16::from(b) * 19 + 127) / 255)
                .expect("luminance coverage must fit in u8")
        }
    }
}

fn apply_group_mask(layer: &mut Layer<'_>, group_mask: &GroupMask) {
    if group_mask.mode == MaskMode::Alpha {
        apply_alpha_mask_from_pixmap(
            match &mut layer.pixmap {
                LayerPixmap::Owned(pixmap) => pixmap,
                LayerPixmap::Borrowed(_) => return,
            },
            group_mask.pixmap.as_ref(),
        );
        return;
    }

    let mask_source = group_mask.pixmap.data();
    let pixels = layer.pixmap.pixels_mut();
    for (idx, pixel) in pixels.iter_mut().enumerate() {
        let coverage = mask_coverage(mask_source, idx, group_mask.mode);
        *pixel = scale_premultiplied_color(*pixel, coverage);
    }
}

fn apply_layer(layer: &Layer<'_>, parent: &mut Layer<'_>) {
    let Some(composite_rect) = layer_composite_rect(layer, parent) else {
        return;
    };

    match determine_blend_strategy(&layer.blend_mode) {
        BlendStrategy::SinglePass(blend_mode) => {
            draw_layer_region(
                parent,
                layer.pixmap.as_ref(),
                composite_rect,
                blend_mode,
                layer.alpha,
            );
        }
        BlendStrategy::MultiPass {
            first_pass,
            second_pass,
        } => {
            let Some(original_parent) = parent.pixmap.clone_rect(composite_rect) else {
                return;
            };
            let Some(coverage) = layer.pixmap.clone_rect(composite_rect) else {
                return;
            };

            draw_layer_region(
                parent,
                layer.pixmap.as_ref(),
                composite_rect,
                first_pass,
                1.0,
            );

            let Some(mut intermediate) = parent.pixmap.clone_rect(composite_rect) else {
                return;
            };
            apply_alpha_mask_from_pixmap(&mut intermediate, coverage.as_ref());

            draw_layer_pixmap(
                &original_parent,
                composite_rect.x(),
                composite_rect.y(),
                parent,
                TinyBlendMode::Source,
                1.0,
            );

            draw_layer_pixmap(
                &intermediate,
                composite_rect.x(),
                composite_rect.y(),
                parent,
                second_pass,
                layer.alpha,
            );
        }
    }
}

fn affine_to_skia(affine: Affine) -> Transform {
    let transform = affine.as_coeffs();
    Transform::from_row(
        f64_to_f32(transform[0]),
        f64_to_f32(transform[1]),
        f64_to_f32(transform[2]),
        f64_to_f32(transform[3]),
        f64_to_f32(transform[4]),
        f64_to_f32(transform[5]),
    )
}

fn skia_transform(affine: Affine) -> Transform {
    affine_to_skia(affine)
}

#[cfg(test)]
mod tests {
    use super::*;
    use imaging::{GroupRef, MaskMode, Painter, record::Scene};
    use peniko::color::{ColorSpaceTag, HueDirection, palette::css};

    /// Creates a `Layer` directly without a window, for offscreen rendering.
    fn make_layer(width: u32, height: u32) -> Layer<'static> {
        Layer {
            pixmap: LayerPixmap::Owned(
                Pixmap::new(width, height).expect("failed to create pixmap"),
            ),
            clip_stack: vec![],
            base_clip: None,
            clip: None,
            simple_clip: None,
            draw_bounds: None,
            mask: Mask::new(width, height).expect("failed to create mask"),
            mask_valid: false,
            transform: Affine::IDENTITY,
            blend_mode: Mix::Normal.into(),
            alpha: 1.0,
            group_mask: None,
        }
    }

    fn pixel_rgba(layer: &Layer<'_>, x: u32, y: u32) -> (u8, u8, u8, u8) {
        let idx = (y * layer.pixmap.width() + x) as usize;
        let data = layer.pixmap.data();
        let pixel = &data[idx * 4..idx * 4 + 4];
        (pixel[0], pixel[1], pixel[2], pixel[3])
    }

    fn rgba_distance(a: (u8, u8, u8, u8), b: (u8, u8, u8, u8)) -> u32 {
        a.0.abs_diff(b.0) as u32
            + a.1.abs_diff(b.1) as u32
            + a.2.abs_diff(b.2) as u32
            + a.3.abs_diff(b.3) as u32
    }

    fn interpolated_midpoint(
        start: DynamicColor,
        end: DynamicColor,
        color_space: ColorSpaceTag,
        hue_direction: HueDirection,
    ) -> (u8, u8, u8, u8) {
        let color = start
            .interpolate(end, color_space, hue_direction)
            .eval(0.5)
            .to_alpha_color::<Srgb>()
            .to_rgba8();
        (color.r, color.g, color.b, color.a)
    }

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
    fn render_pixmap_rect_uses_transform_and_mask() {
        let mut layer = make_layer(12, 12);
        layer.transform = Affine::translate((4.0, 0.0));
        layer.clip(&Rect::new(1.0, 0.0, 3.0, 4.0));

        let mut src = Pixmap::new(2, 2).expect("failed to create src pixmap");
        src.fill(tiny_skia::Color::from_rgba8(255, 0, 0, 255));

        layer.render_pixmap_rect(
            &src,
            Rect::new(0.0, 0.0, 4.0, 4.0),
            layer.device_transform(),
            ImageQuality::Medium,
        );

        assert_eq!(pixel_rgba(&layer, 3, 1), (0, 0, 0, 0));
        assert_eq!(pixel_rgba(&layer, 4, 1), (0, 0, 0, 0));
        assert_eq!(pixel_rgba(&layer, 5, 1), (255, 0, 0, 255));
        assert_eq!(pixel_rgba(&layer, 6, 1), (255, 0, 0, 255));
        assert_eq!(pixel_rgba(&layer, 7, 1), (0, 0, 0, 0));
        assert_eq!(pixel_rgba(&layer, 8, 1), (0, 0, 0, 0));
    }

    #[test]
    fn rect_clip_avoids_materializing_mask() {
        let mut renderer = TinySkiaRenderer::new_with_size(8, 8).expect("renderer");
        renderer.begin(8, 8);

        PaintSink::push_clip(&mut renderer, ClipRef::fill(Rect::new(2.0, 2.0, 6.0, 6.0)));
        PaintSink::push_clip(&mut renderer, ClipRef::fill(Rect::new(3.0, 1.0, 7.0, 5.0)));
        PaintSink::fill(
            &mut renderer,
            FillRef::new(Rect::new(0.0, 0.0, 8.0, 8.0), Color::from_rgb8(255, 0, 0)),
        );

        let root = &renderer.layers[0];
        assert!(root.clip_mask_is_empty());
        assert_eq!(pixel_rgba(root, 2, 2), (0, 0, 0, 0));
        assert_eq!(pixel_rgba(root, 3, 2), (255, 0, 0, 255));
        assert_eq!(pixel_rgba(root, 5, 4), (255, 0, 0, 255));
        assert_eq!(pixel_rgba(root, 6, 5), (0, 0, 0, 0));
    }

    #[test]
    fn render_pixmap_rect_detects_exact_device_blit_when_scale_cancels() {
        let rect = Rect::new(1.0, 2.0, 2.0, 3.0);
        let pixmap = Pixmap::new(2, 2).expect("failed to create src pixmap");
        let local_transform = Affine::translate((rect.x0, rect.y0)).then_scale_non_uniform(
            rect.width() / pixmap.width() as f64,
            rect.height() / pixmap.height() as f64,
        );
        let composite_transform = Affine::scale(2.0) * local_transform;

        assert_eq!(
            integer_translation(composite_transform, 0.0, 0.0),
            Some((1, 2))
        );
    }

    #[test]
    fn image_quality_low_maps_to_nearest_filtering() {
        assert_eq!(
            image_quality_to_filter_quality(ImageQuality::Low),
            FilterQuality::Nearest
        );
        assert_eq!(
            image_quality_to_filter_quality(ImageQuality::Medium),
            FilterQuality::Bilinear
        );
        assert_eq!(
            image_quality_to_filter_quality(ImageQuality::High),
            FilterQuality::Bilinear
        );
    }

    #[test]
    fn nested_layer_marks_parent_draw_bounds() {
        let mut root = make_layer(8, 8);
        let mut parent = make_layer(8, 8);
        let mut child = make_layer(8, 8);

        child.fill(
            &Rect::new(2.0, 2.0, 4.0, 4.0),
            Color::from_rgb8(255, 0, 0),
            0.0,
        );

        apply_layer(&child, &mut parent);
        assert!(parent.draw_bounds.is_some());

        apply_layer(&parent, &mut root);
        assert_eq!(pixel_rgba(&root, 3, 3), (255, 0, 0, 255));
    }

    #[test]
    fn multipass_blend_respects_non_rect_clip_coverage() {
        let mut parent = make_layer(8, 8);
        parent.fill(
            &Rect::new(0.0, 0.0, 8.0, 8.0),
            Color::from_rgb8(0, 0, 255),
            0.0,
        );

        let mut child = make_layer(8, 8);
        child.blend_mode = BlendMode {
            mix: Mix::Difference,
            compose: Compose::SrcIn,
        };

        let mut builder = PathBuilder::new();
        builder.move_to(3.0, 0.0);
        builder.line_to(6.0, 3.0);
        builder.line_to(3.0, 6.0);
        builder.line_to(0.0, 3.0);
        builder.close();
        let clip_path = builder.finish().expect("failed to create clip path");
        child.set_base_clip(Some(ClipPath {
            path: clip_path,
            rect: Rect::new(0.0, 0.0, 6.0, 6.0),
            simple_rect: None,
        }));

        child.fill(
            &Rect::new(0.0, 0.0, 6.0, 6.0),
            Color::from_rgb8(255, 0, 0),
            0.0,
        );

        apply_layer(&child, &mut parent);

        assert_ne!(pixel_rgba(&parent, 3, 3), (0, 0, 255, 255));
        assert_eq!(pixel_rgba(&parent, 1, 1), (0, 0, 255, 255));
        assert_eq!(pixel_rgba(&parent, 6, 6), (0, 0, 255, 255));
    }

    #[test]
    fn path_clip_does_not_fall_back_to_bounding_box() {
        let mut renderer = TinySkiaRenderer::new_with_size(8, 8).expect("renderer");
        renderer.begin(8, 8);

        let mut clip = BezPath::new();
        clip.move_to((4.0, 0.0));
        clip.line_to((8.0, 4.0));
        clip.line_to((4.0, 8.0));
        clip.line_to((0.0, 4.0));
        clip.close_path();

        PaintSink::push_clip(&mut renderer, ClipRef::fill(clip));
        PaintSink::fill(
            &mut renderer,
            FillRef::new(Rect::new(0.0, 0.0, 8.0, 8.0), Color::from_rgb8(255, 0, 0)),
        );
        PaintSink::pop_clip(&mut renderer);

        let root = &renderer.layers[0];
        assert_eq!(pixel_rgba(root, 0, 0), (0, 0, 0, 0));
        assert_eq!(pixel_rgba(root, 7, 7), (0, 0, 0, 0));
        assert_eq!(pixel_rgba(root, 4, 4), (255, 0, 0, 255));
    }

    #[test]
    fn brush_transform_changes_gradient_sampling() {
        let gradient = Gradient::new_linear((0.0, 0.0), (8.0, 0.0)).with_stops([
            peniko::ColorStop {
                offset: 0.0,
                color: DynamicColor::from_alpha_color(Color::from_rgb8(255, 0, 0)),
            },
            peniko::ColorStop {
                offset: 1.0,
                color: DynamicColor::from_alpha_color(Color::from_rgb8(0, 0, 255)),
            },
        ]);

        let mut plain = make_layer(8, 2);
        plain.fill(&Rect::new(0.0, 0.0, 8.0, 2.0), &gradient, 0.0);

        let mut transformed = make_layer(8, 2);
        transformed.fill_with_brush_transform(
            &Rect::new(0.0, 0.0, 8.0, 2.0),
            &gradient,
            0.0,
            Some(Affine::translate((2.0, 0.0))),
        );

        assert_ne!(pixel_rgba(&plain, 2, 0), pixel_rgba(&transformed, 2, 0));
        assert_ne!(pixel_rgba(&plain, 6, 0), pixel_rgba(&transformed, 6, 0));
    }

    #[test]
    fn render_pixmap_direct_blends_premultiplied_pixels() {
        let mut layer = make_layer(4, 4);
        layer
            .pixmap
            .fill(tiny_skia::Color::from_rgba8(0, 0, 255, 255));

        let mut src = Pixmap::new(1, 1).expect("failed to create src pixmap");
        src.fill(tiny_skia::Color::from_rgba8(255, 0, 0, 128));

        layer.render_pixmap_direct(&src, 1.0, 1.0, Affine::IDENTITY, FilterQuality::Nearest);

        assert_eq!(pixel_rgba(&layer, 1, 1), (128, 0, 127, 255));
    }

    #[test]
    fn normalized_text_transform_keeps_translation_and_rotation_separate() {
        let transform = Affine::translate((30.0, 20.0))
            * Affine::rotate(std::f64::consts::FRAC_PI_2)
            * Affine::scale(2.0);

        let normalized = normalize_affine(transform, true);
        let (_, _, raster_scale) = affine_scale_components(transform);
        let device_origin = normalized * Point::new(5.0 * raster_scale, 0.0);

        assert!((device_origin.x - 30.0).abs() < 1e-6);
        assert!((device_origin.y - 30.0).abs() < 1e-6);
    }

    #[test]
    fn embolden_strength_scales_with_raster_scale() {
        assert!((scaled_embolden_strength(Vec2::new(0.2, 0.0), 1.5) - 0.3).abs() < f32::EPSILON);
        assert_eq!(scaled_embolden_strength(Vec2::new(0.2, 0.0), 0.0), 0.0);
    }

    #[test]
    fn text_raster_scale_has_two_x_floor() {
        assert_eq!(effective_text_raster_scale(0.0), 0.0);
        assert_eq!(effective_text_raster_scale(1.0), 2.0);
        assert_eq!(effective_text_raster_scale(1.5), 2.0);
        assert_eq!(effective_text_raster_scale(2.0), 2.0);
        assert_eq!(effective_text_raster_scale(3.0), 3.0);
    }

    #[test]
    fn glyph_cache_entries_get_a_minimum_ttl() {
        let now = Instant::now();
        let stale_but_recent = GlyphCacheEntry {
            cache_color: CacheColor(true),
            glyph: None,
            last_touched: now - Duration::from_millis(50),
        };
        let stale_and_old = GlyphCacheEntry {
            cache_color: CacheColor(true),
            glyph: None,
            last_touched: now - Duration::from_millis(150),
        };

        assert!(should_retain_glyph_entry(
            &stale_but_recent,
            CacheColor(false),
            now
        ));
        assert!(!should_retain_glyph_entry(
            &stale_and_old,
            CacheColor(false),
            now
        ));
        assert!(should_retain_glyph_entry(
            &stale_and_old,
            CacheColor(true),
            now
        ));
    }

    #[test]
    fn linear_gradient_honors_interpolation_color_space() {
        let mut layer = make_layer(101, 1);
        let gradient = Gradient::new_linear(Point::new(0.0, 0.0), Point::new(101.0, 0.0))
            .with_interpolation_cs(ColorSpaceTag::Oklab)
            .with_stops([(0.0, css::RED), (1.0, css::BLUE)]);

        layer.fill(&Rect::new(0.0, 0.0, 101.0, 1.0), &gradient, 0.0);

        let rendered = pixel_rgba(&layer, 50, 0);
        let expected_oklab = interpolated_midpoint(
            css::RED.into(),
            css::BLUE.into(),
            ColorSpaceTag::Oklab,
            HueDirection::Shorter,
        );
        let expected_srgb = interpolated_midpoint(
            css::RED.into(),
            css::BLUE.into(),
            ColorSpaceTag::Srgb,
            HueDirection::Shorter,
        );

        assert!(rgba_distance(rendered, expected_oklab) <= 10);
        assert!(rgba_distance(rendered, expected_srgb) >= 30);
    }

    #[test]
    fn linear_gradient_honors_hue_direction() {
        let mut layer = make_layer(101, 1);
        let gradient = Gradient::new_linear(Point::new(0.0, 0.0), Point::new(101.0, 0.0))
            .with_interpolation_cs(ColorSpaceTag::Oklch)
            .with_hue_direction(HueDirection::Longer)
            .with_stops([(0.0, css::RED), (1.0, css::BLUE)]);

        layer.fill(&Rect::new(0.0, 0.0, 101.0, 1.0), &gradient, 0.0);

        let rendered = pixel_rgba(&layer, 50, 0);
        let expected_longer = interpolated_midpoint(
            css::RED.into(),
            css::BLUE.into(),
            ColorSpaceTag::Oklch,
            HueDirection::Longer,
        );
        let expected_shorter = interpolated_midpoint(
            css::RED.into(),
            css::BLUE.into(),
            ColorSpaceTag::Oklch,
            HueDirection::Shorter,
        );

        assert!(rgba_distance(rendered, expected_longer) <= 10);
        assert!(rgba_distance(rendered, expected_shorter) >= 40);
    }

    #[test]
    fn target_renderer_supports_only_packed_rgba8_targets() {
        assert!(
            TinySkiaTargetRenderer::supports_target_info(&CpuBufferTargetInfo {
                width: 2,
                height: 2,
                bytes_per_row: 8,
                format: CpuBufferFormat::RGBA8_OPAQUE,
            })
            .is_ok()
        );

        assert!(
            TinySkiaTargetRenderer::supports_target_info(&CpuBufferTargetInfo {
                width: 2,
                height: 2,
                bytes_per_row: 8,
                format: CpuBufferFormat::BGRA8_OPAQUE,
            })
            .is_err()
        );

        assert!(
            TinySkiaTargetRenderer::supports_target_info(&CpuBufferTargetInfo {
                width: 2,
                height: 2,
                bytes_per_row: 16,
                format: CpuBufferFormat::RGBA8_OPAQUE,
            })
            .is_err()
        );
    }

    #[test]
    fn render_scene_replays_masked_group_content() {
        let scene = masked_scene(MaskMode::Alpha);
        let mut renderer = TinySkiaRenderer::new_with_size(64, 64).expect("renderer");
        renderer.begin(64, 64);

        imaging::record::replay(&scene, &mut renderer);

        let root = &renderer.layers[0];
        assert_eq!(pixel_rgba(root, 4, 4), (0, 0, 0, 0));
        let center = pixel_rgba(root, 16, 16);
        assert!(center.0 > 0 || center.1 > 0 || center.2 > 0);
        assert!(center.3 > 0);
    }
}
