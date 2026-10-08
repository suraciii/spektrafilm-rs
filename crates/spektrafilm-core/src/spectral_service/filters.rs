//! Measured camera color-filter selection and sampling.
use spektrafilm_math::spectral::N_WAVELENGTHS;
use std::path::Path;
/// Camera taking filters shipped with the experimental runtime.
pub const NO_COLOR_FILTER: &str = "none";
const COLOR_FILTERS: &[&str] = &[
    "none", "hoya_x0", "hoya_x1", "hoya_y2", "hoya_ya3", "hoya_r1",
];

pub fn available_color_filters() -> Vec<&'static str> {
    COLOR_FILTERS.to_vec()
}

pub fn is_supported_color_filter(name: &str) -> bool {
    COLOR_FILTERS.contains(&name)
}

/// Load a measured camera-filter curve and interpolate it onto the engine's
/// 380–780 nm, 5 nm spectral grid. Unknown names are rejected before any
/// filesystem access so configuration errors are deterministic.
pub fn load_color_filter_transmittance(
    data_dir: &Path,
    name: &str,
) -> Result<Option<Vec<f64>>, String> {
    if name == NO_COLOR_FILTER {
        return Ok(None);
    }
    if !is_supported_color_filter(name) {
        return Err(format!(
            "unsupported camera color filter {name:?}; supported: {}",
            COLOR_FILTERS.join(", ")
        ));
    }
    let stem = name.strip_prefix("hoya_").unwrap();
    let path = data_dir
        .join("filters/colored/hoya")
        .join(format!("{stem}.csv"));
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("reading camera color filter {}: {e}", path.display()))?;
    let mut points = Vec::new();
    for (line, raw) in text.lines().enumerate() {
        let mut fields = raw.split(',');
        let wavelength = fields
            .next()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .ok_or_else(|| format!("invalid wavelength at {}:{}", path.display(), line + 1))?;
        let transmission = fields
            .next()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .ok_or_else(|| format!("invalid transmittance at {}:{}", path.display(), line + 1))?;
        if !wavelength.is_finite() || !transmission.is_finite() {
            return Err(format!(
                "non-finite camera filter sample at {}:{}",
                path.display(),
                line + 1
            ));
        }
        points.push((wavelength, transmission.clamp(0.0, 1.0)));
    }
    if points.len() < 2 {
        return Err(format!(
            "camera color filter {} has fewer than two samples",
            path.display()
        ));
    }
    points.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = Vec::with_capacity(N_WAVELENGTHS);
    for index in 0..N_WAVELENGTHS {
        let wavelength = 380.0 + 5.0 * index as f64;
        let value = if wavelength <= points[0].0 {
            points[0].1
        } else if wavelength >= points[points.len() - 1].0 {
            points[points.len() - 1].1
        } else {
            let upper = points.partition_point(|(x, _)| *x < wavelength);
            let (x0, y0) = points[upper - 1];
            let (x1, y1) = points[upper];
            y0 + (y1 - y0) * (wavelength - x0) / (x1 - x0)
        };
        out.push(value);
    }
    Ok(Some(out))
}
