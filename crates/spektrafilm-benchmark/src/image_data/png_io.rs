use super::err;
use crate::common::{Error, Result};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
};

pub(crate) fn write_png16(
    path: &Path,
    rgb: &[[f64; 3]],
    width: usize,
    height: usize,
) -> Result<()> {
    if rgb.len() != width * height {
        return Err(err("invalid_image_layout", "PNG dimensions mismatch"));
    }
    let mut e = png::Encoder::new(fs::File::create(path)?, width as u32, height as u32);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Sixteen);
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

unsafe extern "C" {
    fn spektrafilm_flip(
        a: *const f32,
        b: *const f32,
        n: usize,
        w: usize,
        h: usize,
        ppd: f32,
        out: *mut f32,
    ) -> i32;
}
pub(crate) fn official_flip(
    a: &[[f64; 3]],
    b: &[[f64; 3]],
    width: usize,
    height: usize,
    ppd: f64,
) -> Result<Vec<f64>> {
    if a.len() != width * height || b.len() != a.len() {
        return Err(err("invalid_image_layout", "FLIP dimensions mismatch"));
    }
    if !ppd.is_finite() || ppd <= 0. {
        return Err(err(
            "invalid_protocol",
            "FLIP pixels_per_degree must be positive",
        ));
    }
    let aa: Vec<f32> = a
        .iter()
        .flat_map(|p| p.map(|x| x.clamp(0., 1.) as f32))
        .collect();
    let bb: Vec<f32> = b
        .iter()
        .flat_map(|p| p.map(|x| x.clamp(0., 1.) as f32))
        .collect();
    let mut o = vec![0f32; a.len()];
    let rc = unsafe {
        spektrafilm_flip(
            aa.as_ptr(),
            bb.as_ptr(),
            a.len(),
            width,
            height,
            ppd as f32,
            o.as_mut_ptr(),
        )
    };
    if rc != 0 {
        return Err(err(
            "computation_failure",
            format!("Official FLIP bridge failed ({rc})"),
        ));
    }
    if o.iter().any(|x| !x.is_finite()) {
        return Err(err(
            "computation_failure",
            "Official FLIP returned invalid map",
        ));
    }
    Ok(o.into_iter().map(|x| x as f64).collect())
}

pub(crate) fn write_png16_codes(
    path: &Path,
    rgb: &[[u16; 3]],
    width: usize,
    height: usize,
) -> Result<()> {
    if rgb.len() != width * height || width == 0 || height == 0 {
        return Err(err("invalid_image_layout", "PNG dimensions mismatch"));
    }
    let mut encoder = png::Encoder::new(fs::File::create(path)?, width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Sixteen);
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
