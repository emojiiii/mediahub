// Image processor tests.

    use std::sync::atomic::{AtomicBool, Ordering};

    use image::{ImageBuffer, Rgb, Rgba};
    use mediahub_core::{CropPosition, VariantFit};

    use super::*;

    fn fixture() -> Vec<u8> {
        let image = DynamicImage::ImageRgb8(ImageBuffer::from_fn(8, 4, |x, y| {
            Rgb([
                u8::try_from(x * 10).expect("x"),
                u8::try_from(y * 20).expect("y"),
                40,
            ])
        }));
        let mut encoded = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut encoded), ImageFormat::Png)
            .expect("fixture encoding");
        encoded
    }

    fn quality_fixture() -> Vec<u8> {
        let image = DynamicImage::ImageRgb8(ImageBuffer::from_fn(320, 240, |x, y| {
            Rgb([
                (x.wrapping_mul(31) + y.wrapping_mul(17) + x.wrapping_mul(y)) as u8,
                (x.wrapping_mul(7) + y.wrapping_mul(29) + x.wrapping_mul(y.wrapping_mul(3))) as u8,
                (x.wrapping_mul(19) + y.wrapping_mul(11) + x.wrapping_mul(y.wrapping_mul(5))) as u8,
            ])
        }));
        let mut encoded = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut encoded), ImageFormat::Png)
            .expect("quality fixture encoding");
        encoded
    }

    #[cfg(all(feature = "libvips", target_os = "linux"))]
    fn jpeg_fixture() -> Vec<u8> {
        let image = DynamicImage::ImageRgb8(ImageBuffer::from_fn(32, 20, |x, y| {
            Rgb([
                u8::try_from(x * 7).expect("x"),
                u8::try_from(y * 11).expect("y"),
                u8::try_from((x + y) * 3).expect("x + y"),
            ])
        }));
        let mut encoded = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut encoded), ImageFormat::Jpeg)
            .expect("JPEG fixture encoding");
        encoded
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cpu_work_runs_outside_the_async_runtime_worker() {
        let runtime_progressed = Arc::new(AtomicBool::new(false));
        let marker = Arc::clone(&runtime_progressed);
        let processing = async {
            let result = run_blocking(|| {
                std::thread::sleep(std::time::Duration::from_millis(50));
                Ok(())
            })
            .await;
            assert!(runtime_progressed.load(Ordering::SeqCst));
            result
        };
        let runtime_task = async move {
            marker.store(true, Ordering::SeqCst);
        };

        let (result, ()) = tokio::join!(processing, runtime_task);
        assert_eq!(result, Ok(()));
    }

    #[tokio::test]
    async fn timed_out_work_retains_its_blocking_capacity_until_it_stops() {
        let slots = Arc::new(tokio::sync::Semaphore::new(1));
        let result = run_blocking_with_slots(Duration::from_millis(5), Arc::clone(&slots), || {
            std::thread::sleep(Duration::from_millis(50));
            Ok(())
        })
        .await;
        assert_eq!(result, Err(ImageProcessorError::ProcessingFailed));

        let second_started = Arc::new(AtomicBool::new(false));
        let marker = Arc::clone(&second_started);
        let second =
            run_blocking_with_slots(Duration::from_millis(5), Arc::clone(&slots), move || {
                marker.store(true, Ordering::SeqCst);
                Ok(())
            })
            .await;
        assert_eq!(second, Err(ImageProcessorError::ProcessingFailed));
        assert!(!second_started.load(Ordering::SeqCst));

        tokio::time::sleep(Duration::from_millis(60)).await;
        run_blocking_with_slots(Duration::from_millis(5), slots, || Ok(()))
            .await
            .expect("capacity returns after the original task stops");
    }

    #[tokio::test]
    async fn blocking_processing_has_a_request_deadline() {
        let result = run_blocking_with_timeout(Duration::from_millis(5), || {
            std::thread::sleep(Duration::from_millis(50));
            Ok(())
        })
        .await;

        assert_eq!(result, Err(ImageProcessorError::ProcessingFailed));
    }

    #[tokio::test]
    async fn cover_generates_exact_bounded_dimensions() {
        let transform = VariantTransform::new(
            Some(3),
            Some(3),
            VariantFit::Cover,
            80,
            VariantFormat::Png,
            0,
            CropPosition::Center,
            "ffffff",
        )
        .expect("transform");
        let result = RustImageProcessor
            .process(&fixture(), &transform)
            .await
            .expect("variant");
        assert_eq!((result.width, result.height), (3, 3));
        assert_eq!(
            image::load_from_memory(&result.content)
                .expect("decode")
                .dimensions(),
            (3, 3)
        );
    }

    #[tokio::test]
    async fn format_only_transform_preserves_source_dimensions() {
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
        .expect("format-only transform");
        let result = RustImageProcessor
            .process(&fixture(), &transform)
            .await
            .expect("format-only variant");

        assert_eq!((result.width, result.height), (8, 4));
        assert_eq!(result.mime, "image/webp");
        assert_eq!(
            image::load_from_memory(&result.content)
                .expect("decode format-only output")
                .dimensions(),
            (8, 4)
        );
    }

    #[tokio::test]
    async fn webp_quality_changes_lossy_output_size() {
        assert!(
            RustImageProcessor
                .processor_version()
                .contains("libwebp-lossy-v1")
        );
        let input = quality_fixture();
        let encode = |quality| {
            VariantTransform::new(
                None,
                None,
                VariantFit::Inside,
                quality,
                VariantFormat::Webp,
                0,
                CropPosition::Center,
                "ffffff",
            )
            .expect("WebP transform")
        };
        let high = RustImageProcessor
            .process(&input, &encode(90))
            .await
            .expect("high-quality WebP");
        let low = RustImageProcessor
            .process(&input, &encode(30))
            .await
            .expect("low-quality WebP");

        assert!(
            low.content.len() < high.content.len(),
            "low quality should reduce output size: low={}, high={}",
            low.content.len(),
            high.content.len()
        );
        assert_ne!(low.content, high.content);
        for content in [&low.content, &high.content] {
            assert_eq!(
                image::load_from_memory(content)
                    .expect("decode lossy WebP")
                    .dimensions(),
                (320, 240)
            );
        }
    }

    #[tokio::test]
    async fn output_formats_encode_valid_images() {
        for format in [VariantFormat::Jpeg, VariantFormat::Png, VariantFormat::Webp] {
            let transform = VariantTransform::new(
                Some(4),
                Some(4),
                VariantFit::Contain,
                75,
                format,
                2,
                CropPosition::Center,
                "112233",
            )
            .expect("transform");
            let result = RustImageProcessor
                .process(&fixture(), &transform)
                .await
                .expect("variant");
            assert!(!result.content.is_empty());
            assert_eq!(result.mime, format.mime());
        }
    }

    #[cfg(all(feature = "libvips", target_os = "linux"))]
    #[tokio::test]
    async fn libvips_processor_generates_exact_decodable_outputs() {
        for format in [VariantFormat::Jpeg, VariantFormat::Png, VariantFormat::Webp] {
            let transform = VariantTransform::new(
                Some(4),
                Some(4),
                VariantFit::Contain,
                75,
                format,
                1,
                CropPosition::Center,
                "112233",
            )
            .expect("transform");
            let result = VipsImageProcessor
                .process(&fixture(), &transform)
                .await
                .expect("libvips variant");
            assert_eq!((result.width, result.height), (4, 4));
            assert_eq!(result.mime, format.mime());
            assert_eq!(
                image::load_from_memory(&result.content)
                    .expect("decode libvips output")
                    .dimensions(),
                (4, 4)
            );
        }
        assert!(
            VipsImageProcessor
                .processor_version()
                .contains("libvips-8.18.4-binding-2.3.0")
        );

        let inside = VariantTransform::new(
            Some(4),
            Some(4),
            VariantFit::Inside,
            75,
            VariantFormat::Png,
            0,
            CropPosition::Center,
            "ffffff",
        )
        .expect("inside transform");
        let result = VipsImageProcessor
            .process(&fixture(), &inside)
            .await
            .expect("inside libvips variant");
        assert_eq!((result.width, result.height), (4, 2));
        assert_eq!(
            image::load_from_memory(&result.content)
                .expect("decode inside libvips output")
                .dimensions(),
            (4, 2)
        );
    }

    #[cfg(all(feature = "libvips", target_os = "linux"))]
    #[tokio::test]
    async fn libvips_processor_transcodes_jpeg_to_jpeg_and_webp() {
        let input = jpeg_fixture();
        for format in [VariantFormat::Jpeg, VariantFormat::Webp] {
            let transform = VariantTransform::new(
                None,
                None,
                VariantFit::Inside,
                75,
                format,
                0,
                CropPosition::Center,
                "ffffff",
            )
            .expect("transform");
            let result = VipsImageProcessor
                .process(&input, &transform)
                .await
                .expect("JPEG transcode");

            assert_eq!((result.width, result.height), (32, 20));
            assert_eq!(result.mime, format.mime());
            assert_eq!(
                image::load_from_memory(&result.content)
                    .expect("decode transcoded JPEG")
                    .dimensions(),
                (32, 20)
            );
        }
    }

    #[tokio::test]
    async fn invalid_or_oversized_input_is_rejected() {
        let transform = VariantTransform::new(
            Some(4),
            Some(4),
            VariantFit::Inside,
            80,
            VariantFormat::Png,
            0,
            CropPosition::Center,
            "ffffff",
        )
        .expect("transform");
        assert_eq!(
            RustImageProcessor
                .process(b"not an image", &transform)
                .await,
            Err(ImageProcessorError::UnsupportedInput)
        );
        assert_eq!(
            RustImageProcessor
                .process(&vec![0; MAX_INPUT_BYTES + 1], &transform)
                .await,
            Err(ImageProcessorError::InputTooLarge)
        );
    }

    #[tokio::test]
    async fn inferred_dimension_cannot_bypass_output_limits() {
        let image = DynamicImage::ImageRgb8(ImageBuffer::from_pixel(1, 32, Rgb([1, 2, 3])));
        let mut encoded = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut encoded), ImageFormat::Png)
            .expect("fixture encoding");
        let transform = VariantTransform::new(
            Some(MAX_VARIANT_DIMENSION),
            None,
            VariantFit::Inside,
            80,
            VariantFormat::Webp,
            0,
            CropPosition::Center,
            "ffffff",
        )
        .expect("transform");
        let result = RustImageProcessor.process(&encoded, &transform).await;
        assert_eq!(result, Err(ImageProcessorError::OutputTooLarge));
    }

    #[test]
    fn cover_rejects_unbounded_intermediate_dimensions() {
        let image = DynamicImage::ImageRgb8(ImageBuffer::from_pixel(16_384, 1, Rgb([1, 2, 3])));
        assert_eq!(
            cover(image, 4_096, 4_096, CropPosition::Center),
            Err(ImageProcessorError::OutputTooLarge)
        );
    }

    #[tokio::test]
    async fn rgba16_cover_respects_intermediate_byte_limit() {
        let image = DynamicImage::ImageRgba16(ImageBuffer::from_pixel(
            1_024,
            256,
            Rgba([1_u16, 2, 3, u16::MAX]),
        ));
        let mut encoded = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut encoded), ImageFormat::Png)
            .expect("16-bit PNG fixture encoding");
        let transform = VariantTransform::new(
            Some(MAX_VARIANT_DIMENSION),
            Some(MAX_VARIANT_DIMENSION),
            VariantFit::Cover,
            80,
            VariantFormat::Png,
            0,
            CropPosition::Center,
            "ffffff",
        )
        .expect("transform");

        assert_eq!(
            RustImageProcessor.process(&encoded, &transform).await,
            Err(ImageProcessorError::OutputTooLarge)
        );
    }
