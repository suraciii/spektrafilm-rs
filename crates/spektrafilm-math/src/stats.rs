/// Fast statistical distributions for grain simulation.
///
/// Faithful port of Python `fast_stats.py` (`fast_binomial`, `fast_poisson`,
/// `fast_lognormal`, `fast_lognormal_from_mean_std`): same dispatch
/// thresholds (n < 25, variance > 10, lambda < 30, sigma < 1e-6) and the
/// same per-element algorithms — direct Bernoulli counting, normal
/// approximation, inverse-CDF walk, Knuth multiplication and the
/// mean/std → mu/sigma lognormal re-parameterization.
///
/// Upstream draws `np.random.rand()` / `np.random.randn()` from numba's
/// thread-local global RNG, so its per-pixel stream order is
/// nondeterministic by construction. The Rust port keeps the draws
/// deterministic by driving each element with its own `FastStatsRng`
/// stream (splitmix64 core, 53-bit uniforms like `rk_double`, cached
/// Marsaglia-polar normals like `rk_gauss`); the statistical regime is
/// preserved, bit-identical texture is not (and cannot be, upstream).
use rayon::prelude::*;

/// Deterministic RNG for the `fast_stats` sampling regime.
///
/// `rand()` returns a 53-bit double in `[0, 1)` (numpy `rk_double`
/// resolution); `randn()` is the Marsaglia polar normal with the spare
/// variate cached, matching the `np.random.randn` (`rk_gauss`) family.
pub struct FastStatsRng {
    state: u64,
    spare: Option<f64>,
}

impl FastStatsRng {
    /// New stream from a raw seed.
    pub fn new(seed: u64) -> Self {
        FastStatsRng {
            state: seed,
            spare: None,
        }
    }

    /// Independent stream for element `index` of an array draw, tagged by
    /// `tag` so a poisson pass and a binomial pass over the same element do
    /// not replay each other's uniforms.
    pub fn stream(base_seed: u64, tag: u64, index: u64) -> Self {
        let mut z = base_seed.wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ tag.wrapping_mul(0xBF58_476D_1CE4_E5B9)
            ^ index.wrapping_mul(0x94D0_49BB_1331_11EB);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        Self::new(z ^ (z >> 31))
    }

    #[inline]
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform double in `[0, 1)` with 53-bit resolution.
    #[inline]
    pub fn rand(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64) * (1.0 / ((1u64 << 53) as f64))
    }

    /// Standard normal via Marsaglia polar rejection, spare variate cached.
    #[inline]
    pub fn randn(&mut self) -> f64 {
        if let Some(v) = self.spare.take() {
            return v;
        }
        loop {
            let u = 2.0 * self.rand() - 1.0;
            let v = 2.0 * self.rand() - 1.0;
            let s = u * u + v * v;
            if s > 0.0 && s < 1.0 {
                let factor = (-2.0 * s.ln() / s).sqrt();
                self.spare = Some(v * factor);
                return u * factor;
            }
        }
    }
}

/// Port of `fast_poisson`: λ ≤ 0 → 0; λ < 30 → Knuth multiplication of
/// uniforms; otherwise a rounded normal approximation clamped at 0.
#[inline]
pub fn fast_poisson(rng: &mut FastStatsRng, lam: f64) -> i64 {
    if lam <= 0.0 {
        return 0;
    }
    if lam < 30.0 {
        let l = (-lam).exp();
        let mut p = 1.0;
        let mut k = 0i64;
        while p > l {
            k += 1;
            p *= rng.rand();
        }
        k - 1
    } else {
        let sample = lam + lam.sqrt() * rng.randn();
        let sample_int = sample.round() as i64;
        if sample_int < 0 { 0 } else { sample_int }
    }
}

/// Port of `fast_binomial`: p ≤ 0 → 0; p ≥ 1 → n; n < 25 → direct Bernoulli
/// counting; variance > 10 → rounded normal approximation clamped to
/// `[0, n]`; otherwise the inverse-CDF walk.
#[inline]
pub fn fast_binomial(rng: &mut FastStatsRng, n: i64, p: f64) -> i64 {
    if p <= 0.0 {
        return 0;
    }
    if p >= 1.0 {
        return n;
    }
    if n < 25 {
        let mut count = 0i64;
        for _ in 0..n {
            if rng.rand() < p {
                count += 1;
            }
        }
        count
    } else {
        let nf = n as f64;
        let mean = nf * p;
        let var = nf * p * (1.0 - p);
        if var > 10.0 {
            let approx = mean + var.sqrt() * rng.randn();
            let mut approx_int = approx.round() as i64;
            if approx_int < 0 {
                approx_int = 0;
            } else if approx_int > n {
                approx_int = n;
            }
            approx_int
        } else {
            let u = rng.rand();
            let mut cdf = 0.0;
            let mut prob = (1.0 - p).powi(n as i32);
            let mut k = 0i64;
            while cdf < u && k <= n {
                cdf += prob;
                if k < n {
                    prob *= ((nf - k as f64) / (k as f64 + 1.0)) * (p / (1.0 - p));
                }
                k += 1;
            }
            k - 1
        }
    }
}

