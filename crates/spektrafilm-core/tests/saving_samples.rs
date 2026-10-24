//! Saving sample conversion at the real writer boundary.
//!
//! Two contracts are exercised here:
//!
//! * samples the device already converted are packed by the native writer
//!   without clipping, transfer encoding, or re-quantization (the exact byte
//!   a converted sample carries is the byte the container stores), and
//! * the device saving conversion reproduces the host reference at the
//!   integer and binary16 boundaries, including clipping, truncation,
//!   ties-to-even, subnormals, and NaN, for PNG, JPEG, TIFF (8/16/32), and
//!   OpenEXR (half/float).
//!
//! The device cases skip when no adapter initializes, exactly like the other
//! device tests; they are not a substitute for hardware acceptance.

use spektrafilm_backend::telemetry::CollectionMode;
use spektrafilm_backend::{
    ComputeBackend, DeviceImage, OutputSampleData, OutputSampleKind, OutputSampleValues,
    Precision, RetainedImageMeta, SampleDomain, telemetry::ObservationContext,
};
use spektrafilm_core::image_io::{self, BitDepth, ImageFormat, SaveOptions};
use spektrafilm_math::colorspace;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{from_f64, to_f64};
use std::path::{Path, PathBuf};

fn context() -> ObservationContext {
    ObservationContext::new(CollectionMode::Off)
}

