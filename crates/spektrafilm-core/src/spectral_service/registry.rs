//! Spectral LUT descriptors, asset loading, shape contracts and cache.
use super::{SpectralShape, default_spectral_shape};
use spektrafilm_math::npy;
use spektrafilm_math::spectral::N_WAVELENGTHS;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
/// Descriptor for one shipped spectral upsampler.  The sidecar format is
/// deliberately small; keeping this representation local avoids making the
/// runtime depend on a TOML parser just to select a LUT.
#[derive(Debug, Clone, PartialEq)]
pub struct LutDescriptor {
    pub identifier: String,
    pub file: String,
    pub kind: String,
    pub title: String,
    pub lut_size: usize,
    pub bands: usize,
    /// Wavelength grid encoded by the asset, as [start_nm, end_nm, step_nm].
    pub spectral_shape: [f64; 3],
    pub scene_illuminant: Option<String>,
    pub midgray: Option<f64>,
}
/// Load the spectra LUT from the .npy file.
/// Shape: (size, size, 81) — maps tc coordinates → 81-wavelength spectra.
pub fn load_spectra_lut(data_dir: &Path) -> Result<SpectraLut, String> {
    let path = data_dir
        .join("luts")
        .join("spectral_upsampling")
        .join("irradiance_xy_tc.npy");

    let file = std::fs::File::open(&path)
        .map_err(|e| format!("opening spectra LUT {}: {e}", path.display()))?;
    let reader = std::io::BufReader::new(file);

    let (shape, data) =
        npy::load_npy_f32(reader).map_err(|e| format!("loading spectra LUT: {e}"))?;

    if shape.len() != 3 || shape[2] != N_WAVELENGTHS {
        return Err(format!(
            "spectra LUT shape mismatch: expected (N, N, {N_WAVELENGTHS}), got {shape:?}"
        ));
    }
    if shape[0] != shape[1] {
        return Err(format!(
            "spectra LUT must be square, got {}x{}",
            shape[0], shape[1]
        ));
    }

    Ok(SpectraLut {
        size: shape[0],
        n_wavelengths: shape[2],
        data,
    })
}

/// Load the arctic2026alpha02 effective-reflectance LUT.
///
/// Shape: (size, size, 81). The table is a D65-recovered unit-bright
/// reflectance surface in triangular coordinates. At render time we relight it
/// by the film reference illuminant before integrating against film
/// sensitivity.
pub fn load_arctic2026alpha02_lut(data_dir: &Path) -> Result<SpectraLut, String> {
    let path = data_dir
        .join("luts")
        .join("spectral_upsampling")
        .join("arctic2026alpha02")
        .join("reflectance_xy_tc.npy");

    let file = std::fs::File::open(&path)
        .map_err(|e| format!("opening arctic2026alpha02 LUT {}: {e}", path.display()))?;
    let reader = std::io::BufReader::new(file);

    let (shape, data) =
        npy::load_npy_f32(reader).map_err(|e| format!("loading arctic2026alpha02 LUT: {e}"))?;

    if shape.len() != 3 || shape[2] != N_WAVELENGTHS {
        return Err(format!(
            "arctic2026alpha02 LUT shape mismatch: expected (N, N, {N_WAVELENGTHS}), got {shape:?}"
        ));
    }
    if shape[0] != shape[1] {
        return Err(format!(
            "arctic2026alpha02 LUT must be square, got {}x{}",
            shape[0], shape[1]
        ));
    }

    Ok(SpectraLut {
        size: shape[0],
        n_wavelengths: shape[2],
        data,
    })
}

