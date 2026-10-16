use crate::common::{Error, Result};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

mod color;
mod flip;
mod npy;
mod png;
mod resize;

pub use color::*;
pub use flip::*;
pub use npy::*;
pub use png::*;
pub use resize::*;

pub fn native_identity() -> Value {
    json!({"flip": {"implementation": "NVlabs FLIP", "version": "1.7", "source_url": "https://files.pythonhosted.org/packages/00/dc/39cc51468562899883b43d04b9754c634798e6f7f8ffcd3b984743da041c/flip_evaluator-1.7.tar.gz", "source_distribution_sha256": "88204ec5000fa90b26e00f6f21e8d73f18e5afac7e8a23cb0803b117b3da081b", "header_sha256": "412b118db343a3a0d030104f17f46c1e3edc7455d2b83a7398a42e25befed104", "build": "CPU C++17 no CUDA no fast-math", "bridge": "spektrafilm_flip", "input": "LDR encoded sRGB clipped cast float32 then official float32 sRGB decode", "apply_magma": false, "ppd": "pixels_per_degree"}, "icc": {"implementation": "LittleCMS", "intent": "relative_colorimetric", "black_point_compensation": false, "format": "RGB_DBL"}})
}

#[derive(Clone, Debug)]
pub struct ImageData {
    pub samples: Vec<[f64; 3]>,
    pub width: usize,
    pub height: usize,
    pub color_space: String,
    pub transfer: String,
    pub metadata: Value,
    pub path: Option<PathBuf>,
}
impl ImageData {
    pub fn dimensions(&self) -> [usize; 2] {
        [self.width, self.height]
    }
}

pub(crate) fn err(c: &str, m: impl Into<String>) -> Error {
    Error::new(c, m.into())
}

