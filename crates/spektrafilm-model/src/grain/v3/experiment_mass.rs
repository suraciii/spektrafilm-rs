//! Isolated random-mark experiments; these mechanism values are uncalibrated.
//!
//! For q = E[W²] = 1.25, increasing the event rate by q and dividing each
//! mark by q preserves mean and variance, but changes higher cumulants and
//! optical ensemble means. Fixed mass remains the default.

use spektrafilm_math::stats::FastStatsRng;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MassLaw {
    Fixed,
    TwoClass,
    Gamma4,
}

impl MassLaw {
    pub(super) fn from_env() -> Result<Self, String> {
        match std::env::var("SPEKTRAFILM_GRAIN_V3_MASS_LAW") {
            Ok(value) => Self::parse(&value),
            Err(std::env::VarError::NotPresent) => Ok(Self::Fixed),
            Err(std::env::VarError::NotUnicode(_)) => Err(
                "film grain V3: SPEKTRAFILM_GRAIN_V3_MASS_LAW must be fixed, two_class or gamma4"
                    .to_owned(),
            ),
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "fixed" => Ok(Self::Fixed),
            "two_class" => Ok(Self::TwoClass),
            "gamma4" => Ok(Self::Gamma4),
            _ => Err(format!(
                "film grain V3: unknown mass law {value:?}; expected fixed, two_class or gamma4"
            )),
        }
    }

    pub(super) fn rate_multiplier(self) -> f64 {
        match self {
            Self::Fixed => 1.0,
            Self::TwoClass | Self::Gamma4 => 1.25,
        }
    }

    /// Total mass in baseline-count units. Callers must use lambda directly
    /// for expectation mode and bound the maximum rate after multiplying by q.
    pub(super) fn sample(&self, rng: &mut FastStatsRng, lambda: f64) -> Result<f64, String> {
        let rate = lambda * self.rate_multiplier();
        if !lambda.is_finite() || lambda < 0.0 || rate > (1u64 << 52) as f64 {
            return Err(
                "film grain V3: mass-law rate must be finite, nonnegative and at most 2^52 after compensation"
                    .to_owned(),
            );
        }
        if lambda == 0.0 {
            return Ok(0.0);
        }
        let mass = match self {
            Self::Fixed => super::poisson_exact(rng, lambda)? as f64,
            Self::TwoClass => {
                // Independent Poisson thinning of equal-probability W={0.5,1.5}.
                // q=1.25, so the compensated weights are 0.4 and 1.2.
                let small = super::poisson_exact(rng, 0.625 * lambda)?;
                let large = super::poisson_exact(rng, 0.625 * lambda)?;
                0.4 * small as f64 + 1.2 * large as f64
            }
            Self::Gamma4 => {
                let count = super::poisson_exact(rng, rate)?;
                if count == 0 {
                    return Ok(0.0);
                }
                // W~Gamma(4,1/4), divided by q: conditional total shape 4K,
                // scale 0.2. Sampling one total avoids enumerating events.
                gamma4_total(rng, count)?
            }
        };
        if !mass.is_finite() || mass < 0.0 {
            return Err("film grain V3: sampled dye mass is unrepresentable".to_owned());
        }
        Ok(mass)
    }
}

/// Marsaglia–Tsang for shape 4K >= 4, using the existing cached normal stream.
fn gamma4_total(rng: &mut FastStatsRng, count: u64) -> Result<f64, String> {
    if count == 0 || count >= 1u64 << 53 {
        return Err("film grain V3: conditional Gamma count is unrepresentable".to_owned());
    }
    let d = 4.0 * count as f64 - 1.0 / 3.0;
    let c = 1.0 / (9.0 * d).sqrt();
    loop {
        let x = rng.randn();
        let t = c * x;
        if t <= -1.0 {
            continue;
        }
        let v = (1.0 + t).powi(3);
        let u = rng.rand();
        if u < 1.0 - 0.0331 * x.powi(4) || u.ln() < gamma_log_acceptance(x, t, d) {
            let mass = (0.2 * d) * v;
            if !mass.is_finite() || mass <= 0.0 {
                return Err("film grain V3: conditional Gamma mass is unrepresentable".to_owned());
            }
            return Ok(mass);
        }
    }
}