/// Parse the shipped sidecar descriptor into the fields consumed at runtime.
pub fn parse_lut_descriptor(text: &str) -> Result<LutDescriptor, String> {
    let mut values = std::collections::HashMap::<String, String>::new();
    let mut section = "";
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = &line[1..line.len() - 1];
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let name = match (section, key.trim()) {
            ("array", "lut_size") => "array.lut_size",
            ("array", "bands") => "array.bands",
            ("array", "spectral_shape") => "array.spectral_shape",
            ("reflectance", "scene_illuminant") => "reflectance.scene_illuminant",
            ("reflectance", "midgray") => "reflectance.midgray",
            _ => key.trim(),
        };
        values.insert(name.to_string(), value.trim().trim_matches('"').to_string());
    }
    let required = |key: &str| {
        values
            .get(key)
            .cloned()
            .ok_or_else(|| format!("descriptor missing {key}"))
    };
    let shape_text = required("array.spectral_shape")?;
    let shape_text = shape_text
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']');
    let shape_values = shape_text
        .split(',')
        .map(|value| {
            value
                .trim()
                .parse::<f64>()
                .map_err(|_| "invalid spectral_shape".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let spectral_shape = <[f64; 3]>::try_from(shape_values)
        .map_err(|_| "spectral_shape must contain [start, end, step]".to_string())?;
    if !spectral_shape.iter().all(|value| value.is_finite())
        || spectral_shape[2] <= 0.0
        || spectral_shape[1] < spectral_shape[0]
    {
        return Err("spectral_shape must contain finite ascending bounds and positive step".into());
    }
    Ok(LutDescriptor {
        identifier: required("identifier")?,
        file: required("file")?,
        kind: required("kind")?,
        title: required("title")?,
        lut_size: required("array.lut_size")?
            .parse()
            .map_err(|_| "invalid lut_size".to_string())?,
        bands: required("array.bands")?
            .parse()
            .map_err(|_| "invalid bands".to_string())?,
        spectral_shape,
        scene_illuminant: values.get("reflectance.scene_illuminant").cloned(),
        midgray: values
            .get("reflectance.midgray")
            .and_then(|v| v.parse().ok()),
    })
}

pub fn available_lut_identifiers(data_dir: &Path) -> Result<Vec<String>, String> {
    let root = data_dir.join("luts").join("spectral_upsampling");
    let mut ids = Vec::new();
    for entry in std::fs::read_dir(&root).map_err(|e| format!("reading {}: {e}", root.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|x| x.to_str()) == Some("toml") {
            ids.push(
                parse_lut_descriptor(&std::fs::read_to_string(path).map_err(|e| e.to_string())?)?
                    .identifier,
            );
        }
    }
    ids.sort();
    Ok(ids)
}

/// Stable public name for callers that do not care about LUT internals.
pub fn available_spectral_methods(data_dir: &Path) -> Result<Vec<String>, String> {
    available_lut_identifiers(data_dir)
}

pub fn lut_descriptor(data_dir: &Path, identifier: &str) -> Result<LutDescriptor, String> {
    let root = data_dir.join("luts").join("spectral_upsampling");
    for entry in std::fs::read_dir(&root).map_err(|e| format!("reading {}: {e}", root.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|x| x.to_str()) != Some("toml") {
            continue;
        }
        let desc =
            parse_lut_descriptor(&std::fs::read_to_string(path).map_err(|e| e.to_string())?)?;
        if desc.identifier == identifier {
            return Ok(desc);
        }
    }
    Err(format!(
        "unknown spectral LUT method {identifier:?}; available: {:?}",
        available_lut_identifiers(data_dir).unwrap_or_default()
    ))
}

static SPECTRA_CACHE: OnceLock<Mutex<std::collections::HashMap<String, SpectraLut>>> =
    OnceLock::new();

fn spectral_cache_key(identifier: &str, shape: [f64; 3]) -> String {
    format!(
        "{identifier}|{:.9}:{:.9}:{:.9}",
        shape[0], shape[1], shape[2]
    )
}

/// Load a registered LUT and validate it against the requested working grid.
/// Resampling is intentionally not implicit: the spectral integration arrays
/// and profile sensitivities must be sampled on the same grid.
pub fn load_lut_on_grid(
    data_dir: &Path,
    identifier: &str,
    requested: SpectralShape,
) -> Result<SpectraLut, String> {
    let desc = lut_descriptor(data_dir, identifier)?;
    let cache = SPECTRA_CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let key = spectral_cache_key(identifier, requested.bounds);
    if let Some(lut) = cache
        .lock()
        .map_err(|_| "spectral cache poisoned".to_string())?
        .get(&key)
        .cloned()
    {
        return Ok(lut);
    }
    let path = data_dir
        .join("luts")
        .join("spectral_upsampling")
        .join(&desc.file);
    let (shape, data) = npy::load_npy_f32(std::io::BufReader::new(
        std::fs::File::open(&path)
            .map_err(|e| format!("opening spectral LUT {}: {e}", path.display()))?,
    ))
    .map_err(|e| format!("loading spectral LUT {}: {e}", path.display()))?;
    let expected_bands = ((desc.spectral_shape[1] - desc.spectral_shape[0])
        / desc.spectral_shape[2])
        .round() as usize
        + 1;
    if shape != vec![desc.lut_size, desc.lut_size, desc.bands] || desc.bands != expected_bands {
        return Err(format!(
            "{identifier} descriptor/data shape mismatch: descriptor {:?}, bands {}, data shape {:?}",
            desc.spectral_shape, desc.bands, shape
        ));
    }
    if desc.spectral_shape != requested.bounds || desc.bands != requested.samples {
        return Err(format!(
            "{identifier} spectral grid mismatch: asset {:?} ({} bands), requested {:?} ({} samples); \
             resampling LUTs is unsupported because profile sensitivities and CMFs must share this grid",
            desc.spectral_shape, desc.bands, requested.bounds, requested.samples
        ));
    }
    let lut = SpectraLut {
        size: shape[0],
        n_wavelengths: shape[2],
        data,
    };
    cache
        .lock()
        .map_err(|_| "spectral cache poisoned".to_string())?
        .insert(key, lut.clone());
    Ok(lut)
}

pub fn load_lut(data_dir: &Path, identifier: &str) -> Result<SpectraLut, String> {
    load_lut_on_grid(
        data_dir,
        identifier,
        SpectralShape::new(default_spectral_shape()).expect("fixed spectral grid"),
    )
}

#[derive(Clone)]
pub struct SpectraLut {
    pub size: usize,
    pub n_wavelengths: usize,
    /// Flat: [size * size * n_wavelengths]
    pub data: Vec<f32>,
}

impl SpectraLut {
    /// Get spectrum at grid position (i, j). Returns slice of n_wavelengths.
    pub fn spectrum(&self, i: usize, j: usize) -> &[f32] {
        let start = (i * self.size + j) * self.n_wavelengths;
        &self.data[start..start + self.n_wavelengths]
    }

    /// Convert the stored f32 LUT to the f64 working cube.
    ///
    /// f32→f64 is exact, so this matches Python's `np.double` promotion of
    /// the float16 table bit-for-bit.
    pub fn to_f64_cube(&self) -> SpectraCube {
        SpectraCube {
            size: self.size,
            n_wavelengths: self.n_wavelengths,
            data: self.data.iter().map(|&v| v as f64).collect(),
        }
    }
}

/// f64 working copy of the spectra LUT — mirrors Python's
/// `np.double(np.load('irradiance_xy_tc.npy'))`, which promotes the stored
/// float16 table to float64 once at load and keeps every downstream step
/// (spectral Gaussian blur, sensitivity contraction) in f64.
#[derive(Clone)]
pub struct SpectraCube {
    pub size: usize,
    pub n_wavelengths: usize,
    /// Flat: [size * size * n_wavelengths]
    pub data: Vec<f64>,
}

impl SpectraCube {
    /// Get the spectrum at grid position (i, j). Returns n_wavelengths samples.
    pub fn spectrum(&self, i: usize, j: usize) -> &[f64] {
        let start = (i * self.size + j) * self.n_wavelengths;
        &self.data[start..start + self.n_wavelengths]
    }
}
