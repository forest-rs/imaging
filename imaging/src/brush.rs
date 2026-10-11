// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{Image, ImageRef};
use peniko::color::{AlphaColor, ColorSpace, DynamicColor, OpaqueColor, Srgb};
use peniko::{Gradient, ImageSampler};

/// Image content and sampling parameters for a fill, stroke, or glyph run.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageBrush {
    /// Image to sample.
    pub image: Image,
    /// Sampling and opacity parameters.
    pub sampler: ImageSampler,
}

/// Borrowed image content and sampling parameters.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ImageBrushRef<'a> {
    /// Image to sample.
    pub image: ImageRef<'a>,
    /// Sampling and opacity parameters.
    pub sampler: ImageSampler,
}

impl ImageBrush {
    /// Create an image brush with default sampling parameters.
    #[must_use]
    pub fn new(image: impl Into<Image>) -> Self {
        Self {
            image: image.into(),
            sampler: ImageSampler::default(),
        }
    }
    /// Set sampling parameters for this image.
    #[must_use]
    pub fn with_sampler(mut self, sampler: ImageSampler) -> Self {
        self.sampler = sampler;
        self
    }
    /// Borrow this brush.
    #[must_use]
    pub fn as_ref(&self) -> ImageBrushRef<'_> {
        ImageBrushRef {
            image: self.image.as_ref(),
            sampler: self.sampler,
        }
    }
}

impl<'a> ImageBrushRef<'a> {
    /// Clone this brush into an owned input.
    #[must_use]
    pub fn to_owned(self) -> ImageBrush {
        ImageBrush {
            image: self.image.to_owned(),
            sampler: self.sampler,
        }
    }
    /// Borrow raster pixels and sampling parameters, if this is a raster image.
    #[must_use]
    pub fn as_raster(self) -> Option<peniko::ImageBrushRef<'a>> {
        match self.image {
            ImageRef::Raster(image) => Some(peniko::ImageBrush {
                image,
                sampler: self.sampler,
            }),
            ImageRef::External(_) => None,
        }
    }
}

impl<'a> From<&'a ImageBrush> for ImageBrushRef<'a> {
    fn from(image: &'a ImageBrush) -> Self {
        image.as_ref()
    }
}
impl<'a, T: ?Sized> From<&'a T> for ImageBrushRef<'a>
where
    &'a T: Into<ImageRef<'a>>,
{
    fn from(image: &'a T) -> Self {
        Self {
            image: image.into(),
            sampler: ImageSampler::default(),
        }
    }
}
impl<'a, T> From<&'a peniko::ImageBrush<T>> for ImageBrushRef<'a>
where
    &'a T: Into<ImageRef<'a>>,
{
    fn from(image: &'a peniko::ImageBrush<T>) -> Self {
        Self {
            image: (&image.image).into(),
            sampler: image.sampler,
        }
    }
}
impl<'a> From<peniko::ImageBrushRef<'a>> for ImageBrushRef<'a> {
    fn from(image: peniko::ImageBrushRef<'a>) -> Self {
        Self {
            image: image.image.into(),
            sampler: image.sampler,
        }
    }
}

/// Owned paint used by retained scenes.
#[derive(Clone, Debug, PartialEq)]
pub enum Brush {
    /// Solid color.
    Solid(peniko::Color),
    /// Gradient.
    Gradient(Gradient),
    /// Raster or external image.
    Image(ImageBrush),
}

/// Borrowed paint accepted by streaming drawing commands.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum BrushRef<'a> {
    /// Solid color.
    Solid(peniko::Color),
    /// Gradient.
    Gradient(&'a Gradient),
    /// Raster or external image.
    Image(ImageBrushRef<'a>),
}

impl Default for Brush {
    fn default() -> Self {
        Self::Solid(peniko::Color::TRANSPARENT)
    }
}
impl Brush {
    /// Borrow this paint without cloning its resources.
    #[must_use]
    pub fn as_ref(&self) -> BrushRef<'_> {
        match self {
            Self::Solid(color) => BrushRef::Solid(*color),
            Self::Gradient(gradient) => BrushRef::Gradient(gradient),
            Self::Image(image) => BrushRef::Image(image.as_ref()),
        }
    }
}
impl<'a> BrushRef<'a> {
    /// Clone this paint into an owned input.
    #[must_use]
    pub fn to_owned(self) -> Brush {
        match self {
            Self::Solid(color) => Brush::Solid(color),
            Self::Gradient(gradient) => Brush::Gradient(gradient.clone()),
            Self::Image(image) => Brush::Image(image.to_owned()),
        }
    }
    /// Borrow Peniko paint when it contains raster pixels or no image.
    ///
    /// External images require a renderer resource binding and return `None`.
    #[must_use]
    pub fn as_raster(self) -> Option<peniko::BrushRef<'a>> {
        Some(match self {
            Self::Solid(color) => peniko::BrushRef::Solid(color),
            Self::Gradient(gradient) => peniko::BrushRef::Gradient(gradient),
            Self::Image(image) => peniko::BrushRef::Image(image.as_raster()?),
        })
    }
}

