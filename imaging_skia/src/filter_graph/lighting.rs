// Copyright 2026 the Imaging Authors and the AnyRender contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use imaging::filter::LightSource;
use peniko::Color;
use skia_safe::{self as sk, image_filters};

use super::{Error, native, scalar};
use crate::color_to_sk_color;

pub(super) enum Lighting {
    Diffuse(f64),
    Specular { coefficient: f64, exponent: f64 },
}

enum NativeLight {
    Distant(sk::Point3),
    Point(sk::Point3),
    Spot {
        location: sk::Point3,
        aim: sk::Point3,
        falloff: f32,
        cone: f32,
    },
}

impl TryFrom<&LightSource> for NativeLight {
    type Error = Error;

    fn try_from(light: &LightSource) -> Result<Self, Error> {
        Ok(match *light {
            LightSource::Distant { azimuth, elevation } => {
                let (azimuth_sin, azimuth_cos) = azimuth.to_radians().sin_cos();
                let (height, lateral) = elevation.to_radians().sin_cos();
                Self::Distant(point([
                    azimuth_cos * lateral,
                    azimuth_sin * lateral,
                    height,
                ])?)
            }
            LightSource::Point { position } => Self::Point(point(position)?),
            LightSource::Spot {
                position,
                points_at,
                specular_exponent,
                limiting_cone_angle,
            } => Self::Spot {
                location: point(position)?,
                aim: point(points_at)?,
                falloff: scalar(specular_exponent)?,
                cone: scalar(limiting_cone_angle.unwrap_or(90.0))?,
            },
        })
    }
}

pub(super) fn lighting_filter(
    input: sk::ImageFilter,
    scale: f64,
    color: Color,
    light: &LightSource,
    lighting: Lighting,
) -> Result<sk::ImageFilter, Error> {
    let light = NativeLight::try_from(light)?;
    let scale = scalar(scale)?;
    let color = color_to_sk_color(color);
    let result = match lighting {
        Lighting::Diffuse(coefficient) => {
            let kd = scalar(coefficient)?;
            match light {
                NativeLight::Distant(direction) => {
                    image_filters::distant_lit_diffuse(direction, color, scale, kd, input, None)
                }
                NativeLight::Point(location) => {
                    image_filters::point_lit_diffuse(location, color, scale, kd, input, None)
                }
                NativeLight::Spot {
                    location,
                    aim,
                    falloff,
                    cone,
                } => image_filters::spot_lit_diffuse(
                    location, aim, falloff, cone, color, scale, kd, input, None,
                ),
            }
        }
        Lighting::Specular {
            coefficient,
            exponent,
        } => {
            let ks = scalar(coefficient)?;
            let exponent = scalar(exponent)?;
            match light {
                NativeLight::Distant(direction) => image_filters::distant_lit_specular(
                    direction, color, scale, ks, exponent, input, None,
                ),
                NativeLight::Point(location) => image_filters::point_lit_specular(
                    location, color, scale, ks, exponent, input, None,
                ),
                NativeLight::Spot {
                    location,
                    aim,
                    falloff,
                    cone,
                } => image_filters::spot_lit_specular(
                    location, aim, falloff, cone, color, scale, ks, exponent, input, None,
                ),
            }
        }
    };
    native(result)
}

fn point([x, y, z]: [f64; 3]) -> Result<sk::Point3, Error> {
    Ok(sk::Point3::new(scalar(x)?, scalar(y)?, scalar(z)?))
}
