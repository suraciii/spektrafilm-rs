//! Frozen Exposure-compatible value-noise tables and control coefficients.
//!
//! This is numerical primitives only. The model crate owns the encoded-RGB
//! appearance composition and the core crate owns serialization.
pub const SPECTRAL_LEN: usize = 516;
pub const PERMUTATION_LEN: usize = 8192;
const MINSTD_MODULUS: u32 = 2_147_483_647;
const MINSTD_A: u32 = 16_807;
const MINSTD_Q: u32 = 127_773;
const MINSTD_R: u32 = 2_836;
const SEED_LIMIT: u32 = 0x7fff_fffe;
const OCTAVE_OFFSETS: [f32; 8] = [23.1, 8.15, 57.0, 69.2, 117.4, 138.6, 198.6, 278.85];

/// Reduce the shared runtime seed to the frozen reference's nonnegative
/// 31-bit domain.
pub fn exposure_seed(seed: u64) -> u32 {
    ((seed & 0x7fff_ffff) as u32).min(SEED_LIMIT)
}

/// Control coefficients resolved from one Exposure request.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExposureCoefficients {
    pub size_low: f32,
    pub size_high: f32,
    pub shadow_amplitude: f32,
    pub midtone_amplitude: f32,
    pub highlight_amplitude: f32,
    pub frequency_gain: f32,
    pub roughness_effective: f32,
}

/// Resolve the automatic or manual size pair and the zone/frequency gains.
#[allow(clippy::too_many_arguments)]
pub fn exposure_effective_size(
    auto_size: bool,
    size: f32,
    format_mm: f32,
    absolute_size_px: f32,
    push: f32,
    render_scale: f32,
    width: u32,
    height: u32,
) -> (f32, f32) {
    let size = if auto_size {
        (width.min(height) as f32 / format_mm / 125.0 * (size * 2.5)).max(1.5)
    } else {
        absolute_size_px
    };
    let x = (1.0 + 0.4 * push / 3.0) * size / render_scale.max(f32::MIN_POSITIVE);
    (x.min(1.0), x.max(1.0))
}

/// Resolve all image-independent control coefficients.
#[allow(clippy::too_many_arguments)]
pub fn exposure_coefficients(
    auto_size: bool,
    size: f32,
    format_mm: f32,
    absolute_size_px: f32,
    strength: f32,
    roughness: f32,
    shadows: f32,
    midtones: f32,
    highlights: f32,
    push: f32,
    render_scale: f32,
    width: u32,
    height: u32,
) -> ExposureCoefficients {
    let (size_low, size_high) = exposure_effective_size(
        auto_size,
        size,
        format_mm,
        absolute_size_px,
        push,
        render_scale,
        width,
        height,
    );
    let amplitude = strength / 100.0;
    let roughness_effective = (0.01 * roughness + 0.3 * push / 3.0).clamp(0.0, 1.0);
    ExposureCoefficients {
        size_low,
        size_high,
        shadow_amplitude: amplitude * zone_gain(shadows, push) * size_low,
        midtone_amplitude: amplitude * zone_gain(midtones, push) * size_low,
        highlight_amplitude: amplitude * zone_gain(highlights, push) * size_low,
        frequency_gain: 0.3 / (2.0 * (0.3 * roughness_effective + 0.35)),
        roughness_effective,
    }
}

/// Fully prepared, image-independent tables for one realization.
#[derive(Debug, Clone)]
pub struct ExposureTables {
    pub t1: Vec<f32>,
    pub t2: Vec<f32>,
    pub t3: Vec<f32>,
    pub perm: Vec<u32>,
    pub perm2: Vec<u32>,
    pub weights: Vec<f32>,
    pub fx: f32,
    pub fy: f32,
    pub amplitude: f32,
}

