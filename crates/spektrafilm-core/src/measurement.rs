//! Measurements of characteristic density curves using SciPy's not-a-knot boundary.

struct Spline {
    x: Vec<f64>,
    y: Vec<f64>,
    second: Vec<f64>,
}

impl Spline {
    fn new(x: Vec<f64>, y: Vec<f64>) -> Result<Self, String> {
        let n = x.len();
        if n < 2 || n != y.len() || x.iter().chain(&y).any(|v| !v.is_finite()) {
            return Err("spline requires matching finite samples".into());
        }
        let h: Vec<_> = x.windows(2).map(|p| p[1] - p[0]).collect();
        if h.iter().any(|v| *v <= 0.0) {
            return Err("spline abscissas must be strictly increasing".into());
        }
        let mut second = vec![0.0; n];
        if n == 3 {
            let q = 2.0 * ((y[2] - y[1]) / h[1] - (y[1] - y[0]) / h[0]) / (h[0] + h[1]);
            second.fill(q);
        } else if n >= 4 {
            let k = n - 2;
            let mut lower = vec![0.0; k];
            let mut diagonal = vec![0.0; k];
            let mut upper = vec![0.0; k];
            let mut rhs = vec![0.0; k];
            for j in 0..k {
                let i = j + 1;
                lower[j] = h[i - 1];
                diagonal[j] = 2.0 * (h[i - 1] + h[i]);
                upper[j] = h[i];
                rhs[j] = 6.0 * ((y[i + 1] - y[i]) / h[i] - (y[i] - y[i - 1]) / h[i - 1]);
            }
            diagonal[0] += h[0] * (h[0] + h[1]) / h[1];
            upper[0] -= h[0] * h[0] / h[1];
            diagonal[k - 1] += h[n - 2] * (h[n - 3] + h[n - 2]) / h[n - 3];
            lower[k - 1] -= h[n - 2] * h[n - 2] / h[n - 3];
            for j in 1..k {
                let factor = lower[j] / diagonal[j - 1];
                diagonal[j] -= factor * upper[j - 1];
                rhs[j] -= factor * rhs[j - 1];
            }
            second[n - 2] = rhs[k - 1] / diagonal[k - 1];
            for j in (0..k - 1).rev() {
                second[j + 1] = (rhs[j] - upper[j] * second[j + 2]) / diagonal[j];
            }
            second[0] = ((h[0] + h[1]) * second[1] - h[0] * second[2]) / h[1];
            second[n - 1] = ((h[n - 3] + h[n - 2]) * second[n - 2] - h[n - 2] * second[n - 3]) / h[n - 3];
        }
        Ok(Self { x, y, second })
    }

    fn at(&self, x: f64) -> f64 {
        let i = self.x.partition_point(|v| *v <= x).saturating_sub(1).min(self.x.len() - 2);
        let h = self.x[i + 1] - self.x[i];
        let a = (self.x[i + 1] - x) / h;
        let b = (x - self.x[i]) / h;
        a * self.y[i] + b * self.y[i + 1]
            + ((a * a * a - a) * self.second[i] + (b * b * b - b) * self.second[i + 1]) * h * h / 6.0
    }
}

/// Invert each density channel with cubic interpolation between two density levels.
/// Like upstream interp1d, sorts density samples and rejects out-of-range queries.
pub fn measure_gamma(exposure: &[f64], density: &[[f64; 3]], d0: f64, d1: f64) -> Result<[f64; 3], String> {
    if exposure.len() != density.len() || !d0.is_finite() || !d1.is_finite() {
        return Err("invalid gamma samples or density levels".into());
    }
    let mut result = [0.0; 3];
    for c in 0..3 {
        let mut pairs: Vec<_> = density.iter().zip(exposure).map(|(d, e)| (d[c], *e)).collect();
        if pairs.len() < 4 || pairs.iter().any(|(x, y)| !x.is_finite() || !y.is_finite()) {
            return Err("cubic gamma inversion requires four finite samples".into());
        }
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (x, y) = pairs.into_iter().unzip();
        let spline = Spline::new(x, y)?;
        for d in [d0, d1] {
            if d < spline.x[0] || d > *spline.x.last().unwrap() {
                return Err("gamma density is outside sampled range".into());
            }
        }
        result[c] = (d1 - d0) / (spline.at(d1) - spline.at(d0));
        if !result[c].is_finite() { return Err("degenerate gamma interval".into()); }
    }
    Ok(result)
}

/// Measure slopes over an exposure interval, excluding NaN density samples.
/// Endpoint extrapolation follows upstream CubicSpline.
pub fn measure_slopes_at_exposure(exposure: &[f64], density: &[[f64; 3]], reference: f64, range: f64) -> Result<[f64; 3], String> {
    if exposure.len() != density.len() || !reference.is_finite() || !range.is_finite() || range == 0.0 {
        return Err("invalid slope samples or exposure range".into());
    }
    let mut result = [0.0; 3];
    for c in 0..3 {
        let (x, y) = exposure.iter().zip(density).filter(|(_, d)| !d[c].is_nan()).map(|(e, d)| (*e, d[c])).unzip();
        let spline = Spline::new(x, y)?;
        result[c] = (spline.at(reference + range / 2.0) - spline.at(reference - range / 2.0)) / range;
        if !result[c].is_finite() { return Err("degenerate slope interval".into()); }
    }
    Ok(result)
}