pub fn load_image(source: &Value) -> Result<ImageData> {
    let declaration = if let Some(path) = source.as_str() {
        json!({"path":path})
    } else {
        source.clone()
    };
    let d = declaration
        .as_object()
        .ok_or_else(|| err("missing_image", "Image declaration requires object"))?;
    let p = d
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| err("missing_image", "Image declaration requires path"))?;
    let path = PathBuf::from(p);
    let mut meta = declaration.clone();
    let side = PathBuf::from(format!("{p}.json"));
    if side.exists() {
        let s: Value = serde_json::from_str(&fs::read_to_string(&side)?)?;
        if !s.is_object() {
            return Err(err(
                "invalid_image_metadata",
                "Image sidecar must be an object",
            ));
        }
        let mut o = s.as_object().unwrap().clone();
        o.extend(d.clone());
        meta = Value::Object(o)
    }
    let mut layout = meta
        .get("layout")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for key in [
        "width",
        "height",
        "channels",
        "dtype",
        "channel_order",
        "endianness",
    ] {
        if let Some(v) = meta.get(key) {
            layout.insert(key.to_owned(), v.clone());
        }
    }
    let l = Some(&layout);
    let ext = path
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let (w, h, flat) = if ext == "npy" {
        let (mut values, shape) = read_npy(&path)?;
        if shape.len() != 3 || shape[2] != 3 {
            return Err(err("invalid_image_layout", "NPY must be H x W x 3"));
        }
        let dtype = npy_dtype(&path)?;
        let maximum = if dtype.contains('u') {
            match dtype.chars().last() {
                Some('1') => Some(u8::MAX as f64),
                Some('2') => Some(u16::MAX as f64),
                Some('4') => Some(u32::MAX as f64),
                Some('8') => Some(u64::MAX as f64),
                _ => None,
            }
        } else if dtype.contains('i') {
            match dtype.chars().last() {
                Some('1') => Some(i8::MAX as f64),
                Some('2') => Some(i16::MAX as f64),
                Some('4') => Some(i32::MAX as f64),
                Some('8') => Some(i64::MAX as f64),
                _ => None,
            }
        } else {
            None
        };
        if let Some(maximum) = maximum {
            for value in &mut values {
                *value /= maximum
            }
        } else if !dtype.contains('f') {
            return Err(err(
                "unsupported_image_layout",
                "Samples must be unsigned raster or floating values",
            ));
        }
        meta["container_dtype"] = json!(dtype_name(&dtype));
        (shape[1], shape[0], values)
    } else if ["raw", "bin", "f32", "f64"].contains(&ext.as_str()) {
        let l = l.ok_or_else(|| err("missing_raw_metadata", "Raw images require metadata"))?;
        let w = l
            .get("width")
            .and_then(Value::as_u64)
            .ok_or_else(|| err("missing_raw_metadata", "width"))? as usize;
        let h = l
            .get("height")
            .and_then(Value::as_u64)
            .ok_or_else(|| err("missing_raw_metadata", "height"))? as usize;
        if l.get("channels").and_then(Value::as_u64) != Some(3)
            || l.get("channel_order").and_then(Value::as_str) != Some("RGB")
        {
            return Err(err(
                "missing_raw_metadata",
                "Raw images require declared RGB dimensions, dtype, order, and endianness",
            ));
        }
        let dt = l
            .get("dtype")
            .and_then(Value::as_str)
            .ok_or_else(|| err("missing_raw_metadata", "dtype"))?;
        let little = match l.get("endianness").and_then(Value::as_str) {
            Some("little") => true,
            Some("big") => false,
            None => return Err(err("missing_raw_metadata", "endianness")),
            _ => {
                return Err(err(
                    "invalid_image_metadata",
                    "Raw endianness must be little or big",
                ));
            }
        };
        meta["container_dtype"] = json!(if matches!(dt, "f32" | "float32" | "<f4" | ">f4") {
            "float32"
        } else {
            "float64"
        });
        let b = fs::read(&path)?;
        let n = w
            .checked_mul(h)
            .and_then(|v| v.checked_mul(3))
            .ok_or_else(|| err("invalid_image_layout", "Raw dimensions overflow"))?;
        let mut v = Vec::with_capacity(n);
        match dt {
            "f32" | "float32" | "<f4" | ">f4" => {
                if b.len() != n * 4 {
                    return Err(err("invalid_image_layout", "Raw file length differs"));
                }
                for c in b.as_chunks::<4>().0 {
                    v.push(f32::from_bits(if little {
                        u32::from_le_bytes(*c)
                    } else {
                        u32::from_be_bytes(*c)
                    }) as f64)
                }
            }
            "f64" | "float64" | "<f8" | ">f8" => {
                if b.len() != n * 8 {
                    return Err(err("invalid_image_layout", "Raw file length differs"));
                }
                for c in b.as_chunks::<8>().0 {
                    v.push(f64::from_bits(if little {
                        u64::from_le_bytes(*c)
                    } else {
                        u64::from_be_bytes(*c)
                    }))
                }
            }
            _ => {
                return Err(err(
                    "unsupported_image_layout",
                    "Raw masters require float32 or float64",
                ));
            }
        }
        (w, h, v)
    } else if ["tif", "tiff"].contains(&ext.as_str()) {
        let mut decoder =
            tiff::decoder::Decoder::new(std::io::BufReader::new(fs::File::open(&path)?))
                .map_err(|e| err("image_load_failed", e.to_string()))?;
        let (w, h) = decoder
            .dimensions()
            .map_err(|e| err("image_load_failed", e.to_string()))?;
        if !matches!(
            decoder
                .colortype()
                .map_err(|e| err("image_load_failed", e.to_string()))?,
            tiff::ColorType::RGB(_)
        ) {
            return Err(err(
                "invalid_image_layout",
                "Expected H x W x 3 RGB samples",
            ));
        }
        use tiff::decoder::DecodingResult as D;
        let result = decoder
            .read_image()
            .map_err(|e| err("image_load_failed", e.to_string()))?;
        let (dtype, v) = match result {
            D::F32(v) => ("float32", v.into_iter().map(|x| x as f64).collect()),
            D::F64(v) => ("float64", v),
            D::U8(v) => (
                "uint8",
                v.into_iter().map(|x| x as f64 / u8::MAX as f64).collect(),
            ),
            D::U16(v) => (
                "uint16",
                v.into_iter().map(|x| x as f64 / u16::MAX as f64).collect(),
            ),
            D::U32(v) => (
                "uint32",
                v.into_iter().map(|x| x as f64 / u32::MAX as f64).collect(),
            ),
            D::U64(v) => (
                "uint64",
                v.into_iter().map(|x| x as f64 / u64::MAX as f64).collect(),
            ),
            D::I8(v) => (
                "int8",
                v.into_iter().map(|x| x as f64 / i8::MAX as f64).collect(),
            ),
            D::I16(v) => (
                "int16",
                v.into_iter().map(|x| x as f64 / i16::MAX as f64).collect(),
            ),
            D::I32(v) => (
                "int32",
                v.into_iter().map(|x| x as f64 / i32::MAX as f64).collect(),
            ),
            D::I64(v) => (
                "int64",
                v.into_iter().map(|x| x as f64 / i64::MAX as f64).collect(),
            ),
            _ => {
                return Err(err(
                    "unsupported_image_layout",
                    "Unsupported TIFF sample dtype",
                ));
            }
        };
        meta["container_dtype"] = json!(dtype);
        (w as usize, h as usize, v)
    } else if ext == "png" {
        let decoder = ::png::Decoder::new(std::io::BufReader::new(fs::File::open(&path)?));
        let mut reader = decoder
            .read_info()
            .map_err(|e| err("image_load_failed", e.to_string()))?;
        if reader.info().color_type != ::png::ColorType::Rgb {
            return Err(err(
                "unsupported_image_layout",
                "Raster must have three native RGB channels",
            ));
        }
        let mut bytes = vec![
            0;
            reader.output_buffer_size().ok_or_else(|| err(
                "image_load_failed",
                "PNG output size unavailable"
            ))?
        ];
        let info = reader
            .next_frame(&mut bytes)
            .map_err(|e| err("image_load_failed", e.to_string()))?;
        let data = &bytes[..info.buffer_size()];
        let (dtype, values) = match info.bit_depth {
            ::png::BitDepth::Sixteen => (
                "uint16",
                data.as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| u16::from_be_bytes(*b) as f64 / 65535.)
                    .collect(),
            ),
            ::png::BitDepth::Eight => ("uint8", data.iter().map(|&b| b as f64 / 255.).collect()),
            _ => {
                return Err(err(
                    "unsupported_image_layout",
                    "Unsupported RGB PNG bit depth",
                ));
            }
        };
        meta["container_dtype"] = json!(dtype);
        (info.width as usize, info.height as usize, values)
    } else {
        if !["jpg", "jpeg", "webp"].contains(&ext.as_str()) {
            return Err(err(
                "unsupported_image_encoding",
                "Supported images are NPY, declared raw, TIFF, PNG, JPEG, and WebP",
            ));
        }
        let im = image::open(&path).map_err(|e| err("image_load_failed", e.to_string()))?;
        if !matches!(
            im,
            image::DynamicImage::ImageRgb8(_) | image::DynamicImage::ImageRgb16(_)
        ) {
            return Err(err(
                "unsupported_image_layout",
                "Raster must have three native RGB channels",
            ));
        }
        meta["container_dtype"] = json!(if matches!(im, image::DynamicImage::ImageRgb16(_)) {
            "uint16"
        } else {
            "uint8"
        });
        let (w, h) = (im.width() as usize, im.height() as usize);
        let v = match im {
            image::DynamicImage::ImageRgb16(a) => a
                .pixels()
                .flat_map(|p| p.0.map(|x| x as f64 / 65535.))
                .collect(),
            image::DynamicImage::ImageRgb32F(a) => {
                a.pixels().flat_map(|p| p.0).map(|x| x as f64).collect()
            }
            _ => im
                .to_rgb8()
                .pixels()
                .flat_map(|p| p.0.map(|x| x as f64 / 255.))
                .collect(),
        };
        (w, h, v)
    };
    if flat.len() != w * h * 3 {
        return Err(err("invalid_image_layout", "Expected RGB samples"));
    }
    let color = meta.get("color").and_then(Value::as_object);
    let cs = meta
        .get("color_space")
        .and_then(Value::as_str)
        .or_else(|| color.and_then(|x| x.get("color_space").and_then(Value::as_str)))
        .unwrap_or("");
    let tr = meta
        .get("transfer")
        .or_else(|| meta.get("cctf_decoding"))
        .or_else(|| color.and_then(|x| x.get("transfer").or_else(|| x.get("cctf_decoding"))))
        .map(norm_transfer)
        .unwrap_or_default();
    if w == 0 || h == 0 {
        return Err(err(
            "invalid_image_layout",
            "Expected nonempty H x W x 3 RGB samples",
        ));
    }
    for (key, actual) in [("width", w), ("height", h), ("channels", 3)] {
        if layout
            .get(key)
            .and_then(Value::as_u64)
            .is_some_and(|v| v as usize != actual)
        {
            return Err(err(
                "invalid_image_layout",
                format!("Decoded image disagrees with declared {key}"),
            ));
        }
    }
    let samples = flat
        .as_chunks::<3>()
        .0
        .iter()
        .map(|x| [x[0], x[1], x[2]])
        .collect();
    Ok(ImageData {
        samples,
        width: w,
        height: h,
        color_space: norm_space(cs),
        transfer: tr,
        metadata: meta,
        path: Some(path),
    })
}

