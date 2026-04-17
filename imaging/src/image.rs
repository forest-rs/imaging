// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shared image and brush types for `imaging`.

use alloc::sync::{Arc, Weak};
use core::{
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicU64, Ordering},
};
use kurbo::Rect;

use crate::record;

static NEXT_SCENE_IMAGE_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_SCENE_PICTURE_ID: AtomicU64 = AtomicU64::new(1);

/// Image payload accepted by `imaging` brushes.
#[derive(Clone, Debug, PartialEq)]
pub enum Image {
    /// Raster image data.
    Raster(peniko::ImageData),
    /// Retained scene content with an explicit natural size.
    Scene(SceneImage),
}

impl Image {
    /// Return the natural width of the image in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        match self {
            Self::Raster(image) => image.width,
            Self::Scene(scene) => scene.width(),
        }
    }

    /// Return the natural height of the image in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        match self {
            Self::Raster(image) => image.height,
            Self::Scene(scene) => scene.height(),
        }
    }

    /// Borrow this image payload.
    #[must_use]
    pub fn as_ref(&self) -> ImageRef<'_> {
        match self {
            Self::Raster(image) => ImageRef::Raster(image),
            Self::Scene(scene) => ImageRef::Scene(scene),
        }
    }
}

impl From<peniko::ImageData> for Image {
    fn from(value: peniko::ImageData) -> Self {
        Self::Raster(value)
    }
}

impl From<SceneImage> for Image {
    fn from(value: SceneImage) -> Self {
        Self::Scene(value)
    }
}

/// Borrowed image payload.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum ImageRef<'a> {
    /// Borrowed raster image data.
    Raster(&'a peniko::ImageData),
    /// Borrowed retained scene image.
    Scene(&'a SceneImage),
}

impl ImageRef<'_> {
    /// Return the natural width of the image in pixels.
    #[must_use]
    pub fn width(self) -> u32 {
        match self {
            Self::Raster(image) => image.width,
            Self::Scene(scene) => scene.width(),
        }
    }

    /// Return the natural height of the image in pixels.
    #[must_use]
    pub fn height(self) -> u32 {
        match self {
            Self::Raster(image) => image.height,
            Self::Scene(scene) => scene.height(),
        }
    }

    /// Convert to an owned image payload.
    #[must_use]
    pub fn to_owned(self) -> Image {
        match self {
            Self::Raster(image) => Image::Raster(image.clone()),
            Self::Scene(scene) => Image::Scene(scene.clone()),
        }
    }
}

impl<'a> From<&'a Image> for ImageRef<'a> {
    fn from(value: &'a Image) -> Self {
        value.as_ref()
    }
}

impl<'a> From<&'a peniko::ImageData> for ImageRef<'a> {
    fn from(value: &'a peniko::ImageData) -> Self {
        Self::Raster(value)
    }
}

impl<'a> From<&'a SceneImage> for ImageRef<'a> {
    fn from(value: &'a SceneImage) -> Self {
        Self::Scene(value)
    }
}

/// A retained scene recording.
#[derive(Clone, Debug, PartialEq)]
pub struct ScenePicture(Arc<ScenePictureInner>);

#[derive(Debug, PartialEq)]
struct ScenePictureInner {
    /// Stable identity for cache lookups.
    id: u64,
    /// Scene content.
    scene: record::Scene,
    /// Conservative cull bounds for the retained recording.
    bounds: Rect,
}

/// Weak retained handle to a scene picture.
#[derive(Clone, Debug)]
pub struct ScenePictureWeak(Weak<ScenePictureInner>);

impl ScenePictureWeak {
    /// Upgrade the weak handle if the picture is still alive.
    #[must_use]
    pub fn upgrade(&self) -> Option<ScenePicture> {
        self.0.upgrade().map(ScenePicture)
    }
}

impl ScenePicture {
    /// Create a retained scene picture.
    #[must_use]
    pub fn new(scene: record::Scene, bounds: Rect) -> Self {
        Self(Arc::new(ScenePictureInner {
            id: NEXT_SCENE_PICTURE_ID.fetch_add(1, Ordering::Relaxed),
            scene,
            bounds,
        }))
    }

