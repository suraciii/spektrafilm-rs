// Stochastic grain generation.
// Poisson-binomial particle model with dye cloud blur and lognormal micro-structure.

use spektrafilm_gpu::ComputeBackend;
use spektrafilm_math::gaussian;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{Scalar, ZERO, from_f64};
use spektrafilm_math::stats::{self, FastStatsRng};
use rayon::prelude::*;
use std::time::Instant;

fn stage_timings_enabled() -> bool {
    std::env::var_os("SPEKTRAFILM_STAGE_TIMINGS").is_some()
}

fn print_stage_timing(enabled: bool, stage: &str, start: Instant) {
    if enabled {
        eprintln!("stage {stage}: {} ms", start.elapsed().as_millis());
    }
}

/// Seed of the micro-structure clumping field. Python draws it from numba's
/// unseeded thread-global RNG (`add_micro_structure` never reseeds), so the
/// upstream texture is nondeterministic; Rust pins one documented stream so
/// renders are reproducible.
const MICRO_STRUCTURE_SEED: u64 = 42;

/// Stream tags separating the poisson pass from the binomial pass inside
/// the fast-stats grain sampler (see `FastStatsRng::stream`).
const POISSON_TAG: u64 = 0;
const BINOMIAL_TAG: u64 = 1;

/// Apply the Poisson-binomial grain particle model to a single-channel density image.
///
/// Port of Python `layer_particle_model`. Density values flow through in `Scalar`
/// (full f64 precision in `precision-f64` mode); RNG sampling uses f64 directly.
///
/// `use_fast_stats` selects the sampling regime exactly like upstream: false
/// draws scipy.stats variates (bit-exact MT19937 `rk_poisson`/`rk_binomial`
/// port — `scipy.stats.poisson.rvs`/`binom.rvs` delegate to numpy's legacy
/// `RandomState`), true uses the `fast_stats` kernels (statistical parity
/// only — upstream's numba thread-global RNG is nondeterministic, so the
/// Rust port drives each element from its own deterministic tagged stream).
pub fn layer_particle_model(
    density: &[Scalar],
    width: u32,
    height: u32,
    density_max: f64,
    n_particles_per_pixel: f64,
    grain_uniformity: f64,
    seed: u64,
    blur_particle: f32,
    use_fast_stats: bool,
) -> Vec<Scalar> {
    // Inputs are f64 to match Python's `layer_particle_model` —
    // density_max/uniformity/particle-area come from JSON profiles at full
    // f64 precision in Python. Passing them as f32 in Rust truncated
    // ~7-decimal noise into `od_particle` and `saturation` which then
    // shifted every Poisson lambda by ~5e-8, producing a different
    // RNG stream and thus visibly different grain patterns.
    let dmax = density_max;
    let od_particle = dmax / n_particles_per_pixel;
    let gu = grain_uniformity;
    let npp = n_particles_per_pixel;

    let n = density.len();

    let mut p_arr = Vec::with_capacity(n);
    let mut sat_arr = Vec::with_capacity(n);
    for &d in density.iter() {
        let p = ((d as f64) / dmax).clamp(1e-6, 1.0 - 1e-6);
        let saturation = 1.0 - p * gu * (1.0 - 1e-6);
        p_arr.push(p);
        sat_arr.push(saturation);
    }

    let grain: Vec<Scalar>;
    if use_fast_stats {
        // `fast_stats` regime: Python runs `fast_poisson(lam_array)` then
        // `fast_binomial(seeds, p)` as whole-array numba kernels whose
        // element draws are independent, so per-element tagged streams
        // reproduce the same statistical regime deterministically (and in
        // parallel) regardless of numba's thread schedule.
        let lam: Vec<f64> = sat_arr.iter().map(|&sat| npp / sat).collect();
        let seeds = stats::fast_poisson_array(seed, POISSON_TAG, &lam);
        grain = stats::fast_binomial_array(seed, BINOMIAL_TAG, &seeds, &p_arr)
            .into_iter()
            .zip(&sat_arr)
            .map(|(developed, &sat)| from_f64((developed as f64) * od_particle * sat))
            .collect();
    } else {
        // scipy regime — Python-bit-exact RNG: numpy's MT19937 +
        // RandomState distribution algorithms (`rk_poisson` / `rk_binomial`).
        // Numpy's seed-to-stream takes a `u32`, so truncate the 64-bit seed
        // — the grain paths never need more than 24 distinct streams
        // (3 channels × 3 sub-layers). Iteration is single-threaded so the
        // RNG state advances in row-major order exactly like
        // `numpy.random.RandomState(seed).poisson(lambda_array)` does.
        //
        // CRITICAL — match Python's call order: it does
        //   seeds = np.random.poisson(lam_array)   # whole array, then
        //   grain = np.random.binomial(seeds, p)   # whole array.
        // So the RNG must draw ALL Poisson samples first, THEN ALL Binomial
        // samples. Interleaving Poisson/Binomial per pixel gives a
        // different RNG state at each draw and produces a completely
        // different grain pattern.
        let mut rng = rand_mt::Mt::new(seed as u32);
        let mut seeds = Vec::with_capacity(n);
        for sat in &sat_arr {
            seeds.push(spektrafilm_math::numpy_rng::rk_poisson(&mut rng, npp / sat));
        }
        let mut out = vec![ZERO; n];
        for (i, slot) in out.iter_mut().enumerate() {
            let developed =
                spektrafilm_math::numpy_rng::rk_binomial(&mut rng, seeds[i], p_arr[i]);
            *slot = from_f64((developed as f64) * od_particle * sat_arr[i]);
        }
        grain = out;
    }
    finish_grain(grain, width, height, blur_particle, od_particle)
}

