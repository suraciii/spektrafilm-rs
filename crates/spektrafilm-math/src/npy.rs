/// Minimal NumPy `.npy` file loaders.
///
/// Supports float16 (`<f2`), float32 (`<f4`) and float64 (`<f8`) arrays in
/// C order. [`load_npy_f32`] converts to f32 for the calibrated pipeline;
/// [`load_npy_f64`] promotes exactly and preserves `<f8` bit-for-bit for lab
/// diagnostics.
use std::io::Read;
use std::io::Write;
use std::path::Path;

use half::f16;

/// Load a `.npy` file and return the shape + f32 data (converting from original dtype).
pub fn load_npy_f32<R: Read>(reader: R) -> Result<(Vec<usize>, Vec<f32>), NpyError> {
    let (descr, shape, mut reader) = read_header(reader)?;
    let total_elements: usize = shape.iter().product();

    let data = match descr.trim_matches('\'') {
        "<f2" => {
            // float16, little-endian
            let mut raw = vec![0u8; total_elements * 2];
            reader.read_exact(&mut raw)?;
            raw.as_chunks::<2>()
                .0
                .iter()
                .map(|chunk| f16::from_le_bytes(*chunk).to_f32())
                .collect()
        }
        "<f4" => {
            // float32, little-endian
            let mut raw = vec![0u8; total_elements * 4];
            reader.read_exact(&mut raw)?;
            raw.as_chunks::<4>()
                .0
                .iter()
                .map(|chunk| f32::from_le_bytes(*chunk))
                .collect()
        }
        "<f8" => {
            // float64, little-endian
            let mut raw = vec![0u8; total_elements * 8];
            reader.read_exact(&mut raw)?;
            raw.as_chunks::<8>()
                .0
                .iter()
                .map(|chunk| f64::from_le_bytes(*chunk) as f32)
                .collect()
        }
        other => return Err(NpyError::Parse(format!("unsupported dtype: {other}"))),
    };

    Ok((shape, data))
}

/// Load a `.npy` file and return the shape + f64 data.
///
/// `<f2` and `<f4` inputs are promoted exactly; `<f8` is preserved bit-exact.
pub fn load_npy_f64<R: Read>(reader: R) -> Result<(Vec<usize>, Vec<f64>), NpyError> {
    let (descr, shape, mut reader) = read_header(reader)?;
    let total_elements: usize = shape.iter().product();

    let data = match descr.trim_matches('\'') {
        "<f2" => {
            let mut raw = vec![0u8; total_elements * 2];
            reader.read_exact(&mut raw)?;
            raw.as_chunks::<2>()
                .0
                .iter()
                .map(|chunk| f16::from_le_bytes(*chunk).to_f64())
                .collect()
        }
        "<f4" => {
            let mut raw = vec![0u8; total_elements * 4];
            reader.read_exact(&mut raw)?;
            raw.as_chunks::<4>()
                .0
                .iter()
                .map(|chunk| f32::from_le_bytes(*chunk) as f64)
                .collect()
        }
        "<f8" => {
            let mut raw = vec![0u8; total_elements * 8];
            reader.read_exact(&mut raw)?;
            raw.as_chunks::<8>()
                .0
                .iter()
                .map(|chunk| f64::from_le_bytes(*chunk))
                .collect()
        }
        other => return Err(NpyError::Parse(format!("unsupported dtype: {other}"))),
    };

    Ok((shape, data))
}

/// Write a C-order little-endian `f64` array as a `.npy` file.
///
/// The header carries `'<f8'` and the shape only; the axis meaning is the
/// caller's to state. Values are written byte-exactly — no scaling, no
/// reordering.
pub fn save_npy_f64(path: &Path, values: &[f64], shape: &[usize]) -> Result<(), NpyError> {
    let expected: usize = shape.iter().product();
    if values.len() != expected {
        return Err(NpyError::Parse(format!(
            "value count {} does not match shape {:?}",
            values.len(),
            shape
        )));
    }
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
    let mut f = std::io::BufWriter::with_capacity(65536, std::fs::File::create(path)?);
    f.write_all(b"\x93NUMPY\x01\x00")?;
    f.write_all(&(header.len() as u16).to_le_bytes())?;
    f.write_all(header.as_bytes())?;
    for v in values {
        f.write_all(&v.to_le_bytes())?;
    }
    f.flush()?;
    Ok(())
}

/// Read the NumPy header, returning the descriptor, shape and the reader
/// positioned at the raw array data.
fn read_header<R: Read>(mut reader: R) -> Result<(String, Vec<usize>, R), NpyError> {
    // Magic: \x93NUMPY
    let mut magic = [0u8; 6];
    reader.read_exact(&mut magic)?;
    if &magic != b"\x93NUMPY" {
        return Err(NpyError::BadMagic);
    }

    // Version
    let mut ver = [0u8; 2];
    reader.read_exact(&mut ver)?;

    // Header length
    let header_len = if ver[0] == 1 {
        let mut buf = [0u8; 2];
        reader.read_exact(&mut buf)?;
        u16::from_le_bytes(buf) as usize
    } else {
        let mut buf = [0u8; 4];
        reader.read_exact(&mut buf)?;
        u32::from_le_bytes(buf) as usize
    };

    // Read header string
    let mut header_bytes = vec![0u8; header_len];
    reader.read_exact(&mut header_bytes)?;
    let header = String::from_utf8_lossy(&header_bytes);

    // Parse header dict — extract descr, shape, fortran_order
    let descr =
        extract_field(&header, "descr").ok_or_else(|| NpyError::Parse("missing descr".into()))?;
    let shape_str =
        extract_field(&header, "shape").ok_or_else(|| NpyError::Parse("missing shape".into()))?;
    let fortran_order = extract_field(&header, "fortran_order")
        .map(|s| s.contains("True"))
        .unwrap_or(false);

    if fortran_order {
        return Err(NpyError::Parse("Fortran order not supported".into()));
    }

    // Parse shape tuple
    let shape: Vec<usize> = shape_str
        .trim_matches(|c: char| c == '(' || c == ')' || c.is_whitespace())
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().parse::<usize>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| NpyError::Parse(format!("bad shape: {e}")))?;

    Ok((descr, shape, reader))
}

/// Extract a field value from a Python dict-like header string.
fn extract_field(header: &str, key: &str) -> Option<String> {
    let pattern = format!("'{key}':");
    let start = header.find(&pattern)?;
    let rest = &header[start + pattern.len()..];
    let rest = rest.trim_start();

    // Handle string values: 'value'
    if let Some(stripped) = rest.strip_prefix('\'') {
        let end = stripped.find('\'')?;
        return Some(rest[..end + 2].to_string());
    }

    // Handle tuple values: (1, 2, 3)
    if rest.starts_with('(') {
        let end = rest.find(')')?;
        return Some(rest[..=end].to_string());
    }

    // Handle bare values: True, False, etc.
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    Some(rest[..end].trim().to_string())
}

#[derive(Debug)]
pub enum NpyError {
    Io(std::io::Error),
    BadMagic,
    Parse(String),
}

impl From<std::io::Error> for NpyError {
    fn from(e: std::io::Error) -> Self {
        NpyError::Io(e)
    }
}

impl std::fmt::Display for NpyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NpyError::Io(e) => write!(f, "IO error: {e}"),
            NpyError::BadMagic => write!(f, "not a .npy file"),
            NpyError::Parse(s) => write!(f, "parse error: {s}"),
        }
    }
}
