// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::sync::Arc;

/// Encoded Vello commands and the external image bindings they reference.
///
/// Scenes containing external images must be rendered with the renderer that encoded them.
/// Unbinding an image does not invalidate an existing scene.
#[derive(Clone, Default)]
pub struct VelloScene {
    pub(crate) native: vello::Scene,
    pub(crate) images: Vec<Arc<crate::ExternalTexture>>,
    pub(crate) renderer: Option<Arc<()>>,
}

impl VelloScene {
    /// Create an empty scene.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear the commands and release their image bindings.
    pub fn reset(&mut self) {
        self.native.reset();
        self.images.clear();
        self.renderer = None;
    }

    /// Borrow the native commands. External images require this scene's retained bindings.
    #[must_use]
    pub fn native(&self) -> &vello::Scene {
        &self.native
    }
}

impl From<vello::Scene> for VelloScene {
    /// Wrap native commands that do not use renderer-specific image overrides.
    fn from(native: vello::Scene) -> Self {
        Self {
            native,
            ..Self::default()
        }
    }
}

impl core::fmt::Debug for VelloScene {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VelloScene")
            .field("images", &self.images)
            .finish_non_exhaustive()
    }
}