/// Shared tail of [`layer_particle_model`]: optional dye-cloud blur.
fn finish_grain(
    grain: Vec<Scalar>,
    width: u32,
    height: u32,
    blur_particle: f32,
    od_particle: f64,
) -> Vec<Scalar> {

    // Python gates on `blur_particle > 0` and hands `sigma = blur_particle
    // * sqrt(od_particle)` to fast_gaussian_filter, whose FIR path rounds
    // the kernel radius down (`int(3σ + 0.5)`); a radius of 0 is an exact
    // identity, so no extra threshold is needed here.
    if blur_particle > 0.0 {
        let sigma = blur_particle as f64 * od_particle.sqrt();
        return gaussian::gaussian_blur_channel(&grain, width, height, sigma);
    }

    grain
}

/// Convert upstream RMS granularity values (sigma × 1000 at a 48 µm
/// aperture) into the coarsest particle area used by the sampler.
pub fn particle_area_from_rms_granularity(
    rms_granularity: [f64; 3],
    density_max: [f64; 3],
    density_min: [f64; 3],
    uniformity: [f64; 3],
) -> f64 {
    let aperture_area = std::f64::consts::PI * 24.0f64.powi(2);
    let mut total = 0.0;
    let mut count = 0;
    for c in 0..3 {
        if rms_granularity[c] > 0.0 {
            let sigma = rms_granularity[c] / 1000.0;
            let reference = 1.0 + density_min[c];
            let variance = (reference * (density_max[c] - uniformity[c] * reference)).max(1e-6);
            total += sigma * sigma * aperture_area / variance;
            count += 1;
        }
    }
    if count == 0 {
        0.0
    } else {
        total / count as f64
    }
}

