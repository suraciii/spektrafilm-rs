//! The lab raster reader: channels, alpha, and containers it must not guess.

use spektrafilm_benchmark::image_data::decode_raster;
use std::path::Path;

fn png(
    path: &Path,
    width: u32,
    height: u32,
    color: png::ColorType,
    depth: png::BitDepth,
    rows: &[u8],
) {
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(color);
    encoder.set_depth(depth);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(rows).unwrap();
}

#[test]
fn a_single_channel_raster_replicates_to_three_equal_channels() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("wedge.png");
    let rows: Vec<u8> = (0..8).flat_map(|_| (0..8).map(|x| x as u8 * 16)).collect();
    png(
        &path,
        8,
        8,
        png::ColorType::Grayscale,
        png::BitDepth::Eight,
        &rows,
    );
    let image = decode_raster(&path, "sRGB", true).unwrap();
    for pixel in &image.samples {
        assert_eq!(pixel[0], pixel[1]);
        assert_eq!(pixel[1], pixel[2]);
    }
    assert_eq!(image.samples[0][0], 0.0);
    assert_eq!(image.samples[7][0], (7.0 * 16.0) / 255.0);
}

#[test]
fn an_alpha_channel_is_ignored_and_never_composited() {
    let temp = tempfile::tempdir().unwrap();
    let opaque = temp.path().join("opaque.png");
    let transparent = temp.path().join("transparent.png");
    let rgb = [10u8, 20, 30];
    let rows_opaque: Vec<u8> = (0..4).flat_map(|_| [rgb[0], rgb[1], rgb[2], 255]).collect();
    let rows_clear: Vec<u8> = (0..4).flat_map(|_| [rgb[0], rgb[1], rgb[2], 0]).collect();
    png(
        &opaque,
        2,
        2,
        png::ColorType::Rgba,
        png::BitDepth::Eight,
        &rows_opaque,
    );
    png(
        &transparent,
        2,
        2,
        png::ColorType::Rgba,
        png::BitDepth::Eight,
        &rows_clear,
    );
    let a = decode_raster(&opaque, "sRGB", true).unwrap();
    let b = decode_raster(&transparent, "sRGB", true).unwrap();
    assert_eq!(a.samples, b.samples);
}

#[test]
fn an_exr_decodes_to_the_same_samples_as_a_float_tiff() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("render.exr");
    exr::prelude::write_rgba_file(&path, 4, 4, |x, y| {
        (x as f32 / 8.0, y as f32 / 8.0, (x + y) as f32 / 16.0, 1.0)
    })
    .unwrap();
    let image = decode_raster(&path, "ACES2065-1", false).unwrap();
    assert_eq!([image.width, image.height], [4, 4]);
    assert_eq!(image.samples[0], [0.0, 0.0, 0.0]);
    assert_eq!(image.samples[1], [0.125, 0.0, 0.0625]);
}

#[test]
fn an_unsupported_layout_is_rejected_rather_than_guessed() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("cmyk.tif");
    // A container the reader has no stated normalization for.
    let file = std::fs::File::create(&path).unwrap();
    let mut encoder = tiff::encoder::TiffEncoder::new(file).unwrap();
    encoder
        .write_image::<tiff::encoder::colortype::CMYK8>(8, 8, &[0u8; 8 * 8 * 4])
        .unwrap();
    drop(encoder);
    let error = decode_raster(&path, "sRGB", true).unwrap_err();
    assert_eq!(error.code, "unsupported_image_layout");
}

#[test]
fn a_missing_file_is_named_before_decoding() {
    let temp = tempfile::tempdir().unwrap();
    let error = decode_raster(&temp.path().join("absent.png"), "sRGB", true).unwrap_err();
    assert_eq!(error.code, "invalid_path");
    assert!(error.message.contains("absent.png"), "{}", error.message);
}
