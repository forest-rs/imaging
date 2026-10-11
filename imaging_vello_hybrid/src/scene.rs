// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::sync::Arc;

use crate::image_registry::RegisteredImage;

/// An encoded hybrid scene that keeps its uploaded images alive.
///
/// Scenes recorded with a renderer must be rendered by that same renderer. Cache eviction
/// and clearing do not release images while a scene still references them.
#[derive(Debug)]
pub struct VelloHybridScene {
    pub(crate) native: vello_gpu::Scene,
    pub(crate) external_images: Vec<Arc<crate::ExternalTexture>>,
    pub(crate) images: Vec<Arc<RegisteredImage>>,
    pub(crate) renderer: Option<Arc<()>>,
}

impl VelloHybridScene {
    /// Create an empty scene with the given viewport dimensions.
    #[must_use]
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            native: vello_gpu::Scene::new(width, height),
            images: Vec::new(),
            external_images: Vec::new(),
            renderer: None,
        }
    }

    /// Remove all drawing commands and release their image references.
    pub fn reset(&mut self) {
        self.native.reset();
        self.images.clear();
        self.external_images.clear();
        self.renderer = None;
    }

    /// Clear the scene and change its viewport dimensions.
    pub fn reset_and_resize(&mut self, width: u16, height: u16) {
        self.reset();
        self.native.reset_and_resize(width, height);
    }

    /// Return the viewport width in pixels.
    #[must_use]
    pub fn width(&self) -> u16 {
        self.native.width()
    }

    /// Return the viewport height in pixels.
    #[must_use]
    pub fn height(&self) -> u16 {
        self.native.height()
    }

    /// Borrow the underlying native scene without releasing its image references.
    #[must_use]
    pub fn native(&self) -> &vello_gpu::Scene {
        &self.native
    }
}
