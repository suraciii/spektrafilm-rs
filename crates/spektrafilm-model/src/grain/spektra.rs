//! Spektra's curve-calibrated, multilayer Poisson-binomial grain model.
//!
//! Preparation is shared by the CPU and device realizations. Grain working
//! buffers are always f32; profile tables, preparation, and clumping are f64.

mod filter;

use rayon::prelude::*;
use spektrafilm_backend::ComputeBackend;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::numpy_rng::{GaussRng, rk_binomial, rk_poisson};
use spektrafilm_math::precision::{from_f64, to_f64};
use spektrafilm_math::stats::{self, FastStatsRng};

const POISSON_TAG: u64 = 0;
const BINOMIAL_TAG: u64 = 1;
const APERTURE_AREA_UM2: f64 = std::f64::consts::PI * (24.0 * 24.0);

/// Prepared rows are indexed `[sublayer][channel]`. Inactive rows are zero.
#[derive(Clone, Debug)]
pub struct PreparedGrain {
    pub layer_count: u32,
    pub density_min: [f64; 3],
    pub uniformity: [f64; 3],
    pub density_min_layers: [[f64; 3]; 3],
    pub density_max_adj: [[f64; 3]; 3],
    pub n_particles: [[f64; 3]; 3],
}

/// Size the coarsest grains from the actual peak of the sum of the layers'
/// variance curves, not a closed-form approximation of their overlap.
#[allow(clippy::too_many_arguments)]
pub fn prepare_grain(
    curves_layers: &[[[f64; 3]; 3]],
    layer_count: u32,
    pixel_size_um: f64,
    rms_granularity: [f64; 3],
    particle_scale_sublayers: [f64; 3],
    density_min: [f64; 3],
    uniformity: [f64; 3],
) -> Result<PreparedGrain, String> {
    if !(1..=3).contains(&layer_count) || curves_layers.is_empty() {
        return Err("Spektra grain requires curves for 1..=3 active layers".into());
    }
    let count = layer_count as usize;
    let pixel_area = pixel_size_um * pixel_size_um;
    if !pixel_size_um.is_finite() || pixel_size_um <= 0.0 || !pixel_area.is_finite() {
        return Err("Spektra grain pixel size must be finite and positive".into());
    }
    for &scale in &particle_scale_sublayers[..count] {
        if !scale.is_finite() || scale <= 0.0 {
            return Err("Spektra grain active sublayer scales must be finite and positive".into());
        }
    }
    for ch in 0..3 {
        if !rms_granularity[ch].is_finite() || rms_granularity[ch] <= 0.0 {
            return Err("Spektra grain RMS granularity must be finite and positive".into());
        }
        if !density_min[ch].is_finite() || density_min[ch] < 0.0 {
            return Err("Spektra grain density floor must be finite and non-negative".into());
        }
        if !uniformity[ch].is_finite() || !(0.0..=1.0).contains(&uniformity[ch]) {
            return Err("Spektra grain uniformity must be in [0, 1]".into());
        }
    }
    let mut maxima = [[f64::NEG_INFINITY; 3]; 3];
    for curve in curves_layers {
        for sl in 0..count {
            for ch in 0..3 {
                let value = curve[sl][ch];
                if value.is_infinite() {
                    return Err("Spektra grain layer curves must not contain infinity".into());
                }
                // np.nanmax ignores missing samples; np.nan_to_num below
                // makes their contribution to the peak calculation zero.
                if !value.is_nan() {
                    maxima[sl][ch] = maxima[sl][ch].max(value);
                }
            }
        }
    }
    let mut prepared = PreparedGrain {
        layer_count,
        density_min,
        uniformity,
        density_min_layers: [[0.0; 3]; 3],
        density_max_adj: [[0.0; 3]; 3],
        n_particles: [[0.0; 3]; 3],
    };
    for ch in 0..3 {
        let total: f64 = maxima[..count].iter().map(|row| row[ch]).sum();
        if !total.is_finite() || total <= 0.0 {
            return Err("Spektra grain layer maxima must have a finite positive sum".into());
        }
        let mut fractions = [0.0; 3];
        for sl in 0..count {
            if !maxima[sl][ch].is_finite() || maxima[sl][ch] <= 0.0 {
                return Err("Spektra grain every active layer must have a positive maximum".into());
            }
            fractions[sl] = maxima[sl][ch] / total;
            prepared.density_min_layers[sl][ch] = fractions[sl] * density_min[ch];
            prepared.density_max_adj[sl][ch] = maxima[sl][ch] + prepared.density_min_layers[sl][ch];
        }
        let mut peak = f64::NEG_INFINITY;
        for curve in curves_layers {
            let mut variance = 0.0;
            for sl in 0..count {
                let net = if curve[sl][ch].is_nan() {
                    0.0
                } else {
                    curve[sl][ch]
                };
                let absolute = net + prepared.density_min_layers[sl][ch];
                variance += (particle_scale_sublayers[sl] / fractions[sl])
                    * absolute
                    * (prepared.density_max_adj[sl][ch] - uniformity[ch] * absolute);
            }
            if !variance.is_finite() {
                return Err("Spektra grain variance curves must be finite".into());
            }
            peak = peak.max(variance);
        }
        let sigma = rms_granularity[ch] / 1000.0;
        let coarsest_area = sigma * sigma * APERTURE_AREA_UM2 / peak.max(1e-9);
        for sl in 0..count {
            prepared.n_particles[sl][ch] =
                pixel_area * fractions[sl] / (coarsest_area * particle_scale_sublayers[sl]);
        }
    }
    validate_prepared(&prepared)?;
    Ok(prepared)
}

