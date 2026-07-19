use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_VARIANT_DIMENSION: u32 = 4_096;
pub const MAX_VARIANT_OUTPUT_PIXELS: u64 = 16_777_216;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VariantFit {
    Cover,
    Contain,
    #[default]
    Inside,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CropPosition {
    #[default]
    Center,
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VariantFormat {
    Jpeg,
    Png,
    Webp,
}

impl VariantFormat {
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Webp => "webp",
        }
    }

    #[must_use]
    pub const fn mime(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Webp => "image/webp",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VariantTransform {
    width: Option<u32>,
    height: Option<u32>,
    fit: VariantFit,
    quality: u8,
    format: VariantFormat,
    blur: u8,
    crop: CropPosition,
    background: String,
}

impl VariantTransform {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        width: Option<u32>,
        height: Option<u32>,
        fit: VariantFit,
        quality: u8,
        format: VariantFormat,
        blur: u8,
        crop: CropPosition,
        background: impl Into<String>,
    ) -> Result<Self, VariantError> {
        if width.is_some_and(|value| value == 0 || value > MAX_VARIANT_DIMENSION)
            || height.is_some_and(|value| value == 0 || value > MAX_VARIANT_DIMENSION)
        {
            return Err(VariantError::InvalidDimension);
        }
        if width.zip(height).is_some_and(|(width, height)| {
            u64::from(width) * u64::from(height) > MAX_VARIANT_OUTPUT_PIXELS
        }) {
            return Err(VariantError::OutputTooLarge);
        }
        if !(1..=100).contains(&quality) {
            return Err(VariantError::InvalidQuality);
        }
        if blur > 100 {
            return Err(VariantError::InvalidBlur);
        }
        let background = background.into().to_ascii_lowercase();
        if background.len() != 6 || !background.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(VariantError::InvalidBackground);
        }
        Ok(Self {
            width,
            height,
            fit,
            quality,
            format,
            blur,
            crop,
            background,
        })
    }

    #[must_use]
    pub const fn width(&self) -> Option<u32> {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> Option<u32> {
        self.height
    }

    #[must_use]
    pub const fn fit(&self) -> VariantFit {
        self.fit
    }

    #[must_use]
    pub const fn quality(&self) -> u8 {
        self.quality
    }

    #[must_use]
    pub const fn format(&self) -> VariantFormat {
        self.format
    }

    #[must_use]
    pub const fn blur(&self) -> u8 {
        self.blur
    }

    #[must_use]
    pub const fn crop(&self) -> CropPosition {
        self.crop
    }

    #[must_use]
    pub fn background(&self) -> &str {
        &self.background
    }

    #[must_use]
    pub fn canonical(&self) -> String {
        format!(
            "w={}&h={}&fit={}&quality={}&format={}&blur={}&crop={}&background={}",
            self.width
                .map_or_else(String::new, |value| value.to_string()),
            self.height
                .map_or_else(String::new, |value| value.to_string()),
            enum_name(self.fit),
            self.quality,
            format_name(self.format),
            self.blur,
            crop_name(self.crop),
            self.background,
        )
    }
}

const fn enum_name(value: VariantFit) -> &'static str {
    match value {
        VariantFit::Cover => "cover",
        VariantFit::Contain => "contain",
        VariantFit::Inside => "inside",
    }
}

const fn format_name(value: VariantFormat) -> &'static str {
    match value {
        VariantFormat::Jpeg => "jpeg",
        VariantFormat::Png => "png",
        VariantFormat::Webp => "webp",
    }
}

const fn crop_name(value: CropPosition) -> &'static str {
    match value {
        CropPosition::Center => "center",
        CropPosition::Top => "top",
        CropPosition::Bottom => "bottom",
        CropPosition::Left => "left",
        CropPosition::Right => "right",
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum VariantError {
    #[error("variant dimensions are outside the supported range")]
    InvalidDimension,
    #[error("variant output exceeds the pixel limit")]
    OutputTooLarge,
    #[error("variant quality must be between 1 and 100")]
    InvalidQuality,
    #[error("variant blur must be between 0 and 100")]
    InvalidBlur,
    #[error("variant background must be a six-digit hexadecimal color")]
    InvalidBackground,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_parameters_include_defaults_and_normalize_color() {
        let transform = VariantTransform::new(
            Some(600),
            None,
            VariantFit::Inside,
            80,
            VariantFormat::Webp,
            0,
            CropPosition::Center,
            "AABBCC",
        )
        .expect("transform");
        assert_eq!(
            transform.canonical(),
            "w=600&h=&fit=inside&quality=80&format=webp&blur=0&crop=center&background=aabbcc"
        );
    }

    #[test]
    fn dimensionless_transform_preserves_an_explicit_cache_identity() {
        let transform = VariantTransform::new(
            None,
            None,
            VariantFit::Inside,
            80,
            VariantFormat::Webp,
            0,
            CropPosition::Center,
            "ffffff",
        )
        .expect("dimensionless transform");

        assert_eq!(
            transform.canonical(),
            "w=&h=&fit=inside&quality=80&format=webp&blur=0&crop=center&background=ffffff"
        );
    }

    #[test]
    fn unsafe_transform_limits_are_rejected() {
        assert_eq!(
            VariantTransform::new(
                Some(MAX_VARIANT_DIMENSION + 1),
                Some(1),
                VariantFit::Cover,
                80,
                VariantFormat::Jpeg,
                0,
                CropPosition::Center,
                "ffffff",
            ),
            Err(VariantError::InvalidDimension)
        );
    }

    #[test]
    fn avif_is_not_a_variant_output_format() {
        assert!(serde_json::from_str::<VariantFormat>("\"avif\"").is_err());
    }
}
