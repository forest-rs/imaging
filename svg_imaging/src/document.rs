// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Document loading and top-level rendering entry points.

use imaging::{PaintSink, Painter};
use kurbo::{Affine, Size};

use crate::{Error, RenderReport, emit, lower};

/// Parsing options forwarded to `usvg`.
pub type ParseOptions<'a> = usvg::Options<'a>;

/// Top-level rendering options.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderOptions {
    /// Additional transform applied after SVG-local transforms.
    pub transform: Affine,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            transform: Affine::IDENTITY,
        }
    }
}

/// Parsed SVG document ready for repeated rendering.
#[derive(Clone, Debug)]
pub struct SvgDocument {
    tree: usvg::Tree,
}

impl SvgDocument {
    /// Parse an SVG document from raw bytes.
    pub fn from_data(data: &[u8], options: &ParseOptions<'_>) -> Result<Self, Error> {
        Ok(Self {
            tree: usvg::Tree::from_data(data, options)?,
        })
    }

    /// Parse an SVG document from UTF-8 text.
    pub fn from_str(text: &str, options: &ParseOptions<'_>) -> Result<Self, Error> {
        Ok(Self {
            tree: usvg::Tree::from_str(text, options)?,
        })
    }

    /// Return the parsed SVG tree.
    #[must_use]
    pub fn tree(&self) -> &usvg::Tree {
        &self.tree
    }

    /// Return the document size in user-space units.
    #[must_use]
    pub fn size(&self) -> Size {
        let size = self.tree.size();
        Size::new(f64::from(size.width()), f64::from(size.height()))
    }

    /// Render the document through an `imaging` painter.
    pub fn render<S>(
        &self,
        painter: &mut Painter<'_, S>,
        options: &RenderOptions,
    ) -> Result<RenderReport, Error>
    where
        S: PaintSink + ?Sized,
    {
        let plan = lower::lower_tree(&self.tree, options);
        emit::emit_plan(&plan, painter);
        Ok(plan.report)
    }
}

#[cfg(test)]
mod tests {
    use alloc::{borrow::ToOwned, boxed::Box, vec, vec::Vec};
    use std::sync::Arc;

    use imaging::{Brush, MaskMode, Painter, record};

    use super::{ParseOptions, RenderOptions, SvgDocument};

