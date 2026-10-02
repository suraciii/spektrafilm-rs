use spektrafilm_core::image_io::{self, BitDepth, SaveOptions};
use spektrafilm_math::{image::ImageBuf, precision::{from_f64, to_f64}};

#[test]
fn integer_boundary_truncates_and_float_formats_preserve_headroom() {
    let directory = std::env::temp_dir().join(format!("spektrafilm-io-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let samples = [-0.25, 0.5, 1.25, 0.1, 0.499, 0.0];
    let image = ImageBuf::from_data(2, 1, samples.iter().copied().map(from_f64).collect());
    for (extension, depth, scale) in [("png", BitDepth::ThirtyTwo, Some(255.0)), ("tif", BitDepth::Eight, Some(255.0)), ("tif", BitDepth::Sixteen, Some(65535.0)), ("tif", BitDepth::ThirtyTwo, None), ("exr", BitDepth::ThirtyTwo, None), ("exr", BitDepth::Sixteen, None)] {
        let path = directory.join(format!("{}.{extension}", depth.bits()));
        image_io::save(&path, &image, SaveOptions { depth, color_space: "sRGB", cctf_encoding: true }, None).unwrap();
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
    assert!(matches!(image_io::save(&unsupported, &image, SaveOptions { depth: BitDepth::Eight, color_space: "sRGB", cctf_encoding: false }, None), Err(image_io::ImageIoError::ExrDepth(8))));
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

