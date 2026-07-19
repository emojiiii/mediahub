// Optional libvips processor implementation.

#[cfg(all(feature = "libvips", target_os = "linux"))]
fn process_vips_sync(
    input: &[u8],
    transform: &VariantTransform,
) -> Result<ProcessedVariant, ImageProcessorError> {
    if VIPS_APP.is_none() {
        return Err(ImageProcessorError::ProcessingFailed);
    }
    let image =
        VipsImage::new_from_buffer(input, "").map_err(|_| ImageProcessorError::UnsupportedInput)?;
    let source_width =
        u32::try_from(image.get_width()).map_err(|_| ImageProcessorError::UnsupportedInput)?;
    let source_height =
        u32::try_from(image.get_height()).map_err(|_| ImageProcessorError::UnsupportedInput)?;
    if source_width == 0
        || source_height == 0
        || source_width > MAX_INPUT_DIMENSION
        || source_height > MAX_INPUT_DIMENSION
        || u64::from(source_width)
            .saturating_mul(u64::from(source_height))
            .saturating_mul(4)
            > MAX_DECODED_BYTES
    {
        return Err(ImageProcessorError::InputTooLarge);
    }
    let (width, height) = requested_dimensions(transform, source_width, source_height)?;
    let transformed = transform_vips(image, width, height, transform)?;
    let transformed = if transform.blur() == 0 {
        transformed
    } else {
        ops::gaussblur(&transformed, f64::from(transform.blur()) / 4.0)
            .map_err(|_| ImageProcessorError::ProcessingFailed)?
    };
    let output_width = u32::try_from(transformed.get_width())
        .map_err(|_| ImageProcessorError::ProcessingFailed)?;
    let output_height = u32::try_from(transformed.get_height())
        .map_err(|_| ImageProcessorError::ProcessingFailed)?;
    let content = encode_vips(&transformed, transform)?;
    Ok(ProcessedVariant {
        content,
        mime: transform.format().mime().to_owned(),
        format: transform.format(),
        width: output_width,
        height: output_height,
    })
}

#[cfg(all(feature = "libvips", target_os = "linux"))]
fn transform_vips(
    image: VipsImage,
    width: u32,
    height: u32,
    transform: &VariantTransform,
) -> Result<VipsImage, ImageProcessorError> {
    let source_width = f64::from(image.get_width());
    let source_height = f64::from(image.get_height());
    let target_width = f64::from(width);
    let target_height = f64::from(height);
    let scale = match transform.fit() {
        VariantFit::Cover => {
            (target_width / source_width).max(target_height / source_height) * (1.0 + f64::EPSILON)
        }
        VariantFit::Contain | VariantFit::Inside => {
            (target_width / source_width).min(target_height / source_height)
        }
    };
    let resized = ops::resize(&image, scale).map_err(|_| ImageProcessorError::ProcessingFailed)?;
    match transform.fit() {
        VariantFit::Cover => {
            let resized_width = u32::try_from(resized.get_width())
                .map_err(|_| ImageProcessorError::ProcessingFailed)?;
            let resized_height = u32::try_from(resized.get_height())
                .map_err(|_| ImageProcessorError::ProcessingFailed)?;
            if resized_width < width || resized_height < height {
                return Err(ImageProcessorError::ProcessingFailed);
            }
            let max_x = resized_width - width;
            let max_y = resized_height - height;
            let (x, y) = match transform.crop() {
                CropPosition::Center => (max_x / 2, max_y / 2),
                CropPosition::Top => (max_x / 2, 0),
                CropPosition::Bottom => (max_x / 2, max_y),
                CropPosition::Left => (0, max_y / 2),
                CropPosition::Right => (max_x, max_y / 2),
            };
            ops::extract_area(
                &resized,
                i32::try_from(x).map_err(|_| ImageProcessorError::ProcessingFailed)?,
                i32::try_from(y).map_err(|_| ImageProcessorError::ProcessingFailed)?,
                i32::try_from(width).map_err(|_| ImageProcessorError::ProcessingFailed)?,
                i32::try_from(height).map_err(|_| ImageProcessorError::ProcessingFailed)?,
            )
            .map_err(|_| ImageProcessorError::ProcessingFailed)
        }
        VariantFit::Contain => {
            let resized_width = u32::try_from(resized.get_width())
                .map_err(|_| ImageProcessorError::ProcessingFailed)?;
            let resized_height = u32::try_from(resized.get_height())
                .map_err(|_| ImageProcessorError::ProcessingFailed)?;
            let x = width.saturating_sub(resized_width) / 2;
            let y = height.saturating_sub(resized_height) / 2;
            let color = parse_background(transform.background())?;
            ops::embed_with_opts(
                &resized,
                i32::try_from(x).map_err(|_| ImageProcessorError::ProcessingFailed)?,
                i32::try_from(y).map_err(|_| ImageProcessorError::ProcessingFailed)?,
                i32::try_from(width).map_err(|_| ImageProcessorError::ProcessingFailed)?,
                i32::try_from(height).map_err(|_| ImageProcessorError::ProcessingFailed)?,
                &ops::EmbedOptions {
                    extend: ops::Extend::Background,
                    background: color.0[..3].iter().map(|value| f64::from(*value)).collect(),
                },
            )
            .map_err(|_| ImageProcessorError::ProcessingFailed)
        }
        VariantFit::Inside => Ok(resized),
    }
}

#[cfg(all(feature = "libvips", target_os = "linux"))]
fn encode_vips(
    image: &VipsImage,
    transform: &VariantTransform,
) -> Result<Vec<u8>, ImageProcessorError> {
    if let Some(app) = VIPS_APP.as_ref() {
        app.error_clear();
    }
    let quality = i32::from(transform.quality());
    let result = match transform.format() {
        VariantFormat::Jpeg => ops::jpegsave_buffer_with_opts(
            image,
            &ops::JpegsaveBufferOptions {
                q: quality,
                keep: ops::ForeignKeep::None,
                profile: None,
                ..ops::JpegsaveBufferOptions::default()
            },
        ),
        VariantFormat::Png => ops::pngsave_buffer_with_opts(
            image,
            &ops::PngsaveBufferOptions {
                q: quality,
                keep: ops::ForeignKeep::None,
                profile: None,
                ..ops::PngsaveBufferOptions::default()
            },
        ),
        VariantFormat::Webp => ops::webpsave_buffer_with_opts(
            image,
            &ops::WebpsaveBufferOptions {
                q: quality,
                keep: ops::ForeignKeep::None,
                profile: None,
                ..ops::WebpsaveBufferOptions::default()
            },
        ),
    };
    result.map_err(|error| {
        let libvips_error = VIPS_APP
            .as_ref()
            .and_then(|app| app.error_buffer().ok())
            .map(str::to_owned)
            .unwrap_or_else(|| "libvips error buffer unavailable".to_owned());
        #[cfg(test)]
        eprintln!("libvips encoding failed: {libvips_error}");
        tracing::error!(
            output_format = ?transform.format(),
            error = ?error,
            libvips_error,
            "libvips image encoding failed"
        );
        ImageProcessorError::EncodingFailed
    })
}

