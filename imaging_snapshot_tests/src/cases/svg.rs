// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use imaging::{PaintSink, Painter};
use svg_imaging::{ParseOptions, RenderOptions, SvgDocument};

use super::SnapshotCase;

const TEST_FONT_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../test_assets/fonts/NotoSerif-Regular.ttf"
));
const TEST_FONT_FAMILY: &str = "Noto Serif";

const SVG_LAYERED_CARD: &[u8] = br#"
    <svg xmlns='http://www.w3.org/2000/svg' width='256' height='256' viewBox='0 0 256 256'>
        <defs>
            <linearGradient id='panel' x1='32' y1='24' x2='224' y2='224'>
                <stop offset='0' stop-color='#1d3347'/>
                <stop offset='0.55' stop-color='#325d81'/>
                <stop offset='1' stop-color='#102030'/>
            </linearGradient>
            <radialGradient id='halo' cx='164' cy='90' r='72' fx='152' fy='76'>
                <stop offset='0' stop-color='#fff7c2' stop-opacity='0.92'/>
                <stop offset='0.4' stop-color='#f6b94b' stop-opacity='0.74'/>
                <stop offset='1' stop-color='#f6b94b' stop-opacity='0'/>
            </radialGradient>
            <clipPath id='card-clip'>
                <path d='M40 28 H216 C228 28 236 36 236 48 V208 C236 220 228 228 216 228 H40 C28 228 20 220 20 208 V48 C20 36 28 28 40 28 Z'/>
            </clipPath>
        </defs>

        <rect x='0' y='0' width='256' height='256' fill='#0d1117'/>
        <rect x='20' y='28' width='216' height='200' rx='20' fill='url(#panel)'/>

        <g clip-path='url(#card-clip)' opacity='0.94'>
            <circle cx='164' cy='90' r='72' fill='url(#halo)'/>
            <path d='M18 172 C72 128 118 214 238 150 L238 238 L18 238 Z' fill='#0b1621' opacity='0.88'/>
            <path d='M36 66 L118 36 L212 108 L154 172 Z' fill='#ffffff' opacity='0.08'/>
            <image
                x='144'
                y='126'
                width='64'
                height='64'
                href='data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxMCIgaGVpZ2h0PSIxMCI+PHJlY3QgeD0iMCIgeT0iMCIgd2lkdGg9IjEwIiBoZWlnaHQ9IjEwIiBmaWxsPSIjMDBmZjAwIi8+PC9zdmc+'
            />
        </g>

        <path d='M44 52 H212' stroke='#dbe7f5' stroke-opacity='0.22' stroke-width='2'/>
        <path d='M44 204 H164' stroke='#dbe7f5' stroke-opacity='0.18' stroke-width='2'/>
        <circle cx='60' cy='52' r='4' fill='#f6b94b' opacity='0.8'/>
        <circle cx='78' cy='52' r='4' fill='#8cd3ff' opacity='0.7'/>
    </svg>
"#;

const SVG_TEXT_BANNER: &[u8] = br#"
    <svg xmlns='http://www.w3.org/2000/svg' width='256' height='256' viewBox='0 0 256 256'>
        <rect x='0' y='0' width='256' height='256' fill='#0e141b'/>
        <rect x='20' y='28' width='216' height='200' rx='24' fill='#15202b'/>
        <rect x='20' y='28' width='216' height='200' rx='24' fill='none' stroke='#314457' stroke-width='2'/>

        <text
            x='34'
            y='102'
            font-size='46'
            fill='#f6f7fb'
            letter-spacing='1.2'
        >Vector</text>

        <text
            x='34'
            y='152'
            font-size='38'
            fill='#f6b94b'
            stroke='#f6f7fb'
            stroke-opacity='0.18'
            stroke-width='2.5'
            paint-order='stroke'
        >Text</text>

        <path d='M36 176 H220' stroke='#314457' stroke-width='2'/>
        <text
            x='36'
            y='208'
            font-size='18'
            fill='#8ea4ba'
        >flattened through usvg</text>
    </svg>
"#;