fn directory(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("spektrafilm-save-samples-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn options(format: ImageFormat, depth: BitDepth, encoded: bool) -> SaveOptions<'static> {
    SaveOptions {
        depth,
        color_space: "sRGB",
        cctf_encoding: encoded,
        jpeg_quality: (format == ImageFormat::Jpeg).then_some(95),
        jpeg_subsampling: (format == ImageFormat::Jpeg).then_some(image_io::JpegSubsampling::Yuv444),
        compression: matches!(format, ImageFormat::Tiff | ImageFormat::Exr)
            .then_some(image_io::Compression::Zip),
    }
}

fn publish(path: &Path, samples: &OutputSampleData, depth: BitDepth, encoded: bool) {
    let format = ImageFormat::detect(path).unwrap();
    image_io::prepare_save_samples_observed(
        path,
        samples,
        options(format, depth, encoded),
        None,
        &context(),
    )
    .unwrap()
    .publish(false)
    .unwrap();
}

fn load_samples(path: &Path) -> Vec<f64> {
    image_io::load(path).unwrap().image.data.iter().copied().map(to_f64).collect()
}

fn samples(data: &[f64], width: u32, height: u32) -> ImageBuf {
    ImageBuf::from_data(width, height, data.iter().copied().map(from_f64).collect())
}

/// The writer receives samples that are already final: an integer payload is
/// stored verbatim, and a float payload keeps its sign and headroom.
#[test]
fn converted_samples_are_packed_without_a_second_conversion() {
    let directory = directory("pack");
    let u8_payload = OutputSampleData {
        width: 2,
        height: 1,
        kind: OutputSampleKind::U8,
        values: OutputSampleValues::U8(vec![0, 1, 200, 255, 128, 64]),
    };
    let png = directory.join("exact.png");
    publish(&png, &u8_payload, BitDepth::Eight, true);
    let decoded = load_samples(&png);
    for (actual, expected) in decoded.iter().zip([0.0, 1.0, 200.0, 255.0, 128.0, 64.0]) {
        assert_eq!(
            (actual * 255.0).round(),
            expected,
            "PNG re-applied a conversion to converted samples"
        );
    }

    let tiff8 = directory.join("exact-8.tif");
    publish(&tiff8, &u8_payload, BitDepth::Eight, true);
    assert_eq!(load_samples(&tiff8), decoded);

    // JPEG is lossy, so only the container contract is asserted: converted
    // samples reach the encoder and decode inside [0,1].
    let jpeg = directory.join("exact.jpg");
    publish(&jpeg, &u8_payload, BitDepth::Eight, true);
    let jpeg_samples = load_samples(&jpeg);
    assert_eq!(jpeg_samples.len(), 6);
    assert!(jpeg_samples.iter().all(|value| (0.0..=1.0).contains(value)));

    let u16_payload = OutputSampleData {
        width: 2,
        height: 1,
        kind: OutputSampleKind::U16,
        values: OutputSampleValues::U16(vec![0, 1, 65535, 32768, 12345, 256]),
    };
    let tiff16 = directory.join("exact-16.tif");
    publish(&tiff16, &u16_payload, BitDepth::Sixteen, true);
    let decoded = load_samples(&tiff16);
    for (actual, expected) in decoded
        .iter()
        .zip([0.0, 1.0, 65535.0, 32768.0, 12345.0, 256.0])
    {
        assert_eq!((actual * 65535.0).round(), expected);
    }

    // Floating output keeps negative and super-white samples: a writer that
    // re-clipped would lose both.
    let f32_payload = OutputSampleData {
        width: 2,
        height: 1,
        kind: OutputSampleKind::F32,
        values: OutputSampleValues::F32(vec![-0.5, 2.0, 0.0, 1.0, 0.25, 1.5]),
    };
    let tiff32 = directory.join("headroom.tif");
    publish(&tiff32, &f32_payload, BitDepth::ThirtyTwo, false);
    let decoded = load_samples(&tiff32);
    for (actual, expected) in decoded.iter().zip([-0.5, 2.0, 0.0, 1.0, 0.25, 1.5]) {
        assert_eq!(*actual, expected);
    }

    // Half payloads are raw IEEE-754 binary16 bit patterns; the writer must
    // store the bits rather than convert through float again.
    let half_payload = OutputSampleData {
        width: 2,
        height: 1,
        kind: OutputSampleKind::Half,
        values: OutputSampleValues::Half(vec![
            0x3800, 0x3c00, 0xb800, 0x3400, 0x0000, 0x3555,
        ]),
    };
    let exr = directory.join("half.exr");
    publish(&exr, &half_payload, BitDepth::Sixteen, false);
    let decoded = load_samples(&exr);
    for (actual, expected) in decoded
        .iter()
        .zip([0.5, 1.0, -0.5, 0.25, 0.0, f64::from(half_value(0x3555))])
    {
        assert_eq!(*actual, expected);
    }

    let _ = std::fs::remove_dir_all(directory);
}

/// A converted payload whose representation the container cannot hold fails
/// before publication instead of being re-converted silently.
#[test]
fn mismatched_converted_samples_fail_before_publication() {
    let directory = directory("mismatch");
    let path = directory.join("mismatch.tif");
    let payload = OutputSampleData {
        width: 1,
        height: 1,
        kind: OutputSampleKind::U8,
        values: OutputSampleValues::U8(vec![1, 2, 3]),
    };
    let error = image_io::prepare_save_samples_observed(
        &path,
        &payload,
        options(ImageFormat::Tiff, BitDepth::Sixteen, true),
        None,
        &context(),
    )
    .unwrap_err();
    assert!(matches!(error, image_io::ImageIoError::SampleKind { .. }));
    assert!(!path.exists());

    let exr = directory.join("mismatch.exr");
    let error = image_io::prepare_save_samples_observed(
        &exr,
        &payload,
        options(ImageFormat::Exr, BitDepth::ThirtyTwo, false),
        None,
        &context(),
    )
    .unwrap_err();
    assert!(matches!(error, image_io::ImageIoError::SampleKind { .. }));
    assert!(!exr.exists());
    let _ = std::fs::remove_dir_all(directory);
}

/// The device conversion must select the same samples as the host reference:
/// integer clipping truncates, NaN becomes zero for integer containers, and
/// binary16 rounds ties-to-even with subnormals and infinities preserved.
#[test]
fn device_saving_conversion_matches_the_reference_boundaries() {
    let gpu = spektrafilm_gpu::test_support::explicit_adapter();
    let backend = gpu.backend();
    let directory = directory("device");

    // 4 pixels: negative, half, super-white, and the values around the
    // integer truncation and half rounding boundaries.
    let values = [
        -0.25, 0.5, 1.25, // pixel 0
        0.0, 1.0, 0.1, // pixel 1
        0.499, 0.25, 0.75, // pixel 2
        0.0031308, 0.04045, 0.5, // pixel 3: sRGB transfer boundary
    ];
    let image = samples(&values, 4, 1);
    let cpu = spektrafilm_backend::cpu_backend::CpuBackend;

    // The same-space guard keeps the colour transform out while forcing the
    // sample conversion, so the expected values are pure clipping.
    for kind in [
        OutputSampleKind::U8,
        OutputSampleKind::U16,
        OutputSampleKind::F32,
    ] {
        let host = image_io::convert_samples_with_backend(
            &image, "sRGB", true, "sRGB", true, kind, &cpu,
        )
        .unwrap();
        let device = image_io::convert_samples_with_backend(
            &image, "sRGB", true, "sRGB", true, kind, backend,
        )
        .unwrap();
        let (host, device) = match (&host, &device) {
            (image_io::SavingPixels::Reference(_), image_io::SavingPixels::Converted(d)) => {
                (expected_packing(&values, kind), d.clone())
            }
            _ => panic!("device saving conversion did not run on the device"),
        };
        assert_eq!(
            device.values, host,
            "{kind:?} device packing differs from the reference"
        );
    }

    // sRGB encoded → encoded is the pinned no-op colour path, so these exact
    // values reach the half conversion unchanged.
    let exact = [
        0.5,
        0.25,
        -0.5,
        f64::from(f32::from_bits(0x3f80_2000)), // 1.0009765625
        2f64.powi(-25),                        // half subnormal midpoint → +0
        70000.0,                               // overflows to +inf
        f64::NAN,
        1.0,
        0.0,
        0.75,
        1.5,
        -1.0,
    ];
    let image = samples(&exact, 4, 1);
    let device = image_io::convert_samples_with_backend(
        &image,
        "sRGB",
        true,
        "sRGB",
        true,
        OutputSampleKind::Half,
        backend,
    )
    .unwrap();
    let device = match device {
        image_io::SavingPixels::Converted(data) => data,
        image_io::SavingPixels::Reference(_) => panic!("half conversion ran on the host"),
    };
    let OutputSampleValues::Half(bits) = &device.values else {
        panic!("half conversion returned another sample kind");
    };
    assert_eq!(bits[0], 0x3800, "0.5");
    assert_eq!(bits[1], 0x3400, "0.25");
    assert_eq!(bits[2], 0xb800, "-0.5");
    assert_eq!(bits[4], 0x0000, "2^-25 ties to zero");
    assert_eq!(bits[5], 0x7c00, "70000 overflows to +inf");
    // A half NaN is all-ones exponent with a nonzero payload. Reinterpreting
    // the shifted half bits as f32 cannot see that: its exponent field lands
    // at 252, so the check must read the half bits themselves.
    assert_eq!(bits[6] & 0x7c00, 0x7c00, "NaN exponent");
    assert_ne!(bits[6] & 0x03ff, 0, "NaN payload");
    assert_eq!(bits[7], 0x3c00, "1.0");
    assert_eq!(bits[8], 0x0000, "0.0");
    assert_eq!(bits[9], 0x3a00, "0.75");
    assert_eq!(bits[10], 0x3e00, "1.5");
    assert_eq!(bits[11], 0xbc00, "-1.0");

    // The published EXR carries those exact half values and rejects a writer
    // that re-quantizes them.
    let exr = directory.join("device-half.exr");
    publish(&exr, &device, BitDepth::Sixteen, false);
    let decoded = load_samples(&exr);
    assert_eq!(decoded[0], 0.5);
    assert_eq!(decoded[1], 0.25);
    assert_eq!(decoded[2], -0.5);
    assert_eq!(decoded[4], 0.0);
    assert!(decoded[5].is_infinite());
    assert!(decoded[6].is_nan());
    assert_eq!(decoded[7], 1.0);

    let _ = std::fs::remove_dir_all(directory);
}

/// A retained device carrier converts without a host round trip, and its
/// samples match the host hook for the same image and request.
#[test]
fn retained_output_is_converted_on_the_device() {
    let gpu = spektrafilm_gpu::test_support::explicit_adapter();
    let backend = gpu.backend();
    let values = [-0.25, 0.5, 1.25, 0.1, 0.499, 0.0, 0.25, 0.75, 1.0];
    let image = samples(&values, 3, 1);
    let retained = backend
        .retain_image(
            &image,
            &RetainedImageMeta {
                width: 3,
                height: 1,
                domain: SampleDomain::Encoded,
                precision: Precision::F32,
                space: &colorspace::SRGB,
                pixel_pitch_um: None,
            },
        )
        .unwrap();
    assert!(retained.is_retained());

    for kind in [
        OutputSampleKind::U8,
        OutputSampleKind::U16,
        OutputSampleKind::Half,
        OutputSampleKind::F32,
    ] {
        let converted = image_io::convert_retained_samples_with_backend(
            &retained,
            "sRGB",
            true,
            kind,
            backend,
        )
        .unwrap();
        let converted = match converted {
            image_io::SavingPixels::Converted(data) => data,
            image_io::SavingPixels::Reference(_) => panic!("retained samples came back to the host"),
        };
        let host =
            image_io::convert_samples_with_backend(&image, "sRGB", true, "sRGB", true, kind, backend)
                .unwrap();
        let host = match host {
            image_io::SavingPixels::Converted(data) => data,
            image_io::SavingPixels::Reference(_) => panic!("host hook did not use the device"),
        };
        assert_eq!(converted.values, host.values, "{kind:?}");
    }

    // A host carrier is not silently treated as retained.
    let host = DeviceImage::host(
        image,
        RetainedImageMeta {
            width: 3,
            height: 1,
            domain: SampleDomain::Encoded,
            precision: Precision::host(),
            space: &colorspace::SRGB,
            pixel_pitch_um: None,
        },
    );
    let converted = image_io::convert_retained_samples_with_backend(
        &host,
        "sRGB",
        true,
        OutputSampleKind::U8,
        backend,
    )
    .unwrap();
    assert!(matches!(converted, image_io::SavingPixels::Converted(_)));
}

/// A selected GPU backend without a saving sample kernel must fail the save,
/// not quietly run the host conversion.
#[test]
fn selected_gpu_without_a_sample_kernel_fails() {
    struct GpuWithoutSamples(spektrafilm_backend::cpu_backend::CpuBackend);

    impl ComputeBackend for GpuWithoutSamples {
        fn colorspace_convert(&self, image: &ImageBuf, matrix: &[[f32; 3]; 3]) -> Result<ImageBuf, String> { self.0.colorspace_convert(image, matrix) }
        fn cctf_encode_srgb(&self, image: &ImageBuf) -> Result<ImageBuf, String> { self.0.cctf_encode_srgb(image) }
        fn cctf_decode_srgb(&self, image: &ImageBuf) -> Result<ImageBuf, String> { self.0.cctf_decode_srgb(image) }
        fn gaussian_blur(&self, image: &ImageBuf, sigma: f32) -> Result<ImageBuf, String> { self.0.gaussian_blur(image, sigma) }
        fn table_lookup(&self, image: &ImageBuf, x: &[f32], y: &[[f32; 3]]) -> Result<ImageBuf, String> { self.0.table_lookup(image, x, y) }
        fn lut3d_interp(&self, image: &ImageBuf, lut: &spektrafilm_backend::Lut3D) -> Result<ImageBuf, String> { self.0.lut3d_interp(image, lut) }
        fn is_gpu(&self) -> bool {
            true
        }
        fn explicit_gpu_request(&self) -> bool {
            true
        }
        fn name(&self) -> &str {
            "test GPU without saving sample conversion"
        }
    }

    let backend = GpuWithoutSamples(spektrafilm_backend::cpu_backend::CpuBackend);
    let image = samples(&[0.5, 0.5, 0.5], 1, 1);
    let error = image_io::convert_samples_with_backend(
        &image,
        "sRGB",
        true,
        "sRGB",
        true,
        OutputSampleKind::U8,
        &backend,
    )
    .err()
    .expect("a selected GPU backend must not demote the saving conversion");
    assert!(matches!(error, image_io::ImageIoError::Convert(_)));
}

/// Reference packing for the integer and float kinds, in the same order the
/// writer's native path uses.
fn expected_packing(values: &[f64], kind: OutputSampleKind) -> OutputSampleValues {
    match kind {
        OutputSampleKind::U8 => OutputSampleValues::U8(
            values
                .iter()
                .map(|value| {
                    if value.is_nan() {
                        0
                    } else {
                        (value.clamp(0.0, 1.0) * 255.0) as u8
                    }
                })
                .collect(),
        ),
        OutputSampleKind::U16 => OutputSampleValues::U16(
            values
                .iter()
                .map(|value| {
                    if value.is_nan() {
                        0
                    } else {
                        (value.clamp(0.0, 1.0) * 65535.0) as u16
                    }
                })
                .collect(),
        ),
        OutputSampleKind::F32 => OutputSampleValues::F32(
            values.iter().map(|value| *value as f32).collect::<Vec<_>>(),
        ),
        OutputSampleKind::Half => unreachable!("half packing is checked separately"),
    }
}

fn half_value(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = (bits >> 10) & 0x1f;
    let fraction = bits & 0x03ff;
    match exponent {
        0 => sign * (fraction as f32) * 2f32.powi(-24),
        0x1f => {
            if fraction == 0 {
                sign * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        _ => sign * (1.0 + f32::from(fraction) / 1024.0) * 2f32.powi(i32::from(exponent) - 15),
    }
}