    /// Return the stable identity of this scene picture.
    #[must_use]
    pub fn id(&self) -> u64 {
        self.0.id
    }

    /// Borrow the retained scene.
    #[must_use]
    pub fn scene(&self) -> &record::Scene {
        &self.0.scene
    }

    /// Return the conservative cull bounds for this retained scene picture.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        self.0.bounds
    }

    /// Downgrade this retained scene picture to a weak handle for cache entries.
    #[must_use]
    pub fn downgrade(&self) -> ScenePictureWeak {
        ScenePictureWeak(Arc::downgrade(&self.0))
    }
}

/// A retained scene used as an image brush source.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneImage(Arc<SceneImageInner>);

#[derive(Debug, PartialEq)]
struct SceneImageInner {
    /// Stable identity for cache lookups.
    id: u64,
    /// Retained scene picture.
    picture: ScenePicture,
    /// Natural width in pixels.
    width: u32,
    /// Natural height in pixels.
    height: u32,
}

/// Weak retained handle to a scene-backed image source.
#[derive(Clone, Debug)]
pub struct SceneImageWeak(Weak<SceneImageInner>);

impl SceneImageWeak {
    /// Upgrade the weak handle if the source is still alive.
    #[must_use]
    pub fn upgrade(&self) -> Option<SceneImage> {
        self.0.upgrade().map(SceneImage)
    }
}

impl SceneImage {
    /// Create a scene-backed image with an explicit natural size.
    #[must_use]
    pub fn new(scene: record::Scene, width: u32, height: u32) -> Self {
        Self::from_picture(
            ScenePicture::new(
                scene,
                Rect::new(0.0, 0.0, f64::from(width), f64::from(height)),
            ),
            width,
            height,
        )
    }

    /// Create a scene-backed image from an existing retained scene picture.
    #[must_use]
    pub fn from_picture(picture: ScenePicture, width: u32, height: u32) -> Self {
        Self(Arc::new(SceneImageInner {
            id: NEXT_SCENE_IMAGE_ID.fetch_add(1, Ordering::Relaxed),
            picture,
            width,
            height,
        }))
    }

    /// Return the stable identity of this scene-backed image.
    #[must_use]
    pub fn id(&self) -> u64 {
        self.0.id
    }

    /// Return the natural width of the scene image in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.0.width
    }

    /// Return the natural height of the scene image in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.0.height
    }

    /// Borrow the retained scene.
    #[must_use]
    pub fn scene(&self) -> &record::Scene {
        self.0.picture.scene()
    }

    /// Borrow the retained picture underlying this scene-backed image.
    #[must_use]
    pub fn picture(&self) -> &ScenePicture {
        &self.0.picture
    }

    /// Downgrade this retained scene image to a weak handle for cache entries.
    #[must_use]
    pub fn downgrade(&self) -> SceneImageWeak {
        SceneImageWeak(Arc::downgrade(&self.0))
    }
}

/// Imaging-owned image brush.
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(transparent)]
pub struct ImageBrush<D = Image>(pub peniko::ImageBrush<D>);

impl<D> Deref for ImageBrush<D> {
    type Target = peniko::ImageBrush<D>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<D> DerefMut for ImageBrush<D> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<D> ImageBrush<D> {
    /// Builder method for setting the image extend mode in both directions.
    #[must_use]
    pub fn with_extend(mut self, mode: peniko::Extend) -> Self {
        self.0 = self.0.with_extend(mode);
        self
    }

    /// Builder method for setting the image extend mode in the horizontal direction.
    #[must_use]
    pub fn with_x_extend(mut self, mode: peniko::Extend) -> Self {
        self.0 = self.0.with_x_extend(mode);
        self
    }

    /// Builder method for setting the image extend mode in the vertical direction.
    #[must_use]
    pub fn with_y_extend(mut self, mode: peniko::Extend) -> Self {
        self.0 = self.0.with_y_extend(mode);
        self
    }

    /// Builder method for setting the desired image quality hint.
    #[must_use]
    pub fn with_quality(mut self, quality: peniko::ImageQuality) -> Self {
        self.0 = self.0.with_quality(quality);
        self
    }

    /// Return the image with the alpha multiplier set to `alpha`.
    #[must_use]
    #[track_caller]
    pub fn with_alpha(mut self, alpha: f32) -> Self {
        self.0 = self.0.with_alpha(alpha);
        self
    }

    /// Return the image with its alpha multiplier multiplied by `alpha`.
    #[must_use]
    #[track_caller]
    pub fn multiply_alpha(mut self, alpha: f32) -> Self {
        self.0 = self.0.multiply_alpha(alpha);
        self
    }
}

impl ImageBrush {
    /// Create a new image brush with default sampling.
    #[must_use]
    pub fn new(image: impl Into<Image>) -> Self {
        Self(peniko::ImageBrush {
            image: image.into(),
            sampler: peniko::ImageSampler::default(),
        })
    }

