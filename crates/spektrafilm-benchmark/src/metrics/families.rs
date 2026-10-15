use crate::common::Result;
use crate::image_data::ImageData;
use rustfft::{FftPlanner, num_complex::Complex};
use serde_json::{Value, json};

use super::err;
use super::evidence::{ArtifactRole, Evidence, GrainRetained};
use super::mask::{crop, cropped_mask, erode, geometry_mask};
use super::math::{
    count, fft2, frequency, gaussian, luminance, mean, median, number, periodic_hann, quantile,
    sort, spelling,
};

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
pub fn summary(
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
pub fn health(samples: &[[f64; 3]], dimensions: [usize; 2]) -> Value {
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
// Grain settings and image dimensions are numeric protocol payload; keep this call explicit.
#[allow(clippy::too_many_arguments)]
pub fn grain(
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

pub fn halation(
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
pub fn mtf50(sfr: &[f64], sample_count: usize) -> Option<f64> {
    let index = sfr.iter().position(|v| *v <= 0.5)?;
    if index == 0 {
        return None;
    }
    Some(
        ((index - 1) as f64 + (0.5 - sfr[index - 1]) / (sfr[index] - sfr[index - 1]))
            / sample_count as f64,
    )
}
pub fn edge(
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
pub fn tone(
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

pub struct Map {
    pub name: &'static str,
    pub units: &'static str,
    pub values: Vec<f64>,
    pub artifact: String,
}
pub fn numeric(e: &mut Evidence, region: &str, mask: &[bool], a: &ImageData, b: &ImageData) {
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
