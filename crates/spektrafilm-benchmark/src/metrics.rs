use crate::common::{Error, Result};
use crate::documents;
use crate::image_data::{self, ImageData};
use rustfft::{FftPlanner, num_complex::Complex};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

pub const METRIC_VERSION: &str = "spektrafilm-appearance-spatial-v3";
pub const COLOR_TRANSFORM: &str = "colour-science-CAT02-srgb-D65-2deg-v1";
const LUMA: [f64; 3] = [0.2126, 0.7152, 0.0722];
fn err(code: &str, message: impl Into<String>) -> Error {
    Error::new(code, message.into())
}
fn luminance(p: [f64; 3]) -> f64 {
    p[0] * LUMA[0] + p[1] * LUMA[1] + p[2] * LUMA[2]
}
fn mean(v: &[f64]) -> Option<f64> {
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}
fn quantile(sorted: &[f64], q: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let index = (sorted.len() - 1) as f64 * q;
    let low = index.floor() as usize;
    let high = index.ceil() as usize;
    let fraction = index - low as f64;
    let difference = sorted[high] - sorted[low];
    Some(if fraction >= 0.5 {
        sorted[high] - difference * (1.0 - fraction)
    } else {
        sorted[low] + difference * fraction
    })
}
fn sort(v: &mut [f64]) {
    v.sort_by(f64::total_cmp);
}
fn median(v: &mut [f64]) -> Option<f64> {
    sort(v);
    quantile(v, 0.5)
}
fn selected(v: &[f64], mask: &[bool]) -> Vec<f64> {
    v.iter()
        .zip(mask)
        .filter_map(|(x, m)| m.then_some(*x))
        .collect()
}
fn count(mask: &[bool]) -> usize {
    mask.iter().filter(|m| **m).count()
}
fn number(settings: &Value, key: &str) -> Result<f64> {
    settings
        .get(key)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
        .ok_or_else(|| err("invalid_protocol", format!("{key} must be finite numeric")))
}
// Python's str distinguishes an integer scale from a floating scale (3 vs 3.0).
fn spelling(value: &Value) -> String {
    value.to_string()
}
fn reflect(i: isize, length: usize) -> usize {
    let period = 2 * length as isize;
    let i = i.rem_euclid(period);
    if i >= length as isize {
        (period - 1 - i) as usize
    } else {
        i as usize
    }
}
fn gaussian(v: &[f64], width: usize, height: usize, sigma: f64) -> Vec<f64> {
    if sigma <= 1e-15 {
        return v.to_vec();
    }
    let radius = (4.0 * sigma + 0.5).floor() as isize;
    let mut kernel: Vec<f64> = (-radius..=radius)
        .map(|i| (-0.5 * (i as f64 / sigma).powi(2)).exp())
        .collect();
    let total = kernel.iter().sum::<f64>();
    for k in &mut kernel {
        *k /= total;
    }
    let mut temporary = vec![0.0; v.len()];
    let mut result = vec![0.0; v.len()];
    // scipy.ndimage processes axes in order: vertical then horizontal.
    for y in 0..height {
        for x in 0..width {
            let mut value = v[y * width + x] * kernel[radius as usize];
            for j in 1..=radius {
                value += (v[reflect(y as isize - j, height) * width + x]
                    + v[reflect(y as isize + j, height) * width + x])
                    * kernel[(radius + j) as usize];
            }
            temporary[y * width + x] = value;
        }
    }
    for y in 0..height {
        for x in 0..width {
            let mut value = temporary[y * width + x] * kernel[radius as usize];
            for j in 1..=radius {
                value += (temporary[y * width + reflect(x as isize - j, width)]
                    + temporary[y * width + reflect(x as isize + j, width)])
                    * kernel[(radius + j) as usize];
            }
            result[y * width + x] = value;
        }
    }
    result
}
#[derive(Clone, Copy)]
struct Crop {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}
fn crop(mask: &[bool], width: usize, height: usize) -> Result<Crop> {
    let mut left = width;
    let mut right = 0;
    let mut top = height;
    let mut bottom = 0;
    for y in 0..height {
        for x in 0..width {
            if mask[y * width + x] {
                left = left.min(x);
                right = right.max(x + 1);
                top = top.min(y);
                bottom = bottom.max(y + 1);
            }
        }
    }
    if left == width {
        return Err(err("insufficient_samples", "Region has no selected pixels"));
    }
    Ok(Crop {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}
fn cropped_mask(mask: &[bool], width: usize, c: Crop) -> Vec<bool> {
    (c.y..c.y + c.height)
        .flat_map(|y| {
            mask[y * width + c.x..y * width + c.x + c.width]
                .iter()
                .copied()
        })
        .collect()
}
fn erode(mask: &mut Vec<bool>, width: usize, height: usize, iterations: usize) {
    if iterations == 0 {
        return;
    }
    let mut temporary = vec![false; mask.len()];
    for _ in 0..iterations {
        for y in 0..height {
            for x in 0..width {
                temporary[y * width + x] = x > 0
                    && y > 0
                    && x + 1 < width
                    && y + 1 < height
                    && mask[y * width + x]
                    && mask[y * width + x - 1]
                    && mask[y * width + x + 1]
                    && mask[(y - 1) * width + x]
                    && mask[(y + 1) * width + x];
            }
        }
        std::mem::swap(mask, &mut temporary);
    }
}
fn geometry_mask(geometry: &Value, width: usize, height: usize) -> Result<Vec<bool>> {
    // The integration owner rewrites mask paths to retained absolute paths.
    documents::geometry_mask(geometry, width, height, Path::new("/"))
}
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Availability {
    Available,
    Unavailable,
}
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum TerminalState {
    Completed,
    Failed,
}
#[derive(Serialize)]
struct Measurement {
    case_id: Option<String>,
    seed_population: Vec<Value>,
    region_id: String,
    metric: String,
    metric_version: &'static str,
    domain: String,
    units: String,
    estimator: String,
    sample_count: usize,
    artifact_refs: Vec<String>,
    availability: Availability,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum ArtifactRole {
    DifferenceMap,
    GrainSpectrum,
    SpectrumCoordinates,
    GrainRadialSpectrum,
    GrainAutocovariance,
    GrainAutocorrelation,
    GrainResidual,
    GrainSignedDifference,
    HalationEdgeSpillProfile,
    HalationProfile,
    EdgeResponse,
    EdgeSfr,
    LocalContrastMap,
    ToneResponse,
}
#[derive(Serialize)]
struct Artifact {
    id: String,
    path: String,
    sha256: String,
    media_type: &'static str,
    role: ArtifactRole,
    shape: Vec<usize>,
    dtype: &'static str,
    coordinate_space: &'static str,
    #[serde(flatten)]
    metadata: BTreeMap<String, Value>,
}
#[derive(Serialize)]
struct Finding {
    id: String,
    case_id: Option<String>,
    region_id: String,
    metric: String,
    decision: &'static str,
    code: String,
    message: String,
    evidence: Vec<String>,
}
struct Evidence {
    out: PathBuf,
    case_id: Option<String>,
    seeds: Vec<Value>,
    measurements: Vec<Measurement>,
    findings: Vec<Finding>,
    artifacts: Vec<Artifact>,
    status: TerminalState,
    extra: BTreeMap<String, Value>,
}
impl Evidence {
    fn new(protocol: &Value, out: &Path) -> Self {
        Self {
            out: out.into(),
            case_id: protocol
                .get("case_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            seeds: protocol
                .get("seed_population")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            measurements: Vec::new(),
            findings: Vec::new(),
            artifacts: Vec::new(),
            status: TerminalState::Completed,
            extra: BTreeMap::new(),
        }
    }
    fn failure(
        &mut self,
        region: &str,
        metric: &str,
        error: &Error,
        refs: &[String],
        execution: bool,
    ) {
        self.findings.push(Finding {
            id: format!("{region}:{metric}:{}", error.code),
            case_id: self.case_id.clone(),
            region_id: region.into(),
            metric: metric.into(),
            decision: "unavailable",
            code: error.code.clone(),
            message: error.message.clone(),
            evidence: refs.to_vec(),
        });
        if execution {
            self.status = TerminalState::Failed;
        }
    }
    // Numeric payload is kept explicit at this boundary; grouping it would obscure metric schema fields.
    #[allow(clippy::too_many_arguments)]
    fn scalar(
        &mut self,
        region: &str,
        metric: &str,
        value: Option<f64>,
        count: usize,
        domain: &str,
        units: &str,
        estimator: &str,
        refs: &[String],
        reason: Option<&str>,
    ) {
        let valid = reason.is_none() && value.is_some_and(f64::is_finite);
        if !valid && reason.is_none() {
            self.failure(
                region,
                metric,
                &err("computation_failure", "Scalar is nonfinite"),
                refs,
                true,
            );
        }
        self.measurements.push(Measurement {
            case_id: self.case_id.clone(),
            seed_population: self.seeds.clone(),
            region_id: region.into(),
            metric: metric.into(),
            metric_version: METRIC_VERSION,
            domain: domain.into(),
            units: units.into(),
            estimator: estimator.into(),
            sample_count: count,
            artifact_refs: refs.to_vec(),
            availability: if valid {
                Availability::Available
            } else {
                Availability::Unavailable
            },
            value: if valid { value } else { None },
            reason: if valid {
                None
            } else {
                Some(reason.unwrap_or("computation_failure").into())
            },
        });
    }
    fn artifact(
        &mut self,
        name: &str,
        values: &[f64],
        shape: &[usize],
        role: ArtifactRole,
        metadata: Value,
    ) -> Result<String> {
        if !values.iter().all(|v| v.is_finite()) {
            return Err(err(
                "computation_failure",
                format!("Artifact contains nonfinite samples: {name}"),
            ));
        }
        fs::create_dir_all(&self.out)?;
        let filename = format!(
            "{}.npy",
            &format!("{:x}", Sha256::digest(name.as_bytes()))[..16]
        );
        let path = self.out.join(&filename);
        if path.exists() {
            return Err(err(
                "computation_failure",
                format!("Artifact already exists: {}", path.display()),
            ));
        }
        image_data::write_npy(&path, values, shape)?;
        let id = format!("{name}_artifact");
        self.artifacts.push(Artifact {
            id: id.clone(),
            path: filename,
            sha256: crate::common::sha256_file(&path)?,
            media_type: "application/x-npy",
            role,
            shape: shape.to_vec(),
            dtype: "float64",
            coordinate_space: "native_render",
            metadata: metadata
                .as_object()
                .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                .unwrap_or_default(),
        });
        Ok(id)
    }
    fn finish(self) -> Value {
        let mut result = json!({"measurements":self.measurements,"findings":self.findings,"artifacts":self.artifacts,"numeric_health":{},"execution_status":self.status,"metric_version":METRIC_VERSION,"color_transform":COLOR_TRANSFORM});
        for (key, value) in self.extra {
            result[key] = value;
        }
        result
    }
}
// Numeric metric sides share schema fields; explicit arguments preserve the frozen output mapping.
#[allow(clippy::too_many_arguments)]
fn paired(
    e: &mut Evidence,
    region: &str,
    prefix: &str,
    name: &str,
    left: Option<f64>,
    right: Option<f64>,
    count: usize,
    domain: &str,
    units: &str,
    estimator: &str,
    refs: &[String],
) {
    let reason = if name.contains("correlation") || name.contains("frequency_centroid") {
        "zero_variance"
    } else if name.contains("half_height") {
        "no_half_height_crossing"
    } else if name.contains("mtf50") {
        "no_mtf50_crossing"
    } else {
        "insufficient_evidence"
    };
    for (side, value) in [
        ("baseline", left),
        ("candidate", right),
        ("delta", left.zip(right).map(|(a, b)| b - a)),
    ] {
        e.scalar(
            region,
            &format!("{prefix}.{side}.{name}"),
            value,
            count,
            domain,
            units,
            &format!("{estimator}; delta=candidate-baseline"),
            refs,
            if value.is_none() { Some(reason) } else { None },
        );
    }
}
// Summary fields map directly to the serialized metric schema.
#[allow(clippy::too_many_arguments)]
fn summary(
    e: &mut Evidence,
    region: &str,
    name: &str,
    mut values: Vec<f64>,
    refs: &[String],
    domain: &str,
    units: &str,
    average: bool,
) {
    let avg = mean(&values);
    sort(&mut values);
    for (label, value) in [
        ("p95", quantile(&values, 0.95)),
        ("p99", quantile(&values, 0.99)),
        ("max", values.last().copied()),
        (
            if average { "mean" } else { "median" },
            if average { avg } else { quantile(&values, 0.5) },
        ),
    ] {
        e.scalar(
            region,
            &format!("{name}.{label}"),
            value,
            values.len(),
            domain,
            units,
            &format!("region pixel {label}; quantile method=linear"),
            refs,
            None,
        );
    }
}
fn health(samples: &[[f64; 3]], dimensions: [usize; 2]) -> Value {
    let mut extrema = Vec::new();
    let mut nonfinite = 0;
    let mut nonfinite_pixels = 0;
    let mut below = 0;
    let mut above = 0;
    let mut mins = [f64::INFINITY; 3];
    let mut maxs = [f64::NEG_INFINITY; 3];
    let mut ns = [0; 3];
    for p in samples {
        let mut bad = false;
        for c in 0..3 {
            let x = p[c];
            if !x.is_finite() {
                nonfinite += 1;
                bad = true;
            } else {
                ns[c] += 1;
                mins[c] = mins[c].min(x);
                maxs[c] = maxs[c].max(x);
                below += usize::from(x < 0.0);
                above += usize::from(x > 1.0);
            }
        }
        nonfinite_pixels += usize::from(bad);
    }
    for c in 0..3 {
        let mut row = json!({"channel":(["R","G","B"][c]),"sample_count":ns[c],"availability":if ns[c]>0{"available"}else{"unavailable"}});
        if ns[c] > 0 {
            row["min"] = json!(mins[c]);
            row["max"] = json!(maxs[c]);
        } else {
            row["reason"] = json!("no_finite_samples");
        }
        extrema.push(row);
    }
    json!({"dimensions":dimensions,"sample_count":samples.len()*3,"finite_sample_count":samples.len()*3-nonfinite,"nonfinite_sample_count":nonfinite,"nonfinite_pixel_count":nonfinite_pixels,"all_finite":nonfinite==0,"channel_extrema":extrema,"below_zero_fraction":below as f64/(samples.len()*3) as f64,"above_one_fraction":above as f64/(samples.len()*3) as f64,"range_fraction_denominator":"all_native_channel_samples"})
}

fn periodic_hann(index: usize, length: usize) -> f64 {
    if length == 1 {
        1.0
    } else {
        0.5 - 0.5 * (2.0 * std::f64::consts::PI * index as f64 / length as f64).cos()
    }
}
fn fft2(
    values: &mut [Complex<f64>],
    width: usize,
    height: usize,
    inverse: bool,
    planner: &mut FftPlanner<f64>,
) {
    let row = if inverse {
        planner.plan_fft_inverse(width)
    } else {
        planner.plan_fft_forward(width)
    };
    let column = if inverse {
        planner.plan_fft_inverse(height)
    } else {
        planner.plan_fft_forward(height)
    };
    let mut scratch = vec![
        Complex::default();
        row.get_inplace_scratch_len()
            .max(column.get_inplace_scratch_len())
    ];
    for line in values.chunks_exact_mut(width) {
        row.process_with_scratch(line, &mut scratch);
    }
    let mut line = vec![Complex::default(); height];
    for x in 0..width {
        for y in 0..height {
            line[y] = values[y * width + x];
        }
        column.process_with_scratch(&mut line, &mut scratch);
        for y in 0..height {
            values[y * width + x] = line[y];
        }
    }
}
fn frequency(index: usize, length: usize) -> f64 {
    // np.fft.fftfreq multiplies the integer lattice by one precomputed reciprocal.
    // Division per coordinate changes rounding at radial annulus boundaries.
    (index as isize - (length / 2) as isize) as f64 * (1.0 / length as f64)
}
struct GrainRetained {
    psd: Vec<f64>,
    radial: Vec<f64>,
    autocovariance: Vec<f64>,
    residual: Vec<f64>,
}
// Grain settings and image dimensions are numeric protocol payload; keep this call explicit.
#[allow(clippy::too_many_arguments)]
fn grain(
    e: &mut Evidence,
    region: &str,
    mask: &[bool],
    images: &[Vec<[f64; 3]>; 2],
    width: usize,
    height: usize,
    settings: &Value,
    flat: bool,
) -> Result<()> {
    let c = crop(mask, width, height)?;
    let (w, h) = (c.width, c.height);
    let mut local = cropped_mask(mask, width, c);
    let residual_settings = &settings["residual"];
    let border = residual_settings
        .get("border_px")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    erode(&mut local, w, h, border);
    let n = count(&local);
    if n < 4 {
        return Err(err(
            "insufficient_samples",
            "Grain requires at least four selected residual pixels",
        ));
    }
    let requested = residual_settings
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or(if flat {
            "flat_mean"
        } else {
            "gaussian_high_pass"
        });
    let method = if requested == "flat_mean" && !flat {
        "gaussian_high_pass"
    } else {
        requested
    };
    if !["flat_mean", "gaussian_high_pass"].contains(&method) {
        return Err(err("invalid_protocol", "Unknown grain detrending method"));
    }
    let radius = residual_settings
        .get("high_pass_radius_px")
        .and_then(Value::as_f64)
        .unwrap_or(3.0);
    let window_name = settings["spectrum"]
        .get("window")
        .and_then(Value::as_str)
        .unwrap_or("hann")
        .to_lowercase();
    if !["hann", "boxcar", "rectangular", "none"].contains(&window_name.as_str()) {
        return Err(err(
            "invalid_protocol",
            "Grain window must be hann or boxcar",
        ));
    }
    let mut window = vec![0.0; w * h];
    for y in 0..h {
        for x in 0..w {
            window[y * w + x] = if local[y * w + x] {
                if window_name == "hann" {
                    periodic_hann(y, h) * periodic_hann(x, w)
                } else {
                    1.0
                }
            } else {
                0.0
            };
        }
    }
    let normalization = window.iter().map(|x| x * x).sum::<f64>();
    if normalization == 0.0 {
        return Err(err("insufficient_samples", "Grain window has zero energy"));
    }
    let fx: Vec<f64> = (0..w).map(|x| frequency(x, w)).collect();
    let fy: Vec<f64> = (0..h).map(|y| frequency(y, h)).collect();
    let edge_count = 3.max(w.min(h) / 2 + 1);
    let bin_width = 0.5_f64.sqrt() / (edge_count - 1) as f64;
    let mut refs = Vec::new();
    let mut retained = Vec::new();
    let mut statistics = Vec::new();
    let mut planner = FftPlanner::new();
    for (side, image) in ["baseline", "candidate"].into_iter().zip(images) {
        let mut channels = vec![[0.0; 4]; w * h];
        for channel in 0..3 {
            let source: Vec<f64> = (c.y..c.y + h)
                .flat_map(|y| (c.x..c.x + w).map(move |x| image[y * width + x][channel]))
                .collect();
            if method == "flat_mean" {
                let average = source
                    .iter()
                    .zip(&local)
                    .filter_map(|(x, m)| m.then_some(*x))
                    .sum::<f64>()
                    / n as f64;
                for i in 0..source.len() {
                    channels[i][channel] = source[i] - average;
                }
            } else {
                let smooth = gaussian(&source, w, h, radius);
                for i in 0..source.len() {
                    channels[i][channel] = source[i] - smooth[i];
                }
            }
        }
        for p in &mut channels {
            p[3] = luminance([p[0], p[1], p[2]]);
        }
        let mut stats = Vec::<(String, Option<f64>)>::new();
        for (channel, label) in ["R", "G", "B", "luminance"].iter().enumerate() {
            stats.push((
                format!("rms.{label}"),
                Some(
                    (channels
                        .iter()
                        .zip(&local)
                        .filter_map(|(p, m)| m.then_some(p[channel] * p[channel]))
                        .sum::<f64>()
                        / n as f64)
                        .sqrt(),
                ),
            ));
        }
        let chromatic = channels
            .iter()
            .zip(&local)
            .filter(|(_, m)| **m)
            .map(|(p, _)| (p[0] - p[3]).powi(2) + (p[1] - p[3]).powi(2) + (p[2] - p[3]).powi(2))
            .sum::<f64>();
        stats.push((
            "rms.chromatic".into(),
            Some((chromatic / (n * 3) as f64).sqrt()),
        ));
        let means: [f64; 3] = std::array::from_fn(|channel| {
            channels
                .iter()
                .zip(&local)
                .filter_map(|(p, m)| m.then_some(p[channel]))
                .sum::<f64>()
                / n as f64
        });
        let mut covariance = [[0.0; 3]; 3];
        for (p, m) in channels.iter().zip(&local) {
            if *m {
                for i in 0..3 {
                    for j in 0..3 {
                        covariance[i][j] += (p[i] - means[i]) * (p[j] - means[j]);
                    }
                }
            }
        }
        for row in &mut covariance {
            for x in row {
                *x /= n as f64;
            }
        }
        for i in 0..3 {
            for j in i + 1..3 {
                let labels = format!("{}{}", ["R", "G", "B"][i], ["R", "G", "B"][j]);
                stats.push((format!("covariance.{labels}"), Some(covariance[i][j])));
                let denominator = (covariance[i][i] * covariance[j][j]).sqrt();
                stats.push((
                    format!("correlation.{labels}"),
                    if denominator > 0.0 {
                        Some(covariance[i][j] / denominator)
                    } else {
                        None
                    },
                ));
            }
        }
        let mut psd = vec![0.0; w * h * 4];
        let mut autocovariance = vec![0.0; w * h * 4];
        let mut transform = vec![Complex::default(); w * h];
        for channel in 0..4 {
            for i in 0..w * h {
                transform[i] = Complex::new(channels[i][channel] * window[i], 0.0);
            }
            fft2(&mut transform, w, h, false, &mut planner);
            for y in 0..h {
                for x in 0..w {
                    let source = ((y + h.div_ceil(2)) % h) * w + (x + w.div_ceil(2)) % w;
                    psd[(y * w + x) * 4 + channel] = transform[source].norm_sqr() / normalization;
                }
            }
            for value in &mut transform {
                *value = Complex::new(value.norm_sqr(), 0.0);
            }
            fft2(&mut transform, w, h, true, &mut planner);
            for y in 0..h {
                for x in 0..w {
                    let source = ((y + h.div_ceil(2)) % h) * w + (x + w.div_ceil(2)) % w;
                    autocovariance[(y * w + x) * 4 + channel] =
                        transform[source].re / (normalization * (w * h) as f64);
                }
            }
        }
        let metadata = json!({"domain":"linear_srgb","units":"sample squared / cycles per pixel","window":window_name,"normalization":"abs(FFT(residual*mask*window))^2 / sum((mask*window)^2)","valid_dimensions":[w,h],"valid_sample_count":n,"residual":{"method":method,"sigma_px":if method=="gaussian_high_pass"{Some(radius)}else{None},"border_px":border},"channels":["R","G","B","luminance"]});
        refs.push(e.artifact(
            &format!("grain.{region}.{side}.psd2d"),
            &psd,
            &[h, w, 4],
            ArtifactRole::GrainSpectrum,
            metadata.clone(),
        )?);
        refs.push(e.artifact(
            &format!("grain.{region}.{side}.frequency_x"),
            &fx,
            &[w],
            ArtifactRole::SpectrumCoordinates,
            json!({"units":"cycles/pixel"}),
        )?);
        refs.push(e.artifact(
            &format!("grain.{region}.{side}.frequency_y"),
            &fy,
            &[h],
            ArtifactRole::SpectrumCoordinates,
            json!({"units":"cycles/pixel"}),
        )?);
        let mut powers = vec![[0.0; 4]; edge_count - 1];
        let mut populations = vec![0usize; edge_count - 1];
        let edges: Vec<f64> = (0..edge_count)
            .map(|i| {
                if i + 1 == edge_count {
                    0.5_f64.sqrt()
                } else {
                    i as f64 * bin_width
                }
            })
            .collect();
        for y in 0..h {
            for x in 0..w {
                let radius = fy[y].hypot(fx[x]);
                let upper = edges.partition_point(|edge| *edge <= radius);
                if upper > 0 && upper < edge_count {
                    let bin = upper - 1;
                    populations[bin] += 1;
                    for channel in 0..4 {
                        powers[bin][channel] += psd[(y * w + x) * 4 + channel];
                    }
                }
            }
        }
        let mut radial = Vec::new();
        for bin in 0..edge_count - 1 {
            let population = populations[bin];
            if population > 0 {
                radial.push((edges[bin] + edges[bin + 1]) / 2.0);
                radial.extend(powers[bin].map(|x| x / population as f64));
                radial.push(population as f64);
            }
        }
        let mut radial_metadata = metadata;
        radial_metadata["columns"] = json!([
            "frequency_cycles_per_pixel",
            "R",
            "G",
            "B",
            "luminance",
            "bin_count"
        ]);
        refs.push(e.artifact(
            &format!("grain.{region}.{side}.radial_psd"),
            &radial,
            &[radial.len() / 6, 6],
            ArtifactRole::GrainRadialSpectrum,
            radial_metadata,
        )?);
        refs.push(e.artifact(
            &format!("grain.{region}.{side}.autocovariance"),
            &autocovariance,
            &[h, w, 4],
            ArtifactRole::GrainAutocovariance,
            json!({"units":"sample squared","lag_origin_px":[w/2,h/2]}),
        )?);
        let origin = ((h / 2) * w + w / 2) * 4;
        let valid: Vec<usize> = (0..4)
            .filter(|channel| autocovariance[origin + channel] > 0.0)
            .collect();
        if !valid.is_empty() {
            let mut acf = Vec::with_capacity(w * h * valid.len());
            for pixel in 0..w * h {
                for &channel in &valid {
                    acf.push(
                        autocovariance[pixel * 4 + channel] / autocovariance[origin + channel],
                    );
                }
            }
            refs.push(e.artifact(&format!("grain.{region}.{side}.acf"),&acf,&[h,w,valid.len()],ArtifactRole::GrainAutocorrelation,json!({"units":"normalized correlation","channels":valid.iter().map(|c|["R","G","B","luminance"][*c]).collect::<Vec<_>>(),"lag_origin_px":[w/2,h/2],"estimator":"windowed cyclic autocovariance / zero lag"}))?);
        }
        let mut residual = Vec::with_capacity(w * h * 3);
        for (p, m) in channels.iter().zip(&local) {
            for value in p.iter().take(3) {
                residual.push(if *m { *value } else { 0.0 });
            }
        }
        refs.push(e.artifact(
            &format!("grain.{region}.{side}.residual"),
            &residual,
            &[h, w, 3],
            ArtifactRole::GrainResidual,
            json!({"domain":"linear_srgb","mask_count":n}),
        )?);
        stats.push((
            "psd.luminance_total".into(),
            Some(psd.as_chunks::<4>().0.iter().map(|p| p[3]).sum::<f64>() / h as f64 / w as f64),
        ));
        let power = radial
            .as_chunks::<6>()
            .0
            .iter()
            .map(|row| row[4])
            .sum::<f64>();
        stats.push((
            "frequency_centroid_cycles_per_pixel".into(),
            if power > 0.0 {
                Some(
                    radial
                        .as_chunks::<6>()
                        .0
                        .iter()
                        .map(|row| row[0] * row[4])
                        .sum::<f64>()
                        / power,
                )
            } else {
                None
            },
        ));
        retained.push(GrainRetained {
            psd,
            radial,
            autocovariance,
            residual,
        });
        statistics.push(stats);
    }
    for (name, a, b, shape) in [
        ("psd2d", &retained[0].psd, &retained[1].psd, vec![h, w, 4]),
        (
            "radial_psd",
            &retained[0].radial,
            &retained[1].radial,
            vec![retained[0].radial.len() / 6, 6],
        ),
        (
            "autocovariance",
            &retained[0].autocovariance,
            &retained[1].autocovariance,
            vec![h, w, 4],
        ),
        (
            "residual",
            &retained[0].residual,
            &retained[1].residual,
            vec![h, w, 3],
        ),
    ] {
        let mut delta: Vec<f64> = b.iter().zip(a).map(|(b, a)| b - a).collect();
        if name == "radial_psd" {
            for (row, original) in delta
                .as_chunks_mut::<6>()
                .0
                .iter_mut()
                .zip(a.as_chunks::<6>().0)
            {
                row[0] = original[0];
                row[5] = original[5];
            }
        }
        refs.push(e.artifact(&format!("grain.{region}.delta.{name}"),&delta,&shape,ArtifactRole::GrainSignedDifference,json!({"domain":"linear_srgb","estimator":format!("candidate minus baseline {name}"),"residual_method":method}))?);
    }
    for ((name, left), (_, right)) in statistics[0].iter().zip(&statistics[1]) {
        let units = if name.starts_with("correlation") {
            "correlation"
        } else if name.contains("frequency") {
            "cycles/pixel"
        } else if name.starts_with("covariance") || name.starts_with("psd") {
            "sample squared"
        } else {
            "linear-sRGB sample RMS"
        };
        paired(
            e,
            region,
            &format!("grain.{method}"),
            name,
            *left,
            *right,
            n,
            "linear_srgb",
            units,
            &format!(
                "{method} residual; border={border}px; population covariance ddof=0; RMS=sqrt(mean(residual^2))"
            ),
            &refs,
        );
    }
    Ok(())
}

fn halation(
    e: &mut Evidence,
    region: &str,
    mask: &[bool],
    images: &[Vec<[f64; 3]>; 2],
    width: usize,
    height: usize,
    settings: &Value,
) -> Result<()> {
    for key in [
        "center_px",
        "core_radius_px",
        "background_inner_radius_px",
        "background_outer_radius_px",
        "extent_px",
    ] {
        if settings.get(key).is_none() {
            return Err(err(
                "missing_probe_geometry",
                "Halation requires frozen center, core, background annulus, and extent",
            ));
        }
    }
    let center = settings["center_px"]
        .as_array()
        .filter(|c| c.len() == 2)
        .ok_or_else(|| {
            err(
                "invalid_probe_geometry",
                "Halation center requires two coordinates",
            )
        })?;
    let cx = center[0]
        .as_f64()
        .ok_or_else(|| err("invalid_probe_geometry", "Invalid center x"))?;
    let cy = center[1]
        .as_f64()
        .ok_or_else(|| err("invalid_probe_geometry", "Invalid center y"))?;
    let core_radius = number(settings, "core_radius_px")?;
    let extent = number(settings, "extent_px")?;
    let inner = number(settings, "background_inner_radius_px")?;
    let outer = number(settings, "background_outer_radius_px")?;
    let bin_width = settings
        .get("bin_width_px")
        .and_then(Value::as_f64)
        .unwrap_or(1.0);
    if bin_width <= 0.0 || !bin_width.is_finite() {
        return Err(err(
            "invalid_protocol",
            "Halation bin width must be positive",
        ));
    }
    let radius: Vec<f64> = (0..width * height)
        .map(|i| (i as f64 % width as f64 - cx).hypot((i / width) as f64 - cy))
        .collect();
    let background: Vec<usize> = (0..mask.len())
        .filter(|i| mask[*i] && radius[*i] >= inner && radius[*i] <= outer)
        .collect();
    let spill: Vec<usize> = (0..mask.len())
        .filter(|i| mask[*i] && radius[*i] > core_radius && radius[*i] <= extent)
        .collect();
    let core: Vec<usize> = (0..mask.len())
        .filter(|i| mask[*i] && radius[*i] <= core_radius)
        .collect();
    if background.is_empty() || spill.is_empty() || core.is_empty() {
        return Err(err(
            "invalid_probe_geometry",
            "Halation core, background, and spill must contain frozen pixels",
        ));
    }
    let mut profiles = Vec::new();
    let mut stats = Vec::new();
    let mut refs = Vec::new();
    let bins = (extent / bin_width).ceil() as usize;
    for (side, image) in ["baseline", "candidate"].into_iter().zip(images) {
        let mut background_median = [0.0; 3];
        for channel in 0..3 {
            let mut samples: Vec<f64> = background.iter().map(|i| image[*i][channel]).collect();
            background_median[channel] = median(&mut samples).unwrap();
        }
        for axis in ["x", "y"] {
            let mut rows = Vec::new();
            let normal_bins = (2.0 * extent / bin_width).ceil() as usize;
            for bin in 0..normal_bins {
                let lo = -extent + bin as f64 * bin_width;
                let mut sums = [0.0; 3];
                let mut n = 0;
                for i in 0..mask.len() {
                    if mask[i] {
                        let dx = (i % width) as f64 - cx;
                        let dy = (i / width) as f64 - cy;
                        let (displacement, transverse) = if axis == "x" {
                            (dx, dy.abs())
                        } else {
                            (dy, dx.abs())
                        };
                        if transverse <= bin_width / 2.0
                            && displacement >= lo
                            && displacement < lo + bin_width
                        {
                            n += 1;
                            for channel in 0..3 {
                                sums[channel] += image[i][channel] - background_median[channel];
                            }
                        }
                    }
                }
                if n > 0 {
                    let rgb = sums.map(|x| x / n as f64);
                    rows.extend_from_slice(&[
                        lo + bin_width / 2.0,
                        rgb[0],
                        rgb[1],
                        rgb[2],
                        rgb[0] - rgb[1],
                        n as f64,
                    ]);
                }
            }
            if !rows.is_empty() {
                refs.push(e.artifact(&format!("halation.{region}.{side}.normal_{axis}"),&rows,&[rows.len()/6,6],ArtifactRole::HalationEdgeSpillProfile,json!({"columns":["signed_displacement_px","R_excess","G_excess","B_excess","red_minus_green","count"],"domain":"linear_srgb","estimator":"frozen center strip; per-channel median annulus background subtraction","source_core_radius_px":core_radius}))?);
            }
        }
        let mut profile = Vec::new();
        for bin in 0..bins {
            let lo = bin as f64 * bin_width;
            let hi = (bin + 1) as f64 * bin_width;
            let mut sum = [0.0; 3];
            let mut n = 0;
            for i in 0..mask.len() {
                if mask[i] && radius[i] >= lo && radius[i] < hi {
                    n += 1;
                    for channel in 0..3 {
                        sum[channel] += image[i][channel] - background_median[channel];
                    }
                }
            }
            if n > 0 {
                let rgb = sum.map(|x| x / n as f64);
                profile.extend_from_slice(&[
                    (lo + hi) / 2.0,
                    rgb[0],
                    rgb[1],
                    rgb[2],
                    rgb[0] - rgb[1],
                    n as f64,
                ]);
            }
        }
        refs.push(e.artifact(&format!("halation.{region}.{side}.radial"),&profile,&[profile.len()/6,6],ArtifactRole::HalationProfile,json!({"columns":["radius_px","R_excess","G_excess","B_excess","red_minus_green","count"],"domain":"linear_srgb","background":"per-channel median frozen annulus","source_core_radius_px":core_radius}))?);
        let mut values = vec![(
            "core_clipped_fraction".to_string(),
            Some(
                core.iter()
                    .filter(|i| image[**i].iter().any(|x| *x >= 1.0))
                    .count() as f64
                    / core.len() as f64,
            ),
        )];
        let outside: Vec<&[f64]> = profile
            .as_chunks::<6>()
            .0
            .iter()
            .filter(|row| row[0] > core_radius)
            .map(|row| &row[..])
            .collect();
        if outside.is_empty() {
            return Err(err(
                "computation_failure",
                "Halation radial profile has no bins outside source core",
            ));
        }
        for (channel, label) in ["R", "G", "B"].iter().enumerate() {
            let mut peak_index = 0;
            for index in 1..outside.len() {
                if outside[index][channel + 1] > outside[peak_index][channel + 1] {
                    peak_index = index;
                }
            }
            let peak = outside[peak_index][channel + 1];
            let half = if peak > 0.0 {
                outside[peak_index..]
                    .iter()
                    .find(|row| row[channel + 1] <= peak / 2.0)
                    .map(|row| row[0] - outside[peak_index][0])
            } else {
                None
            };
            values.push((format!("{label}.peak_amplitude"), Some(peak)));
            values.push((
                format!("{label}.peak_radius_px"),
                Some(outside[peak_index][0]),
            ));
            values.push((format!("{label}.half_height_width_px"), half));
            values.push((
                format!("{label}.integrated_spill"),
                Some(
                    spill
                        .iter()
                        .map(|i| image[*i][channel] - background_median[channel])
                        .sum(),
                ),
            ));
        }
        values.push((
            "red_minus_green.integrated_spill".into(),
            Some(
                spill
                    .iter()
                    .map(|i| {
                        image[*i][0] - background_median[0] - image[*i][1] + background_median[1]
                    })
                    .sum(),
            ),
        ));
        profiles.push(profile);
        stats.push(values);
    }
    let mut delta = profiles[1].clone();
    for (row, left) in delta
        .as_chunks_mut::<6>()
        .0
        .iter_mut()
        .zip(profiles[0].as_chunks::<6>().0)
    {
        for channel in 1..5 {
            row[channel] -= left[channel];
        }
    }
    refs.push(e.artifact(&format!("halation.{region}.delta.radial"),&delta,&[delta.len()/6,6],ArtifactRole::HalationProfile,json!({"columns":["radius_px","R_delta","G_delta","B_delta","red_minus_green_delta","count"],"domain":"linear_srgb"}))?);
    for ((name, left), (_, right)) in stats[0].iter().zip(&stats[1]) {
        let units = if name.ends_with("_px") {
            "pixels"
        } else if name.contains("fraction") {
            "fraction"
        } else if name.contains("spill") {
            "linear-sRGB sample pixels"
        } else {
            "linear-sRGB sample"
        };
        let n = if name == "core_clipped_fraction" {
            core.len()
        } else {
            spill.len()
        };
        let estimator = if name == "core_clipped_fraction" {
            "fraction of frozen core pixels with any linear-sRGB channel >= 1"
        } else {
            "frozen radial bins; peak outside source core; half-height first outward crossing; signed excess integral"
        };
        paired(
            e,
            region,
            "halation",
            name,
            *left,
            *right,
            n,
            "linear_srgb",
            units,
            estimator,
            &refs,
        );
    }
    Ok(())
}
fn gradient(values: &[f64], coordinates: &[f64]) -> Vec<f64> {
    let n = values.len();
    let mut result = vec![0.0; n];
    result[0] = (values[1] - values[0]) / (coordinates[1] - coordinates[0]);
    result[n - 1] = (values[n - 1] - values[n - 2]) / (coordinates[n - 1] - coordinates[n - 2]);
    for i in 1..n - 1 {
        let before = coordinates[i] - coordinates[i - 1];
        let after = coordinates[i + 1] - coordinates[i];
        result[i] = -after / (before * (before + after)) * values[i - 1]
            + (after - before) / (before * after) * values[i]
            + before / (after * (before + after)) * values[i + 1];
    }
    result
}
fn mtf50(sfr: &[f64], sample_count: usize) -> Option<f64> {
    let index = sfr.iter().position(|v| *v <= 0.5)?;
    if index == 0 {
        return None;
    }
    Some(
        ((index - 1) as f64 + (0.5 - sfr[index - 1]) / (sfr[index] - sfr[index - 1]))
            / sample_count as f64,
    )
}
fn edge(
    e: &mut Evidence,
    region: &str,
    mask: &[bool],
    images: &[Vec<[f64; 3]>; 2],
    width: usize,
    height: usize,
    settings: &Value,
) -> Result<()> {
    if ["axis", "input_contrast", "input_exposure_stops"]
        .iter()
        .any(|k| settings.get(*k).is_none())
    {
        return Err(err(
            "missing_probe_geometry",
            "Edge needs frozen normal axis, input contrast, and exposure",
        ));
    }
    let axis = settings["axis"].as_str().unwrap_or("");
    if axis != "x" && axis != "y" {
        return Err(err("invalid_protocol", "Edge axis must be x or y"));
    }
    let c = crop(mask, width, height)?;
    let local = cropped_mask(mask, width, c);
    let (normal, transverse) = if axis == "x" {
        (c.width, c.height)
    } else {
        (c.height, c.width)
    };
    let index = |position: usize, line: usize| {
        if axis == "x" {
            line * c.width + position
        } else {
            position * c.width + line
        }
    };
    let mut positions = Vec::new();
    let mut counts = Vec::new();
    for position in 0..normal {
        let n = (0..transverse)
            .filter(|line| local[index(position, *line)])
            .count();
        if n > 0 {
            positions.push(position);
            counts.push(n);
        }
    }
    let coordinates: Vec<f64> = positions.iter().map(|x| *x as f64).collect();
    if coordinates.len() < 8 {
        return Err(err(
            "insufficient_samples",
            "Edge requires eight normal profile samples",
        ));
    }
    let n = coordinates.len();
    let plateau = 2
        .max(
            (n as f64
                * settings
                    .get("plateau_fraction")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.2)) as usize,
        )
        .min(n);
    let mut statistics = Vec::new();
    let mut esfs = Vec::new();
    let mut refs = Vec::new();
    for (side, image) in ["baseline", "candidate"].into_iter().zip(images) {
        let mut profile = Vec::new();
        for (&position, &population) in positions.iter().zip(&counts) {
            let mut total = 0.0;
            for line in 0..transverse {
                if local[index(position, line)] {
                    let (x, y) = if axis == "x" {
                        (c.x + position, c.y + line)
                    } else {
                        (c.x + line, c.y + position)
                    };
                    total += luminance(image[y * width + x]);
                }
            }
            profile.push(total / population as f64);
        }
        let dark = median(&mut profile[..plateau].to_vec()).unwrap();
        let bright = median(&mut profile[n - plateau..].to_vec()).unwrap();
        let amplitude = bright - dark;
        if amplitude == 0.0 {
            return Err(err(
                "zero_edge_contrast",
                "Edge plateau luminances are equal",
            ));
        }
        let esf: Vec<f64> = profile.iter().map(|x| (x - dark) / amplitude).collect();
        let lsf = gradient(&esf, &coordinates);
        let mut spectrum: Vec<Complex<f64>> = lsf
            .iter()
            .enumerate()
            .map(|(i, x)| Complex::new(x * periodic_hann(i, n), 0.0))
            .collect();
        FftPlanner::<f64>::new()
            .plan_fft_forward(n)
            .process(&mut spectrum);
        let dc = spectrum[0].norm();
        if dc == 0.0 {
            return Err(err("zero_edge_contrast", "Edge LSF DC is zero"));
        }
        let sfr: Vec<f64> = spectrum[..n / 2 + 1]
            .iter()
            .map(|x| x.norm() / dc)
            .collect();
        let mut esf_rows = Vec::with_capacity(n * 4);
        for i in 0..n {
            esf_rows.extend_from_slice(&[coordinates[i], profile[i], esf[i], lsf[i]]);
        }
        refs.push(e.artifact(&format!("edge.{region}.{side}.esf"),&esf_rows,&[n,4],ArtifactRole::EdgeResponse,json!({"columns":["normal_position_px","linear_luminance","normalized_ESF","LSF_per_pixel"],"domain":"linear_srgb","input_contrast":settings["input_contrast"],"input_exposure_stops":settings["input_exposure_stops"]}))?);
        let mut sfr_rows = Vec::new();
        for (i, x) in sfr.iter().enumerate() {
            sfr_rows.extend_from_slice(&[i as f64 / n as f64, *x]);
        }
        refs.push(e.artifact(&format!("edge.{region}.{side}.sfr"),&sfr_rows,&[sfr.len(),2],ArtifactRole::EdgeSfr,json!({"columns":["cycles_per_pixel","SFR"],"estimator":"axis-normal mask-weighted ESF; gradient LSF; periodic Hann; abs(rFFT); normalize DC","interpretation":"end-to-end nonlinear response, not independent emulsion MTF"}))?);
        statistics.push([
            Some((esf.iter().copied().fold(f64::NEG_INFINITY, f64::max) - 1.0).max(0.0)),
            Some((-esf.iter().copied().fold(f64::INFINITY, f64::min)).max(0.0)),
            mtf50(&sfr, n),
            Some(amplitude),
        ]);
        esfs.push(esf);
    }
    let mut delta = Vec::new();
    for i in 0..n {
        delta.extend_from_slice(&[coordinates[i], esfs[1][i] - esfs[0][i]]);
    }
    refs.push(e.artifact(
        &format!("edge.{region}.delta.esf"),
        &delta,
        &[n, 2],
        ArtifactRole::EdgeResponse,
        json!({"columns":["normal_position_px","candidate_minus_baseline_normalized_ESF"]}),
    )?);
    for (i, name) in [
        "overshoot",
        "undershoot",
        "mtf50_cycles_per_pixel",
        "plateau_contrast",
    ]
    .iter()
    .enumerate()
    {
        let units = if name.contains("mtf") {
            "cycles/pixel"
        } else if *name == "plateau_contrast" {
            "linear luminance"
        } else {
            "normalized edge amplitude"
        };
        paired(
            e,
            region,
            "edge",
            name,
            statistics[0][i],
            statistics[1][i],
            count(mask),
            "linear_srgb",
            units,
            "fixed axis ESF; endpoint median plateaus; Hann-windowed gradient LSF SFR; linear first 0.5 crossing",
            &refs,
        );
    }
    Ok(())
}

// Tone protocol and image arrays are kept explicit to preserve the frozen numeric pipeline.
#[allow(clippy::too_many_arguments)]
fn tone(
    e: &mut Evidence,
    region: &str,
    mask: &[bool],
    labs: &[Vec<[f64; 3]>; 2],
    images: &[Vec<[f64; 3]>; 2],
    protocol: &Value,
    width: usize,
    height: usize,
    refs: &[String],
) -> Result<()> {
    let n = count(mask);
    let mut quantiles = [[0.0; 5]; 2];
    for side in 0..2 {
        let mut values: Vec<f64> = labs[side]
            .iter()
            .zip(mask)
            .filter_map(|(p, m)| m.then_some(p[0]))
            .collect();
        sort(&mut values);
        for (i, q) in [0.05, 0.25, 0.5, 0.75, 0.95].into_iter().enumerate() {
            quantiles[side][i] = quantile(&values, q)
                .ok_or_else(|| err("insufficient_samples", "Tone region has no selected pixels"))?;
        }
    }
    for (side, index) in [("baseline", 0), ("candidate", 1), ("delta", 2)] {
        for (i, percent) in [5, 25, 50, 75, 95].into_iter().enumerate() {
            let value = if index == 2 {
                quantiles[1][i] - quantiles[0][i]
            } else {
                quantiles[index][i]
            };
            e.scalar(
                region,
                &format!("tone.l_star.{side}.q{percent:02}"),
                Some(value),
                n,
                "bounded_srgb_d65_lab",
                "L*",
                "frozen-region lightness pixel quantile method=linear",
                refs,
                None,
            );
        }
    }
    if let Some(scales) = protocol
        .get("region_sampling_scales")
        .and_then(Value::as_array)
    {
        for scale in scales {
            let sigma = scale
                .as_f64()
                .ok_or_else(|| err("invalid_protocol", "Tone scale must be numeric"))?;
            let label = spelling(scale);
            let mut rms = [0.0; 2];
            let mut contrast_refs = Vec::new();
            for (index, side) in ["baseline", "candidate"].into_iter().enumerate() {
                let luminances: Vec<f64> = images[index].iter().map(|p| luminance(*p)).collect();
                let mut residual = gaussian(&luminances, width, height, sigma);
                for (value, original) in residual.iter_mut().zip(luminances) {
                    *value = original - *value;
                }
                rms[index] = (residual
                    .iter()
                    .zip(mask)
                    .filter_map(|(x, m)| m.then_some(x * x))
                    .sum::<f64>()
                    / n as f64)
                    .sqrt();
                contrast_refs.push(e.artifact(&format!("tone.{region}.{side}.local_contrast.{label}"),&residual,&[height,width],ArtifactRole::LocalContrastMap,json!({"domain":"linear_srgb_luminance","units":"linear luminance","sigma_px":sigma}))?);
            }
            paired(
                e,
                region,
                &format!("tone.local_contrast.sigma_{label}"),
                "rms",
                Some(rms[0]),
                Some(rms[1]),
                n,
                "linear_srgb_luminance",
                "linear luminance RMS",
                &format!("Gaussian high-pass reflect; sigma={label}px; selected frozen pixels RMS"),
                &contrast_refs,
            );
        }
    }
    let points = protocol["probe"]
        .get("tone_response")
        .and_then(Value::as_array);
    if region != "full_image" || points.is_none_or(Vec::is_empty) {
        return Ok(());
    }
    let points = points.unwrap();
    let coordinates: Vec<f64> = points
        .iter()
        .map(|p| number(p, "exposure_stops"))
        .collect::<Result<_>>()?;
    if coordinates.len() < 2 || coordinates.windows(2).any(|pair| pair[1] <= pair[0]) {
        return Err(err(
            "invalid_protocol",
            "Tone exposures must be strictly increasing with at least two points",
        ));
    }
    let mut responses = Vec::new();
    let mut curve_refs = Vec::new();
    for (side, index) in [("baseline", 0), ("candidate", 1)] {
        let mut response = Vec::new();
        let mut curve = Vec::new();
        for (point, &exposure) in points.iter().zip(&coordinates) {
            let geometry = point
                .get("geometry")
                .ok_or_else(|| err("invalid_probe_geometry", "Tone point requires geometry"))?;
            let mut selected_mask = geometry_mask(geometry, width, height)?;
            for (selected, m) in selected_mask.iter_mut().zip(mask) {
                *selected &= *m;
            }
            let population = count(&selected_mask);
            if population == 0 {
                return Err(err(
                    "invalid_probe_geometry",
                    "Tone response point has no selected pixels",
                ));
            }
            let mut y: Vec<f64> = images[index]
                .iter()
                .zip(&selected_mask)
                .filter_map(|(p, m)| m.then_some(luminance(*p)))
                .collect();
            let mut l: Vec<f64> = labs[index]
                .iter()
                .zip(&selected_mask)
                .filter_map(|(p, m)| m.then_some(p[0]))
                .collect();
            let y = median(&mut y).unwrap();
            let l = median(&mut l).unwrap();
            response.push([y, l]);
            curve.extend_from_slice(&[exposure, y, l, population as f64]);
        }
        curve_refs.push(e.artifact(&format!("tone.{region}.{side}.response"),&curve,&[points.len(),4],ArtifactRole::ToneResponse,json!({"columns":["input_exposure_stops","linear_luminance","L_star","pixel_count"],"estimator":"frozen patch median"}))?);
        responses.push(response);
    }
    let mut delta = Vec::new();
    for i in 0..points.len() {
        delta.extend_from_slice(&[
            coordinates[i],
            responses[1][i][0] - responses[0][i][0],
            responses[1][i][1] - responses[0][i][1],
        ]);
    }
    curve_refs.push(e.artifact(
        &format!("tone.{region}.delta.response"),
        &delta,
        &[points.len(), 3],
        ArtifactRole::ToneResponse,
        json!({"columns":["input_exposure_stops","delta_linear_luminance","delta_L_star"]}),
    )?);
    for i in 0..points.len() - 1 {
        let interval = coordinates[i + 1] - coordinates[i];
        for (channel, label) in ["linear_luminance", "l_star"].into_iter().enumerate() {
            let slope = |side: usize| {
                (responses[side][i + 1][channel] - responses[side][i][channel]) / interval
            };
            let domain = if channel == 0 {
                "linear_srgb_luminance"
            } else {
                "bounded_srgb_d65_lab"
            };
            let units = if channel == 0 {
                "linear luminance/stop"
            } else {
                "L*/stop"
            };
            paired(
                e,
                region,
                &format!("tone.response.interval_{i}.{label}"),
                "slope",
                Some(slope(0)),
                Some(slope(1)),
                2,
                domain,
                units,
                &format!(
                    "secant slope exposure interval [{:.1},{:.1}] stops",
                    coordinates[i],
                    coordinates[i + 1]
                ),
                &curve_refs,
            );
        }
    }
    Ok(())
}

struct Map {
    name: &'static str,
    units: &'static str,
    values: Vec<f64>,
    artifact: String,
}
fn numeric(e: &mut Evidence, region: &str, mask: &[bool], a: &ImageData, b: &ImageData) {
    let n = count(mask);
    let mut maximum = 0.0_f64;
    for (i, selected) in mask.iter().enumerate() {
        if *selected {
            for c in 0..3 {
                maximum = maximum.max((b.samples[i][c] - a.samples[i][c]).abs());
            }
        }
    }
    let mut square = 0.0;
    if maximum != 0.0 {
        for (i, selected) in mask.iter().enumerate() {
            if *selected {
                for c in 0..3 {
                    square += ((b.samples[i][c] - a.samples[i][c]) / maximum).powi(2);
                }
            }
        }
    }
    let rmse = if maximum == 0.0 {
        0.0
    } else {
        maximum * (square / (n * 3) as f64).sqrt()
    };
    let reason = if a.color_space == b.color_space && a.transfer == b.transfer {
        None
    } else {
        Some("color_declaration_mismatch")
    };
    for (name, value) in [("max_abs_difference", maximum), ("rmse", rmse)] {
        e.scalar(
            region,
            &format!("numeric.{name}"),
            Some(value),
            n * 3,
            &format!("native:{}:{}", a.color_space, a.transfer),
            "native channel sample",
            &format!("aligned native channel {name}"),
            &[],
            reason,
        );
    }
    for (side, image) in [("baseline", a), ("candidate", b)] {
        let mut below = 0;
        let mut above = 0;
        for (p, m) in image.samples.iter().zip(mask) {
            if *m {
                for x in p {
                    below += usize::from(*x < 0.0);
                    above += usize::from(*x > 1.0);
                }
            }
        }
        for (name, value) in [
            ("nonfinite_sample_count", 0.0),
            ("nonfinite_pixel_count", 0.0),
            ("below_zero_fraction", below as f64 / (n * 3) as f64),
            ("above_one_fraction", above as f64 / (n * 3) as f64),
        ] {
            e.scalar(
                region,
                &format!("numeric.{side}.{name}"),
                Some(value),
                if name.ends_with("pixel_count") {
                    n
                } else {
                    n * 3
                },
                "native",
                if name.ends_with("fraction") {
                    "fraction"
                } else {
                    "count"
                },
                "native all-channel sample population",
                &[],
                None,
            );
        }
    }
}
fn populations(regions: &[Value], protocol: &Value) -> Vec<Value> {
    let mut result = vec![
        json!({"id":"full_image","measurements":protocol.get("measurements").cloned().unwrap_or(json!(["numeric","color","tone"]))}),
    ];
    result.extend_from_slice(regions);
    result
}
fn compare_inner(
    e: &mut Evidence,
    a: &ImageData,
    b: &ImageData,
    populations: &[Value],
    protocol: &Value,
) -> Result<()> {
    e.extra.insert("numeric_health".into(),json!({"baseline":health(&a.samples,a.dimensions()),"candidate":health(&b.samples,b.dimensions())}));
    if a.dimensions() != b.dimensions() {
        return Err(err(
            "dimension_mismatch",
            "Masters differ in dimensions; no resize performed",
        ));
    }
    if a.samples
        .iter()
        .chain(&b.samples)
        .any(|p| p.iter().any(|x| !x.is_finite()))
    {
        return Err(err(
            "nonfinite_samples",
            "Native master contains nonfinite samples",
        ));
    }
    let (w, h) = (a.width, a.height);
    let encoded = [image_data::display_rgb(a)?, image_data::display_rgb(b)?];
    if encoded
        .iter()
        .flatten()
        .any(|p| p.iter().any(|x| !x.is_finite()))
    {
        return Err(err(
            "computation_failure",
            "Display transform produced nonfinite values",
        ));
    }
    let mut display_health = json!({});
    for (side, image) in ["baseline", "candidate"].into_iter().zip(&encoded) {
        let below = image.iter().flatten().filter(|x| **x < 0.0).count();
        let above = image.iter().flatten().filter(|x| **x > 1.0).count();
        let total = image.len() * 3;
        display_health[side] = json!({"below_zero_fraction":below as f64/total as f64,"above_one_fraction":above as f64/total as f64,"clipped_sample_fraction":(below+above) as f64/total as f64,"sample_count":total,"domain":"unclipped_encoded_srgb"});
    }
    e.extra.insert("display_health".into(), display_health);
    let labs = std::array::from_fn::<_, 2, _>(|side| {
        let mut lab = Vec::with_capacity(w * h);
        for chunk in encoded[side].chunks(65536) {
            let bounded: Vec<[f64; 3]> =
                chunk.iter().map(|p| p.map(|x| x.clamp(0.0, 1.0))).collect();
            lab.extend(image_data::rgb_to_lab(&bounded));
        }
        lab
    });
    let mut maps = Vec::new();
    for (name, units, component) in [
        ("delta_e00", "delta E 00", None),
        ("delta_l_star", "L*", Some(0)),
        ("delta_a_star", "a*", Some(1)),
        ("delta_b_star", "b*", Some(2)),
        ("delta_c_star", "C*", Some(3)),
    ] {
        let values: Vec<f64> = labs[0]
            .iter()
            .zip(&labs[1])
            .map(|(a, b)| match component {
                None => image_data::delta_e_2000(*a, *b),
                Some(3) => b[1].hypot(b[2]) - a[1].hypot(a[2]),
                Some(c) => b[c] - a[c],
            })
            .collect();
        let artifact = e.artifact(
            name,
            &values,
            &[h, w],
            ArtifactRole::DifferenceMap,
            json!({"metric":name,"domain":"bounded_srgb_d65_lab","units":units,"dimensions":[w,h]}),
        )?;
        maps.push(Map {
            name,
            units,
            values,
            artifact,
        });
    }
    let has_family = |family: &str| {
        populations.iter().any(|p| {
            p["measurements"]
                .as_array()
                .is_some_and(|x| x.iter().any(|x| x.as_str() == Some(family)))
        })
    };
    if has_family("flip") {
        let ppd = number(protocol, "pixels_per_degree")?;
        let values = image_data::official_flip(&encoded[0], &encoded[1], w, h, ppd)?;
        let artifact=e.artifact("flip",&values,&[h,w],ArtifactRole::DifferenceMap,json!({"metric":"flip","domain":"bounded_encoded_srgb","units":"FLIP error","pixels_per_degree":protocol["pixels_per_degree"],"implementation":"NVlabs flip-evaluator 1.7","output_size":[w,h]}))?;
        maps.push(Map {
            name: "flip",
            units: "FLIP error",
            values,
            artifact,
        });
        e.extra.insert("flip_parameters".into(), json!({"ppd":ppd}));
    }
    let linear = if ["tone", "grain", "halation", "edge"]
        .iter()
        .any(|f| has_family(f))
    {
        Some(std::array::from_fn::<_, 2, _>(|side| {
            encoded[side]
                .iter()
                .map(|p| p.map(image_data::srgb_decode))
                .collect::<Vec<_>>()
        }))
    } else {
        None
    };
    drop(encoded);
    let refs: Vec<String> = maps.iter().map(|m| m.artifact.clone()).collect();
    let mut seen = HashSet::new();
    for population in populations {
        let region = population["id"].as_str().unwrap_or("");
        let mut active = "region";
        let execution = (|| -> Result<()> {
            if !seen.insert(region) {
                return Err(err(
                    "invalid_region",
                    "Duplicate or reserved full_image region",
                ));
            }
            let mask = if region == "full_image" {
                vec![true; w * h]
            } else {
                geometry_mask(&population["geometry"], w, h)?
            };
            let n = count(&mask);
            if n == 0 {
                return Err(err("insufficient_samples", "Region has no selected pixels"));
            }
            for family in population["measurements"].as_array().into_iter().flatten() {
                active = family.as_str().unwrap_or("");
                match active {
                    "numeric" => numeric(e, region, &mask, a, b),
                    "color" => {
                        summary(
                            e,
                            region,
                            "delta_e00",
                            selected(&maps[0].values, &mask),
                            std::slice::from_ref(&maps[0].artifact),
                            "bounded_srgb_d65_lab",
                            "delta E 00",
                            false,
                        );
                        for m in &maps[1..5] {
                            let mut values = selected(&m.values, &mask);
                            let average = mean(&values);
                            let middle = median(&mut values);
                            for (label, value) in [("mean", average), ("median", middle)] {
                                e.scalar(
                                    region,
                                    &format!("{}.{label}", m.name),
                                    value,
                                    n,
                                    "bounded_srgb_d65_lab",
                                    m.units,
                                    &format!("signed pixel {label} candidate minus baseline"),
                                    std::slice::from_ref(&m.artifact),
                                    None,
                                );
                            }
                        }
                        for threshold in protocol["delta_e_thresholds"]
                            .as_array()
                            .into_iter()
                            .flatten()
                        {
                            let t = threshold.as_f64().ok_or_else(|| {
                                err("invalid_protocol", "Threshold must be numeric")
                            })?;
                            let label = spelling(threshold);
                            let above = maps[0]
                                .values
                                .iter()
                                .zip(&mask)
                                .filter(|(x, m)| **m && **x > t)
                                .count();
                            e.scalar(
                                region,
                                &format!("delta_e00.fraction_above_{label}"),
                                Some(above as f64 / n as f64),
                                n,
                                "bounded_srgb_d65_lab",
                                "fraction",
                                &format!(
                                    "pixel count above threshold={label} / valid selected pixels"
                                ),
                                std::slice::from_ref(&maps[0].artifact),
                                None,
                            );
                        }
                    }
                    "flip" => {
                        let map = maps.last().unwrap();
                        summary(
                            e,
                            region,
                            "flip",
                            selected(&map.values, &mask),
                            std::slice::from_ref(&map.artifact),
                            "bounded_encoded_srgb",
                            "FLIP error",
                            true,
                        );
                    }
                    "tone" => tone(
                        e,
                        region,
                        &mask,
                        &labs,
                        linear.as_ref().unwrap(),
                        protocol,
                        w,
                        h,
                        std::slice::from_ref(&maps[1].artifact),
                    )?,
                    "grain" => grain(
                        e,
                        region,
                        &mask,
                        linear.as_ref().unwrap(),
                        w,
                        h,
                        &protocol["grain"],
                        population["role"].as_str() == Some("flat_field")
                            || protocol["probe"]["kind"].as_str() == Some("flat_field"),
                    )?,
                    "halation" => halation(
                        e,
                        region,
                        &mask,
                        linear.as_ref().unwrap(),
                        w,
                        h,
                        &protocol["halation"],
                    )?,
                    "edge" => edge(
                        e,
                        region,
                        &mask,
                        linear.as_ref().unwrap(),
                        w,
                        h,
                        &protocol["edge"],
                    )?,
                    _ => return Err(err("invalid_protocol", format!("Unknown family: {active}"))),
                }
            }
            Ok(())
        })();
        if let Err(error) = execution {
            e.scalar(
                region,
                active,
                None,
                0,
                "declared_measurement",
                "unavailable",
                "selected family execution",
                &[],
                Some(&error.code),
            );
            e.failure(
                region,
                active,
                &error,
                &refs,
                ![
                    "missing_probe_geometry",
                    "insufficient_samples",
                    "zero_edge_contrast",
                ]
                .contains(&error.code.as_str()),
            );
        }
    }
    let ranking = maps.last().filter(|m| m.name == "flip").unwrap_or(&maps[0]);
    e.extra.insert(
        "automatic_regions".into(),
        json!(select_hotspots(
            &ranking.values,
            w,
            h,
            &protocol["hotspots"],
            ranking.name,
            std::slice::from_ref(&ranking.artifact)
        )),
    );
    Ok(())
}
pub fn compare_images(
    a: &ImageData,
    b: &ImageData,
    regions: &[Value],
    protocol: &Value,
    out: &Path,
) -> Result<Value> {
    let mut evidence = Evidence::new(protocol, out);
    let populations = populations(regions, protocol);
    if let Err(error) = compare_inner(&mut evidence, a, b, &populations, protocol) {
        evidence.failure("full_image", "image", &error, &[], true);
        for population in populations {
            for family in population["measurements"].as_array().into_iter().flatten() {
                evidence.scalar(
                    population["id"].as_str().unwrap_or(""),
                    family.as_str().unwrap_or(""),
                    None,
                    0,
                    "declared_measurement",
                    "unavailable",
                    "selected family boundary failure",
                    &[],
                    Some(&error.code),
                );
            }
        }
    }
    Ok(evidence.finish())
}
pub fn select_hotspots(
    values: &[f64],
    width: usize,
    height: usize,
    settings: &Value,
    metric: &str,
    refs: &[String],
) -> Vec<Value> {
    if values.len() != width * height || values.iter().any(|x| !x.is_finite()) {
        return Vec::new();
    }
    let requested = settings.get("count").and_then(Value::as_u64).unwrap_or(0) as usize;
    let minimum = settings
        .get("min_area_px")
        .and_then(Value::as_u64)
        .unwrap_or(1) as usize;
    if requested == 0 || minimum == 0 {
        return Vec::new();
    }
    let side = (minimum as f64).sqrt().ceil() as usize;
    let mut rectangles = Vec::new();
    for y in (0..height).step_by(side) {
        for x in (0..width).step_by(side) {
            let w = side.min(width - x);
            let h = side.min(height - y);
            if w * h >= minimum {
                rectangles.push([x, y, w, h]);
            }
        }
    }
    let rank = |rect: [usize; 4]| {
        let mut sum = 0.0;
        for y in rect[1]..rect[1] + rect[3] {
            sum += values[y * width + rect[0]..y * width + rect[0] + rect[2]]
                .iter()
                .sum::<f64>();
        }
        (-sum / (rect[2] * rect[3]) as f64, rect[1], rect[0])
    };
    rectangles.sort_by(|a, b| rank(*a).partial_cmp(&rank(*b)).unwrap());
    rectangles.truncate(requested);
    loop {
        let mut pair = None;
        'search: for i in 0..rectangles.len() {
            for j in i + 1..rectangles.len() {
                let a = rectangles[i];
                let b = rectangles[j];
                if (a[0] <= b[0] + b[2]
                    && b[0] <= a[0] + a[2]
                    && a[1] < b[1] + b[3]
                    && b[1] < a[1] + a[3])
                    || (a[1] <= b[1] + b[3]
                        && b[1] <= a[1] + a[3]
                        && a[0] < b[0] + b[2]
                        && b[0] < a[0] + a[2])
                {
                    pair = Some((i, j));
                    break 'search;
                }
            }
        }
        let Some((i, j)) = pair else {
            break;
        };
        let a = rectangles[i];
        let b = rectangles.remove(j);
        let x = a[0].min(b[0]);
        let y = a[1].min(b[1]);
        rectangles[i] = [
            x,
            y,
            (a[0] + a[2]).max(b[0] + b[2]) - x,
            (a[1] + a[3]).max(b[1] + b[3]) - y,
        ];
    }
    rectangles.sort_by(|a, b| rank(*a).partial_cmp(&rank(*b)).unwrap());
    rectangles.into_iter().enumerate().map(|(i,rectangle)|json!({"id":format!("automatic_{:03}",i+1),"geometry":rectangle,"rectangle":rectangle,"ranking_metric":metric,"ranking_value":-rank(rectangle).0,"selection_method":"rank square tiles by mean descending, ties y/x; select count; merge touching or overlapping rectangles to fixed point; rerank merged rectangle means","artifact_refs":refs})).collect()
}
pub fn aggregate_seed_measurements(results: &[Value]) -> Vec<Value> {
    let mut expected: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut groups: BTreeMap<Vec<String>, Vec<Value>> = BTreeMap::new();
    for result in results {
        if let Some(seed) = result.get("seed") {
            expected
                .entry(result["case_id"].as_str().unwrap_or("").into())
                .or_default()
                .push(seed.clone());
        }
        for row in result["measurements"].as_array().into_iter().flatten() {
            let key = [
                "case_id",
                "region_id",
                "metric",
                "domain",
                "units",
                "estimator",
            ]
            .map(|k| row[k].as_str().unwrap_or("").to_owned())
            .to_vec();
            groups.entry(key).or_default().push(row.clone());
        }
    }
    let mut output = Vec::new();
    for (key, rows) in groups {
        let seeds: Vec<Value> = rows
            .iter()
            .flat_map(|r| {
                r["seed_population"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .cloned()
            })
            .collect();
        let wanted = expected.get(&key[0]).unwrap_or(&seeds);
        let set = |v: &[Value]| v.iter().map(Value::to_string).collect::<HashSet<_>>();
        let complete = set(&seeds) == set(wanted);
        let available = complete && rows.iter().all(|r| r["availability"] == "available");
        let values: Vec<f64> = rows
            .iter()
            .filter(|r| r["availability"] == "available")
            .filter_map(|r| r["value"].as_f64())
            .collect();
        let mut refs = Vec::new();
        let mut seen = HashSet::new();
        for r in &rows {
            for reference in r["artifact_refs"].as_array().into_iter().flatten() {
                if seen.insert(reference.to_string()) {
                    refs.push(reference.clone());
                }
            }
        }
        for suffix in ["seed_mean", "seed_std"] {
            let mut row = rows[0].clone();
            let object = row.as_object_mut().unwrap();
            object.remove("value");
            object.remove("reason");
            row["metric"] = json!(format!("{}.{suffix}", key[2]));
            row["seed_population"] = json!(seeds);
            row["sample_count"] = json!(rows.len());
            row["seed_count"] = json!(rows.len());
            row["estimator"] = json!(if suffix == "seed_mean" {
                "equal-weight per-seed arithmetic mean"
            } else {
                "cross-seed sample standard deviation ddof=1"
            });
            row["artifact_refs"] = json!(refs);
            row["per_seed_measurements"] = json!(rows);
            row["expected_seed_population"] = json!(wanted);
            let reason = if !complete {
                Some("missing_seed_measurement")
            } else if !available {
                Some("unavailable_seed_measurement")
            } else if suffix == "seed_std" && rows.len() < 2 {
                Some("insufficient_seeds")
            } else {
                None
            };
            if let Some(reason) = reason {
                row["availability"] = json!("unavailable");
                row["reason"] = json!(reason);
            } else {
                let average = mean(&values).unwrap();
                let value = if suffix == "seed_mean" {
                    average
                } else {
                    (values.iter().map(|v| (v - average).powi(2)).sum::<f64>()
                        / (values.len() - 1) as f64)
                        .sqrt()
                };
                row["availability"] = json!("available");
                row["value"] = json!(value);
            }
            output.push(row);
        }
    }
    output
}

fn metric_definitions(case: &Value, region_id: &str, protocol: &Value) -> HashSet<String> {
    let mut definitions = HashSet::new();
    let families = if region_id == "full_image" {
        case.get("measurements")
            .or_else(|| protocol.get("measurements"))
    } else {
        case["regions"]
            .as_array()
            .and_then(|regions| regions.iter().find(|r| r["id"].as_str() == Some(region_id)))
            .and_then(|r| r.get("measurements"))
    };
    let default_families = json!(["numeric", "color", "tone"]);
    let chosen: HashSet<&str> = families
        .or_else(|| (region_id == "full_image").then_some(&default_families))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if chosen.contains("numeric") {
        for name in ["max_abs_difference", "rmse"] {
            definitions.insert(format!("numeric.{name}"));
        }
        for side in ["baseline", "candidate"] {
            for name in [
                "nonfinite_sample_count",
                "nonfinite_pixel_count",
                "below_zero_fraction",
                "above_one_fraction",
            ] {
                definitions.insert(format!("numeric.{side}.{name}"));
            }
        }
    }
    if chosen.contains("color") {
        for stat in ["median", "p95", "p99", "max"] {
            definitions.insert(format!("delta_e00.{stat}"));
        }
        for name in [
            "delta_l_star",
            "delta_a_star",
            "delta_b_star",
            "delta_c_star",
        ] {
            for stat in ["mean", "median"] {
                definitions.insert(format!("{name}.{stat}"));
            }
        }
        for threshold in protocol["delta_e_thresholds"]
            .as_array()
            .into_iter()
            .flatten()
        {
            definitions.insert(format!("delta_e00.fraction_above_{}", spelling(threshold)));
        }
    }
    if chosen.contains("flip") {
        for stat in ["mean", "p95", "p99", "max"] {
            definitions.insert(format!("flip.{stat}"));
        }
    }
    for side in ["baseline", "candidate", "delta"] {
        if chosen.contains("tone") {
            for q in [5, 25, 50, 75, 95] {
                definitions.insert(format!("tone.l_star.{side}.q{q:02}"));
            }
            for scale in protocol["region_sampling_scales"]
                .as_array()
                .into_iter()
                .flatten()
            {
                definitions.insert(format!(
                    "tone.local_contrast.sigma_{}.{side}.rms",
                    spelling(scale)
                ));
            }
            if region_id == "full_image" {
                let probe = case.get("probe").unwrap_or(&protocol["probe"]);
                let n = probe["tone_response"].as_array().map_or(0, Vec::len);
                for interval in 0..n.saturating_sub(1) {
                    for label in ["linear_luminance", "l_star"] {
                        definitions.insert(format!(
                            "tone.response.interval_{interval}.{label}.{side}.slope"
                        ));
                    }
                }
            }
        }
        if chosen.contains("grain") {
            let flat = case.get("probe").unwrap_or(&protocol["probe"])["kind"].as_str()
                == Some("flat_field")
                || case["regions"].as_array().into_iter().flatten().any(|r| {
                    r["id"].as_str() == Some(region_id) && r["role"].as_str() == Some("flat_field")
                });
            let requested = protocol["grain"]["residual"]["method"]
                .as_str()
                .unwrap_or(if flat {
                    "flat_mean"
                } else {
                    "gaussian_high_pass"
                });
            let method = if requested == "flat_mean" && !flat {
                "gaussian_high_pass"
            } else {
                requested
            };
            for channel in ["R", "G", "B", "luminance", "chromatic"] {
                definitions.insert(format!("grain.{method}.{side}.rms.{channel}"));
            }
            for pair in ["RG", "RB", "GB"] {
                for name in ["covariance", "correlation"] {
                    definitions.insert(format!("grain.{method}.{side}.{name}.{pair}"));
                }
            }
            for name in ["psd.luminance_total", "frequency_centroid_cycles_per_pixel"] {
                definitions.insert(format!("grain.{method}.{side}.{name}"));
            }
        }
        if chosen.contains("halation") {
            definitions.insert(format!("halation.{side}.core_clipped_fraction"));
            definitions.insert(format!("halation.{side}.red_minus_green.integrated_spill"));
            for channel in ["R", "G", "B"] {
                for name in [
                    "peak_amplitude",
                    "peak_radius_px",
                    "half_height_width_px",
                    "integrated_spill",
                ] {
                    definitions.insert(format!("halation.{side}.{channel}.{name}"));
                }
            }
        }
        if chosen.contains("edge") {
            for name in [
                "overshoot",
                "undershoot",
                "mtf50_cycles_per_pixel",
                "plateau_contrast",
            ] {
                definitions.insert(format!("edge.{side}.{name}"));
            }
        }
    }
    definitions
}
pub fn metric_supported_with_protocol(
    metric: &str,
    case: &Value,
    region_id: &str,
    protocol: &Value,
) -> bool {
    let metric = metric
        .strip_suffix(".seed_mean")
        .or_else(|| metric.strip_suffix(".seed_std"))
        .unwrap_or(metric);
    if metric.starts_with("reference.") {
        let base = ["baseline_distance", "candidate_distance", "distance_delta"]
            .iter()
            .find_map(|side| metric.strip_suffix(&format!(".{side}")))
            .unwrap_or(metric);
        let reference = &case["reference"];
        return match reference["scope"].as_str() {
            Some("aligned_scene") => matches!(
                base,
                "reference.delta_e00.median"
                    | "reference.delta_e00.p95"
                    | "reference.delta_e00.p99"
                    | "reference.delta_e00.max"
                    | "reference.tone_l_star.q05"
                    | "reference.tone_l_star.q25"
                    | "reference.tone_l_star.q50"
                    | "reference.tone_l_star.q75"
                    | "reference.tone_l_star.q95"
                    | "reference.chroma_ab.median"
            ),
            Some("regional") => matches!(
                base,
                "reference.patch_delta_e00"
                    | "reference.distribution_l_star"
                    | "reference.patch_chroma_ab"
            ),
            Some("material") => base
                .strip_prefix("reference.material.")
                .is_some_and(|curve| {
                    case["probe_response"]["curves"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|row| row["id"].as_str() == Some(curve))
                }),
            _ => false,
        };
    }
    metric_definitions(case, region_id, protocol).contains(metric)
}
pub fn metric_supported(metric: &str, case: &Value, region_id: &str) -> bool {
    metric_supported_with_protocol(
        metric,
        case,
        region_id,
        case.get("protocol").unwrap_or(case),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn image(value: f64, width: usize, height: usize) -> ImageData {
        ImageData {
            samples: vec![[value; 3]; width * height],
            width,
            height,
            color_space: "sRGB".into(),
            transfer: "encoded".into(),
            metadata: json!({}),
            path: None,
        }
    }
    fn output() -> PathBuf {
        std::env::temp_dir().join(format!(
            "benchmark-metric-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    #[test]
    fn structured_boundary_failure() {
        let a = image(0.4, 4, 4);
        let b = image(0.4, 5, 4);
        let result = compare_images(
            &a,
            &b,
            &[],
            &json!({"measurements":["numeric","color"]}),
            &output(),
        )
        .unwrap();
        assert_eq!(result["execution_status"], "failed");
        assert_eq!(result["measurements"][0]["reason"], "dimension_mismatch");
        let mut b = a.clone();
        b.samples[3][1] = f64::NAN;
        let result =
            compare_images(&a, &b, &[], &json!({"measurements":["numeric"]}), &output()).unwrap();
        assert_eq!(
            result["numeric_health"]["candidate"]["nonfinite_pixel_count"],
            1
        );
        assert_eq!(result["measurements"][0]["reason"], "nonfinite_samples");
    }
    #[test]
    fn identical_and_signed_official_flip() {
        let a = image(0.4, 16, 16);
        let path = output();
        let protocol = json!({"measurements":["numeric","color","flip"],"pixels_per_degree":67});
        let result = compare_images(&a, &a, &[], &protocol, &path).unwrap();
        assert_eq!(result["execution_status"], "completed");
        for row in result["measurements"].as_array().unwrap() {
            if row["metric"].as_str().unwrap().starts_with("flip.") {
                assert_eq!(row["value"], 0.0);
            }
        }
        fs::remove_dir_all(path).unwrap();
        let path = output();
        let result = compare_images(&a, &image(0.5, 16, 16), &[], &protocol, &path).unwrap();
        let rows = result["measurements"].as_array().unwrap();
        assert!(
            rows.iter()
                .find(|r| r["metric"] == "delta_l_star.mean")
                .unwrap()["value"]
                .as_f64()
                .unwrap()
                > 0.0
        );
        assert!(
            rows.iter().find(|r| r["metric"] == "flip.mean").unwrap()["value"]
                .as_f64()
                .unwrap()
                > 0.0
        );
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn flat_zero_variance_retains_radial_spectrum() {
        let a = image(0.0, 8, 8);
        let path = output();
        let result=compare_images(&a,&a,&[],&json!({"measurements":["grain"],"probe":{"kind":"flat_field"},"grain":{"residual":{"method":"flat_mean"},"spectrum":{"window":"boxcar"}}}),&path).unwrap();
        let row = result["measurements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["metric"] == "grain.flat_mean.baseline.correlation.RG")
            .unwrap();
        assert_eq!(row["reason"], "zero_variance");
        assert!(
            result["artifacts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["id"] == "grain.full_image.baseline.radial_psd_artifact")
        );
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn missing_seed_precedes_insufficient_seeds() {
        let row = json!({"case_id":"a","region_id":"full_image","metric":"numeric.rmse","domain":"native","units":"sample","estimator":"rmse","availability":"available","value":1.0,"seed_population":[1],"artifact_refs":[]});
        let result = aggregate_seed_measurements(&[
            json!({"case_id":"a","seed":1,"measurements":[row]}),
            json!({"case_id":"a","seed":2,"measurements":[]}),
        ]);
        assert!(
            result
                .iter()
                .all(|r| r["reason"] == "missing_seed_measurement")
        );
    }
    #[test]
    fn hotspot_ties_and_touching_merge() {
        let values = vec![1.0; 16];
        let regions = select_hotspots(
            &values,
            4,
            4,
            &json!({"count":2,"min_area_px":4}),
            "flip",
            &[],
        );
        assert_eq!(regions[0]["rectangle"], json!([0, 0, 4, 2]));
        assert!(
            select_hotspots(
                &values,
                4,
                4,
                &json!({"count":3,"min_area_px":17}),
                "flip",
                &[]
            )
            .is_empty()
        );
        assert_eq!(mtf50(&[1.0, 0.75, 0.25], 8), Some(0.1875));
        assert_eq!(mtf50(&[1.0, 0.7], 8), None);
    }
}
