// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

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
    BlurredRoundedRect, ClipRef, Composite, FillRef, Filter, GlyphRunRef, GroupRef, MaskMode,
    PaintSink, RgbaImage, StrokeRef,
    record::{Scene, ValidateError},
    render::{ImageRenderer, RenderSource},
};
use kurbo::{Affine, BezPath, Cap, Join, Point, Rect, Shape, Stroke as KurboStroke, Vec2};
use peniko::{
    BlendMode, BrushRef, Color, Compose, Extend, Gradient, GradientKind, ImageData, ImageQuality,
    InterpolationAlphaSpace, Mix, RadialGradientPosition,
    color::{self, ColorSpaceTag, DynamicColor, HueDirection, Srgb},
    kurbo::{PathEl, Size},
};
use rustc_hash::FxHashMap;
use std::{
    borrow::Borrow,
    collections::VecDeque,
    mem::Discriminant,
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
        let x_bin = u8::try_from(f32_to_i32((x_fract * 4.0).min(3.0).round()))
            .expect("x subpixel bin must fit in u8");
        let y_bin = u8::try_from(f32_to_i32((y_fract * 4.0).min(3.0).round()))
            .expect("y subpixel bin must fit in u8");
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

type ImageCacheMap = FxHashMap<u64, (CacheColor, Arc<Pixmap>)>;
type ScaledImageCacheMap = FxHashMap<ScaledImageCacheKey, (CacheColor, Arc<Pixmap>)>;
type BlurredRRectCacheMap = FxHashMap<BlurredRRectCacheKey, (CacheColor, Arc<Pixmap>)>;
type GlyphCacheMap = FxHashMap<(GlyphCacheKey, u32), GlyphCacheEntry>;

struct RendererCaches {
    image_cache: ImageCacheMap,
    scaled_image_cache: ScaledImageCacheMap,
    blurred_rrect_cache: BlurredRRectCacheMap,
    // The `u32` is a color encoded as a u32 so that it is hashable and eq.
    glyph_cache: GlyphCacheMap,
    scale_context: ScaleContext,
}

impl RendererCaches {
    fn new() -> Self {
        Self {
            image_cache: FxHashMap::default(),
            scaled_image_cache: FxHashMap::default(),
            blurred_rrect_cache: FxHashMap::default(),
            glyph_cache: FxHashMap::default(),
            scale_context: ScaleContext::new(),
        }
    }
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

fn cache_glyph(caches: &mut RendererCaches, request: GlyphRasterRequest<'_>) -> Option<Arc<Glyph>> {
    let c = request.color.to_rgba8();
    let now = Instant::now();

    if let Some(entry) = caches.glyph_cache.get_mut(&(request.cache_key, c.to_u32())) {
        entry.cache_color = request.cache_color;
        entry.last_touched = now;
        let opt_glyph = entry.glyph.clone();
        return opt_glyph;
    }

    let mut scaler = caches
        .scale_context
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
    let image = render.render(&mut scaler, request.cache_key.glyph_id)?;

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

    caches.glyph_cache.insert(
        (request.cache_key, c.to_u32()),
        GlyphCacheEntry {
            cache_color: request.cache_color,
            glyph: result.clone(),
            last_touched: now,
        },
    );

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
    stroke_source: Option<(Path, TinyStroke)>,
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
    quality: Discriminant<ImageQuality>,
}

#[derive(Hash, PartialEq, Eq)]
struct BlurredRRectCacheKey {
    x_bits: u64,
    y_bits: u64,
    width_bits: u64,
    height_bits: u64,
    radius_bits: u64,
    std_dev_bits: u64,
    color_rgba: u32,
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
    origin: Point,
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
    group_mask: Option<PendingGroupMask>,
    filters: Vec<Filter>,
    clip_applied_in_content: bool,
}

struct PendingGroupMask {
    scene: Scene,
    transform: Affine,
    mode: MaskMode,
}

struct GroupMask {
    bounds: IntRect,
    coverage: Arc<[u8]>,
}

#[derive(Clone, Debug)]
struct CachedMask {
    scene: Scene,
    mode: MaskMode,
    transform: Affine,
    bounds: (i32, i32, u32, u32),
    coverage: Arc<[u8]>,
}

impl CachedMask {
    fn matches(&self, scene: &Scene, mode: MaskMode, transform: Affine, bounds: IntRect) -> bool {
        self.scene == *scene
            && self.mode == mode
            && self.transform == transform
            && self.bounds == (bounds.x(), bounds.y(), bounds.width(), bounds.height())
    }
}
impl Layer<'static> {
    fn new_root(width: u32, height: u32) -> Result<Self> {
        Ok(Self {
            pixmap: LayerPixmap::Owned(
                Pixmap::new(width, height).ok_or(Error::Internal("unable to create pixmap"))?,
            ),
            origin: Point::ZERO,
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
            filters: Vec::new(),
            clip_applied_in_content: false,
        })
    }

    fn new_with_base_clip(
        blend_mode: BlendMode,
        alpha: f32,
        clip: ClipPath,
        origin: Point,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let mut layer = Self {
            pixmap: LayerPixmap::Owned(
                Pixmap::new(width, height).ok_or(Error::Internal("unable to create pixmap"))?,
            ),
            origin,
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
            filters: Vec::new(),
            clip_applied_in_content: false,
        };
        layer.rebuild_clip_mask();
        Ok(layer)
    }
}

impl<'a> Layer<'a> {
    fn all_clips<'b>(
        &'b self,
        extra_clips: &'b [ClipPath],
    ) -> impl Iterator<Item = &'b ClipPath> + 'b {
        self.base_clip
            .iter()
            .chain(self.clip_stack.iter())
            .chain(extra_clips.iter())
    }

    fn active_mask(&self) -> Option<&Mask> {
        (self.clip.is_some() && self.mask_valid).then_some(&self.mask)
    }

    fn new_root_borrowed(data: &'a mut [u8], width: u32, height: u32) -> Result<Self> {
        Ok(Self {
            pixmap: LayerPixmap::Borrowed(
                PixmapMut::from_bytes(data, width, height)
                    .ok_or(Error::Internal("unable to wrap target pixmap"))?,
            ),
            origin: Point::ZERO,
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
            filters: Vec::new(),
            clip_applied_in_content: false,
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

    fn clip_mask_from_stroke_source(mask: &Mask, path: &Path, stroke: &TinyStroke) -> Option<Mask> {
        let mut pixmap = Pixmap::new(mask.width(), mask.height())?;
        let paint = Paint {
            shader: Shader::SolidColor(tiny_skia::Color::WHITE),
            anti_alias: true,
            ..Default::default()
        };
        pixmap.stroke_path(path, &paint, stroke, Transform::identity(), None);
        Some(Mask::from_pixmap(pixmap.as_ref(), MaskType::Alpha))
    }

    fn fill_mask_from_clip(mask: &mut Mask, clip: &ClipPath) {
        if let Some((path, stroke)) = &clip.stroke_source {
            mask.clear();
            if let Some(stroke_mask) = Self::clip_mask_from_stroke_source(mask, path, stroke) {
                mask.data_mut().copy_from_slice(stroke_mask.data());
            }
            return;
        }

        if clip.simple_rect.is_some() {
            Self::write_mask_rect_to(mask, clip.rect);
        } else {
            mask.fill_path(&clip.path, FillRule::Winding, true, Transform::identity());
        }
    }

    fn intersect_mask_with_clip(mask: &mut Mask, clip: &ClipPath) {
        if let Some(simple_rect) = clip.simple_rect {
            Self::intersect_mask_rect_in(mask, simple_rect);
            return;
        }
        if let Some((path, stroke)) = &clip.stroke_source {
            let Some(stroke_mask) = Self::clip_mask_from_stroke_source(mask, path, stroke) else {
                mask.clear();
                return;
            };
            for (dst, src) in mask.data_mut().iter_mut().zip(stroke_mask.data().iter()) {
                *dst = mul_div_255(*dst, *src);
            }
            return;
        }
        mask.intersect_path(&clip.path, FillRule::Winding, true, Transform::identity());
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
            Self::intersect_mask_with_clip(&mut self.mask, clip);
        } else {
            if let Some(simple_clip) = prior_simple_clip {
                self.fill_mask_rect(simple_clip);
                Self::intersect_mask_with_clip(&mut self.mask, clip);
            } else {
                Self::fill_mask_from_clip(&mut self.mask, clip);
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
        } else {
            Self::fill_mask_from_clip(mask, first);
        }

        for clip in clips {
            Self::intersect_mask_with_clip(mask, clip);
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
        self.all_clips(&[]).cloned().collect()
    }

    fn effective_clips_in_root(&self) -> Vec<ClipPath> {
        self.all_clips(&[])
            .map(|clip| translate_clip_path(clip, self.origin.x, self.origin.y))
            .collect()
    }

    fn clip_in_root(&self) -> Option<Rect> {
        self.clip
            .map(|clip| translate_rect(clip, self.origin.x, self.origin.y))
    }

    fn mark_drawn_local_rect(&mut self, rect: Rect) {
        let mut root_rect = translate_rect(rect, self.origin.x, self.origin.y);
        if let Some(clip) = self.clip_in_root() {
            root_rect = root_rect.intersect(clip);
        }

        if root_rect.is_zero_area() {
            return;
        }

        self.draw_bounds = Some(
            self.draw_bounds
                .map(|bounds| bounds.union(root_rect))
                .unwrap_or(root_rect),
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

        self.mark_drawn_local_rect(Rect::new(
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
        Affine::translate((-self.origin.x, -self.origin.y)) * self.transform
    }

    fn intersects_clip(&self, img_rect: Rect, transform: Affine) -> bool {
        let device_rect = transform.transform_rect_bbox(img_rect);
        self.clip
            .map(|clip| to_skia_rect(clip.intersect(device_rect)).is_some())
            .unwrap_or(true)
    }

    fn mark_drawn_rect_inflated(&mut self, rect: Rect, transform: Affine, pad: f64) {
        let mut root_rect = transform.transform_rect_bbox(rect).inset(-pad);
        if let Some(clip) = self.clip_in_root() {
            root_rect = root_rect.intersect(clip);
        }
        if root_rect.is_zero_area() {
            return;
        }
        self.draw_bounds = Some(
            self.draw_bounds
                .map(|bounds| bounds.union(root_rect))
                .unwrap_or(root_rect),
        );
    }

    fn mark_stroke_bounds(&mut self, shape: &impl Shape, stroke: &KurboStroke) {
        if let Some(clip) = self.clip_in_root() {
            self.mark_drawn_rect_inflated(clip, Affine::IDENTITY, 0.0);
            return;
        }

        let stroke_pad = stroke.width + stroke.miter_limit.max(1.0) + 4.0;
        self.mark_drawn_rect_inflated(shape.bounding_box().inset(-stroke_pad), self.transform, 4.0);
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

        self.mark_drawn_rect_inflated(
            translate_rect(rect, self.origin.x, self.origin.y),
            Affine::IDENTITY,
            2.0,
        );
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
        self.draw_pixmap_rect(
            pixmap,
            rect,
            transform,
            PixmapPaint {
                opacity: 1.0,
                blend_mode: tiny_skia::BlendMode::SourceOver,
                quality: image_quality_to_filter_quality(quality),
            },
        );
    }

    fn draw_pixmap_rect(
        &mut self,
        pixmap: &Pixmap,
        rect: Rect,
        transform: Affine,
        paint: PixmapPaint,
    ) {
        let local_transform = Affine::translate((rect.x0, rect.y0)).then_scale_non_uniform(
            rect.width() / pixmap.width() as f64,
            rect.height() / pixmap.height() as f64,
        );
        let composite_transform = transform * local_transform;

        if self.try_draw_pixmap_translate_only(pixmap, 0.0, 0.0, composite_transform, paint.quality)
        {
            return;
        }

        if !self.intersects_clip(rect, transform) {
            return;
        }
        self.mark_drawn_rect_inflated(rect, transform, 2.0);
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

    fn try_fill_gradient_fallback(
        &mut self,
        shape: &impl Shape,
        gradient: &Gradient,
        brush_transform: Option<Affine>,
        opacity: f32,
        blend_mode: TinyBlendMode,
    ) -> bool {
        let stops = expand_gradient_stops(gradient, opacity);
        if stops.is_empty() {
            return true;
        }
        let kind = gradient.kind;
        let needs_fallback = match kind {
            GradientKind::Sweep(_) => true,
            GradientKind::Radial(radial) => radial.start_radius > 0.0,
            GradientKind::Linear(_) => false,
        };
        if !needs_fallback {
            return false;
        }
        let Some(path) = shape_to_path(shape) else {
            return false;
        };

        let device_bounds = self
            .device_transform()
            .transform_rect_bbox(shape.bounding_box());
        let device_bounds = self
            .clip
            .map_or(device_bounds, |clip| clip.intersect(device_bounds));
        let Some(bounds) = rect_to_int_rect(device_bounds) else {
            return true;
        };
        let x0 = bounds.x().max(0);
        let y0 = bounds.y().max(0);
        let x1 = (bounds.x() + i32::try_from(bounds.width()).expect("width fits in i32"))
            .min(i32::try_from(self.pixmap.width()).expect("pixmap width fits in i32"));
        let y1 = (bounds.y() + i32::try_from(bounds.height()).expect("height fits in i32"))
            .min(i32::try_from(self.pixmap.height()).expect("pixmap height fits in i32"));
        if x0 >= x1 || y0 >= y1 {
            return true;
        }

        let local_width = u32::try_from(x1 - x0).expect("local width fits u32");
        let local_height = u32::try_from(y1 - y0).expect("local height fits u32");
        let mut coverage = match Mask::new(local_width, local_height) {
            Some(mask) => mask,
            None => return false,
        };
        coverage.fill_path(
            &path,
            FillRule::Winding,
            true,
            self.skia_transform()
                .post_translate(-(x0 as f32), -(y0 as f32)),
        );

        let mut pixmap = match Pixmap::new(local_width, local_height) {
            Some(pixmap) => pixmap,
            None => return false,
        };
        let sample_transform = brush_transform.unwrap_or(Affine::IDENTITY).inverse();
        let sample_origin = sample_transform * Point::new(f64::from(x0) + 0.5, f64::from(y0) + 0.5);
        let sample_dx = Vec2::new(
            sample_transform.as_coeffs()[0],
            sample_transform.as_coeffs()[1],
        );
        let sample_dy = Vec2::new(
            sample_transform.as_coeffs()[2],
            sample_transform.as_coeffs()[3],
        );
        let coverage_data = coverage.data();
        let pixels = pixmap.pixels_mut();
        let stride = usize_from_u32(local_width);
        match kind {
            GradientKind::Sweep(sweep) => {
                let prepared = PreparedSweep::new(sweep);
                let sweep_lut = build_expanded_gradient_lut_256(&stops, gradient.extend);
                let (mut row_x, mut row_y) = prepared.project_point(sample_origin);
                let (step_x_x, step_x_y) = prepared.project_delta(sample_dx);
                let (step_y_x, step_y_y) = prepared.project_delta(sample_dy);
                for local_y in 0..local_height {
                    let mut x = row_x;
                    let mut y = row_y;
                    for local_x in 0..local_width {
                        let idx = usize_from_u32(local_y) * stride + usize_from_u32(local_x);
                        let coverage_alpha = coverage_data[idx];
                        if coverage_alpha != 0 {
                            let t = prepared.sample(x, y);
                            let color = sweep_lut[sweep_lut_index_256(gradient.extend, t)];
                            let rgba = color.to_color_u8();
                            pixels[idx] = tiny_skia::Color::from_rgba8(
                                rgba.red(),
                                rgba.green(),
                                rgba.blue(),
                                mul_div_255(rgba.alpha(), coverage_alpha),
                            )
                            .premultiply()
                            .to_color_u8();
                        }
                        x += step_x_x;
                        y += step_x_y;
                    }
                    row_x += step_y_x;
                    row_y += step_y_y;
                }
            }
            GradientKind::Radial(radial) => {
                let prepared = prepare_two_point_radial(radial);
                let radial_lut = build_original_gradient_lut_1024(gradient, opacity);
                let (mut row_x, mut row_y) = prepared.project_point(sample_origin);
                let (step_x_x, step_x_y) = prepared.project_delta(sample_dx);
                let (step_y_x, step_y_y) = prepared.project_delta(sample_dy);
                for local_y in 0..local_height {
                    let mut x = row_x;
                    let mut y = row_y;
                    for local_x in 0..local_width {
                        let idx = usize_from_u32(local_y) * stride + usize_from_u32(local_x);
                        let coverage_alpha = coverage_data[idx];
                        if coverage_alpha != 0 {
                            let t = prepared.sample(x, y);
                            let color = radial_lut[gradient_lut_index_1024(gradient.extend, t)];
                            let rgba = color.to_color_u8();
                            pixels[idx] = tiny_skia::Color::from_rgba8(
                                rgba.red(),
                                rgba.green(),
                                rgba.blue(),
                                mul_div_255(rgba.alpha(), coverage_alpha),
                            )
                            .premultiply()
                            .to_color_u8();
                        }
                        x += step_x_x;
                        y += step_x_y;
                    }
                    row_x += step_y_x;
                    row_y += step_y_y;
                }
            }
            GradientKind::Linear(_) => unreachable!("checked unsupported fallback kinds"),
        }

        self.mark_drawn_rect_inflated(shape.bounding_box(), self.transform, 2.0);
        self.materialize_simple_clip_mask();
        let clip_mask = self.clip.is_some().then_some(&self.mask);
        self.pixmap.draw_pixmap(
            x0,
            y0,
            pixmap.as_ref(),
            &PixmapPaint {
                opacity: 1.0,
                blend_mode,
                quality: FilterQuality::Nearest,
            },
            Transform::identity(),
            clip_mask,
        );
        true
    }

    fn try_fill_image_fallback<T>(
        &mut self,
        shape: &impl Shape,
        image_pixmap: &Pixmap,
        image: &peniko::ImageBrush<T>,
        brush_transform: Option<Affine>,
        opacity: f32,
        blend_mode: TinyBlendMode,
    ) -> bool
    where
        T: Borrow<ImageData>,
    {
        if !image_brush_has_mixed_extend(image) {
            return false;
        }
        let Some(path) = shape_to_path(shape) else {
            return false;
        };

        let device_bounds = self
            .device_transform()
            .transform_rect_bbox(shape.bounding_box());
        let device_bounds = self
            .clip
            .map_or(device_bounds, |clip| clip.intersect(device_bounds));
        let Some(bounds) = rect_to_int_rect(device_bounds) else {
            return true;
        };
        let x0 = bounds.x().max(0);
        let y0 = bounds.y().max(0);
        let x1 = (bounds.x() + i32::try_from(bounds.width()).expect("width fits in i32"))
            .min(i32::try_from(self.pixmap.width()).expect("pixmap width fits in i32"));
        let y1 = (bounds.y() + i32::try_from(bounds.height()).expect("height fits in i32"))
            .min(i32::try_from(self.pixmap.height()).expect("pixmap height fits in i32"));
        if x0 >= x1 || y0 >= y1 {
            return true;
        }

        let local_width = u32::try_from(x1 - x0).expect("local width fits u32");
        let local_height = u32::try_from(y1 - y0).expect("local height fits u32");
        let mut coverage = match Mask::new(local_width, local_height) {
            Some(mask) => mask,
            None => return false,
        };
        coverage.fill_path(
            &path,
            FillRule::Winding,
            true,
            self.skia_transform()
                .post_translate(-(x0 as f32), -(y0 as f32)),
        );

        let mut pixmap = match Pixmap::new(local_width, local_height) {
            Some(pixmap) => pixmap,
            None => return false,
        };
        let sample_transform = brush_transform.unwrap_or(Affine::IDENTITY);
        let sample_origin = sample_transform * Point::new(f64::from(x0) + 0.5, f64::from(y0) + 0.5);
        let sample_dx = Vec2::new(
            sample_transform.as_coeffs()[0],
            sample_transform.as_coeffs()[1],
        );
        let sample_dy = Vec2::new(
            sample_transform.as_coeffs()[2],
            sample_transform.as_coeffs()[3],
        );
        let coverage_data = coverage.data();
        let pixels = pixmap.pixels_mut();
        let stride = usize_from_u32(local_width);
        let alpha = opacity_to_u8(opacity);

        if sample_dx.y == 0.0 && sample_dy.x == 0.0 {
            let source_pixels = image_pixmap.pixels();
            let source_width = usize_from_u32(image_pixmap.width());
            match image_quality_to_filter_quality(image.sampler.quality) {
                FilterQuality::Nearest => {
                    let x_samples = nearest_axis_samples(
                        f64_to_f32(sample_origin.x),
                        f64_to_f32(sample_dx.x),
                        local_width,
                        image_pixmap.width(),
                        image.sampler.x_extend,
                    );
                    let y_samples = nearest_axis_samples(
                        f64_to_f32(sample_origin.y),
                        f64_to_f32(sample_dy.y),
                        local_height,
                        image_pixmap.height(),
                        image.sampler.y_extend,
                    );
                    for local_y in 0..local_height {
                        let y_sample = y_samples[usize_from_u32(local_y)];
                        for local_x in 0..local_width {
                            let idx = usize_from_u32(local_y) * stride + usize_from_u32(local_x);
                            let coverage_alpha = coverage_data[idx];
                            if coverage_alpha == 0 {
                                continue;
                            }
                            let x_sample = x_samples[usize_from_u32(local_x)];
                            let sampled = pixmap_pixel_or_transparent(
                                source_pixels,
                                source_width,
                                x_sample.index,
                                y_sample.index,
                            );
                            pixels[idx] = scale_premultiplied_color(
                                sampled,
                                mul_div_255(alpha, coverage_alpha),
                            );
                        }
                    }
                    self.mark_drawn_rect_inflated(shape.bounding_box(), self.transform, 2.0);
                    self.materialize_simple_clip_mask();
                    let clip_mask = self.clip.is_some().then_some(&self.mask);
                    self.pixmap.draw_pixmap(
                        x0,
                        y0,
                        pixmap.as_ref(),
                        &PixmapPaint {
                            opacity: 1.0,
                            blend_mode,
                            quality: FilterQuality::Nearest,
                        },
                        Transform::identity(),
                        clip_mask,
                    );
                    return true;
                }
                FilterQuality::Bilinear => {
                    let x_samples = bilinear_axis_samples(
                        f64_to_f32(sample_origin.x),
                        f64_to_f32(sample_dx.x),
                        local_width,
                        image_pixmap.width(),
                        image.sampler.x_extend,
                    );
                    let y_samples = bilinear_axis_samples(
                        f64_to_f32(sample_origin.y),
                        f64_to_f32(sample_dy.y),
                        local_height,
                        image_pixmap.height(),
                        image.sampler.y_extend,
                    );
                    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
                    for local_y in 0..local_height {
                        let y_sample = y_samples[usize_from_u32(local_y)];
                        for local_x in 0..local_width {
                            let idx = usize_from_u32(local_y) * stride + usize_from_u32(local_x);
                            let coverage_alpha = coverage_data[idx];
                            if coverage_alpha == 0 {
                                continue;
                            }
                            let x_sample = x_samples[usize_from_u32(local_x)];
                            let c00 = premul_to_rgba_f32(pixmap_pixel_or_transparent(
                                source_pixels,
                                source_width,
                                x_sample.i0,
                                y_sample.i0,
                            ));
                            let c10 = premul_to_rgba_f32(pixmap_pixel_or_transparent(
                                source_pixels,
                                source_width,
                                x_sample.i1,
                                y_sample.i0,
                            ));
                            let c01 = premul_to_rgba_f32(pixmap_pixel_or_transparent(
                                source_pixels,
                                source_width,
                                x_sample.i0,
                                y_sample.i1,
                            ));
                            let c11 = premul_to_rgba_f32(pixmap_pixel_or_transparent(
                                source_pixels,
                                source_width,
                                x_sample.i1,
                                y_sample.i1,
                            ));
                            let mut rgba = [0.0; 4];
                            for i in 0..4 {
                                let top = lerp(c00[i], c10[i], x_sample.t);
                                let bottom = lerp(c01[i], c11[i], x_sample.t);
                                rgba[i] = lerp(top, bottom, y_sample.t);
                            }
                            let sampled = premul_from_rgba_f32(rgba);
                            pixels[idx] = scale_premultiplied_color(
                                sampled,
                                mul_div_255(alpha, coverage_alpha),
                            );
                        }
                    }
                    self.mark_drawn_rect_inflated(shape.bounding_box(), self.transform, 2.0);
                    self.materialize_simple_clip_mask();
                    let clip_mask = self.clip.is_some().then_some(&self.mask);
                    self.pixmap.draw_pixmap(
                        x0,
                        y0,
                        pixmap.as_ref(),
                        &PixmapPaint {
                            opacity: 1.0,
                            blend_mode,
                            quality: FilterQuality::Nearest,
                        },
                        Transform::identity(),
                        clip_mask,
                    );
                    return true;
                }
                FilterQuality::Bicubic => {}
            }
        }

        let mut row_point = sample_origin;
        for local_y in 0..local_height {
            let mut point = row_point;
            for local_x in 0..local_width {
                let idx = usize_from_u32(local_y) * stride + usize_from_u32(local_x);
                let coverage_alpha = coverage_data[idx];
                if coverage_alpha != 0 {
                    let sampled = sample_image_brush_at(image_pixmap, image, point);
                    pixels[idx] =
                        scale_premultiplied_color(sampled, mul_div_255(alpha, coverage_alpha));
                }
                point += sample_dx;
            }
            row_point += sample_dy;
        }

        self.mark_drawn_rect_inflated(shape.bounding_box(), self.transform, 2.0);
        self.materialize_simple_clip_mask();
        let clip_mask = self.clip.is_some().then_some(&self.mask);
        self.pixmap.draw_pixmap(
            x0,
            y0,
            pixmap.as_ref(),
            &PixmapPaint {
                opacity: 1.0,
                blend_mode,
                quality: FilterQuality::Nearest,
            },
            Transform::identity(),
            clip_mask,
        );
        true
    }

    fn fill_image_with_pixmap_and_mode<T>(
        &mut self,
        shape: &impl Shape,
        image_pixmap: &Pixmap,
        image: &peniko::ImageBrush<T>,
        brush_transform: Option<Affine>,
        opacity: f32,
        blend_mode: TinyBlendMode,
    ) where
        T: Borrow<ImageData>,
    {
        if self.try_fill_image_fallback(
            shape,
            image_pixmap,
            image,
            brush_transform,
            opacity,
            blend_mode,
        ) {
            return;
        }

        let paint = Paint {
            shader: Pattern::new(
                image_pixmap.as_ref(),
                image_brush_spread_mode(image),
                image_quality_to_filter_quality(image.sampler.quality),
                image.sampler.alpha * opacity,
                affine_to_skia(brush_transform.unwrap_or(Affine::IDENTITY)),
            ),
            blend_mode,
            ..Default::default()
        };
        self.mark_drawn_rect_inflated(shape.bounding_box(), self.transform, 2.0);
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

    fn stroke_image_with_pixmap_and_mode<T>(
        &mut self,
        shape: &impl Shape,
        image_pixmap: &Pixmap,
        image: &peniko::ImageBrush<T>,
        stroke: &KurboStroke,
        brush_transform: Option<Affine>,
        opacity: f32,
        blend_mode: TinyBlendMode,
    ) where
        T: Borrow<ImageData>,
    {
        if image_brush_has_mixed_extend(image) {
            return;
        }

        let path = try_ret!(shape_to_path(shape));
        self.mark_stroke_bounds(shape, stroke);
        let stroke = kurbo_stroke_to_tiny_stroke(stroke);
        self.materialize_simple_clip_mask();
        let clip_mask = self.clip.is_some().then_some(&self.mask);
        let paint = Paint {
            shader: Pattern::new(
                image_pixmap.as_ref(),
                image_brush_spread_mode(image),
                image_quality_to_filter_quality(image.sampler.quality),
                image.sampler.alpha * opacity,
                affine_to_skia(brush_transform.unwrap_or(Affine::IDENTITY)),
            ),
            blend_mode,
            ..Default::default()
        };
        self.pixmap
            .stroke_path(&path, &paint, &stroke, self.skia_transform(), clip_mask);
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
            stroke_source: None,
        }));
    }
    fn stroke_with_brush_transform<'b, 's>(
        &mut self,
        shape: &impl Shape,
        brush: impl Into<BrushRef<'b>>,
        stroke: &'s KurboStroke,
        brush_transform: Option<Affine>,
    ) {
        self.stroke_with_brush_transform_and_mode(
            shape,
            brush,
            stroke,
            brush_transform,
            1.0,
            TinyBlendMode::SourceOver,
        );
    }

    fn stroke_with_brush_transform_and_mode<'b, 's>(
        &mut self,
        shape: &impl Shape,
        brush: impl Into<BrushRef<'b>>,
        stroke: &'s KurboStroke,
        brush_transform: Option<Affine>,
        opacity: f32,
        blend_mode: TinyBlendMode,
    ) {
        let path = try_ret!(shape_to_path(shape));
        let brush = brush.into();
        if let BrushRef::Image(image) = brush {
            let image_pixmap = try_ret!(image_brush_pixmap(&image));
            self.stroke_image_with_pixmap_and_mode(
                shape,
                &image_pixmap,
                &image,
                stroke,
                brush_transform,
                opacity,
                blend_mode,
            );
            return;
        }

        self.mark_stroke_bounds(shape, stroke);
        let stroke = kurbo_stroke_to_tiny_stroke(stroke);
        self.materialize_simple_clip_mask();
        let clip_mask = self.clip.is_some().then_some(&self.mask);
        let paint = try_ret!(brush_to_paint(brush, brush_transform, opacity, blend_mode));
        self.pixmap
            .stroke_path(&path, &paint, &stroke, self.skia_transform(), clip_mask);
    }

    fn fill<'b>(&mut self, shape: &impl Shape, brush: impl Into<BrushRef<'b>>) {
        self.fill_with_brush_transform(shape, brush, None);
    }

    fn fill_with_brush_transform<'b>(
        &mut self,
        shape: &impl Shape,
        brush: impl Into<BrushRef<'b>>,
        brush_transform: Option<Affine>,
    ) {
        self.fill_with_brush_transform_and_mode(
            shape,
            brush,
            brush_transform,
            1.0,
            TinyBlendMode::SourceOver,
        );
    }

    fn fill_with_brush_transform_and_mode<'b>(
        &mut self,
        shape: &impl Shape,
        brush: impl Into<BrushRef<'b>>,
        brush_transform: Option<Affine>,
        opacity: f32,
        blend_mode: TinyBlendMode,
    ) {
        let brush = brush.into();
        if let BrushRef::Image(image) = brush {
            let image_pixmap = try_ret!(image_brush_pixmap(&image));
            self.fill_image_with_pixmap_and_mode(
                shape,
                &image_pixmap,
                &image,
                brush_transform,
                opacity,
                blend_mode,
            );
            return;
        }

        if let Some(rect) = shape.as_rect()
            && let BrushRef::Solid(color) = brush
            && blend_mode == TinyBlendMode::SourceOver
            && opacity >= 1.0
            && self.try_fill_solid_rect_fast(rect, color)
        {
            return;
        }

        if let BrushRef::Gradient(gradient) = brush
            && self.try_fill_gradient_fallback(
                shape,
                gradient,
                brush_transform,
                opacity,
                blend_mode,
            )
        {
            return;
        }

        let paint = try_ret!(brush_to_paint(brush, brush_transform, opacity, blend_mode));
        self.mark_drawn_rect_inflated(shape.bounding_box(), self.transform, 2.0);
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
    caches: RendererCaches,
    cache_color: CacheColor,
    transform: Affine,
    mask_cache: VecDeque<CachedMask>,
    layers: Vec<Layer<'a>>,
    group_frames: Vec<GroupFrame>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GroupFrame {
    Direct { pushed_clip: bool },
    Isolated,
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
                caches: RendererCaches::new(),
                transform: Affine::IDENTITY,
                cache_color: CacheColor(false),
                mask_cache: VecDeque::new(),
                layers: vec![Layer::new_root_borrowed(
                    target.buffer,
                    target.width,
                    target.height,
                )?],
                group_frames: Vec::new(),
            };
            inner.clear_root_layer();
            return Ok(Self { inner });
        }

        Err(Error::UnsupportedTargetFormat)
    }
}

fn translate_clip_path(clip: &ClipPath, dx: f64, dy: f64) -> ClipPath {
    let path = clip
        .path
        .clone()
        .transform(Transform::from_translate(f64_to_f32(dx), f64_to_f32(dy)))
        .expect("translation should preserve clip path");
    let stroke_source = clip.stroke_source.as_ref().map(|(path, stroke)| {
        (
            path.clone()
                .transform(Transform::from_translate(f64_to_f32(dx), f64_to_f32(dy)))
                .expect("translation should preserve stroke clip path"),
            stroke.clone(),
        )
    });
    ClipPath {
        path,
        rect: translate_rect(clip.rect, dx, dy),
        simple_rect: clip.simple_rect.map(|rect| translate_rect(rect, dx, dy)),
        stroke_source,
    }
}

fn translate_clip_path_in_place(clip: &mut ClipPath, dx: f64, dy: f64) {
    *clip = translate_clip_path(clip, dx, dy);
}

fn translate_clip_paths_in_place(clips: &mut [ClipPath], dx: f64, dy: f64) {
    for clip in clips {
        translate_clip_path_in_place(clip, dx, dy);
    }
}

impl<'a> TinySkiaRendererImpl<'a> {
    fn simple_inherited_clips(clips: &[ClipPath]) -> Vec<ClipPath> {
        clips
            .iter()
            .filter(|clip| clip.simple_rect.is_some())
            .cloned()
            .collect()
    }

    fn group_requires_isolation(group: &GroupRef<'_>) -> bool {
        group.composite != Composite::default() || !group.filters.is_empty() || group.mask.is_some()
    }

    fn default_group_clip(&self) -> Option<ClipPath> {
        let rect = self.canvas_size().to_rect();
        self.clip_path_for_geometry(imaging::GeometryRef::Rect(rect), Affine::IDENTITY)
    }

    fn group_clip_path(&self, clip: Option<ClipRef<'_>>) -> Option<ClipPath> {
        match clip {
            Some(clip) => self.clip_path_for_clip(clip),
            None => self.default_group_clip(),
        }
    }

    fn pending_group_mask(
        &self,
        mask: Option<imaging::AppliedMaskRef<'_>>,
    ) -> Option<PendingGroupMask> {
        mask.map(|mask| PendingGroupMask {
            scene: mask.mask.scene.clone(),
            transform: mask.transform,
            mode: mask.mask.mode,
        })
    }

    fn build_group_child_layer(
        &self,
        composite: Composite,
        filters: &[Filter],
        group_mask: Option<PendingGroupMask>,
        clip: ClipPath,
    ) -> Option<Layer<'static>> {
        let parent_origin = self.current_layer().origin;
        let clip = translate_clip_path(&clip, parent_origin.x, parent_origin.y);
        let inherited_clips = self.current_layer().effective_clips_in_root();
        let mut group_bounds = clip.rect;
        for inherited in &inherited_clips {
            group_bounds = group_bounds.intersect(inherited.rect);
        }
        group_bounds = group_bounds.intersect(self.canvas_size().to_rect());
        let group_bounds = rect_to_int_rect(group_bounds)?;
        let group_origin = Point::new(f64::from(group_bounds.x()), f64::from(group_bounds.y()));
        let clip = translate_clip_path(&clip, -group_origin.x, -group_origin.y);
        let mut applied_inherited_clips = Self::simple_inherited_clips(&inherited_clips);
        let omitted_non_simple = applied_inherited_clips.len() != inherited_clips.len();
        translate_clip_paths_in_place(
            &mut applied_inherited_clips,
            -group_origin.x,
            -group_origin.y,
        );
        let mut child = Layer::new_with_base_clip(
            composite.blend,
            composite.alpha,
            clip,
            group_origin,
            group_bounds.width(),
            group_bounds.height(),
        )
        .ok()?;
        child.rebuild_clip_mask_with_extra_clips(&applied_inherited_clips);
        child.group_mask = group_mask;
        child.filters = filters.to_vec();
        child.clip_applied_in_content = !omitted_non_simple;
        Some(child)
    }

    fn finish_popped_group(&mut self, mut child: Layer<'_>) {
        let mask_bounds = child
            .group_mask
            .as_ref()
            .and_then(|_| group_mask_bounds(&child));

        if self.try_pop_group_with_localized_mask(&mut child, mask_bounds) {
            return;
        }

        self.apply_deferred_group_mask(&mut child, mask_bounds);
        apply_group_filters(&mut child);
        let parent = self.current_layer_mut();
        apply_layer(&child, parent);
    }

    fn try_pop_group_with_localized_mask(
        &mut self,
        child: &mut Layer<'_>,
        mask_bounds: Option<IntRect>,
    ) -> bool {
        if !child.filters.is_empty() {
            return false;
        }
        let Some(group_mask) = child.group_mask.take() else {
            return false;
        };
        let Some(bounds) = mask_bounds else {
            return false;
        };
        let BlendStrategy::SinglePass(blend_mode) = determine_blend_strategy(&child.blend_mode)
        else {
            child.group_mask = Some(group_mask);
            return false;
        };
        let Some(local_bounds) = root_rect_to_local_int_rect(child, bounds) else {
            child.group_mask = Some(group_mask);
            return false;
        };
        let Some(mut localized_child) = clone_pixmap_region(child.pixmap.as_ref(), local_bounds)
        else {
            child.group_mask = Some(group_mask);
            return false;
        };
        let Some(localized_mask) = self.realize_group_mask(
            &group_mask.scene,
            group_mask.mode,
            group_mask.transform,
            bounds,
        ) else {
            child.group_mask = Some(group_mask);
            return false;
        };

        let local_bounds = IntRect::from_xywh(
            0,
            0,
            localized_mask.bounds.width(),
            localized_mask.bounds.height(),
        )
        .expect("localized mask bounds must be valid");
        apply_group_mask_to_pixmap(
            &mut localized_child,
            &GroupMask {
                bounds: local_bounds,
                coverage: localized_mask.coverage,
            },
        );
        let parent = self.current_layer_mut();
        draw_layer_pixmap(
            &localized_child,
            bounds.x(),
            bounds.y(),
            parent,
            blend_mode,
            child.alpha,
            false,
        );
        true
    }

    fn apply_deferred_group_mask(&mut self, child: &mut Layer<'_>, mask_bounds: Option<IntRect>) {
        let Some(group_mask) = child.group_mask.take() else {
            return;
        };
        let Some(bounds) = mask_bounds else {
            return;
        };
        let Some(localized_mask) = self.realize_group_mask(
            &group_mask.scene,
            group_mask.mode,
            group_mask.transform,
            bounds,
        ) else {
            return;
        };
        apply_group_mask(child, &localized_mask);
    }

    fn lookup_cached_mask(
        &mut self,
        scene: &Scene,
        mode: MaskMode,
        transform: Affine,
        bounds: IntRect,
    ) -> Option<Arc<[u8]>> {
        let index = self
            .mask_cache
            .iter()
            .position(|entry| entry.matches(scene, mode, transform, bounds))?;
        let coverage = self.mask_cache.get(index)?.coverage.clone();
        if index + 1 != self.mask_cache.len()
            && let Some(entry) = self.mask_cache.remove(index)
        {
            self.mask_cache.push_back(entry);
        }
        Some(coverage)
    }

    fn store_cached_mask(
        &mut self,
        scene: &Scene,
        mode: MaskMode,
        transform: Affine,
        bounds: IntRect,
        coverage: Arc<[u8]>,
    ) {
        if let Some(index) = self
            .mask_cache
            .iter()
            .position(|entry| entry.matches(scene, mode, transform, bounds))
        {
            let _ = self.mask_cache.remove(index);
        }
        self.mask_cache.push_back(CachedMask {
            scene: scene.clone(),
            mode,
            transform,
            bounds: (bounds.x(), bounds.y(), bounds.width(), bounds.height()),
            coverage,
        });
        while self.mask_cache.len() > 16 {
            let _ = self.mask_cache.pop_front();
        }
    }

    fn realize_group_mask(
        &mut self,
        scene: &Scene,
        mode: MaskMode,
        transform: Affine,
        bounds: IntRect,
    ) -> Option<GroupMask> {
        if let Some(coverage) = self.lookup_cached_mask(scene, mode, transform, bounds) {
            return Some(GroupMask { bounds, coverage });
        }
        let pixmap = rasterize_scene_mask(scene, bounds, transform)?;
        let coverage = coverage_from_mask_pixmap(pixmap.as_ref(), mode);
        self.store_cached_mask(scene, mode, transform, bounds, coverage.clone());
        Some(GroupMask { bounds, coverage })
    }

    fn clip_path_for_clip(&self, clip: ClipRef<'_>) -> Option<ClipPath> {
        match clip {
            ClipRef::Fill {
                transform, shape, ..
            } => self.clip_path_for_geometry(shape, transform),
            ClipRef::Stroke {
                transform,
                shape,
                stroke,
            } => {
                let origin = self.current_layer().origin;
                let transform = Affine::translate((-origin.x, -origin.y)) * transform;
                let shape_path = match shape {
                    imaging::GeometryRef::Rect(rect) => shape_to_path(&rect)?,
                    imaging::GeometryRef::RoundedRect(rect) => shape_to_path(&rect)?,
                    imaging::GeometryRef::Path(path) => path_to_tiny_skia_path(path)?,
                    imaging::GeometryRef::OwnedPath(ref path) => path_to_tiny_skia_path(path)?,
                };
                let tiny_stroke = kurbo_stroke_to_tiny_stroke(stroke);
                let res_scale =
                    tiny_skia::PathStroker::compute_resolution_scale(&affine_to_skia(transform));
                let stroked =
                    tiny_skia::PathStroker::new().stroke(&shape_path, &tiny_stroke, res_scale)?;
                let rect = Rect::new(
                    f64::from(stroked.bounds().left()),
                    f64::from(stroked.bounds().top()),
                    f64::from(stroked.bounds().right()),
                    f64::from(stroked.bounds().bottom()),
                );
                let path = stroked.transform(affine_to_skia(transform))?;
                Some(ClipPath {
                    path,
                    rect: transform.transform_rect_bbox(rect),
                    simple_rect: None,
                    stroke_source: Some((
                        shape_path.transform(affine_to_skia(transform))?,
                        tiny_stroke,
                    )),
                })
            }
        }
    }

    fn clip_path_for_geometry(
        &self,
        shape: imaging::GeometryRef<'_>,
        transform: Affine,
    ) -> Option<ClipPath> {
        let origin = self.current_layer().origin;
        let transform = Affine::translate((-origin.x, -origin.y)) * transform;
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
            stroke_source: None,
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
        first_layer.filters.clear();
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

    fn current_layer(&self) -> &Layer<'a> {
        self.layers
            .last()
            .expect("TinySkiaRenderer always has a root layer")
    }

    fn new_composite_child_layer(
        &mut self,
        composite: Composite,
        transform: Affine,
    ) -> Option<Layer<'static>> {
        let width = self.layers[0].pixmap.width();
        let height = self.layers[0].pixmap.height();
        let inherited_clips = {
            let layer = self.current_layer_mut();
            layer.effective_clips()
        };
        let omitted_non_simple = inherited_clips
            .iter()
            .any(|clip| clip.simple_rect.is_none());
        let applied_inherited_clips = Self::simple_inherited_clips(&inherited_clips);

        let mut child = Layer::new_root(width, height).ok()?;
        child.blend_mode = composite.blend;
        child.alpha = composite.alpha;
        child.transform = transform;
        child.rebuild_clip_mask_with_extra_clips(&applied_inherited_clips);
        child.clip_applied_in_content = !omitted_non_simple;
        Some(child)
    }

    fn try_fill_cached_image_rect(
        &mut self,
        image: &peniko::ImageBrush,
        rect: Rect,
        draw: &FillRef<'_>,
    ) -> bool {
        if let BlendStrategy::SinglePass(blend_mode) =
            determine_blend_strategy(&draw.composite.blend)
        {
            let cache_color = self.cache_color;
            let transform = draw.transform;
            let brush_transform = draw.brush_transform;
            let (caches, layers) = (&mut self.caches, &mut self.layers);
            let layer = layers
                .last_mut()
                .expect("TinySkiaRenderer always has a root layer");
            layer.transform = transform;
            return render_cached_image_rect(
                caches,
                cache_color,
                layer,
                image,
                rect,
                transform,
                brush_transform,
                draw.composite.alpha,
                blend_mode,
            );
        }

        let Some(mut child) = self.new_composite_child_layer(draw.composite, draw.transform) else {
            return false;
        };
        if !render_cached_image_rect(
            &mut self.caches,
            self.cache_color,
            &mut child,
            image,
            rect,
            draw.transform,
            draw.brush_transform,
            1.0,
            TinyBlendMode::SourceOver,
        ) {
            return false;
        }

        let parent = self.current_layer_mut();
        apply_layer(&child, parent);
        true
    }

    fn fill_geometry<'b>(
        layer: &mut Layer<'_>,
        shape: &imaging::GeometryRef<'_>,
        brush: impl Into<BrushRef<'b>>,
        brush_transform: Option<Affine>,
    ) {
        match shape {
            imaging::GeometryRef::Rect(rect) => {
                layer.fill_with_brush_transform(rect, brush, brush_transform);
            }
            imaging::GeometryRef::RoundedRect(rect) => {
                layer.fill_with_brush_transform(rect, brush, brush_transform);
            }
            imaging::GeometryRef::Path(path) => {
                layer.fill_with_brush_transform(path, brush, brush_transform);
            }
            imaging::GeometryRef::OwnedPath(path) => {
                layer.fill_with_brush_transform(path, brush, brush_transform);
            }
        }
    }

    fn fill_geometry_with_mode<'b>(
        layer: &mut Layer<'_>,
        shape: &imaging::GeometryRef<'_>,
        brush: impl Into<BrushRef<'b>>,
        brush_transform: Option<Affine>,
        opacity: f32,
        blend_mode: TinyBlendMode,
    ) {
        match shape {
            imaging::GeometryRef::Rect(rect) => layer.fill_with_brush_transform_and_mode(
                rect,
                brush,
                brush_transform,
                opacity,
                blend_mode,
            ),
            imaging::GeometryRef::RoundedRect(rect) => layer.fill_with_brush_transform_and_mode(
                rect,
                brush,
                brush_transform,
                opacity,
                blend_mode,
            ),
            imaging::GeometryRef::Path(path) => layer.fill_with_brush_transform_and_mode(
                path,
                brush,
                brush_transform,
                opacity,
                blend_mode,
            ),
            imaging::GeometryRef::OwnedPath(path) => layer.fill_with_brush_transform_and_mode(
                path,
                brush,
                brush_transform,
                opacity,
                blend_mode,
            ),
        }
    }

    fn fill_image_geometry_with_mode(
        layer: &mut Layer<'_>,
        shape: &imaging::GeometryRef<'_>,
        image_pixmap: &Pixmap,
        image: &peniko::ImageBrush,
        brush_transform: Option<Affine>,
        opacity: f32,
        blend_mode: TinyBlendMode,
    ) {
        match shape {
            imaging::GeometryRef::Rect(rect) => layer.fill_image_with_pixmap_and_mode(
                rect,
                image_pixmap,
                image,
                brush_transform,
                opacity,
                blend_mode,
            ),
            imaging::GeometryRef::RoundedRect(rect) => layer.fill_image_with_pixmap_and_mode(
                rect,
                image_pixmap,
                image,
                brush_transform,
                opacity,
                blend_mode,
            ),
            imaging::GeometryRef::Path(path) => layer.fill_image_with_pixmap_and_mode(
                path,
                image_pixmap,
                image,
                brush_transform,
                opacity,
                blend_mode,
            ),
            imaging::GeometryRef::OwnedPath(path) => layer.fill_image_with_pixmap_and_mode(
                path,
                image_pixmap,
                image,
                brush_transform,
                opacity,
                blend_mode,
            ),
        }
    }

    fn stroke_geometry<'b>(
        layer: &mut Layer<'_>,
        shape: &imaging::GeometryRef<'_>,
        brush: impl Into<BrushRef<'b>>,
        stroke: &KurboStroke,
        brush_transform: Option<Affine>,
    ) {
        match shape {
            imaging::GeometryRef::Rect(rect) => {
                layer.stroke_with_brush_transform(rect, brush, stroke, brush_transform);
            }
            imaging::GeometryRef::RoundedRect(rect) => {
                layer.stroke_with_brush_transform(rect, brush, stroke, brush_transform);
            }
            imaging::GeometryRef::Path(path) => {
                layer.stroke_with_brush_transform(path, brush, stroke, brush_transform);
            }
            imaging::GeometryRef::OwnedPath(path) => {
                layer.stroke_with_brush_transform(path, brush, stroke, brush_transform);
            }
        }
    }

    fn stroke_geometry_with_mode<'b>(
        layer: &mut Layer<'_>,
        shape: &imaging::GeometryRef<'_>,
        brush: impl Into<BrushRef<'b>>,
        stroke: &KurboStroke,
        brush_transform: Option<Affine>,
        opacity: f32,
        blend_mode: TinyBlendMode,
    ) {
        match shape {
            imaging::GeometryRef::Rect(rect) => layer.stroke_with_brush_transform_and_mode(
                rect,
                brush,
                stroke,
                brush_transform,
                opacity,
                blend_mode,
            ),
            imaging::GeometryRef::RoundedRect(rect) => layer.stroke_with_brush_transform_and_mode(
                rect,
                brush,
                stroke,
                brush_transform,
                opacity,
                blend_mode,
            ),
            imaging::GeometryRef::Path(path) => layer.stroke_with_brush_transform_and_mode(
                path,
                brush,
                stroke,
                brush_transform,
                opacity,
                blend_mode,
            ),
            imaging::GeometryRef::OwnedPath(path) => layer.stroke_with_brush_transform_and_mode(
                path,
                brush,
                stroke,
                brush_transform,
                opacity,
                blend_mode,
            ),
        }
    }

    fn stroke_image_geometry_with_mode(
        layer: &mut Layer<'_>,
        shape: &imaging::GeometryRef<'_>,
        image_pixmap: &Pixmap,
        image: &peniko::ImageBrush,
        stroke: &KurboStroke,
        brush_transform: Option<Affine>,
        opacity: f32,
        blend_mode: TinyBlendMode,
    ) {
        match shape {
            imaging::GeometryRef::Rect(rect) => layer.stroke_image_with_pixmap_and_mode(
                rect,
                image_pixmap,
                image,
                stroke,
                brush_transform,
                opacity,
                blend_mode,
            ),
            imaging::GeometryRef::RoundedRect(rect) => layer.stroke_image_with_pixmap_and_mode(
                rect,
                image_pixmap,
                image,
                stroke,
                brush_transform,
                opacity,
                blend_mode,
            ),
            imaging::GeometryRef::Path(path) => layer.stroke_image_with_pixmap_and_mode(
                path,
                image_pixmap,
                image,
                stroke,
                brush_transform,
                opacity,
                blend_mode,
            ),
            imaging::GeometryRef::OwnedPath(path) => layer.stroke_image_with_pixmap_and_mode(
                path,
                image_pixmap,
                image,
                stroke,
                brush_transform,
                opacity,
                blend_mode,
            ),
        }
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
            caches: RendererCaches::new(),
            transform: Affine::IDENTITY,
            cache_color: CacheColor(false),
            mask_cache: VecDeque::new(),
            layers: vec![main_layer],
            group_frames: Vec::new(),
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
        assert!(
            self.group_frames.is_empty(),
            "TinySkiaRenderer must not have open groups at frame start"
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
    imaging::record::replay_transformed(scene, &mut renderer, transform);
    let layer = renderer.layers.into_iter().next()?;
    match layer.pixmap {
        LayerPixmap::Owned(pixmap) => Some(Arc::new(pixmap)),
        LayerPixmap::Borrowed(_) => None,
    }
}

fn rasterize_scene_mask(scene: &Scene, bounds: IntRect, transform: Affine) -> Option<Arc<Pixmap>> {
    let width = bounds.width();
    let height = bounds.height();
    let origin = Affine::translate((-f64::from(bounds.x()), -f64::from(bounds.y())));
    let pixmap = rasterize_scene_pixmap(scene, width, height, origin * transform)?;
    Some(pixmap)
}

impl PaintSink for TinySkiaRendererImpl<'_> {
    fn push_clip(&mut self, clip: ClipRef<'_>) {
        let clip_path = self.clip_path_for_clip(clip);
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
        if !Self::group_requires_isolation(&group) {
            let pushed_clip = if let Some(clip) = group.clip {
                self.push_clip(clip);
                true
            } else {
                false
            };
            self.group_frames.push(GroupFrame::Direct { pushed_clip });
            return;
        }

        if let Some(clip) = self.group_clip_path(group.clip)
            && let Some(child) = self.build_group_child_layer(
                group.composite,
                group.filters,
                self.pending_group_mask(group.mask),
                clip,
            )
        {
            self.layers.push(child);
            self.group_frames.push(GroupFrame::Isolated);
        }
    }

    fn pop_group(&mut self) {
        let Some(frame) = self.group_frames.pop() else {
            return;
        };
        match frame {
            GroupFrame::Direct { pushed_clip } => {
                if pushed_clip {
                    self.pop_clip();
                }
            }
            GroupFrame::Isolated => {
                if self.layers.len() <= 1 {
                    return;
                }
                let child = self.layers.pop().expect("checked layer depth");
                self.finish_popped_group(child);
            }
        }
    }

    fn fill(&mut self, draw: FillRef<'_>) {
        let Some(brush) = self.brush_to_owned(draw.brush) else {
            return;
        };
        if let peniko::Brush::Image(image) = &brush {
            let Some(image_pixmap) = cache_image_pixmap(&mut self.caches, self.cache_color, image)
            else {
                return;
            };
            if let imaging::GeometryRef::Rect(rect) = draw.shape
                && self.try_fill_cached_image_rect(image, rect, &draw)
            {
                return;
            }

            if let BlendStrategy::SinglePass(blend_mode) =
                determine_blend_strategy(&draw.composite.blend)
            {
                let layer = self.current_layer_mut();
                layer.transform = draw.transform;
                Self::fill_image_geometry_with_mode(
                    layer,
                    &draw.shape,
                    &image_pixmap,
                    image,
                    draw.brush_transform,
                    draw.composite.alpha,
                    blend_mode,
                );
                return;
            }

            let Some(mut child) = self.new_composite_child_layer(draw.composite, draw.transform)
            else {
                return;
            };
            Self::fill_image_geometry_with_mode(
                &mut child,
                &draw.shape,
                &image_pixmap,
                image,
                draw.brush_transform,
                1.0,
                TinyBlendMode::SourceOver,
            );
            let parent = self.current_layer_mut();
            apply_layer(&child, parent);
            return;
        }

        if let BlendStrategy::SinglePass(blend_mode) =
            determine_blend_strategy(&draw.composite.blend)
        {
            let layer = self.current_layer_mut();
            layer.transform = draw.transform;
            Self::fill_geometry_with_mode(
                layer,
                &draw.shape,
                &brush,
                draw.brush_transform,
                draw.composite.alpha,
                blend_mode,
            );
            return;
        }

        let transform = draw.transform;
        let brush_transform = draw.brush_transform;
        self.draw_with_composite(draw.composite, |layer| {
            layer.transform = transform;
            Self::fill_geometry(layer, &draw.shape, &brush, brush_transform);
        });
    }

    fn stroke(&mut self, draw: StrokeRef<'_>) {
        let Some(brush) = self.brush_to_owned(draw.brush) else {
            return;
        };
        if let peniko::Brush::Image(image) = &brush {
            let Some(image_pixmap) = cache_image_pixmap(&mut self.caches, self.cache_color, image)
            else {
                return;
            };
            if let BlendStrategy::SinglePass(blend_mode) =
                determine_blend_strategy(&draw.composite.blend)
            {
                let layer = self.current_layer_mut();
                layer.transform = draw.transform;
                Self::stroke_image_geometry_with_mode(
                    layer,
                    &draw.shape,
                    &image_pixmap,
                    image,
                    draw.stroke,
                    draw.brush_transform,
                    draw.composite.alpha,
                    blend_mode,
                );
                return;
            }

            let Some(mut child) = self.new_composite_child_layer(draw.composite, draw.transform)
            else {
                return;
            };
            Self::stroke_image_geometry_with_mode(
                &mut child,
                &draw.shape,
                &image_pixmap,
                image,
                draw.stroke,
                draw.brush_transform,
                1.0,
                TinyBlendMode::SourceOver,
            );
            let parent = self.current_layer_mut();
            apply_layer(&child, parent);
            return;
        }

        if let BlendStrategy::SinglePass(blend_mode) =
            determine_blend_strategy(&draw.composite.blend)
        {
            let layer = self.current_layer_mut();
            layer.transform = draw.transform;
            Self::stroke_geometry_with_mode(
                layer,
                &draw.shape,
                &brush,
                draw.stroke,
                draw.brush_transform,
                draw.composite.alpha,
                blend_mode,
            );
            return;
        }

        let transform = draw.transform;
        let brush_transform = draw.brush_transform;
        self.draw_with_composite(draw.composite, |layer| {
            layer.transform = transform;
            Self::stroke_geometry(layer, &draw.shape, &brush, draw.stroke, brush_transform);
        });
    }

    fn glyph_run(
        &mut self,
        draw: GlyphRunRef<'_>,
        glyphs: &mut dyn Iterator<Item = imaging::record::Glyph>,
    ) {
        let cache_color = self.cache_color;
        if draw.composite == Composite::default() {
            let transform = self.transform;
            let (caches, layers) = (&mut self.caches, &mut self.layers);
            let layer = layers
                .last_mut()
                .expect("TinySkiaRenderer always has a root layer");
            layer.transform = transform;
            draw_glyphs_into_layer(caches, layer, cache_color, Point::ZERO, &draw, glyphs);
            return;
        }

        let Some(mut child) = self.new_composite_child_layer(draw.composite, self.transform) else {
            return;
        };
        draw_glyphs_into_layer(
            &mut self.caches,
            &mut child,
            cache_color,
            Point::ZERO,
            &draw,
            glyphs,
        );

        let parent = self.current_layer_mut();
        apply_layer(&child, parent);
    }

    fn blurred_rounded_rect(&mut self, draw: BlurredRoundedRect) {
        if let BlendStrategy::SinglePass(blend_mode) =
            determine_blend_strategy(&draw.composite.blend)
        {
            let cache_color = self.cache_color;
            let (caches, layers) = (&mut self.caches, &mut self.layers);
            let layer = layers
                .last_mut()
                .expect("TinySkiaRenderer always has a root layer");
            if render_cached_blurred_rounded_rect(caches, cache_color, layer, &draw, blend_mode) {
                return;
            }
        }

        let Some(mut child) = self.new_composite_child_layer(draw.composite, draw.transform) else {
            return;
        };
        let shape = draw.rect.to_rounded_rect(draw.radius);
        child.fill(&shape, draw.color);
        child.filters.push(Filter::Blur {
            std_deviation_x: f64_to_f32(draw.std_dev),
            std_deviation_y: f64_to_f32(draw.std_dev),
        });
        apply_group_filters(&mut child);

        let parent = self.current_layer_mut();
        apply_layer(&child, parent);
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
        self.finish_into_rgba8(image.data.as_mut_slice(), usize_from_u32(width) * 4)
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
    usize::try_from(value).expect("u32 value must fit in usize")
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

fn kurbo_stroke_to_tiny_stroke(stroke: &KurboStroke) -> TinyStroke {
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
    TinyStroke {
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
    }
}

fn mul_div_255(value: u8, factor: u8) -> u8 {
    u8::try_from((u16::from(value) * u16::from(factor) + 127) / 255)
        .expect("scaled 8-bit value must fit in u8")
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

fn unpremultiply_channel(channel: u8, alpha: u8) -> u8 {
    if alpha == 0 {
        return 0;
    }
    if alpha == u8::MAX {
        return channel;
    }

    let value = (u32::from(channel) * 255 + u32::from(alpha) / 2) / u32::from(alpha);
    u8::try_from(value.min(u32::from(u8::MAX))).expect("unpremultiplied channel must fit in u8")
}

fn draw_solid_color_glyphs_into_layer<'a>(
    caches: &mut RendererCaches,
    layer: &mut Layer<'_>,
    cache_color: CacheColor,
    origin: Point,
    run: GlyphRunRef<'a>,
    glyphs: &[imaging::record::Glyph],
    brush_color: Color,
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

        let cached = cache_glyph(
            caches,
            GlyphRasterRequest {
                cache_color,
                cache_key,
                color: brush_color,
                font_ref,
                font_size: scaled_font_size,
                hint: run.hint,
                normalized_coords: run.normalized_coords,
                embolden_strength: 0.0,
                skew,
                offset_x: new_x,
                offset_y: new_y,
            },
        );

        if let Some(cached) = cached {
            layer.render_pixmap_direct(
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

fn draw_brushed_glyphs_into_layer<'a>(
    caches: &mut RendererCaches,
    layer: &mut Layer<'_>,
    cache_color: CacheColor,
    origin: Point,
    run: GlyphRunRef<'a>,
    glyphs: &[imaging::record::Glyph],
) {
    let width = layer.pixmap.width();
    let height = layer.pixmap.height();
    let mut mask_layer = match Layer::new_root(width, height) {
        Ok(layer) => layer,
        Err(_) => return,
    };
    let mask_run = GlyphRunRef {
        brush: BrushRef::Solid(Color::WHITE),
        ..run
    };
    draw_solid_color_glyphs_into_layer(
        caches,
        &mut mask_layer,
        cache_color,
        origin,
        mask_run,
        glyphs,
        Color::WHITE,
    );
    let Some(mask_bounds) = mask_layer.draw_bounds else {
        return;
    };
    let Some(mask_bounds) = rect_to_int_rect(mask_bounds) else {
        return;
    };
    let Some(local_mask) = clone_pixmap_region(mask_layer.pixmap.as_ref(), mask_bounds) else {
        return;
    };
    let mut content_layer = match Layer::new_root(mask_bounds.width(), mask_bounds.height()) {
        Ok(layer) => layer,
        Err(_) => return,
    };
    let canvas_rect = Rect::from_origin_size(
        Point::ZERO,
        Size::new(
            f64::from(mask_bounds.width()),
            f64::from(mask_bounds.height()),
        ),
    );
    content_layer.fill_with_brush_transform_and_mode(
        &canvas_rect,
        run.brush,
        Some(Affine::translate((
            f64::from(mask_bounds.x()),
            f64::from(mask_bounds.y()),
        ))),
        1.0,
        TinyBlendMode::SourceOver,
    );
    apply_alpha_mask_from_pixmap(
        match &mut content_layer.pixmap {
            LayerPixmap::Owned(pixmap) => pixmap,
            LayerPixmap::Borrowed(_) => return,
        },
        local_mask.as_ref(),
    );
    content_layer.origin = Point::new(f64::from(mask_bounds.x()), f64::from(mask_bounds.y()));
    content_layer.draw_bounds = Some(Rect::new(
        f64::from(mask_bounds.x()),
        f64::from(mask_bounds.y()),
        f64::from(mask_bounds.x())
            + f64::from(i32::try_from(mask_bounds.width()).expect("width fits i32")),
        f64::from(mask_bounds.y())
            + f64::from(i32::try_from(mask_bounds.height()).expect("height fits i32")),
    ));
    apply_layer(&content_layer, layer);
}

fn draw_glyphs_into_layer<'a>(
    caches: &mut RendererCaches,
    layer: &mut Layer<'_>,
    cache_color: CacheColor,
    origin: Point,
    run: &GlyphRunRef<'a>,
    glyphs: impl Iterator<Item = imaging::record::Glyph> + 'a,
) {
    let glyphs: Vec<_> = glyphs.collect();
    if glyphs.is_empty() {
        return;
    }
    match run.brush {
        BrushRef::Solid(color) => draw_solid_color_glyphs_into_layer(
            caches,
            layer,
            cache_color,
            origin,
            run.clone(),
            &glyphs,
            Color::from(color),
        ),
        _ => {
            draw_brushed_glyphs_into_layer(
                caches,
                layer,
                cache_color,
                origin,
                run.clone(),
                &glyphs,
            );
        }
    }
}

impl TinySkiaRendererImpl<'_> {
    fn canvas_size(&self) -> Size {
        Size::new(
            self.layers[0].pixmap.width() as f64,
            self.layers[0].pixmap.height() as f64,
        )
    }

    fn draw_with_composite(&mut self, composite: Composite, draw: impl FnOnce(&mut Layer<'_>)) {
        if composite == Composite::default() {
            let transform = self.transform;
            let layer = self.current_layer_mut();
            layer.transform = transform;
            draw(layer);
            return;
        }

        let Some(mut child) = self.new_composite_child_layer(composite, self.transform) else {
            return;
        };
        draw(&mut child);

        let parent = self.current_layer_mut();
        apply_layer(&child, parent);
    }

    fn finish_into_rgba8(&mut self, dst: &mut [u8], bytes_per_row: usize) -> Option<()> {
        self.finish_into_unpremultiplied(dst, bytes_per_row, true)
    }

    fn finish_direct_rgba8_opaque(&mut self) -> Option<()> {
        self.finalize_frame();
        for pixel in self.layers[0].pixmap.data_mut().chunks_exact_mut(4) {
            pixel[3] = 0xff;
        }
        Some(())
    }

    fn finalize_frame(&mut self) {
        self.caches
            .image_cache
            .retain(|_, (c, _)| *c == self.cache_color);
        self.caches
            .scaled_image_cache
            .retain(|_, (c, _)| *c == self.cache_color);
        self.caches
            .blurred_rrect_cache
            .retain(|_, (c, _)| *c == self.cache_color);
        let now = Instant::now();
        self.caches
            .glyph_cache
            .retain(|_, entry| should_retain_glyph_entry(entry, self.cache_color, now));
        self.cache_color = CacheColor(!self.cache_color.0);
    }

    fn finish_into_unpremultiplied(
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
                let alpha = src[3];
                let red = unpremultiply_channel(src[0], alpha);
                let green = unpremultiply_channel(src[1], alpha);
                let blue = unpremultiply_channel(src[2], alpha);
                if rgba {
                    out.copy_from_slice(&[red, green, blue, alpha]);
                } else {
                    out.copy_from_slice(&[blue, green, red, alpha]);
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

fn realize_image_pixmap(image_data: &ImageData) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(image_data.width, image_data.height)?;
    for (pixel, bytes) in pixmap
        .pixels_mut()
        .iter_mut()
        .zip(image_data.data.data().chunks_exact(4))
    {
        *pixel = tiny_skia::Color::from_rgba8(bytes[0], bytes[1], bytes[2], bytes[3])
            .premultiply()
            .to_color_u8();
    }
    Some(pixmap)
}

fn cache_image_pixmap<T>(
    caches: &mut RendererCaches,
    cache_color: CacheColor,
    image: &peniko::ImageBrush<T>,
) -> Option<Arc<Pixmap>>
where
    T: Borrow<ImageData>,
{
    let image_data = image.image.borrow();
    let image_id = image_data.data.id();
    if let Some((entry_color, pixmap)) = caches.image_cache.get_mut(&image_id) {
        *entry_color = cache_color;
        return Some(pixmap.clone());
    }

    let pixmap = Arc::new(realize_image_pixmap(image_data)?);
    caches
        .image_cache
        .insert(image_id, (cache_color, pixmap.clone()));
    Some(pixmap)
}

fn resize_pixmap(
    source: &Pixmap,
    width: u32,
    height: u32,
    quality: ImageQuality,
) -> Option<Pixmap> {
    let mut scaled = Pixmap::new(width, height)?;
    let paint = PixmapPaint {
        opacity: 1.0,
        blend_mode: tiny_skia::BlendMode::SourceOver,
        quality: image_quality_to_filter_quality(quality),
    };
    let transform = Transform::from_scale(
        f64_to_f32(f64::from(width) / f64::from(source.width())),
        f64_to_f32(f64::from(height) / f64::from(source.height())),
    );
    scaled.draw_pixmap(0, 0, source.as_ref(), &paint, transform, None);
    Some(scaled)
}

fn cache_scaled_image_pixmap<T>(
    caches: &mut RendererCaches,
    cache_color: CacheColor,
    image: &peniko::ImageBrush<T>,
    width: u32,
    height: u32,
) -> Option<Arc<Pixmap>>
where
    T: Borrow<ImageData>,
{
    let image_id = image.image.borrow().data.id();
    let key = ScaledImageCacheKey {
        image_id,
        width,
        height,
        quality: std::mem::discriminant(&image.sampler.quality),
    };
    if let Some((entry_color, pixmap)) = caches.scaled_image_cache.get_mut(&key) {
        *entry_color = cache_color;
        return Some(pixmap.clone());
    }

    let source = cache_image_pixmap(caches, cache_color, image)?;
    let scaled = Arc::new(resize_pixmap(
        &source,
        width,
        height,
        image.sampler.quality,
    )?);
    caches
        .scaled_image_cache
        .insert(key, (cache_color, scaled.clone()));
    Some(scaled)
}

fn cached_image_rect_size<T>(
    image: &peniko::ImageBrush<T>,
    rect: Rect,
    brush_transform: Option<Affine>,
) -> Option<(u32, u32)>
where
    T: Borrow<ImageData>,
{
    (brush_transform.is_none()
        && image.sampler.x_extend == Extend::Pad
        && image.sampler.y_extend == Extend::Pad)
        .then_some(())?;
    let width = nearly_integral(rect.width())?;
    let height = nearly_integral(rect.height())?;
    (width > 0 && height > 0).then_some((
        u32::try_from(width).expect("positive width fits u32"),
        u32::try_from(height).expect("positive height fits u32"),
    ))
}

fn render_cached_image_rect<T>(
    caches: &mut RendererCaches,
    cache_color: CacheColor,
    layer: &mut Layer<'_>,
    image: &peniko::ImageBrush<T>,
    rect: Rect,
    transform: Affine,
    brush_transform: Option<Affine>,
    opacity: f32,
    blend_mode: TinyBlendMode,
) -> bool
where
    T: Borrow<ImageData>,
{
    let Some((width, height)) = cached_image_rect_size(image, rect, brush_transform) else {
        return false;
    };
    let Some(pixmap) = cache_scaled_image_pixmap(caches, cache_color, image, width, height) else {
        return false;
    };
    layer.draw_pixmap_rect(
        &pixmap,
        rect,
        transform,
        PixmapPaint {
            opacity: image.sampler.alpha * opacity,
            blend_mode,
            quality: image_quality_to_filter_quality(image.sampler.quality),
        },
    );
    true
}

fn translation_components_if_axis_aligned(affine: Affine) -> Option<(f64, f64)> {
    let [xx, yx, xy, yy, dx, dy] = affine.as_coeffs();
    (xx == 1.0 && yx == 0.0 && xy == 0.0 && yy == 1.0).then_some((dx, dy))
}

fn blurred_rrect_cache_key(
    rect: Rect,
    radius: f64,
    std_dev: f64,
    color: Color,
) -> BlurredRRectCacheKey {
    BlurredRRectCacheKey {
        x_bits: rect.x0.to_bits(),
        y_bits: rect.y0.to_bits(),
        width_bits: rect.width().to_bits(),
        height_bits: rect.height().to_bits(),
        radius_bits: radius.to_bits(),
        std_dev_bits: std_dev.to_bits(),
        color_rgba: color.to_rgba8().to_u32(),
    }
}

fn render_blurred_rounded_rect_pixmap(
    rect: Rect,
    radius: f64,
    std_dev: f64,
    color: Color,
    width: u32,
    height: u32,
) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(width, height)?;
    let shape = rect.to_rounded_rect(radius);
    let path = shape_to_path(&shape)?;
    let paint = Paint {
        shader: Shader::SolidColor(to_color(color)),
        blend_mode: TinyBlendMode::SourceOver,
        anti_alias: true,
        ..Default::default()
    };
    pixmap.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
    blur_pixmap(&pixmap, f64_to_f32(std_dev), f64_to_f32(std_dev))
}

fn render_cached_blurred_rounded_rect(
    caches: &mut RendererCaches,
    cache_color: CacheColor,
    layer: &mut Layer<'_>,
    draw: &BlurredRoundedRect,
    blend_mode: TinyBlendMode,
) -> bool {
    let Some((dx, dy)) = translation_components_if_axis_aligned(draw.transform) else {
        return false;
    };
    let translated_rect = translate_rect(draw.rect, dx, dy);
    let Some(bounds) = rect_to_int_rect(blur_bounds(
        translated_rect,
        f64_to_f32(draw.std_dev),
        f64_to_f32(draw.std_dev),
    )) else {
        return false;
    };
    let local_rect = translate_rect(
        translated_rect,
        -f64::from(bounds.x()),
        -f64::from(bounds.y()),
    );
    let key = blurred_rrect_cache_key(local_rect, draw.radius, draw.std_dev, draw.color);
    let pixmap = if let Some((entry_color, pixmap)) = caches.blurred_rrect_cache.get_mut(&key) {
        *entry_color = cache_color;
        pixmap.clone()
    } else {
        let Some(pixmap) = render_blurred_rounded_rect_pixmap(
            local_rect,
            draw.radius,
            draw.std_dev,
            draw.color,
            bounds.width(),
            bounds.height(),
        ) else {
            return false;
        };
        let pixmap = Arc::new(pixmap);
        caches
            .blurred_rrect_cache
            .insert(key, (cache_color, pixmap.clone()));
        pixmap
    };

    let x = bounds.x() - round_to_i32(layer.origin.x);
    let y = bounds.y() - round_to_i32(layer.origin.y);
    draw_layer_pixmap(
        pixmap.as_ref(),
        x,
        y,
        layer,
        blend_mode,
        draw.composite.alpha.clamp(0.0, 1.0),
        true,
    );
    true
}

fn brush_to_paint<'b>(
    brush: impl Into<BrushRef<'b>>,
    brush_transform: Option<Affine>,
    opacity: f32,
    blend_mode: TinyBlendMode,
) -> Option<Paint<'static>> {
    let shader_transform = affine_to_skia(brush_transform.unwrap_or(Affine::IDENTITY));
    let opacity = opacity.clamp(0.0, 1.0);
    let shader = match brush.into() {
        BrushRef::Solid(c) => Shader::SolidColor(scale_tiny_skia_color_alpha(to_color(c), opacity)),
        BrushRef::Gradient(g) => {
            let stops = expand_gradient_stops(g, opacity);
            let tiny_stops = stops
                .iter()
                .map(|stop| GradientStop::new(stop.offset, stop.color))
                .collect();
            let spread_mode = to_spread_mode(g.extend);
            match g.kind {
                GradientKind::Linear(linear) => LinearGradient::new(
                    to_point(linear.start),
                    to_point(linear.end),
                    tiny_stops,
                    spread_mode,
                    shader_transform,
                )?,
                GradientKind::Radial(RadialGradientPosition {
                    start_center,
                    start_radius,
                    end_center,
                    end_radius,
                }) => {
                    debug_assert!(
                        start_radius.abs() <= f32::EPSILON,
                        "two-point radial gradients should be handled by the fallback path"
                    );
                    RadialGradient::new(
                        to_point(start_center),
                        to_point(end_center),
                        end_radius,
                        tiny_stops,
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
        blend_mode,
        ..Default::default()
    })
}

fn image_brush_pixmap<T>(image: &peniko::ImageBrush<T>) -> Option<Pixmap>
where
    T: Borrow<ImageData>,
{
    realize_image_pixmap(image.image.borrow())
}

fn image_brush_has_mixed_extend<T>(image: &peniko::ImageBrush<T>) -> bool {
    image.sampler.x_extend != image.sampler.y_extend
}

fn image_brush_spread_mode<T>(image: &peniko::ImageBrush<T>) -> SpreadMode {
    debug_assert!(
        !image_brush_has_mixed_extend(image),
        "mixed-axis image brushes must use the custom fallback path"
    );
    to_spread_mode(image.sampler.x_extend)
}

fn extend_image_axis(coord: f32, size: u32, extend: Extend) -> Option<f32> {
    let size = size as f32;
    if size <= 0.0 {
        return None;
    }
    Some(match extend {
        Extend::Pad => coord.clamp(0.0, size - 1.0),
        Extend::Repeat => coord.rem_euclid(size),
        Extend::Reflect => {
            let period = size * 2.0;
            let value = coord.rem_euclid(period);
            if value < size {
                value
            } else {
                period - value - 1.0
            }
        }
    })
}

fn premul_to_rgba_f32(color: PremultipliedColorU8) -> [f32; 4] {
    [
        f32::from(color.red()) / 255.0,
        f32::from(color.green()) / 255.0,
        f32::from(color.blue()) / 255.0,
        f32::from(color.alpha()) / 255.0,
    ]
}

#[derive(Clone, Copy)]
struct NearestAxisSample {
    index: Option<u32>,
}

#[derive(Clone, Copy)]
struct BilinearAxisSample {
    i0: Option<u32>,
    i1: Option<u32>,
    t: f32,
}

fn transparent_premul() -> PremultipliedColorU8 {
    PremultipliedColorU8::from_rgba(0, 0, 0, 0).expect("transparent is valid")
}

fn premul_from_rgba_f32(rgba: [f32; 4]) -> PremultipliedColorU8 {
    tiny_skia::Color::from_rgba(
        rgba[0].clamp(0.0, 1.0),
        rgba[1].clamp(0.0, 1.0),
        rgba[2].clamp(0.0, 1.0),
        rgba[3].clamp(0.0, 1.0),
    )
    .expect("sampled image color must be valid")
    .premultiply()
    .to_color_u8()
}

fn nearest_axis_samples(
    start: f32,
    step: f32,
    out_len: u32,
    source_len: u32,
    extent: Extend,
) -> Vec<NearestAxisSample> {
    let mut value = start;
    let mut out = Vec::with_capacity(usize_from_u32(out_len));
    for _ in 0..out_len {
        let index = extend_image_axis(value.floor(), source_len, extent)
            .map(|coord| u32::try_from(f32_to_i32(coord.round())).expect("wrapped index fits u32"));
        out.push(NearestAxisSample { index });
        value += step;
    }
    out
}

fn bilinear_axis_samples(
    start: f32,
    step: f32,
    out_len: u32,
    source_len: u32,
    extent: Extend,
) -> Vec<BilinearAxisSample> {
    let mut value = start;
    let mut out = Vec::with_capacity(usize_from_u32(out_len));
    for _ in 0..out_len {
        let pos = value - 0.5;
        let p0 = pos.floor();
        let t = pos - p0;
        let i0 = extend_image_axis(p0, source_len, extent)
            .map(|coord| u32::try_from(f32_to_i32(coord.round())).expect("wrapped index fits u32"));
        let i1 = extend_image_axis(p0 + 1.0, source_len, extent)
            .map(|coord| u32::try_from(f32_to_i32(coord.round())).expect("wrapped index fits u32"));
        out.push(BilinearAxisSample { i0, i1, t });
        value += step;
    }
    out
}

fn pixmap_pixel_or_transparent(
    pixels: &[PremultipliedColorU8],
    width: usize,
    x: Option<u32>,
    y: Option<u32>,
) -> PremultipliedColorU8 {
    match (x, y) {
        (Some(x), Some(y)) => pixels[usize_from_u32(y) * width + usize_from_u32(x)],
        _ => transparent_premul(),
    }
}

fn sample_image_brush_at<T>(
    pixmap: &Pixmap,
    image: &peniko::ImageBrush<T>,
    point: Point,
) -> PremultipliedColorU8
where
    T: Borrow<ImageData>,
{
    let quality = image_quality_to_filter_quality(image.sampler.quality);
    let opacity = opacity_to_u8(image.sampler.alpha);
    let width = pixmap.width();
    let height = pixmap.height();
    let sample = |x: f32, y: f32| -> PremultipliedColorU8 {
        let Some(x) = extend_image_axis(x, width, image.sampler.x_extend) else {
            return PremultipliedColorU8::from_rgba(0, 0, 0, 0).expect("transparent is valid");
        };
        let Some(y) = extend_image_axis(y, height, image.sampler.y_extend) else {
            return PremultipliedColorU8::from_rgba(0, 0, 0, 0).expect("transparent is valid");
        };
        pixmap
            .pixel(
                u32::try_from(f32_to_i32(x.round())).expect("wrapped x must fit u32"),
                u32::try_from(f32_to_i32(y.round())).expect("wrapped y must fit u32"),
            )
            .unwrap_or_else(|| tiny_skia::Color::TRANSPARENT.premultiply().to_color_u8())
    };

    let color = match quality {
        FilterQuality::Nearest => sample(f64_to_f32(point.x).floor(), f64_to_f32(point.y).floor()),
        FilterQuality::Bilinear => {
            let x = f64_to_f32(point.x) - 0.5;
            let y = f64_to_f32(point.y) - 0.5;
            let x0 = x.floor();
            let y0 = y.floor();
            let tx = x - x0;
            let ty = y - y0;
            let c00 = premul_to_rgba_f32(sample(x0, y0));
            let c10 = premul_to_rgba_f32(sample(x0 + 1.0, y0));
            let c01 = premul_to_rgba_f32(sample(x0, y0 + 1.0));
            let c11 = premul_to_rgba_f32(sample(x0 + 1.0, y0 + 1.0));
            let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
            let mut rgba = [0.0; 4];
            for i in 0..4 {
                let top = lerp(c00[i], c10[i], tx);
                let bottom = lerp(c01[i], c11[i], tx);
                rgba[i] = lerp(top, bottom, ty);
            }
            premul_from_rgba_f32(rgba)
        }
        FilterQuality::Bicubic => sample(f64_to_f32(point.x), f64_to_f32(point.y)),
    };

    scale_premultiplied_color(color, opacity)
}

const GRADIENT_TOLERANCE: f32 = 0.01;

#[derive(Clone, Copy)]
struct ExpandedGradientStop {
    offset: f32,
    color: tiny_skia::Color,
}

#[derive(Clone, Copy)]
struct GradientSegment {
    start_offset: f32,
    start_color: DynamicColor,
    end_offset: f32,
    end_color: DynamicColor,
}

fn expand_gradient_stops(gradient: &Gradient, opacity: f32) -> Vec<ExpandedGradientStop> {
    if gradient.stops.is_empty() {
        return Vec::new();
    }

    if gradient.stops.len() == 1 {
        let stop = &gradient.stops[0];
        let color = stop.color.to_alpha_color::<Srgb>().convert::<Srgb>();
        return vec![ExpandedGradientStop {
            offset: stop.offset,
            color: scale_tiny_skia_color_alpha(alpha_color_to_tiny_skia(color), opacity),
        }];
    }

    let mut expanded = Vec::new();
    for segment in gradient.stops.windows(2) {
        let segment = GradientSegment {
            start_offset: segment[0].offset,
            start_color: segment[0].color,
            end_offset: segment[1].offset,
            end_color: segment[1].color,
        };
        if segment.start_offset == segment.end_offset {
            push_gradient_stop(
                &mut expanded,
                segment.start_offset,
                segment.start_color.to_alpha_color::<Srgb>(),
                opacity,
            );
            push_gradient_stop(
                &mut expanded,
                segment.end_offset,
                segment.end_color.to_alpha_color::<Srgb>(),
                opacity,
            );
            continue;
        }

        expand_gradient_segment(
            &mut expanded,
            segment,
            gradient.interpolation_cs,
            gradient.hue_direction,
            opacity,
        );
    }

    expanded
}

fn expand_gradient_segment(
    expanded: &mut Vec<ExpandedGradientStop>,
    segment: GradientSegment,
    interpolation_cs: ColorSpaceTag,
    hue_direction: HueDirection,
    opacity: f32,
) {
    let push_sample =
        |expanded: &mut Vec<ExpandedGradientStop>, t: f32, color: color::AlphaColor<Srgb>| {
            let offset = segment.start_offset + (segment.end_offset - segment.start_offset) * t;
            push_gradient_stop(expanded, offset, color, opacity);
        };

    for (i, (t, color)) in color::gradient::<Srgb>(
        segment.start_color,
        segment.end_color,
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
    expanded: &mut Vec<ExpandedGradientStop>,
    offset: f32,
    color: color::AlphaColor<Srgb>,
    opacity: f32,
) {
    let tiny_color = scale_tiny_skia_color_alpha(alpha_color_to_tiny_skia(color), opacity);
    if let Some(previous) = expanded.last()
        && previous.offset == offset
        && previous.color == tiny_color
    {
        return;
    }
    expanded.push(ExpandedGradientStop {
        offset,
        color: tiny_color,
    });
}

fn extend_gradient_t(extend: Extend, t: f32) -> f32 {
    match extend {
        Extend::Pad => t.clamp(0.0, 1.0),
        Extend::Repeat => t.rem_euclid(1.0),
        Extend::Reflect => {
            let reflected = t.rem_euclid(2.0);
            if reflected > 1.0 {
                2.0 - reflected
            } else {
                reflected
            }
        }
    }
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    let value = f32::from(a) + (f32::from(b) - f32::from(a)) * t.clamp(0.0, 1.0);
    opacity_to_u8(value / 255.0)
}

fn sample_expanded_gradient(
    stops: &[ExpandedGradientStop],
    extend: Extend,
    t: f32,
) -> tiny_skia::Color {
    let t = extend_gradient_t(extend, t);
    let Some(first) = stops.first() else {
        return tiny_skia::Color::TRANSPARENT;
    };
    if t <= first.offset {
        return first.color;
    }

    for window in stops.windows(2) {
        let start = window[0];
        let end = window[1];
        if t > end.offset {
            continue;
        }

        if end.offset <= start.offset {
            return end.color;
        }

        let local_t = (t - start.offset) / (end.offset - start.offset);
        let start_rgba = start.color.to_color_u8();
        let end_rgba = end.color.to_color_u8();
        return tiny_skia::Color::from_rgba8(
            lerp_u8(start_rgba.red(), end_rgba.red(), local_t),
            lerp_u8(start_rgba.green(), end_rgba.green(), local_t),
            lerp_u8(start_rgba.blue(), end_rgba.blue(), local_t),
            lerp_u8(start_rgba.alpha(), end_rgba.alpha(), local_t),
        );
    }

    stops
        .last()
        .map(|stop| stop.color)
        .unwrap_or(tiny_skia::Color::TRANSPARENT)
}

fn build_expanded_gradient_lut_256(
    stops: &[ExpandedGradientStop],
    extend: Extend,
) -> [tiny_skia::Color; 256] {
    let mut lut = [tiny_skia::Color::TRANSPARENT; 256];
    for (i, color) in lut.iter_mut().enumerate() {
        let t = (i as f32) / 255.0;
        *color = sample_expanded_gradient(stops, extend, t);
    }
    lut
}

fn sweep_lut_index_256(extend: Extend, t: f32) -> usize {
    let t = extend_gradient_t(extend, t);
    let idx = (t.clamp(0.0, 1.0) * 255.0).round();
    usize::try_from(f32_to_i32(idx)).expect("gradient LUT index must fit usize")
}

fn build_original_gradient_lut_1024(gradient: &Gradient, opacity: f32) -> [tiny_skia::Color; 1024] {
    let mut lut = [tiny_skia::Color::TRANSPARENT; 1024];
    for (i, color) in lut.iter_mut().enumerate() {
        let t = (i as f32) / 1023.0;
        *color = sample_original_gradient(gradient, opacity, t);
    }
    lut
}

fn gradient_lut_index_1024(extend: Extend, t: f32) -> usize {
    let t = extend_gradient_t(extend, t);
    let idx = (t.clamp(0.0, 1.0) * 1023.0).round();
    usize::try_from(f32_to_i32(idx)).expect("gradient LUT index must fit usize")
}

struct PreparedSweep {
    center_x: f32,
    center_y: f32,
    start_angle: f32,
    inv_angle_delta: f32,
    wrap: bool,
}

impl PreparedSweep {
    fn new(sweep: peniko::SweepGradientPosition) -> Self {
        let span = sweep.end_angle - sweep.start_angle;
        let span_abs = span.abs();
        let wrap = span_abs >= core::f32::consts::TAU - 1e-6;
        let inv_angle_delta = if span_abs <= f32::EPSILON {
            0.0
        } else if wrap {
            1.0 / span_abs
        } else {
            1.0 / span
        };
        Self {
            center_x: f64_to_f32(sweep.center.x),
            center_y: f64_to_f32(sweep.center.y),
            start_angle: sweep.start_angle,
            inv_angle_delta,
            wrap,
        }
    }

    fn project_point(&self, point: Point) -> (f32, f32) {
        (
            f64_to_f32(point.x) - self.center_x,
            f64_to_f32(point.y) - self.center_y,
        )
    }

    fn project_delta(&self, delta: Vec2) -> (f32, f32) {
        (f64_to_f32(delta.x), f64_to_f32(delta.y))
    }

    fn sample(&self, x: f32, y: f32) -> f32 {
        if self.inv_angle_delta == 0.0 {
            return 0.0;
        }
        let angle = unit_angle_approx(x, y) * core::f32::consts::TAU;
        if self.wrap {
            (angle - self.start_angle).rem_euclid(core::f32::consts::TAU) * self.inv_angle_delta
        } else {
            (angle - self.start_angle) * self.inv_angle_delta
        }
    }
}

fn sample_original_gradient(gradient: &Gradient, opacity: f32, t: f32) -> tiny_skia::Color {
    let t = extend_gradient_t(gradient.extend, t);
    let Some(first) = gradient.stops.first() else {
        return tiny_skia::Color::TRANSPARENT;
    };
    if t <= first.offset {
        let color = first.color.to_alpha_color::<Srgb>().convert::<Srgb>();
        let [r, g, b, a] = color.components;
        let alpha = a * opacity.clamp(0.0, 1.0);
        return tiny_skia::Color::from_rgba(
            r.clamp(0.0, 1.0),
            g.clamp(0.0, 1.0),
            b.clamp(0.0, 1.0),
            alpha.clamp(0.0, 1.0),
        )
        .expect("sampled gradient color must be valid");
    }

    for window in gradient.stops.windows(2) {
        let start = window[0];
        let end = window[1];
        if t > end.offset {
            continue;
        }

        if end.offset <= start.offset {
            let color = end.color.to_alpha_color::<Srgb>().convert::<Srgb>();
            let [r, g, b, a] = color.components;
            return tiny_skia::Color::from_rgba(
                r.clamp(0.0, 1.0),
                g.clamp(0.0, 1.0),
                b.clamp(0.0, 1.0),
                (a * opacity.clamp(0.0, 1.0)).clamp(0.0, 1.0),
            )
            .expect("sampled gradient color must be valid");
        }

        let local_t = (t - start.offset) / (end.offset - start.offset);
        let start = start.color.to_alpha_color::<Srgb>().convert::<Srgb>();
        let end = end.color.to_alpha_color::<Srgb>().convert::<Srgb>();
        let [sr, sg, sb, sa] = start.components;
        let [er, eg, eb, ea] = end.components;
        let opacity = opacity.clamp(0.0, 1.0);
        let sa = sa * opacity;
        let ea = ea * opacity;

        let (r, g, b, a) = match gradient.interpolation_alpha_space {
            InterpolationAlphaSpace::Premultiplied => {
                let a = sa + (ea - sa) * local_t;
                let rp = sr * sa + (er * ea - sr * sa) * local_t;
                let gp = sg * sa + (eg * ea - sg * sa) * local_t;
                let bp = sb * sa + (eb * ea - sb * sa) * local_t;
                if a <= f32::EPSILON {
                    (0.0, 0.0, 0.0, 0.0)
                } else {
                    (rp / a, gp / a, bp / a, a)
                }
            }
            InterpolationAlphaSpace::Unpremultiplied => (
                sr + (er - sr) * local_t,
                sg + (eg - sg) * local_t,
                sb + (eb - sb) * local_t,
                sa + (ea - sa) * local_t,
            ),
        };

        return tiny_skia::Color::from_rgba(
            r.clamp(0.0, 1.0),
            g.clamp(0.0, 1.0),
            b.clamp(0.0, 1.0),
            a.clamp(0.0, 1.0),
        )
        .expect("sampled gradient color must be valid");
    }

    let last = gradient
        .stops
        .last()
        .expect("checked non-empty gradient stops")
        .color
        .to_alpha_color::<Srgb>()
        .convert::<Srgb>();
    let [r, g, b, a] = last.components;
    tiny_skia::Color::from_rgba(
        r.clamp(0.0, 1.0),
        g.clamp(0.0, 1.0),
        b.clamp(0.0, 1.0),
        (a * opacity.clamp(0.0, 1.0)).clamp(0.0, 1.0),
    )
    .expect("sampled gradient color must be valid")
}

fn unit_angle_approx(x: f32, y: f32) -> f32 {
    let x_abs = x.abs();
    let y_abs = y.abs();
    let max_abs = x_abs.max(y_abs);
    if max_abs <= f32::EPSILON {
        return 0.0;
    }

    let slope = x_abs.min(y_abs) / max_abs;
    let s = slope * slope;
    let a = (-7.054_738_2e-3_f32).mul_add(s, 2.476_102e-2);
    let b = a.mul_add(s, -5.185_397e-2);
    let c = b.mul_add(s, 0.159_121_17);

    let mut phi = slope * c;
    if x_abs < y_abs {
        phi = 0.25 - phi;
    }
    if x < 0.0 {
        phi = 0.5 - phi;
    }
    if y < 0.0 {
        phi = 1.0 - phi;
    }
    if phi.is_nan() { 0.0 } else { phi }
}

struct PreparedTwoPointRadial {
    valid: bool,
    x_coeff_x: f64,
    x_coeff_y: f64,
    x_bias: f64,
    y_coeff_x: f64,
    y_coeff_y: f64,
    y_bias: f64,
    f: f64,
    one_minus_f: f64,
    one_minus_f_abs: f64,
    one_minus_f_signum: f64,
    r1_normalized: f64,
    swapped: bool,
    near_one: bool,
    x_prime_scale: f64,
    y_prime_scale: f64,
}

impl PreparedTwoPointRadial {
    fn project_point(&self, point: Point) -> (f64, f64) {
        (
            point.x * self.x_coeff_x + point.y * self.x_coeff_y + self.x_bias,
            point.x * self.y_coeff_x + point.y * self.y_coeff_y + self.y_bias,
        )
    }

    fn project_delta(&self, delta: Vec2) -> (f64, f64) {
        (
            delta.x * self.x_coeff_x + delta.y * self.x_coeff_y,
            delta.x * self.y_coeff_x + delta.y * self.y_coeff_y,
        )
    }

    fn sample(&self, x: f64, y: f64) -> f32 {
        if !self.valid {
            return 0.0;
        }
        let x_prime = x * self.x_prime_scale;
        let y_prime = y * self.y_prime_scale;
        if self.near_one {
            if x_prime.abs() <= f64::EPSILON {
                return 2.0;
            }
            let hat_x = self.one_minus_f_abs * x_prime;
            let hat_y = self.one_minus_f_abs * y_prime;
            let hat_x_t = (hat_x * hat_x + hat_y * hat_y) / hat_x;
            if self.r1_normalized <= 1.0 && hat_x_t < 0.0 {
                return 2.0;
            }
            let mut t = self.f + self.one_minus_f_signum * hat_x_t;
            if self.swapped {
                t = 1.0 - t;
            }
            return f64_to_f32(t);
        }

        let hat_x = self.one_minus_f_abs * x_prime;
        let hat_y = self.one_minus_f_abs * y_prime;
        let hat_x_t = if self.r1_normalized > 1.0 {
            (hat_x * hat_x + hat_y * hat_y).sqrt() - hat_x / self.r1_normalized
        } else {
            let disc = hat_x * hat_x - hat_y * hat_y;
            if disc < 0.0 {
                return 2.0;
            }
            let root = disc.sqrt();
            if self.swapped || self.one_minus_f < 0.0 {
                -root - hat_x / self.r1_normalized
            } else {
                root - hat_x / self.r1_normalized
            }
        };

        if self.r1_normalized <= 1.0 && hat_x_t < 0.0 {
            return 2.0;
        }

        let mut t = self.f + self.one_minus_f_signum * hat_x_t;
        if self.swapped {
            t = 1.0 - t;
        }
        f64_to_f32(t)
    }
}

fn prepare_two_point_radial(radial: RadialGradientPosition) -> PreparedTwoPointRadial {
    let mut c0 = radial.start_center;
    let mut c1 = radial.end_center;
    let mut r0 = f64::from(radial.start_radius);
    let mut r1 = f64::from(radial.end_radius);
    let mut swapped = false;

    if r1.abs() <= f64::EPSILON {
        core::mem::swap(&mut c0, &mut c1);
        core::mem::swap(&mut r0, &mut r1);
        swapped = true;
    }

    let denom = r0 - r1;
    if denom.abs() <= f64::EPSILON {
        return PreparedTwoPointRadial {
            valid: false,
            x_coeff_x: 0.0,
            x_coeff_y: 0.0,
            x_bias: 0.0,
            y_coeff_x: 0.0,
            y_coeff_y: 0.0,
            y_bias: 0.0,
            f: 0.0,
            one_minus_f: 1.0,
            one_minus_f_abs: 1.0,
            one_minus_f_signum: 1.0,
            r1_normalized: 0.0,
            swapped,
            near_one: false,
            x_prime_scale: 0.0,
            y_prime_scale: 0.0,
        };
    }

    let f = r0 / denom;
    let one_minus_f = 1.0 - f;
    let focal = Point::new(c0.x + (c1.x - c0.x) * f, c0.y + (c1.y - c0.y) * f);
    let axis = c1 - focal;
    let axis_len = axis.hypot();
    if axis_len <= f64::EPSILON {
        return PreparedTwoPointRadial {
            valid: false,
            x_coeff_x: 0.0,
            x_coeff_y: 0.0,
            x_bias: 0.0,
            y_coeff_x: 0.0,
            y_coeff_y: 0.0,
            y_bias: 0.0,
            f: 0.0,
            one_minus_f: 1.0,
            one_minus_f_abs: 1.0,
            one_minus_f_signum: 1.0,
            r1_normalized: 0.0,
            swapped,
            near_one: false,
            x_prime_scale: 0.0,
            y_prime_scale: 0.0,
        };
    }

    let inv_len_sq = 1.0 / (axis_len * axis_len);
    let r1_normalized = r1 / axis_len;
    let near_one = (r1_normalized - 1.0).abs() <= 1e-6;
    let (x_prime_scale, y_prime_scale) = if near_one {
        (0.5, 0.5)
    } else {
        let r1_sq_minus_1 = r1_normalized * r1_normalized - 1.0;
        (
            r1_normalized / r1_sq_minus_1,
            r1_sq_minus_1.abs().sqrt() / r1_sq_minus_1,
        )
    };
    PreparedTwoPointRadial {
        valid: true,
        x_coeff_x: axis.x * inv_len_sq,
        x_coeff_y: axis.y * inv_len_sq,
        x_bias: -(focal.x * axis.x + focal.y * axis.y) * inv_len_sq,
        y_coeff_x: axis.y * inv_len_sq,
        y_coeff_y: -axis.x * inv_len_sq,
        y_bias: (-focal.x * axis.y + focal.y * axis.x) * inv_len_sq,
        f,
        one_minus_f,
        one_minus_f_abs: one_minus_f.abs(),
        one_minus_f_signum: one_minus_f.signum(),
        r1_normalized,
        swapped,
        near_one,
        x_prime_scale,
        y_prime_scale,
    }
}

fn alpha_color_to_tiny_skia(color: color::AlphaColor<Srgb>) -> tiny_skia::Color {
    let color = color.to_rgba8();
    tiny_skia::Color::from_rgba8(color.r, color.g, color.b, color.a)
}

fn opacity_to_u8(opacity: f32) -> u8 {
    let scaled = (opacity.clamp(0.0, 1.0) * 255.0).round();
    if scaled <= 0.0 {
        0
    } else if scaled >= f32::from(u8::MAX) {
        u8::MAX
    } else {
        u8::try_from(f32_to_i32(scaled)).expect("rounded opacity must fit in u8")
    }
}

fn scale_tiny_skia_color_alpha(color: tiny_skia::Color, opacity: f32) -> tiny_skia::Color {
    let rgba = color.to_color_u8();
    tiny_skia::Color::from_rgba8(
        rgba.red(),
        rgba.green(),
        rgba.blue(),
        mul_div_255(rgba.alpha(), opacity_to_u8(opacity)),
    )
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
    let mut rect = layer.draw_bounds?;
    if let Some(layer_clip) = layer.clip_in_root() {
        rect = rect.intersect(layer_clip);
    }
    if !layer.clip_applied_in_content
        && let Some(parent_clip) = parent.clip_in_root()
    {
        rect = rect.intersect(parent_clip);
    }

    if rect.is_zero_area() {
        return None;
    }

    rect_to_int_rect(rect)
}

fn root_rect_to_local_int_rect(layer: &Layer<'_>, rect: IntRect) -> Option<IntRect> {
    let root_rect = Rect::new(
        f64::from(rect.x()),
        f64::from(rect.y()),
        f64::from(rect.x() + i32::try_from(rect.width()).expect("width fits i32")),
        f64::from(rect.y() + i32::try_from(rect.height()).expect("height fits i32")),
    );
    let local = translate_rect(root_rect, -layer.origin.x, -layer.origin.y).intersect(
        Rect::from_origin_size(
            Point::ZERO,
            Size::new(
                f64::from(layer.pixmap.width()),
                f64::from(layer.pixmap.height()),
            ),
        ),
    );
    rect_to_int_rect(local)
}

fn draw_layer_pixmap(
    pixmap: &Pixmap,
    x: i32,
    y: i32,
    parent: &mut Layer<'_>,
    blend_mode: TinyBlendMode,
    alpha: f32,
    apply_parent_clip: bool,
) {
    parent.mark_drawn_local_rect(Rect::new(
        f64::from(x),
        f64::from(y),
        f64::from(x + i32::try_from(pixmap.width()).expect("pixmap width must fit in i32")),
        f64::from(y + i32::try_from(pixmap.height()).expect("pixmap height must fit in i32")),
    ));

    if alpha == 1.0
        && blend_mode == TinyBlendMode::SourceOver
        && parent.blit_pixmap_source_over(pixmap, x, y)
    {
        return;
    }

    let paint = PixmapPaint {
        opacity: alpha,
        blend_mode,
        quality: FilterQuality::Nearest,
    };
    let clip_mask = if apply_parent_clip {
        parent.materialize_simple_clip_mask();
        parent.clip.is_some().then_some(&parent.mask)
    } else {
        None
    };

    parent.pixmap.draw_pixmap(
        x,
        y,
        pixmap.as_ref(),
        &paint,
        Transform::identity(),
        clip_mask,
    );
}

fn draw_layer_region_at(
    parent: &mut Layer<'_>,
    pixmap: PixmapRef<'_>,
    src_rect: IntRect,
    x: i32,
    y: i32,
    blend_mode: TinyBlendMode,
    alpha: f32,
    apply_parent_clip: bool,
) {
    let Some(cropped) = pixmap.clone_rect(src_rect) else {
        return;
    };

    draw_layer_pixmap(&cropped, x, y, parent, blend_mode, alpha, apply_parent_clip);
}

struct LocalPixmapRegion {
    bounds: IntRect,
    pixmap: Pixmap,
}

impl LocalPixmapRegion {
    fn extract(pixmap: PixmapRef<'_>, bounds: IntRect) -> Option<Self> {
        Some(Self {
            bounds,
            pixmap: pixmap.clone_rect(bounds)?,
        })
    }

    fn place_into_full_size(self, target_width: u32, target_height: u32) -> Option<Pixmap> {
        let mut output = Pixmap::new(target_width, target_height)?;
        let origin_x = usize::try_from(self.bounds.x()).expect("origin_x fits usize");
        let origin_y = usize::try_from(self.bounds.y()).expect("origin_y fits usize");
        let target_stride = usize_from_u32(target_width) * 4;
        let local_stride = usize_from_u32(self.pixmap.width()) * 4;
        let local_height = usize_from_u32(self.pixmap.height());
        let src = self.pixmap.data();
        let dst = output.data_mut();

        for row in 0..local_height {
            let src_start = row * local_stride;
            let src_end = src_start + local_stride;
            let dst_start = (origin_y + row) * target_stride + origin_x * 4;
            let dst_end = dst_start + local_stride;
            dst[dst_start..dst_end].copy_from_slice(&src[src_start..src_end]);
        }
        Some(output)
    }
}

fn clone_pixmap_region(pixmap: PixmapRef<'_>, bounds: IntRect) -> Option<Pixmap> {
    pixmap.clone_rect(bounds)
}

fn apply_alpha_mask_from_pixmap(target: &mut Pixmap, mask_source: PixmapRef<'_>) {
    let mask = Mask::from_pixmap(mask_source, MaskType::Alpha);
    target.apply_mask(&mask);
}

fn coverage_from_mask_pixmap(pixmap: &Pixmap, mode: MaskMode) -> Arc<[u8]> {
    match mode {
        MaskMode::Alpha => pixmap
            .data()
            .chunks_exact(4)
            .map(|px| px[3])
            .collect::<Vec<_>>()
            .into(),
        MaskMode::Luminance => pixmap
            .data()
            .chunks_exact(4)
            .map(|px| {
                let coverage =
                    (u16::from(px[0]) * 54 + u16::from(px[1]) * 183 + u16::from(px[2]) * 19 + 127)
                        / 255;
                u8::try_from(coverage.min(u16::from(u8::MAX)))
                    .expect("luminance coverage must fit in u8")
            })
            .collect::<Vec<_>>()
            .into(),
    }
}

fn group_mask_bounds(layer: &Layer<'_>) -> Option<IntRect> {
    let mut bounds = layer.draw_bounds?;
    if let Some(clip) = layer.clip_in_root() {
        bounds = bounds.intersect(clip);
    }
    let surface_bounds = Rect::from_origin_size(
        layer.origin,
        Size::new(
            f64::from(layer.pixmap.width()),
            f64::from(layer.pixmap.height()),
        ),
    );
    bounds = bounds.intersect(surface_bounds);
    if bounds.is_zero_area() {
        return None;
    }
    rect_to_int_rect(bounds)
}

fn apply_group_mask_to_pixmap(target: &mut Pixmap, group_mask: &GroupMask) {
    let mask_source = group_mask.coverage.as_ref();
    let target_width = usize::try_from(target.width()).expect("pixmap width must fit in usize");
    let target_bytes = target.data_mut();
    let mask_width =
        usize::try_from(group_mask.bounds.width()).expect("bounds width must fit in usize");
    let width = usize::try_from(group_mask.bounds.width()).expect("bounds width must fit in usize");
    let height =
        usize::try_from(group_mask.bounds.height()).expect("bounds height must fit in usize");
    let origin_x = usize::try_from(group_mask.bounds.x()).expect("bounds x must fit in usize");
    let origin_y = usize::try_from(group_mask.bounds.y()).expect("bounds y must fit in usize");

    for y in 0..height {
        let target_row = ((origin_y + y) * target_width + origin_x) * 4;
        let mask_row = y * mask_width;
        let target_row_bytes = &mut target_bytes[target_row..target_row + width * 4];
        let mask_row_bytes = &mask_source[mask_row..mask_row + width];
        for (pixel, &coverage) in target_row_bytes
            .chunks_exact_mut(4)
            .zip(mask_row_bytes.iter())
        {
            if coverage == 255 {
                continue;
            }
            pixel[0] = mul_div_255(pixel[0], coverage);
            pixel[1] = mul_div_255(pixel[1], coverage);
            pixel[2] = mul_div_255(pixel[2], coverage);
            pixel[3] = mul_div_255(pixel[3], coverage);
        }
    }
}

fn apply_group_mask(layer: &mut Layer<'_>, group_mask: &GroupMask) {
    let target = match &mut layer.pixmap {
        LayerPixmap::Owned(pixmap) => pixmap,
        LayerPixmap::Borrowed(_) => return,
    };
    let local_bounds = IntRect::from_xywh(
        group_mask.bounds.x() - round_to_i32(layer.origin.x),
        group_mask.bounds.y() - round_to_i32(layer.origin.y),
        group_mask.bounds.width(),
        group_mask.bounds.height(),
    )
    .expect("group mask local bounds must be valid");

    apply_group_mask_to_pixmap(
        target,
        &GroupMask {
            bounds: local_bounds,
            coverage: group_mask.coverage.clone(),
        },
    );
}

fn gaussian_box_radii(sigma: f32) -> Option<[i32; 3]> {
    if !sigma.is_finite() || sigma <= 0.0 {
        return None;
    }

    let n = 3.0_f64;
    let sigma = f64::from(sigma);
    let ideal_width = ((12.0 * sigma * sigma / n) + 1.0).sqrt();
    let mut lower_width = floor_to_i32(ideal_width);
    if lower_width % 2 == 0 {
        lower_width -= 1;
    }
    let upper_width = lower_width + 2;
    let m_ideal = (12.0 * sigma * sigma
        - n * f64::from(lower_width * lower_width)
        - 4.0 * n * f64::from(lower_width)
        - 3.0 * n)
        / (-4.0 * f64::from(lower_width) - 4.0);
    let lower_count = usize::try_from(round_to_i32(m_ideal.clamp(0.0, n)))
        .expect("box blur lower count must fit usize");

    let mut radii = [0_i32; 3];
    for (i, radius) in radii.iter_mut().enumerate() {
        let width = if i < lower_count {
            lower_width
        } else {
            upper_width
        };
        *radius = (width - 1) / 2;
    }
    Some(radii)
}

fn box_blur_pass(
    src: &[PremultipliedColorU8],
    width: usize,
    height: usize,
    radius: i32,
    horizontal: bool,
) -> Vec<PremultipliedColorU8> {
    if radius <= 0 {
        return src.to_vec();
    }

    let mut dst = vec![
        PremultipliedColorU8::from_rgba(0, 0, 0, 0)
            .expect("transparent premultiplied color is valid");
        src.len()
    ];
    let window = u32::try_from(radius * 2 + 1).expect("window fits u32");
    let width_i32 = i32::try_from(width).expect("width fits i32");
    let height_i32 = i32::try_from(height).expect("height fits i32");

    if horizontal {
        for y in 0..height {
            let row = y * width;
            let mut sum_r = 0_u32;
            let mut sum_g = 0_u32;
            let mut sum_b = 0_u32;
            let mut sum_a = 0_u32;

            for sample_x in 0..=radius.min(width_i32 - 1) {
                let px = src[row + usize::try_from(sample_x).expect("sample_x fits usize")];
                sum_r += u32::from(px.red());
                sum_g += u32::from(px.green());
                sum_b += u32::from(px.blue());
                sum_a += u32::from(px.alpha());
            }

            for x in 0..width {
                let idx = row + x;
                dst[idx] = average_premultiplied_channels(sum_r, sum_g, sum_b, sum_a, window);

                let x_i32 = i32::try_from(x).expect("x fits i32");
                let remove_x = x_i32 - radius;
                if remove_x >= 0 {
                    let px = src[row + usize::try_from(remove_x).expect("remove_x fits usize")];
                    sum_r -= u32::from(px.red());
                    sum_g -= u32::from(px.green());
                    sum_b -= u32::from(px.blue());
                    sum_a -= u32::from(px.alpha());
                }

                let add_x = x_i32 + radius + 1;
                if add_x < width_i32 {
                    let px = src[row + usize::try_from(add_x).expect("add_x fits usize")];
                    sum_r += u32::from(px.red());
                    sum_g += u32::from(px.green());
                    sum_b += u32::from(px.blue());
                    sum_a += u32::from(px.alpha());
                }
            }
        }
    } else {
        for x in 0..width {
            let mut sum_r = 0_u32;
            let mut sum_g = 0_u32;
            let mut sum_b = 0_u32;
            let mut sum_a = 0_u32;

            for sample_y in 0..=radius.min(height_i32 - 1) {
                let px = src[usize::try_from(sample_y).expect("sample_y fits usize") * width + x];
                sum_r += u32::from(px.red());
                sum_g += u32::from(px.green());
                sum_b += u32::from(px.blue());
                sum_a += u32::from(px.alpha());
            }

            for y in 0..height {
                let idx = y * width + x;
                dst[idx] = average_premultiplied_channels(sum_r, sum_g, sum_b, sum_a, window);

                let y_i32 = i32::try_from(y).expect("y fits i32");
                let remove_y = y_i32 - radius;
                if remove_y >= 0 {
                    let px =
                        src[usize::try_from(remove_y).expect("remove_y fits usize") * width + x];
                    sum_r -= u32::from(px.red());
                    sum_g -= u32::from(px.green());
                    sum_b -= u32::from(px.blue());
                    sum_a -= u32::from(px.alpha());
                }

                let add_y = y_i32 + radius + 1;
                if add_y < height_i32 {
                    let px = src[usize::try_from(add_y).expect("add_y fits usize") * width + x];
                    sum_r += u32::from(px.red());
                    sum_g += u32::from(px.green());
                    sum_b += u32::from(px.blue());
                    sum_a += u32::from(px.alpha());
                }
            }
        }
    }

    dst
}

fn average_premultiplied_channels(
    sum_r: u32,
    sum_g: u32,
    sum_b: u32,
    sum_a: u32,
    window: u32,
) -> PremultipliedColorU8 {
    PremultipliedColorU8::from_rgba(
        u8::try_from((sum_r + window / 2) / window).expect("r fits u8"),
        u8::try_from((sum_g + window / 2) / window).expect("g fits u8"),
        u8::try_from((sum_b + window / 2) / window).expect("b fits u8"),
        u8::try_from((sum_a + window / 2) / window).expect("a fits u8"),
    )
    .expect("box blur of premultiplied colors must remain premultiplied")
}

fn blur_pixmap(pixmap: &Pixmap, sigma_x: f32, sigma_y: f32) -> Option<Pixmap> {
    let width = usize_from_u32(pixmap.width());
    let height = usize_from_u32(pixmap.height());
    let mut current = pixmap.pixels().to_vec();

    if let Some(radii_x) = gaussian_box_radii(sigma_x) {
        for radius in radii_x {
            current = box_blur_pass(&current, width, height, radius, true);
        }
    }
    if let Some(radii_y) = gaussian_box_radii(sigma_y) {
        for radius in radii_y {
            current = box_blur_pass(&current, width, height, radius, false);
        }
    }

    let mut blurred = Pixmap::new(pixmap.width(), pixmap.height())?;
    blurred.pixels_mut().copy_from_slice(&current);
    Some(blurred)
}

fn offset_pixmap(pixmap: &Pixmap, dx: i32, dy: i32) -> Option<Pixmap> {
    let mut shifted = Pixmap::new(pixmap.width(), pixmap.height())?;
    shifted.draw_pixmap(
        dx,
        dy,
        pixmap.as_ref(),
        &PixmapPaint {
            opacity: 1.0,
            blend_mode: TinyBlendMode::SourceOver,
            quality: FilterQuality::Nearest,
        },
        Transform::identity(),
        None,
    );
    Some(shifted)
}

fn drop_shadow_pixmap(
    pixmap: &Pixmap,
    dx: i32,
    dy: i32,
    sigma_x: f32,
    sigma_y: f32,
    color: Color,
) -> Option<Pixmap> {
    let mut shadow = Pixmap::new(pixmap.width(), pixmap.height())?;
    let tint = color.to_rgba8();
    for (src, dst) in pixmap.pixels().iter().zip(shadow.pixels_mut()) {
        let alpha = mul_div_255(src.alpha(), tint.a);
        *dst = tiny_skia::Color::from_rgba8(tint.r, tint.g, tint.b, alpha)
            .premultiply()
            .to_color_u8();
    }

    let blurred = blur_pixmap(&shadow, sigma_x, sigma_y)?;
    let mut combined = offset_pixmap(&blurred, dx, dy)?;
    combined.draw_pixmap(
        0,
        0,
        pixmap.as_ref(),
        &PixmapPaint {
            opacity: 1.0,
            blend_mode: TinyBlendMode::SourceOver,
            quality: FilterQuality::Nearest,
        },
        Transform::identity(),
        None,
    );
    Some(combined)
}

fn translate_rect(rect: Rect, dx: f64, dy: f64) -> Rect {
    Rect::new(rect.x0 + dx, rect.y0 + dy, rect.x1 + dx, rect.y1 + dy)
}

fn blur_bounds(rect: Rect, sigma_x: f32, sigma_y: f32) -> Rect {
    let pad_x = (f64::from(sigma_x) * 3.0).ceil();
    let pad_y = (f64::from(sigma_y) * 3.0).ceil();
    Rect::new(
        rect.x0 - pad_x,
        rect.y0 - pad_y,
        rect.x1 + pad_x,
        rect.y1 + pad_y,
    )
}

fn pixmap_region_bounds(pixmap: &Pixmap, rect: Rect) -> Option<IntRect> {
    let bounds = rect.intersect(Rect::from_origin_size(
        Point::ZERO,
        Size::new(f64::from(pixmap.width()), f64::from(pixmap.height())),
    ));
    rect_to_int_rect(bounds)
}

fn blur_pixmap_local(
    pixmap: &Pixmap,
    bounds: Option<Rect>,
    sigma_x: f32,
    sigma_y: f32,
) -> Option<Pixmap> {
    let Some(bounds) = bounds else {
        return blur_pixmap(pixmap, sigma_x, sigma_y);
    };
    let region = pixmap_region_bounds(pixmap, blur_bounds(bounds, sigma_x, sigma_y))?;
    let mut local = LocalPixmapRegion::extract(pixmap.as_ref(), region)?;
    local.pixmap = blur_pixmap(&local.pixmap, sigma_x, sigma_y)?;
    local.place_into_full_size(pixmap.width(), pixmap.height())
}

fn drop_shadow_pixmap_local(
    pixmap: &Pixmap,
    bounds: Option<Rect>,
    dx: i32,
    dy: i32,
    sigma_x: f32,
    sigma_y: f32,
    color: Color,
) -> Option<Pixmap> {
    let Some(bounds) = bounds else {
        return drop_shadow_pixmap(pixmap, dx, dy, sigma_x, sigma_y, color);
    };

    let padded = blur_bounds(bounds, sigma_x, sigma_y);
    let shadow_bounds = translate_rect(padded, f64::from(dx), f64::from(dy));
    let region = pixmap_region_bounds(pixmap, bounds.union(shadow_bounds))?;
    let mut local = LocalPixmapRegion::extract(pixmap.as_ref(), region)?;
    local.pixmap = drop_shadow_pixmap(&local.pixmap, dx, dy, sigma_x, sigma_y, color)?;
    local.place_into_full_size(pixmap.width(), pixmap.height())
}

fn filter_output_bounds(bounds: Option<Rect>, filter: &Filter, pixmap: &Pixmap) -> Option<Rect> {
    match *filter {
        Filter::Flood { .. } => Some(Rect::from_origin_size(
            Point::ZERO,
            Size::new(f64::from(pixmap.width()), f64::from(pixmap.height())),
        )),
        Filter::Blur {
            std_deviation_x,
            std_deviation_y,
        } => bounds.map(|rect| blur_bounds(rect, std_deviation_x, std_deviation_y)),
        Filter::Offset { dx, dy } => {
            let dx = f64::from(round_to_i32(f64::from(dx)));
            let dy = f64::from(round_to_i32(f64::from(dy)));
            bounds.map(|rect| translate_rect(rect, dx, dy))
        }
        Filter::DropShadow {
            dx,
            dy,
            std_deviation_x,
            std_deviation_y,
            ..
        } => {
            let dx = f64::from(round_to_i32(f64::from(dx)));
            let dy = f64::from(round_to_i32(f64::from(dy)));
            bounds.map(|rect| {
                let shadow =
                    translate_rect(blur_bounds(rect, std_deviation_x, std_deviation_y), dx, dy);
                rect.union(shadow)
            })
        }
    }
}

fn filter_pixmap(pixmap: &Pixmap, bounds: Option<Rect>, filter: &Filter) -> Option<Pixmap> {
    match *filter {
        Filter::Flood { color } => {
            let mut flooded = Pixmap::new(pixmap.width(), pixmap.height())?;
            flooded.fill(to_color(color));
            Some(flooded)
        }
        Filter::Blur {
            std_deviation_x,
            std_deviation_y,
        } => blur_pixmap_local(pixmap, bounds, std_deviation_x, std_deviation_y),
        Filter::DropShadow {
            dx,
            dy,
            std_deviation_x,
            std_deviation_y,
            color,
        } => drop_shadow_pixmap_local(
            pixmap,
            bounds,
            round_to_i32(f64::from(dx)),
            round_to_i32(f64::from(dy)),
            std_deviation_x,
            std_deviation_y,
            color,
        ),
        Filter::Offset { dx, dy } => offset_pixmap(
            pixmap,
            round_to_i32(f64::from(dx)),
            round_to_i32(f64::from(dy)),
        ),
    }
}

fn apply_group_filters(layer: &mut Layer<'_>) {
    if layer.filters.is_empty() {
        return;
    }
    let Some(pixmap) = (match &layer.pixmap {
        LayerPixmap::Owned(pixmap) => Some(pixmap.clone()),
        LayerPixmap::Borrowed(_) => None,
    }) else {
        return;
    };

    let mut current = pixmap;
    let mut bounds = layer.draw_bounds;
    for filter in &layer.filters {
        let Some(next) = filter_pixmap(&current, bounds, filter) else {
            continue;
        };
        bounds = filter_output_bounds(bounds, filter, &current);
        current = next;
    }

    if let LayerPixmap::Owned(pixmap) = &mut layer.pixmap {
        *pixmap = current;
    }
    layer.draw_bounds = bounds;
}

fn apply_layer_multipass(
    layer: &Layer<'_>,
    parent: &mut Layer<'_>,
    layer_rect: IntRect,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    first_pass: TinyBlendMode,
    second_pass: TinyBlendMode,
) {
    let parent_rect = IntRect::from_xywh(x, y, width, height).expect("parent rect must be valid");
    let Some(original_parent) = parent.pixmap.clone_rect(parent_rect) else {
        return;
    };
    let Some(coverage) = layer.pixmap.clone_rect(layer_rect) else {
        return;
    };

    draw_layer_region_at(
        parent,
        layer.pixmap.as_ref(),
        layer_rect,
        x,
        y,
        first_pass,
        1.0,
        !layer.clip_applied_in_content,
    );

    let Some(mut intermediate) = parent.pixmap.clone_rect(parent_rect) else {
        return;
    };
    apply_alpha_mask_from_pixmap(&mut intermediate, coverage.as_ref());

    draw_layer_pixmap(
        &original_parent,
        x,
        y,
        parent,
        TinyBlendMode::Source,
        1.0,
        true,
    );
    draw_layer_pixmap(
        &intermediate,
        x,
        y,
        parent,
        second_pass,
        layer.alpha,
        !layer.clip_applied_in_content,
    );
}

fn apply_layer(layer: &Layer<'_>, parent: &mut Layer<'_>) {
    let Some(composite_rect) = layer_composite_rect(layer, parent) else {
        return;
    };
    let Some(layer_rect) = root_rect_to_local_int_rect(layer, composite_rect) else {
        return;
    };
    let x = composite_rect.x() - round_to_i32(parent.origin.x);
    let y = composite_rect.y() - round_to_i32(parent.origin.y);

    match determine_blend_strategy(&layer.blend_mode) {
        BlendStrategy::SinglePass(blend_mode) => {
            draw_layer_region_at(
                parent,
                layer.pixmap.as_ref(),
                layer_rect,
                x,
                y,
                blend_mode,
                layer.alpha,
                !layer.clip_applied_in_content,
            );
        }
        BlendStrategy::MultiPass {
            first_pass,
            second_pass,
        } => apply_layer_multipass(
            layer,
            parent,
            layer_rect,
            x,
            y,
            composite_rect.width(),
            composite_rect.height(),
            first_pass,
            second_pass,
        ),
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
    use peniko::{Blob, ImageAlphaType, ImageData, ImageFormat};

    /// Creates a `Layer` directly without a window, for offscreen rendering.
    fn make_layer(width: u32, height: u32) -> Layer<'static> {
        Layer {
            pixmap: LayerPixmap::Owned(
                Pixmap::new(width, height).expect("failed to create pixmap"),
            ),
            origin: Point::ZERO,
            base_clip: None,
            clip_stack: vec![],
            clip: None,
            simple_clip: None,
            draw_bounds: None,
            mask: Mask::new(width, height).expect("failed to create mask"),
            mask_valid: false,
            transform: Affine::IDENTITY,
            blend_mode: Mix::Normal.into(),
            alpha: 1.0,
            group_mask: None,
            filters: Vec::new(),
            clip_applied_in_content: false,
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
    fn scaled_image_cache_is_used_for_rect_image_fills() {
        let image = ImageData {
            data: Blob::new(Arc::new([
                255_u8, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ])),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: 2,
            height: 2,
        };
        let brush = peniko::ImageBrush::new(image)
            .with_extend(Extend::Pad)
            .with_quality(ImageQuality::Medium);

        let mut caches = RendererCaches::new();
        let first = cache_scaled_image_pixmap(&mut caches, CacheColor(false), &brush, 8, 8)
            .expect("scale image");
        assert_eq!(caches.image_cache.len(), 1);
        assert_eq!(caches.scaled_image_cache.len(), 1);

        let second = cache_scaled_image_pixmap(&mut caches, CacheColor(true), &brush, 8, 8)
            .expect("reuse scaled image");
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(caches.image_cache.len(), 1);
        assert_eq!(caches.scaled_image_cache.len(), 1);
    }

    #[test]
    fn blurred_rrect_cache_is_used_for_translation_only_draws() {
        let mut caches = RendererCaches::new();
        let draw = BlurredRoundedRect {
            transform: Affine::translate((0.0, 8.0)),
            rect: Rect::new(8.5, 10.25, 40.5, 46.25),
            color: Color::from_rgba8(23, 33, 66, 150),
            radius: 12.0,
            std_dev: 6.0,
            composite: Composite::default(),
        };

        let mut first_layer = make_layer(64, 64);
        assert!(render_cached_blurred_rounded_rect(
            &mut caches,
            CacheColor(false),
            &mut first_layer,
            &draw,
            TinyBlendMode::SourceOver,
        ));
        assert_eq!(caches.blurred_rrect_cache.len(), 1);
        let cached = caches
            .blurred_rrect_cache
            .values()
            .next()
            .expect("cached blurred rounded rect")
            .1
            .clone();

        let mut second_layer = make_layer(64, 64);
        assert!(render_cached_blurred_rounded_rect(
            &mut caches,
            CacheColor(true),
            &mut second_layer,
            &draw,
            TinyBlendMode::SourceOver,
        ));
        let reused = caches
            .blurred_rrect_cache
            .values()
            .next()
            .expect("cached blurred rounded rect")
            .1
            .clone();
        assert!(Arc::ptr_eq(&cached, &reused));
    }

    #[test]
    fn mixed_axis_image_extend_does_not_collapse_to_pad() {
        let image = ImageData {
            data: Blob::new(Arc::new([
                255_u8, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ])),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: 2,
            height: 2,
        };
        let brush = peniko::ImageBrush::new(image)
            .with_quality(ImageQuality::Low)
            .with_x_extend(Extend::Repeat)
            .with_y_extend(Extend::Pad);

        let mut layer = make_layer(6, 4);
        layer.fill(&Rect::new(0.0, 0.0, 6.0, 4.0), &brush);

        assert_eq!(pixel_rgba(&layer, 0, 0), (255, 0, 0, 255));
        assert_eq!(pixel_rgba(&layer, 2, 0), (255, 0, 0, 255));
        assert_eq!(pixel_rgba(&layer, 4, 0), (255, 0, 0, 255));
        assert_eq!(pixel_rgba(&layer, 2, 3), (0, 0, 255, 255));
    }

    #[test]
    fn nested_layer_marks_parent_draw_bounds() {
        let mut root = make_layer(8, 8);
        let mut parent = make_layer(8, 8);
        let mut child = make_layer(8, 8);

        child.fill(&Rect::new(2.0, 2.0, 4.0, 4.0), Color::from_rgb8(255, 0, 0));

        apply_layer(&child, &mut parent);
        assert!(parent.draw_bounds.is_some());

        apply_layer(&parent, &mut root);
        assert_eq!(pixel_rgba(&root, 3, 3), (255, 0, 0, 255));
    }

    #[test]
    fn blur_pixmap_spreads_alpha_outside_source_pixels() {
        let mut pixmap = Pixmap::new(7, 7).expect("pixmap");
        pixmap.fill_rect(
            tiny_skia::Rect::from_xywh(3.0, 3.0, 1.0, 1.0).expect("rect"),
            &Paint {
                shader: Shader::SolidColor(tiny_skia::Color::BLACK),
                ..Default::default()
            },
            Transform::identity(),
            None,
        );

        let blurred = blur_pixmap(&pixmap, 1.5, 1.5).expect("blurred pixmap");
        let neighbor = blurred.pixel(2, 3).expect("neighbor pixel");
        assert!(neighbor.alpha() > 0);
    }

    #[test]
    fn apply_group_filters_expands_blur_beyond_original_draw_bounds() {
        let mut layer = make_layer(32, 32);
        layer.fill(&Rect::new(12.0, 12.0, 20.0, 20.0), Color::BLACK);
        layer.filters.push(Filter::Blur {
            std_deviation_x: 3.0,
            std_deviation_y: 3.0,
        });

        apply_group_filters(&mut layer);

        let outside = pixel_rgba(&layer, 9, 16);
        assert!(outside.3 > 0);
        let bounds = layer.draw_bounds.expect("filtered draw bounds");
        assert!(bounds.x0 < 12.0);
        assert!(bounds.x1 > 20.0);
    }

    #[test]
    fn unpremultiply_lookup_matches_exact_formula() {
        for alpha in 0..=u8::MAX {
            for channel in 0..=u8::MAX {
                let expected = if alpha == 0 {
                    0
                } else if alpha == u8::MAX {
                    channel
                } else {
                    let value =
                        (u32::from(channel) * 255 + u32::from(alpha) / 2) / u32::from(alpha);
                    u8::try_from(value.min(u32::from(u8::MAX))).expect("fits u8")
                };
                assert_eq!(unpremultiply_channel(channel, alpha), expected);
            }
        }
    }

    #[test]
    fn multipass_blend_respects_non_rect_clip_coverage() {
        let mut parent = make_layer(8, 8);
        parent.fill(&Rect::new(0.0, 0.0, 8.0, 8.0), Color::from_rgb8(0, 0, 255));

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
            stroke_source: None,
        }));

        child.fill(&Rect::new(0.0, 0.0, 6.0, 6.0), Color::from_rgb8(255, 0, 0));

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
        plain.fill(&Rect::new(0.0, 0.0, 8.0, 2.0), &gradient);

        let mut transformed = make_layer(8, 2);
        transformed.fill_with_brush_transform(
            &Rect::new(0.0, 0.0, 8.0, 2.0),
            &gradient,
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

        layer.fill(&Rect::new(0.0, 0.0, 101.0, 1.0), &gradient);

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

        layer.fill(&Rect::new(0.0, 0.0, 101.0, 1.0), &gradient);

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
    fn default_group_with_clip_does_not_isolate() {
        let mut renderer = TinySkiaRenderer::new_with_size(16, 16).expect("renderer");
        let clip = ClipRef::fill(imaging::GeometryRef::Rect(Rect::new(2.0, 2.0, 14.0, 14.0)));

        renderer.push_group(GroupRef::new().with_clip(clip));

        assert_eq!(renderer.layers.len(), 1);
        assert_eq!(
            renderer.group_frames,
            vec![GroupFrame::Direct { pushed_clip: true }]
        );
        assert_eq!(renderer.current_layer().clip_stack.len(), 1);

        renderer.pop_group();

        assert!(renderer.group_frames.is_empty());
        assert_eq!(renderer.layers.len(), 1);
        assert!(renderer.current_layer().clip_stack.is_empty());
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