/// Apply grain to a CMY density image.
///
/// Port of Python `apply_grain_to_density` (the composite-density path,
/// `grain.sublayers_active == false`). Upstream never passes `use_fast_stats`
/// on this path, so it always samples the scipy regime.
#[allow(clippy::too_many_arguments)]
pub fn apply_grain_to_density(
    density_cmy: &ImageBuf,
    pixel_size_um: f64,
    particle_area_um2: f64,
    particle_scale: [f64; 3],
    density_min: [f64; 3],
    density_max_curves: [f64; 3],
    grain_uniformity: [f64; 3],
    grain_blur: f32,
    n_sub_layers: u32,
    monochrome: bool,
    backend: &dyn ComputeBackend,
) -> ImageBuf {
    let stage_timings = stage_timings_enabled();
    let w = density_cmy.width;
    let h = density_cmy.height;
    let pixel_area = pixel_size_um * pixel_size_um;
    let density_max: [f64; 3] = [
        density_max_curves[0] + density_min[0],
        density_max_curves[1] + density_min[1],
        density_max_curves[2] + density_min[2],
    ];

    let channels: Vec<(usize, Vec<Scalar>)> = (0..3)
        .into_par_iter()
        .map(|ch| {
            let t_ch = Instant::now();
            let particle_area = particle_area_um2 * particle_scale[ch];
            let mut n_particles = pixel_area / particle_area;
            if n_sub_layers > 1 {
                n_particles /= n_sub_layers as f64;
            }

            // Add density_min to input (kept in Scalar precision)
            let dmin_s = from_f64(density_min[ch]);
            let t = Instant::now();
            let density_ch: Vec<Scalar> =
                density_cmy.pixels().map(|px| px[ch] + dmin_s).collect();
            print_stage_timing(stage_timings, "grain.extract_channel", t);

            let mut grain_sum = vec![ZERO; density_ch.len()];

            for sl in 0..n_sub_layers {
                let t = Instant::now();
                // Monochrome (B&W): one shared noise field — every lane draws
                // from channel 0's RNG stream (upstream n_channels==1 has a
                // single emulsion, seed ch=0).
                let seed_ch = if monochrome { 0 } else { ch as u64 };
                let seed = seed_ch + (sl as u64) * 10;
                let g = layer_particle_model(
                    &density_ch,
                    w,
                    h,
                    density_max[ch],
                    n_particles,
                    grain_uniformity[ch],
                    seed,
                    0.0,
                    false,
                );
                print_stage_timing(stage_timings, "grain.layer_particle_model", t);
                for (s, &v) in grain_sum.iter_mut().zip(g.iter()) {
                    *s += v;
                }
            }

            // Average sub-layers
            let scale = from_f64(1.0 / n_sub_layers as f64);
            for v in &mut grain_sum {
                *v = *v * scale - dmin_s;
            }

            print_stage_timing(stage_timings, "grain.channel_total", t_ch);
            (ch, grain_sum)
        })
        .collect();

    let mut out = ImageBuf::new(w, h);
    for (ch, grain_sum) in channels {
        out.write_channel(ch, &grain_sum);
    }

    // Final blur — typically a few px sigma at 1–6 MP, big enough that the
    // GPU separable kernel wins. Upstream gates the composite path on
    // `sigma_blur_pixel > 0.4`.
    if grain_blur > 0.4 {
        let t = Instant::now();
        out = backend.gaussian_blur(&out, grain_blur);
        print_stage_timing(stage_timings, "grain.final_blur", t);
    }

    out
}

/// Port of Python `add_micro_structure`: multiply each density channel by a
/// lognormal "clumping" field with linear-space mean 1 and standard
/// deviation `micro_structure[1] · 0.001 / pixel_size_um` (the control is in
/// nm), optionally blurred by `micro_structure[0] / pixel_size_um` pixels.
/// The clumping field is only drawn when that sigma exceeds 0.05 and the
/// blur only applies above 0.4 px — both gates exactly as upstream.
///
/// Upstream always uses the `fast_stats` lognormal here, regardless of the
/// `use_fast_stats` setting, so this function does too.
fn add_micro_structure(
    planes: &mut [Vec<Scalar>; 3],
    width: u32,
    height: u32,
    micro_structure: [f32; 2],
    pixel_size_um: f64,
) {
    // Keep the unit math and Gaussian sigma in f64.
    let blur_px = micro_structure[0] as f64 / pixel_size_um;
    let sigma = micro_structure[1] as f64 * 0.001 / pixel_size_um;
    if sigma <= 0.05 {
        return;
    }

    let n = planes[0].len();
    for ch in 0..3 {
        // Clumping field: iid lognormal(mean=1, std=sigma) per channel —
        // each element from its own deterministic stream (Python draws the
        // whole (h, w, 3) array from the unseeded numba global RNG).
        let mut clumping: Vec<Scalar> = (0..n)
            .into_par_iter()
            .map(|i| {
                let mut rng = FastStatsRng::stream(MICRO_STRUCTURE_SEED, ch as u64, i as u64);
                from_f64(stats::fast_lognormal_from_mean_std(&mut rng, 1.0, sigma))
            })
            .collect();
        if blur_px > 0.4 {
            clumping = gaussian::gaussian_blur_channel(&clumping, width, height, blur_px);
        }
        let plane = &mut planes[ch];
        for (v, &c) in plane.iter_mut().zip(clumping.iter()) {
            *v *= c;
        }
    }
}