pub fn display_rgb(image: &ImageData) -> Result<Vec<[f64; 3]>> {
    if image.metadata.get("icc").is_some()
        || image
            .metadata
            .get("color")
            .and_then(|x| x.get("icc"))
            .is_some()
    {
        return crate::icc::transform_rgb(image);
    }
    if image.color_space.is_empty() || image.transfer.is_empty() {
        return Err(err(
            "missing_color_declaration",
            "Color space and transfer must be explicit",
        ));
    }
    if ![
        "sRGB",
        "Display P3",
        "DCI-P3",
        "Adobe RGB (1998)",
        "ProPhoto RGB",
        "ITU-R BT.2020",
        "ACEScg",
        "ACES2065-1",
    ]
    .contains(&image.color_space.as_str())
    {
        return Err(err(
            "unsupported_color_transform",
            format!("Unknown colour space: {}", image.color_space),
        ));
    }
    if image.transfer != "linear" && image.transfer != "srgb" && image.transfer != "encoded" {
        return Err(err(
            "unsupported_color_transform",
            format!("Unknown transfer: {}", image.transfer),
        ));
    }
    let mut o = Vec::with_capacity(image.samples.len());
    for &p in &image.samples {
        let l = if image.transfer == "linear" {
            p
        } else {
            [
                decode(&image.color_space, p[0]),
                decode(&image.color_space, p[1]),
                decode(&image.color_space, p[2]),
            ]
        };
        let q = if image.color_space == "sRGB" {
            l
        } else {
            to_srgb_linear(l, &image.color_space)
                .ok_or_else(|| err("unsupported_color_transform", "Unknown colour space"))?
        };
        o.push([srgb_encode(q[0]), srgb_encode(q[1]), srgb_encode(q[2])])
    }
    Ok(o)
}
pub fn rgb_to_lab(rgb: &[[f64; 3]]) -> Vec<[f64; 3]> {
    rgb.iter()
        .map(|p| spektrafilm_math::lab::srgb_encoded_to_lab(p.map(|v| v.clamp(0., 1.))))
        .collect()
}
pub use spektrafilm_math::lab::delta_e_2000;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn srgb_extended_range_roundtrip() {
        for x in [-0.125, 0., 0.03125, 0.18, 1., 1.5, 2.] {
            assert!((srgb_decode(srgb_encode(x)) - x).abs() < 1e-12)
        }
    }
    #[test]
    fn lab_gray_uses_encoded_decode() {
        let l = rgb_to_lab(&[[0.5; 3]])[0];
        assert!((l[0] - 53.38896474).abs() < 1e-6)
    }
    #[test]
    fn flip_identical_and_exposure() {
        let a = vec![[0.2; 3]; 64];
        let b = vec![[0.3; 3]; 64];
        assert!(
            official_flip(&a, &a, 8, 8, 60.)
                .unwrap()
                .iter()
                .all(|x| *x == 0.)
        );
        let changed = official_flip(&a, &b, 8, 8, 60.).unwrap();
        assert!(changed.iter().all(|x| *x > 0.));
    }
}

