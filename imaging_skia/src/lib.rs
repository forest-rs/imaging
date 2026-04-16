// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

#![allow(
    unsafe_code,
    reason = "Skia Ganesh interop needs raw wgpu-hal handle access in the gpu modules"
)]

//! Skia backend for `imaging`.
//!
//! This crate provides a CPU raster renderer that consumes `imaging::record::Scene` or native
//! Skia draw targets and produces an RGBA8 image buffer using Skia.
//!
//! # Render A Recorded Scene
//!
//! Record commands into [`imaging::record::Scene`], then hand the scene to [`SkiaCpuRenderer`].
//!
//! ```no_run
//! use imaging::{Painter, record};
//! use imaging_skia::SkiaCpuRenderer;
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
//!     let mut renderer = SkiaCpuRenderer::new();
//!     let image = renderer.render_scene(&scene, 128, 128)?;
//!     assert_eq!(image.data.len(), 128 * 128 * 4);
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
//! If you already have a recorded picture, hand it directly to [`SkiaCpuRenderer`].
//!
//! ```no_run
//! use imaging::Painter;
//! use imaging_skia::{SkPictureRecorderSink, SkiaCpuRenderer};
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
//!     let mut renderer = SkiaCpuRenderer::new();
//!     let image = renderer.render_picture(&picture, 128, 128)?;
//!     assert_eq!(image.data.len(), 128 * 128 * 4);
//!     Ok(())
//! }
//! ```
//!
//! # GPU Rendering
//!
//! Enable the `gpu` feature when you want Skia Ganesh rendering through app-owned `wgpu`
//! handles. [`SkiaRenderer`] reuses the current backend selected by `wgpu` and renders
//! native [`skia_safe::Picture`] values into caller-owned `wgpu::Texture` targets while also
//! supporting RGBA8 image output.
//!
//! ```no_run
//! # #[cfg(feature = "gpu")]
//! # {
//! use imaging::{Painter, record};
//! use imaging_skia::SkiaRenderer;
//! use kurbo::Rect;
//! use peniko::{Brush, Color};
//!
//! # let adapter: imaging_skia::wgpu::Adapter = todo!();
//! # let device: imaging_skia::wgpu::Device = todo!();
//! # let queue: imaging_skia::wgpu::Queue = todo!();
//! # let texture: imaging_skia::wgpu::Texture = todo!();
//! let mut scene = record::Scene::new();
//! {
//!     let mut painter = Painter::new(&mut scene);
//!     painter.fill_rect(
//!         Rect::new(0.0, 0.0, 128.0, 128.0),
//!         &Brush::Solid(Color::from_rgb8(0x2a, 0x6f, 0xdb)),
//!     );
//! }
//!
//! let mut renderer = SkiaRenderer::new(adapter, device, queue)?;
//! let picture = renderer.encode_scene(&scene, 128, 128)?;
//! renderer.render_picture_to_texture(&picture, &texture)?;
//! # }
//! # Ok::<(), imaging_skia::Error>(())
//! ```

#![cfg_attr(not(feature = "gpu"), deny(unsafe_code))]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

#[cfg(all(feature = "gpu", windows))]
mod d3d;
#[cfg(feature = "gpu")]
mod ganesh;
#[cfg(feature = "gpu")]
mod gpu_readback;
#[cfg(all(feature = "gpu", any(target_os = "macos", target_os = "ios")))]
mod metal;
mod sinks;
#[cfg(all(feature = "gpu", not(any(target_os = "macos", target_os = "ios"))))]
mod vulkan;

#[cfg(all(feature = "gpu", any(target_os = "macos", target_os = "ios")))]
use foreign_types_shared as _;
use imaging::{
    Filter, GeometryRef, GlyphRunRef, RgbaImage,
    record::{Scene, ValidateError, replay},
    render::{
        ImageBufferFormat, ImageBufferTarget, ImageRenderer, ImageRendererError, ImageTargetError,
        RenderContentError, RenderSource, RenderUnsupportedError,
    },
};
use kurbo::{Affine, Shape as _};
use peniko::color::{ColorSpaceTag, HueDirection};
use peniko::{
    BrushRef, ImageAlphaType, ImageData, ImageFormat, ImageQuality, InterpolationAlphaSpace,
};
use skia_safe as sk;
use std::{
    cell::{RefCell, RefMut},
    collections::{HashMap, VecDeque},
    rc::Rc,
};

#[cfg(feature = "gpu")]
use crate::ganesh::GaneshBackend;
#[cfg(feature = "gpu")]
use crate::gpu_readback::{
    ReadbackError, ScratchTexture, read_texture_into, read_texture_into_target,
};
#[cfg(feature = "gpu")]
use imaging_wgpu::{TextureRenderer, TextureRendererError, TextureTargetError};
use sinks::MaskCache;
pub use sinks::{SkCanvasSink, SkPictureRecorderSink};
#[cfg(feature = "gpu")]
pub use wgpu;

/// Errors that can occur when rendering via Skia.
#[derive(Debug)]
pub enum Error {
    /// The scene is invalid (unbalanced stacks).
    InvalidScene(ValidateError),
    /// No supported Ganesh backend was available for the active platform or `wgpu` backend.
    #[cfg(feature = "gpu")]
    UnsupportedGpuBackend,
    /// A Ganesh backend context could not be created from the supplied `wgpu` handles.
    #[cfg(feature = "gpu")]
    CreateGpuContext(&'static str),
    /// A caller-owned GPU texture could not be wrapped as a Skia surface.
    #[cfg(feature = "gpu")]
    CreateGpuSurface,
    /// The target texture format cannot be represented through Skia Ganesh.
    #[cfg(feature = "gpu")]
    UnsupportedGpuTextureFormat,
    /// The target image buffer format cannot be represented through Skia.
    UnsupportedImageTargetFormat,
    /// An image brush was encountered; this backend does not support it.
    UnsupportedImageBrush,
    /// A filter configuration could not be translated.
    UnsupportedFilter,
    /// A glyph run used a per-glyph transform unsupported by this backend.
    UnsupportedGlyphTransform,
    /// Font bytes could not be loaded by Skia.
    InvalidFontData,
    /// A glyph identifier could not be represented by Skia's glyph type.
    InvalidGlyphId,
    /// An internal invariant was violated.
    Internal(&'static str),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl core::error::Error for Error {}

/// Share Skia font and typeface caches across renderer instances.
///
/// This is useful when multiple raster or GPU renderers draw text from the same font set and you
/// want them to reuse the same resolved Skia font state.
#[derive(Clone, Debug)]
pub struct SkiaFontCache {
    inner: Rc<RefCell<FontCache>>,
}

impl Default for SkiaFontCache {
    fn default() -> Self {
        Self::new()
    }
}

impl SkiaFontCache {
    /// Create an empty shared font cache.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(FontCache::new())),
        }
    }

    fn borrow_mut(&self) -> RefMut<'_, FontCache> {
        self.inner.borrow_mut()
    }

    fn clear(&self) {
        self.inner.borrow_mut().clear();
    }