    const TEST_FONT_BYTES: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../test_assets/fonts/NotoSerif-Regular.ttf"
    ));
    const TEST_FONT_FAMILY: &str = "Noto Serif";

    fn parse_options_with_string_image(
        href: &'static str,
        kind: usvg::ImageKind,
    ) -> ParseOptions<'static> {
        let mut options = ParseOptions::default();
        options.image_href_resolver.resolve_string =
            Box::new(move |requested, _| (requested == href).then(|| kind.clone()));
        options
    }

    fn parse_options_with_test_font() -> ParseOptions<'static> {
        let mut options = ParseOptions {
            font_family: TEST_FONT_FAMILY.to_owned(),
            ..ParseOptions::default()
        };
        options
            .fontdb_mut()
            .load_font_data(TEST_FONT_BYTES.to_vec());
        options
    }

    fn png_1x1_red() -> Vec<u8> {
        let mut out = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut out);
        image::ImageEncoder::write_image(
            encoder,
            &[0xff, 0x00, 0x00, 0xff],
            1,
            1,
            image::ExtendedColorType::Rgba8,
        )
        .expect("encode png");
        out
    }

    fn gif_1x1_red() -> Vec<u8> {
        let mut out = Vec::new();
        let encoder = image::codecs::gif::GifEncoder::new(&mut out);
        image::ImageEncoder::write_image(
            encoder,
            &[0xff, 0x00, 0x00, 0xff],
            1,
            1,
            image::ExtendedColorType::Rgba8,
        )
        .expect("encode gif");
        out
    }

    fn webp_1x1_red() -> Vec<u8> {
        let mut out = Vec::new();
        let encoder = image::codecs::webp::WebPEncoder::new_lossless(&mut out);
        image::ImageEncoder::write_image(
            encoder,
            &[0xff, 0x00, 0x00, 0xff],
            1,
            1,
            image::ExtendedColorType::Rgba8,
        )
        .expect("encode webp");
        out
    }

    #[test]
    fn parses_document_size() {
        let document = SvgDocument::from_data(
            br#"<svg xmlns='http://www.w3.org/2000/svg' width='12' height='34'/>"#,
            &ParseOptions::default(),
        )
        .expect("parse SVG");

        assert_eq!(document.size(), kurbo::Size::new(12.0, 34.0));
    }

    #[test]
    fn renders_basic_fill_and_stroke() {
        let document = SvgDocument::from_data(
            br#"
                <svg xmlns='http://www.w3.org/2000/svg' width='20' height='20'>
                    <rect x='2' y='3' width='10' height='11' fill='#123456' stroke='#abcdef' stroke-width='2'/>
                </svg>
            "#,
            &ParseOptions::default(),
        )
        .expect("parse SVG");

        let mut scene = record::Scene::new();
        let mut painter = Painter::new(&mut scene);
        let report = document
            .render(&mut painter, &RenderOptions::default())
            .expect("render SVG");

        assert!(report.unsupported_features.is_empty());
        assert_eq!(scene.commands().len(), 2);
    }

    #[test]
    fn lowers_text_via_flattened_vectors_when_fonts_are_available() {
        let document = SvgDocument::from_data(
            br#"
                <svg xmlns='http://www.w3.org/2000/svg' width='160' height='40'>
                    <text x='12' y='28' font-size='24' fill='#123456'>SVG</text>
                </svg>
            "#,
            &parse_options_with_test_font(),
        )
        .expect("parse SVG");

        let mut scene = record::Scene::new();
        let mut painter = Painter::new(&mut scene);
        let report = document
            .render(&mut painter, &RenderOptions::default())
            .expect("render SVG");

        assert!(
            report
                .unsupported_features
                .iter()
                .all(|feature| feature.kind != crate::UnsupportedFeatureKind::Text)
        );
        assert!(!scene.commands().is_empty());
    }

    #[test]
    fn lowers_text_paint_order_and_stroke_when_fonts_are_available() {
        let document = SvgDocument::from_data(
            br#"
                <svg xmlns='http://www.w3.org/2000/svg' width='200' height='60'>
                    <text
                        x='12'
                        y='38'
                        font-size='28'
                        fill='#f6b94b'
                        stroke='#102030'
                        stroke-width='2'
                        paint-order='stroke'
                    >Stroke</text>
                </svg>
            "#,
            &parse_options_with_test_font(),
        )
        .expect("parse SVG");

        let mut scene = record::Scene::new();
        let mut painter = Painter::new(&mut scene);
        let report = document
            .render(&mut painter, &RenderOptions::default())
            .expect("render SVG");

        assert!(
            report
                .unsupported_features
                .iter()
                .all(|feature| feature.kind != crate::UnsupportedFeatureKind::Text)
        );
        assert!(scene.commands().len() >= 2);
    }

    #[test]
    fn lowers_masked_group_without_reporting_mask_unsupported() {
        let document = SvgDocument::from_data(
            br#"
                <svg xmlns='http://www.w3.org/2000/svg' width='128' height='128'>
                    <defs>
                        <mask
                            id='cutout'
                            mask-type='alpha'
                            maskUnits='userSpaceOnUse'
                            maskContentUnits='userSpaceOnUse'
                            x='12'
                            y='12'
                            width='104'
                            height='104'
                        >
                            <rect x='12' y='12' width='104' height='104' fill='white'/>
                            <circle cx='64' cy='64' r='20' fill='white' fill-opacity='0'/>
                        </mask>
                    </defs>
                    <g mask='url(#cutout)'>
                        <rect x='16' y='16' width='96' height='96' fill='#4f8dd8'/>
                    </g>
                </svg>
            "#,
            &ParseOptions::default(),
        )
        .expect("parse SVG");

        let mut scene = record::Scene::new();
        let mut painter = Painter::new(&mut scene);
        let report = document
            .render(&mut painter, &RenderOptions::default())
            .expect("render SVG");

        assert!(
            report
                .unsupported_features
                .iter()
                .all(|feature| feature.kind != crate::UnsupportedFeatureKind::Mask)
        );

        let group_id = scene
            .commands()
            .iter()
            .find_map(|command| match command {
                record::Command::PushGroup(group) if scene.group(*group).mask.is_some() => {
                    Some(*group)
                }
                _ => None,
            })
            .expect("expected masked group");
        let group = scene.group(group_id);
        let mask = group.mask.as_ref().expect("group should carry mask");
        assert_eq!(scene.mask(mask.mask).mode, MaskMode::Alpha);
        assert!(
            scene.commands().iter().any(|command| match command {
                record::Command::PushGroup(group) => scene.group(*group).clip.is_some(),
                _ => false,
            }),
            "mask region should also create an isolated clip layer"
        );
    }

    #[test]
    fn reuses_mask_definitions_in_the_lowered_plan() {
        let document = SvgDocument::from_data(
            br#"
                <svg xmlns='http://www.w3.org/2000/svg' width='160' height='96'>
                    <defs>
                        <mask
                            id='window'
                            mask-type='luminance'
                            maskUnits='userSpaceOnUse'
                            maskContentUnits='userSpaceOnUse'
                            x='8'
                            y='8'
                            width='144'
                            height='80'
                        >
                            <rect x='8' y='8' width='144' height='80' fill='black'/>
                            <rect x='24' y='20' width='112' height='56' rx='16' fill='white'/>
                        </mask>
                    </defs>
                    <g mask='url(#window)'>
                        <rect x='8' y='8' width='64' height='80' fill='#f6b94b'/>
                    </g>
                    <g mask='url(#window)'>
                        <rect x='88' y='8' width='64' height='80' fill='#4f8dd8'/>
                    </g>
                </svg>
            "#,
            &ParseOptions::default(),
        )
        .expect("parse SVG");

        let plan = crate::lower::lower_tree(document.tree(), &RenderOptions::default());
        assert_eq!(plan.masks.len(), 1, "expected one shared lowered mask");

        let mut mask_ids = Vec::new();
        collect_mask_ids(&plan.nodes, &mut mask_ids);
        assert_eq!(
            mask_ids,
            vec![crate::plan::PlanMaskId(0), crate::plan::PlanMaskId(0)]
        );
    }

    fn collect_mask_ids(nodes: &[crate::plan::PlanNode], out: &mut Vec<crate::plan::PlanMaskId>) {
        for node in nodes {
            if let crate::plan::PlanNode::Group(group) = node {
                if let Some(mask) = group.mask {
                    out.push(mask.mask);
                }
                collect_mask_ids(&group.children, out);
            }
        }
    }

    #[test]
    fn lowers_linear_gradient_with_brush_transform() {
        let document = SvgDocument::from_data(
            br#"
                <svg xmlns='http://www.w3.org/2000/svg' width='20' height='20'>
                    <defs>
                        <linearGradient id='g' x1='0' y1='0' x2='10' y2='0' gradientTransform='translate(1 2)'>
                            <stop offset='0' stop-color='#000000'/>
                            <stop offset='1' stop-color='#ffffff'/>
                        </linearGradient>
                    </defs>
                    <rect x='0' y='0' width='10' height='10' fill='url(#g)'/>
                </svg>
            "#,
            &ParseOptions::default(),
        )
        .expect("parse SVG");

        let mut scene = record::Scene::new();
        let mut painter = Painter::new(&mut scene);
        document
            .render(&mut painter, &RenderOptions::default())
            .expect("render SVG");

        let draw_id = match scene.commands()[0] {
            record::Command::Draw(id) => id,
            ref other => panic!("expected draw command, got {other:?}"),
        };
        let draw = match scene.draw_op(draw_id) {
            record::Draw::Fill {
                brush,
                brush_transform,
                ..
            } => (brush, brush_transform),
            other => panic!("expected fill draw, got {other:?}"),
        };
        assert!(matches!(draw.0, Brush::Gradient(_)));
        assert_eq!(
            draw.1,
            &Some(kurbo::Affine::new([10.0, 0.0, 0.0, 10.0, 10.0, 20.0]))
        );
    }

    #[test]
    fn lowers_simple_clip_paths() {
        let document = SvgDocument::from_data(
            br#"
                <svg xmlns='http://www.w3.org/2000/svg' width='20' height='20'>
                    <defs>
                        <clipPath id='clip'>
                            <rect x='0' y='0' width='10' height='10'/>
                        </clipPath>
                    </defs>
                    <g clip-path='url(#clip)'>
                        <rect x='0' y='0' width='20' height='20' fill='#ff0000'/>
                    </g>
                </svg>
            "#,
            &ParseOptions::default(),
        )
        .expect("parse SVG");

        let mut scene = record::Scene::new();
        let mut painter = Painter::new(&mut scene);
        let report = document
            .render(&mut painter, &RenderOptions::default())
            .expect("render SVG");

        assert!(report.unsupported_features.is_empty());
        assert_eq!(scene.commands().len(), 3);
    }

    #[test]
    fn lowers_referenced_clip_path_chains() {
        let document = SvgDocument::from_data(
            br#"
                <svg xmlns='http://www.w3.org/2000/svg' width='20' height='20'>
                    <defs>
                        <clipPath id='outer'>
                            <rect x='0' y='0' width='16' height='16'/>
                        </clipPath>
                        <clipPath id='inner' clip-path='url(#outer)'>
                            <circle cx='10' cy='10' r='10'/>
                        </clipPath>
                    </defs>
                    <g clip-path='url(#inner)'>
                        <rect x='0' y='0' width='20' height='20' fill='#ff0000'/>
                    </g>
                </svg>
            "#,
            &ParseOptions::default(),
        )
        .expect("parse SVG");

        let mut scene = record::Scene::new();
        let mut painter = Painter::new(&mut scene);
        let report = document
            .render(&mut painter, &RenderOptions::default())
            .expect("render SVG");

        assert!(report.unsupported_features.is_empty());
        assert_eq!(scene.commands().len(), 5);
        assert!(matches!(scene.commands()[0], record::Command::PushGroup(_)));
        assert!(matches!(scene.commands()[1], record::Command::PushGroup(_)));
        assert!(matches!(scene.commands()[2], record::Command::Draw(_)));
        assert!(matches!(scene.commands()[3], record::Command::PopGroup));
        assert!(matches!(scene.commands()[4], record::Command::PopGroup));
    }

    #[test]
    fn lowers_nested_svg_image_nodes() {
        let document = SvgDocument::from_data(
            br#"
                <svg xmlns='http://www.w3.org/2000/svg' width='20' height='20'>
                    <image
                        x='5'
                        y='6'
                        width='10'
                        height='10'
                        href='data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxMCIgaGVpZ2h0PSIxMCI+PHJlY3QgeD0iMCIgeT0iMCIgd2lkdGg9IjEwIiBoZWlnaHQ9IjEwIiBmaWxsPSIjMDBmZjAwIi8+PC9zdmc+'
                    />
                </svg>
            "#,
            &ParseOptions::default(),
        )
        .expect("parse SVG");

        let mut scene = record::Scene::new();
        let mut painter = Painter::new(&mut scene);
        let report = document
            .render(&mut painter, &RenderOptions::default())
            .expect("render SVG");

        assert!(report.unsupported_features.is_empty());
        assert!(!scene.commands().is_empty());
    }

    #[test]
    fn lowers_png_image_nodes() {
        let svg = r#"
            <svg xmlns='http://www.w3.org/2000/svg' width='20' height='20'>
                <image x='4' y='5' width='6' height='7' href='memory:test.png'/>
            </svg>
        "#;
        let options = parse_options_with_string_image(
            "memory:test.png",
            usvg::ImageKind::PNG(Arc::new(png_1x1_red())),
        );
        let document = SvgDocument::from_str(svg, &options).expect("parse SVG");

        let mut scene = record::Scene::new();
        let mut painter = Painter::new(&mut scene);
        let report = document
            .render(&mut painter, &RenderOptions::default())
            .expect("render SVG");

        assert!(report.unsupported_features.is_empty());
        let draw_id = match scene.commands()[0] {
            record::Command::Draw(id) => id,
            ref other => panic!("expected draw command, got {other:?}"),
        };
        match scene.draw_op(draw_id) {
            record::Draw::Fill {
                shape,
                brush: Brush::Image(image),
                ..
            } => {
                assert_eq!(
                    *shape,
                    record::Geometry::Rect(kurbo::Rect::new(0.0, 0.0, 1.0, 1.0))
                );
                assert_eq!(image.image.width(), 1);
                assert_eq!(image.image.height(), 1);
            }
            other => panic!("expected image fill draw, got {other:?}"),
        }
    }

    #[test]
    fn lowers_gif_image_nodes() {
        let svg = r#"
            <svg xmlns='http://www.w3.org/2000/svg' width='20' height='20'>
                <image x='0' y='0' width='10' height='10' href='memory:test.gif'/>
            </svg>
        "#;
        let options = parse_options_with_string_image(
            "memory:test.gif",
            usvg::ImageKind::GIF(Arc::new(gif_1x1_red())),
        );
        let document = SvgDocument::from_str(svg, &options).expect("parse SVG");

        let mut scene = record::Scene::new();
        let mut painter = Painter::new(&mut scene);
        let report = document
            .render(&mut painter, &RenderOptions::default())
            .expect("render SVG");

        assert!(report.unsupported_features.is_empty());
        assert_eq!(scene.commands().len(), 1);
    }

    #[test]
    fn lowers_webp_image_nodes() {
        let svg = r#"
            <svg xmlns='http://www.w3.org/2000/svg' width='20' height='20'>
                <image x='1' y='2' width='10' height='10' href='memory:test.webp'/>
            </svg>
        "#;
        let options = parse_options_with_string_image(
            "memory:test.webp",
            usvg::ImageKind::WEBP(Arc::new(webp_1x1_red())),
        );
        let document = SvgDocument::from_str(svg, &options).expect("parse SVG");

        let mut scene = record::Scene::new();
        let mut painter = Painter::new(&mut scene);
        let report = document
            .render(&mut painter, &RenderOptions::default())
            .expect("render SVG");

        assert!(report.unsupported_features.is_empty());
        assert_eq!(scene.commands().len(), 1);
    }
}