/// Port of `fast_lognormal`: σ < 1e-6 → `exp(mu)`; otherwise
/// `exp(mu + sigma * z)`.
#[inline]
pub fn fast_lognormal(rng: &mut FastStatsRng, mu: f64, sigma: f64) -> f64 {
    if sigma < 1e-6 {
        mu.exp()
    } else {
        (mu + sigma * rng.randn()).exp()
    }
}

/// Port of `fast_lognormal_from_mean_std`: invert the linear-space moments
/// `m = exp(mu + sigma²/2)`, `s² = (exp(sigma²) - 1)·exp(2mu + sigma²)` to
/// `(mu, sigma)` and draw a lognormal. `m <= 0` degenerates to 1.0.
#[inline]
pub fn fast_lognormal_from_mean_std(rng: &mut FastStatsRng, m: f64, s: f64) -> f64 {
    let (mu, sigma) = if m <= 0.0 {
        (0.0, 0.0)
    } else {
        let sigma2 = (1.0 + (s * s) / (m * m)).ln();
        (m.ln() - sigma2 / 2.0, sigma2.sqrt())
    };
    fast_lognormal(rng, mu, sigma)
}

/// Array draw of `fast_poisson` over independently seeded elements — the
/// Rust-deterministic counterpart of numba's `prange` kernel.
pub fn fast_poisson_array(base_seed: u64, tag: u64, lam: &[f64]) -> Vec<i64> {
    lam.par_iter()
        .enumerate()
        .map(|(i, &l)| fast_poisson(&mut FastStatsRng::stream(base_seed, tag, i as u64), l))
        .collect()
}

