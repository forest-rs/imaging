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
    /// Parse an SVG document from [`usvg::Tree`].
    pub fn from_tree(tree: usvg::Tree) -> Self {
        Self { tree }
    }

    /// Parse an SVG or gzip-compressed SVGZ document from bytes.
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

    use imaging::Brush;
    use imaging::{
        MaskMode, Painter,
        filter::{FilterEffectsRef, FilterGraph},
        record,
    };
    use kurbo::Affine;

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
    fn parses_compressed_svg() {
        let document = SvgDocument::from_data(
            include_bytes!("../tests/fixtures/size.svgz"),
            &ParseOptions::default(),
        )
        .expect("parse SVGZ");

        assert_eq!(document.size(), kurbo::Size::new(12.0, 34.0));
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
    fn lowers_linear_filter_inputs_and_resolves_primitive_units() {
        let document = SvgDocument::from_str(
            "<svg xmlns='http://www.w3.org/2000/svg' width='80' height='80'>
                <defs><filter id='effects' primitiveUnits='objectBoundingBox'
                        color-interpolation-filters='sRGB'>
                    <feGaussianBlur in='SourceGraphic' stdDeviation='.125 .25' result='soft'/>
                    <feOffset in='soft' dx='.5' dy='.25'/>
                    <feDropShadow dx='.125' dy='.25' stdDeviation='.125 .25'
                        flood-color='#123456' flood-opacity='.5'/>
                </filter></defs>
                <g transform='translate(2 3) scale(2)' filter='url(#effects)'>
                    <rect width='16' height='4'/>
                </g>
            </svg>",
            &ParseOptions::default(),
        )
        .unwrap();
        let transform = Affine::translate((5.0, 7.0)) * Affine::scale(1.5);
        let mut scene = record::Scene::new();
        let report = document
            .render(&mut Painter::new(&mut scene), &RenderOptions { transform })
            .unwrap();

        assert!(report.is_fully_supported(), "{report:?}");
        scene.validate().unwrap();
        let groups: Vec<_> = scene
            .commands()
            .iter()
            .rev()
            .filter_map(|command| match command {
                record::Command::PushGroup(id) => match scene.group(*id).filters.as_ref() {
                    FilterEffectsRef::Chain(filters) if !filters.is_empty() => {
                        Some((scene.group(*id), filters))
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect();
        let filters: Vec<_> = groups.iter().flat_map(|(_, filters)| *filters).collect();
        assert_eq!(
            filters,
            [
                &imaging::Filter::blur_xy(2.0, 1.0),
                &imaging::Filter::offset(8.0, 1.0),
                &imaging::Filter::DropShadow {
                    dx: 2.0,
                    dy: 1.0,
                    std_deviation_x: 2.0,
                    std_deviation_y: 1.0,
                    color: peniko::Color::from_rgba8(0x12, 0x34, 0x56, 128),
                },
            ]
        );
        for (group, _) in groups {
            assert_eq!(
                group.filter_transform,
                transform * Affine::translate((2.0, 3.0)) * Affine::scale(2.0),
            );
        }
    }

    #[test]
    fn lowers_named_graph_inputs_and_keeps_filter_boundaries() {
        use imaging::filter::{
            CompositeOperator, FilterColorSpace, FilterId, FilterInput, FilterPrimitiveKind,
        };
        use kurbo::{Rect, Vec2};

        let (scene, report) = render_filter_test(
            "<svg xmlns='http://www.w3.org/2000/svg' width='32' height='32'>
                <defs>
                    <filter id='a' filterUnits='userSpaceOnUse' x='0' y='0' width='12' height='8'>
                        <feFlood x='2' y='1' width='3' height='3' result='color'/>
                        <feGaussianBlur in='SourceAlpha' stdDeviation='1'
                            color-interpolation-filters='sRGB' result='blurred'/>
                        <feOffset in='color' dx='1' dy='2'/>
                        <feMerge>
                            <feMergeNode in='blurred'/>
                            <feMergeNode in='SourceGraphic'/>
                            <feMergeNode/>
                        </feMerge>
                    </filter>
                    <filter id='b' filterUnits='userSpaceOnUse' x='3' y='1' width='20' height='15'>
                        <feComposite in='SourceGraphic' in2='SourceAlpha'
                            operator='lighter' result='color'/>
                        <feOffset in='color' dx='1'/>
                    </filter>
                </defs>
                <g filter='url(#a) url(#b)' transform='translate(2 3)' opacity='.5'>
                    <rect width='5' height='6'/>
                </g>
            </svg>",
        );
        assert!(report.is_fully_supported(), "{report:?}");
        let graphs = recorded_graphs(&scene);
        let [(second, second_transform), (first, first_transform)] = graphs.as_slice() else {
            panic!("expected two nested filter graphs");
        };
        assert_eq!(*first_transform, Affine::translate((2.0, 3.0)));
        assert_eq!(*second_transform, *first_transform);
        let result = |index| FilterInput::Result(FilterId(index));
        let flood_rect = Rect::new(2.0, 1.0, 5.0, 4.0);
        assert_eq!(first.region, Rect::new(0.0, 0.0, 12.0, 8.0));
        assert_eq!(first.primitives[0].subregion, Some(flood_rect));
        assert_eq!(first.primitives[0].color_space, FilterColorSpace::LinearRgb);
        assert_eq!(first.primitives[1].color_space, FilterColorSpace::Srgb);
        assert_eq!(
            first.primitives[1].kind,
            FilterPrimitiveKind::GaussianBlur {
                input: FilterInput::SourceAlpha,
                std_deviation: Vec2::new(1.0, 1.0),
            },
        );
        assert_eq!(first.primitives[2].subregion, Some(flood_rect));
        assert_eq!(
            first.primitives[2].kind,
            FilterPrimitiveKind::Offset {
                input: result(0),
                offset: Vec2::new(1.0, 2.0),
            },
        );
        assert_eq!(
            first.primitives[3].kind,
            FilterPrimitiveKind::Merge {
                inputs: vec![result(1), FilterInput::SourceGraphic, result(2)],
            },
        );
        assert_eq!(first.output, result(3));

        assert_eq!(second.region, Rect::new(3.0, 1.0, 23.0, 16.0));
        assert_eq!(
            second.primitives[0].kind,
            FilterPrimitiveKind::Composite {
                source: FilterInput::SourceGraphic,
                backdrop: FilterInput::SourceAlpha,
                operator: CompositeOperator::Lighter,
            },
        );
        assert_eq!(
            second.primitives[1].kind.inputs().collect::<Vec<_>>(),
            [result(0)]
        );
        assert_eq!(second.output, result(1));
        let [record::Command::PushGroup(group), ..] = scene.commands() else {
            panic!("expected group opacity outside the filter graphs");
        };
        assert_eq!(scene.group(*group).composite.alpha, 0.5);
        assert!(scene.group(*group).filters.as_ref().is_empty());
    }

    #[test]
    fn records_filter_image_content_at_its_subregion_origin() {
        use imaging::filter::FilterPrimitiveKind;
        use kurbo::Rect;

        let (scene, report) = render_filter_test(
            "<svg xmlns='http://www.w3.org/2000/svg' width='32' height='32'>
                <defs>
                    <rect id='image' width='2' height='3' fill='red'/>
                    <filter id='f' filterUnits='userSpaceOnUse' x='0' y='0' width='32' height='32'>
                        <feImage href='#image' x='10' y='12' width='8' height='6'/>
                    </filter>
                </defs>
                <g filter='url(#f)' transform='translate(2 3)'>
                    <rect width='20' height='20'/>
                </g>
            </svg>",
        );
        assert!(report.is_fully_supported(), "{report:?}");
        let graphs = recorded_graphs(&scene);
        let [(graph, transform)] = graphs.as_slice() else {
            panic!("expected one filter graph");
        };
        assert_eq!(*transform, Affine::translate((2.0, 3.0)));
        let node = &graph.primitives[0];
        assert_eq!(node.subregion, Some(Rect::new(10.0, 12.0, 18.0, 18.0)));
        let FilterPrimitiveKind::Image { scene } = &node.kind else {
            panic!("expected retained image content");
        };
        let [record::Command::Draw(draw)] = scene.commands() else {
            panic!("expected one draw in the retained filter image");
        };
        let record::Draw::Fill { transform, .. } = scene.draw_op(*draw) else {
            panic!("expected a retained fill");
        };
        assert_eq!(*transform, Affine::translate((10.0, 12.0)));
    }

    #[test]
    fn preserves_convolution_noise_and_lighting_parameters() {
        use imaging::filter::{
            EdgeMode, FilterInput, FilterPrimitiveKind, LightSource, TurbulenceKind,
        };
        use kurbo::Vec2;

        let (scene, report) = render_filter_test(
            "<svg xmlns='http://www.w3.org/2000/svg' width='32' height='32'>
                <defs><filter id='f'>
                    <feConvolveMatrix order='3 2' kernelMatrix='1 2 3 4 5 6'
                        targetX='2' targetY='0' edgeMode='wrap' divisor='8'
                        bias='.25' preserveAlpha='true'/>
                    <feTurbulence type='fractalNoise' baseFrequency='.125 .25'
                        numOctaves='2' seed='83' stitchTiles='stitch'/>
                    <feDiffuseLighting lighting-color='#123456' surfaceScale='2' diffuseConstant='3'>
                        <fePointLight x='5' y='8' z='13'/>
                    </feDiffuseLighting>
                </filter></defs>
                <rect width='20' height='20' filter='url(#f)'/>
            </svg>",
        );
        assert!(report.is_fully_supported(), "{report:?}");
        let graphs = recorded_graphs(&scene);
        let [(graph, _)] = graphs.as_slice() else {
            panic!("expected one filter graph");
        };
        assert_eq!(
            graph.primitives[0].kind,
            FilterPrimitiveKind::ConvolveMatrix {
                input: FilterInput::SourceGraphic,
                order: [3, 2],
                target: [2, 0],
                kernel: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
                divisor: 8.0,
                bias: 0.25,
                edge_mode: EdgeMode::Wrap,
                preserve_alpha: true,
            },
        );
        assert_eq!(
            graph.primitives[1].kind,
            FilterPrimitiveKind::Turbulence {
                base_frequency: Vec2::new(0.125, 0.25),
                num_octaves: 2,
                seed: 83,
                stitch_tiles: true,
                kind: TurbulenceKind::FractalNoise,
            },
        );
        assert_eq!(
            graph.primitives[2].kind,
            FilterPrimitiveKind::DiffuseLighting {
                input: FilterInput::Result(imaging::filter::FilterId(1)),
                surface_scale: 2.0,
                diffuse_constant: 3.0,
                lighting_color: peniko::Color::from_rgb8(0x12, 0x34, 0x56),
                light: LightSource::Point {
                    position: [5.0, 8.0, 13.0],
                },
            },
        );
    }

    #[test]
    fn falls_back_to_unfiltered_content_when_a_filter_image_cannot_be_lowered() {
        let (scene, report) = render_filter_test(
            "<svg xmlns='http://www.w3.org/2000/svg' width='24' height='24'>
                <defs>
                    <pattern id='p' width='.5' height='.5'><rect width='4' height='4'/></pattern>
                    <rect id='image' width='20' height='20' fill='url(#p)'/>
                    <filter id='first'><feOffset dx='1'/></filter>
                    <filter id='unsupported'><feImage href='#image'/></filter>
                </defs>
                <g id='subject' opacity='.5' filter='url(#first) url(#unsupported)'>
                    <rect width='20' height='20' fill='red'/>
                </g>
            </svg>",
        );
        assert_eq!(
            report.unsupported_features,
            [crate::UnsupportedFeature {
                kind: crate::UnsupportedFeatureKind::Filter,
                node_id: Some("subject".to_owned()),
            }],
        );
        let [
            record::Command::PushGroup(group),
            record::Command::Draw(_),
            record::Command::PopGroup,
        ] = scene.commands()
        else {
            panic!("expected unfiltered content with opacity");
        };
        let group = scene.group(*group);
        assert!(group.filters.as_ref().is_empty());
        assert_eq!(group.composite.alpha, 0.5);
    }

    fn render_filter_test(svg: &str) -> (record::Scene, crate::RenderReport) {
        let document = SvgDocument::from_str(svg, &ParseOptions::default()).unwrap();
        let mut scene = record::Scene::new();
        let report = document
            .render(&mut Painter::new(&mut scene), &RenderOptions::default())
            .unwrap();
        scene.validate().unwrap();
        (scene, report)
    }

    fn recorded_graphs(scene: &record::Scene) -> Vec<(&FilterGraph, Affine)> {
        scene
            .commands()
            .iter()
            .filter_map(|command| {
                let record::Command::PushGroup(group) = command else {
                    return None;
                };
                let group = scene.group(*group);
                match group.filters.as_ref() {
                    FilterEffectsRef::Graph(graph) => Some((graph, group.filter_transform)),
                    FilterEffectsRef::Chain(_) => None,
                }
            })
            .collect()
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
        let mask_scene = &scene.mask(mask.mask).scene;
        assert!(
            mask_scene.commands().iter().any(|command| match command {
                record::Command::PushGroup(group) => mask_scene.group(*group).clip.is_some(),
                _ => false,
            }),
            "the reusable mask definition should clip its content to the mask region"
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
            &Some(Affine::new([10.0, 0.0, 0.0, 10.0, 10.0, 20.0]))
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