fn npy_dtype(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;
    let start = if bytes[6] == 1 { 10 } else { 12 };
    let len = if start == 10 {
        u16::from_le_bytes(bytes[8..10].try_into().unwrap()) as usize
    } else {
        u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize
    };
    let header = std::str::from_utf8(&bytes[start..start + len])
        .map_err(|_| err("invalid_npy", "Invalid header"))?;
    let key = header
        .find("descr")
        .ok_or_else(|| err("invalid_npy", "Missing dtype"))?;
    let tail = header[key..].split_once(':').unwrap().1.trim();
    let quote = tail.chars().next().unwrap();
    let end = tail[1..].find(quote).unwrap() + 1;
    Ok(tail[1..end].to_owned())
}
fn dtype_name(dtype: &str) -> String {
    if dtype.starts_with('>') && !dtype.ends_with('1') {
        return dtype.to_owned();
    }
    let kind = dtype.chars().nth(1).unwrap_or('f');
    let size = dtype[2..].parse::<usize>().unwrap_or(8);
    match kind {
        'u' => format!("uint{}", size * 8),
        'i' => format!("int{}", size * 8),
        'f' => format!("float{}", size * 8),
        _ => "bool".to_owned(),
    }
}

#[cfg(test)]
mod carrier_tests {
    use super::*;
    #[test]
    fn flat_raw_metadata_preserves_extremes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("master.raw");
        let values = [-0.125f64, 0., 2.];
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_be_bytes()).collect();
        fs::write(&path, bytes).unwrap();
        fs::write(format!("{}.json",path.display()),r#"{"width":1,"height":1,"channels":3,"dtype":"float64","channel_order":"RGB","endianness":"big","color_space":"sRGB","transfer":"linear"}"#).unwrap();
        let image = load_image(&json!({"path":path})).unwrap();
        assert_eq!(image.samples, vec![values]);
        assert_eq!(image.metadata["container_dtype"], "float64");
    }
    #[test]
    fn fortran_npy_is_reordered() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mask.npy");
        let header = "{'descr': '<f8', 'fortran_order': True, 'shape': (2, 3), }\n";
        let mut bytes = b"\x93NUMPY\x01\x00".to_vec();
        bytes.extend_from_slice(&(header.len() as u16).to_le_bytes());
        bytes.extend_from_slice(header.as_bytes());
        for value in [0f64, 3., 1., 4., 2., 5.] {
            bytes.extend_from_slice(&value.to_le_bytes())
        }
        fs::write(&path, bytes).unwrap();
        let (values, shape) = read_npy(&path).unwrap();
        assert_eq!(shape, vec![2, 3]);
        assert_eq!(values, vec![0., 1., 2., 3., 4., 5.]);
    }
    #[test]
    fn rgba_native_samples_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rgba.png");
        let mut encoder = ::png::Encoder::new(fs::File::create(&path).unwrap(), 1, 1);
        encoder.set_color(::png::ColorType::Rgba);
        encoder.set_depth(::png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[1, 2, 3, 255])
            .unwrap();
        assert_eq!(
            load_image(&json!({"path":path})).unwrap_err().code,
            "unsupported_image_layout"
        );
    }
    #[test]
    fn display_fit_retains_float_precision() {
        let samples = vec![[0.123456789, 0.5, 1.5]; 16];
        let (resized, w, h) = resize_display(&samples, 4, 4, 2).unwrap();
        assert_eq!((w, h), (2, 2));
        for p in resized {
            assert!((p[0] - 0.123456789f64 as f32 as f64).abs() < 1e-7);
            assert_eq!(p[2], 1.);
        }
    }
}