fn validate_prepared(prepared: &PreparedGrain) -> Result<(), String> {
    if !(1..=3).contains(&prepared.layer_count) {
        return Err("Spektra grain requires 1..=3 active layers".into());
    }
    for ch in 0..3 {
        if !prepared.density_min[ch].is_finite()
            || prepared.density_min[ch] < 0.0
            || !(prepared.density_min[ch] as f32).is_finite()
            || !prepared.uniformity[ch].is_finite()
            || !(0.0..=1.0).contains(&prepared.uniformity[ch])
        {
            return Err("Spektra grain prepared channel parameters are invalid".into());
        }
        for sl in 0..prepared.layer_count as usize {
            let maximum = prepared.density_max_adj[sl][ch];
            let particles = prepared.n_particles[sl][ch];
            let floor = prepared.density_min_layers[sl][ch];
            let saturation =
                1.0f32 - (1.0f32 - 1e-6f32) * prepared.uniformity[ch] as f32 * (1.0f32 - 1e-6f32);
            let max_rate = ((particles as f32) / saturation) as f64;
            let optical_density = maximum / particles;
            if !maximum.is_finite()
                || maximum <= 0.0
                || !(maximum as f32).is_finite()
                || maximum as f32 <= 0.0
                || !particles.is_finite()
                || particles <= 0.0
                || !(particles as f32).is_finite()
                || particles as f32 <= 0.0
                || !floor.is_finite()
                || floor < 0.0
                || !(floor as f32).is_finite()
                || !(optical_density as f32).is_finite()
                || optical_density as f32 <= 0.0
                || !max_rate.is_finite()
                || max_rate >= i64::MAX as f64 - 10.0 * (i64::MAX as f64).sqrt()
            {
                return Err(
                    "Spektra grain prepared layer is outside the finite sampling range".into(),
                );
            }
        }
    }
    Ok(())
}