    #[cfg(test)]
    fn counts(&self) -> (usize, usize, usize) {
        self.inner.borrow().counts()
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ImageCacheKey {
    blob_id: u64,
    format: core::mem::Discriminant<ImageFormat>,
    alpha_type: core::mem::Discriminant<ImageAlphaType>,
    width: u32,
    height: u32,
}

impl ImageCacheKey {
    fn new(image: &ImageData) -> Self {
        Self {
            blob_id: image.data.id(),
            format: core::mem::discriminant(&image.format),
            alpha_type: core::mem::discriminant(&image.alpha_type),
            width: image.width,
            height: image.height,
        }
    }
}

#[derive(Clone, Debug)]
struct CachedImage {
    key: ImageCacheKey,
    image: sk::Image,
    bytes: usize,
}

#[derive(Debug)]
struct ImageCache {
    bytes_used: usize,
    max_bytes: usize,
    entries: VecDeque<CachedImage>,
}

impl ImageCache {
    fn new(max_bytes: usize) -> Self {
        Self {
            bytes_used: 0,
            max_bytes,
            entries: VecDeque::new(),
        }
    }

    fn clear(&mut self) {
        self.bytes_used = 0;
        self.entries.clear();
    }

    fn set_max_bytes(&mut self, max_bytes: usize) {
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
            self.bytes_used = self.bytes_used.saturating_sub(oldest.bytes);
        }
    }

    fn get_or_create(&mut self, image: &ImageData) -> Option<sk::Image> {
        let key = ImageCacheKey::new(image);
        if let Some(index) = self.entries.iter().position(|entry| entry.key == key) {
            let cached = self.entries.get(index)?.image.clone();
            self.touch(index);
            return Some(cached);
        }

        let cached = CachedImage {
            key,
            image: make_skia_image_from_peniko(image)?,
            bytes: image
                .format
                .size_in_bytes(image.width, image.height)
                .unwrap_or_else(|| image.data.data().len()),
        };
        let image = cached.image.clone();
        self.bytes_used = self.bytes_used.saturating_add(cached.bytes);
        self.entries.push_back(cached);
        self.evict_to_budget();
        Some(image)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

impl Default for ImageCache {
    fn default() -> Self {
        Self::new(64 * 1024 * 1024)
    }
}

/// Shared cache bundle for Skia renderers.
///
/// This exists so renderer construction does not grow a new constructor every time more shareable
/// Skia state is introduced.
#[derive(Clone, Debug, Default)]
pub struct SkiaCaches {
    font_cache: SkiaFontCache,
    image_cache: Rc<RefCell<ImageCache>>,
    mask_cache: Rc<RefCell<MaskCache>>,
}

impl SkiaCaches {
    /// Create a cache bundle with fresh default caches.
    #[must_use]
    pub fn new() -> Self {
        Self {
            font_cache: SkiaFontCache::new(),
            image_cache: Rc::new(RefCell::new(ImageCache::default())),
            mask_cache: Rc::new(RefCell::new(MaskCache::default())),
        }
    }

    /// Replace the shared font cache in this bundle.
    #[must_use]
    pub fn with_font_cache(mut self, font_cache: SkiaFontCache) -> Self {
        self.font_cache = font_cache;
        self
    }

    fn font_cache(&self) -> SkiaFontCache {
        self.font_cache.clone()
    }

    fn image_cache(&self) -> Rc<RefCell<ImageCache>> {
        Rc::clone(&self.image_cache)
    }
    fn mask_cache(&self) -> Rc<RefCell<MaskCache>> {
        Rc::clone(&self.mask_cache)
    }

    fn clear(&self) {
        self.font_cache.clear();
        self.image_cache.borrow_mut().clear();
        self.mask_cache.borrow_mut().clear();
    }

    fn set_image_cache_total_bytes_limit(&self, limit: usize) {
        self.image_cache.borrow_mut().set_max_bytes(limit);
    }
    fn set_mask_cache_total_bytes_limit(&self, limit: usize) {
        self.mask_cache.borrow_mut().set_max_bytes(limit);
    }
}

/// Configurable cache and resource budgets for Skia-backed rendering.
///
/// These limits are applied through Skia's process-global `graphics` cache settings when a
/// renderer is constructed from [`SkiaConfig`]. If multiple renderers apply different cache
/// configs, the most recently constructed renderer wins for Skia's global resource limits.
#[derive(Clone, Copy, Debug)]
pub struct SkiaCacheConfig {
    /// Maximum number of entries Skia keeps in its global font cache.
    pub font_cache_count_limit: i32,
    /// Maximum number of cached typefaces Skia keeps globally.
    pub typeface_cache_count_limit: i32,
    /// Maximum number of bytes Skia keeps in its global resource cache.
    pub resource_cache_total_bytes_limit: usize,
    /// Maximum size of a single entry in Skia's global resource cache.
    pub resource_cache_single_allocation_byte_limit: Option<usize>,
    /// Maximum number of bytes retained by the shared realized-image cache.
    pub image_cache_total_bytes_limit: usize,
    /// Maximum number of bytes retained by the shared realized mask cache.
    pub mask_cache_total_bytes_limit: usize,
}

impl Default for SkiaCacheConfig {
    fn default() -> Self {
        Self {
            font_cache_count_limit: 100,
            typeface_cache_count_limit: 100,
            resource_cache_total_bytes_limit: 10 * 1024 * 1024,
            resource_cache_single_allocation_byte_limit: None,
            image_cache_total_bytes_limit: 64 * 1024 * 1024,
            mask_cache_total_bytes_limit: 64 * 1024 * 1024,
        }
    }
}

impl SkiaCacheConfig {
    /// Create cache budgets with the default limits.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the Skia global font-cache entry limit.
    #[must_use]
    pub fn with_font_cache_count_limit(mut self, limit: i32) -> Self {
        self.font_cache_count_limit = limit;
        self
    }

    /// Override the Skia global typeface-cache entry limit.
    #[must_use]
    pub fn with_typeface_cache_count_limit(mut self, limit: i32) -> Self {
        self.typeface_cache_count_limit = limit;
        self
    }

    /// Override the Skia global resource-cache byte limit.
    #[must_use]
    pub fn with_resource_cache_total_bytes_limit(mut self, limit: usize) -> Self {
        self.resource_cache_total_bytes_limit = limit;
        self
    }

    /// Override the Skia global single-allocation resource-cache byte limit.
    #[must_use]
    pub fn with_resource_cache_single_allocation_byte_limit(
        mut self,
        limit: Option<usize>,
    ) -> Self {
        self.resource_cache_single_allocation_byte_limit = limit;
        self
    }

    /// Override the shared realized-image cache byte limit.
    #[must_use]
    pub fn with_image_cache_total_bytes_limit(mut self, limit: usize) -> Self {
        self.image_cache_total_bytes_limit = limit;
        self
    }

    /// Override the shared realized-mask cache byte limit.
    #[must_use]
    pub fn with_mask_cache_total_bytes_limit(mut self, limit: usize) -> Self {
        self.mask_cache_total_bytes_limit = limit;
        self
    }

    fn apply(self) {
        sk::graphics::set_font_cache_count_limit(self.font_cache_count_limit);
        sk::graphics::set_typeface_cache_count_limit(self.typeface_cache_count_limit);
        sk::graphics::set_resource_cache_total_bytes_limit(self.resource_cache_total_bytes_limit);
        sk::graphics::set_resource_cache_single_allocation_byte_limit(
            self.resource_cache_single_allocation_byte_limit,
        );
    }
}

/// Shared renderer configuration for Skia backends.
///
/// This groups shareable renderer state and cache budgets so construction stays stable as Skia
/// grows more configurable over time.
#[derive(Clone, Debug, Default)]
pub struct SkiaConfig {
    caches: SkiaCaches,
    cache_config: SkiaCacheConfig,
}

impl SkiaConfig {
    /// Create renderer configuration with fresh default caches and cache budgets.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the shared caches in this config.
    #[must_use]
    pub fn with_caches(mut self, caches: SkiaCaches) -> Self {
        self.caches = caches;
        self
    }

    /// Replace the cache budgets in this config.
    #[must_use]
    pub fn with_cache_config(mut self, cache_config: SkiaCacheConfig) -> Self {
        self.cache_config = cache_config;
        self
    }
}

/// Renderer that executes `imaging` commands using a Skia raster surface.
#[derive(Debug)]
pub struct SkiaCpuRenderer {
    surface: sk::Surface,
    width: i32,
    height: i32,
    tolerance: f64,
    caches: SkiaCaches,
}

impl Default for SkiaCpuRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl SkiaCpuRenderer {
    fn checked_size(width: u32, height: u32) -> Result<(i32, i32), Error> {
        let width = i32::try_from(width).map_err(|_| Error::Internal("render width too large"))?;
        let height =
            i32::try_from(height).map_err(|_| Error::Internal("render height too large"))?;
        Ok((width, height))
    }

    fn create_surface(width: i32, height: i32) -> sk::Surface {
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
        sk::surfaces::raster(&info, None, None).expect("create skia raster RGBA8888/premul surface")
    }

    /// Create a renderer.
    pub fn new() -> Self {
        Self::new_with_config(SkiaConfig::new())
    }

    /// Create a renderer using the provided shared caches and cache budgets.
    pub fn new_with_config(config: SkiaConfig) -> Self {
        config.cache_config.apply();
        config
            .caches
            .set_image_cache_total_bytes_limit(config.cache_config.image_cache_total_bytes_limit);
        config
            .caches
            .set_mask_cache_total_bytes_limit(config.cache_config.mask_cache_total_bytes_limit);
        let surface = Self::create_surface(1, 1);
        Self {
            surface,
            width: 1,
            height: 1,
            tolerance: 0.1,
            caches: config.caches,
        }
    }

    /// Set the tolerance used when converting shapes to paths.
    pub fn set_tolerance(&mut self, tolerance: f64) {
        if self.tolerance != tolerance {
            self.caches.mask_cache().borrow_mut().clear();
        }
        self.tolerance = tolerance;
    }

    /// Drop any realized mask artifacts cached by the renderer.
    ///
    /// The cache is shared through [`SkiaCaches`], so unchanged masked subscenes can be reused
    /// across compatible renderers. Call this if you need to release memory aggressively or after
    /// changing assumptions that affect mask realization outside the recorded scene itself.
    pub fn clear_cached_masks(&mut self) {
        self.caches.mask_cache().borrow_mut().clear();
    }

    /// Drop any realized image resources cached by the renderer.
    pub fn clear_cached_images(&mut self) {
        self.caches.image_cache().borrow_mut().clear();
    }

    /// Drop all renderer-local caches, including shared font state.
    pub fn clear_caches(&mut self) {
        self.caches.clear();
    }

    fn resize(&mut self, width: i32, height: i32) {
        if self.width == width && self.height == height {
            return;
        }

        self.surface = Self::create_surface(width, height);
        self.width = width;
        self.height = height;
        self.clear_cached_masks();
    }

    fn reset(&mut self) {
        let canvas = self.surface.canvas();
        canvas.restore_to_count(1);
        canvas.reset_matrix();
        canvas.clear(sk::Color::TRANSPARENT);
    }

    /// Render a recorded scene into an RGBA8 image (unpremultiplied).
    pub fn render_scene_into(
        &mut self,
        scene: &Scene,
        width: u16,
        height: u16,
        image: &mut RgbaImage,
    ) -> Result<(), Error> {
        scene.validate().map_err(Error::InvalidScene)?;
        self.resize(i32::from(width), i32::from(height));
        self.reset();
        let mut sink = SkCanvasSink::new_with_caches(
            self.surface.canvas(),
            Some(self.caches.image_cache()),
            self.caches.mask_cache(),
            self.caches.font_cache(),
        );
        sink.set_tolerance(self.tolerance);
        replay(scene, &mut sink);
        sink.finish()?;
        self.read_into_target(ImageBufferTarget::from_rgba_image(image))
    }

    /// Render a recorded scene and return an RGBA8 image (unpremultiplied).
    pub fn render_scene(
        &mut self,
        scene: &Scene,
        width: u16,
        height: u16,
    ) -> Result<RgbaImage, Error> {
        let mut image = RgbaImage::new(u32::from(width), u32::from(height));
        self.render_scene_into(scene, width, height, &mut image)?;
        Ok(image)
    }

    /// Render a native [`skia_safe::Picture`] into an RGBA8 image (unpremultiplied).
    pub fn render_picture_into(
        &mut self,
        picture: &sk::Picture,
        width: u16,
        height: u16,
        image: &mut RgbaImage,
    ) -> Result<(), Error> {
        self.resize(i32::from(width), i32::from(height));
        self.reset();
        self.surface.canvas().draw_picture(picture, None, None);
        self.read_into_target(ImageBufferTarget::from_rgba_image(image))
    }

    /// Render a native [`skia_safe::Picture`] and return an RGBA8 image (unpremultiplied).
    pub fn render_picture(
        &mut self,
        picture: &sk::Picture,
        width: u16,
        height: u16,
    ) -> Result<RgbaImage, Error> {
        let mut image = RgbaImage::new(u32::from(width), u32::from(height));
        self.render_picture_into(picture, width, height, &mut image)?;
        Ok(image)
    }

    fn read_into_target(&mut self, target: ImageBufferTarget<'_>) -> Result<(), Error> {
        read_surface_into_target(&mut self.surface, target)
    }
}

impl ImageRenderer for SkiaCpuRenderer {
    fn supported_image_formats(&self) -> Vec<ImageBufferFormat> {
        supported_image_formats()
    }

    fn render_source_into(
        &mut self,
        source: &mut dyn RenderSource,
        target: ImageBufferTarget<'_>,
    ) -> Result<(), ImageRendererError> {
        let (width, height) =
            Self::checked_size(target.width, target.height).map_err(map_image_renderer_error)?;
        source
            .validate()
            .map_err(Error::InvalidScene)
            .map_err(map_image_renderer_error)?;
        self.resize(width, height);
        self.reset();
        let mut sink = SkCanvasSink::new_with_caches(
            self.surface.canvas(),
            Some(self.caches.image_cache()),
            self.caches.mask_cache(),
            self.caches.font_cache(),
        );
        sink.set_tolerance(self.tolerance);
        source.paint_into(&mut sink);
        sink.finish().map_err(map_image_renderer_error)?;
        self.read_into_target(target)
            .map_err(map_image_renderer_error)
    }
}

#[cfg(feature = "gpu")]
fn encode_source_to_picture<S: RenderSource + ?Sized>(
    source: &mut S,
    width: u32,
    height: u32,
    tolerance: f64,
    image_cache: Rc<RefCell<ImageCache>>,
    font_cache: SkiaFontCache,
) -> Result<sk::Picture, Error> {
    source.validate().map_err(Error::InvalidScene)?;
    let bounds = kurbo::Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
    let mut sink = SkPictureRecorderSink::new_with_caches(bounds, Some(image_cache), font_cache);
    sink.set_tolerance(tolerance);
    source.paint_into(&mut sink);
    sink.finish_picture()
}

#[cfg(feature = "gpu")]
#[derive(Debug)]
struct SkiaGpuRendererState {
    backend: GaneshBackend,
    device: wgpu::Device,
    queue: wgpu::Queue,
    tolerance: f64,
    caches: SkiaCaches,
}

#[cfg(feature = "gpu")]
/// GPU Skia renderer that shares an app-owned `wgpu` device and queue and renders into
/// caller-owned textures.
#[derive(Debug)]
pub struct SkiaRenderer {
    state: SkiaGpuRendererState,
    scratch: Option<ScratchTexture>,
}

#[cfg(feature = "gpu")]
impl SkiaGpuRendererState {
    fn checked_texture_size(texture: &wgpu::Texture) -> Result<(u32, u32), Error> {
        if texture.dimension() != wgpu::TextureDimension::D2 {
            return Err(Error::Internal(
                "Skia GPU renderer only supports 2D textures",
            ));
        }
        if texture.sample_count() != 1 {
            return Err(Error::Internal(
                "Skia GPU renderer only supports single-sampled textures",
            ));
        }
        Ok((texture.width(), texture.height()))
    }

    fn new(
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
        config: SkiaConfig,
    ) -> Result<Self, Error> {
        config.cache_config.apply();
        config
            .caches
            .set_image_cache_total_bytes_limit(config.cache_config.image_cache_total_bytes_limit);
        config
            .caches
            .set_mask_cache_total_bytes_limit(config.cache_config.mask_cache_total_bytes_limit);
        let backend = GaneshBackend::from_wgpu(&adapter, &device, &queue)?;
        Ok(Self {
            backend,
            device,
            queue,
            tolerance: 0.1,
            caches: config.caches,
        })
    }

    fn render_picture_to_texture(
        &mut self,
        picture: &sk::Picture,
        texture: &wgpu::Texture,
    ) -> Result<(), Error> {
        let _ = Self::checked_texture_size(texture)?;
        let mut surface = self.backend.wrap_texture(texture)?;
        surface.canvas().clear(sk::Color::TRANSPARENT);
        surface.canvas().draw_picture(picture, None, None);
        self.backend.flush_surface(&mut surface);
        Ok(())
    }

    fn render_source_into_texture(
        &mut self,
        source: &mut dyn RenderSource,
        texture: &wgpu::Texture,
    ) -> Result<(), Error> {
        let _ = Self::checked_texture_size(texture)?;
        source.validate().map_err(Error::InvalidScene)?;
        let mut surface = self.backend.wrap_texture(texture)?;
        surface.canvas().clear(sk::Color::TRANSPARENT);
        let mut sink = SkCanvasSink::new_with_caches(
            surface.canvas(),
            Some(self.caches.image_cache()),
            self.caches.mask_cache(),
            self.caches.font_cache(),
        );
        sink.set_tolerance(self.tolerance);
        source.paint_into(&mut sink);
        sink.finish()?;
        self.backend.flush_surface(&mut surface);
        Ok(())
    }

    fn render_picture_to_texture_for_readback(
        &mut self,
        picture: &sk::Picture,
        texture: &wgpu::Texture,
    ) -> Result<(), Error> {
        let _ = Self::checked_texture_size(texture)?;
        let mut surface = self.backend.wrap_texture(texture)?;
        surface.canvas().clear(sk::Color::TRANSPARENT);
        surface.canvas().draw_picture(picture, None, None);
        self.backend.flush_surface_for_readback(&mut surface);
        Ok(())
    }
}

#[cfg(feature = "gpu")]
impl SkiaRenderer {
    /// Create a GPU renderer bound to an existing `wgpu` adapter, device, and queue.
    ///
    /// The adapter is used to select the active Ganesh interop backend at runtime. This matters
    /// on platforms like Windows where `wgpu` may run over either D3D12 or Vulkan.
    pub fn new(
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) -> Result<Self, Error> {
        Self::new_with_config(adapter, device, queue, SkiaConfig::new())
    }

    /// Create a GPU renderer using the provided shared caches and cache budgets.
    pub fn new_with_config(
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
        config: SkiaConfig,
    ) -> Result<Self, Error> {
        Ok(Self {
            state: SkiaGpuRendererState::new(adapter, device, queue, config)?,
            scratch: None,
        })
    }

    /// Set the tolerance used when converting shapes to paths during scene encoding.
    pub fn set_tolerance(&mut self, tolerance: f64) {
        if self.state.tolerance != tolerance {
            self.state.caches.mask_cache().borrow_mut().clear();
        }
        self.state.tolerance = tolerance;
    }

    /// Drop any realized mask artifacts cached by the renderer.
    pub fn clear_cached_masks(&mut self) {
        self.state.caches.mask_cache().borrow_mut().clear();
    }

    /// Drop any realized image resources cached by the renderer.
    pub fn clear_cached_images(&mut self) {
        self.state.caches.image_cache().borrow_mut().clear();
    }

    /// Drop all renderer-local caches, including shared font state.
    pub fn clear_caches(&mut self) {
        self.state.caches.clear();
    }

    /// Lower a semantic [`imaging::record::Scene`] into a native [`skia_safe::Picture`].
    pub fn encode_scene(
        &mut self,
        scene: &Scene,
        width: u32,
        height: u32,
    ) -> Result<sk::Picture, Error> {
        let mut source = scene;
        encode_source_to_picture(
            &mut source,
            width,
            height,
            self.state.tolerance,
            self.state.caches.image_cache(),
            self.state.caches.font_cache(),
        )
    }

    /// Render a native [`skia_safe::Picture`] into a caller-owned `wgpu::Texture`.
    pub fn render_picture_to_texture(
        &mut self,
        picture: &sk::Picture,
        texture: &wgpu::Texture,
    ) -> Result<(), Error> {
        self.state.render_picture_to_texture(picture, texture)
    }
}

fn supported_image_formats() -> Vec<ImageBufferFormat> {
    const CANDIDATE_FORMATS: &[ImageBufferFormat] = &[
        ImageBufferFormat::Rgba8Unorm,
        ImageBufferFormat::Rgba8UnormSrgb,
        ImageBufferFormat::Bgra8Unorm,
        ImageBufferFormat::Bgra8UnormSrgb,
        ImageBufferFormat::Rgb10a2Unorm,
        ImageBufferFormat::Rgba16Unorm,
        ImageBufferFormat::Rgba16Float,
    ];
    CANDIDATE_FORMATS
        .iter()
        .copied()
        .filter(|format| color_type_for_image_buffer_format(*format).is_ok())
        .collect()
}

#[cfg(feature = "gpu")]
impl TextureRenderer for SkiaRenderer {
    type TextureTarget = wgpu::Texture;
    type Texture = wgpu::Texture;

    fn supported_texture_formats(&self) -> Vec<wgpu::TextureFormat> {
        self.state.backend.supported_texture_formats()
    }

    fn render_source_into_texture(
        &mut self,
        source: &mut dyn RenderSource,
        target: Self::TextureTarget,
    ) -> Result<(), TextureRendererError> {
        self.state
            .render_source_into_texture(source, &target)
            .map_err(map_texture_renderer_error)
    }

    fn render_source_texture(
        &mut self,
        source: &mut dyn RenderSource,
        width: u32,
        height: u32,
    ) -> Result<Self::Texture, TextureRendererError> {
        let texture = ScratchTexture::new(
            &self.state.device,
            &self.state.queue,
            width,
            height,
            wgpu::TextureFormat::Rgba8Unorm,
            "imaging_skia gpu render target",
        )
        .texture()
        .clone();
        self.state
            .render_source_into_texture(source, &texture)
            .map_err(map_texture_renderer_error)?;
        Ok(texture)
    }
}

#[cfg(feature = "gpu")]
impl SkiaRenderer {
    fn scratch_texture(&mut self, width: u32, height: u32) -> wgpu::Texture {
        self.scratch_texture_for_format(width, height, wgpu::TextureFormat::Rgba8Unorm)
    }

    fn scratch_texture_for_format(
        &mut self,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> wgpu::Texture {
        if self
            .scratch
            .as_ref()
            .is_some_and(|scratch| scratch.format() != format)
        {
            self.scratch = None;
        }
        let scratch = self.scratch.get_or_insert_with(|| {
            ScratchTexture::new(
                &self.state.device,
                &self.state.queue,
                width,
                height,
                format,
                "imaging_skia gpu scratch target",
            )
        });
        scratch.resize(&self.state.device, width, height);
        scratch.texture().clone()
    }

    /// Render a native [`skia_safe::Picture`] into an RGBA8 image (unpremultiplied).
    pub fn render_picture_into(
        &mut self,
        picture: &sk::Picture,
        width: u32,
        height: u32,
        image: &mut RgbaImage,
    ) -> Result<(), Error> {
        let scratch = self.scratch_texture(width, height);
        self.state
            .render_picture_to_texture_for_readback(picture, &scratch)?;
        read_texture_into(
            &self.state.device,
            &self.state.queue,
            &scratch,
            width,
            height,
            image,
        )
        .map_err(|err| match err {
            ReadbackError::DevicePoll => Error::Internal("wgpu device poll failed"),
            ReadbackError::CallbackDropped => Error::Internal("wgpu readback callback dropped"),
            ReadbackError::BufferMap => Error::Internal("wgpu readback buffer map failed"),
        })
    }

    /// Render a native [`skia_safe::Picture`] and return an RGBA8 image (unpremultiplied).
    pub fn render_picture(
        &mut self,
        picture: &sk::Picture,
        width: u32,
        height: u32,
    ) -> Result<RgbaImage, Error> {
        let mut image = RgbaImage::new(width, height);
        self.render_picture_into(picture, width, height, &mut image)?;
        Ok(image)
    }
}

#[cfg(feature = "gpu")]
impl ImageRenderer for SkiaRenderer {
    fn supported_image_formats(&self) -> Vec<ImageBufferFormat> {
        supported_image_formats()
    }

    fn render_source_into(
        &mut self,
        source: &mut dyn RenderSource,
        target: ImageBufferTarget<'_>,
    ) -> Result<(), ImageRendererError> {
        let texture_format = wgpu_texture_format_for_image_buffer_format(target.format)
            .map_err(map_image_renderer_error)?;
        let picture = encode_source_to_picture(
            source,
            target.width,
            target.height,
            self.state.tolerance,
            self.state.caches.image_cache(),
            self.state.caches.font_cache(),
        )
        .map_err(map_image_renderer_error)?;
        let texture = self.scratch_texture_for_format(target.width, target.height, texture_format);
        let mut surface = self
            .state
            .backend
            .wrap_texture(&texture)
            .map_err(map_image_renderer_error)?;
        surface.canvas().clear(sk::Color::TRANSPARENT);
        surface.canvas().draw_picture(&picture, None, None);
        self.state.backend.flush_surface_for_readback(&mut surface);
        drop(surface);
        read_texture_into_target(
            &self.state.device,
            &self.state.queue,
            &texture,
            target.width,
            target.height,
            target.data,
            target.bytes_per_row,
        )
        .map_err(map_readback_image_error)
    }
}

fn map_image_renderer_error(error: Error) -> ImageRendererError {
    match error {
        Error::InvalidScene(error) => {
            ImageRendererError::Content(RenderContentError::InvalidScene(error))
        }
        Error::UnsupportedImageBrush => {
            ImageRendererError::Unsupported(RenderUnsupportedError::ImageBrush)
        }
        Error::UnsupportedFilter => ImageRendererError::Unsupported(RenderUnsupportedError::Filter),
        Error::UnsupportedGlyphTransform => {
            ImageRendererError::Unsupported(RenderUnsupportedError::Glyph)
        }
        Error::InvalidFontData => ImageRendererError::Content(RenderContentError::InvalidFontData),
        Error::InvalidGlyphId => ImageRendererError::Content(RenderContentError::InvalidGlyphId),
        #[cfg(feature = "gpu")]
        Error::UnsupportedGpuTextureFormat | Error::UnsupportedImageTargetFormat => {
            ImageRendererError::Target(ImageTargetError::UnsupportedTargetFormat)
        }
        Error::Internal("image target dimensions do not match renderer output") => {
            ImageRendererError::Target(ImageTargetError::InvalidTarget(
                "image target dimensions do not match renderer output",
            ))
        }
        Error::Internal("image target row stride is too small") => {
            ImageRendererError::Target(ImageTargetError::InvalidTarget(
                "image target row stride is smaller than the rendered width",
            ))
        }
        Error::Internal("image target buffer is too small") => {
            ImageRendererError::Target(ImageTargetError::InvalidTargetBuffer)
        }
        Error::Internal("render width too large" | "render height too large") => {
            ImageRendererError::Target(ImageTargetError::DimensionsTooLarge)
        }
        other => ImageRendererError::backend(other),
    }
}

#[cfg(feature = "gpu")]
fn map_readback_image_error(error: ReadbackError) -> ImageRendererError {
    use imaging::render::GpuReadbackError;
    match error {
        ReadbackError::DevicePoll => ImageRendererError::Readback(GpuReadbackError::DevicePoll),
        ReadbackError::CallbackDropped => {
            ImageRendererError::Readback(GpuReadbackError::CallbackDropped)
        }
        ReadbackError::BufferMap => ImageRendererError::Readback(GpuReadbackError::BufferMap),
    }
}

#[cfg(feature = "gpu")]
fn map_texture_renderer_error(error: Error) -> TextureRendererError {
    match error {
        Error::InvalidScene(error) => {
            TextureRendererError::Content(RenderContentError::InvalidScene(error))
        }
        Error::UnsupportedImageBrush => {
            TextureRendererError::Unsupported(RenderUnsupportedError::ImageBrush)
        }
        Error::UnsupportedFilter => {
            TextureRendererError::Unsupported(RenderUnsupportedError::Filter)
        }
        Error::UnsupportedGlyphTransform => {
            TextureRendererError::Unsupported(RenderUnsupportedError::Glyph)
        }
        Error::Internal("render width too large" | "render height too large") => {
            TextureRendererError::Target(TextureTargetError::DimensionsTooLarge)
        }
        Error::UnsupportedGpuTextureFormat => {
            TextureRendererError::Target(TextureTargetError::UnsupportedTextureFormat)
        }
        Error::UnsupportedGpuBackend => {
            TextureRendererError::Target(TextureTargetError::UnsupportedGpuBackend)
        }
        Error::CreateGpuContext(message) => {
            TextureRendererError::Target(TextureTargetError::CreateGpuContext(message))
        }
        Error::CreateGpuSurface => {
            TextureRendererError::Target(TextureTargetError::CreateGpuSurface)
        }
        other => TextureRendererError::backend(other),
    }
}

#[cfg(feature = "gpu")]
fn initialize_texture_for_wgpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) {
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("imaging_skia texture init"),
    });
    let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("imaging_skia texture init"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
            depth_slice: None,
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    drop(_pass);
    queue.submit([encoder.finish()]);
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

fn image_target_width_bytes(width: u32, format: ImageBufferFormat) -> usize {
    usize::try_from(width)
        .expect("image width should fit in usize")
        .checked_mul(format.bytes_per_pixel())
        .expect("image row bytes should fit in usize")
}

fn color_type_for_image_buffer_format(format: ImageBufferFormat) -> Result<sk::ColorType, Error> {
    match format {
        ImageBufferFormat::Rgba8Unorm => Ok(sk::ColorType::RGBA8888),
        ImageBufferFormat::Rgba8UnormSrgb => Ok(sk::ColorType::SRGBA8888),
        ImageBufferFormat::Bgra8Unorm | ImageBufferFormat::Bgra8UnormSrgb => {
            Ok(sk::ColorType::BGRA8888)
        }
        ImageBufferFormat::Rgb10a2Unorm => Ok(sk::ColorType::RGBA1010102),
        ImageBufferFormat::Rgba16Unorm => Ok(sk::ColorType::R16G16B16A16UNorm),
        ImageBufferFormat::Rgba16Float => Ok(sk::ColorType::RGBAF16),
    }
}

fn color_space_for_image_buffer_format(format: ImageBufferFormat) -> Option<sk::ColorSpace> {
    match format {
        ImageBufferFormat::Rgba8UnormSrgb | ImageBufferFormat::Bgra8UnormSrgb => {
            Some(sk::ColorSpace::new_srgb())
        }
        _ => None,
    }
}

#[cfg(feature = "gpu")]
fn wgpu_texture_format_for_image_buffer_format(
    format: ImageBufferFormat,
) -> Result<wgpu::TextureFormat, Error> {
    match format {
        ImageBufferFormat::Rgba8Unorm => Ok(wgpu::TextureFormat::Rgba8Unorm),
        ImageBufferFormat::Rgba8UnormSrgb => Ok(wgpu::TextureFormat::Rgba8UnormSrgb),
        ImageBufferFormat::Bgra8Unorm => Ok(wgpu::TextureFormat::Bgra8Unorm),
        ImageBufferFormat::Bgra8UnormSrgb => Ok(wgpu::TextureFormat::Bgra8UnormSrgb),
        ImageBufferFormat::Rgb10a2Unorm => Ok(wgpu::TextureFormat::Rgb10a2Unorm),
        ImageBufferFormat::Rgba16Unorm => Ok(wgpu::TextureFormat::Rgba16Unorm),
        ImageBufferFormat::Rgba16Float => Ok(wgpu::TextureFormat::Rgba16Float),
    }
}

fn read_surface_into_target(
    surface: &mut sk::Surface,
    target: ImageBufferTarget<'_>,
) -> Result<(), Error> {
    let snapshot = surface.image_snapshot();
    let width = u32::try_from(snapshot.width()).expect("positive skia width should fit in u32");
    let height = u32::try_from(snapshot.height()).expect("positive skia height should fit in u32");
    let width_bytes = image_target_width_bytes(width, target.format);
    if target.width != width || target.height != height {
        return Err(Error::Internal(
            "image target dimensions do not match renderer output",
        ));
    }
    if target.bytes_per_row < width_bytes {
        return Err(Error::Internal("image target row stride is too small"));
    }
    let required_len = target
        .bytes_per_row
        .checked_mul(usize::try_from(height).expect("image height should fit in usize"))
        .expect("image target byte length should fit in usize");
    if target.data.len() < required_len {
        return Err(Error::Internal("image target buffer is too small"));
    }
    let info = sk::ImageInfo::new(
        (
            i32::try_from(width).expect("image width should fit in i32"),
            i32::try_from(height).expect("image height should fit in i32"),
        ),
        color_type_for_image_buffer_format(target.format)?,
        sk::AlphaType::Unpremul,
        color_space_for_image_buffer_format(target.format),
    );
    if snapshot.read_pixels(
        &info,
        target.data,
        target.bytes_per_row,
        (0, 0),
        sk::image::CachingHint::Disallow,
    ) {
        Ok(())
    } else {
        Err(Error::Internal("read_pixels failed"))
    }
}

#[cfg(feature = "gpu")]
fn color_type_for_wgpu_texture_format(
    texture_format: wgpu::TextureFormat,
) -> Result<sk::ColorType, Error> {
    match texture_format {
        wgpu::TextureFormat::Rgba8Unorm => Ok(sk::ColorType::RGBA8888),
        wgpu::TextureFormat::Rgba8UnormSrgb => Ok(sk::ColorType::SRGBA8888),
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => {
            Ok(sk::ColorType::BGRA8888)
        }
        wgpu::TextureFormat::Rgb10a2Unorm => Ok(sk::ColorType::RGBA1010102),
        wgpu::TextureFormat::Rgba16Unorm => Ok(sk::ColorType::R16G16B16A16UNorm),
        wgpu::TextureFormat::Rgba16Float => Ok(sk::ColorType::RGBAF16),
        _ => Err(Error::UnsupportedGpuTextureFormat),
    }
}

#[cfg(feature = "gpu")]
fn color_space_for_wgpu_texture_format(
    texture_format: wgpu::TextureFormat,
) -> Option<sk::ColorSpace> {
    match texture_format {
        wgpu::TextureFormat::Rgba8UnormSrgb | wgpu::TextureFormat::Bgra8UnormSrgb => {
            Some(sk::ColorSpace::new_srgb())
        }
        _ => None,
    }
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

fn denormalize_variation_coord(
    normalized_coord: imaging::NormalizedCoord,
    axis: &sk::font_parameters::VariationAxis,
) -> f32 {
    let normalized = (f32::from(normalized_coord) / 16_384.0).clamp(-1.0, 1.0);
    if normalized <= 0.0 {
        axis.def + (axis.def - axis.min) * normalized
    } else {
        axis.def + (axis.max - axis.def) * normalized
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct BaseTypefaceKey {
    font_data_id: u64,
    font_index: u32,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TypefaceKey {
    base: BaseTypefaceKey,
    normalized_coords: Vec<imaging::NormalizedCoord>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct FontKey {
    typeface: TypefaceKey,
    font_size_bits: u32,
    hint: bool,
}

#[derive(Debug)]
struct FontCache {
    font_mgr: sk::FontMgr,
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    extracted_font_data: HashMap<BaseTypefaceKey, peniko::FontData>,
    base_typefaces: HashMap<BaseTypefaceKey, sk::Typeface>,
    typefaces: HashMap<TypefaceKey, sk::Typeface>,
    fonts: HashMap<FontKey, sk::Font>,
}

impl FontCache {
    fn new() -> Self {
        Self {
            font_mgr: sk::FontMgr::default(),
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            extracted_font_data: HashMap::new(),
            base_typefaces: HashMap::new(),
            typefaces: HashMap::new(),
            fonts: HashMap::new(),
        }
    }

    fn clear(&mut self) {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        self.extracted_font_data.clear();
        self.base_typefaces.clear();
        self.typefaces.clear();
        self.fonts.clear();
    }

    fn font_from_glyph_run(&mut self, glyph_run: &GlyphRunRef<'_>) -> Option<sk::Font> {
        let typeface_key = TypefaceKey {
            base: BaseTypefaceKey {
                font_data_id: glyph_run.font.data.id(),
                font_index: glyph_run.font.index,
            },
            normalized_coords: glyph_run.normalized_coords.to_vec(),
        };
        let font_key = FontKey {
            typeface: typeface_key.clone(),
            font_size_bits: glyph_run.font_size.to_bits(),
            hint: glyph_run.hint,
        };

        let mut font = if let Some(font) = self.fonts.get(&font_key) {
            font.clone()
        } else {
            let typeface = self.typeface_for_key(&typeface_key, glyph_run.font)?;
            let mut font = sk::Font::from_typeface(typeface, glyph_run.font_size);
            font.set_hinting(if glyph_run.hint {
                sk::FontHinting::Slight
            } else {
                sk::FontHinting::None
            });
            self.fonts.insert(font_key, font.clone());
            font
        };

        apply_glyph_transform(&mut font, glyph_run.glyph_transform, glyph_run.font_size)?;
        Some(font)
    }

    fn typeface_for_key(
        &mut self,
        key: &TypefaceKey,
        font: &peniko::FontData,
    ) -> Option<sk::Typeface> {
        if key.normalized_coords.is_empty() {
            return self.base_typeface(&key.base, font);
        }
        if let Some(typeface) = self.typefaces.get(key) {
            return Some(typeface.clone());
        }

        let typeface = self.base_typeface(&key.base, font)?;
        let axes = typeface.variation_design_parameters().unwrap_or_default();
        if axes.is_empty() {
            self.typefaces.insert(key.clone(), typeface.clone());
            return Some(typeface);
        }

        let coordinates: Vec<sk::font_arguments::variation_position::Coordinate> = axes
            .iter()
            .zip(key.normalized_coords.iter())
            .map(
                |(axis, &normalized_coord)| sk::font_arguments::variation_position::Coordinate {
                    axis: axis.tag,
                    value: denormalize_variation_coord(normalized_coord, axis),
                },
            )
            .filter(|coord| coord.value != 0.0)
            .collect();

        if coordinates.is_empty() {
            self.typefaces.insert(key.clone(), typeface.clone());
            return Some(typeface);
        }

        let arguments = sk::FontArguments::new().set_variation_design_position(
            sk::font_arguments::VariationPosition {
                coordinates: &coordinates,
            },
        );
        let typeface = typeface.clone_with_arguments(&arguments)?;
        self.typefaces.insert(key.clone(), typeface.clone());
        Some(typeface)
    }

    fn base_typeface(
        &mut self,
        key: &BaseTypefaceKey,
        font: &peniko::FontData,
    ) -> Option<sk::Typeface> {
        if let Some(typeface) = self.base_typefaces.get(key) {
            return Some(typeface.clone());
        }

        let extracted_font = extracted_font_data(self, key, font)?;
        let font_bytes = extracted_font.data.as_ref();
        let font_index = extracted_font.index as usize;
        let typeface = self.font_mgr.new_from_data(font_bytes, font_index)?;
        self.base_typefaces.insert(key.clone(), typeface.clone());
        Some(typeface)
    }

    #[cfg(test)]
    fn counts(&self) -> (usize, usize, usize) {
        (
            self.base_typefaces.len(),
            self.typefaces.len(),
            self.fonts.len(),
        )
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn extracted_font_data(
    cache: &mut FontCache,
    key: &BaseTypefaceKey,
    font: &peniko::FontData,
) -> Option<peniko::FontData> {
    use peniko::Blob;
    use std::sync::Arc;

    if let Some(collection) = oaty::Collection::new(font.data.data()) {
        cache
            .extracted_font_data
            .entry(key.clone())
            .or_insert_with(|| {
                let data = collection
                    .get_font(font.index)
                    .and_then(|font| font.copy_data())
                    .unwrap_or_default();
                peniko::FontData::new(Blob::new(Arc::new(data)), 0)
            });
        if let Some(extracted) = cache.extracted_font_data.get(key) {
            return Some(extracted.clone());
        }
    }

    Some(font.clone())
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
fn extracted_font_data(
    _: &mut FontCache,
    _: &BaseTypefaceKey,
    font: &peniko::FontData,
) -> Option<peniko::FontData> {
    Some(font.clone())
}

fn skia_font_from_glyph_run(
    font_cache: Option<&SkiaFontCache>,
    glyph_run: &GlyphRunRef<'_>,
) -> Option<sk::Font> {
    match font_cache {
        Some(font_cache) => font_cache.borrow_mut().font_from_glyph_run(glyph_run),
        None => FontCache::new().font_from_glyph_run(glyph_run),
    }
}

fn apply_glyph_transform(
    font: &mut sk::Font,
    glyph_transform: Option<Affine>,
    font_size: f32,
) -> Option<()> {
    let Some(transform) = glyph_transform else {
        return Some(());
    };

    let [a, b, c, d, e, f] = transform.as_coeffs();
    if b != 0.0 || e != 0.0 || f != 0.0 || d <= 0.0 {
        return None;
    }

    font.set_size(f64_to_f32(font_size as f64 * d));
    font.set_scale_x(f64_to_f32(a / d));
    font.set_skew_x(f64_to_f32(c / d));
    Some(())
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
    let mut path = sk::PathBuilder::new();
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
    Some(path.detach())
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

fn brush_to_paint(
    brush: BrushRef<'_>,
    opacity: f32,
    paint_xf: Affine,
    image_cache: Option<&Rc<RefCell<ImageCache>>>,
) -> Option<sk::Paint> {
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
                let color = s
                    .color
                    .to_alpha_color::<peniko::color::Srgb>()
                    .multiply_alpha(alpha_scale);
                colors.push(sk::Color4f::new(
                    color.components[0],
                    color.components[1],
                    color.components[2],
                    color.components[3],
                ));
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
                    if let Some(shader) = sk::gradient_shader::linear_with_interpolation(
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

                    if let Some(shader) = sk::gradient_shader::two_point_conical_with_interpolation(
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
                    if let Some(shader) = sk::gradient_shader::sweep_with_interpolation(
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
            let image = skia_image_from_peniko(image_brush.image, image_cache)?;
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

fn skia_image_from_peniko(
    image: &ImageData,
    image_cache: Option<&Rc<RefCell<ImageCache>>>,
) -> Option<sk::Image> {
    match image_cache {
        Some(image_cache) => image_cache.borrow_mut().get_or_create(image),
        None => make_skia_image_from_peniko(image),
    }
}

fn make_skia_image_from_peniko(image: &ImageData) -> Option<sk::Image> {
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
    use imaging::{
        GroupRef, MaskMode, Painter,
        record::Glyph,
        render::{ImageBufferTarget, ImageTargetError},
    };
    use kurbo::Rect;
    use peniko::{
        Blob, Brush, Color, Fill, FontData, ImageAlphaType, ImageData, ImageFormat, Style,
    };
    use std::sync::{Arc, OnceLock};
    #[cfg(feature = "gpu")]
    use std::{
        future::Future,
        pin::pin,
        task::{Context, Poll, Waker},
    };

    const TEST_FONT_BYTES: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../test_assets/fonts/NotoSans-Regular.ttf"
    ));

    fn test_font() -> FontData {
        static FONT: OnceLock<FontData> = OnceLock::new();
        FONT.get_or_init(|| FontData::new(Blob::new(Arc::new(TEST_FONT_BYTES)), 0))
            .clone()
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

    fn test_image() -> ImageData {
        ImageData {
            data: Blob::new(Arc::new([
                0xff, 0x00, 0x00, 0xff, 0x00, 0xff, 0x00, 0xff, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff,
                0x00, 0xff,
            ])),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: 2,
            height: 2,
        }
    }

    fn image_scene() -> Scene {
        let brush = Brush::Image(peniko::ImageBrush::new(test_image()));
        let mut scene = Scene::new();
        {
            let mut painter = Painter::new(&mut scene);
            painter.fill(Rect::new(0.0, 0.0, 32.0, 32.0), &brush).draw();
        }
        scene
    }

    #[test]
    fn render_picture_renders_native_picture() {
        let mut sink = SkPictureRecorderSink::new(Rect::new(0.0, 0.0, 32.0, 32.0));
        let paint = Brush::Solid(Color::from_rgb8(0x22, 0x66, 0xaa));
        {
            let mut painter = Painter::new(&mut sink);
            painter.fill_rect(Rect::new(0.0, 0.0, 32.0, 32.0), &paint);
        }

        let picture = sink.finish_picture().unwrap();
        let mut renderer = SkiaCpuRenderer::new();
        let image = renderer.render_picture(&picture, 32, 32).unwrap();

        assert_eq!(image.data.len(), 32 * 32 * 4);
        assert_eq!(&image.data[..4], &[0x22, 0x66, 0xaa, 0xff]);
    }

    #[test]
    fn render_scene_reuses_cached_masks_for_identical_scenes() {
        let scene = masked_scene(MaskMode::Alpha);
        let mut renderer = SkiaCpuRenderer::new();

        renderer.render_scene(&scene, 64, 64).unwrap();
        assert_eq!(renderer.caches.mask_cache().borrow().len(), 1);

        renderer.render_scene(&scene, 64, 64).unwrap();
        assert_eq!(renderer.caches.mask_cache().borrow().len(), 1);
    }

    #[test]
    fn clear_cached_masks_drops_realized_masks() {
        let scene = masked_scene(MaskMode::Luminance);
        let mut renderer = SkiaCpuRenderer::new();

        renderer.render_scene(&scene, 64, 64).unwrap();
        assert_eq!(renderer.caches.mask_cache().borrow().len(), 1);

        renderer.clear_cached_masks();
        assert_eq!(renderer.caches.mask_cache().borrow().len(), 0);

        renderer.render_scene(&scene, 64, 64).unwrap();
        assert_eq!(renderer.caches.mask_cache().borrow().len(), 1);
    }

    #[test]
    fn render_scene_reuses_cached_images_for_identical_scenes() {
        let scene = image_scene();
        let mut renderer = SkiaCpuRenderer::new();

        renderer.render_scene(&scene, 32, 32).unwrap();
        assert_eq!(renderer.caches.image_cache().borrow().len(), 1);

        renderer.render_scene(&scene, 32, 32).unwrap();
        assert_eq!(renderer.caches.image_cache().borrow().len(), 1);
    }

    #[test]
    fn clear_cached_images_drops_realized_images() {
        let scene = image_scene();
        let mut renderer = SkiaCpuRenderer::new();

        renderer.render_scene(&scene, 32, 32).unwrap();
        assert_eq!(renderer.caches.image_cache().borrow().len(), 1);

        renderer.clear_cached_images();
        assert_eq!(renderer.caches.image_cache().borrow().len(), 0);
    }

    #[test]
    fn changing_tolerance_clears_cached_masks() {
        let scene = masked_scene(MaskMode::Alpha);
        let mut renderer = SkiaCpuRenderer::new();

        renderer.render_scene(&scene, 64, 64).unwrap();
        assert_eq!(renderer.caches.mask_cache().borrow().len(), 1);

        renderer.set_tolerance(0.25);
        assert_eq!(renderer.caches.mask_cache().borrow().len(), 0);
    }

    #[test]
    fn config_can_disable_realized_mask_retention() {
        let scene = masked_scene(MaskMode::Alpha);
        let config = SkiaConfig::new()
            .with_cache_config(SkiaCacheConfig::new().with_mask_cache_total_bytes_limit(0));
        let mut renderer = SkiaCpuRenderer::new_with_config(config);

        renderer.render_scene(&scene, 64, 64).unwrap();
        assert_eq!(renderer.caches.mask_cache().borrow().len(), 0);
    }

    #[test]
    fn config_can_disable_realized_image_retention() {
        let scene = image_scene();
        let config = SkiaConfig::new()
            .with_cache_config(SkiaCacheConfig::new().with_image_cache_total_bytes_limit(0));
        let mut renderer = SkiaCpuRenderer::new_with_config(config);

        renderer.render_scene(&scene, 32, 32).unwrap();
        assert_eq!(renderer.caches.image_cache().borrow().len(), 0);
    }

    #[test]
    fn render_scene_renders_image() {
        let mut renderer = SkiaCpuRenderer::new();
        let mut scene = Scene::new();
        {
            let mut painter = Painter::new(&mut scene);
            painter
                .fill(
                    Rect::new(0.0, 0.0, 64.0, 64.0),
                    Color::from_rgb8(0x2a, 0x6f, 0xdb),
                )
                .draw();
        }
        let image = renderer.render_scene(&scene, 64, 64).unwrap();
        assert_eq!(image.width, 64);
        assert_eq!(image.height, 64);
    }

    #[test]
    fn render_source_renders_image() {
        let mut renderer = SkiaCpuRenderer::new();
        let mut scene = Scene::new();
        {
            let mut painter = Painter::new(&mut scene);
            painter
                .fill(
                    Rect::new(0.0, 0.0, 48.0, 48.0),
                    Color::from_rgb8(0x2a, 0x6f, 0xdb),
                )
                .draw();
        }

        let mut source = &scene;
        let image = ImageRenderer::render_source(&mut renderer, &mut source, 48, 48).unwrap();
        assert_eq!(image.width, 48);
        assert_eq!(image.height, 48);
    }

    #[test]
    fn cpu_renderer_reports_supported_image_formats() {
        let renderer = SkiaCpuRenderer::new();
        assert_eq!(
            renderer.supported_image_formats(),
            supported_image_formats()
        );
    }

    #[test]
    fn cpu_renderer_renders_into_bgra8_target() {
        let mut renderer = SkiaCpuRenderer::new();
        let mut scene = Scene::new();
        {
            let mut painter = Painter::new(&mut scene);
            painter
                .fill(
                    Rect::new(0.0, 0.0, 4.0, 4.0),
                    Color::from_rgb8(0x2a, 0x6f, 0xdb),
                )
                .draw();
        }

        let mut data = vec![0; 4 * 4 * 4];
        let mut source = &scene;
        renderer
            .render_source_into(
                &mut source,
                ImageBufferTarget {
                    data: &mut data,
                    width: 4,
                    height: 4,
                    bytes_per_row: 16,
                    format: ImageBufferFormat::Bgra8Unorm,
                },
            )
            .unwrap();

        assert_eq!(&data[..4], &[0xdb, 0x6f, 0x2a, 0xff]);
    }

    #[test]
    fn cpu_renderer_rejects_short_row_stride_as_target_error() {
        let mut renderer = SkiaCpuRenderer::new();
        let scene = image_scene();
        let mut data = vec![0; 4 * 4 * 4];
        let mut source = &scene;

        let error = ImageRenderer::render_source_into(
            &mut renderer,
            &mut source,
            ImageBufferTarget {
                data: &mut data,
                width: 4,
                height: 4,
                bytes_per_row: 12,
                format: ImageBufferFormat::Rgba8Unorm,
            },
        )
        .unwrap_err();

        assert!(matches!(
            error,
            ImageRendererError::Target(ImageTargetError::InvalidTarget(
                "image target row stride is smaller than the rendered width",
            ))
        ));
    }

    #[test]
    fn cpu_renderer_rejects_short_buffer_as_target_error() {
        let mut renderer = SkiaCpuRenderer::new();
        let scene = image_scene();
        let mut data = vec![0; 15];
        let mut source = &scene;

        let error = ImageRenderer::render_source_into(
            &mut renderer,
            &mut source,
            ImageBufferTarget {
                data: &mut data,
                width: 4,
                height: 4,
                bytes_per_row: 16,
                format: ImageBufferFormat::Rgba8Unorm,
            },
        )
        .unwrap_err();

        assert!(matches!(
            error,
            ImageRendererError::Target(ImageTargetError::InvalidTargetBuffer)
        ));
    }

    #[test]
    fn normalized_coords_on_non_variable_font_render_and_cache() {
        let font = test_font();
        let fill_style = Style::Fill(Fill::NonZero);
        let glyphs = [Glyph {
            id: 0,
            x: 8.0,
            y: 24.0,
        }];
        let normalized_coords = [2048_i16];

        let mut scene = Scene::new();
        {
            let mut painter = Painter::new(&mut scene);
            painter
                .glyphs(&font, &Brush::Solid(Color::from_rgb8(0x22, 0x66, 0xaa)))
                .font_size(18.0)
                .normalized_coords(&normalized_coords)
                .draw(&fill_style, glyphs);
        }

        let caches = SkiaCaches::new().with_font_cache(SkiaFontCache::new());
        let config = SkiaConfig::new().with_caches(caches.clone());
        let mut renderer = SkiaCpuRenderer::new_with_config(config.clone());
        renderer.render_scene(&scene, 48, 48).unwrap();
        let counts = caches.font_cache().counts();
        assert_eq!(counts, (1, 1, 1));

        let mut second_renderer = SkiaCpuRenderer::new_with_config(config);
        second_renderer.render_scene(&scene, 48, 48).unwrap();
        assert_eq!(caches.font_cache().counts(), counts);
    }

    #[cfg(feature = "gpu")]
    fn block_on<F: Future>(future: F) -> F::Output {
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut future = pin!(future);
        loop {
            match future.as_mut().poll(&mut cx) {
                Poll::Ready(value) => return value,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    #[cfg(feature = "gpu")]
    fn try_init_gpu_renderer() -> Option<SkiaRenderer> {
        let instance = wgpu::Instance::default();
        let adapter =
            block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
        let desc = wgpu::DeviceDescriptor::default();
        let (device, queue) = block_on(adapter.request_device(&desc)).ok()?;
        SkiaRenderer::new(adapter, device, queue).ok()
    }

    #[cfg(feature = "gpu")]
    fn try_init_gpu_renderer_with_device() -> Option<(SkiaRenderer, wgpu::Device)> {
        let instance = wgpu::Instance::default();
        let adapter =
            block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
        let desc = wgpu::DeviceDescriptor::default();
        let (device, queue) = block_on(adapter.request_device(&desc)).ok()?;
        let renderer = SkiaRenderer::new(adapter, device.clone(), queue).ok()?;
        Some((renderer, device))
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn gpu_renderer_reports_supported_texture_formats() {
        let Some((renderer, _device)) = try_init_gpu_renderer_with_device() else {
            return;
        };
        let expected: Vec<_> = GaneshBackend::CANDIDATE_TEXTURE_FORMATS
            .iter()
            .copied()
            .filter(|format| {
                renderer
                    .state
                    .backend
                    .can_wrap_texture_format(*format)
                    .is_ok()
            })
            .collect();
        assert_eq!(renderer.supported_texture_formats(), expected);
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn gpu_renderer_reports_supported_image_formats() {
        let Some(renderer) = try_init_gpu_renderer() else {
            return;
        };
        assert_eq!(
            renderer.supported_image_formats(),
            supported_image_formats()
        );
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn gpu_renderer_renders_picture_to_image() {
        let Some(mut renderer) = try_init_gpu_renderer() else {
            return;
        };

        let mut sink = SkPictureRecorderSink::new(Rect::new(0.0, 0.0, 16.0, 16.0));
        {
            let mut painter = Painter::new(&mut sink);
            painter.fill_rect(
                Rect::new(0.0, 0.0, 16.0, 16.0),
                &Brush::Solid(Color::from_rgb8(0x11, 0x22, 0x33)),
            );
        }

        let picture = sink.finish_picture().unwrap();
        let image = renderer.render_picture(&picture, 16, 16).unwrap();

        assert_eq!(image.width, 16);
        assert_eq!(image.height, 16);
        assert_eq!(&image.data[..4], &[0x11, 0x22, 0x33, 0xff]);
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn gpu_renderer_renders_source_to_texture() {
        let Some((mut renderer, device)) = try_init_gpu_renderer_with_device() else {
            return;
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("imaging_skia gpu target"),
            size: wgpu::Extent3d {
                width: 24,
                height: 24,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let mut scene = Scene::new();
        {
            let mut painter = Painter::new(&mut scene);
            painter.fill_rect(
                Rect::new(0.0, 0.0, 24.0, 24.0),
                &Brush::Solid(Color::from_rgb8(0x2a, 0x6f, 0xdb)),
            );
        }

        let mut source = &scene;
        TextureRenderer::render_source_into_texture(&mut renderer, &mut source, texture.clone())
            .unwrap();
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn gpu_renderer_render_source_texture_returns_independent_texture() {
        let Some((mut renderer, device)) = try_init_gpu_renderer_with_device() else {
            return;
        };

        let mut first_scene = Scene::new();
        {
            let mut painter = Painter::new(&mut first_scene);
            painter.fill_rect(
                Rect::new(0.0, 0.0, 8.0, 8.0),
                &Brush::Solid(Color::from_rgb8(0xff, 0x00, 0x00)),
            );
        }

        let mut second_scene = Scene::new();
        {
            let mut painter = Painter::new(&mut second_scene);
            painter.fill_rect(
                Rect::new(0.0, 0.0, 8.0, 8.0),
                &Brush::Solid(Color::from_rgb8(0x00, 0xff, 0x00)),
            );
        }

        let mut first_source = &first_scene;
        let first_texture =
            TextureRenderer::render_source_texture(&mut renderer, &mut first_source, 8, 8).unwrap();

        let mut second_source = &second_scene;
        let _second_texture =
            TextureRenderer::render_source_texture(&mut renderer, &mut second_source, 8, 8)
                .unwrap();

        let mut image = RgbaImage::new(8, 8);
        read_texture_into(
            &device,
            &renderer.state.queue,
            &first_texture,
            8,
            8,
            &mut image,
        )
        .unwrap();
        assert_eq!(&image.data[..4], &[0xff, 0x00, 0x00, 0xff]);
    }
}
