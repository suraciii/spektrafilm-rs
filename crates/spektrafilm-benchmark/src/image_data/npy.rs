use super::err;
use crate::common::{Error, Result};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
};

pub(crate) fn parse_shape(h: &str) -> Result<Vec<usize>> {
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
pub(crate) fn read_npy(path: &Path) -> Result<(Vec<f64>, Vec<usize>)> {
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

pub(crate) fn write_npy(path: &Path, values: &[f64], shape: &[usize]) -> Result<()> {
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
