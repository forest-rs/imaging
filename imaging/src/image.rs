// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::sync::Arc;
use core::hash::{Hash, Hasher};

use peniko::{ImageAlphaType, ImageData};

/// A renderer-bound image with stable identity and immutable pixel metadata.
///
/// Clones identify the same image; separately created handles identify different images even
/// when their metadata matches. Renderers bind resources to these handles explicitly. A handle
/// does not contain pixels or keep a renderer alive. Unsupported or unbound images are errors.
///
/// The bound resource's contents may change between renders. Callers must finish writing it
/// before rendering a scene that samples it, using the renderer's GPU queue for ordering.
#[derive(Clone, Debug)]
pub struct ExternalImage(Arc<ExternalImageInfo>);

#[derive(Debug)]
struct ExternalImageInfo {
    width: u32,
    height: u32,
    alpha_type: ImageAlphaType,
}

impl ExternalImage {
    /// Create an unbound handle. Dimensions must be nonzero when used in a scene.
    #[must_use]
    pub fn new(width: u32, height: u32, alpha_type: ImageAlphaType) -> Self {
        Self(Arc::new(ExternalImageInfo {
            width,
            height,
            alpha_type,
        }))
    }

    /// Width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.0.width
    }

    /// Height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.0.height
    }

    /// Encoding of alpha in the bound pixels.
    #[must_use]
    pub fn alpha_type(&self) -> ImageAlphaType {
        self.0.alpha_type
    }
}

impl PartialEq for ExternalImage {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for ExternalImage {}
impl Hash for ExternalImage {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::ptr::hash(Arc::as_ptr(&self.0), state);
    }
}

/// An image used by a brush or [`crate::Painter::draw_image`].
#[derive(Clone, Debug, PartialEq)]
pub enum Image {
    /// Immutable, shared raster pixels.
    Raster(ImageData),
    /// A resource bound by the renderer.
    External(ExternalImage),
}

/// Borrowed image input, without cloning pixel data or handles.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum ImageRef<'a> {
    /// Immutable, shared raster pixels.
    Raster(&'a ImageData),
    /// A resource bound by the renderer.
    External(&'a ExternalImage),
}

impl Image {
    /// Borrow this image.
    #[must_use]
    pub fn as_ref(&self) -> ImageRef<'_> {
        match self {
            Self::Raster(image) => ImageRef::Raster(image),
            Self::External(image) => ImageRef::External(image),
        }
    }
    /// Width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.as_ref().width()
    }
    /// Height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.as_ref().height()
    }
}

impl ImageRef<'_> {
    /// Clone this image into an owned input.
    #[must_use]
    pub fn to_owned(self) -> Image {
        match self {
            Self::Raster(image) => Image::Raster(image.clone()),
            Self::External(image) => Image::External(image.clone()),
        }
    }
    /// Width in pixels.
    #[must_use]
    pub fn width(self) -> u32 {
        match self {
            Self::Raster(image) => image.width,
            Self::External(image) => image.width(),
        }
    }
    /// Height in pixels.
    #[must_use]
    pub fn height(self) -> u32 {
        match self {
            Self::Raster(image) => image.height,
            Self::External(image) => image.height(),
        }
    }
}

impl From<ImageData> for Image {
    fn from(image: ImageData) -> Self {
        Self::Raster(image)
    }
}
impl From<ExternalImage> for Image {
    fn from(image: ExternalImage) -> Self {
        Self::External(image)
    }
}
impl<'a> From<&'a Image> for ImageRef<'a> {
    fn from(image: &'a Image) -> Self {
        image.as_ref()
    }
}
impl<'a> From<&'a ImageData> for ImageRef<'a> {
    fn from(image: &'a ImageData) -> Self {
        Self::Raster(image)
    }
}
impl<'a> From<&'a ExternalImage> for ImageRef<'a> {
    fn from(image: &'a ExternalImage) -> Self {
        Self::External(image)
    }
}