impl<'a> From<&'a Brush> for BrushRef<'a> {
    fn from(brush: &'a Brush) -> Self {
        brush.as_ref()
    }
}
impl<'a> From<&'a Gradient> for BrushRef<'a> {
    fn from(gradient: &'a Gradient) -> Self {
        Self::Gradient(gradient)
    }
}
impl From<Gradient> for Brush {
    fn from(gradient: Gradient) -> Self {
        Self::Gradient(gradient)
    }
}
impl From<ImageBrush> for Brush {
    fn from(image: ImageBrush) -> Self {
        Self::Image(image)
    }
}
impl<'a> From<ImageBrushRef<'a>> for BrushRef<'a> {
    fn from(image: ImageBrushRef<'a>) -> Self {
        Self::Image(image)
    }
}
impl<'a> From<&'a ImageBrush> for BrushRef<'a> {
    fn from(image: &'a ImageBrush) -> Self {
        Self::Image(image.as_ref())
    }
}
impl<'a> From<&'a peniko::ImageBrush> for BrushRef<'a> {
    fn from(image: &'a peniko::ImageBrush) -> Self {
        Self::Image(image.into())
    }
}
impl<'a> From<peniko::ImageBrushRef<'a>> for BrushRef<'a> {
    fn from(image: peniko::ImageBrushRef<'a>) -> Self {
        Self::Image(image.into())
    }
}
impl<'a> From<peniko::BrushRef<'a>> for BrushRef<'a> {
    fn from(brush: peniko::BrushRef<'a>) -> Self {
        match brush {
            peniko::BrushRef::Solid(color) => Self::Solid(color),
            peniko::BrushRef::Gradient(gradient) => Self::Gradient(gradient),
            peniko::BrushRef::Image(image) => Self::Image(image.into()),
        }
    }
}
impl<'a> From<&'a peniko::Brush> for BrushRef<'a> {
    fn from(brush: &'a peniko::Brush) -> Self {
        peniko::BrushRef::from(brush).into()
    }
}
impl From<peniko::Brush> for Brush {
    fn from(brush: peniko::Brush) -> Self {
        match brush {
            peniko::Brush::Solid(color) => Self::Solid(color),
            peniko::Brush::Gradient(gradient) => Self::Gradient(gradient),
            peniko::Brush::Image(image) => Self::Image(ImageBrush {
                image: image.image.into(),
                sampler: image.sampler,
            }),
        }
    }
}

macro_rules! color_conversions {
    ($target:ty) => {
        impl<CS: ColorSpace> From<AlphaColor<CS>> for $target {
            fn from(color: AlphaColor<CS>) -> Self {
                Self::Solid(color.convert())
            }
        }
        impl<CS: ColorSpace> From<OpaqueColor<CS>> for $target {
            fn from(color: OpaqueColor<CS>) -> Self {
                Self::Solid(color.with_alpha(1.0).convert())
            }
        }
        impl From<DynamicColor> for $target {
            fn from(color: DynamicColor) -> Self {
                Self::Solid(color.to_alpha_color::<Srgb>())
            }
        }
        impl<CS: ColorSpace> From<&AlphaColor<CS>> for $target {
            fn from(color: &AlphaColor<CS>) -> Self {
                (*color).into()
            }
        }
        impl<CS: ColorSpace> From<&OpaqueColor<CS>> for $target {
            fn from(color: &OpaqueColor<CS>) -> Self {
                (*color).into()
            }
        }
        impl From<&DynamicColor> for $target {
            fn from(color: &DynamicColor) -> Self {
                (*color).into()
            }
        }
    };
}
color_conversions!(Brush);
color_conversions!(BrushRef<'_>);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ExternalImage, Painter,
        record::{self, Scene},
    };
    use kurbo::Affine;
    use peniko::ImageAlphaType;

    #[test]
    fn external_image_identity_and_sampling_survive_recording_and_replay() {
        let image = ExternalImage::new(2, 3, ImageAlphaType::AlphaPremultiplied);
        assert_eq!(image, image.clone());
        assert_ne!(image, ExternalImage::new(2, 3, image.alpha_type()));
        let brush = ImageBrush::new(image.clone()).with_sampler(
            ImageSampler::new()
                .with_extend(peniko::Extend::Repeat)
                .with_alpha(0.5),
        );
        let mut scene = Scene::new();
        Painter::new(&mut scene).draw_image(&brush, Affine::translate((4.0, 5.0)));
        drop(brush);
        assert!(scene.has_external_images());
        scene.validate().unwrap();
        let mut copy = Scene::new();
        record::replay(&scene, &mut copy);
        assert_eq!(scene, copy);
        let record::Command::Draw(id) = copy.commands()[0] else {
            panic!("expected draw");
        };
        let record::Draw::Fill {
            brush: Brush::Image(brush),
            shape: record::Geometry::Rect(rect),
            ..
        } = copy.draw_op(id)
        else {
            panic!("expected image fill");
        };
        assert_eq!(brush.image, Image::External(image));
        assert_eq!(brush.sampler.alpha, 0.5);
        assert_eq!(brush.sampler.x_extend, peniko::Extend::Repeat);
        assert_eq!(rect.width(), 2.0);
        assert_eq!(rect.height(), 3.0);
        assert!(brush.as_ref().as_raster().is_none());
    }
}
