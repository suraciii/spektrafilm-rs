use spektrafilm_core::image_io::{self, BitDepth, SaveOptions};
use spektrafilm_math::{
    image::ImageBuf,
    precision::{from_f64, to_f64},
};

#[test]
fn integer_boundary_truncates_and_float_formats_preserve_headroom() {
    let directory = std::env::temp_dir().join(format!("spektrafilm-io-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let samples = [-0.25, 0.5, 1.25, 0.1, 0.499, 0.0];
    let image = ImageBuf::from_data(2, 1, samples.iter().copied().map(from_f64).collect());
    for (extension, depth, scale) in [
        ("png", BitDepth::Eight, Some(255.0)),
        ("tif", BitDepth::Eight, Some(255.0)),
        ("tif", BitDepth::Sixteen, Some(65535.0)),
        ("tif", BitDepth::ThirtyTwo, None),
        ("exr", BitDepth::ThirtyTwo, None),
        ("exr", BitDepth::Sixteen, None),
    ] {
        let path = directory.join(format!("{}.{extension}", depth.bits()));
        image_io::save(
            &path,
            &image,
            SaveOptions {
                depth,
                color_space: "sRGB",
                cctf_encoding: extension != "exr",
                jpeg_quality: None,
                jpeg_subsampling: None,
                compression: None,
            },
            None,
        )
        .unwrap();
        let decoded = image_io::load(&path).unwrap().image;
        assert_eq!((decoded.width, decoded.height), (2, 1));
        if let Some(scale) = scale {
            for (&actual, &input) in decoded.data.iter().zip(image.data.iter()) {
                let expected = (to_f64(input).clamp(0.0, 1.0) * scale).trunc() / scale;
                assert!((to_f64(actual) - expected).abs() < 1e-7);
            }
        } else {
            assert_eq!(to_f64(decoded.data[0]), -0.25);
            assert_eq!(to_f64(decoded.data[2]), 1.25);
        }
    }
    let unsupported = directory.join("unsupported.exr");
    assert!(matches!(
        image_io::save(
            &unsupported,
            &image,
            SaveOptions {
                depth: BitDepth::Eight,
                color_space: "sRGB",
                cctf_encoding: false,
                jpeg_quality: None,
                jpeg_subsampling: None,
                compression: None,
            },
            None
        ),
        Err(image_io::ImageIoError::ExrDepth(8))
    ));
    assert!(!unsupported.exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn saving_into_the_output_space_and_encoding_is_a_bit_exact_no_op() {
    // The 4-digit IEC sRGB matrices are not exact inverses of each other, so a
    // same-space round trip shifts pixels by ~2e-5. The pinned Python save path
    // skips the transform when the saving space and encoding already match the
    // output layer; the export must do the same.
    let samples = [0.0, 0.09846869, 0.34267145, 1.0, 0.6001501387, 0.019607843];
    let image = ImageBuf::from_data(2, 1, samples.iter().copied().map(from_f64).collect());
    for encoded in [false, true] {
        let converted = image_io::convert_image(&image, "sRGB", encoded, "sRGB", encoded).unwrap();
        assert_eq!(converted.data, image.data);
    }
}

#[test]
fn jpeg_quality_controls_decoded_detail() {
    let directory =
        std::env::temp_dir().join(format!("spektrafilm-jpeg-quality-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    // Monochrome texture isolates quantization from chroma subsampling.
    let image = ImageBuf::from_data(
        64,
        64,
        (0..64 * 64)
            .flat_map(|i| [from_f64(((i * 73 + i / 64 * 19) % 256) as f64 / 255.0); 3])
            .collect(),
    );
    let mut errors = Vec::new();
    for quality in [20, 100] {
        let path = directory.join(format!("{quality}.jpg"));
        image_io::save_jpeg_quality(
            &path,
            &image,
            SaveOptions {
                depth: BitDepth::Eight,
                color_space: "sRGB",
                cctf_encoding: true,
                jpeg_quality: None,
                jpeg_subsampling: None,
                compression: None,
            },
            None,
            quality,
        )
        .unwrap();
        let decoded = image_io::load(&path).unwrap().image;
        assert_eq!((decoded.width, decoded.height), (64, 64));
        errors.push(
            decoded
                .data
                .iter()
                .zip(&image.data)
                .map(|(&a, &b)| (to_f64(a) - to_f64(b)).powi(2))
                .sum::<f64>()
                / image.data.len() as f64,
        );
    }
    assert!(
        errors[1] < errors[0] * 0.1,
        "quality 100 must retain more texture: {errors:?}"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[cfg(feature = "precision-f64")]
#[test]
fn exr_half_rounds_directly_from_f64_and_float32_exports_keep_their_precision() {
    // Expected values follow numpy's direct float64 -> float16 conversion.
    let midpoint = 1.00048828125;
    let adjacent_midpoint = 1.00146484375;
    let half_subnormal = 2.0_f64.powi(-25);
    let min_subnormal = 2.0_f64.powi(-24);
    let min_normal = 2.0_f64.powi(-14);
    let epsilon = 2.0_f64.powi(-60);
    let cases = [
        (midpoint + 1e-10, 1.0009765625),
        (midpoint, 1.0),
        (midpoint - 1e-10, 1.0),
        (-midpoint - 1e-10, -1.0009765625),
        (-midpoint, -1.0),
        (-midpoint + 1e-10, -1.0),
        (adjacent_midpoint, 1.001953125),
        (adjacent_midpoint - 1e-10, 1.0009765625),
        (adjacent_midpoint + 1e-10, 1.001953125),
        (-adjacent_midpoint, -1.001953125),
        (65504.0, 65504.0),
        (65520.0 - 1e-6, 65504.0),
        (65520.0, f64::INFINITY),
        (65520.0 + 1e-6, f64::INFINITY),
        (-65520.0 + 1e-6, -65504.0),
        (-65520.0, f64::NEG_INFINITY),
        (half_subnormal - epsilon, 0.0),
        (half_subnormal, 0.0),
        (half_subnormal + epsilon, min_subnormal),
        (-half_subnormal, -0.0),
        (-half_subnormal - epsilon, -min_subnormal),
        (3.0 * half_subnormal, 2.0 * min_subnormal),
        (min_normal - half_subnormal, min_normal),
        (
            min_normal - half_subnormal - epsilon,
            min_normal - min_subnormal,
        ),
        (f64::MIN_POSITIVE, 0.0),
        (f64::from_bits(1), 0.0),
        (0.0, 0.0),
        (-0.0, -0.0),
        (f64::INFINITY, f64::INFINITY),
        (f64::NEG_INFINITY, f64::NEG_INFINITY),
        (f64::NAN, f64::NAN),
    ];
    let image = ImageBuf::from_data(
        cases.len() as u32,
        1,
        cases.iter().flat_map(|&(input, _)| [input; 3]).collect(),
    );
    let directory =
        std::env::temp_dir().join(format!("spektrafilm-half-io-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    for (extension, depth) in [
        ("exr", BitDepth::Sixteen),
        ("exr", BitDepth::ThirtyTwo),
        ("tif", BitDepth::ThirtyTwo),
    ] {
        let path = directory.join(format!("{}.{}", depth.bits(), extension));
        image_io::save(
            &path,
            &image,
            SaveOptions {
                depth,
                color_space: "sRGB",
                cctf_encoding: false,
                jpeg_quality: None,
                jpeg_subsampling: None,
                compression: None,
            },
            None,
        )
        .unwrap();
        let decoded = image_io::load(&path).unwrap().image;
        assert_eq!((decoded.width, decoded.height), (cases.len() as u32, 1));
        for (pixel, &(input, half_expected)) in decoded.data.chunks_exact(3).zip(&cases) {
            let expected = if depth == BitDepth::Sixteen {
                half_expected
            } else {
                input as f32 as f64
            };
            for &actual in pixel {
                if expected.is_nan() {
                    assert!(
                        actual.is_nan(),
                        "{extension} {depth:?}: {input:?} must preserve NaN"
                    );
                } else {
                    assert_eq!(
                        actual.to_bits(),
                        expected.to_bits(),
                        "{extension} {depth:?}: input {input:?}, actual {actual:?}, expected {expected:?}"
                    );
                }
            }
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn export_settings_reject_format_mismatches() {
    let directory = std::env::temp_dir().join(format!(
        "spektrafilm-export-settings-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let image = ImageBuf::from_data(1, 1, vec![from_f64(0.25); 3]);
    let png = directory.join("invalid.png");
    assert!(matches!(
        image_io::save(
            &png,
            &image,
            SaveOptions {
                depth: BitDepth::Sixteen,
                color_space: "sRGB",
                cctf_encoding: true,
                jpeg_quality: None,
                jpeg_subsampling: None,
                compression: None,
            },
            None
        ),
        Err(image_io::ImageIoError::InvalidExport { .. })
    ));
    let png_quality = directory.join("invalid-quality.png");
    assert!(matches!(
        image_io::save(
            &png_quality,
            &image,
            SaveOptions {
                depth: BitDepth::Eight,
                color_space: "sRGB",
                cctf_encoding: true,
                jpeg_quality: Some(95),
                jpeg_subsampling: None,
                compression: None,
            },
            None
        ),
        Err(image_io::ImageIoError::InvalidExport { .. })
    ));
    let exr = directory.join("invalid.exr");
    assert!(matches!(
        image_io::save(
            &exr,
            &image,
            SaveOptions {
                depth: BitDepth::Sixteen,
                color_space: "sRGB",
                cctf_encoding: true,
                jpeg_quality: None,
                jpeg_subsampling: None,
                compression: Some(spektrafilm_core::image_io::Compression::Zip),
            },
            None
        ),
        Err(image_io::ImageIoError::InvalidExport { .. })
    ));
    assert!(!png.exists());
    assert!(!png_quality.exists());
    assert!(!exr.exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn rendered_output_preserves_selected_encoding_and_float_samples() {
    let directory = std::env::temp_dir().join(format!("spektrafilm-rendered-save-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let samples = [-0.25, 0.5, 1.25];
    let image = ImageBuf::from_data(1, 1, samples.map(from_f64).to_vec());
    for (extension, depth, encoded) in [
        ("exr", BitDepth::Sixteen, true),
        ("exr", BitDepth::ThirtyTwo, true),
        ("png", BitDepth::Eight, false),
        ("tif", BitDepth::ThirtyTwo, false),
    ] {
        let path = directory.join(format!("{}.{extension}", depth.bits()));
        image_io::save_rendered_output(&path, &image, SaveOptions {
            depth, color_space: "Display P3", cctf_encoding: encoded,
            jpeg_quality: None, jpeg_subsampling: None, compression: None,
        }, None).unwrap();
        let decoded = image_io::load(&path).unwrap().image;
        assert_eq!((decoded.width, decoded.height), (1, 1));
        for (&actual, expected) in decoded.data.iter().zip(samples) {
            let expected = if extension == "png" { (expected.clamp(0.0, 1.0) * 255.0).trunc() / 255.0 } else { expected };
            assert!((to_f64(actual) - expected).abs() < 1e-7);
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}
