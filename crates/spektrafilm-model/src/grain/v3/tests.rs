use super::*;

/// Source pixel pitch used by these tests, in micrometers.
const PITCH: f64 = 6.0;
/// Default dye support (design §Resolved parameter contract).
const SIGMA: f64 = 3.5;
/// Raw layer maxima `[layer]`, equal across channels.
const MAXIMA: [f64; 3] = [0.18, 0.09, 0.03];
/// Base density offset, equal across channels.
const MIN_DENSITY: f64 = 0.03;

fn curves() -> (Vec<[f64; 3]>, Vec<[[f64; 3]; 3]>) {
    let knots = 17usize;
    let t = |k: usize| k as f64 / (knots - 1) as f64;
    let composite = (0..knots).map(|k| [t(k); 3]).collect();
    let layers = (0..knots)
        .map(|k| std::array::from_fn(|l| [MAXIMA[l] * t(k); 3]))
        .collect();
    (composite, layers)
}

fn params(cell_um: Option<f64>, expectation: bool, seed: u64, area: f64) -> GrainV3Params {
    let (composite_curves, layer_curves) = curves();
    GrainV3Params {
        monochrome: false,
        support_um: SIGMA,
        cell_um,
        event_area_um2: area,
        // The shipped default. The correlation tests set their own
        // values instead of relying on this helper.
        channel_correlation: 0.0,
        channel_scale: [1.0; 3],
        particle_scale_sublayers: [1.0; 3],
        density_min: [MIN_DENSITY; 3],
        uniformity: [0.5; 3],
        layer_max: std::array::from_fn(|l| [MAXIMA[l]; 3]),
        composite_curves,
        layer_curves,
        positive_film: false,
        seed,
        expectation,
    }
}

#[test]
fn cell_response_matches_independent_fractional_optical_integral() {
    let p = params(None, false, 1234, 1.28);
    let target = ImageBuf::from_data(16, 16, vec![from_f64(0.15); 16 * 16 * 3]);
    let h = SIGMA / 2.0;
    let coarse = ReadoutGrid {
        origin_um: [9.13, 8.27],
        pitch_um: [5.31, 4.73],
        width: 6,
        height: 5,
    };
    let optical = |density: &ImageBuf| {
        let mut out = density.clone();
        for pixel in out.data.as_chunks_mut::<3>().0 {
            let cmy = [to_f64(pixel[0]), to_f64(pixel[1]), to_f64(pixel[2])];
            for (c, value) in pixel.iter_mut().enumerate() {
                *value = from_f64(10.0f64.powf(-cmy[c] - 0.4 * cmy[(c + 1) % 3]));
            }
        }
        Ok(out)
    };
    let actual = apply_with_response(&target, PITCH, coarse, &p, optical).unwrap();
    let fine_grid = ReadoutGrid {
        origin_um: [7.0, 7.0],
        pitch_um: [h; 2],
        width: 24,
        height: 20,
    };
    let cells = apply(&target, PITCH, fine_grid, &p).unwrap();
    let light = optical(&cells).unwrap();
    for y in 0..coarse.height as usize {
        for x in 0..coarse.width as usize {
            let lo = [
                coarse.origin_um[0] + x as f64 * coarse.pitch_um[0],
                coarse.origin_um[1] + y as f64 * coarse.pitch_um[1],
            ];
            let hi = [lo[0] + coarse.pitch_um[0], lo[1] + coarse.pitch_um[1]];
            let mut expected = [0.0; 3];
            for j in 0..fine_grid.height as usize {
                let cy = fine_grid.origin_um[1] + j as f64 * h;
                let dy = (hi[1].min(cy + h) - lo[1].max(cy)).max(0.0);
                for i in 0..fine_grid.width as usize {
                    let cx = fine_grid.origin_um[0] + i as f64 * h;
                    let dx = (hi[0].min(cx + h) - lo[0].max(cx)).max(0.0);
                    for (c, value) in expected.iter_mut().enumerate() {
                        *value += dx
                            * dy
                            * to_f64(light.data[(j * fine_grid.width as usize + i) * 3 + c])
                            / (coarse.pitch_um[0] * coarse.pitch_um[1]);
                    }
                }
            }
            for (c, value) in expected.iter().enumerate() {
                assert!(
                    (to_f64(actual.data[(y * coarse.width as usize + x) * 3 + c]) - value).abs()
                        < 2e-7
                );
            }
        }
    }
    let narrow = apply_impl(&target, PITCH, coarse, &p, 1_100, Some(&optical)).unwrap();
    for (&a, &b) in actual.data.iter().zip(&narrow.data) {
        assert!((to_f64(a) - to_f64(b)).abs() < 2e-7);
    }
    let crop = ReadoutGrid {
        origin_um: [
            coarse.origin_um[0] + coarse.pitch_um[0],
            coarse.origin_um[1] + coarse.pitch_um[1],
        ],
        width: 3,
        height: 2,
        ..coarse
    };
    let cropped = apply_with_response(&target, PITCH, crop, &p, optical).unwrap();
    for y in 0..2 {
        for x in 0..3 {
            for c in 0..3 {
                assert!(
                    (to_f64(cropped.data[(y * 3 + x) * 3 + c])
                        - to_f64(actual.data[((y + 1) * 6 + x + 1) * 3 + c]))
                    .abs()
                        < 2e-7
                );
            }
        }
    }
}

