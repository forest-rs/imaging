//! Shared renderer-facing types and traits for `imaging` backends.

use std::{sync::Arc, sync::mpsc};

use crate::{PaintSink, record::Glyph};
use peniko::{Blob, ImageAlphaType, ImageData, ImageFormat, kurbo::Size};

/// Per-frame metadata passed from a UI or render loop into a renderer.
#[derive(Clone, Copy, Debug)]
pub struct BeginFrame {
    /// Output size in display-space coordinates.
    pub size: Size,
    /// Display scale factor for this frame.
    pub scale: f64,
    /// Faux-bold strength applied by higher layers for text rendering.
    pub font_embolden: f32,
}

/// Backend-neutral render output returned from a renderer readback path.
#[derive(Debug)]
pub enum RenderOutput {
    /// CPU-readable image output.
    Image(ImageData),
    /// GPU texture output for backends that render to a GPU target.
    GpuTexture(wgpu::TextureView),
}

impl RenderOutput {
    /// Return the image payload when this output is CPU-backed.
    pub fn into_image(self) -> Option<ImageData> {
        match self {
            Self::Image(image) => Some(image),
            Self::GpuTexture(_) => None,
        }
    }

    /// Read a GPU texture output into an image using the supplied `wgpu` device and queue.
    pub fn into_image_with(self, device: &wgpu::Device, queue: &wgpu::Queue) -> Option<ImageData> {
        match self {
            Self::Image(image) => Some(image),
            Self::GpuTexture(texture) => read_texture_view_to_image(&texture, device, queue).ok(),
        }
    }
}

/// CPU target buffer pixel layout understood by target renderers.
#[derive(Clone, Copy, Debug)]
pub enum CpuBufferFormat {
    /// Opaque RGBA8 pixels.
    Rgba8Opaque,
    /// Opaque BGRA8 pixels.
    Bgra8Opaque,
}

/// CPU target metadata used for compatibility checks before binding a renderer.
#[derive(Clone, Copy, Debug)]
pub struct CpuBufferTargetInfo {
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
    /// Distance in bytes between the start of adjacent rows.
    pub bytes_per_row: usize,
    /// Pixel format of the destination buffer.
    pub format: CpuBufferFormat,
}

/// Borrowed CPU target buffer passed to a renderer that writes directly into caller-owned memory.
#[derive(Debug)]
pub struct CpuBufferTarget<'a> {
    /// Destination pixel bytes.
    pub buffer: &'a mut [u8],
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
    /// Distance in bytes between the start of adjacent rows.
    pub bytes_per_row: usize,
    /// Pixel format of the destination buffer.
    pub format: CpuBufferFormat,
}

impl CpuBufferTarget<'_> {
    /// Return buffer metadata without exposing the caller-owned storage.
    pub fn info(&self) -> CpuBufferTargetInfo {
        CpuBufferTargetInfo {
            width: self.width,
            height: self.height,
            bytes_per_row: self.bytes_per_row,
            format: self.format,
        }
    }
}

/// Owned GPU target description passed to a renderer that writes directly into a GPU texture.
#[derive(Debug)]
pub struct GpuTextureTarget {
    /// Device that owns the target texture.
    pub device: wgpu::Device,
    /// Queue used to submit rendering commands.
    pub queue: wgpu::Queue,
    /// Texture view to render into.
    pub texture_view: wgpu::TextureView,
}

/// Minimal object-safe renderer interface used by higher-level integrations.
pub trait RenderCore {
    /// Stream paint commands into the renderer for the current frame.
    fn render(&mut self, f: &mut dyn FnMut(&mut dyn PaintSink));
    /// Finalize any pending work for the current frame.
    fn finish(&mut self);
    /// Read back the current frame output when available.
    fn readback(&mut self) -> Option<RenderOutput>;
    /// Return backend-specific diagnostic information.
    fn debug_info(&self) -> String {
        String::new()
    }
}

/// Renderer that owns its output target and can be resized/reset across frames.
pub trait Renderer: RenderCore {
    /// Backend-specific target handle exposed by this renderer.
    type Target;

    /// Resize or recreate the renderer for a new frame configuration.
    fn set_size(&mut self, frame: BeginFrame);
    /// Reset per-frame state before rendering starts.
    fn reset(&mut self);
    /// Return the backend-native target when available.
    fn read_target(&mut self) -> Option<Self::Target>;
}

/// Renderer that binds to a caller-provided target at creation time.
pub trait TargetRenderer: RenderCore + Sized {
    /// Backend-specific target type required to create this renderer.
    type Target;

    /// Validate whether this renderer supports the supplied CPU target metadata before creation.
    ///
    /// Higher-level integrations can use this to choose another renderer in a fallback chain
    /// before binding a window backend to an incompatible CPU target.
    fn supports_cpu_buffer_target(_target: &CpuBufferTargetInfo) -> Result<(), String> {
        Ok(())
    }

    /// Create a renderer bound to the supplied target.
    fn create(frame: BeginFrame, target: Self::Target) -> Result<Self, String>;
}

/// Convenience alias for glyph iterators streamed through renderer-facing APIs.
pub type GlyphIter<'a> = dyn Iterator<Item = Glyph> + 'a;

fn read_texture_view_to_image(
    texture_view: &wgpu::TextureView,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> Result<ImageData, String> {
    let texture = texture_view.texture();
    let size = texture.size();
    let width = size.width;
    let height = size.height;
    let (image_format, bytes_per_pixel) = match texture.format() {
        wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => {
            (ImageFormat::Rgba8, 4_usize)
        }
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => {
            (ImageFormat::Bgra8, 4_usize)
        }
        format => {
            return Err(format!(
                "unsupported texture format for readback: {format:?}"
            ));
        }
    };
    let width_bytes = width
        .checked_mul(
            u32::try_from(bytes_per_pixel)
                .expect("bytes per pixel fits in u32 for supported formats"),
        )
        .ok_or_else(|| "texture row size overflow during readback".to_string())?;
    let padded_bytes_per_row = width_bytes.div_ceil(256) * 256;
    let width_bytes_usize =
        usize::try_from(width_bytes).map_err(|_| "texture row size too large".to_string())?;
    let padded_bytes_per_row_usize = usize::try_from(padded_bytes_per_row)
        .map_err(|_| "padded texture row size too large".to_string())?;

    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("imaging readback"),
        size: u64::from(padded_bytes_per_row) * u64::from(height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("imaging readback"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        size,
    );
    queue.submit([encoder.finish()]);

    let slice = readback.slice(..);
    let (tx, rx) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|_| "device poll failed".to_string())?;
    rx.recv()
        .map_err(|_| "map_async callback dropped".to_string())?
        .map_err(|_| "buffer map failed".to_string())?;

    let mapped = slice.get_mapped_range();
    let mut data = Vec::with_capacity(width_bytes_usize * height as usize);
    for row in mapped.chunks_exact(padded_bytes_per_row_usize) {
        data.extend_from_slice(&row[..width_bytes_usize]);
    }
    drop(mapped);
    readback.unmap();

    Ok(ImageData {
        data: Blob::new(Arc::new(data)),
        format: image_format,
        width,
        height,
        alpha_type: ImageAlphaType::Alpha,
    })
}
