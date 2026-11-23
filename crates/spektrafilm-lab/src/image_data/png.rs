use super::err;
use crate::common::Result;
use std::{fs, path::Path};

pub fn write_png16(path: &Path, rgb: &[[f64; 3]], width: usize, height: usize) -> Result<()> {
    if rgb.len() != width * height {
        return Err(err("invalid_image_layout", "PNG dimensions mismatch"));
    }
    let mut e = ::png::Encoder::new(fs::File::create(path)?, width as u32, height as u32);
    e.set_color(::png::ColorType::Rgb);
    e.set_depth(::png::BitDepth::Sixteen);
    let mut w = e
        .write_header()
        .map_err(|x| err("image_write_failed", x.to_string()))?;
    let mut b = Vec::with_capacity(rgb.len() * 6);
    for p in rgb {
        for &x in p {
            b.extend_from_slice(
                &((x.clamp(0., 1.) * 65535.).round_ties_even() as u16).to_be_bytes(),
            )
        }
    }
    w.write_image_data(&b)
        .map_err(|x| err("image_write_failed", x.to_string()))?;
    Ok(())
}

pub fn write_png16_codes(path: &Path, rgb: &[[u16; 3]], width: usize, height: usize) -> Result<()> {
    if rgb.len() != width * height || width == 0 || height == 0 {
        return Err(err("invalid_image_layout", "PNG dimensions mismatch"));
    }
    let mut encoder = ::png::Encoder::new(fs::File::create(path)?, width as u32, height as u32);
    encoder.set_color(::png::ColorType::Rgb);
    encoder.set_depth(::png::BitDepth::Sixteen);
    let mut writer = encoder
        .write_header()
        .map_err(|e| err("image_write_failed", e.to_string()))?;
    let bytes: Vec<u8> = rgb
        .iter()
        .flat_map(|p| p.iter().flat_map(|v| v.to_be_bytes()))
        .collect();
    writer
        .write_image_data(&bytes)
        .map_err(|e| err("image_write_failed", e.to_string()))?;
    Ok(())
}