    /// Borrow this image brush.
    #[must_use]
    pub fn as_ref(&self) -> ImageBrushRef<'_> {
        ImageBrush(peniko::ImageBrush {
            image: self.image.as_ref(),
            sampler: self.sampler,
        })
    }
}

impl From<Image> for ImageBrush {
    fn from(image: Image) -> Self {
        Self(peniko::ImageBrush {
            image,
            sampler: peniko::ImageSampler::default(),
        })
    }
}

impl From<peniko::ImageData> for ImageBrush {
    fn from(image: peniko::ImageData) -> Self {
        Image::from(image).into()
    }
}

impl From<SceneImage> for ImageBrush {
    fn from(image: SceneImage) -> Self {
        Image::from(image).into()
    }
}

impl From<peniko::ImageBrush> for ImageBrush {
    fn from(image: peniko::ImageBrush) -> Self {
        Self(peniko::ImageBrush {
            image: Image::Raster(image.image),
            sampler: image.sampler,
        })
    }
}

/// Borrowed image brush.
pub type ImageBrushRef<'a> = ImageBrush<ImageRef<'a>>;

fn image_brush_as_ref(image: &ImageBrush) -> ImageBrushRef<'_> {
    image.as_ref()
}

fn image_brush_ref_to_owned(image: &ImageBrushRef<'_>) -> ImageBrush {
    ImageBrush(peniko::ImageBrush {
        image: image.image.to_owned(),
        sampler: image.sampler,
    })
}

impl<'a> From<&'a ImageBrush> for ImageBrushRef<'a> {
    fn from(value: &'a ImageBrush) -> Self {
        image_brush_as_ref(value)
    }
}

impl<'a> From<&'a peniko::ImageBrush> for ImageBrushRef<'a> {
    fn from(value: &'a peniko::ImageBrush) -> Self {
        Self(peniko::ImageBrush {
            image: ImageRef::Raster(&value.image),
            sampler: value.sampler,
        })
    }
}

impl<'a> From<&'a peniko::ImageData> for ImageBrushRef<'a> {
    fn from(image: &'a peniko::ImageData) -> Self {
        Self(peniko::ImageBrush {
            image: image.into(),
            sampler: peniko::ImageSampler::default(),
        })
    }
}

impl<'a> From<&'a SceneImage> for ImageBrushRef<'a> {
    fn from(image: &'a SceneImage) -> Self {
        Self(peniko::ImageBrush {
            image: image.into(),
            sampler: peniko::ImageSampler::default(),
        })
    }
}

/// Imaging-owned brush.
#[derive(Clone, Debug, PartialEq)]
pub enum Brush {
    /// Solid color brush.
    Solid(peniko::Color),
    /// Gradient brush.
    Gradient(peniko::Gradient),
    /// Image brush.
    Image(ImageBrush),
}

impl Brush {
    /// Return the brush with the alpha component set to `alpha`.
    #[must_use]
    pub fn with_alpha(self, alpha: f32) -> Self {
        match self {
            Self::Solid(color) => Self::Solid(color.with_alpha(alpha)),
            Self::Gradient(gradient) => Self::Gradient(gradient.with_alpha(alpha)),
            Self::Image(image) => Self::Image(image.with_alpha(alpha)),
        }
    }

