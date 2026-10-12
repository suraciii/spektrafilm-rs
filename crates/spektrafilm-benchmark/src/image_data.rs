use crate::common::{Error, Result};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
};
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
fn err(c: &str, m: impl Into<String>) -> Error {
    Error::new(c, m.into())
}
fn norm_space(s: &str) -> String {
    let k = s
        .to_ascii_lowercase()
        .replace(['-', '_', ' ', '(', ')'], "");
    match k.as_str() {
        "srgb" => "sRGB",
        "adobergb1998" | "adobergb" => "Adobe RGB (1998)",
        "prophotorgb" | "prophoto" => "ProPhoto RGB",
        "displayp3" | "p3" => "Display P3",
        "dcip3" => "DCI-P3",
        "iturbt2020" | "rec2020" | "bt2020" => "ITU-R BT.2020",
        "acescg" => "ACEScg",
        "aces2065" | "aces20651" => "ACES2065-1",
        _ => s,
    }
    .to_owned()
}
fn norm_transfer(v: &Value) -> String {
    if let Some(b) = v.as_bool() {
        return if b { "encoded" } else { "linear" }.to_owned();
    }
    let s = match v.as_str() {
        Some(x) => x,
        None => return String::new(),
    };
    let k = s.to_ascii_lowercase().replace(['-', '_', ' ', '"'], "");
    match k.as_str() {
        "srgb" => "srgb",
        "encoded" | "true" | "cctfencoded" => "encoded",
        "linear" | "identity" | "false" => "linear",
        _ => s,
    }
    .to_owned()
}
pub fn srgb_decode(x: f64) -> f64 {
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x.max(0.04045) + 0.055) / 1.055).powf(2.4)
    }
}
pub fn srgb_encode(x: f64) -> f64 {
    if x <= 0.0031308 {
        x * 12.92
    } else {
        1.055 * x.max(0.0031308).powf(1.0 / 2.4) - 0.055
    }
}
fn decode(space: &str, x: f64) -> f64 {
    match space {
        "sRGB" | "Display P3" => srgb_decode(x),
        "DCI-P3" => x.powf(2.6),
        "Adobe RGB (1998)" => x.powf(2.19921875),
        "ProPhoto RGB" => {
            if x < 16.0 / 512.0 {
                x / 16.0
            } else {
                x.powf(1.8)
            }
        }
        "ITU-R BT.2020" => {
            let a = 1.099;
            let b = 0.018_f64;
            let threshold = a * b.powf(0.45) - (a - 1.);
            if x < threshold {
                x / 4.5
            } else {
                ((x + a - 1.0) / a).powf(1.0 / 0.45)
            }
        }
        _ => x,
    }
}
fn mul(m: [[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}
fn inv(m: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let d = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    [
        [
            (m[1][1] * m[2][2] - m[1][2] * m[2][1]) / d,
            (m[0][2] * m[2][1] - m[0][1] * m[2][2]) / d,
            (m[0][1] * m[1][2] - m[0][2] * m[1][1]) / d,
        ],
        [
            (m[1][2] * m[2][0] - m[1][0] * m[2][2]) / d,
            (m[0][0] * m[2][2] - m[0][2] * m[2][0]) / d,
            (m[0][2] * m[1][0] - m[0][0] * m[1][2]) / d,
        ],
        [
            (m[1][0] * m[2][1] - m[1][1] * m[2][0]) / d,
            (m[0][1] * m[2][0] - m[0][0] * m[2][1]) / d,
            (m[0][0] * m[1][1] - m[0][1] * m[1][0]) / d,
        ],
    ]
}
fn space_matrix(s: &str) -> ([[f64; 3]; 3], [f64; 2]) {
    match s {
        "sRGB" => (
            [
                [0.4124, 0.3576, 0.1805],
                [0.2126, 0.7152, 0.0722],
                [0.0193, 0.1192, 0.9505],
            ],
            [0.3127, 0.329],
        ),
        "ProPhoto RGB" => (
            [
                [0.7977, 0.1352, 0.0313],
                [0.288, 0.7119, 0.0001],
                [0.0, 0.0, 0.8249],
            ],
            [0.3457, 0.3585],
        ),
        "DCI-P3" => (
            [
                [0.4451698155645524, 0.27713440920677773, 0.17228266981556453],
                [0.20949167791273052, 0.7215952541610438, 0.06891306792622581],
                [
                    -3.634101316969856e-17,
                    0.04706056005398116,
                    0.9073553943619733,
                ],
            ],
            [0.314, 0.351],
        ),
        "Adobe RGB (1998)" => (
            [
                [0.57667, 0.18556, 0.18823],
                [0.29734, 0.62736, 0.07529],
                [0.02703, 0.07069, 0.99134],
            ],
            [0.3127, 0.329],
        ),
        "Display P3" => (
            [
                [0.486570948648216, 0.2656676931690931, 0.1982172852343625],
                [0.2289745640697487, 0.6917385218365064, 0.079286914093745],
                [
                    -3.9720755169334855e-17,
                    0.04511338185890264,
                    1.0439443689009757,
                ],
            ],
            [0.3127, 0.329],
        ),
        "ITU-R BT.2020" => (
            [
                [0.6369580483012912, 0.1446169035862084, 0.16888097516417208],
                [0.262700212011267, 0.6779980715188711, 0.05930171646986195],
                [
                    4.994106574466074e-17,
                    0.028072693049087445,
                    1.0609850577107909,
                ],
            ],
            [0.3127, 0.329],
        ),
        "ACEScg" => (
            [
                [0.6624541811085055, 0.13400420645643313, 0.15618768700490773],
                [0.2722287167809146, 0.6740817658111483, 0.05368951740793703],
                [
                    -0.005574649490394109,
                    0.004060733528982825,
                    1.010339100312997,
                ],
            ],
            [0.32168, 0.33767],
        ),
        "ACES2065-1" => (
            [
                [0.9525523959, 0.0, 9.36786e-05],
                [0.3439664498, 0.7281660966, -0.0721325464],
                [0.0, 0.0, 1.0088251844],
            ],
            [0.32168, 0.33767],
        ),
        _ => ([[0.; 3]; 3], [0.; 2]),
    }
}
fn cat02(a: [f64; 2], b: [f64; 2]) -> [[f64; 3]; 3] {
    let xyz = |p: [f64; 2]| [p[0] / p[1], 1., (1. - p[0] - p[1]) / p[1]];
    let l = [
        [0.7328, 0.4296, -0.1624],
        [-0.7036, 1.6975, 0.0061],
        [0.003, 0.0136, 0.9834],
    ];
    let li = inv(l);
    let x = mul(l, xyz(a));
    let y = mul(l, xyz(b));
    let mut o = [[0.; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            for k in 0..3 {
                o[i][j] += li[i][k] * (y[k] / x[k]) * l[k][j];
            }
        }
    }
    o
}
fn to_srgb_linear(v: [f64; 3], s: &str) -> Option<[f64; 3]> {
    let (m, w) = space_matrix(s);
    if m[0][0] == 0. {
        return None;
    }
    let xyz = mul(m, v);
    Some(mul(
        [
            [3.2406, -1.5372, -0.4986],
            [-0.9689, 1.8758, 0.0415],
            [0.0557, -0.204, 1.057],
        ],
        mul(cat02(w, [0.3127, 0.329]), xyz),
    ))
}

fn parse_shape(h: &str) -> Result<Vec<usize>> {
    let a = h
        .find("'shape'")
        .or_else(|| h.find("\"shape\""))
        .ok_or_else(|| err("invalid_npy", "Missing shape"))?;
    let q = h[a..]
        .find('(')
        .ok_or_else(|| err("invalid_npy", "Missing shape"))?
        + a;
    let z = h[q + 1..]
        .find(')')
        .ok_or_else(|| err("invalid_npy", "Missing shape"))?
        + q
        + 1;
    let v = h[q + 1..z]
        .split(',')
        .filter(|x| !x.trim().is_empty())
        .map(|x| {
            x.trim()
                .parse::<usize>()
                .map_err(|_| err("invalid_npy", "Invalid shape"))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(v)
}
pub fn read_npy(path: &Path) -> Result<(Vec<f64>, Vec<usize>)> {
    let mut b = Vec::new();
    fs::File::open(path)?.read_to_end(&mut b)?;
    if b.len() < 10 || &b[..6] != b"\x93NUMPY" {
        return Err(err("invalid_npy", "Invalid NPY header"));
    }
    let (pos, len) = match b[6] {
        1 => (10, u16::from_le_bytes([b[8], b[9]]) as usize),
        2 | 3 => {
            if b.len() < 12 {
                return Err(err("invalid_npy", "Truncated NPY header"));
            }
            (12, u32::from_le_bytes([b[8], b[9], b[10], b[11]]) as usize)
        }
        _ => return Err(err("unsupported_npy", "Unsupported NPY version")),
    };
    if pos + len > b.len() {
        return Err(err("invalid_npy", "Truncated NPY header"));
    }
    let h = std::str::from_utf8(&b[pos..pos + len])
        .map_err(|_| err("invalid_npy", "Invalid NPY header"))?;
    let fortran = h
        .split("fortran_order")
        .nth(1)
        .map(|x| x.split(',').next().unwrap_or("").contains("True"))
        .unwrap_or(false);
    let shape = parse_shape(h)?;
    let a = h
        .find("'descr'")
        .or_else(|| h.find("\"descr\""))
        .ok_or_else(|| err("invalid_npy", "Missing dtype"))?;
    let q = h[a..].find(':').unwrap() + a;
    let tail = h[q + 1..].trim();
    let quote = tail
        .chars()
        .next()
        .ok_or_else(|| err("invalid_npy", "Missing dtype"))?;
    let end = tail[1..]
        .find(quote)
        .ok_or_else(|| err("invalid_npy", "Unterminated dtype"))?
        + 1;
    let dtype = tail[1..end].to_owned();
    let (endian, kind, size) = match dtype.as_str() {
        "|b1" | "b1" => ('=', 'b', 1),
        "<f4" | "=f4" => ('<', 'f', 4),
        ">f4" => ('>', 'f', 4),
        "<f8" | "=f8" => ('<', 'f', 8),
        ">f8" => ('>', 'f', 8),
        "|i1" | "<i1" | "=i1" | ">i1" => ('=', 'i', 1),
        "|u1" | "<u1" | "=u1" | ">u1" => ('=', 'u', 1),
        "<i2" | "=i2" => ('<', 'i', 2),
        ">i2" => ('>', 'i', 2),
        "<u2" | "=u2" => ('<', 'u', 2),
        ">u2" => ('>', 'u', 2),
        "<i4" | "=i4" => ('<', 'i', 4),
        ">i4" => ('>', 'i', 4),
        "<u4" | "=u4" => ('<', 'u', 4),
        ">u4" => ('>', 'u', 4),
        "<i8" | "=i8" => ('<', 'i', 8),
        ">i8" => ('>', 'i', 8),
        "<u8" | "=u8" => ('<', 'u', 8),
        ">u8" => ('>', 'u', 8),
        _ => {
            return Err(err(
                "unsupported_npy",
                format!("Unsupported NPY dtype {dtype}"),
            ));
        }
    };
    let data = &b[pos + len..];
    if data.len() % size != 0 {
        return Err(err("invalid_npy", "Invalid NPY payload length"));
    }
    let mut out = Vec::with_capacity(data.len() / size);
    for c in data.chunks_exact(size) {
        let x = match (kind, size) {
            ('b', _) => {
                if c[0] == 0 {
                    0.
                } else {
                    1.
                }
            }
            ('f', 4) => f32::from_bits(if endian == '<' {
                u32::from_le_bytes(c.try_into().unwrap())
            } else {
                u32::from_be_bytes(c.try_into().unwrap())
            }) as f64,
            ('f', 8) => f64::from_bits(if endian == '<' {
                u64::from_le_bytes(c.try_into().unwrap())
            } else {
                u64::from_be_bytes(c.try_into().unwrap())
            }),
            ('i', 1) => i8::from_ne_bytes([c[0]]) as f64,
            ('u', 1) => c[0] as f64,
            ('i', 2) => {
                (if endian == '<' {
                    i16::from_le_bytes(c.try_into().unwrap())
                } else {
                    i16::from_be_bytes(c.try_into().unwrap())
                }) as f64
            }
            ('u', 2) => {
                (if endian == '<' {
                    u16::from_le_bytes(c.try_into().unwrap())
                } else {
                    u16::from_be_bytes(c.try_into().unwrap())
                }) as f64
            }
            ('i', 4) => {
                (if endian == '<' {
                    i32::from_le_bytes(c.try_into().unwrap())
                } else {
                    i32::from_be_bytes(c.try_into().unwrap())
                }) as f64
            }
            ('u', 4) => {
                (if endian == '<' {
                    u32::from_le_bytes(c.try_into().unwrap())
                } else {
                    u32::from_be_bytes(c.try_into().unwrap())
                }) as f64
            }
            ('i', 8) => {
                (if endian == '<' {
                    i64::from_le_bytes(c.try_into().unwrap())
                } else {
                    i64::from_be_bytes(c.try_into().unwrap())
                }) as f64
            }
            ('u', 8) => {
                (if endian == '<' {
                    u64::from_le_bytes(c.try_into().unwrap())
                } else {
                    u64::from_be_bytes(c.try_into().unwrap())
                }) as f64
            }
            _ => return Err(err("unsupported_npy", "Unsupported NPY dtype")),
        };
        out.push(x)
    }
    let count = shape
        .iter()
        .try_fold(1usize, |a, &b| a.checked_mul(b))
        .ok_or_else(|| err("invalid_npy", "Shape overflow"))?;
    if out.len() != count {
        return Err(err("invalid_npy", "Shape differs from payload"));
    }
    if fortran {
        let original = out.clone();
        for (i, v) in out.iter_mut().enumerate() {
            let mut index = i;
            let mut offset = 0;
            let mut stride = 1;
            let mut coordinates = vec![0; shape.len()];
            for k in (0..shape.len()).rev() {
                coordinates[k] = index % shape[k];
                index /= shape[k]
            }
            for k in 0..shape.len() {
                offset += coordinates[k] * stride;
                stride *= shape[k]
            }
            *v = original[offset]
        }
    }
    Ok((out, shape))
}

pub fn write_npy(path: &Path, values: &[f64], shape: &[usize]) -> Result<()> {
    let tuple = shape
        .iter()
        .map(|x| x.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let tuple = if shape.len() == 1 {
        format!("{tuple},")
    } else {
        tuple
    };
    let raw = format!("{{'descr': '<f8', 'fortran_order': False, 'shape': ({tuple}), }}");
    let pad = (16 - (10 + raw.len() + 1) % 16) % 16;
    let header = format!("{}{}\n", raw, " ".repeat(pad));
    let mut f = BufWriter::with_capacity(65536, fs::File::create(path)?);
    f.write_all(b"\x93NUMPY\x01\x00")?;
    f.write_all(&(header.len() as u16).to_le_bytes())?;
    f.write_all(header.as_bytes())?;
    for v in values {
        f.write_all(&v.to_le_bytes())?
    }
    f.flush()?;
    Ok(())
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
        let decoder = png::Decoder::new(std::io::BufReader::new(fs::File::open(&path)?));
        let mut reader = decoder
            .read_info()
            .map_err(|e| err("image_load_failed", e.to_string()))?;
        if reader.info().color_type != png::ColorType::Rgb {
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
            png::BitDepth::Sixteen => (
                "uint16",
                data.as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| u16::from_be_bytes(*b) as f64 / 65535.)
                    .collect(),
            ),
            png::BitDepth::Eight => ("uint8", data.iter().map(|&b| b as f64 / 255.).collect()),
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
pub fn linear_rgb(image: &ImageData) -> Result<Vec<[f64; 3]>> {
    display_rgb(image).map(|v| {
        v.into_iter()
            .map(|p| [srgb_decode(p[0]), srgb_decode(p[1]), srgb_decode(p[2])])
            .collect()
    })
}
pub fn rgb_to_lab(rgb: &[[f64; 3]]) -> Vec<[f64; 3]> {
    rgb.iter()
        .map(|p| {
            let p = p.map(|v| srgb_decode(v.clamp(0., 1.)));
            let x = 0.4124 * p[0] + 0.3576 * p[1] + 0.1805 * p[2];
            let y = 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
            let z = 0.0193 * p[0] + 0.1192 * p[1] + 0.9505 * p[2];
            let f = |t: f64| {
                if t > 216.0 / 24389.0 {
                    t.cbrt()
                } else {
                    (24389.0 / 27.0 * t + 16.) / 116.
                }
            };
            let (fx, fy, fz) = (
                f(x / (0.3127 / 0.329)),
                f(y),
                f(z / ((1. - 0.3127 - 0.329) / 0.329)),
            );
            [116. * fy - 16., 500. * (fx - fy), 200. * (fy - fz)]
        })
        .collect()
}
pub fn delta_e_2000(a: [f64; 3], b: [f64; 3]) -> f64 {
    let (l1, a1, b1) = (a[0], a[1], a[2]);
    let (l2, a2, b2) = (b[0], b[1], b[2]);
    let (c1, c2) = ((a1 * a1 + b1 * b1).sqrt(), (a2 * a2 + b2 * b2).sqrt());
    let cb = (c1 + c2) / 2.;
    let g = 0.5 * (1. - (cb.powi(7) / (cb.powi(7) + 25_f64.powi(7))).sqrt());
    let (ap1, ap2) = ((1. + g) * a1, (1. + g) * a2);
    let (cp1, cp2) = ((ap1 * ap1 + b1 * b1).sqrt(), (ap2 * ap2 + b2 * b2).sqrt());
    let hp = |x: f64, y: f64| {
        let mut h = y.atan2(x).to_degrees();
        if h < 0. {
            h += 360.
        }
        h
    };
    let (h1, h2) = (hp(ap1, b1), hp(ap2, b2));
    let dh = if cp1 * cp2 == 0. {
        0.
    } else if (h2 - h1).abs() <= 180. {
        h2 - h1
    } else if h2 <= h1 {
        h2 - h1 + 360.
    } else {
        h2 - h1 - 360.
    };
    let dhp = 2. * (cp1 * cp2).sqrt() * (dh.to_radians() / 2.).sin();
    let lb = (l1 + l2) / 2.;
    let cbp = (cp1 + cp2) / 2.;
    let hb = if cp1 * cp2 == 0. {
        h1 + h2
    } else if (h1 - h2).abs() <= 180. {
        (h1 + h2) / 2.
    } else if h1 + h2 < 360. {
        (h1 + h2 + 360.) / 2.
    } else {
        (h1 + h2 - 360.) / 2.
    };
    let t = 1. - 0.17 * ((hb - 30.).to_radians()).cos()
        + 0.24 * ((2. * hb).to_radians()).cos()
        + 0.32 * ((3. * hb + 6.).to_radians()).cos()
        - 0.20 * ((4. * hb - 63.).to_radians()).cos();
    let sl = 1. + 0.015 * (lb - 50.).powi(2) / (20. + (lb - 50.).powi(2)).sqrt();
    let sc = 1. + 0.045 * cbp;
    let sh = 1. + 0.015 * cbp * t;
    let rt = -2.
        * (cbp.powi(7) / (cbp.powi(7) + 25_f64.powi(7))).sqrt()
        * (60. * (-((hb - 275.) / 25.).powi(2)).exp())
            .to_radians()
            .sin();
    (((l2 - l1) / sl).powi(2)
        + ((cp2 - cp1) / sc).powi(2)
        + (dhp / sh).powi(2)
        + rt * ((cp2 - cp1) / sc) * (dhp / sh))
        .sqrt()
}
pub fn write_png16(path: &Path, rgb: &[[f64; 3]], width: usize, height: usize) -> Result<()> {
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
pub fn official_flip(
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

pub fn write_png16_codes(path: &Path, rgb: &[[u16; 3]], width: usize, height: usize) -> Result<()> {
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
fn lanczos(x: f64) -> f64 {
    if x == 0. {
        1.
    } else if x.abs() < 3. {
        let p = std::f64::consts::PI * x;
        (p.sin() / p) * ((p / 3.).sin() / (p / 3.))
    } else {
        0.
    }
}
fn resize_weights(input: usize, output: usize) -> Vec<(usize, Vec<f64>)> {
    if input == output {
        return (0..input).map(|i| (i, vec![1.])).collect();
    }
    let scale = input as f64 / output as f64;
    let filter_scale = scale.max(1.);
    let support = 3. * filter_scale;
    (0..output)
        .map(|i| {
            let center = (i as f64 + 0.5) * scale;
            let first = ((center - support + 0.5) as isize).max(0) as usize;
            let last = ((center + support + 0.5) as usize).min(input);
            let mut weights: Vec<f64> = (first..last)
                .map(|j| lanczos((j as f64 - center + 0.5) / filter_scale))
                .collect();
            let sum: f64 = weights.iter().sum();
            if sum != 0. {
                for w in &mut weights {
                    *w /= sum
                }
            }
            (first, weights)
        })
        .collect()
}
pub fn resize_display(
    samples: &[[f64; 3]],
    width: usize,
    height: usize,
    max_edge: usize,
) -> Result<(Vec<[f64; 3]>, usize, usize)> {
    if width == 0 || height == 0 || max_edge == 0 || samples.len() != width * height {
        return Err(err("invalid_image_layout", "Resize dimensions mismatch"));
    }
    let scale = (max_edge as f64 / width.max(height) as f64).min(1.);
    let w = (width as f64 * scale).round_ties_even().max(1.) as usize;
    let h = (height as f64 * scale).round_ties_even().max(1.) as usize;
    let wx = resize_weights(width, w);
    let wy = resize_weights(height, h);
    let mut temp = vec![[0f64; 3]; height * w];
    for y in 0..height {
        for (x, (first, weights)) in wx.iter().enumerate() {
            for c in 0..3 {
                let v: f64 = weights
                    .iter()
                    .enumerate()
                    .map(|(k, weight)| {
                        samples[y * width + first + k][c].clamp(0.0, 1.0) as f32 as f64 * weight
                    })
                    .sum();
                temp[y * w + x][c] = v as f32 as f64
            }
        }
    }
    let mut out = vec![[0f64; 3]; w * h];
    for (y, (first, weights)) in wy.iter().enumerate() {
        for x in 0..w {
            for c in 0..3 {
                let v: f64 = weights
                    .iter()
                    .enumerate()
                    .map(|(k, weight)| temp[(first + k) * w + x][c] * weight)
                    .sum();
                out[y * w + x][c] = (v as f32 as f64).clamp(0., 1.)
            }
        }
    }
    Ok((out, w, h))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cie_reference_pair() {
        assert!(
            (delta_e_2000([50., 2.6772, -79.7751], [50., 0., -82.7485]) - 2.0425).abs() < 0.0001
        )
    }
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
        assert!(changed.iter().all(|x| *x > 0. && *x <= 1.));
    }
}

pub fn norms(s: &str) -> String {
    norm_space(s)
}
pub fn icc_version() -> Result<u32> {
    crate::icc::icc_version()
}
pub fn numeric_library_identity() -> Value {
    json!({"implementation":"Rust native f64","colour_reference":"colour-science 0.4.7", "flip":native_identity()["flip"].clone()})
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
        let mut encoder = png::Encoder::new(fs::File::create(&path).unwrap(), 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
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
    fn display_fit_resamples_bounded_source() {
        let samples = [
            [-0.5, 0.5, 1.5],
            [0.5, 1.5, -0.5],
            [1.5, -0.5, 0.5],
            [0.5, 0.5, 0.5],
        ];
        let (resized, width, height) = resize_display(&samples, 4, 1, 2).unwrap();
        assert_eq!((width, height), (2, 1));
        let expected = [
            [0.33835920691490173, 0.6616408228874207, 0.5],
            [0.7667439579963684, 0.33835920691490173, 0.39489680528640747],
        ];
        assert_eq!(resized, expected);
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