impl ExposureTables {
    pub fn new(seed: u32, size_low: f32, size_high: f32) -> Self {
        let seed = seed.min(SEED_LIMIT);
        let amplitude = size_low.abs() as f64;
        let mut state = minstd_seed(seed);
        let mut t1 = vec![0.0f32; SPECTRAL_LEN];
        let mut t2 = vec![0.0f32; SPECTRAL_LEN];
        let mut t3 = vec![0.0f32; SPECTRAL_LEN];
        for k in 0..(SPECTRAL_LEN / 2) {
            let (next, unit) = minstd_unit(state);
            state = next;
            let theta = std::f64::consts::TAU * unit;
            let (sin, cos) = theta.sin_cos();
            t1[2 * k] = (cos * amplitude) as f32;
            t1[2 * k + 1] = (sin * amplitude) as f32;
            t2[2 * k] = (cos * amplitude) as f32;
            t2[2 * k + 1] = (sin * amplitude) as f32;
            t3[2 * k] = (sin * amplitude * 0.5) as f32;
            t3[2 * k + 1] = (cos * amplitude * 0.5) as f32;
        }
        // The reconstructed Fisher-Yates bound is deliberately N - 1.
        for i in 0..SPECTRAL_LEN {
            let (next, j) = minstd_int(state, (SPECTRAL_LEN - 1) as u32);
            state = next;
            let j = j as usize;
            t1.swap(i, j);
            t2.swap(i, j);
            t3.swap(i, j);
        }

        let mut perm_state = minstd_seed(seed);
        let mut perm: Vec<u32> = (0..PERMUTATION_LEN).map(|i| (i & 511) as u32).collect();
        let mut perm2: Vec<u32> = (0..PERMUTATION_LEN).map(|i| i as u32).collect();
        for i in 0..PERMUTATION_LEN {
            let (next, j) = minstd_int(perm_state, PERMUTATION_LEN as u32);
            perm_state = next;
            let j = j as usize;
            perm.swap(i, j);
            perm2.swap(i, j);
        }

        let p = 0.5f64;
        let mut weights = [1.0f64, p, p * p, p * p * (p + 1.0), p * p * p * (p + 1.0)];
        let total: f64 = weights.iter().sum();
        for weight in &mut weights {
            *weight /= total;
        }
        let alive = weights.iter().filter(|weight| **weight >= 0.03).count();
        let count = alive.min(weights.len());
        let renorm: f64 = weights[..count].iter().sum();
        let weights: Vec<f32> = weights[..count]
            .iter()
            .map(|weight| (weight / renorm) as f32)
            .collect();

        Self {
            t1,
            t2,
            t3,
            perm,
            perm2,
            weights,
            fx: 1.0 / size_low,
            fy: 1.0 / size_high,
            amplitude: size_low.abs(),
        }
    }

    fn lattice(&self, ix: i32, iy: i32) -> [f32; 4] {
        let h0 = self.perm2[(ix & 8191) as usize];
        let h1 = self.perm2[((ix + 1) & 8191) as usize];
        let i0 = (iy & 8191) as u32;
        let i1 = ((iy + 1) & 8191) as u32;
        let c00 = self.perm[(h0 ^ i0) as usize] as usize;
        let c01 = self.perm[(h0 ^ i1) as usize] as usize;
        let c10 = self.perm[(h1 ^ i0) as usize] as usize;
        let c11 = self.perm[(h1 ^ i1) as usize] as usize;
        [
            self.t1[c00] + self.t3[c00] + self.t2[c00],
            self.t1[c01] + self.t3[c01] + self.t2[c01],
            self.t1[c10] + self.t3[c10] + self.t2[c10],
            self.t1[c11] + self.t3[c11] + self.t2[c11],
        ]
    }