const SVG_LUMINANCE_MASK: &[u8] = br#"
    <svg xmlns='http://www.w3.org/2000/svg' width='256' height='256' viewBox='0 0 256 256'>
        <defs>
            <linearGradient id='panel' x1='32' y1='36' x2='224' y2='220'>
                <stop offset='0' stop-color='#21415a'/>
                <stop offset='0.58' stop-color='#3d739f'/>
                <stop offset='1' stop-color='#122433'/>
            </linearGradient>
            <mask
                id='card-mask'
                mask-type='luminance'
                maskUnits='userSpaceOnUse'
                maskContentUnits='userSpaceOnUse'
                x='20'
                y='28'
                width='216'
                height='200'
            >
                <rect x='20' y='28' width='216' height='200' fill='black'/>
                <rect x='28' y='36' width='200' height='184' rx='24' fill='white'/>
                <circle cx='174' cy='96' r='42' fill='black'/>
                <path d='M28 170 C86 122 132 214 228 150 L228 220 L28 220 Z' fill='white'/>
            </mask>
        </defs>

        <rect x='0' y='0' width='256' height='256' fill='#0d1117'/>
        <g mask='url(#card-mask)'>
            <rect x='20' y='28' width='216' height='200' rx='24' fill='url(#panel)'/>
            <circle cx='164' cy='90' r='72' fill='#f6b94b' fill-opacity='0.75'/>
            <path d='M24 174 C80 128 128 212 238 146 L238 238 L24 238 Z' fill='#0a1622' fill-opacity='0.92'/>
            <path d='M40 66 L122 36 L214 104 L156 168 Z' fill='#ffffff' fill-opacity='0.09'/>
        </g>

        <path d='M44 52 H212' stroke='#dbe7f5' stroke-opacity='0.22' stroke-width='2'/>
        <path d='M44 204 H164' stroke='#dbe7f5' stroke-opacity='0.18' stroke-width='2'/>
        <circle cx='60' cy='52' r='4' fill='#f6b94b' opacity='0.8'/>
        <circle cx='78' cy='52' r='4' fill='#8cd3ff' opacity='0.7'/>
    </svg>
"#;

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

pub(crate) struct GmSvgLayeredCard;

impl SnapshotCase for GmSvgLayeredCard {
    fn name(&self) -> &'static str {
        "gm_svg_layered_card"
    }

    fn vello_max_diff_pixels(&self) -> u64 {
        8
    }

    fn run(&self, sink: &mut dyn PaintSink, _width: f64, _height: f64) {
        let document =
            SvgDocument::from_data(SVG_LAYERED_CARD, &ParseOptions::default()).expect("parse svg");
        let mut painter = Painter::new(sink);
        let report = document
            .render(&mut painter, &RenderOptions::default())
            .expect("render svg");
        assert!(
            report.unsupported_features.is_empty(),
            "unexpected unsupported SVG features: {:?}",
            report.unsupported_features
        );
    }
}

pub(crate) struct GmSvgTextBanner;

impl SnapshotCase for GmSvgTextBanner {
    fn name(&self) -> &'static str {
        "gm_svg_text_banner"
    }

    fn vello_max_diff_pixels(&self) -> u64 {
        12
    }

    fn run(&self, sink: &mut dyn PaintSink, _width: f64, _height: f64) {
        let options = parse_options_with_test_font();
        let document = SvgDocument::from_data(SVG_TEXT_BANNER, &options).expect("parse svg");
        let mut painter = Painter::new(sink);
        let report = document
            .render(&mut painter, &RenderOptions::default())
            .expect("render svg");
        assert!(
            report.unsupported_features.is_empty(),
            "unexpected unsupported SVG features: {:?}",
            report.unsupported_features
        );
    }
}

pub(crate) struct GmSvgLuminanceMask;

impl SnapshotCase for GmSvgLuminanceMask {
    fn name(&self) -> &'static str {
        "gm_svg_luminance_mask"
    }

    fn vello_max_diff_pixels(&self) -> u64 {
        10
    }

    fn vello_hybrid_max_diff_pixels(&self) -> u64 {
        10
    }

    fn supports_backend(&self, backend: &str) -> bool {
        matches!(
            backend,
            "skia" | "tiny_skia" | "vello_cpu" | "vello" | "vello_hybrid"
        )
    }

    fn run(&self, sink: &mut dyn PaintSink, _width: f64, _height: f64) {
        let document = SvgDocument::from_data(SVG_LUMINANCE_MASK, &ParseOptions::default())
            .expect("parse svg");
        let mut painter = Painter::new(sink);
        let report = document
            .render(&mut painter, &RenderOptions::default())
            .expect("render svg");
        assert!(
            report.unsupported_features.is_empty(),
            "unexpected unsupported SVG features: {:?}",
            report.unsupported_features
        );
    }
}