fn observe_sampler(
    backend: &dyn ComputeBackend,
) -> Option<spektrafilm_backend::telemetry::ObservationScope> {
    backend
        .observation_context()
        .filter(|context| context.enabled())
        .map(|context| {
            let scope = context.scope(
                "grain_spektra_sampler",
                spektrafilm_backend::telemetry::ObservationKind::Stage,
                context.purpose(),
            );
            scope.context().record_executor(
                spektrafilm_backend::telemetry::Executor::Cpu,
                Some(spektrafilm_backend::telemetry::CpuReason::SpektraGrainSampler),
            );
            scope
        })
}

/// Apply the original whole-plane sampling law in f32 working precision.
/// The returned MT state is the state after the entire binomial plane, or
/// the freshly seeded state in fast-stats mode (numba does not advance it).
#[allow(clippy::too_many_arguments)]
fn layer_particle_model(
    mut density: Vec<f32>,
    width: usize,
    height: usize,
    density_max: f64,
    n_particles_per_pixel: f64,
    uniformity: f64,
    seed: u64,
    blur_particle: f64,
    use_fast_stats: bool,
) -> (Vec<f32>, rand_mt::Mt) {
    let mut rng = rand_mt::Mt::new(seed as u32);
    let od_particle = density_max / n_particles_per_pixel;
    let npp = n_particles_per_pixel as f32;
    let uniformity = uniformity as f32;
    // Store the saturation separately; reuse density for probability, then
    // sampled grain. All weak Python scalar operations round in f32.
    let mut saturation = Vec::with_capacity(density.len());
    for value in &mut density {
        *value = (*value / density_max as f32).clamp(1e-6f32, 1.0f32 - 1e-6f32);
        saturation.push(1.0f32 - *value * uniformity * (1.0f32 - 1e-6f32));
    }
    if use_fast_stats {
        density
            .par_iter_mut()
            .zip(&saturation)
            .enumerate()
            .for_each(|(i, (value, &sat))| {
                let mut poisson = FastStatsRng::stream(seed, POISSON_TAG, i as u64);
                let sensitized = stats::fast_poisson(&mut poisson, (npp / sat) as f64);
                let mut binomial = FastStatsRng::stream(seed, BINOMIAL_TAG, i as u64);
                let developed = stats::fast_binomial(&mut binomial, sensitized, *value as f64);
                *value = developed as f32;
                *value *= od_particle as f32;
                *value *= sat;
            });
    } else {
        // scipy draws the whole Poisson array before any binomial sample.
        let counts: Vec<u64> = saturation
            .iter()
            .map(|&sat| rk_poisson(&mut rng, (npp / sat) as f64))
            .collect();
        for ((value, &sat), count) in density.iter_mut().zip(&saturation).zip(counts) {
            *value = rk_binomial(&mut rng, count, *value as f64) as f32;
            *value *= od_particle as f32;
            *value *= sat;
        }
    }
    if blur_particle > 0.0 {
        density = filter::blur(&density, width, height, blur_particle * od_particle.sqrt());
    }
    (density, rng)
}

