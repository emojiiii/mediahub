// Rust image decoding, transforms, and encoding.

fn process_sync(
    input: &[u8],
    transform: &VariantTransform,
) -> Result<ProcessedVariant, ImageProcessorError> {
    let mut reader = ImageReader::new(Cursor::new(input))
        .with_guessed_format()
        .map_err(|_| ImageProcessorError::UnsupportedInput)?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_INPUT_DIMENSION);
    limits.max_image_height = Some(MAX_INPUT_DIMENSION);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|_| ImageProcessorError::UnsupportedInput)?;
    let image = transform_image(image, transform)?;
    let (width, height) = image.dimensions();
    let content = encode_image(&image, transform)?;
    Ok(ProcessedVariant {
        content,
        mime: transform.format().mime().to_owned(),
        format: transform.format(),
        width,
        height,
    })
}

fn transform_image(
    image: DynamicImage,
    transform: &VariantTransform,
) -> Result<DynamicImage, ImageProcessorError> {
    let (source_width, source_height) = image.dimensions();
    let (width, height) = requested_dimensions(transform, source_width, source_height)?;
    let transformed = match transform.fit() {
        VariantFit::Cover => cover(image, width, height, transform.crop()),
        VariantFit::Contain => contain(image, width, height, transform.background())?,
        VariantFit::Inside => image.resize(width, height, FilterType::Lanczos3),
    };
    if transform.blur() == 0 {
        Ok(transformed)
    } else {
        Ok(transformed.blur(f32::from(transform.blur()) / 4.0))
    }
}

fn requested_dimensions(
    transform: &VariantTransform,
    source_width: u32,
    source_height: u32,
) -> Result<(u32, u32), ImageProcessorError> {
    let dimensions = match (transform.width(), transform.height()) {
        (Some(width), Some(height)) => Ok((width, height)),
        (Some(width), None) => {
            let height = u64::from(source_height)
                .saturating_mul(u64::from(width))
                .checked_div(u64::from(source_width))
                .unwrap_or(0)
                .max(1);
            Ok((
                width,
                u32::try_from(height).map_err(|_| ImageProcessorError::ProcessingFailed)?,
            ))
        }
        (None, Some(height)) => {
            let width = u64::from(source_width)
                .saturating_mul(u64::from(height))
                .checked_div(u64::from(source_height))
                .unwrap_or(0)
                .max(1);
            Ok((
                u32::try_from(width).map_err(|_| ImageProcessorError::ProcessingFailed)?,
                height,
            ))
        }
        (None, None) => Ok((source_width, source_height)),
    }?;
    if dimensions.0 > MAX_VARIANT_DIMENSION
        || dimensions.1 > MAX_VARIANT_DIMENSION
        || u64::from(dimensions.0) * u64::from(dimensions.1) > MAX_VARIANT_OUTPUT_PIXELS
    {
        return Err(ImageProcessorError::OutputTooLarge);
    }
    Ok(dimensions)
}

fn cover(image: DynamicImage, width: u32, height: u32, crop: CropPosition) -> DynamicImage {
    let (source_width, source_height) = image.dimensions();
    let scale = (width as f64 / source_width as f64).max(height as f64 / source_height as f64);
    let resized_width = (source_width as f64 * scale).ceil() as u32;
    let resized_height = (source_height as f64 * scale).ceil() as u32;
    let resized = image.resize_exact(resized_width, resized_height, FilterType::Lanczos3);
    let max_x = resized_width.saturating_sub(width);
    let max_y = resized_height.saturating_sub(height);
    let (x, y) = match crop {
        CropPosition::Center => (max_x / 2, max_y / 2),
        CropPosition::Top => (max_x / 2, 0),
        CropPosition::Bottom => (max_x / 2, max_y),
        CropPosition::Left => (0, max_y / 2),
        CropPosition::Right => (max_x, max_y / 2),
    };
    resized.crop_imm(x, y, width, height)
}

fn contain(
    image: DynamicImage,
    width: u32,
    height: u32,
    background: &str,
) -> Result<DynamicImage, ImageProcessorError> {
    let resized = image.resize(width, height, FilterType::Lanczos3).to_rgba8();
    let color = parse_background(background)?;
    let mut canvas = image::RgbaImage::from_pixel(width, height, color);
    let x = (width - resized.width()) / 2;
    let y = (height - resized.height()) / 2;
    image::imageops::overlay(&mut canvas, &resized, i64::from(x), i64::from(y));
    Ok(DynamicImage::ImageRgba8(canvas))
}

fn parse_background(value: &str) -> Result<Rgba<u8>, ImageProcessorError> {
    let red =
        u8::from_str_radix(&value[0..2], 16).map_err(|_| ImageProcessorError::ProcessingFailed)?;
    let green =
        u8::from_str_radix(&value[2..4], 16).map_err(|_| ImageProcessorError::ProcessingFailed)?;
    let blue =
        u8::from_str_radix(&value[4..6], 16).map_err(|_| ImageProcessorError::ProcessingFailed)?;
    Ok(Rgba([red, green, blue, 255]))
}

fn encode_image(
    image: &DynamicImage,
    transform: &VariantTransform,
) -> Result<Vec<u8>, ImageProcessorError> {
    let mut output = Vec::new();
    match transform.format() {
        VariantFormat::Jpeg => {
            JpegEncoder::new_with_quality(&mut output, transform.quality())
                .encode_image(image)
                .map_err(|_| ImageProcessorError::EncodingFailed)?;
        }
        VariantFormat::Png => image
            .write_to(&mut Cursor::new(&mut output), ImageFormat::Png)
            .map_err(|_| ImageProcessorError::EncodingFailed)?,
        VariantFormat::Webp => {
            let rgba = image.to_rgba8();
            let encoded = webp::Encoder::from_rgba(rgba.as_raw(), rgba.width(), rgba.height())
                .encode_simple(false, f32::from(transform.quality()))
                .map_err(|_| ImageProcessorError::EncodingFailed)?;
            output.extend_from_slice(&encoded);
        }
    }
    Ok(output)
}

