// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Thread-local font caching used by Skia text translation.

use std::cell::RefCell;
use std::collections::HashMap;

use imaging::GlyphRunRef;
use skia_safe as sk;

struct FontCache {
    font_mgr: sk::FontMgr,
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    extracted_font_data: HashMap<(u64, u32), peniko::FontData>,
    typefaces: HashMap<(u64, u32), sk::Typeface>,
    fonts: HashMap<(u64, u32, u32, bool), sk::Font>,
}

impl FontCache {
    /// Create an empty per-thread cache for typefaces and sized fonts.
    fn new() -> Self {
        Self {
            font_mgr: sk::FontMgr::new(),
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            extracted_font_data: HashMap::new(),
            typefaces: HashMap::new(),
            fonts: HashMap::new(),
        }
    }

    /// Resolve and cache the Skia typeface backing a `peniko` font reference.
    fn get_or_cache_typeface<'a>(
        &'a mut self,
        #[allow(
            unused_mut,
            reason = "Apple font extraction rewrites the borrowed font input behind cfg gates."
        )]
        mut font: &'a peniko::FontData,
    ) -> Option<sk::Typeface> {
        let cache_key = (font.data.id(), font.index);

        #[cfg(any(target_os = "macos", target_os = "ios"))]
        {
            use peniko::Blob;
            use std::sync::Arc;

            if let Some(collection) = oaty::Collection::new(font.data.data()) {
                self.extracted_font_data
                    .entry(cache_key)
                    .or_insert_with(|| {
                        let data = collection
                            .get_font(font.index)
                            .and_then(|font| font.copy_data())
                            .unwrap_or_default();
                        peniko::FontData::new(Blob::new(Arc::new(data)), 0)
                    });
                font = self.extracted_font_data.get(&cache_key)?;
            }
        }

        if let Some(typeface) = self.typefaces.get(&cache_key) {
            return Some(typeface.clone());
        }

        let typeface = self
            .font_mgr
            .new_from_data(font.data.data(), font.index as usize)
            .or_else(|| sk::Typeface::make_deserialize(font.data.data(), None))?;
        self.typefaces.insert(cache_key, typeface.clone());
        Some(typeface)
    }

    /// Resolve and cache the fully configured Skia font for a glyph run.
    fn get_or_cache_font(&mut self, glyph_run: &GlyphRunRef<'_>) -> Option<sk::Font> {
        let cache_key = (
            glyph_run.font.data.id(),
            glyph_run.font.index,
            glyph_run.font_size.to_bits(),
            glyph_run.hint,
        );

        if let Some(font) = self.fonts.get(&cache_key) {
            return Some(font.clone());
        }

        let typeface = self.get_or_cache_typeface(glyph_run.font)?;
        let mut font = sk::Font::from_typeface(typeface, glyph_run.font_size);
        font.set_hinting(if glyph_run.hint {
            sk::FontHinting::Slight
        } else {
            sk::FontHinting::None
        });
        self.fonts.insert(cache_key, font.clone());
        Some(font)
    }
}

thread_local! {
    static FONT_CACHE: RefCell<FontCache> = RefCell::new(FontCache::new());
}

/// Build the Skia font instance needed to draw a glyph run, including supported transforms.
pub(crate) fn skia_font_from_glyph_run(glyph_run: &GlyphRunRef<'_>) -> Option<sk::Font> {
    let mut font = FONT_CACHE.with_borrow_mut(|cache| cache.get_or_cache_font(glyph_run))?;

    if let Some(transform) = glyph_run.glyph_transform {
        let [a, b, c, d, e, f] = transform.as_coeffs();
        if b != 0.0 || e != 0.0 || f != 0.0 || d <= 0.0 {
            return None;
        }
        let y_scale = f64_to_f32(d);
        font.set_size(f64_to_f32(glyph_run.font_size as f64 * d));
        font.set_scale_x(f64_to_f32(a / d));
        font.set_skew_x(f64_to_f32(c / d));
        if y_scale <= 0.0 {
            return None;
        }
    }

    Some(font)
}

/// Narrow an `f64` value to `f32` for Skia text APIs that operate in single precision.
#[allow(
    clippy::cast_possible_truncation,
    reason = "Skia text APIs consume f32; truncation from f64 transforms is acceptable"
)]
fn f64_to_f32(v: f64) -> f32 {
    v as f32
}
