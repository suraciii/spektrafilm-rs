use super::{err, file_record, obj, resolve_path, text};
use crate::common::Result;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

pub fn geometry_mask(g: &Value, width: usize, height: usize, base: &Path) -> Result<Vec<bool>> {
    let count = width
        .checked_mul(height)
        .ok_or_else(|| err("invalid_mask", "image size overflow"))?;
    let g = g.get("mask").or_else(|| g.get("rectangle")).unwrap_or(g);
    if let Some(a) = g.as_array() {
        if a.len() != 4 || a.iter().any(|x| x.as_u64().is_none()) {
            return Err(err("invalid_bounds", "rectangle requires four integers"));
        }
        let x = a[0].as_u64().unwrap() as usize;
        let y = a[1].as_u64().unwrap() as usize;
        let w = a[2].as_u64().unwrap() as usize;
        let h = a[3].as_u64().unwrap() as usize;
        if w == 0
            || h == 0
            || x.checked_add(w).is_none_or(|n| n > width)
            || y.checked_add(h).is_none_or(|n| n > height)
        {
            return Err(err("invalid_bounds", "geometry exceeds image"));
        }
        let mut m = vec![false; count];
        for yy in y..y + h {
            for xx in x..x + w {
                m[yy * width + xx] = true
            }
        }
        return Ok(m);
    }
    let o = obj(g)?;
    file_record(g, base)?;
    if o.get("width").and_then(Value::as_u64) != Some(width as u64)
        || o.get("height").and_then(Value::as_u64) != Some(height as u64)
    {
        return Err(err(
            "invalid_region",
            "declared mask dimensions differ from master",
        ));
    }
    let p = resolve_path(text(o, "path")?, base)?;
    let ext = p
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let values = match ext.as_str() {
        "npy" => {
            let (v, shape) = crate::image_data::read_npy(&p)?;
            if shape != [height, width] {
                return Err(err("invalid_region", "mask dimensions differ from master"));
            }
            v
        }
        "tif" | "tiff" => {
            let mut decoder =
                tiff::decoder::Decoder::new(std::io::BufReader::new(fs::File::open(&p)?))
                    .map_err(|e| err("invalid_region", e.to_string()))?;
            if decoder
                .dimensions()
                .map_err(|e| err("invalid_region", e.to_string()))?
                != (width as u32, height as u32)
                || !matches!(
                    decoder
                        .colortype()
                        .map_err(|e| err("invalid_region", e.to_string()))?,
                    tiff::ColorType::Gray(_)
                )
            {
                return Err(err(
                    "invalid_region",
                    "mask must be single-channel master-sized array",
                ));
            }
            use tiff::decoder::DecodingResult::*;
            match decoder
                .read_image()
                .map_err(|e| err("invalid_region", e.to_string()))?
            {
                U8(v) => v.into_iter().map(|x| x as f64).collect(),
                U16(v) => v.into_iter().map(|x| x as f64).collect(),
                U32(v) => v.into_iter().map(|x| x as f64).collect(),
                U64(v) => v.into_iter().map(|x| x as f64).collect(),
                I8(v) => v.into_iter().map(|x| x as f64).collect(),
                I16(v) => v.into_iter().map(|x| x as f64).collect(),
                I32(v) => v.into_iter().map(|x| x as f64).collect(),
                I64(v) => v.into_iter().map(|x| x as f64).collect(),
                F32(v) => v.into_iter().map(|x| x as f64).collect(),
                F64(v) => v,
                _ => return Err(err("invalid_region", "unsupported TIFF mask samples")),
            }
        }
        "png" => {
            let im = image::open(&p).map_err(|e| err("invalid_region", e.to_string()))?;
            if im.width() != width as u32 || im.height() != height as u32 {
                return Err(err("invalid_region", "mask dimensions differ from master"));
            }
            match im {
                image::DynamicImage::ImageLuma8(v) => {
                    v.into_raw().into_iter().map(f64::from).collect()
                }
                image::DynamicImage::ImageLuma16(v) => {
                    v.into_raw().into_iter().map(f64::from).collect()
                }
                _ => {
                    return Err(err(
                        "invalid_region",
                        "mask must be single-channel master-sized array",
                    ));
                }
            }
        }
        _ => return Err(err("invalid_region", "mask must be NPY, PNG, or TIFF")),
    };
    if values.len() != count || values.iter().any(|x| !x.is_finite()) {
        return Err(err(
            "invalid_region",
            "mask must be finite single-channel master-sized array",
        ));
    }
    let mask = values.into_iter().map(|x| x != 0.0).collect::<Vec<_>>();
    if !mask.iter().any(|x| *x) {
        return Err(err("invalid_region", "mask has no selected pixels"));
    }
    Ok(mask)
}
pub fn reference_geometry_mask(
    g: &Value,
    width: usize,
    height: usize,
    base: &Path,
) -> Result<Vec<bool>> {
    let g = g.get("rectangle").unwrap_or(g);
    if g.is_array() {
        return geometry_mask(g, width, height, base);
    }
    let o = obj(g).map_err(|_| err("invalid_region", "mask requires frozen rectangle or file"))?;
    let name = text(o, "path")?;
    let p = if Path::new(name).is_absolute() {
        PathBuf::from(name)
    } else {
        base.join(name)
    };
    let (v, shape) = if p
        .extension()
        .and_then(|x| x.to_str())
        .is_some_and(|x| x.eq_ignore_ascii_case("npy"))
    {
        crate::image_data::read_npy(&p)?
    } else {
        let im = image::open(&p).map_err(|e| err("invalid_region", e.to_string()))?;
        let dims = vec![im.height() as usize, im.width() as usize];
        match im {
            image::DynamicImage::ImageLuma8(v) => {
                (v.into_raw().into_iter().map(f64::from).collect(), dims)
            }
            _ => {
                return Err(err(
                    "invalid_region",
                    "mask must be single-channel binary image",
                ));
            }
        }
    };
    if shape != [height, width]
        || ["width", "height"]
            .iter()
            .zip([width, height])
            .any(|(k, n)| o.get(*k).is_some_and(|x| x.as_u64() != Some(n as u64)))
    {
        return Err(err(
            "invalid_region",
            "mask dimensions differ from declared coordinate system",
        ));
    }
    if v.iter().any(|x| !x.is_finite()) {
        return Err(err("invalid_region", "mask contains invalid samples"));
    }
    if v.iter().any(|x| *x != 0.0 && *x != 1.0 && *x != 255.0) {
        return Err(err(
            "invalid_region",
            "mask must be binary; no implicit thresholding",
        ));
    }
    Ok(v.into_iter().map(|x| x != 0.0).collect())
}
