use std::{
    io::Cursor,
    sync::{Arc, LazyLock},
    time::Duration,
};

use async_trait::async_trait;
use image::{
    DynamicImage, GenericImageView, ImageFormat, ImageReader, Limits, Rgba,
    codecs::jpeg::JpegEncoder, imageops::FilterType,
};
#[cfg(all(feature = "libvips", target_os = "linux"))]
use libvips::{VipsApp, VipsImage, ops};
use mediahub_app::{ImageProcessor, ImageProcessorError, ProcessedVariant};
use mediahub_core::{
    CropPosition, MAX_VARIANT_DIMENSION, MAX_VARIANT_OUTPUT_PIXELS, VariantFit, VariantFormat,
    VariantTransform,
};

pub const PROCESSOR_VERSION: &str =
    concat!("image-rs-libwebp-lossy-v1-", env!("CARGO_PKG_VERSION"));
#[cfg(all(feature = "libvips", target_os = "linux"))]
pub const VIPS_PROCESSOR_VERSION: &str = concat!(
    "libvips-8.18.4-binding-2.3.0-mediahub-",
    env!("CARGO_PKG_VERSION")
);
pub const MAX_INPUT_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_INPUT_DIMENSION: u32 = 16_384;
pub const MAX_DECODED_BYTES: u64 = 256 * 1024 * 1024;
pub const PROCESSING_TIMEOUT: Duration = Duration::from_secs(30);
pub const MAX_BLOCKING_IMAGE_TASKS: usize = 4;

static BLOCKING_IMAGE_SLOTS: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(MAX_BLOCKING_IMAGE_TASKS)));
#[cfg(all(feature = "libvips", target_os = "linux"))]
static VIPS_APP: LazyLock<Option<VipsApp>> = LazyLock::new(|| {
    VipsApp::new("mediahub", false).ok().inspect(|app| {
        app.concurrency_set(MAX_BLOCKING_IMAGE_TASKS as i32);
    })
});

#[derive(Clone, Debug, Default)]
pub struct RustImageProcessor;

#[async_trait]
impl ImageProcessor for RustImageProcessor {
    fn processor_version(&self) -> &str {
        PROCESSOR_VERSION
    }

    async fn process(
        &self,
        input: &[u8],
        transform: &VariantTransform,
    ) -> Result<ProcessedVariant, ImageProcessorError> {
        if input.is_empty() || input.len() > MAX_INPUT_BYTES {
            return Err(ImageProcessorError::InputTooLarge);
        }
        let input = input.to_vec();
        let transform = transform.clone();
        run_blocking(move || process_sync(&input, &transform)).await
    }
}

/// Docker image processor backed by the native libvips runtime.
#[cfg(all(feature = "libvips", target_os = "linux"))]
#[derive(Clone, Debug, Default)]
pub struct VipsImageProcessor;

#[cfg(all(feature = "libvips", target_os = "linux"))]
#[async_trait]
impl ImageProcessor for VipsImageProcessor {
    fn processor_version(&self) -> &str {
        VIPS_PROCESSOR_VERSION
    }

    async fn process(
        &self,
        input: &[u8],
        transform: &VariantTransform,
    ) -> Result<ProcessedVariant, ImageProcessorError> {
        if input.is_empty() || input.len() > MAX_INPUT_BYTES {
            return Err(ImageProcessorError::InputTooLarge);
        }
        let input = input.to_vec();
        let transform = transform.clone();
        run_blocking(move || process_vips_sync(&input, &transform)).await
    }
}

include!("image_vips.rs");
include!("image_runtime.rs");
include!("image_rust.rs");

#[cfg(test)]
mod tests {
    include!("image_tests.rs");
}
