use async_trait::async_trait;
use mediahub_core::{VariantFormat, VariantTransform};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessedVariant {
    pub content: Vec<u8>,
    pub mime: String,
    pub format: VariantFormat,
    pub width: u32,
    pub height: u32,
}

#[allow(clippy::missing_errors_doc)]
#[async_trait]
pub trait ImageProcessor: Send + Sync {
    fn processor_version(&self) -> &str;

    async fn process(
        &self,
        input: &[u8],
        transform: &VariantTransform,
    ) -> Result<ProcessedVariant, ImageProcessorError>;
}

#[must_use]
pub fn variant_cache_key(
    media_sha256: &str,
    transform: &VariantTransform,
    processor_version: &str,
) -> String {
    let source = format!(
        "{}\n{}\n{}",
        media_sha256.to_ascii_lowercase(),
        transform.canonical(),
        processor_version
    );
    hex::encode(Sha256::digest(source.as_bytes()))
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ImageProcessorError {
    #[error("input is not a supported image")]
    UnsupportedInput,
    #[error("input image exceeds configured limits")]
    InputTooLarge,
    #[error("output image exceeds configured limits")]
    OutputTooLarge,
    #[error("image transformation failed")]
    ProcessingFailed,
    #[error("image encoding failed")]
    EncodingFailed,
}

#[cfg(test)]
mod tests {
    use mediahub_core::{CropPosition, VariantFit};

    use super::*;

    #[test]
    fn cache_key_binds_source_parameters_and_processor_version() {
        let transform = VariantTransform::new(
            Some(100),
            Some(100),
            VariantFit::Cover,
            80,
            VariantFormat::Webp,
            0,
            CropPosition::Center,
            "ffffff",
        )
        .expect("transform");
        let first = variant_cache_key(&"a".repeat(64), &transform, "image-1");
        assert_eq!(first.len(), 64);
        assert_ne!(
            first,
            variant_cache_key(&"b".repeat(64), &transform, "image-1")
        );
        assert_ne!(
            first,
            variant_cache_key(&"a".repeat(64), &transform, "image-2")
        );
    }
}