fn inverse_differences(curves: &[[f64; 3]], channel: usize, sign: f64) -> Vec<f64> {
    curves
        .windows(2)
        .map(|rows| {
            let delta = rows[1][channel] * sign - rows[0][channel] * sign;
            if delta == 0.0 { 0.0 } else { 1.0 / delta }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn interpolate_layer(
    density: f64,
    channel: usize,
    layer: usize,
    curves: &[[f64; 3]],
    layers: &[[[f64; 3]; 3]],
    inv_dx: &[f64],
    sign: f64,
    floor: f64,
) -> f32 {
    let x = density * sign;
    let last = curves.len() - 1;
    let interpolated = if x <= curves[0][channel] * sign {
        layers[0][layer][channel]
    } else if x >= curves[last][channel] * sign {
        layers[last][layer][channel]
    } else {
        let upper = curves.partition_point(|row| row[channel] * sign <= x);
        let lower = upper - 1;
        let x0 = curves[lower][channel] * sign;
        let t = (x - x0) * inv_dx[lower];
        layers[lower][layer][channel]
            + t * (layers[upper][layer][channel] - layers[lower][layer][channel])
    } as f32;
    // Unlike weak Python scalar operations in the sampler, the f64 ndarray
    // floor promotes this in-place addition before its f32 working store.
    (interpolated as f64 + floor) as f32
}

fn add_micro_structure(
    planes: &mut [Vec<f32>],
    width: usize,
    height: usize,
    micro_structure: [f64; 2],
    pixel_size_um: f64,
    state: rand_mt::Mt,
) {
    let sigma = micro_structure[1] * 0.001 / pixel_size_um;
    if sigma <= 0.05 {
        return;
    }
    let sigma2 = (sigma * sigma).ln_1p();
    let log_sigma = sigma2.sqrt();
    let mut rng = GaussRng::from_mt(state);
    let mut clumping: Vec<Vec<f64>> = (0..planes.len())
        .map(|_| Vec::with_capacity(width * height))
        .collect();
    // NumPy generates an HWC array, not one plane's normals at a time.
    for _ in 0..width * height {
        for plane in &mut clumping {
            let mut value = rng.gauss();
            value *= log_sigma;
            value -= 0.5 * sigma2;
            plane.push(value.exp());
        }
    }
    let blur_px = micro_structure[0] / pixel_size_um;
    for (density, mut clump) in planes.iter_mut().zip(clumping) {
        if blur_px > 0.4 {
            clump = filter::blur(&clump, width, height, blur_px);
        }
        // f32 *= f64 array computes in f64 then rounds at the store.
        for (value, factor) in density.iter_mut().zip(clump) {
            *value = (*value as f64 * factor) as f32;
        }
    }
}

// NumPy's pairwise float32 reduction (including its eight accumulators).
fn sum_f32(values: &[f32]) -> f32 {
    if values.len() < 8 {
        values.iter().fold(-0.0, |sum, &value| sum + value)
    } else if values.len() <= 128 {
        let mut sums: [f32; 8] = values[..8].try_into().unwrap();
        let end = values.len() - values.len() % 8;
        for chunk in values[8..end].as_chunks::<8>().0 {
            for i in 0..8 {
                sums[i] += chunk[i];
            }
        }
        let mut sum = ((sums[0] + sums[1]) + (sums[2] + sums[3]))
            + ((sums[4] + sums[5]) + (sums[6] + sums[7]));
        for &value in &values[end..] {
            sum += value;
        }
        sum
    } else {
        let midpoint = (values.len() / 2) & !7;
        sum_f32(&values[..midpoint]) + sum_f32(&values[midpoint..])
    }
}

fn apply_multiplicative_unsharp_mask(
    plane: &mut Vec<f32>,
    width: usize,
    height: usize,
    sigma: f64,
    amount: f64,
) {
    if sigma <= 0.0 || amount <= 0.0 {
        return;
    }
    for value in plane.iter_mut() {
        *value = value.max(0.0);
    }
    let original_mass = sum_f32(plane);
    let blurred = filter::blur(plane, width, height, sigma);
    for (density, blur) in plane.iter_mut().zip(blurred) {
        *density = if *density <= 0.0 {
            0.0
        } else {
            density.max(1e-6f32).powf((1.0 + amount) as f32) / blur.max(1e-6f32).powf(amount as f32)
        };
    }
    let sharpened_mass = sum_f32(plane);
    if sharpened_mass > 0.0 {
        let ratio = original_mass / sharpened_mass;
        for value in plane {
            *value *= ratio;
        }
    }
}

/// Grain one channel's sublayers at a time, then finish in absolute density
/// before subtracting fog. BW is one panchromatic channel, replicated only
/// after sampling, clumping, filtering and floor subtraction are complete.
#[allow(clippy::too_many_arguments)]
pub fn apply_grain(
    density_cmy: &ImageBuf,
    norm_curves_f64: &[[f64; 3]],
    curves_layers: &[[[f64; 3]; 3]],
    prepared: &PreparedGrain,
    pixel_size_um: f64,
    grain_blur: f64,
    blur_dye_clouds_um: f64,
    micro_structure: [f64; 2],
    usm_sigma: f64,
    usm_amount: f64,
    positive_film: bool,
    monochrome: bool,
    use_fast_stats: bool,
    base_seed: u64,
    backend: &dyn ComputeBackend,
) -> Result<ImageBuf, String> {
    validate_prepared(prepared)?;
    if !pixel_size_um.is_finite()
        || pixel_size_um <= 0.0
        || [
            grain_blur,
            blur_dye_clouds_um,
            micro_structure[0],
            micro_structure[1],
            usm_sigma,
            usm_amount,
        ]
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err("Spektra grain finishing parameters must be finite and non-negative, with positive pixel size".into());
    }
    if norm_curves_f64.is_empty() || norm_curves_f64.len() != curves_layers.len() {
        return Err(
            "Spektra grain composite and layer curves must have matching non-empty axes".into(),
        );
    }
    let channels = if monochrome { 1 } else { 3 };
    let sign = if positive_film { -1.0 } else { 1.0 };
    for ch in 0..channels {
        let mut previous = f64::NEG_INFINITY;
        for (row, layers) in norm_curves_f64.iter().zip(curves_layers) {
            let x = row[ch] * sign;
            if !x.is_finite()
                || x < previous
                || layers[..prepared.layer_count as usize]
                    .iter()
                    .any(|layer| !layer[ch].is_finite())
            {
                return Err(
                    "Spektra grain interpolation requires finite, monotonic source curves".into(),
                );
            }
            previous = x;
        }
    }
    let width = density_cmy.width as usize;
    let height = density_cmy.height as usize;
    let n = width
        .checked_mul(height)
        .ok_or("Spektra grain image dimensions overflow")?;
    if density_cmy.data.len()
        != n.checked_mul(3)
            .ok_or("Spektra grain image size overflows")?
        || density_cmy
            .data
            .iter()
            .any(|&value| !to_f64(value).is_finite())
    {
        return Err("Spektra grain image must have three finite samples per pixel".into());
    }
    // All fallible preflight finishes before allocating any grain image planes.
    let _sampler = observe_sampler(backend);
    let mut planes = Vec::with_capacity(channels);
    let mut final_state = None;
    for ch in 0..channels {
        let inv_dx = inverse_differences(norm_curves_f64, ch, sign);
        let mut channel = Vec::new();
        for sl in 0..prepared.layer_count as usize {
            let floor = prepared.density_min_layers[sl][ch];
            let density: Vec<f32> = density_cmy
                .pixels()
                .map(|pixel| {
                    interpolate_layer(
                        to_f64(pixel[ch]),
                        ch,
                        sl,
                        norm_curves_f64,
                        curves_layers,
                        &inv_dx,
                        sign,
                        floor,
                    )
                })
                .collect();
            let seed = base_seed.wrapping_add(ch as u64 + sl as u64 * 10);
            let (layer, state) = layer_particle_model(
                density,
                width,
                height,
                prepared.density_max_adj[sl][ch],
                prepared.n_particles[sl][ch],
                prepared.uniformity[ch],
                seed,
                blur_dye_clouds_um,
                use_fast_stats,
            );
            if sl == 0 {
                channel = layer;
            } else {
                for (value, contribution) in channel.iter_mut().zip(layer) {
                    *value += contribution;
                }
            }
            final_state = Some(state);
        }
        planes.push(channel);
    }
    add_micro_structure(
        &mut planes,
        width,
        height,
        micro_structure,
        pixel_size_um,
        final_state.unwrap(),
    );
    for (ch, plane) in planes.iter_mut().enumerate() {
        if grain_blur > 0.0 {
            *plane = filter::blur(plane, width, height, grain_blur);
        }
        apply_multiplicative_unsharp_mask(plane, width, height, usm_sigma, usm_amount);
        for value in plane {
            *value = (*value as f64 - prepared.density_min[ch]) as f32;
        }
    }
    let mut out = ImageBuf::new(density_cmy.width, density_cmy.height);
    for (i, pixel) in out.pixels_mut().enumerate() {
        for ch in 0..3 {
            pixel[ch] = from_f64(planes[if monochrome { 0 } else { ch }][i] as f64);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_backend::cpu_backend::CpuBackend;

    fn curves() -> (Vec<[f64; 3]>, Vec<[[f64; 3]; 3]>) {
        let composite = vec![[0.0; 3], [0.5; 3], [1.1; 3], [1.7; 3], [2.2; 3]];
        let layers = composite
            .iter()
            .map(|row| [[row[0] * 0.3; 3], [row[0] * 0.3; 3], [row[0] * 0.4; 3]])
            .collect();
        (composite, layers)
    }

    fn prepared(layers: &[[[f64; 3]; 3]], count: u32, pixel_size: f64) -> PreparedGrain {
        prepare_grain(
            layers,
            count,
            pixel_size,
            [6.0, 8.0, 10.0],
            [1.0, 0.5, 0.25],
            [0.03; 3],
            [0.97; 3],
        )
        .unwrap()
    }

    fn render(
        monochrome: bool,
        fast: bool,
        pixel_size: f64,
        blur: f64,
        micro: [f64; 2],
    ) -> ImageBuf {
        let (composite, layers) = curves();
        let params = prepared(&layers, 3, pixel_size);
        let mut image = ImageBuf::new(64, 32);
        for pixel in image.pixels_mut() {
            pixel.fill(from_f64(1.1));
        }
        apply_grain(
            &image,
            &composite,
            &layers,
            &params,
            pixel_size,
            blur,
            0.0,
            micro,
            0.0,
            0.0,
            false,
            monochrome,
            fast,
            0,
            &CpuBackend,
        )
        .unwrap()
    }

    fn variance(plane: &[spektrafilm_math::precision::Scalar]) -> f64 {
        let mean = plane.iter().map(|&value| to_f64(value)).sum::<f64>() / plane.len() as f64;
        plane
            .iter()
            .map(|&value| (to_f64(value) - mean).powi(2))
            .sum::<f64>()
            / plane.len() as f64
    }

    #[test]
    fn prepared_curve_peak_recovers_catalogue_rms() {
        let (_, layers) = curves();
        let aperture_pixel = APERTURE_AREA_UM2.sqrt();
        for count in [1, 3] {
            let params = prepared(&layers, count, aperture_pixel);
            for (ch, target) in [6.0, 8.0, 10.0].into_iter().enumerate() {
                let peak = layers
                    .iter()
                    .map(|curve| {
                        (0..count as usize)
                            .map(|sl| {
                                let d = curve[sl][ch] + params.density_min_layers[sl][ch];
                                d * (params.density_max_adj[sl][ch] - params.uniformity[ch] * d)
                                    / params.n_particles[sl][ch]
                            })
                            .sum::<f64>()
                    })
                    .fold(0.0f64, f64::max);
                assert!((1000.0 * peak.sqrt() - target).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn invalid_preparation_rejects_before_rendering() {
        let (_, mut layers) = curves();
        assert!(prepare_grain(&layers, 0, 12.0, [6.0; 3], [1.0; 3], [0.03; 3], [0.97; 3]).is_err());
        assert!(prepare_grain(&layers, 3, 0.0, [6.0; 3], [1.0; 3], [0.03; 3], [0.97; 3]).is_err());
        for row in &mut layers {
            row[1] = [0.0; 3];
        }
        assert!(prepare_grain(&layers, 3, 12.0, [6.0; 3], [1.0; 3], [0.03; 3], [0.97; 3]).is_err());
    }

    #[test]
    fn monochrome_samples_one_channel_then_replicates_clumping() {
        let mono = render(true, false, 0.2, 0.89, [0.1, 30.0]);
        assert_eq!(mono.extract_channel(0), mono.extract_channel(1));
        assert_eq!(mono.extract_channel(0), mono.extract_channel(2));
        let color = render(false, false, 0.2, 0.89, [0.1, 30.0]);
        assert_ne!(color.extract_channel(0), color.extract_channel(1));
    }

    #[test]
    fn grain_blur_smooths_both_sampling_regimes() {
        for fast in [false, true] {
            let raw = render(false, fast, 12.0, 0.0, [0.0; 2]);
            let blurred = render(false, fast, 12.0, 0.89, [0.0; 2]);
            for ch in 0..3 {
                assert!(
                    variance(&blurred.extract_channel(ch)) < variance(&raw.extract_channel(ch))
                );
            }
        }
    }

    #[test]
    fn finishing_controls_preserve_sigma_across_f32_dispatch_boundary() {
        let below = render(false, false, 12.0, 2.99999999, [0.0; 2]);
        let boundary = render(false, false, 12.0, 3.0, [0.0; 2]);
        // Both controls narrow to the same f32. Preserve the Python float
        // until dispatch so these still select FIR and IIR respectively.
        assert!(
            below
                .data
                .iter()
                .zip(boundary.data)
                .any(|(&a, b)| (to_f64(a) - to_f64(b)).abs() > 1e-6)
        );
    }

    #[test]
    fn curve_overlap_changes_rms_calibration_without_changing_maxima() {
        let coincident = [[0.0, 0.0, 0.0], [0.5, 0.5, 0.5], [1.0, 1.0, 1.0]]
            .map(|row| row.map(|value| [value; 3]));
        let separated = [
            [0.0, 0.0, 0.0],
            [0.5, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.5, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 1.0, 0.5],
            [1.0, 1.0, 1.0],
        ]
        .map(|row| row.map(|value| [value; 3]));
        let together = prepared(&coincident, 3, 12.0);
        let apart = prepared(&separated, 3, 12.0);
        // Identical layer maxima do not imply identical realized peak RMS.
        assert!(together.n_particles[0][0] > apart.n_particles[0][0]);
    }

    #[test]
    fn curve_calibrated_scipy_realization_matches_original() {
        // Frozen direct-original output: experimental 28bf883, NumPy 2.5.4,
        // scipy 1.18.1; controlled production-rms-x1 from the algorithm audit.
        // All 256*256 Poisson draws precede any binomial draws in each plane,
        // so shrinking this image would invalidate these reference samples.
        let composite: Vec<[f64; 3]> = (0..129).map(|i| [i as f64 * (2.2 / 128.0); 3]).collect();
        let layers: Vec<[[f64; 3]; 3]> = composite
            .iter()
            .map(|row| [[row[0] * 0.3; 3], [row[0] * 0.3; 3], [row[0] * 0.4; 3]])
            .collect();
        let pixel_size = APERTURE_AREA_UM2.sqrt();
        let params = prepared(&layers, 3, pixel_size);
        let mut image = ImageBuf::new(256, 256);
        image.data.fill(from_f64(1.1));
        let output = apply_grain(
            &image,
            &composite,
            &layers,
            &params,
            pixel_size,
            0.0,
            0.0,
            [0.0; 2],
            0.0,
            0.0,
            false,
            false,
            false,
            0,
            &CpuBackend,
        )
        .unwrap();
        let expected = [
            1.1001908779144287,
            1.0948054790496826,
            1.0982413291931152,
            1.0969480276107788,
            1.0876505374908447,
            1.1050374507904053,
            1.0966005325317383,
            1.0829664468765259,
            1.09446120262146,
            1.0987576246261597,
            1.114262580871582,
            1.0951850414276123,
            1.099814534187317,
            1.1038390398025513,
            1.0962306261062622,
            1.0970349311828613,
            1.095268726348877,
            1.112637996673584,
            1.0988590717315674,
            1.0911765098571777,
            1.10648512840271,
            1.0965137481689453,
            1.1003903150558472,
            1.1008552312850952,
        ];
        for (&actual, reference) in output.data.iter().zip(expected) {
            assert!(
                (to_f64(actual) - reference).abs() < 3e-7,
                "original scipy realization: {} vs {reference}",
                to_f64(actual)
            );
        }
    }

    #[test]
    fn source_table_interpolation_keeps_f64_knots_and_right_biased_ties() {
        let axis = [[0.0; 3], [1.0; 3], [1.0; 3], [1.00000001; 3], [2.0; 3]];
        let layers = [0.0, 0.2, 0.4, 0.8, 1.0].map(|value| [[value; 3]; 3]);
        let inv_dx = inverse_differences(&axis, 0, 1.0);
        assert_eq!(
            interpolate_layer(1.0, 0, 0, &axis, &layers, &inv_dx, 1.0, 0.0),
            0.4f32
        );
        assert_eq!(
            interpolate_layer(1.000000005, 0, 0, &axis, &layers, &inv_dx, 1.0, 0.0),
            0.6f32
        );
        let negative = axis.map(|row| row.map(|value| -value));
        assert_eq!(
            interpolate_layer(-1.000000005, 0, 0, &negative, &layers, &inv_dx, -1.0, 0.0),
            0.6f32
        );
    }

    #[test]
    fn interpolated_work_density_adds_f64_ndarray_floor_before_sampling() {
        let axis = [[0.0; 3], [1.0; 3]];
        let layers = [[[0.0; 3]; 3], [[1.0; 3]; 3]];
        let inverse = inverse_differences(&axis, 0, 1.0);
        let input = 0.0006600660271942616f32;
        // Measured against the original NumPy 2.5.4 in-place ndarray
        // addition. Early floor narrowing yields 0.009660065174102783
        // instead, changing the subsequent discrete count conditioning.
        let absolute = interpolate_layer(input as f64, 0, 0, &axis, &layers, &inverse, 1.0, 0.009);
        assert_eq!(absolute as f64, 0.009660066105425358);
    }

    #[test]
    fn microstructure_continues_mt_in_hwc_order() {
        let mut state = rand_mt::Mt::new(22);
        for _ in 0..6 {
            rk_poisson(&mut state, 5.0);
        }
        let mut reference = GaussRng::from_mt(state.clone());
        let sigma2 = (0.15f64 * 0.15).ln_1p();
        let expected: Vec<f32> = (0..6)
            .map(|_| {
                let mut normal = reference.gauss();
                normal *= sigma2.sqrt();
                normal -= sigma2 * 0.5;
                normal.exp() as f32
            })
            .collect();
        let mut planes = vec![vec![1.0f32; 2]; 3];
        add_micro_structure(&mut planes, 2, 1, [0.0, 30.0], 0.2, state);
        for ch in 0..3 {
            assert_eq!(planes[ch], vec![expected[ch], expected[ch + 3]]);
        }
    }

    #[test]
    fn multiplicative_usm_uses_epsilon_in_positive_numerator() {
        let mut density = vec![0.0f32, 1e-10, 1e-7, 1e-5, 1e-4];
        let before = sum_f32(&density);
        apply_multiplicative_unsharp_mask(&mut density, 5, 1, 0.89, 0.5);
        assert_eq!(density[0], 0.0);
        // Below epsilon both positive inputs use the SAME numerator floor,
        // so the darker sample is no longer suppressed by D^(1+amount).
        assert!(density[1] > density[2] * 0.1);
        assert!((sum_f32(&density) - before).abs() < 2e-11);
    }
}