/// Apply layered (per-sublayer) grain to a CMY density image.
///
/// Port of Python `apply_grain_to_density_layers` (the
/// `grain.sublayers_active == true` path). `density_cmy_layers` holds the
/// per-sublayer densities from `interp_density_cmy_layers` as
/// `[sublayer][channel]` planes and `density_max_layers` the per-sublayer
/// curve maxima (`np.nanmax(density_curves_layers, axis=0)`) in the same
/// `[sublayer][channel]` layout.
///
/// Per sublayer: the density maximum splits into fractions that scale both
/// the per-layer `density_min` offset and the particle count
/// (`pixel_area · fraction / (particle_area · scale · scale_layer)`), each
/// layer samples its own Poisson-binomial field seeded `ch + sl·10` (0 for
/// every channel when `monochrome`), and the dye-cloud blur runs inside the
/// particle model at `blur_particle · sqrt(od_particle)`. The summed layers
/// are multiplied by the lognormal micro-structure, offset by the base
/// `density_min`, and finally blurred by `grain_blur` (gated `> 0`, unlike
/// the composite path's `> 0.4`).
#[allow(clippy::too_many_arguments)]
pub fn apply_grain_to_density_layers(
    density_cmy_layers: &[[Vec<Scalar>; 3]; 3],
    density_max_layers: &[[f64; 3]; 3],
    width: u32,
    height: u32,
    pixel_size_um: f64,
    particle_area_um2: f64,
    particle_scale: [f64; 3],
    particle_scale_layers: [f64; 3],
    density_min: [f64; 3],
    grain_uniformity: [f64; 3],
    grain_blur: f32,
    grain_blur_dye_clouds_um: f32,
    grain_micro_structure: [f32; 2],
    monochrome: bool,
    use_fast_stats: bool,
    backend: &dyn ComputeBackend,
) -> ImageBuf {
    let stage_timings = stage_timings_enabled();
    let pixel_area = pixel_size_um * pixel_size_um;

    // Python:
    //   density_max_total      = sum(density_max_layers, axis=sublayer)
    //   density_max_fractions  = density_max_layers / density_max_total
    //   density_min_layers     = density_max_fractions * density_min
    //   density_max_layers    += density_min_layers
    //   n_particles_per_pixel  = pixel_area * fraction / particle_area_layer
    let mut density_max_total = [0.0f64; 3];
    for sl in 0..3 {
        for ch in 0..3 {
            density_max_total[ch] += density_max_layers[sl][ch];
        }
    }
    let mut fractions = [[0.0f64; 3]; 3];
    let mut density_min_layers = [[0.0f64; 3]; 3];
    let mut density_max_adj = [[0.0f64; 3]; 3];
    let mut n_particles = [[0.0f64; 3]; 3];
    for sl in 0..3 {
        for ch in 0..3 {
            fractions[sl][ch] = density_max_layers[sl][ch] / density_max_total[ch];
            density_min_layers[sl][ch] = fractions[sl][ch] * density_min[ch];
            density_max_adj[sl][ch] = density_max_layers[sl][ch] + density_min_layers[sl][ch];
            let particle_area_layer =
                particle_area_um2 * particle_scale[ch] * particle_scale_layers[sl];
            n_particles[sl][ch] = pixel_area * fractions[sl][ch] / particle_area_layer;
        }
    }

    // Python adds density_min_layers to the interpolated layers once, then
    // sums the 3 sublayer particle fields per channel with seeds
    // `seed[ch] + sl*10` (`seed = [0, 1, 2]`; monochrome keeps the shared
    // channel-0 stream like the composite path).
    let channels: Vec<(usize, Vec<Scalar>)> = (0..3)
        .into_par_iter()
        .map(|ch| {
            let t_ch = Instant::now();
            let mut grain_sum = vec![ZERO; (width as usize) * (height as usize)];
            for sl in 0..3 {
                let t = Instant::now();
                let seed_ch = if monochrome { 0 } else { ch as u64 };
                let seed = seed_ch + (sl as u64) * 10;
                let dmin_l = from_f64(density_min_layers[sl][ch]);
                let density_sl: Vec<Scalar> =
                    density_cmy_layers[sl][ch].iter().map(|&v| v + dmin_l).collect();
                let g = layer_particle_model(
                    &density_sl,
                    width,
                    height,
                    density_max_adj[sl][ch],
                    n_particles[sl][ch],
                    grain_uniformity[ch],
                    seed,
                    grain_blur_dye_clouds_um,
                    use_fast_stats,
                );
                print_stage_timing(stage_timings, "grain.layer_particle_model", t);
                for (s, &v) in grain_sum.iter_mut().zip(g.iter()) {
                    *s += v;
                }
            }
            print_stage_timing(stage_timings, "grain.channel_total", t_ch);
            (ch, grain_sum)
        })
        .collect();

    let mut planes = [vec![], vec![], vec![]];
    for (ch, plane) in channels {
        planes[ch] = plane;
    }
    add_micro_structure(
        &mut planes,
        width,
        height,
        grain_micro_structure,
        pixel_size_um,
    );

    let mut out = ImageBuf::new(width, height);
    for ch in 0..3 {
        let dmin_s = from_f64(density_min[ch]);
        let plane: Vec<Scalar> = planes[ch].iter().map(|&v| v - dmin_s).collect();
        out.write_channel(ch, &plane);
    }

    // Final blur — upstream gates the layered path on `grain_blur > 0`.
    if grain_blur > 0.0 {
        let t = Instant::now();
        out = backend.gaussian_blur(&out, grain_blur);
        print_stage_timing(stage_timings, "grain.final_blur", t);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_gpu::cpu_backend::CpuBackend;
    use spektrafilm_math::precision::to_f64;

    const DENSITY_MIN: [f64; 3] = [0.03, 0.03, 0.03];
    const UNIFORMITY: [f64; 3] = [0.97, 0.99, 0.97];
    const PARTICLE_SCALE_LAYERS: [f64; 3] = [2.0, 1.0, 0.5];
    const PARTICLE_SCALE: [f64; 3] = [1.6, 1.6, 3.2];
    /// Sublayer split of the composite density (must sum to 1).
    const SPLIT: [f64; 3] = [0.3, 0.3, 0.4];

    /// Layer planes for a uniform composite density: sublayer sl carries
    /// `SPLIT[sl] · d[ch]`.
    fn uniform_layers(d: [f64; 3]) -> [[Vec<Scalar>; 3]; 3] {
        let mut planes = std::array::from_fn(|_| std::array::from_fn(|_| vec![ZERO; 9]));
        for sl in 0..3 {
            for ch in 0..3 {
                for v in &mut planes[sl][ch] {
                    *v = from_f64(SPLIT[sl] * d[ch]);
                }
            }
        }
        planes
    }

    /// Layer maxima proportional to the split, composite max ≈ 2.2.
    fn split_max_layers() -> [[f64; 3]; 3] {
        [
            [SPLIT[0] * 2.2; 3],
            [SPLIT[1] * 2.2; 3],
            [SPLIT[2] * 2.2; 3],
        ]
    }

    fn mean(plane: &[Scalar]) -> f64 {
        plane.iter().map(|&v| to_f64(v)).sum::<f64>() / plane.len() as f64
    }

    fn var(plane: &[Scalar]) -> f64 {
        let m = mean(plane);
        plane.iter().map(|&v| to_f64(v) * to_f64(v)).sum::<f64>() / plane.len() as f64 - m * m
    }

    /// Layered grain on a 3×3 uniform image (final blur off to inspect the
    /// raw particle field).
    fn layered(
        d: [f64; 3],
        particle_scale_layers: [f64; 3],
        dye_blur: f32,
        micro_structure: [f32; 2],
        pixel_size_um: f64,
        use_fast_stats: bool,
    ) -> ImageBuf {
        apply_grain_to_density_layers(
            &uniform_layers(d),
            &split_max_layers(),
            3,
            3,
            pixel_size_um,
            0.2,
            PARTICLE_SCALE,
            particle_scale_layers,
            DENSITY_MIN,
            UNIFORMITY,
            0.0,
            dye_blur,
            micro_structure,
            false,
            use_fast_stats,
            &CpuBackend,
        )
    }

    #[test]
    fn layered_grain_is_deterministic_per_regime() {
        let d = [0.8, 1.0, 1.2];
        for use_fast_stats in [false, true] {
            let a = layered(d, PARTICLE_SCALE_LAYERS, 1.0, [0.2, 30.0], 12.0, use_fast_stats);
            let b = layered(d, PARTICLE_SCALE_LAYERS, 1.0, [0.2, 30.0], 12.0, use_fast_stats);
            assert_eq!(a.data, b.data, "fast_stats={use_fast_stats}");
        }
    }

    #[test]
    fn layered_grain_mean_tracks_composite_density() {
        // The particle model is mean-preserving (E[grain] = density offset)
        // and the 30/30/40 split carries the full composite density, so the
        // output mean must sit near the input density. At 12 µm pixels the
        // micro-structure sigma (30 nm / 12 µm) stays below the 0.05 gate —
        // upstream defaults disable the clumping field there.
        let d = [0.8, 1.0, 1.2];
        let out = layered(d, PARTICLE_SCALE_LAYERS, 1.0, [0.2, 30.0], 12.0, false);
        for ch in 0..3 {
            let m = mean(&out.extract_channel(ch));
            assert!(
                (m - d[ch]).abs() < 0.15,
                "ch {ch}: mean {m} vs density {}",
                d[ch]
            );
        }
        assert!(var(&out.extract_channel(0)) > 1e-6);
    }

    #[test]
    fn layered_particle_scale_layers_change_output() {
        let d = [0.9, 0.9, 0.9];
        let a = layered(d, [2.0, 1.0, 0.5], 0.0, [0.0, 0.0], 12.0, false);
        let b = layered(d, [0.5, 1.0, 2.0], 0.0, [0.0, 0.0], 12.0, false);
        assert_ne!(a.data, b.data);
        // Uneven layer scales also shift the variance structure.
        assert!((var(&a.extract_channel(0)) - var(&b.extract_channel(0))).abs() > 1e-9);
    }

    #[test]
    fn layered_dye_cloud_blur_reduces_variance() {
        let d = [0.9, 0.9, 0.9];
        let off = layered(d, PARTICLE_SCALE_LAYERS, 0.0, [0.0, 0.0], 12.0, false);
        let on = layered(d, PARTICLE_SCALE_LAYERS, 40.0, [0.0, 0.0], 12.0, false);
        assert_ne!(off.data, on.data);
        // A large dye-cloud blur smooths the particle field: per-pixel
        // variance shrinks.
        assert!(var(&on.extract_channel(0)) < var(&off.extract_channel(0)));
    }

    #[test]
    fn layered_micro_structure_multiplies_output() {
        // 0.2 µm pixels push the micro-structure sigma over the 0.05 gate
        // (30 nm / 0.2 µm = 0.15 > 0.05); 8 nm stays below (0.04 ≤ 0.05).
        // Dye blur 0 isolates the clumping multiply.
        let d = [0.9, 0.9, 0.9];
        let off = layered(d, PARTICLE_SCALE_LAYERS, 0.0, [0.0, 8.0], 0.2, false);
        let on = layered(d, PARTICLE_SCALE_LAYERS, 0.0, [0.0, 30.0], 0.2, false);
        assert_ne!(off.data, on.data);
        // The multiplicative field also inflates the variance.
        assert!(var(&on.extract_channel(0)) > var(&off.extract_channel(0)));
    }

    #[test]
    fn layered_micro_structure_blur_smooths_the_clumping() {
        // Same sigma gate open, different blur controls: the clumping blur
        // (0.4 / 0.2 µm = 2 px > 0.4) must change the field again.
        let d = [0.9, 0.9, 0.9];
        let unblurred = layered(d, PARTICLE_SCALE_LAYERS, 0.0, [0.0, 30.0], 0.2, false);
        let blurred = layered(d, PARTICLE_SCALE_LAYERS, 0.0, [0.4, 30.0], 0.2, false);
        assert_ne!(unblurred.data, blurred.data);
    }

    #[test]
    fn layered_use_fast_stats_changes_texture_not_regime() {
        let d = [0.8, 1.0, 1.2];
        let scipy = layered(d, PARTICLE_SCALE_LAYERS, 1.0, [0.2, 30.0], 12.0, false);
        let fast = layered(d, PARTICLE_SCALE_LAYERS, 1.0, [0.2, 30.0], 12.0, true);
        // Different RNG regimes → different texture, same statistical
        // center (bounded comparison — bit parity is impossible upstream).
        assert_ne!(scipy.data, fast.data);
        for ch in 0..3 {
            let (m_s, m_f) = (
                mean(&scipy.extract_channel(ch)),
                mean(&fast.extract_channel(ch)),
            );
            assert!(
                (m_s - m_f).abs() < 0.2,
                "ch {ch}: scipy mean {m_s} vs fast mean {m_f}"
            );
        }
    }

    #[test]
    fn layered_monochrome_shares_the_noise_field() {
        // Identical per-channel inputs and uniformities: monochrome seeds
        // every channel from stream 0 → identical planes; the color path
        // decorrelates them.
        let layers = uniform_layers([0.9; 3]);
        let run = |monochrome: bool| {
            apply_grain_to_density_layers(
                &layers,
                &split_max_layers(),
                3,
                3,
                12.0,
                0.2,
                [PARTICLE_SCALE[0]; 3],
                PARTICLE_SCALE_LAYERS,
                DENSITY_MIN,
                [0.97; 3],
                0.0,
                0.0,
                [0.0, 0.0],
                monochrome,
                false,
                &CpuBackend,
            )
        };
        let mono = run(true);
        assert_eq!(mono.extract_channel(0), mono.extract_channel(1));
        assert_eq!(mono.extract_channel(0), mono.extract_channel(2));
        let color = run(false);
        assert_ne!(color.extract_channel(0), color.extract_channel(1));
    }

    #[test]
    fn composite_path_is_deterministic() {
        // Regression guard for the false-path dispatch: the composite model
        // consumes only its own controls (no layer scales, dye-cloud blur
        // or micro-structure) and stays deterministic.
        let mut img = ImageBuf::new(3, 3);
        for y in 0..3u32 {
            for x in 0..3u32 {
                img.set(x, y, [from_f64(0.8), from_f64(1.0), from_f64(1.2)]);
            }
        }
        let run = || {
            apply_grain_to_density(
                &img,
                12.0,
                0.2,
                PARTICLE_SCALE,
                DENSITY_MIN,
                [2.2, 2.2, 2.2],
                UNIFORMITY,
                0.0,
                1,
                false,
                &CpuBackend,
            )
        };
        assert_eq!(run().data, run().data);
    }

    #[test]
    fn layer_particle_model_dye_blur_gate_matches_python() {
        // Python gates on `blur_particle > 0` only; the gaussian's radius
        // rounding (`int(3σ + 0.5)`) makes sub-1/6 σ an exact identity.
        let density = vec![from_f64(1.0); 16];
        let base = layer_particle_model(&density, 4, 4, 2.2, 200.0, 0.98, 3, 0.0, false);
        let tiny = layer_particle_model(&density, 4, 4, 2.2, 200.0, 0.98, 3, 1e-3, false);
        assert_eq!(base, tiny, "radius-0 dye blur must be identity");
        let big = layer_particle_model(&density, 4, 4, 2.2, 200.0, 0.98, 3, 100.0, false);
        assert_ne!(base, big, "large dye blur must smooth the field");
    }

    #[test]
    fn layer_particle_model_regimes_share_statistics() {
        // Same inputs, both regimes: means must agree within sampling noise
        // (bounded statistical comparison — the fast regime cannot be
        // bit-identical to anything, including itself across numba runs).
        let density = vec![from_f64(1.0); 4096];
        let scipy = layer_particle_model(&density, 64, 64, 2.2, 200.0, 0.98, 3, 0.0, false);
        let fast = layer_particle_model(&density, 64, 64, 2.2, 200.0, 0.98, 3, 0.0, true);
        assert_ne!(scipy, fast);
        let (m_s, m_f) = (mean(&scipy), mean(&fast));
        assert!((m_s - m_f).abs() < 0.05, "scipy {m_s} vs fast {m_f}");
        // E[grain] = density (mean-preserving model, no dye blur).
        assert!((m_s - 1.0).abs() < 0.05, "scipy mean {m_s}");
        assert!((m_f - 1.0).abs() < 0.05, "fast mean {m_f}");
    }
}
