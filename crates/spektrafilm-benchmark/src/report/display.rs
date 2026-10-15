//! High-quality sRGB JPEG full-image display copies for human review.
//!
//! Display copies never replace evidence: numerical masters and lossless
//! native detail stay untouched, and every generated copy records its
//! display-only role, encoding, ICC profile and source identity. Color
//! conversion reuses the existing `image_data` display conversion; publication
//! reuses the report asset pipeline (hash-checked copy, manifest identity).

use crate::{
    common::{Result, sha256_file},
    image_data,
};
use image::{ExtendedColorType, ImageEncoder, codecs::jpeg::JpegEncoder};
use serde_json::{Value, json};
use std::{fs, io::BufWriter};

use super::assets::Assets;
use super::escape::*;

/// The authoritative sRGB profile the renderer embeds for encoded sRGB output
/// (`data/icc` asset family). The same bytes are embedded in every display
/// copy; the report never synthesizes its own profile.
pub(crate) const SRGB_ICC: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../data/icc/ellelstone/sRGB-elle-V2-srgbtrc.icc"
));
pub(crate) const SRGB_ICC_PATH: &str = "data/icc/ellelstone/sRGB-elle-V2-srgbtrc.icc";
pub(crate) const JPEG_QUALITY: u8 = 95;

/// Declared display color of a retained full image, with the provenance of the
/// claim. Unknown color is never assumed.
fn display_color(entry: &Value) -> Option<(String, String, &'static str)> {
    let color = entry.get("color")?;
    let space = color.get("color_space").and_then(Value::as_str)?;
    let transfer = color
        .get("transfer")
        .or_else(|| color.get("cctf_decoding"))
        .and_then(Value::as_str)
        .map(norm_transfer)
        .unwrap_or_else(|| {
            if color.get("cctf_decoding") == Some(&json!(true)) {
                "srgb".into()
            } else {
                "linear".into()
            }
        });
    Some((
        space.to_owned(),
        transfer,
        "artifact record color declaration",
    ))
}

fn display_convention_color(entry: &Value) -> Option<(String, String, &'static str)> {
    if entry.get("color").is_some() {
        return None;
    }
    if entry["media_type"] == "image/png"
        && entry["bit_depth"] == 16
        && entry["quantization"] == "16-bit rounded; no dither"
    {
        // Report display artifacts are written from `image_data::display_rgb`
        // or `references::save_display` as 16-bit sRGB display codes.
        return Some((
            "sRGB".into(),
            "srgb".into(),
            "report display artifact convention (16-bit sRGB display codes)",
        ));
    }
    None
}

fn norm_transfer(value: &str) -> String {
    match value {
        "linear" => "linear".into(),
        _ => "srgb".into(),
    }
}

/// Generate the human display copy of one retained full image.
///
/// Returns `Ok(None)` when the copy cannot be produced without inventing color
/// meaning; the caller keeps showing the original entry and reports the
/// omission. The generated record is appended to the report manifest with an
/// explicit display-only role.
pub(crate) fn jpeg_display_copy(assets: &mut Assets, entry: &Value) -> Result<Option<Value>> {
    if entry["availability"] != "available" {
        return Ok(None);
    }
    let (space, transfer, provenance) =
        match display_color(entry).or_else(|| display_convention_color(entry)) {
            Some(value) => value,
            None => return Ok(None),
        };
    let source_rel = get(entry, "path", "").to_owned();
    if source_rel.is_empty() {
        return Ok(None);
    }
    let source = assets.out.join(&source_rel);
    let (width, height, codes, clipped) = if space == "sRGB" && transfer == "srgb" {
        // Already display-encoded codes need only carrier quantization.
        let image = image::open(&source)
            .map_err(|e| crate::common::Error::new("image_write", e.to_string()))?;
        let rgb = image.into_rgb8();
        (rgb.width(), rgb.height(), rgb.into_raw(), 0)
    } else {
        let declaration =
            json!({"path":source.display().to_string(),"color_space":space,"transfer":transfer});
        let image = image_data::load_image(&declaration)?;
        let samples = image_data::display_rgb(&image)?;
        let mut clipped = 0usize;
        let mut codes = Vec::with_capacity(samples.len() * 3);
        for pixel in &samples {
            for value in pixel {
                if !(0.0..=1.0).contains(value) {
                    clipped += 1;
                }
                codes.push((value.clamp(0.0, 1.0) * 255.0).round_ties_even() as u8);
            }
        }
        (image.width as u32, image.height as u32, codes, clipped)
    };
    let (relative, target) = assets.generated_path("display.jpg")?;
    let file = fs::File::create(&target)?;
    let mut encoder = JpegEncoder::new_with_quality(BufWriter::new(file), JPEG_QUALITY);
    encoder
        .set_icc_profile(SRGB_ICC.to_vec())
        .map_err(|e| crate::common::Error::new("image_write", e.to_string()))?;
    encoder
        .write_image(&codes, width, height, ExtendedColorType::Rgb8)
        .map_err(|e| crate::common::Error::new("image_write", e.to_string()))?;
    let digest = sha256_file(&target)?;
    let record = json!({
        "path": relative,
        "sha256": digest,
        "media_type": "image/jpeg",
        "role": "display_only",
        "availability": "available",
        "dimensions": [width, height],
        "source_path": entry.get("source_path").cloned().unwrap_or(json!(source_rel)),
        "source_sha256": entry.get("sha256").cloned().unwrap_or(Value::Null),
        "color": {"color_space": "sRGB", "transfer": "srgb", "provenance": provenance},
        "display": {"carrier": "jpeg", "quality": JPEG_QUALITY, "subsampling": "4:2:2", "purpose": "human full-image display only; never a measurement source"},
        "icc": {"path": SRGB_ICC_PATH, "sha256": sha256_bytes(SRGB_ICC), "embedded": true},
        "quantization": "8-bit rounded from display values; 4:2:2 chroma",
        "clipped_sample_fraction": if codes.is_empty() { 0.0 } else { clipped as f64 / codes.len() as f64 },
    });
    assets.manifest.push(record.clone());
    Ok(Some(record))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
