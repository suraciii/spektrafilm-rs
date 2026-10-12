use super::*;
use std::f64::consts::PI;
pub type Bin = Sublayer;
#[derive(Debug, Clone)]
pub struct Sublayer {
    pub cloud_radius_mm: f64,
    pub dye_per_cloud: f64,
    pub crystals_per_mm2: f64,
    pub pool: f64,
    pub demand_at_full: f64,
    pub gain: f64,
    pub classes: Vec<usize>,
    pub shares: Vec<f64>,
    pub forming: Vec<f64>,
    pub capacity: f64,
    pub edge: f64,
    pub sigma_mm: f64,
    pub cell_mm: f64,
    pub void_integral_mm2: f64,
    pub silver: bool,
    pub smallest_sigma_mm: f64,
}
#[derive(Debug, Clone)]
pub struct Record {
    pub sublayers: Vec<Sublayer>,
    pub d_min: f64,
    pub d_max: f64,
}
#[derive(Debug, Clone, Default)]
pub struct RecordDiagnostics {
    pub fit_error: f64,
    pub weights: Vec<f64>,
    pub log_radius: Vec<f64>,
    pub fog_fractions: Vec<f64>,
    pub developability: f64,
    pub bins: Vec<Bin>,
    pub read_net_density: f64,
    pub read_sigma: f64,
}
fn grid(i: usize) -> f64 {
    (-4.0 + i as f64 * 0.05) as f32 as f64
}
fn class(i: usize) -> f64 {
    (-3.0 + i as f64 * 0.2) as f32 as f64
}
pub(super) fn hit(x: f64, e: f64) -> f64 {
    let m = 10_f32.powf((x - e) as f32) as f64;
    (1.0 - (-m).exp() * (1.0 + m + m * m / 2.0)).clamp(0.0, 1.0) as f32 as f64
}
fn latent(x: f64, e: f64, reach: f64) -> f64 {
    let threshold = hit(x, e);
    if reach <= 1.0 {
        threshold * reach.max(0.0)
    } else {
        let m = 10_f32.powf((x - e) as f32) as f64;
        (threshold + (reach - 1.0) * (-m).exp() * m * m / 2.0).min(1.0)
    }
}
fn saturated(f: f64, g: f64) -> f64 {
    if g >= 1.0 {
        f
    } else {
        (1.0 - g.powf(f)) / (1.0 - g)
    }
}
fn dye(d: f64, p: f64) -> f64 {
    if p > 0.0 {
        p * (1.0 - (-d / p).exp())
    } else {
        d
    }
}
fn fractions(bins: &[Bin], fog: &[f64], x: f64, reach: f64, reversal: bool) -> Vec<f64> {
    bins.iter()
        .map(|b| {
            b.classes
                .iter()
                .zip(&b.shares)
                .map(|(k, s)| {
                    let developed = 1.0 - (1.0 - latent(x, class(*k), reach)) * (1.0 - fog[*k]);
                    s * if reversal { 1.0 - developed } else { developed }
                })
                .sum()
        })
        .collect()
}
fn nnls(a: &[Vec<f64>], y: &[f64]) -> Vec<f64> {
    let n = a[0].len();
    let mut gram = vec![0.0; n * n];
    let mut moment = vec![0.0; n];
    for (row, target) in a.iter().zip(y) {
        for i in 0..n {
            moment[i] += row[i] * target;
            for j in 0..n {
                gram[i * n + j] += row[i] * row[j];
            }
        }
    }
    let mut weights = vec![0.0; n];
    for _ in 0..5000 {
        let mut moved = 0.0_f64;
        for i in 0..n {
            if gram[i * n + i] <= 0.0 {
                continue;
            }
            let mut residual = moment[i];
            for j in 0..n {
                if j != i {
                    residual -= gram[i * n + j] * weights[j];
                }
            }
            let updated = (residual / gram[i * n + i]).max(0.0);
            moved = moved.max((updated - weights[i]).abs());
            weights[i] = updated;
        }
        if moved < 1e-9 {
            break;
        }
    }
    weights.into_iter().map(|x| x as f32 as f64).collect()
}
fn fit_error(a: &[Vec<f64>], w: &[f64], y: &[f64]) -> f64 {
    a.iter()
        .zip(y)
        .map(|(r, t)| (r.iter().zip(w).map(|(a, b)| a * b).sum::<f64>() - t).abs())
        .fold(0.0, f64::max)
}
fn fog_fractions(weights: &[f64], radii: &[f64], density: f64) -> Vec<f64> {
    let largest = weights
        .iter()
        .zip(radii)
        .filter(|(w, _)| **w > 0.0)
        .map(|(_, r)| *r)
        .fold(0.0, f64::max);
    let surface: Vec<f64> = radii
        .iter()
        .map(|r| 10.0_f64.powf(2.0 * (r - largest)))
        .collect();
    let formed = |s: f64| {
        weights
            .iter()
            .zip(&surface)
            .map(|(w, r)| w * (s * r).min(0.5))
            .sum::<f64>()
    };
    if density <= 0.0 || formed(f64::MAX) <= 0.0 {
        return vec![0.0; weights.len()];
    }
    let (mut low, mut high) = (0.0, 1.0);
    while formed(high) < density && high < 1e9 {
        high *= 2.0;
    }
    for _ in 0..60 {
        let mid = (low + high) / 2.0;
        if formed(mid) < density {
            low = mid;
        } else {
            high = mid;
        }
    }
    surface.iter().map(|s| (high * s).min(0.5)).collect()
}
fn aperture_response(sigma: f64) -> f64 {
    if sigma <= 0.0 {
        return 1.0;
    }
    let z = 0.024_f64.powi(2) / (2.0 * sigma * sigma);
    let (i0, i1) = if z < 3.75 {
        let t = (z / 3.75).powi(2);
        let a = 1.0
            + t * (3.5156229
                + t * (3.0899424
                    + t * (1.2067492 + t * (0.2659732 + t * (0.0360768 + t * 0.0045813)))));
        let b = z
            * (0.5
                + t * (0.87890594
                    + t * (0.51498869
                        + t * (0.15084934
                            + t * (0.02658733 + t * (0.00301532 + t * 0.00032411))))));
        (a * (-z).exp(), b * (-z).exp())
    } else {
        let t = 3.75 / z;
        let a = 0.39894228
            + t * (0.01328592
                + t * (0.00225319
                    + t * (-0.00157565
                        + t * (0.00916281
                            + t * (-0.02057706
                                + t * (0.02635537 + t * (-0.01647633 + t * 0.00392377)))))));
        let tail = 0.02282967 + t * (-0.02895312 + t * (0.01787654 - t * 0.00420059));
        let b = 0.39894228
            + t * (-0.03988024
                + t * (-0.00362018 + t * (0.00163801 + t * (-0.01031555 + t * tail))));
        (a / z.sqrt(), b / z.sqrt())
    };
    (1.0 - i0 - i1).max(1e-4).sqrt()
}
fn variance(bins: &[Bin], f: &[f64]) -> f64 {
    bins.iter()
        .zip(f)
        .map(|(b, f)| {
            let gain = if b.pool > 0.0 {
                (-b.demand_at_full * f / b.pool).exp()
            } else {
                1.0
            };
            gain * gain
                * b.crystals_per_mm2
                * f
                * b.dye_per_cloud
                * b.dye_per_cloud
                * 1.25
                * aperture_response(b.cloud_radius_mm / 2.0).powi(2)
                / (PI * 0.024 * 0.024)
        })
        .sum()
}
pub(super) fn prepare(
    input: PopulationInput,
    cancel: Cancellation<'_>,
) -> Result<Population, String> {
    (cancel)()?;
    input.metadata.validate()?;
    if let Some(m) = &input.reference_metadata {
        m.validate()?;
    }
    for c in &input.curves {
        c.validate()?;
    }
    let references = input.reference_curves.as_ref().unwrap_or(&input.curves);
    for c in references {
        c.validate()?;
    }
    let rm = input.reference_metadata.as_ref().unwrap_or(&input.metadata);
    if rm.silver != input.metadata.silver || rm.reversal != input.metadata.reversal {
        return Err("Fotufilm reference and developed material laws must agree".into());
    }
    let mut records = Vec::with_capacity(3);
    let mut reports = Vec::with_capacity(3);
    let mut anchor_gross = [0.0; 3];
    let mut targets = [0.0; 3];
    for r in 0..3 {
        (cancel)()?;
        let c = &input.curves[r];
        let reference = &references[r];
        let gain = if rm.silver { 1.0_f64 } else { 0.1 };
        let kappa = if gain >= 1.0 {
            1.0
        } else {
            -gain.ln() / (1.0 - gain)
        };
        let target: Vec<f64> = (0..161)
            .map(|i| reference.density_at(grid(i)) - reference.d_min)
            .collect();
        let basis: Vec<Vec<f64>> = (0..161)
            .map(|i| {
                (0..36)
                    .map(|k| {
                        let p = hit(grid(i), class(k));
                        let y = saturated(if rm.reversal { 1.0 - p } else { p }, gain);
                        if rm.reversal { 1.0 - y } else { y }
                    })
                    .collect()
            })
            .collect();
        let mut weights = nnls(&basis, &target);
        let peak = weights.iter().copied().fold(0.0, f64::max);
        let active: Vec<usize> = (0..36).filter(|k| weights[*k] > 1e-6 * peak).collect();
        let mut radii: Vec<f64> = (0..36).map(|k| -class(k) / 3.0).collect();
        if !active.is_empty() {
            let w = active.iter().map(|k| weights[*k]).sum::<f64>();
            let mean = active.iter().map(|k| weights[*k] * radii[*k]).sum::<f64>() / w;
            let half = rm.radius_span.log10() / 2.0;
            for x in &mut radii {
                *x = x.clamp(mean - half, mean + half);
            }
        }
        let mut members: [Vec<usize>; 4] = std::array::from_fn(|_| Vec::new());
        let total = active.iter().map(|k| weights[*k]).sum::<f64>();
        let cuts = [
            rm.sublayer_shares[0],
            rm.sublayer_shares[..2].iter().sum(),
            rm.sublayer_shares[..3].iter().sum(),
        ];
        let mut cumulative = 0.0;
        for k in active {
            cumulative += weights[k];
            let f = cumulative / total - 1e-6;
            let b = cuts.iter().position(|cut| f < *cut).unwrap_or(3);
            members[b].push(k);
        }
        let mut bins = Vec::new();
        for ks in members.into_iter().filter(|ks| !ks.is_empty()) {
            let weight = ks.iter().map(|k| weights[*k]).sum::<f64>();
            let radius =
                10.0_f64.powf(ks.iter().map(|k| weights[*k] * radii[*k]).sum::<f64>() / weight);
            let shares = ks.iter().map(|k| weights[*k] / weight).collect();
            bins.push(Sublayer {
                cloud_radius_mm: radius,
                dye_per_cloud: 0.0,
                crystals_per_mm2: 0.0,
                pool: 0.0,
                demand_at_full: weight * kappa,
                gain: 1.0,
                classes: ks,
                shares,
                forming: Vec::new(),
                capacity: 0.0,
                edge: if rm.silver { 12.0 } else { 1.0 },
                sigma_mm: 0.0,
                cell_mm: 0.0,
                void_integral_mm2: 0.0,
                silver: rm.silver,
                smallest_sigma_mm: 0.0,
            });
        }
        let zero = vec![0.0; 36];
        let bin_basis: Vec<Vec<f64>> = (0..161)
            .map(|i| {
                fractions(&bins, &zero, grid(i), 1.0, rm.reversal)
                    .iter()
                    .map(|f| {
                        let y = saturated(*f, gain);
                        if rm.reversal { 1.0 - y } else { y }
                    })
                    .collect()
            })
            .collect();
        let refitted = if bins.is_empty() {
            Vec::new()
        } else {
            nnls(&bin_basis, &target)
        };
        let mut error = if bins.is_empty() {
            target.iter().copied().fold(0.0, f64::max)
        } else {
            fit_error(&bin_basis, &refitted, &target)
        };
        for (i, b) in bins.iter_mut().enumerate() {
            let before = b.demand_at_full / kappa;
            let scale = if before > 0.0 {
                refitted[i] / before
            } else {
                0.0
            };
            b.demand_at_full = refitted[i] * kappa;
            for k in &b.classes {
                weights[*k] *= scale;
            }
        }
        let fastest = bins.first().map(|b| b.cloud_radius_mm).unwrap_or(1.0);
        for b in &mut bins {
            let relative = b.cloud_radius_mm / fastest;
            b.cloud_radius_mm = rm.cloud_radius_mm * rm.layer_size_ratio[r] * relative;
            b.dye_per_cloud = relative.powi(2);
            b.pool = if rm.silver {
                0.0
            } else {
                b.demand_at_full / -gain.ln()
            };
            b.crystals_per_mm2 = b.demand_at_full / b.dye_per_cloud;
        }
        let fog = fog_fractions(&weights, &radii, input.metadata.fog_density);
        let net = rm.anchor_net_density[r].clamp(
            0.05 * (reference.d_max - reference.d_min),
            0.9 * (reference.d_max - reference.d_min),
        );
        let gross = if rm.reversal {
            reference.d_max - net
        } else {
            reference.d_min + net
        };
        let at = reference.exposure_at(gross);
        let v = variance(&bins, &fractions(&bins, &fog, at, 1.0, rm.reversal));
        let per_cloud = if v > 0.0 {
            rm.sigma48[r].powi(2) / v
        } else {
            0.0
        };
        for b in &mut bins {
            b.dye_per_cloud *= per_cloud;
            b.crystals_per_mm2 = if b.dye_per_cloud > 0.0 {
                b.demand_at_full / b.dye_per_cloud
            } else {
                0.0
            };
        }
        let mut reach = 1.0;
        if (0..161).any(|i| (c.density_at(grid(i)) - reference.density_at(grid(i))).abs() >= 1e-4) {
            let targets: Vec<f64> = (0..161).map(|i| c.density_at(grid(i)) - c.d_min).collect();
            let mut best = (f64::INFINITY, 1.0, vec![1.0; bins.len()], 0.0);
            for step in 0..31 {
                (cancel)()?;
                let dev = 0.3 + step as f64 * 0.05;
                let fs: Vec<Vec<f64>> = (0..161)
                    .map(|i| fractions(&bins, &fog, grid(i), dev, rm.reversal))
                    .collect();
                let formed = |g: &[f64], row: &[f64]| {
                    bins.iter()
                        .zip(g)
                        .zip(row)
                        .map(|((b, g), f)| dye(b.demand_at_full * g * f, b.pool))
                        .sum::<f64>()
                };
                let residual = |g: &[f64]| {
                    fs.iter()
                        .zip(&targets)
                        .map(|(row, t)| (formed(g, row) - t).powi(2))
                        .sum::<f64>()
                };
                let mut gains = vec![1.0; bins.len()];
                for _ in 0..40 {
                    let mut moved = 0.0_f64;
                    for b in 0..bins.len() {
                        let (mut low, mut high) = (0.0, 6.0);
                        let phi = 0.6180339887;
                        let (mut left, mut right) =
                            (high - phi * (high - low), low + phi * (high - low));
                        let mut probe = gains.clone();
                        probe[b] = left;
                        let mut fl = residual(&probe);
                        probe[b] = right;
                        let mut fr = residual(&probe);
                        for _ in 0..30 {
                            if fl < fr {
                                high = right;
                                right = left;
                                fr = fl;
                                left = high - phi * (high - low);
                                probe[b] = left;
                                fl = residual(&probe);
                            } else {
                                low = left;
                                left = right;
                                fl = fr;
                                right = low + phi * (high - low);
                                probe[b] = right;
                                fr = residual(&probe);
                            }
                        }
                        let next = (low + high) / 2.0;
                        moved = moved.max((next - gains[b]).abs());
                        gains[b] = next;
                    }
                    if moved < 1e-5 {
                        break;
                    }
                }
                let sse = residual(&gains);
                if sse < best.0 {
                    let worst = fs
                        .iter()
                        .zip(&targets)
                        .map(|(row, t)| (formed(&gains, row) - t).abs())
                        .fold(0.0, f64::max);
                    best = (sse, dev, gains, worst);
                }
            }
            reach = best.1;
            error = best.3;
            for (b, g) in bins.iter_mut().zip(best.2) {
                b.gain = g;
                b.demand_at_full *= g;
                b.dye_per_cloud *= g;
            }
            if (input.metadata.sigma48[r] - rm.sigma48[r]).abs() > 1e-6 {
                let n = input.metadata.anchor_net_density[r]
                    .clamp(0.05 * (c.d_max - c.d_min), 0.9 * (c.d_max - c.d_min));
                let x = c.exposure_at(if rm.reversal {
                    c.d_max - n
                } else {
                    c.d_min + n
                });
                let modelled = variance(&bins, &fractions(&bins, &fog, x, reach, rm.reversal));
                if modelled > 0.0 {
                    let factor = input.metadata.sigma48[r].powi(2) / modelled;
                    for b in &mut bins {
                        b.dye_per_cloud *= factor;
                        b.crystals_per_mm2 /= factor;
                    }
                }
            }
        }
        let read_net = input.metadata.anchor_net_density[r]
            .clamp(0.05 * (c.d_max - c.d_min), 0.9 * (c.d_max - c.d_min));
        targets[r] = input.metadata.sigma48[r] / rm.coating_density_scale.sqrt();
        anchor_gross[r] = if rm.reversal {
            c.d_max - read_net
        } else {
            c.d_min + read_net
        };
        for b in &mut bins {
            b.crystals_per_mm2 *= rm.coating_density_scale;
            b.dye_per_cloud /= rm.coating_density_scale;
            b.capacity = if rm.silver { 2.0 } else { b.pool.max(0.05) };
            let relative = b.cloud_radius_mm / (rm.cloud_radius_mm * rm.layer_size_ratio[r]);
            b.smallest_sigma_mm = if rm.silver {
                0.0012 * relative / (2.0 * (2.0 * (12.0 / std::f64::consts::LN_2).ln()).sqrt())
            } else {
                0.00145
            };
            b.cell_mm = (8.0 / b.crystals_per_mm2.max(1e-30))
                .sqrt()
                .clamp(0.00025, 0.004);
        }
        let table: Vec<Vec<f64>> = (0..129)
            .map(|k| {
                let d = c.d_min + (c.d_max - c.d_min) * k as f64 / 128.0;
                fractions(&bins, &fog, c.exposure_at(d), reach, rm.reversal)
            })
            .collect();
        for (i, b) in bins.iter_mut().enumerate() {
            b.forming = table.iter().map(|row| row[i]).collect();
            if b.dye_per_cloud > 0.0 {
                b.shape();
            }
        }
        reports.push(RecordDiagnostics {
            fit_error: error,
            weights,
            log_radius: radii,
            fog_fractions: fog,
            developability: reach,
            bins: bins.clone(),
            read_net_density: read_net,
            read_sigma: targets[r],
        });
        records.push(Record {
            sublayers: bins
                .into_iter()
                .filter(|b| b.crystals_per_mm2 > 0.0 && b.dye_per_cloud > 0.0)
                .collect(),
            d_min: c.d_min,
            d_max: c.d_max,
        });
    }
    let mut p = Population {
        records: records.try_into().unwrap(),
        monochrome: input.metadata.monochrome,
        diagnostics: Diagnostics {
            provenance: PROVENANCE,
            records: reports.try_into().unwrap(),
            anchor_target: targets,
            anchor_measured: [0.0; 3],
            anchor_log_error: [0.0; 3],
            anchor_passes: [0; 3],
        },
    };
    super::tiles::anchor(&mut p, anchor_gross, cancel)?;
    Ok(p)
}
const TERMS: [(f64, f64); 4] = [
    (0.1162, 0.1096),
    (0.3089, 0.2167),
    (0.7469, 0.4030),
    (1.732, 0.2510),
];
fn profile(r: f64, silver: bool) -> f64 {
    if silver {
        (-r * r / 2.0).exp()
    } else {
        TERMS
            .iter()
            .map(|(s, w)| w * (-r * r / (2.0 * s * s)).exp())
            .sum()
    }
}
fn reach(peak: f64, silver: bool) -> f64 {
    (if silver { 1.0 } else { 1.732 }) * (2.0 * ((peak * 8.0).max(1.0) / 1e-7).ln()).sqrt()
}
fn gamma_quantile(p: f64) -> f64 {
    let (mut low, mut high) = (0.0_f64, 80.0_f64);
    for _ in 0..80 {
        let x = (low + high) / 2.0;
        let cdf = 1.0 - (-x).exp() * (1.0 + x + x * x / 2.0 + x * x * x / 6.0);
        if cdf < p { low = x } else { high = x }
    }
    (low + high) / 8.0
}
// The source stores these constants as decimal literals (FilmGrain.swift:447
// and :491); keeping the same digits preserves the ported arithmetic.
#[allow(clippy::approx_constant)]
fn unit_cloud(capacity: f64, edge: f64, silver: bool) -> f64 {
    let marks: Vec<f64> = (0..24)
        .map(|q| gamma_quantile((q as f64 + 0.5) / 24.0))
        .collect();
    let dr = reach(edge * marks[23], silver) / 800.0;
    let mut total = 0.0;
    for m in marks {
        let mut area = 0.0;
        for i in 0..800 {
            let r = (i as f64 + 0.5) * dr;
            let d = capacity * (1.0 - (-edge * m * profile(r, silver)).exp());
            area += (1.0 - (-std::f64::consts::LN_10 * d).exp()) * r * dr;
        }
        total += 0.4342944819 * 2.0 * PI * area;
    }
    total / 24.0
}
// The source stores Euler's constant as a decimal literal (FilmGrain.swift:491).
#[allow(clippy::approx_constant)]
pub(super) fn occupied_area(peak: f64, silver: bool) -> f64 {
    if !silver {
        let dr = reach(peak, false) / 800.0;
        return 2.0
            * PI
            * (0..800)
                .map(|i| {
                    let r = (i as f64 + 0.5) * dr;
                    (1.0 - (-peak * profile(r, false)).exp()) * r * dr
                })
                .sum::<f64>();
    }
    let ein = if peak < 6.0 {
        let mut term = peak;
        let mut sum = 0.0;
        for k in 1..=60 {
            sum += term / k as f64;
            term *= -peak / (k + 1) as f64;
            if term.abs() < 1e-14 {
                break;
            }
        }
        sum
    } else {
        (-peak).exp() / peak * (1.0 - 1.0 / peak + 2.0 / peak.powi(2) - 6.0 / peak.powi(3))
            + peak.ln()
            + 0.5772156649
    };
    2.0 * PI * ein
}
impl Sublayer {
    pub(super) fn shape(&mut self) {
        let free = if self.silver {
            (self.dye_per_cloud / unit_cloud(self.capacity, self.edge, true)).sqrt()
        } else {
            0.0
        };
        let mut peak = self.edge;
        if free >= self.smallest_sigma_mm {
            self.sigma_mm = free;
        } else {
            self.sigma_mm = self.smallest_sigma_mm;
            let wanted = (self.dye_per_cloud / self.sigma_mm.powi(2)).ln();
            let miss = |x: f64| unit_cloud(self.capacity, x.exp(), self.silver).ln() - wanted;
            let (mut low, mut high) = if self.silver {
                ((self.edge * 1e-5).ln(), self.edge.ln())
            } else {
                (1e-6_f64.ln(), 1e4_f64.ln())
            };
            let (mut fl, mut fh) = (miss(low), miss(high));
            let mut x = if fl >= 0.0 { low } else { high };
            if fl < 0.0 && fh > 0.0 {
                let mut side = 0;
                for _ in 0..40 {
                    x = (low * fh - high * fl) / (fh - fl);
                    let fx = miss(x);
                    if fx.abs() < 1e-6 || high - low < 1e-6 {
                        break;
                    }
                    if fx < 0.0 {
                        low = x;
                        fl = fx;
                        if side == -1 {
                            fh /= 2.0;
                        }
                        side = -1;
                    } else {
                        high = x;
                        fh = fx;
                        if side == 1 {
                            fl /= 2.0;
                        }
                        side = 1;
                    }
                }
            }
            peak = x.exp();
        }
        self.edge = peak;
        self.void_integral_mm2 = self.sigma_mm.powi(2)
            * (0..24)
                .map(|q| occupied_area(peak * gamma_quantile((q as f64 + 0.5) / 24.0), self.silver))
                .sum::<f64>()
            / 24.0;
    }
}