    /// Return the brush with the alpha component multiplied by `alpha`.
    #[must_use]
    #[track_caller]
    pub fn multiply_alpha(self, alpha: f32) -> Self {
        debug_assert!(
            alpha.is_finite() && alpha >= 0.0,
            "A non-finite or negative alpha ({alpha}) is meaningless."
        );
        if alpha == 1.0 {
            self
        } else {
            match self {
                Self::Solid(color) => Self::Solid(color.multiply_alpha(alpha)),
                Self::Gradient(gradient) => Self::Gradient(gradient.multiply_alpha(alpha)),
                Self::Image(image) => Self::Image(image.multiply_alpha(alpha)),
            }
        }
    }
}

impl Default for Brush {
    fn default() -> Self {
        Self::Solid(peniko::Color::TRANSPARENT)
    }
}

impl From<peniko::Color> for Brush {
    fn from(value: peniko::Color) -> Self {
        Self::Solid(value)
    }
}

impl From<&peniko::Color> for Brush {
    fn from(value: &peniko::Color) -> Self {
        Self::Solid(*value)
    }
}

impl From<peniko::Gradient> for Brush {
    fn from(value: peniko::Gradient) -> Self {
        Self::Gradient(value)
    }
}

impl From<ImageBrush> for Brush {
    fn from(value: ImageBrush) -> Self {
        Self::Image(value)
    }
}

impl From<peniko::Brush> for Brush {
    fn from(value: peniko::Brush) -> Self {
        match value {
            peniko::Brush::Solid(color) => Self::Solid(color),
            peniko::Brush::Gradient(gradient) => Self::Gradient(gradient),
            peniko::Brush::Image(image) => Self::Image(image.into()),
        }
    }
}

/// Borrowed imaging brush.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum BrushRef<'a> {
    /// Solid color brush.
    Solid(peniko::Color),
    /// Gradient brush.
    Gradient(&'a peniko::Gradient),
    /// Image brush.
    Image(ImageBrushRef<'a>),
}

impl BrushRef<'_> {
    /// Convert the borrowed brush into an owned brush.
    #[must_use]
    pub fn to_owned(&self) -> Brush {
        match self {
            Self::Solid(color) => Brush::Solid(*color),
            Self::Gradient(gradient) => Brush::Gradient((*gradient).clone()),
            Self::Image(image) => Brush::Image(image_brush_ref_to_owned(image)),
        }
    }
}

impl<'a> From<peniko::Color> for BrushRef<'a> {
    fn from(value: peniko::Color) -> Self {
        Self::Solid(value)
    }
}

impl<'a> From<&'a peniko::Color> for BrushRef<'a> {
    fn from(value: &'a peniko::Color) -> Self {
        Self::Solid(*value)
    }
}

impl<'a> From<&'a peniko::Gradient> for BrushRef<'a> {
    fn from(value: &'a peniko::Gradient) -> Self {
        Self::Gradient(value)
    }
}

impl<'a> From<&'a ImageBrush> for BrushRef<'a> {
    fn from(value: &'a ImageBrush) -> Self {
        Self::Image(image_brush_as_ref(value))
    }
}

impl<'a> From<ImageBrushRef<'a>> for BrushRef<'a> {
    fn from(value: ImageBrushRef<'a>) -> Self {
        Self::Image(value)
    }
}

impl<'a> From<&'a peniko::ImageData> for BrushRef<'a> {
    fn from(value: &'a peniko::ImageData) -> Self {
        Self::Image(value.into())
    }
}

impl<'a> From<&'a SceneImage> for BrushRef<'a> {
    fn from(value: &'a SceneImage) -> Self {
        Self::Image(value.into())
    }
}

impl<'a> From<&'a Brush> for BrushRef<'a> {
    fn from(value: &'a Brush) -> Self {
        match value {
            Brush::Solid(color) => Self::Solid(*color),
            Brush::Gradient(gradient) => Self::Gradient(gradient),
            Brush::Image(image) => Self::Image(image_brush_as_ref(image)),
        }
    }
}

impl<'a> From<&'a peniko::Brush> for BrushRef<'a> {
    fn from(value: &'a peniko::Brush) -> Self {
        match value {
            peniko::Brush::Solid(color) => Self::Solid(*color),
            peniko::Brush::Gradient(gradient) => Self::Gradient(gradient),
            peniko::Brush::Image(image) => Self::Image(ImageBrush(peniko::ImageBrush {
                image: ImageRef::Raster(&image.image),
                sampler: image.sampler,
            })),
        }
    }
}