/// The usual x²/2+d*(1-v+log(v)) cancels quantities of order x².
/// For huge shapes, even forming v-1 loses its small residual. Expanding in
/// t=x/sqrt(9d) cancels the quadratic analytically; the remainder starts at
/// -3t⁴/4. The fixed polynomial on |t|<=0.1 retains that remainder down to
/// the largest supported shape, without switching to a normal approximation.
fn gamma_log_acceptance(x: f64, t: f64, d: f64) -> f64 {
    if t.abs() <= 0.1 {
        let mut coefficient = -3.0 / 18.0;
        for n in (4..18).rev() {
            let sign = if n % 2 == 0 { -1.0 } else { 1.0 };
            coefficient = coefficient * t + sign * 3.0 / n as f64;
        }
        d * t.powi(4) * coefficient
    } else {
        0.5 * x * x + d * (3.0 * t.ln_1p() - t * (3.0 + t * (3.0 + t)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAWS: [MassLaw; 3] = [MassLaw::Fixed, MassLaw::TwoClass, MassLaw::Gamma4];

    // Compound-Poisson cumulants are rate times raw mark moments. This
    // independent oracle also supplies the sampling errors of central moments.
    fn cumulant(law: MassLaw, lambda: f64, order: i32) -> f64 {
        match law {
            MassLaw::Fixed => lambda,
            MassLaw::TwoClass => 0.625 * lambda * (0.4f64.powi(order) + 1.2f64.powi(order)),
            MassLaw::Gamma4 => {
                let rising: f64 = (0..order).map(|i| (4 + i) as f64).product();
                1.25 * lambda * 0.2f64.powi(order) * rising
            }
        }
    }

    fn check_moments(law: MassLaw, lambda: f64, samples: u64, third: bool) {
        let mut rng = FastStatsRng::new(0x51A7_2026);
        let mut sums = [0.0; 3];
        let mut zeros = 0u64;
        for _ in 0..samples {
            let value = law.sample(&mut rng, lambda).unwrap();
            assert!(value.is_finite() && value >= 0.0);
            zeros += u64::from(value == 0.0);
            let delta = value - lambda;
            sums[0] += delta;
            sums[1] += delta * delta;
            sums[2] += delta * delta * delta;
        }
        let k2 = cumulant(law, lambda, 2);
        let k3 = cumulant(law, lambda, 3);
        let k4 = cumulant(law, lambda, 4);
        let mu4 = k4 + 3.0 * k2 * k2;
        let mu6 = cumulant(law, lambda, 6) + 15.0 * k4 * k2 + 10.0 * k3 * k3 + 15.0 * k2.powi(3);
        let n = samples as f64;
        let expected = [0.0, k2, k3];
        let errors = [
            (k2 / n).sqrt(),
            ((mu4 - k2 * k2) / n).sqrt(),
            ((mu6 - k3 * k3) / n).sqrt(),
        ];
        for moment in 0..if third { 3 } else { 2 } {
            let actual = sums[moment] / n;
            assert!(
                (actual - expected[moment]).abs() <= 5.5 * errors[moment] + 1e-12,
                "{law:?}, lambda={lambda}, moment={}: {actual} vs {} (SE {})",
                moment + 1,
                expected[moment],
                errors[moment]
            );
        }
        let p0 = (-lambda * law.rate_multiplier()).exp();
        let zero_error = (p0 * (1.0 - p0) / n).sqrt();
        assert!(
            (zeros as f64 / n - p0).abs() <= 5.5 * zero_error + 1.0 / n,
            "{law:?}, lambda={lambda}: zero fraction {} vs {p0}",
            zeros as f64 / n
        );
    }

    #[test]
    fn compensated_laws_preserve_two_moments_and_change_third_and_zeros() {
        for law in LAWS {
            for lambda in [0.35, 2.0] {
                check_moments(law, lambda, 600_000, true);
            }
        }
    }

    #[test]
    fn tiny_and_million_scale_rates_retain_mean_and_variance() {
        for law in LAWS {
            check_moments(law, 0.0001, 1_000_000, true);
            check_moments(law, 1_000_000.0, 100_000, true);
        }
    }

    #[test]
    fn exact_poisson_dispatch_boundaries_retain_moments() {
        // Fixed crosses at lambda=30, Gamma at 24, each two-class count at 48.
        for (law, boundary) in [
            (MassLaw::Fixed, 30.0),
            (MassLaw::Gamma4, 24.0),
            (MassLaw::TwoClass, 48.0),
        ] {
            for lambda in [boundary - 0.001, boundary, boundary + 0.001] {
                check_moments(law, lambda, 100_000, true);
            }
        }
    }

    #[test]
    fn fixed_replays_baseline_and_all_laws_replay_seeded_streams() {
        for lambda in [0.0, 0.0001, 1.0, 29.999, 30.0, 30.001, 1_000_000.0] {
            let mut baseline = FastStatsRng::new(0xB453);
            let mut fixed = FastStatsRng::new(0xB453);
            for _ in 0..100 {
                assert_eq!(
                    MassLaw::Fixed.sample(&mut fixed, lambda).unwrap(),
                    super::super::poisson_exact(&mut baseline, lambda).unwrap() as f64
                );
            }
            for law in LAWS {
                let mut first = FastStatsRng::stream(0x5EED, 2, 73);
                let mut replay = FastStatsRng::stream(0x5EED, 2, 73);
                for _ in 0..100 {
                    assert_eq!(
                        law.sample(&mut first, lambda).unwrap().to_bits(),
                        law.sample(&mut replay, lambda).unwrap().to_bits()
                    );
                }
            }
        }
    }

    #[test]
    fn zero_rates_do_not_consume_randomness() {
        for law in LAWS {
            let mut rng = FastStatsRng::new(71);
            let mut untouched = FastStatsRng::new(71);
            assert_eq!(law.sample(&mut rng, 0.0).unwrap(), 0.0);
            assert_eq!(rng.rand(), untouched.rand());
        }
    }

    #[test]
    fn invalid_inputs_fail_without_clipping_or_fallback() {
        for law in LAWS {
            for lambda in [-1.0, f64::NAN, f64::INFINITY, f64::MAX] {
                assert!(law.sample(&mut FastStatsRng::new(1), lambda).is_err());
            }
            let maximum = (1u64 << 52) as f64 / law.rate_multiplier();
            assert!(
                law.sample(&mut FastStatsRng::new(1), maximum * 1.001)
                    .is_err()
            );
        }
        assert!(gamma4_total(&mut FastStatsRng::new(1), 0).is_err());
        assert!(gamma4_total(&mut FastStatsRng::new(1), 1u64 << 53).is_err());
    }

    #[test]
    fn large_shape_acceptance_keeps_the_small_negative_residual() {
        for d in [4_999_999.666_666_667, 4.0 * (1u64 << 52) as f64] {
            for x in [-3.0f64, -1.0, 1.0, 3.0] {
                let t = x / (9.0 * d).sqrt();
                let actual = gamma_log_acceptance(x, t, d);
                let leading = -x.powi(4) / (108.0 * d);
                assert!(actual < 0.0 && actual.is_finite());
                assert!((actual / leading - 1.0).abs() < 0.002);
            }
        }
        // Conditional sampling at the count limit remains finite and has the
        // correct shape-scaled mean/variance, rather than a constant rounded mean.
        let count = (1u64 << 52) - 1;
        let mean = 0.8 * count as f64;
        let variance = 0.16 * count as f64;
        let mut rng = FastStatsRng::new(0x6A44A);
        let n = 50_000;
        let mut first = 0.0;
        let mut second = 0.0;
        for _ in 0..n {
            let delta = gamma4_total(&mut rng, count).unwrap() - mean;
            first += delta;
            second += delta * delta;
        }
        assert!((first / n as f64).abs() < 5.5 * (variance / n as f64).sqrt());
        assert!((second / n as f64 / variance - 1.0).abs() < 0.04);
    }

    #[test]
    fn selector_is_strict_without_mutating_the_environment() {
        assert_eq!(MassLaw::parse("fixed").unwrap(), MassLaw::Fixed);
        assert_eq!(MassLaw::parse("two_class").unwrap(), MassLaw::TwoClass);
        assert_eq!(MassLaw::parse("gamma4").unwrap(), MassLaw::Gamma4);
        for value in ["", "gamma", "Fixed", " gamma4", "gamma4 "] {
            assert!(MassLaw::parse(value).is_err());
        }
        assert_eq!(MassLaw::Fixed.rate_multiplier(), 1.0);
        assert_eq!(MassLaw::TwoClass.rate_multiplier(), 1.25);
        assert_eq!(MassLaw::Gamma4.rate_multiplier(), 1.25);
    }
}