    pub fn noise(&self, x: f32, y: f32) -> f32 {
        let mut acc = 0.0f32;
        for (octave, &weight) in self.weights.iter().enumerate() {
            let x_frequency = (1u32 << octave) as f32;
            let px = x * self.fx * x_frequency + OCTAVE_OFFSETS[octave];
            let py = y * self.fy + OCTAVE_OFFSETS[octave];
            let ix = px.floor();
            let iy = py.floor();
            let tx = smoothstep(px - ix);
            let ty = smoothstep(py - iy);
            let [v00, v01, v10, v11] = self.lattice(ix as i32, iy as i32);
            let a = v00 + (v10 - v00) * tx;
            let b = v01 + (v11 - v01) * tx;
            acc += weight * (a + (b - a) * ty);
        }
        acc.clamp(-self.amplitude, self.amplitude)
    }
}

fn minstd_seed(seed: u32) -> u32 {
    minstd_step(seed.min(SEED_LIMIT) + 1)
}

fn minstd_step(state: u32) -> u32 {
    let q = state / MINSTD_Q;
    let r = state % MINSTD_Q;
    let value = i64::from(MINSTD_A * r) - i64::from(MINSTD_R * q);
    if value <= 0 {
        (value + i64::from(MINSTD_MODULUS)) as u32
    } else {
        value as u32
    }
}

fn minstd_int(state: u32, bound: u32) -> (u32, u32) {
    let state = minstd_step(state);
    (state, state % bound)
}

fn minstd_unit(state: u32) -> (u32, f64) {
    let state = minstd_step(state);
    (state, f64::from(state) / f64::from(MINSTD_MODULUS))
}

#[inline]
fn smoothstep(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    (3.0 - 2.0 * value) * value * value
}

#[inline]
fn zone_gain(value: f32, push: f32) -> f32 {
    (0.01 * value + 0.2 * push / 3.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_size_resolves_auto_manual_push_and_render_scale() {
        // Auto uses the short edge of the output raster and clamps at 1.5 px.
        let (low, high) = exposure_effective_size(true, 1.0, 24.0, 1.5, 0.0, 1.0, 1200, 1600);
        assert_eq!((low, high), (1.0, 1.5));
        let (low, high) = exposure_effective_size(true, 1.0, 24.0, 1.5, 0.0, 1.0, 8000, 12000);
        assert!((low - 1.0).abs() < 1e-6);
        assert!((high - 6.666_666_5).abs() < 1e-5);

        // Manual size ignores the format and is split around the 1 px pivot.
        let (low, high) = exposure_effective_size(false, 4.0, 101.6, 2.0, 0.0, 1.0, 16, 16);
        assert_eq!((low, high), (1.0, 2.0));

        // Push scales coarser; render scale divides the result for a reduced raster.
        let (low, high) = exposure_effective_size(false, 4.0, 101.6, 2.0, 3.0, 1.0, 16, 16);
        assert!((low - 1.0).abs() < 1e-6);
        assert!((high - 2.8).abs() < 1e-6);
        let (low, high) = exposure_effective_size(false, 4.0, 101.6, 2.0, 3.0, 2.0, 16, 16);
        assert!((low - 1.0).abs() < 1e-6);
        assert!((high - 1.4).abs() < 1e-6);
    }

    #[test]
    fn seed_reduction_uses_low_31_bits_and_clamps() {
        assert_eq!(exposure_seed(0), 0);
        assert_eq!(exposure_seed(0x8000_0001), 1);
        assert_eq!(exposure_seed(u64::MAX), SEED_LIMIT);
        assert_eq!(exposure_seed(7), 7);
    }

    #[test]
    fn table_generation_is_stable_for_a_fixed_seed_and_size() {
        let tables = ExposureTables::new(7, 1.0, 1.0);
        assert_eq!(tables.t1.len(), SPECTRAL_LEN);
        assert_eq!(tables.perm2.len(), PERMUTATION_LEN);
        assert_eq!(tables.weights.len(), 5);
        assert!((tables.weights.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        let first = tables.noise(0.0, 0.0);
        assert!(first.is_finite());
        assert_eq!(first, ExposureTables::new(7, 1.0, 1.0).noise(0.0, 0.0));
    }
}
