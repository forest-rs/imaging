# svg_imaging

`svg_imaging` parses SVG with `usvg` and renders it through `imaging`.

The crate itself is `no_std` plus `alloc`. Its optional `std` feature only enables additional
`std` integration and dependency features, and the current dependency stack still effectively
requires `std` today.

The crate is intentionally layered:

- `usvg` owns SVG parsing and normalization.
- `svg_imaging` owns SVG semantic lowering and unsupported-feature reporting.
- `imaging` owns backend-neutral painting and recording.

Current focus:

- document loading
- group lowering for opacity/blend/isolation
- clip paths, including referenced clip-path chains, lowered into isolated group clips
- masks lowered into reusable `imaging` mask definitions plus masked groups
- path fills and strokes
- solid colors and gradients
- text lowered through `usvg`'s flattened vector output
- nested SVG `<image>` nodes
- raster PNG/JPEG/GIF/WebP `<image>` nodes
- paint order
- explicit reporting for unsupported features such as filters, image decode failures, and pattern
  paints

Text rendering depends on the `usvg::Options` font database. `ParseOptions::default()` starts
with an empty font database, so callers that need text should load fonts before parsing.

```rust
use imaging::{Painter, record::Scene};
use svg_imaging::{ParseOptions, RenderOptions, SvgDocument};

let svg = br#"
    <svg xmlns='http://www.w3.org/2000/svg' width='16' height='16'>
        <rect x='1' y='1' width='14' height='14' fill='#3465a4'/>
    </svg>
"#;

let document = SvgDocument::from_data(svg, &ParseOptions::default())?;
let mut scene = Scene::new();
let mut painter = Painter::new(&mut scene);
let report = document.render(&mut painter, &RenderOptions::default())?;

assert!(report.unsupported_features.is_empty());
assert!(!scene.commands().is_empty());
# Ok::<(), svg_imaging::Error>(())
```
