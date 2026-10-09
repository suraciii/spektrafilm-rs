//! Numerical primitives shared by Grain V2 host paths.

use rand_mt::Mt;

/// Build the folded Gaussian weights used by Dehancer's FastBlur line pass.
///
/// Each pair is `[weight, offset]`; the pair is sampled in both directions,
/// so the weights sum to one half. The Gaussian is generated and normalized
/// in-place, then folded directly without a second half-kernel allocation.
pub fn fast_blur_weights(radius: f32) -> Vec<[f32; 2]> {
    if radius <= 0.0 {
        return vec![[0.5, 0.0]];
    }

    let sigma = 2.0 * radius / 3.0;
    let n = (4.0 * sigma.ceil() - 1.0) as usize;
    let c = n / 2;

    // One temporary only: retain the reference's f32 generation and
    // normalization order, then fold by indexing the normalized kernel.
    let mut kernel = Vec::with_capacity(n);
    let mut sum = 0.0f32;
    for i in 0..n {
        let value = (-0.5 * ((i as f32 - c as f32) / sigma).powi(2)).exp();
        kernel.push(value);
        sum += value;
    }
    for value in &mut kernel {
        *value /= sum;
    }

    let samples = (c + 1) / 2;
    let mut folded = Vec::with_capacity(samples);
    for i in 0..samples {
        let a = if i == 0 {
            kernel[c] * 0.5
        } else {
            kernel[c - 2 * i]
        };
        let b = kernel[c - (2 * i + 1)];
        let weight = a + b;
        // The normal path has positive Gaussian weights. Keep the reference
        // offset well-defined even if underflow produces a zero pair.
        let offset = if weight == 0.0 {
            2.0 * i as f32
        } else {
            2.0 * i as f32 + b / weight
        };
        folded.push([weight, offset]);
    }
    folded
}

/// Build Dehancer's OpticalResolution magic-resampler kernel.
pub fn optical_weights(radius: f32) -> Vec<f32> {
    if radius <= 0.0 {
        return vec![1.0];
    }

    let c = (1.5f32 * radius).ceil().max(4.0) as i32;
    let k = ((c as f32) * 0.5).ceil() as i32;
    let adj = ((k & 1) ^ 5) + k;
    let mut weights = Vec::with_capacity((2 * adj + 1) as usize);
    let mut sum = 0.0f32;
    for i in -adj..=adj {
        let x = i as f32 / radius;
        let ax = x.abs();
        let weight = if ax >= 1.5 {
            0.0
        } else if ax > 0.5 {
            0.5 * (ax - 1.5) * (ax - 1.5)
        } else {
            1.33333 - x * x
        };
        weights.push(weight);
        sum += weight;
    }
    for weight in &mut weights {
        *weight /= sum;
    }
    weights
}

/// Generate the host's `generate_canonical<double, 53>` value as an f32.
pub fn next_phase(rng: &mut Mt) -> f32 {
    const R: f64 = 4_294_967_296.0;
    let mut total = 0.0f64;
    let mut tmp = 1.0f64;
    let mut range = 1.0f64;
    for _ in 0..2 {
        total += rng.next_u32() as f64 * tmp;
        tmp *= R;
        range *= R;
    }
    let mut value = total / range;
    if value >= 1.0 {
        value = f64::from_bits(0x3FEF_FFFF_FFFF_FFFF);
    }
    value as f32
}

/// Generate a canonical phase from one freshly seeded MT19937 instance.
pub fn seeded_phase(seed: u32) -> f32 {
    let mut rng = Mt::new(seed);
    next_phase(&mut rng)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_phase_matches_libstdcpp_bits() {
        for (seed, bits) in [
            (5489, 0x3e0aba7c),
            (0xdeadbeef, 0x3f65052f),
            (1, 0x3f7f4781),
            (0x2a2a2a2a, 0x3e8fd877),
            (0x01352890, 0x3f51bf49),
        ] {
            assert_eq!(seeded_phase(seed).to_bits(), bits, "seed={seed}");
        }
    }

    #[test]
    fn recovered_filter_weight_anchors() {
        let gaussian = fast_blur_weights(4.0);
        for (actual, expected) in gaussian.iter().zip([
            [0.222714234, 0.650862978],
            [0.199983006, 2.413003570],
            [0.077302760, 4.346873086],
        ]) {
            assert!((actual[0] - expected[0]).abs() < 1e-6);
            assert!((actual[1] - expected[1]).abs() < 1e-6);
        }
        assert_eq!(gaussian.len(), 3);
        let optical = optical_weights(1.0);
        assert!((optical[7] - 0.842104931).abs() < 1e-6);
        assert!((optical[6] - 0.078947535).abs() < 1e-6);
        for radius in [0.0, 0.1, 0.5, 1.0, 2.0, 4.0, 24.0] {
            assert!((fast_blur_weights(radius).iter().map(|p| p[0]).sum::<f32>() - 0.5).abs() < 1e-6);
            assert!((optical_weights(radius).iter().sum::<f32>() - 1.0).abs() < 1e-6);
        }
        let narrow = optical_weights(0.5);
        assert_eq!(narrow[7], 1.0);
        assert!(narrow.iter().enumerate().all(|(i, &v)| i == 7 || v == 0.0));
    }
}