/// Array draw of `fast_binomial` over independently seeded elements.
pub fn fast_binomial_array(base_seed: u64, tag: u64, n: &[i64], p: &[f64]) -> Vec<i64> {
    n.par_iter()
        .zip(p)
        .enumerate()
        .map(|(i, (&ni, &pi))| {
            fast_binomial(&mut FastStatsRng::stream(base_seed, tag, i as u64), ni, pi)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sample mean and (population) variance of a per-index sampler. Each
    /// sample index gets its own deterministic stream, mirroring an array
    /// draw over independent elements.
    fn mean_var(n: usize, mut draw: impl FnMut(usize) -> f64) -> (f64, f64) {
        let mut sum = 0.0;
        let mut sum2 = 0.0;
        for i in 0..n {
            let v = draw(i);
            sum += v;
            sum2 += v * v;
        }
        let mean = sum / n as f64;
        (mean, sum2 / n as f64 - mean * mean)
    }

    #[test]
    fn fast_stats_rng_is_deterministic() {
        let mut a = FastStatsRng::new(7);
        let mut b = FastStatsRng::new(7);
        for _ in 0..64 {
            assert_eq!(a.rand(), b.rand());
        }
        let mut c = FastStatsRng::new(7);
        let mut d = FastStatsRng::new(7);
        for _ in 0..64 {
            assert_eq!(c.randn(), d.randn());
        }
        // Distinct seeds/indices must not alias.
        let mut e = FastStatsRng::stream(3, 0, 5);
        let mut f = FastStatsRng::stream(3, 1, 5);
        let mut g = FastStatsRng::stream(3, 0, 6);
        assert!(e.rand() != f.rand() || e.rand() != g.rand());
    }

    #[test]
    fn fast_poisson_edges_and_regimes() {
        let mut rng = FastStatsRng::new(11);
        assert_eq!(fast_poisson(&mut rng, 0.0), 0);
        assert_eq!(fast_poisson(&mut rng, -2.0), 0);
        // Knuth regime: draws are non-negative and bounded well below the
        // normal regime's scale.
        for _ in 0..1000 {
            let k = fast_poisson(&mut rng, 4.0);
            assert!(k >= 0 && k <= 30);
        }
        // Normal regime: rounded, clamped at 0, centered on λ (individual
        // draws can dip below 30 — Poisson(60) has σ ≈ 7.7).
        let mut sum = 0i64;
        for _ in 0..1000 {
            let k = fast_poisson(&mut rng, 60.0);
            assert!(k >= 0);
            sum += k;
        }
        assert!((sum as f64 / 1000.0 - 60.0).abs() < 3.0);
    }

    #[test]
    fn fast_poisson_statistics() {
        // Both regimes: mean ≈ λ, variance ≈ λ (loose bounds — the Python
        // reference itself is only statistically specified here).
        for (tag, lam) in [(0u64, 0.7f64), (1, 8.0), (2, 45.0)] {
            let (mean, var) = mean_var(20_000, |i| {
                fast_poisson(&mut FastStatsRng::stream(99, tag, i as u64), lam) as f64
            });
            assert!(
                (mean - lam).abs() < 0.06 * lam + 0.3,
                "lam {lam}: mean {mean}"
            );
            assert!((var - lam).abs() < 0.12 * lam + 0.5, "lam {lam}: var {var}");
        }
    }

    #[test]
    fn fast_binomial_edges() {
        let mut rng = FastStatsRng::new(13);
        assert_eq!(fast_binomial(&mut rng, 40, 0.0), 0);
        assert_eq!(fast_binomial(&mut rng, 40, -0.1), 0);
        assert_eq!(fast_binomial(&mut rng, 40, 1.0), 40);
        assert_eq!(fast_binomial(&mut rng, 40, 1.5), 40);
        assert_eq!(fast_binomial(&mut rng, 0, 0.3), 0);
    }

    #[test]
    fn fast_binomial_statistics() {
        // Direct regime (n < 25): n=12, p=0.4 → mean 4.8, var 2.88.
        let (mean, var) = mean_var(20_000, |i| {
            fast_binomial(&mut FastStatsRng::stream(21, 0, i as u64), 12, 0.4) as f64
        });
        assert!((mean - 4.8).abs() < 0.15, "mean {mean}");
        assert!((var - 2.88).abs() < 0.35, "var {var}");

        // Normal-approximation regime (var > 10): n=400, p=0.5.
        let (mean, var) = mean_var(20_000, |i| {
            fast_binomial(&mut FastStatsRng::stream(22, 1, i as u64), 400, 0.5) as f64
        });
        assert!((mean - 200.0).abs() < 2.0, "mean {mean}");
        assert!((var - 100.0).abs() < 12.0, "var {var}");

        // Inversion regime (var <= 10, n >= 25): n=50, p=0.03 → mean 1.5.
        let (mean, var) = mean_var(20_000, |i| {
            fast_binomial(&mut FastStatsRng::stream(23, 2, i as u64), 50, 0.03) as f64
        });
        assert!((mean - 1.5).abs() < 0.08, "mean {mean}");
        assert!((var - 1.455).abs() < 0.2, "var {var}");
    }

    #[test]
    fn fast_lognormal_degenerate_sigma_is_exp_mu() {
        let mut rng = FastStatsRng::new(17);
        assert_eq!(fast_lognormal(&mut rng, 0.5, 0.0), (0.5f64).exp());
        assert_eq!(fast_lognormal(&mut rng, -2.0, 1e-9), (-2.0f64).exp());
    }

    #[test]
    fn fast_lognormal_from_mean_std_statistics() {
        // m <= 0 degenerates to exactly 1.0 (mu = sigma = 0 → exp(0)).
        let mut rng = FastStatsRng::new(19);
        assert_eq!(fast_lognormal_from_mean_std(&mut rng, 0.0, 0.3), 1.0);
        assert_eq!(fast_lognormal_from_mean_std(&mut rng, -1.0, 0.3), 1.0);

        // m = 1, s = 0.25: the lognormal mean must stay ~1.
        let (mean, _) = mean_var(20_000, |i| {
            fast_lognormal_from_mean_std(&mut FastStatsRng::stream(24, 0, i as u64), 1.0, 0.25)
        });
        assert!((mean - 1.0).abs() < 0.02, "mean {mean}");
    }

    #[test]
    fn array_draws_are_deterministic_and_tagged() {
        let lam = vec![5.0f64; 32];
        let a = fast_poisson_array(7, 0, &lam);
        let b = fast_poisson_array(7, 0, &lam);
        assert_eq!(a, b);
        let n = vec![40i64; 32];
        let p = vec![0.3f64; 32];
        let c = fast_binomial_array(7, 1, &n, &p);
        let d = fast_binomial_array(7, 1, &n, &p);
        assert_eq!(c, d);
        assert!(a.iter().any(|&v| v > 0));
        assert!(c.iter().all(|&v| (0..=40).contains(&v)));
    }
}