#[derive(Clone, Copy)]
struct SourceRect {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

fn grid(rect: SourceRect, out: (u32, u32), pitch: f64) -> ReadoutGrid {
    ReadoutGrid {
        origin_um: [rect.x as f64 * pitch, rect.y as f64 * pitch],
        pitch_um: [
            rect.width as f64 * pitch / out.0 as f64,
            rect.height as f64 * pitch / out.1 as f64,
        ],
        width: out.0,
        height: out.1,
    }
}

fn apply_region(
    target: &ImageBuf,
    pitch: f64,
    rect: SourceRect,
    width: u32,
    height: u32,
    p: &GrainV3Params,
) -> Result<ImageBuf, String> {
    apply(target, pitch, grid(rect, (width, height), pitch), p)
}

fn region(width: u32, height: u32) -> SourceRect {
    SourceRect {
        x: 0,
        y: 0,
        width: width as usize,
        height: height as usize,
    }
}

fn flat(width: u32, height: u32, value: f64) -> ImageBuf {
    let mut image = ImageBuf::new(width, height);
    for pixel in image.pixels_mut() {
        pixel.fill(from_f64(value));
    }
    image
}

/// Slanted edge, a narrow line, and flat dark/bright areas.
fn patterned(width: u32, height: u32) -> ImageBuf {
    let mut image = ImageBuf::new(width, height);
    let line = (width * 5 / 8) as usize;
    let row_len = width as usize * 3;
    for (y, row) in image.data.chunks_mut(row_len).enumerate() {
        let edge = 0.25 * width as f64 + 0.3 * y as f64;
        for (x, pixel) in row.chunks_mut(3).enumerate() {
            let mut value = if (x as f64) < edge { 0.05 } else { 0.24 };
            if x == line || x == line + 1 {
                value = 0.28;
            }
            pixel.fill(from_f64(value));
        }
    }
    image
}

fn render(p: &GrainV3Params, image: &ImageBuf, rect: SourceRect, out: (u32, u32)) -> ImageBuf {
    apply_region(image, PITCH, rect, out.0, out.1, p).expect("Dye render")
}

fn pixel(image: &ImageBuf, px: usize, py: usize) -> [f64; 3] {
    let index = (py * image.width as usize + px) * 3;
    std::array::from_fn(|c| to_f64(image.data[index + c]))
}

/// Output tolerance of this numeric precision.
fn precision_tol() -> f64 {
    if cfg!(feature = "precision-f64") {
        1e-11
    } else {
        2e-5
    }
}

/// Independent oracle: the normalized Gaussian truncated to
/// `[-4 sigma, 4 sigma]`.
fn trunc_cdf(x: f64, sigma: f64) -> f64 {
    let cdf = |v: f64| 0.5 * (1.0 + libm::erf(v / std::f64::consts::SQRT_2));
    let raw = (cdf(x / sigma) - cdf(-4.0)) / (cdf(4.0) - cdf(-4.0));
    raw.clamp(0.0, 1.0)
}

/// Independent oracle for the expected density of one output footprint.
///
/// Midpoint quadrature over the footprint of the exact CDF-based integral
/// of the truncated Gaussian over each source rectangle, with the same
/// constant edge extension. It shares no filter or overlap helper with
/// production.
fn oracle_readout(
    image: &ImageBuf,
    rect: SourceRect,
    out: (u32, u32),
    px: usize,
    py: usize,
    step_um: f64,
) -> [f64; 3] {
    let width = image.width as usize;
    let height = image.height as usize;
    let x0 = rect.x as f64 * PITCH;
    let y0 = rect.y as f64 * PITCH;
    let col = rect.width as f64 * PITCH / out.0 as f64;
    let row = rect.height as f64 * PITCH / out.1 as f64;
    let (ax, bx) = (x0 + px as f64 * col, x0 + (px + 1) as f64 * col);
    let (ay, by) = (y0 + py as f64 * row, y0 + (py + 1) as f64 * row);
    let nx = ((bx - ax) / step_um).round().max(1.0) as usize;
    let ny = ((by - ay) / step_um).round().max(1.0) as usize;
    let dx = (bx - ax) / nx as f64;
    let dy = (by - ay) / ny as f64;
    let reach = SUPPORT_SIGMAS * SIGMA;
    let mut acc = [0.0f64; 3];
    for iy in 0..ny {
        let y = ay + (iy as f64 + 0.5) * dy;
        let sy_lo = ((y - reach) / PITCH).floor() as i64;
        let sy_hi = ((y + reach) / PITCH).ceil() as i64;
        for ix in 0..nx {
            let x = ax + (ix as f64 + 0.5) * dx;
            let sx_lo = ((x - reach) / PITCH).floor() as i64;
            let sx_hi = ((x + reach) / PITCH).ceil() as i64;
            for sy in sy_lo..=sy_hi {
                let wy = trunc_cdf(y - sy as f64 * PITCH, SIGMA)
                    - trunc_cdf(y - (sy + 1) as f64 * PITCH, SIGMA);
                if wy <= 0.0 {
                    continue;
                }
                let row_index = sy.clamp(0, height as i64 - 1) as usize * width;
                for sx in sx_lo..=sx_hi {
                    let wx = trunc_cdf(x - sx as f64 * PITCH, SIGMA)
                        - trunc_cdf(x - (sx + 1) as f64 * PITCH, SIGMA);
                    if wx <= 0.0 {
                        continue;
                    }
                    let column = sx.clamp(0, width as i64 - 1) as usize;
                    let index = (row_index + column) * 3;
                    let weight = wx * wy;
                    for (c, value) in acc.iter_mut().enumerate() {
                        *value += weight * to_f64(image.data[index + c]);
                    }
                }
            }
        }
    }
    let norm = 1.0 / (nx as f64 * ny as f64);
    std::array::from_fn(|c| acc[c] * norm)
}

/// Exact Poisson probability mass, used as an independent CDF oracle.
fn poisson_pmf(k: u64, lambda: f64) -> f64 {
    (-lambda + k as f64 * lambda.ln() - libm::lgamma(k as f64 + 1.0)).exp()
}

/// Discrete readout weight of one cell's density for one output pixel:
/// `sum_t k_t * w_j(n + t - radius)`.
fn readout_weight(taps: &[f64], radius: i64, stencil: &[(i64, f64)], n: i64) -> f64 {
    let mut acc = 0.0;
    for (t, &k) in taps.iter().enumerate() {
        let index = n + t as i64 - radius;
        for &(cell, weight) in stencil {
            if cell == index {
                acc += k * weight;
            }
        }
    }
    acc
}

#[test]
fn allocation_saturates_capacity_and_preserves_the_target() {
    let capacity = [0.198, 0.099, 0.033];
    let weights = [0.06, 0.03, 0.01];
    let interior = allocate(0.2, &capacity, &weights);
    assert!(
        (interior.iter().sum::<f64>() - 0.2).abs() < 1e-12,
        "{interior:?}"
    );
    for l in 0..3 {
        assert!(interior[l] < capacity[l], "layer {l} must stay unsaturated");
        let proportional = 0.2 * weights[l] / weights.iter().sum::<f64>();
        assert!((interior[l] - proportional).abs() < 1e-12, "{interior:?}");
    }
    // Full and over-full targets commit every layer at its capacity.
    assert_eq!(allocate(0.33, &capacity, &weights), capacity);
    assert_eq!(allocate(1.0, &capacity, &weights), capacity);
    assert_eq!(allocate(0.0, &capacity, &weights), [0.0; 3]);
    // Zero weights fall back to unsaturated capacity fractions per
    // iteration, not to one multiplier equation on the original weights.
    let zero = allocate(0.25, &[0.2, 0.1, 0.05], &[1.0, 0.0, 0.0]);
    assert!((zero[0] - 0.2).abs() < 1e-12, "{zero:?}");
    assert!((zero[1] - 0.1 / 3.0).abs() < 1e-12, "{zero:?}");
    assert!((zero[2] - 0.05 / 3.0).abs() < 1e-12, "{zero:?}");
    assert!((zero.iter().sum::<f64>() - 0.25).abs() < 1e-12);
    // A layer without capacity receives nothing.
    let skip = allocate(0.15, &[0.2, 0.0, 0.05], &[1.0, 1.0, 1.0]);
    assert_eq!(skip[1], 0.0);
    assert!((skip.iter().sum::<f64>() - 0.15).abs() < 1e-12);
}

#[test]
fn formation_taps_match_dense_integration_and_sum_to_one() {
    for cell in [SIGMA / 2.0, SIGMA / 8.0, 3.0 * SIGMA] {
        let taps = support_taps(SIGMA, cell);
        let radius = (SUPPORT_SIGMAS * SIGMA / cell).ceil() as i64 + 1;
        assert_eq!(taps.len() as i64, 2 * radius + 1);
        assert!(taps.iter().all(|&t| t >= 0.0));
        assert!((taps.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        for (index, &value) in taps.iter().enumerate() {
            let offset = index as i64 - radius;
            // Dense midpoint quadrature of
            // `(1/h) * integral_dest integral_source g(x - z) dz dx`,
            // exact in z through the CDF and independent of the
            // production double antiderivative.
            let steps = 4096usize;
            let sub = cell / steps as f64;
            let mut acc = 0.0;
            for n in 0..steps {
                let dest = offset as f64 * cell + (n as f64 + 0.5) * sub;
                acc += (trunc_cdf(dest, SIGMA) - trunc_cdf(dest - cell, SIGMA)) * sub;
            }
            let expected = acc / cell;
            assert!(
                (value - expected).abs() < 1e-6,
                "cell {cell} offset {offset}: {value} vs {expected}"
            );
        }
    }
}

#[test]
fn device_mass_law_rejects_host_only_experiments() {
    assert!(check_mass_law(MassLaw::Fixed).is_ok());
    for law in [MassLaw::TwoClass, MassLaw::Gamma4] {
        let error = check_mass_law(law).unwrap_err();
        assert!(
            error.contains("SPEKTRAFILM_GRAIN_V3_MASS_LAW")
                && error.contains("no device field path"),
            "{error}"
        );
    }
}

#[test]
fn count_law_matches_poisson_probabilities() {
    let samples = 50_000u64;
    for &lambda in &[0.35f64, 3.0, 29.5, 30.5, 200.0] {
        let counts: Vec<u64> = (0..samples)
            .map(|index| {
                let mut rng = FastStatsRng::stream(0x5EED_1234, 0, index);
                poisson_exact(&mut rng, lambda).unwrap()
            })
            .collect();
        let mean = counts.iter().sum::<u64>() as f64 / samples as f64;
        let variance = counts
            .iter()
            .map(|&k| (k as f64 - mean).powi(2))
            .sum::<f64>()
            / (samples as f64 - 1.0);
        let mean_sigma = (lambda / samples as f64).sqrt();
        assert!(
            (mean - lambda).abs() <= 5.0 * mean_sigma.max(1e-9),
            "lambda {lambda}: mean {mean}"
        );
        // Exact variance of the unbiased sample variance for Poisson data:
        // mu4=lambda+3*lambda^2, not the Gaussian 3*lambda^2.
        let variance_sigma =
            (lambda / samples as f64 + 2.0 * lambda * lambda / (samples as f64 - 1.0)).sqrt();
        assert!(
            (variance - lambda).abs() <= 5.0 * variance_sigma.max(1e-9),
            "lambda {lambda}: variance {variance}"
        );
        // CDF checks at zero, small, and tail-adjacent counts.
        let probes: [u64; 5] = [
            0,
            1,
            2,
            (lambda - 2.0 * lambda.sqrt()).max(3.0) as u64,
            lambda as u64,
        ];
        for &k in &probes {
            let exact: f64 = (0..=k).map(|j| poisson_pmf(j, lambda)).sum();
            if exact <= 0.0 || exact >= 1.0 {
                continue;
            }
            let empirical = counts.iter().filter(|&&c| c <= k).count() as f64 / samples as f64;
            let sigma = (exact * (1.0 - exact) / samples as f64).sqrt();
            assert!(
                (empirical - exact).abs() <= 5.0 * sigma + 1e-9,
                "lambda {lambda}, k {k}: {empirical} vs {exact}"
            );
        }
    }
}

#[test]
fn expectation_mode_reads_back_uniform_and_zero_targets() {
    let tol = precision_tol();
    let target = flat(48, 32, 0.12);
    let out = render(
        &params(None, true, 3, 1.0),
        &target,
        region(48, 32),
        (48, 32),
    );
    for py in 0..32usize {
        for px in 0..48usize {
            let value = pixel(&out, px, py);
            for c in 0..3 {
                assert!((value[c] - 0.12).abs() < tol, "{px},{py} c{c}: {value:?}");
            }
        }
    }
    // Fractional footprints and the film boundary preserve constants.
    let frac = render(
        &params(None, true, 3, 1.0),
        &flat(20, 20, 0.12),
        region(20, 20),
        (13, 7),
    );
    for py in 0..7usize {
        for px in 0..13usize {
            let value = pixel(&frac, px, py);
            for c in 0..3 {
                assert!(
                    (value[c] - 0.12).abs() < tol,
                    "fractional {px},{py}: {value:?}"
                );
            }
        }
    }
    // A zero developed target reads back as zero.
    let zero = render(
        &params(None, true, 3, 1.0),
        &flat(16, 16, 0.0),
        region(16, 16),
        (16, 16),
    );
    for py in 0..16usize {
        for px in 0..16usize {
            let value = pixel(&zero, px, py);
            for c in 0..3 {
                assert!(value[c].abs() < tol, "zero target {px},{py}: {value:?}");
            }
        }
    }
    // At the bottom of the supported set every count is exactly zero, so
    // realizations and the expectation coincide bit for bit.
    let floor = -MIN_DENSITY;
    let expected = render(
        &params(None, true, 1, 1.0),
        &flat(8, 8, floor),
        region(8, 8),
        (8, 8),
    );
    let realized = render(
        &params(None, false, 1, 1.0),
        &flat(8, 8, floor),
        region(8, 8),
        (8, 8),
    );
    for py in 0..8usize {
        for px in 0..8usize {
            let value = pixel(&expected, px, py);
            let actual = pixel(&realized, px, py);
            for c in 0..3 {
                assert!((value[c] - floor).abs() < tol, "{px},{py}: {value:?}");
                assert_eq!(actual[c], value[c], "zero-count floor {px},{py}");
            }
        }
    }
}

#[test]
fn seed_ensemble_mean_matches_the_expectation_render() {
    let target = flat(32, 32, 0.12);
    let rect = region(32, 32);
    let out = (32u32, 32u32);
    let seeds = 32u64;
    let mut sum = vec![[0.0f64; 3]; (out.0 * out.1) as usize];
    let mut sum2 = vec![[0.0f64; 3]; sum.len()];
    for seed in 0..seeds {
        let image = render(&params(None, false, 0x1000 + seed, 1.0), &target, rect, out);
        for (index, value) in image.pixels().enumerate() {
            for c in 0..3 {
                let v = to_f64(value[c]);
                sum[index][c] += v;
                sum2[index][c] += v * v;
            }
        }
    }
    let expectation = render(&params(None, true, 7, 1.0), &target, rect, out);
    let n = seeds as f64;
    let mut worst = 0.0f64;
    for (index, value) in expectation.pixels().enumerate() {
        for c in 0..3 {
            let mean = sum[index][c] / n;
            let variance = (sum2[index][c] - sum[index][c] * sum[index][c] / n) / (n - 1.0);
            let sigma = (variance / n).sqrt();
            let delta = (mean - to_f64(value[c])).abs();
            worst = worst.max(delta / sigma.max(1e-12));
        }
    }
    assert!(worst <= 5.0, "worst seed-mean deviation {worst} sigma");
}

#[test]
fn readout_variance_scales_with_event_area() {
    let target = flat(32, 32, 0.12);
    let rect = region(32, 32);
    let out = (32u32, 32u32);
    // Exact statement: the per-cell variance scales with event area and
    // the support is untouched.
    let measure_area = |area: f64| -> (f64, f64) {
        let model = Model::new(&params(Some(SIGMA / 4.0), true, 1, area), PITCH).expect("model");
        let layers = model.layer_targets([0.12; 3]);
        let mut variance = 0.0;
        for (c, targets) in layers.iter().enumerate() {
            let ch = &model.channels[c];
            for (l, &t) in targets.iter().enumerate() {
                if t <= 0.0 || ch.cap[l] <= 0.0 {
                    continue;
                }
                let p = (t / ch.cap[l]).clamp(0.0, 1.0);
                let s = 1.0 - ch.uniformity * (1.0 - 1e-6) * p;
                let lambda = model.cell_um * model.cell_um * ch.frac[l] * p / (ch.area[l] * s);
                let mass = ch.cap[l] * ch.area[l] * s / ch.frac[l];
                variance += lambda * (mass / (model.cell_um * model.cell_um)).powi(2);
            }
        }
        (variance, model.taps.iter().sum())
    };
    let (v1, support1) = measure_area(0.5);
    let (v2, support2) = measure_area(2.0);
    assert!((support1 - 1.0).abs() < 1e-12 && (support2 - 1.0).abs() < 1e-12);
    assert!(
        (v2 / v1 - 4.0).abs() < 1e-9,
        "analytic variance ratio {}",
        v2 / v1
    );
    // Rendered evidence: the same scaling survives the realized path.
    let seeds = 24u64;
    let measure_render = |area: f64| -> (f64, f64) {
        let mut sum = vec![0.0f64; (out.0 * out.1) as usize];
        let mut sum2 = vec![0.0f64; sum.len()];
        for seed in 0..seeds {
            let image = render(
                &params(None, false, 0x2000 + seed, area),
                &target,
                rect,
                out,
            );
            for (index, value) in image.pixels().enumerate() {
                let v = to_f64(value[1]);
                sum[index] += v;
                sum2[index] += v * v;
            }
        }
        let n = seeds as f64;
        let mut mean = 0.0;
        let mut variance = 0.0;
        for index in 0..sum.len() {
            mean += sum[index] / n;
            variance += (sum2[index] - sum[index] * sum[index] / n) / (n - 1.0);
        }
        (mean / sum.len() as f64, variance / sum.len() as f64)
    };
    let (mean1, var1) = measure_render(0.5);
    let (mean2, var2) = measure_render(2.0);
    assert!(
        (mean1 - mean2).abs() < 2e-3,
        "event area must not move the mean: {mean1} vs {mean2}"
    );
    let ratio = var2 / var1;
    assert!(
        (ratio - 4.0).abs() <= 1.0,
        "rendered variance ratio {ratio} (mean {mean1} vs {mean2})"
    );
}

#[test]
fn crop_and_band_reads_agree_exactly() {
    let target = patterned(96, 200);
    let p = params(None, false, 0x99, 1.0);
    // Two output bands at the default band height.
    let full = render(&p, &target, region(96, 200), (96, 200));
    let crop = render(
        &p,
        &target,
        SourceRect {
            x: 8,
            y: 120,
            width: 32,
            height: 60,
        },
        (32, 60),
    );
    for py in 0..60usize {
        for px in 0..32usize {
            let a = pixel(&full, 8 + px, 120 + py);
            let b = pixel(&crop, px, py);
            for c in 0..3 {
                assert_eq!(a[c], b[c], "crop pixel {px},{py} channel {c}");
            }
        }
    }
    // Identical footprints at an upscaled output agree too.
    let full2 = render(&p, &target, region(96, 96), (192, 192));
    let crop2 = render(
        &p,
        &target,
        SourceRect {
            x: 8,
            y: 40,
            width: 32,
            height: 24,
        },
        (64, 48),
    );
    for py in 0..48usize {
        for px in 0..64usize {
            let a = pixel(&full2, 16 + px, 80 + py);
            let b = pixel(&crop2, px, py);
            for c in 0..3 {
                assert_eq!(a[c], b[c], "upscaled crop pixel {px},{py} channel {c}");
            }
        }
    }
}

#[test]
fn renders_are_repeatable_and_seed_sensitive() {
    let target = patterned(32, 32);
    let a = render(
        &params(None, false, 5, 1.0),
        &target,
        region(32, 32),
        (32, 32),
    );
    let b = render(
        &params(None, false, 5, 1.0),
        &target,
        region(32, 32),
        (32, 32),
    );
    assert_eq!(a.data, b.data);
    let c = render(
        &params(None, false, 6, 1.0),
        &target,
        region(32, 32),
        (32, 32),
    );
    assert_ne!(a.data, c.data);
}

#[test]
fn field_refinement_converges_to_the_independent_oracle() {
    let (width, height) = (48u32, 48u32);
    let target = patterned(width, height);
    let rect = region(width, height);
    let out = (width, height);
    let line = (width * 5 / 8) as usize;
    let probes = [
        (4usize, 24usize),
        (19usize, 24usize),
        (line, 24usize),
        (40usize, 24usize),
    ];
    // The oracle's own integration must converge.
    for &(px, py) in &probes {
        let coarse = oracle_readout(&target, rect, out, px, py, 1.0);
        let fine = oracle_readout(&target, rect, out, px, py, 0.5);
        for c in 0..3 {
            assert!(
                (coarse[c] - fine[c]).abs() < 1e-4,
                "oracle convergence {px},{py} c{c}: {coarse:?} vs {fine:?}"
            );
        }
    }
    let mut previous = f64::INFINITY;
    let mut production = 0.0f64;
    let mut certified = 0.0f64;
    // Production pitch `sigma/2`, then the certified pitches `sigma/4`
    // and `sigma/8`.
    for level in 0..3 {
        let cell = Some(SIGMA / (2.0 * (1 << level) as f64));
        let rendered = render(&params(cell, true, 0x77, 1.0), &target, rect, out);
        let mut worst = 0.0f64;
        for &(px, py) in &probes {
            let expected = oracle_readout(&target, rect, out, px, py, 0.5);
            let actual = pixel(&rendered, px, py);
            for c in 0..3 {
                worst = worst.max((actual[c] - expected[c]).abs());
            }
        }
        assert!(
            worst <= previous.max(1e-9),
            "pitch refinement regressed at level {level}: {worst} vs {previous}"
        );
        if level == 0 {
            production = worst;
        }
        certified = worst;
        previous = worst;
    }
    // Declared production-pitch deviation, and the certification gate.
    assert!(
        production <= 5e-3,
        "production expected-density error {production}"
    );
    assert!(
        certified <= 1e-3,
        "certified expected-density error {certified}"
    );
}

#[test]
fn discrete_covariance_tracks_the_continuum_oracle() {
    // Production pitch: measure how far the discrete operator is from the
    // continuum covariance of the declared model.
    let production = covariance_error(None);
    assert!(
        production.worst <= 0.10,
        "production-pitch covariance error {}, lag {}",
        production.worst,
        production.worst_lag
    );
    // The certified `sigma/8` gate.
    let certified = covariance_error(Some(SIGMA / 8.0));
    assert!(
        certified.worst <= 0.01,
        "certified covariance error {}, lag {}",
        certified.worst,
        certified.worst_lag
    );
    // A stable but wrongly scaled kernel must fail the comparison.
    let wrong = covariance_error_with_sigma(Some(SIGMA / 8.0), SIGMA * 1.25);
    assert!(
        wrong.worst > 0.01,
        "a wrong-width kernel must fail: {}",
        wrong.worst
    );
}

struct CovarianceError {
    worst: f64,
    worst_lag: usize,
}

fn covariance_error(cell: Option<f64>) -> CovarianceError {
    covariance_error_with_sigma(cell, SIGMA)
}

/// Error of the discrete readout covariance against the continuum
/// `sum_lc v_lc * integral H_j(z) H_k(z) dz`, with `H` built from the
/// oracle sigma. Every lag is measured against the oracle's zero-lag
/// variance, because tail covariances approach zero. Production taps and
/// readout weights supply the discrete side; the oracle side is an
/// independent quadrature.
fn covariance_error_with_sigma(cell: Option<f64>, oracle_sigma: f64) -> CovarianceError {
    let values = covariance_values(cell, oracle_sigma);
    let variance = values
        .iter()
        .find(|(lag, _, _)| *lag == 0)
        .expect("zero lag")
        .2;
    let mut worst = 0.0f64;
    let mut worst_lag = 0usize;
    for (lag, discrete, oracle) in values {
        let error = ((discrete - oracle) / variance).abs();
        if error > worst {
            worst = error;
            worst_lag = lag;
        }
    }
    CovarianceError { worst, worst_lag }
}

/// `(lag, discrete, oracle)` readout covariances of the declared model.
fn covariance_values(cell: Option<f64>, oracle_sigma: f64) -> Vec<(usize, f64, f64)> {
    let dense = 0.12;
    let p = params(cell, true, 0x31, 1.0);
    let model = Model::new(&p, PITCH).expect("model");
    let area = model.cell_um * model.cell_um;
    let layers = model.layer_targets([dense; 3]);
    let mut variance = [[0.0f64; 3]; 3];
    let mut continuum = [[0.0f64; 3]; 3];
    for c in 0..3 {
        let ch = &model.channels[c];
        for l in 0..3 {
            let t = layers[c][l];
            if t <= 0.0 || ch.cap[l] <= 0.0 {
                continue;
            }
            let pl = (t / ch.cap[l]).clamp(0.0, 1.0);
            let s = 1.0 - ch.uniformity * (1.0 - 1e-6) * pl;
            let lambda = area * ch.frac[l] * pl / (ch.area[l] * s);
            let mass = ch.cap[l] * ch.area[l] * s / ch.frac[l];
            variance[c][l] = lambda * (mass / area).powi(2);
            continuum[c][l] = variance[c][l] * area;
        }
    }
    let d = model.radius;
    let footprint = PITCH;
    let x_read: Vec<Vec<(i64, f64)>> = (0..6)
        .map(|j| {
            readout_stencil(
                30.0 + j as f64 * footprint,
                30.0 + (j + 1) as f64 * footprint,
                model.cell_um,
            )
        })
        .collect();
    let y_read = readout_stencil(30.0, 30.0 + footprint, model.cell_um);
    let (y_lo, y_hi) = (
        y_read.first().expect("stencil").0 - d - 1,
        y_read.last().expect("stencil").0 + d + 1,
    );
    let mut sy = 0.0;
    for n in y_lo..=y_hi {
        let w = readout_weight(&model.taps, d, &y_read, n);
        sy += w * w;
    }
    let oy = integrate(
        &|z: f64| {
            let h = (trunc_cdf(30.0 + footprint - z, oracle_sigma)
                - trunc_cdf(30.0 - z, oracle_sigma))
                / footprint;
            h * h
        },
        30.0 - 4.0 * oracle_sigma - footprint,
        30.0 + footprint + 4.0 * oracle_sigma + footprint,
    );
    let mut out = Vec::new();
    for lag in [0usize, 1, 2, 4] {
        let (n_lo, n_hi) = (
            x_read[0].first().expect("stencil").0 - d - 2,
            x_read[lag].last().expect("stencil").0 + d + 2,
        );
        let mut sx = 0.0;
        for n in n_lo..=n_hi {
            let a = readout_weight(&model.taps, d, &x_read[0], n);
            let b = readout_weight(&model.taps, d, &x_read[lag], n);
            sx += a * b;
        }
        let ox = integrate(
            &|z: f64| {
                let left = |x0: f64| {
                    (trunc_cdf(x0 + footprint - z, oracle_sigma) - trunc_cdf(x0 - z, oracle_sigma))
                        / footprint
                };
                left(30.0) * left(30.0 + lag as f64 * footprint)
            },
            30.0 - 4.0 * oracle_sigma - footprint,
            30.0 + (lag + 1) as f64 * footprint + 4.0 * oracle_sigma + footprint,
        );
        let mut discrete = 0.0;
        let mut oracle = 0.0;
        for c in 0..3 {
            for l in 0..3 {
                discrete += variance[c][l] * sx * sy;
                oracle += continuum[c][l] * ox * oy;
            }
        }
        out.push((lag, discrete, oracle));
    }
    out
}

/// Fine trapezoid integral on a fixed fine step.
fn integrate(f: &dyn Fn(f64) -> f64, lo: f64, hi: f64) -> f64 {
    let steps = ((hi - lo) / (SIGMA / 400.0)).ceil() as usize;
    let step = (hi - lo) / steps as f64;
    let mut acc = 0.0;
    for i in 0..steps {
        let a = f(lo + i as f64 * step);
        let b = f(lo + (i + 1) as f64 * step);
        acc += 0.5 * (a + b) * step;
    }
    acc
}

#[test]
fn stochastic_corner_readout_matches_independent_cell_variance() {
    let mut model = Model::new(&params(None, false, 0, 1.0), PITCH).unwrap();
    let layers = model.layer_targets([0.12; 3]);
    let h = model.cell_um;
    let stencil = readout_stencil(0.0, PITCH, h);
    let lo = stencil[0].0 - model.radius;
    let hi = stencil.last().unwrap().0 + model.radius;
    let weights: Vec<f64> = (lo..=hi)
        .map(|n| readout_weight(&model.taps, model.radius, &stencil, n))
        .collect();
    let ch = &model.channels[0];
    let mut cell_variance = 0.0;
    for (l, &target) in layers[0].iter().enumerate() {
        let occupancy = target / ch.cap[l];
        let s = 1.0 - ch.uniformity * (1.0 - 1e-6) * occupancy;
        let mass = ch.cap[l] * ch.area[l] * s / ch.frac[l];
        let rate = h * h * ch.frac[l] * occupancy / (ch.area[l] * s);
        cell_variance += rate * (mass / (h * h)).powi(2);
    }
    let expected_variance = cell_variance * weights.iter().map(|w| w * w).sum::<f64>().powi(2);
    let mut sum = 0.0;
    let mut sum2 = 0.0;
    let samples = 4096;
    for seed in 0..samples {
        model.seed = seed;
        let mut value = 0.0;
        for (y, &wy) in weights.iter().enumerate() {
            for (x, &wx) in weights.iter().enumerate() {
                value += wx
                    * wy
                    * model
                        .accumulate(lo + x as i64, lo + y as i64, layers, h * h)
                        .unwrap()[0];
            }
        }
        sum += value;
        sum2 += value * value;
    }
    let mean = sum / samples as f64;
    let variance = (sum2 - sum * mean) / (samples - 1) as f64;
    assert!((mean - 0.15).abs() < 5.0 * (expected_variance / samples as f64).sqrt());
    assert!(
        (variance / expected_variance - 1.0).abs() < 0.1,
        "corner variance {variance}, independent-field variance {expected_variance}"
    );
}

/// FNV-1a digest of the realized cell densities of a fixed 3x3 grid at a
/// fixed configuration.
///
/// `accumulate` returns plain f64 and takes plain f64, so this digest is
/// the same in both `precision-f64` configurations: it pins the count
/// realization, not the pixel pipeline.
fn cell_density_digest(correlation: f64) -> u64 {
    let mut p = params(None, false, 0x5EED, 1.0);
    p.channel_correlation = correlation;
    let model = Model::new(&p, PITCH).unwrap();
    let layers = model.layer_targets([0.12; 3]);
    let area = model.cell_um.powi(2);
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for j in -1..=1 {
        for i in -1..=1 {
            for value in model.accumulate(i, j, layers, area).unwrap() {
                for byte in value.to_bits().to_le_bytes() {
                    hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
                }
            }
        }
    }
    hash
}

#[test]
fn zero_correlation_reproduces_the_frozen_cell_densities() {
    // Recorded from the pre-split base revision (`b2901609`): the same
    // probe there computes `0x51df_8f68_423e_c90b`, so zero correlation
    // must leave the independent per-`(channel, layer)` draws exactly as
    // they were, including their accumulation order.
    //
    // The digest is a constant of the default mass law, so an experiment
    // that selects another one is skipped by name instead of failing with a
    // digest mismatch that says nothing about the split.
    if std::env::var("SPEKTRAFILM_GRAIN_V3_MASS_LAW").is_ok() {
        eprintln!(
            "skipped: the frozen digest is recorded for the default mass law, and \
             SPEKTRAFILM_GRAIN_V3_MASS_LAW selects another one"
        );
        return;
    }
    assert_eq!(cell_density_digest(0.0), 0x51df_8f68_423e_c90b);
    // And the correlation is not a no-op: it must move the realization.
    assert_ne!(cell_density_digest(0.5), cell_density_digest(0.0));
    assert_ne!(cell_density_digest(0.9), cell_density_digest(0.5));
}

/// Per-channel means, standard deviations and cross-channel correlations
/// of the realized density of one cell, over a seed ensemble.
#[allow(clippy::type_complexity)]
fn cell_channel_statistics(correlation: f64, samples: u64) -> ([f64; 3], [f64; 3], [[f64; 3]; 3]) {
    let mut first = [0.0f64; 3];
    let mut second = [0.0f64; 3];
    let mut pairs = [[0.0f64; 3]; 3];
    for seed in 0..samples {
        let mut p = params(None, false, seed, 1.0);
        p.channel_correlation = correlation;
        let model = Model::new(&p, PITCH).unwrap();
        let layers = model.layer_targets([0.12; 3]);
        let value = model
            .accumulate(1, 1, layers, model.cell_um.powi(2))
            .unwrap();
        for a in 0..3 {
            first[a] += value[a];
            second[a] += value[a] * value[a];
            for (b, item) in pairs[a].iter_mut().enumerate() {
                *item += value[a] * value[b];
            }
        }
    }
    let n = samples as f64;
    let mean = std::array::from_fn(|c| first[c] / n);
    let deviation =
        std::array::from_fn(|c| (second[c] / n - (first[c] / n).powi(2)).max(0.0).sqrt());
    let correlation_matrix = std::array::from_fn(|a| {
        std::array::from_fn(|b| {
            let covariance = pairs[a][b] / n - mean[a] * mean[b];
            covariance / (deviation[a] * deviation[b])
        })
    });
    (mean, deviation, correlation_matrix)
}

#[test]
fn correlation_raises_cross_channel_coupling_without_moving_marginals() {
    let samples = 8192;
    let (mean0, deviation0, corr0) = cell_channel_statistics(0.0, samples);
    let (mean5, deviation5, corr5) = cell_channel_statistics(0.5, samples);
    let (mean9, deviation9, _) = cell_channel_statistics(0.9, samples);
    for (a, row) in corr0.iter().enumerate() {
        for (b, value) in row.iter().enumerate() {
            if a != b {
                assert!(
                    value.abs() < 0.05,
                    "channels {a},{b} correlate without the control: {}",
                    corr0[a][b]
                );
            }
        }
    }
    // The shared rate is `corr * min_c(lambda_c)`, so the realized
    // correlation is at most the control value and reaches it when the
    // channels of a layer have equal rates.
    for (a, row) in corr0.iter().enumerate() {
        for (b, control) in row.iter().enumerate() {
            if a != b {
                let low = control.abs().max(0.05);
                assert!(
                    corr5[a][b] > 0.2 && corr5[a][b] <= 0.75,
                    "corr 0.5 gave {} for {a},{b}",
                    corr5[a][b]
                );
                assert!(corr5[a][b] > low, "corr 0.5 did not exceed the floor");
            }
        }
    }
    // The marginals are the same law at every correlation: the split only
    // regroups the same total rate.
    for c in 0..3 {
        let band = 4.0 * deviation0[c] / (samples as f64).sqrt();
        assert!(
            (mean0[c] - mean5[c]).abs() < band && (mean0[c] - mean9[c]).abs() < band,
            "channel {c} mean moved with the correlation: {} {} {}",
            mean0[c],
            mean5[c],
            mean9[c]
        );
        let relative = (deviation5[c] / deviation0[c] - 1.0).abs();
        assert!(
            relative < 0.05,
            "channel {c} sigma moved with the correlation: {} vs {}",
            deviation0[c],
            deviation5[c]
        );
        let relative = (deviation9[c] / deviation0[c] - 1.0).abs();
        assert!(relative < 0.05, "channel {c} sigma moved at 0.9");
    }
}

#[test]
fn inactive_channel_leaves_the_layer_correlation_intact() {
    // `min` must run over the layer's active channels: a layer whose third
    // channel realizes nothing has no rate to correlate with, and counting
    // it as zero would silently drop the whole layer's correlation.
    let mut p = params(None, false, 0x77, 1.0);
    p.channel_correlation = 0.9;
    let model = Model::new(&p, PITCH).unwrap();
    let mut layers = model.layer_targets([0.12; 3]);
    layers[2] = [0.0; 3];
    let area = model.cell_um.powi(2);
    let mut first = [0.0f64; 2];
    let mut second = [0.0f64; 2];
    let mut pair = 0.0f64;
    let samples = 8192u64;
    for seed in 0..samples {
        let mut p = params(None, false, seed, 1.0);
        p.channel_correlation = 0.9;
        let model = Model::new(&p, PITCH).unwrap();
        let value = model.accumulate(1, 1, layers, area).unwrap();
        assert_eq!(value[2], 0.0, "the silenced channel still realized mass");
        first[0] += value[0];
        first[1] += value[1];
        second[0] += value[0] * value[0];
        second[1] += value[1] * value[1];
        pair += value[0] * value[1];
    }
    let n = samples as f64;
    let covariance = pair / n - (first[0] / n) * (first[1] / n);
    let sigma_a = (second[0] / n - (first[0] / n).powi(2)).sqrt();
    let sigma_b = (second[1] / n - (first[1] / n).powi(2)).sqrt();
    let correlation = covariance / (sigma_a * sigma_b);
    assert!(
        correlation > 0.4,
        "correlation collapsed to {correlation} with one channel inactive"
    );
}

#[test]
fn a_layer_with_one_active_channel_takes_no_shared_rate() {
    // A layer needs two active channels to have a rate worth sharing. With
    // one, the layer must fall back to its own independent draw, so the
    // correlated model has to realize exactly what the uncorrelated model
    // realized for that channel -- not merely a matching mean.
    let area = Model::new(&params(None, false, 0, 1.0), PITCH)
        .unwrap()
        .cell_um
        .powi(2);
    let mut realized = 0usize;
    for seed in 0..64u64 {
        let build = |correlation: f64| {
            let mut p = params(None, false, seed, 1.0);
            p.channel_correlation = correlation;
            let model = Model::new(&p, PITCH).unwrap();
            let mut layers = model.layer_targets([0.12; 3]);
            // Rearrange so that exactly one layer realizes channel 2 and no
            // layer realizes anything else: the computed targets need not be
            // nonzero in a given layer, so the shapes are set directly from
            // the channel capacities instead of being read back.
            let spare = model.channels[2].cap[1] * 0.25;
            for channel in layers.iter_mut() {
                *channel = [0.0; 3];
            }
            layers[0][0] = model.channels[0].cap[0] * 0.25;
            layers[1][2] = model.channels[1].cap[2] * 0.25;
            layers[2][1] = spare.max(model.channels[2].cap[1] * 0.25);
            (model, layers)
        };
        let (independent, independent_layers) = build(0.0);
        let (correlated, correlated_layers) = build(0.9);
        let plain = independent
            .accumulate(3, 4, independent_layers, area)
            .unwrap();
        let shared = correlated
            .accumulate(3, 4, correlated_layers, area)
            .unwrap();
        assert_eq!(
            plain[2], shared[2],
            "a layer with one active channel drew a shared rate"
        );
        if plain[2] > 0.0 {
            realized += 1;
        }
    }
    // A quarter of the channel capacity at one cell is a small rate, so the
    // point of the loop is that some seed realizes mass at all; the equality
    // above is what the test is about.
    assert!(realized > 0, "the single active layer never realized mass");
}

#[test]
fn expectation_is_independent_of_the_correlation() {
    let layers = [0.12; 3];
    let mut expected = None;
    for correlation in [0.0, 0.5, 0.9] {
        let mut p = params(None, true, 0x5EED, 1.0);
        p.channel_correlation = correlation;
        let model = Model::new(&p, PITCH).unwrap();
        let value = model
            .accumulate(1, 1, model.layer_targets(layers), model.cell_um.powi(2))
            .unwrap();
        match expected {
            None => expected = Some(value),
            Some(reference) => assert_eq!(
                reference, value,
                "correlation {correlation} moved the deterministic expectation"
            ),
        }
    }
}

#[test]
fn channel_correlation_out_of_range_fails_before_rendering() {
    for value in [-0.01, 1.0, 1.5, f64::NAN, f64::INFINITY] {
        let mut p = params(None, false, 1, 1.0);
        p.channel_correlation = value;
        let err = match Model::new(&p, PITCH) {
            Err(err) => err,
            Ok(_) => panic!("channel correlation {value} was accepted"),
        };
        assert!(err.contains("channel correlation"), "{value}: {err}");
    }
    for value in [0.0, 0.5, 0.999_999] {
        let mut p = params(None, false, 1, 1.0);
        p.channel_correlation = value;
        Model::new(&p, PITCH).unwrap();
    }
}

#[test]
fn monochrome_expectation_forms_edges_instead_of_preserving_sharp_detail() {
    let target = patterned(16, 12);
    let rect = region(16, 12);
    let mut p = params(None, true, 0xED6E, 1.28);
    p.monochrome = true;
    let actual = render(&p, &target, rect, (16, 12));
    for (x, y) in [(4, 6), (5, 6), (10, 6), (15, 6)] {
        let reference = oracle_readout(&target, rect, (16, 12), x, y, 0.5);
        let value = pixel(&actual, x, y);
        assert_eq!(value, [value[0]; 3]);
        assert!(
            (value[0] - reference[0]).abs() < 5e-3,
            "expected formed density at {x},{y}: {value:?}, oracle {reference:?}"
        );
    }
    assert!((pixel(&actual, 5, 6)[0] - pixel(&target, 5, 6)[0]).abs() > 1e-3);
}

#[test]
fn monochrome_matches_color_record_zero_and_replicates_before_readout() {
    let target = patterned(8, 7);
    let grid = ReadoutGrid {
        origin_um: [-1.25, 2.125],
        pitch_um: [2.75, 11.25],
        width: 13,
        height: 4,
    };
    for positive_film in [false, true] {
        for expectation in [false, true] {
            let mut color = params(None, expectation, 0xBADC0DE, 1.28);
            color.positive_film = positive_film;
            if positive_film {
                color.composite_curves.reverse();
                color.layer_curves.reverse();
            }
            let mut bw = color.clone();
            bw.monochrome = true;
            // BW must use the private record-0 component streams, not the
            // color shared streams even when correlation was requested.
            bw.channel_correlation = 0.45;
            let color_model = Model::new(&color, PITCH).unwrap();
            let bw_model = Model::new(&bw, PITCH).unwrap();
            let area = bw_model.cell_um.powi(2);
            for density in [-MIN_DENSITY, 0.0, 0.12, 0.3] {
                let layers = bw_model.layer_targets([density; 3]);
                assert_eq!(layers, color_model.layer_targets([density; 3]));
                for (i, j) in [(-7, -3), (-1, 0), (0, 0), (5, 11)] {
                    let reference = color_model.accumulate(i, j, layers, area).unwrap()[0];
                    let actual = bw_model.accumulate(i, j, layers, area).unwrap();
                    assert_eq!(actual, [reference; 3]);
                }
            }
            let reference = apply(&target, PITCH, grid, &color).unwrap();
            let actual = apply(&target, PITCH, grid, &bw).unwrap();
            for (expected, value) in reference.pixels().zip(actual.pixels()) {
                assert_eq!(value, &[expected[0]; 3]);
            }
            // The response sees replicated material samples before any
            // nonlinear optical mapping, not a final RGB desaturation.
            let response = |density: &ImageBuf| {
                for px in density.pixels() {
                    assert_eq!(px, &[px[0]; 3]);
                }
                Ok(density.clone())
            };
            let through_response =
                apply_with_response(&target, PITCH, grid, &bw, response).unwrap();
            for (a, b) in actual.data.iter().zip(&through_response.data) {
                assert!((to_f64(*a) - to_f64(*b)).abs() < precision_tol());
            }
        }
    }
}

#[test]
fn monochrome_correlation_is_inactive_without_bypassing_validation() {
    let target = patterned(9, 7);
    for expectation in [false, true] {
        let mut p = params(None, expectation, 0xC011, 1.28);
        p.monochrome = true;
        let reference = render(&p, &target, region(9, 7), (13, 5));
        for correlation in [0.2, 0.45] {
            p.channel_correlation = correlation;
            let actual = render(&p, &target, region(9, 7), (13, 5));
            assert_eq!(actual.data, reference.data);
        }
        for correlation in [f64::NAN, f64::INFINITY, -0.1, 1.0] {
            p.channel_correlation = correlation;
            let error = Model::new(&p, PITCH).err().unwrap();
            assert!(error.contains("correlation"), "{error}");
        }
    }
}

#[test]
fn monochrome_cell_moments_preserve_the_single_record_law_and_statistical_floor() {
    let mut p = params(None, false, 0, 1.28);
    p.monochrome = true;
    p.channel_correlation = 0.45;
    let mut model = Model::new(&p, PITCH).unwrap();
    let area = model.cell_um.powi(2);
    let raw_total: f64 = MAXIMA.iter().sum();
    let samples = 16384u64;
    for density in [-MIN_DENSITY, 0.0, 0.12, 0.3] {
        let layers = model.layer_targets([density; 3]);
        let mut expected_variance = 0.0;
        for (l, &raw_max) in MAXIMA.iter().enumerate() {
            let fraction = raw_max / raw_total;
            let capacity = raw_max + fraction * MIN_DENSITY;
            let occupancy = layers[0][l] / capacity;
            let s = 1.0 - 0.5 * (1.0 - 1e-6) * occupancy;
            expected_variance += capacity * p.event_area_um2 * s * layers[0][l] / (fraction * area);
        }
        let expected_mean = density + MIN_DENSITY;
        model.expectation = true;
        let expected = model.accumulate(-3, 7, layers, area).unwrap();
        assert_eq!(expected, [expected[0]; 3]);
        assert!((expected[0] - expected_mean).abs() < 1e-14);
        model.expectation = false;
        let mut sum = 0.0;
        let mut sum2 = 0.0;
        for seed in 0..samples {
            model.seed = seed;
            let value = model.accumulate(-3, 7, layers, area).unwrap();
            assert_eq!(value, [value[0]; 3]);
            sum += value[0];
            sum2 += value[0].powi(2);
        }
        let n = samples as f64;
        let mean = sum / n;
        let variance = (sum2 - sum * mean) / (n - 1.0);
        if density == -MIN_DENSITY {
            assert_eq!(sum, 0.0);
            assert_eq!(sum2, 0.0);
            assert_eq!(expected_variance, 0.0);
        } else {
            assert!(expected_variance > 0.0);
            assert!(
                (mean - expected_mean).abs() < 5.0 * (expected_variance / n).sqrt(),
                "density {density}: mean {mean}, expected {expected_mean}"
            );
            assert!(
                (variance / expected_variance - 1.0).abs() < 0.08,
                "density {density}: variance {variance}, expected {expected_variance}"
            );
        }
    }
}

#[test]
fn monochrome_expectation_and_response_subtract_the_floor_once() {
    let mut p = params(None, true, 0xF100, 1.28);
    p.monochrome = true;
    // Exact in both pixel precisions, so the true zero-count floor is not
    // confused with a rounded developed density just above it.
    p.density_min = [0.03125; 3];
    let grid = ReadoutGrid {
        origin_um: [-3.75, 1.125],
        pitch_um: [7.25, 4.75],
        width: 5,
        height: 3,
    };
    for density in [-0.03125, 0.0, 0.12] {
        let target = flat(6, 5, density);
        let expected = apply(&target, PITCH, grid, &p).unwrap();
        for px in expected.pixels() {
            assert_eq!(px, &[px[0]; 3]);
            assert!((to_f64(px[0]) - density).abs() < precision_tol());
        }
        let optical = apply_with_response(&target, PITCH, grid, &p, |field| {
            let mut linear = field.clone();
            for px in linear.pixels_mut() {
                assert_eq!(px[1], px[0]);
                assert_eq!(px[2], px[0]);
                px.fill(from_f64(10.0f64.powf(-to_f64(px[0]) - 0.2)));
            }
            Ok(linear)
        })
        .unwrap();
        for px in optical.pixels() {
            assert_eq!(px, &[px[0]; 3]);
            let reference = 10.0f64.powf(-density - 0.2);
            assert!((to_f64(px[0]) - reference).abs() < precision_tol());
        }
        if density == -0.03125 {
            p.expectation = false;
            let realized = apply(&target, PITCH, grid, &p).unwrap();
            assert_eq!(realized.data, expected.data);
            p.expectation = true;
        }
    }
    // Zero developed density still samples its positive absolute statistical
    // floor; it is not a zero stochastic output or a zero-variance condition.
    p.expectation = false;
    let realized = apply(&flat(6, 5, 0.0), PITCH, grid, &p).unwrap();
    assert!(
        realized
            .data
            .iter()
            .any(|v| to_f64(*v).abs() > precision_tol())
    );
}

#[test]
fn monochrome_fractional_crops_and_bands_keep_the_global_field() {
    let target = patterned(9, 20);
    let grid = ReadoutGrid {
        origin_um: [-3.75, 1.125],
        pitch_um: [2.75, 0.75],
        width: 19,
        height: 137,
    };
    let crop_grid = ReadoutGrid {
        origin_um: [
            grid.origin_um[0] + 4.0 * 2.75,
            grid.origin_um[1] + 125.0 * 0.75,
        ],
        width: 9,
        height: 9,
        ..grid
    };
    for expectation in [false, true] {
        let mut p = params(None, expectation, 0xBADD, 1.28);
        p.monochrome = true;
        let full = apply(&target, PITCH, grid, &p).unwrap();
        let repeated = apply(&target, PITCH, grid, &p).unwrap();
        assert_eq!(full.data, repeated.data);
        let bounded = apply_with_budget(&target, PITCH, grid, &p, 2000).unwrap();
        assert_eq!(full.data, bounded.data);
        let cropped = apply(&target, PITCH, crop_grid, &p).unwrap();
        for y in 0..9 {
            for x in 0..9 {
                let value = pixel(&cropped, x, y);
                assert_eq!(value, pixel(&full, x + 4, y + 125));
                assert_eq!(value, [value[0]; 3]);
            }
        }
    }
}

#[test]
fn monochrome_rejects_unresolved_single_record_and_mislabeled_color_curves() {
    let mut canonical = params(None, true, 0xCA110, 1.28);
    canonical.monochrome = true;
    assert!(Model::new(&canonical, PITCH).is_ok());

    // The monochrome spectral basis may be [s, 0, 0], but developed
    // composite/component tables must already be resolved to three copies.
    let mut unresolved_composite = canonical.clone();
    for row in &mut unresolved_composite.composite_curves {
        *row = [row[0], 0.0, 0.0];
    }
    let error = Model::new(&unresolved_composite, PITCH).err().unwrap();
    assert!(error.contains("monochrome composite curve"), "{error}");
    let mut unresolved_components = canonical.clone();
    for row in &mut unresolved_components.layer_curves {
        for layer in row {
            *layer = [layer[0], 0.0, 0.0];
        }
    }
    let error = Model::new(&unresolved_components, PITCH).err().unwrap();
    assert!(error.contains("monochrome layer curve"), "{error}");

    // Valid, individually consistent color records cannot become a scalar
    // material merely by changing the request's profile-derived layout.
    let mut color = canonical.clone();
    color.monochrome = false;
    let scales = [1.0, 0.8, 1.1];
    for row in &mut color.layer_max {
        for (value, scale) in row.iter_mut().zip(scales) {
            *value *= scale;
        }
    }
    for row in &mut color.composite_curves {
        for (value, scale) in row.iter_mut().zip(scales) {
            *value *= scale;
        }
    }
    for row in &mut color.layer_curves {
        for layer in row {
            for (value, scale) in layer.iter_mut().zip(scales) {
                *value *= scale;
            }
        }
    }
    assert!(Model::new(&color, PITCH).is_ok());
    color.monochrome = true;
    let error = Model::new(&color, PITCH).err().unwrap();
    assert!(error.contains("monochrome layer maxima"), "{error}");
}

#[test]
fn monochrome_rejects_noncanonical_tables_and_nonfinite_or_inconsistent_density() {
    let mut good = params(None, true, 1, 1.28);
    good.monochrome = true;
    let cases: [fn(&mut GrainV3Params); 8] = [
        |p| p.channel_scale[1] += 0.1,
        |p| p.density_min[2] += 0.01,
        |p| p.uniformity[1] += 0.1,
        |p| p.layer_max[1][2] += 0.01,
        |p| p.composite_curves[3][1] += 0.01,
        |p| p.layer_curves[2][1][2] += 0.01,
        |p| p.density_min = [f64::NAN; 3],
        |p| p.layer_curves[2][1] = [f64::INFINITY; 3],
    ];
    for change in cases {
        let mut bad = good.clone();
        change(&mut bad);
        let error = Model::new(&bad, PITCH).err().unwrap();
        assert!(
            error.contains("monochrome") && error.contains("finite and identical"),
            "{error}"
        );
    }
    let reference = flat(4, 3, 0.12);
    let grid = grid(region(4, 3), (4, 3), PITCH);
    let expected = apply(&reference, PITCH, grid, &good).unwrap();
    let mut close = reference.clone();
    close.data[1] += from_f64(0.5e-6);
    let accepted = apply(&close, PITCH, grid, &good).unwrap();
    assert_eq!(accepted.data, expected.data, "record zero is authoritative");
    let mut inconsistent = reference.clone();
    inconsistent.data[1] += from_f64(2e-6);
    let error = apply(&inconsistent, PITCH, grid, &good).unwrap_err();
    assert!(error.contains("lanes must agree"), "{error}");
    for monochrome in [false, true] {
        let mut p = good.clone();
        p.monochrome = monochrome;
        for lane in 0..3 {
            for nonfinite in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                let mut invalid = reference.clone();
                invalid.data[lane] = from_f64(nonfinite);
                let error = apply(&invalid, PITCH, grid, &p).unwrap_err();
                assert!(error.contains("must be finite"), "{error}");
            }
        }
    }
}

#[test]
fn signed_halo_cells_have_independent_streams_and_counts() {
    let mut addresses = std::collections::HashSet::new();
    for j in -32..=32 {
        for i in -32..=32 {
            assert!(
                addresses.insert(cell_stream_index(i, j)),
                "aliased cell ({i},{j})"
            );
        }
    }
    let samples = 8192;
    let mut sums = [0.0; 5];
    for seed in 0..samples {
        let model = Model::new(&params(None, false, seed, 1.0), PITCH).unwrap();
        let layers = model.layer_targets([0.12; 3]);
        let a = model
            .accumulate(1, 1, layers, model.cell_um.powi(2))
            .unwrap()[0];
        let b = model
            .accumulate(-1, -1, layers, model.cell_um.powi(2))
            .unwrap()[0];
        sums[0] += a;
        sums[1] += b;
        sums[2] += a * a;
        sums[3] += b * b;
        sums[4] += a * b;
    }
    let n = samples as f64;
    let covariance = sums[4] / n - sums[0] * sums[1] / (n * n);
    let variance_a = sums[2] / n - (sums[0] / n).powi(2);
    let variance_b = sums[3] / n - (sums[1] / n).powi(2);
    let correlation = covariance / (variance_a * variance_b).sqrt();
    assert!(
        correlation.abs() < 0.05,
        "mirrored halo count correlation {correlation}"
    );
}

#[test]
fn rounded_crop_keeps_global_readout_footprints() {
    let target = patterned(17, 13);
    let p = params(None, false, 0x1234, 1.0);
    let full_grid = ReadoutGrid {
        origin_um: [-0.7, -0.3],
        pitch_um: [17.0 * PITCH / 11.0, 13.0 * PITCH / 9.0],
        width: 11,
        height: 9,
    };
    let full = apply(&target, PITCH, full_grid, &p).unwrap();
    let crop_grid = ReadoutGrid {
        origin_um: [
            full_grid.origin_um[0] + 3.0 * full_grid.pitch_um[0],
            full_grid.origin_um[1] + 2.0 * full_grid.pitch_um[1],
        ],
        width: 5,
        height: 4,
        ..full_grid
    };
    let crop = apply(&target, PITCH, crop_grid, &p).unwrap();
    for y in 0..4 {
        for x in 0..5 {
            let a = pixel(&crop, x, y);
            let b = pixel(&full, x + 3, y + 2);
            for c in 0..3 {
                assert!((a[c] - b[c]).abs() < precision_tol());
            }
        }
    }
}

#[test]
fn shrinking_the_cell_budget_preserves_every_output_value() {
    let target = patterned(16, 40);
    let p = params(None, false, 0x99, 1.0);
    let full = apply_region(&target, PITCH, region(16, 40), 16, 40, &p).unwrap();
    let small = apply_with_budget(
        &target,
        PITCH,
        grid(region(16, 40), (16, 40), PITCH),
        &p,
        2000,
    )
    .unwrap();
    assert_eq!(full.data, small.data);
}

#[test]
fn cell_partition_budget_preserves_global_density_and_optical_response() {
    let target = patterned(9, 7);
    let grid = ReadoutGrid {
        origin_um: [-3.7, 2.1],
        pitch_um: [17.9, 15.3],
        width: 3,
        height: 2,
    };
    for expectation in [false, true] {
        let p = params(None, expectation, 0xA11CE, 1.28);
        let reference = apply_with_budget(&target, PITCH, grid, &p, MAX_BAND_CELLS).unwrap();
        // 17x17 halo plus one inner cell fits, while an output footprint
        // does not. This forces partitions inside individual footprints.
        let bounded = apply_with_budget(&target, PITCH, grid, &p, 500).unwrap();
        for (&a, &b) in reference.data.iter().zip(&bounded.data) {
            assert!((to_f64(a) - to_f64(b)).abs() < precision_tol());
        }
        let response = |density: &ImageBuf| {
            let mut linear = density.clone();
            for pixel in linear.data.as_chunks_mut::<3>().0 {
                let cmy = [to_f64(pixel[0]), to_f64(pixel[1]), to_f64(pixel[2])];
                for c in 0..3 {
                    pixel[c] = from_f64(10.0_f64.powf(-cmy[c] - 0.4 * cmy[(c + 1) % 3]));
                }
            }
            Ok(linear)
        };
        let reference =
            apply_impl(&target, PITCH, grid, &p, MAX_BAND_CELLS, Some(&response)).unwrap();
        let bounded = apply_impl(&target, PITCH, grid, &p, 500, Some(&response)).unwrap();
        for (&a, &b) in reference.data.iter().zip(&bounded.data) {
            assert!((to_f64(a) - to_f64(b)).abs() < precision_tol());
        }
        let error = apply_with_budget(&target, PITCH, grid, &p, 1).unwrap_err();
        assert!(error.contains("one formed cell"), "{error}");
    }
}

#[test]
fn extreme_parameters_fail_before_stencils_or_sampling() {
    let target = flat(1, 1, 0.12);
    let good = params(None, false, 1, 1.0);
    for area in [f64::MIN_POSITIVE, f64::MAX] {
        let mut p = good.clone();
        p.event_area_um2 = area;
        assert!(apply_region(&target, PITCH, region(1, 1), 1, 1, &p).is_err());
    }
    let mut p = good.clone();
    p.event_area_um2 = f64::MAX;
    p.channel_scale = [2.0; 3];
    assert!(
        apply_region(&target, PITCH, region(1, 1), 1, 1, &p)
            .unwrap_err()
            .contains("event_area_um2")
    );
    let mut p = good.clone();
    p.cell_um = Some(f64::MIN_POSITIVE);
    assert!(
        apply_region(&target, PITCH, region(1, 1), 1, 1, &p)
            .unwrap_err()
            .contains("support halo")
    );
    let mut p = good.clone();
    p.support_um = f64::MAX;
    assert!(
        apply_region(&target, PITCH, region(1, 1), 1, 1, &p)
            .unwrap_err()
            .contains("squared field pitch")
    );
    assert!(
        apply_region(&target, 1e12, region(1, 1), 1, 1, &good)
            .unwrap_err()
            .contains("cell address")
    );
    let mut p = good.clone();
    p.event_area_um2 = f64::MIN_POSITIVE;
    p.channel_scale = [f64::MIN_POSITIVE; 3];
    assert!(
        apply_region(&target, PITCH, region(1, 1), 1, 1, &p)
            .unwrap_err()
            .contains("event_area_um2")
    );
    assert!(
        apply_region(&target, PITCH, region(1, 1), u32::MAX, 1, &good)
            .unwrap_err()
            .contains("output-axis")
    );
}

#[test]
fn invalid_inputs_fail_before_rendering() {
    let good = params(None, true, 1, 1.0);
    // Developed target outside the capacity envelope.
    let err = apply_region(&flat(8, 8, 0.4), PITCH, region(8, 8), 8, 8, &good).unwrap_err();
    assert!(err.contains("outside the supported"), "{err}");
    let err = apply_region(&flat(8, 8, -0.05), PITCH, region(8, 8), 8, 8, &good).unwrap_err();
    assert!(err.contains("outside the supported"), "{err}");
    let mut bad_grid = grid(region(8, 8), (8, 8), PITCH);
    bad_grid.origin_um[0] = f64::NAN;
    let err = apply(&flat(8, 8, 0.1), PITCH, bad_grid, &good).unwrap_err();
    assert!(err.contains("readout origin"), "{err}");
    bad_grid.origin_um = [0.0; 2];
    bad_grid.width = 0;
    let err = apply(&flat(8, 8, 0.1), PITCH, bad_grid, &good).unwrap_err();
    assert!(err.contains("dimensions"), "{err}");
    // Support, pitch, and field pitch validation.
    let mut bad = good.clone();
    bad.support_um = 0.0;
    let err = apply_region(&flat(8, 8, 0.1), PITCH, region(8, 8), 8, 8, &bad).unwrap_err();
    assert!(err.contains("v3_dye_support_um"), "{err}");
    let err = apply_region(&flat(8, 8, 0.1), 0.0, region(8, 8), 8, 8, &good).unwrap_err();
    assert!(err.contains("pitch"), "{err}");
    let mut bad = good.clone();
    bad.cell_um = Some(SIGMA);
    let err = apply_region(&flat(8, 8, 0.1), PITCH, region(8, 8), 8, 8, &bad).unwrap_err();
    assert!(err.contains("field pitch"), "{err}");
    // Curve and control validation.
    let mut bad = good.clone();
    bad.composite_curves[3][1] = -1.0;
    let err = apply_region(&flat(8, 8, 0.1), PITCH, region(8, 8), 8, 8, &bad).unwrap_err();
    assert!(err.contains("nondecreasing"), "{err}");
    let mut bad = good.clone();
    bad.layer_curves[2][1][0] = -1.0;
    let err = apply_region(&flat(8, 8, 0.1), PITCH, region(8, 8), 8, 8, &bad).unwrap_err();
    assert!(err.contains("nonnegative"), "{err}");
    let mut bad = good.clone();
    bad.uniformity = [1.5, 0.0, 0.0];
    let err = apply_region(&flat(8, 8, 0.1), PITCH, region(8, 8), 8, 8, &bad).unwrap_err();
    assert!(err.contains("uniformity"), "{err}");
    let mut bad = good.clone();
    bad.density_min = [-0.1, 0.0, 0.0];
    let err = apply_region(&flat(8, 8, 0.1), PITCH, region(8, 8), 8, 8, &bad).unwrap_err();
    assert!(err.contains("density_min"), "{err}");
    let mut bad = good.clone();
    bad.event_area_um2 = 0.0;
    let err = apply_region(&flat(8, 8, 0.1), PITCH, region(8, 8), 8, 8, &bad).unwrap_err();
    assert!(err.contains("event_area_um2"), "{err}");
    let mut bad = good.clone();
    bad.layer_max = [[0.0; 3]; 3];
    let err = apply_region(&flat(8, 8, 0.1), PITCH, region(8, 8), 8, 8, &bad).unwrap_err();
    assert!(err.contains("capacity"), "{err}");
}

#[test]
fn zero_capacity_layer_is_skipped() {
    let mut p = params(None, false, 0x44, 1.0);
    p.layer_max = [[0.18, 0.18, 0.18], [0.09, 0.09, 0.09], [0.0, 0.0, 0.0]];
    for row in p.layer_curves.iter_mut() {
        row[2] = [0.0; 3];
    }
    let target = flat(24, 24, 0.12);
    let out = render(&p, &target, region(24, 24), (24, 24));
    let expectation = render(
        &GrainV3Params {
            expectation: true,
            ..p.clone()
        },
        &target,
        region(24, 24),
        (24, 24),
    );
    for py in 0..24usize {
        for px in 0..24usize {
            let value = pixel(&out, px, py);
            let mean = pixel(&expectation, px, py);
            for c in 0..3 {
                assert!((mean[c] - 0.12).abs() < 1e-6, "skipped layer mean {mean:?}");
                assert!(value[c].is_finite());
            }
        }
    }
}

/// One layer-curve row with the same values in every channel.
fn row(_composite: f64, layers: [f64; 3]) -> [[f64; 3]; 3] {
    layers.map(|layer| [layer; 3])
}

/// Independently reproduce one channel's interpolated allocation weights.
fn weight_proportions(
    knots: &[f64],
    rows: &[[[f64; 3]; 3]],
    x: f64,
    floor_share: [f64; 3],
) -> [f64; 3] {
    weight_proportions_channel(knots, rows, x, floor_share, 0)
}

fn weight_proportions_channel(
    knots: &[f64],
    rows: &[[[f64; 3]; 3]],
    x: f64,
    floor_share: [f64; 3],
    channel: usize,
) -> [f64; 3] {
    let k = knots.len();
    let (low, t) = if x <= knots[0] {
        (0, 0.0)
    } else if x >= knots[k - 1] {
        (k - 2, 1.0)
    } else {
        let idx = knots.partition_point(|&v| v <= x);
        let low = if idx > 0 { idx - 1 } else { 0 };
        (low, (x - knots[low]) / (knots[low + 1] - knots[low]))
    };
    std::array::from_fn(|layer| {
        let value = rows[low][layer][channel]
            + t * (rows[low + 1][layer][channel] - rows[low][layer][channel]);
        (value + floor_share[layer]).max(0.0)
    })
}

/// Every channel allocation must conserve the clamped target `D + min`.
/// `Model::layer_targets` is channel-major: `[channel][layer]`.
fn assert_conserved(target: [f64; 3], layers: &[[f64; 3]; 3], model: &Model) {
    for c in 0..3 {
        let sum: f64 = layers[c].iter().sum();
        let expected = (target[c] + model.channels[c].min).clamp(0.0, model.channels[c].cap_total);
        assert!(
            (sum - expected).abs() < 1e-12,
            "target {target:?}, channel {c} allocated {sum} of {expected}"
        );
    }
}

/// Params with explicit composite knots, layer rows, and per-channel maxima.
fn allocation_params(
    knots: Vec<f64>,
    rows: Vec<[[f64; 3]; 3]>,
    maxima: [[f64; 3]; 3],
    positive_film: bool,
    density_min: [f64; 3],
) -> GrainV3Params {
    GrainV3Params {
        composite_curves: knots.iter().map(|&k| [k; 3]).collect(),
        layer_curves: rows,
        layer_max: maxima,
        positive_film,
        density_min,
        ..params(None, true, 0x51, 1.0)
    }
}

/// A model with linearly increasing layer weights on the negative-film axis.
fn linear_model(positive_film: bool) -> Model {
    let knots = if positive_film {
        vec![1.0, 0.5, 0.0]
    } else {
        vec![0.0, 0.5, 1.0]
    };
    let rows = vec![
        row(0.0, [0.0; 3]),
        row(0.5, [MAXIMA[0] / 2.0, MAXIMA[1] / 2.0, MAXIMA[2] / 2.0]),
        row(1.0, MAXIMA),
    ];
    Model::new(
        &allocation_params(
            knots,
            rows,
            std::array::from_fn(|layer| [MAXIMA[layer]; 3]),
            positive_film,
            [MIN_DENSITY; 3],
        ),
        PITCH,
    )
    .expect("model")
}

/// A model with duplicate composite knots and flat layer weights.
fn flat_model() -> Model {
    let knots = vec![0.0, 0.5, 0.5, 1.0];
    let rows = vec![
        row(0.0, [0.06, 0.03, 0.01]),
        row(0.5, [0.06, 0.03, 0.01]),
        row(0.5, [0.06, 0.03, 0.01]),
        row(1.0, [0.06, 0.03, 0.01]),
    ];
    Model::new(
        &allocation_params(
            knots,
            rows,
            std::array::from_fn(|layer| [MAXIMA[layer]; 3]),
            false,
            [MIN_DENSITY; 3],
        ),
        PITCH,
    )
    .expect("model")
}

#[test]
fn interior_target_splits_by_the_interpolated_weights() {
    let model = linear_model(false);
    let knots = [0.0, 0.5, 1.0];
    let rows = [
        row(0.0, [0.0; 3]),
        row(0.5, [MAXIMA[0] / 2.0, MAXIMA[1] / 2.0, MAXIMA[2] / 2.0]),
        row(1.0, MAXIMA),
    ];
    let floor_share = [
        MAXIMA[0] / 0.3 * MIN_DENSITY,
        MAXIMA[1] / 0.3 * MIN_DENSITY,
        MAXIMA[2] / 0.3 * MIN_DENSITY,
    ];
    for &x in &[0.125, 0.25] {
        let target = [x; 3];
        let layers = model.layer_targets(target);
        assert_conserved(target, &layers, &model);
        let weights = weight_proportions(&knots, &rows, x, floor_share);
        let total = target[0] + MIN_DENSITY;
        for layer in 0..3 {
            let expected = total * weights[layer] / weights.iter().sum::<f64>();
            assert!(
                (layers[0][layer] - expected).abs() < 1e-12,
                "query {x}: layer {layer} got {}, expected {expected}, weights {weights:?}",
                layers[0][layer]
            );
        }
    }
}

#[test]
fn endpoint_queries_clamp_before_allocation() {
    let model = linear_model(false);
    let low = model.layer_targets([0.0; 3]);
    let high = model.layer_targets([1.0; 3]);
    assert!(low[0][0] < high[0][0], "endpoint clamp collapsed: {low:?}");
    assert!((low[0][2] - MAXIMA[2] / 0.3 * MIN_DENSITY).abs() < 1e-12);
    for channel in 0..3 {
        assert_eq!(model.layer_targets([-0.25; 3])[channel], [0.0; 3]);
        assert_eq!(
            model.layer_targets([1.75; 3])[channel],
            model.channels[channel].cap
        );
    }
}

#[test]
fn duplicate_composite_knots_select_the_right_hand_bracket() {
    let model = flat_model();
    let weights = [
        0.06 + MAXIMA[0] / 0.3 * MIN_DENSITY,
        0.03 + MAXIMA[1] / 0.3 * MIN_DENSITY,
        0.01 + MAXIMA[2] / 0.3 * MIN_DENSITY,
    ];
    let sum: f64 = weights.iter().sum();
    for &x in &[0.0, 0.25, 0.5, 0.75, 1.0] {
        let layers = model.layer_targets([x; 3]);
        for layer in 0..3 {
            let expected =
                (x + MIN_DENSITY).clamp(0.0, model.channels[0].cap_total) * weights[layer] / sum;
            assert!(
                (layers[0][layer] - expected).abs() < 1e-12,
                "x {x}, layer {layer}: got {}, expected {expected}, weights {weights:?}",
                layers[0][layer]
            );
        }
    }
}
#[test]
fn saturated_layers_commit_capacity_and_redistribute_the_remainder() {
    let rows = vec![
        row(0.0, [MAXIMA[0], 0.0, 0.0]),
        row(0.5, [MAXIMA[0], 0.0, 0.0]),
        row(1.0, [MAXIMA[0], 0.0, 0.0]),
    ];
    let model = Model::new(
        &allocation_params(
            vec![0.0, 0.5, 1.0],
            rows,
            std::array::from_fn(|layer| [MAXIMA[layer]; 3]),
            false,
            [MIN_DENSITY; 3],
        ),
        PITCH,
    )
    .expect("model");
    let target = [0.2; 3];
    let layers = model.layer_targets(target);
    assert_conserved(target, &layers, &model);
    let cap0 = MAXIMA[0] + MAXIMA[0] / 0.3 * MIN_DENSITY;
    assert!((layers[0][0] - cap0).abs() < 1e-12);
    let weights = [
        cap0,
        MAXIMA[1] / 0.3 * MIN_DENSITY,
        MAXIMA[2] / 0.3 * MIN_DENSITY,
    ];
    let remainder = target[0] + MIN_DENSITY - cap0;
    let rest: f64 = weights[1..].iter().sum();
    for layer in 1..3 {
        let expected = remainder * weights[layer] / rest;
        assert!((layers[0][layer] - expected).abs() < 1e-12);
        assert!(layers[0][layer] < model.channels[0].cap[layer]);
    }
    let full = model.layer_targets([0.3; 3]);
    assert_eq!(full[0], model.channels[0].cap);
}

#[test]
fn all_zero_weights_fall_back_to_capacity_fractions() {
    let rows = vec![row(0.0, [0.0; 3]), row(0.5, [0.0; 3]), row(1.0, [0.0; 3])];
    let maxima = [[0.18, 0.18, 0.18], [0.09, 0.09, 0.09], [0.03, 0.03, 0.03]];
    let model = Model::new(
        &allocation_params(vec![0.0, 0.5, 1.0], rows, maxima, false, [0.0; 3]),
        PITCH,
    )
    .expect("model");
    let layers = model.layer_targets([0.2; 3]);
    let total = 0.2;
    for layer in 0..3 {
        assert!((layers[0][layer] - total * maxima[layer][0] / 0.3).abs() < 1e-12);
    }
}

#[test]
fn a_zero_capacity_layer_receives_nothing() {
    let mut p = params(None, true, 0x52, 1.0);
    p.layer_max = [[0.18, 0.18, 0.18], [0.09, 0.09, 0.09], [0.0, 0.0, 0.0]];
    for row in p.layer_curves.iter_mut() {
        row[2] = [0.0; 3];
    }
    let model = Model::new(&p, PITCH).expect("model");
    for &x in &[0.0, 0.05, 0.12, 0.3] {
        let target = [x; 3];
        let layers = model.layer_targets(target);
        assert_conserved(target, &layers, &model);
        for channel in &layers {
            assert_eq!(channel[2], 0.0, "zero-capacity layer took mass at {x}");
        }
    }
}

#[test]
fn a_single_active_layer_takes_the_whole_target() {
    let mut p = params(None, true, 0x53, 1.0);
    p.layer_max = [[0.18, 0.18, 0.18], [0.0, 0.0, 0.0], [0.0, 0.0, 0.0]];
    for row in p.layer_curves.iter_mut() {
        row[1] = [0.0; 3];
        row[2] = [0.0; 3];
    }
    let model = Model::new(&p, PITCH).expect("model");
    for &x in &[0.0, 0.05, 0.12, 0.21] {
        let target = [x; 3];
        let layers = model.layer_targets(target);
        assert_conserved(target, &layers, &model);
        for channel in &layers {
            assert_eq!(channel[1], 0.0);
            assert_eq!(channel[2], 0.0);
            assert!((channel[0] - (x + MIN_DENSITY).clamp(0.0, 0.21)).abs() < 1e-12);
        }
    }
}

#[test]
fn zero_density_min_allocates_the_bare_target() {
    let model = Model::new(
        &allocation_params(
            vec![0.0, 0.5, 1.0],
            vec![
                row(0.0, [0.0; 3]),
                row(0.5, [MAXIMA[0] / 2.0, MAXIMA[1] / 2.0, MAXIMA[2] / 2.0]),
                row(1.0, MAXIMA),
            ],
            std::array::from_fn(|layer| [MAXIMA[layer]; 3]),
            false,
            [0.0; 3],
        ),
        PITCH,
    )
    .expect("model");
    let layers = model.layer_targets([0.12; 3]);
    assert!((layers[0].iter().sum::<f64>() - 0.12).abs() < 1e-12);
    let weights = [0.06, 0.03, 0.01];
    let sum: f64 = weights.iter().sum();
    for layer in 0..3 {
        assert!((layers[0][layer] - 0.12 * weights[layer] / sum).abs() < 1e-12);
    }
    assert_eq!(model.layer_targets([0.0; 3]), [[0.0; 3]; 3]);
}

#[test]
fn nonzero_density_min_adds_the_floor_share_to_every_weight() {
    let floor = [0.02, 0.05, 0.08];
    let rows = vec![
        row(0.0, [0.0; 3]),
        row(0.5, [MAXIMA[0] / 2.0, MAXIMA[1] / 2.0, MAXIMA[2] / 2.0]),
        row(1.0, MAXIMA),
    ];
    let model = Model::new(
        &allocation_params(
            vec![0.0, 0.5, 1.0],
            rows.clone(),
            std::array::from_fn(|layer| [MAXIMA[layer]; 3]),
            false,
            floor,
        ),
        PITCH,
    )
    .expect("model");
    let target = [0.12; 3];
    let layers = model.layer_targets(target);
    for channel in 0..3 {
        let weights = weight_proportions_channel(
            &[0.0, 0.5, 1.0],
            &rows,
            target[channel],
            [
                MAXIMA[0] / 0.3 * floor[channel],
                MAXIMA[1] / 0.3 * floor[channel],
                MAXIMA[2] / 0.3 * floor[channel],
            ],
            channel,
        );
        let sum: f64 = weights.iter().sum();
        assert!(
            (layers[channel].iter().sum::<f64>() - target[channel] - floor[channel]).abs() < 1e-12
        );
        for layer in 0..3 {
            let expected = (target[channel] + floor[channel]) * weights[layer] / sum;
            assert!((layers[channel][layer] - expected).abs() < 1e-12);
        }
    }
}

#[test]
fn positive_film_flips_the_composite_lookup_axis() {
    // Positive profile inputs decrease with exposure. The production path
    // reverses the composite axis and negates the source query before lookup.
    // Distinct layer ratios at each knot make a missing query sign observable.
    let rows = vec![
        row(1.0, [MAXIMA[0], 0.0, 0.0]),
        row(0.5, [0.0, MAXIMA[1], 0.0]),
        row(0.0, [0.0, 0.0, MAXIMA[2]]),
    ];
    let positive = Model::new(
        &allocation_params(
            vec![1.0, 0.5, 0.0],
            rows.clone(),
            std::array::from_fn(|layer| [MAXIMA[layer]; 3]),
            true,
            [MIN_DENSITY; 3],
        ),
        PITCH,
    )
    .expect("model");
    let floor_share = [
        MAXIMA[0] / 0.3 * MIN_DENSITY,
        MAXIMA[1] / 0.3 * MIN_DENSITY,
        MAXIMA[2] / 0.3 * MIN_DENSITY,
    ];
    let signed_knots = [-1.0, -0.5, 0.0];
    // Keep the test targets below the smallest layer capacity so the expected
    // proportions isolate lookup direction rather than water-fill saturation.
    for &x in &[0.0125, 0.025] {
        let target = [x; 3];
        let actual = positive.layer_targets(target);
        assert_conserved(target, &actual, &positive);
        let weights = weight_proportions(&signed_knots, &rows, -x, floor_share);
        let total = target[0] + MIN_DENSITY;
        let sum: f64 = weights.iter().sum();
        for layer in 0..3 {
            let expected = total * weights[layer] / sum;
            assert!(
                (actual[0][layer] - expected).abs() < 1e-12,
                "x {x}: layer {layer} got {}, expected {expected}, weights {weights:?}",
                actual[0][layer]
            );
        }
    }
    let target = [0.0; 3];
    let actual = positive.layer_targets(target);
    let weights = weight_proportions(&signed_knots, &rows, 0.0, floor_share);
    let total = MIN_DENSITY;
    let sum: f64 = weights.iter().sum();
    for layer in 0..3 {
        let expected = total * weights[layer] / sum;
        assert!(
            (actual[0][layer] - expected).abs() < 1e-12,
            "endpoint x 0: layer {layer} got {}, expected {expected}",
            actual[0][layer]
        );
    }
}
