//! Isotropic value-noise field for the Dye grain law.
//!
//! The field is one coordinate-addressed smoothstep value-noise lattice. The
//! profile's density layers, not an octave stack, carry the multi-layer
//! structure. This keeps one correlation scale per record and avoids the
//! low-frequency lobes of a filtered field.
//!
//! The realization is deterministic at a film coordinate. The same hash,
//! stream, cell size and normalization are evaluated by the CPU reference
//! and the device field pass, so a crop or band split samples one film.

use rayon::prelude::*;
use spektrafilm_backend::dye_params::law;
use spektrafilm_math::precision::{Scalar, ZERO, from_f64, to_f64};

fn hash(mut x: u32, y: u32, seed: u32) -> f64 {
    x = x
        .wrapping_mul(0x9e37_79b1)
        .wrapping_add(y.wrapping_mul(0x85eb_ca6b))
        .wrapping_add(seed.wrapping_mul(0xc2b2_ae35));
    x ^= x >> 15;
    x = x.wrapping_mul(0x2c1b_3c6d);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297a_2d39);
    x ^= x >> 15;
    (x as f64) * (2.0 / 4_294_967_295.0) - 1.0
}

fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn value_noise(x: f64, y: f64, seed: u32) -> f64 {
    let ix = x.floor();
    let iy = y.floor();
    let fx = smoothstep(x - ix);
    let fy = smoothstep(y - iy);
    let x0 = ix as i64 as u32;
    let y0 = iy as i64 as u32;
    let x1 = (ix as i64 + 1) as u32;
    let y1 = (iy as i64 + 1) as u32;
    let a = hash(x0, y0, seed);
    let b = hash(x1, y0, seed);
    let c = hash(x0, y1, seed);
    let d = hash(x1, y1, seed);
    let top = a + (b - a) * fx;
    let bottom = c + (d - c) * fx;
    top + (bottom - top) * fy
}

/// Build one zero-mean, unit-variance field on the source film lattice.
///
/// `stream` selects the shared field (`0`) or one record's own field
/// (`c + 1`). `scale` is the recipe's relative correlation length.
fn field_one(w: usize, h: usize, pitch_um: f64, scale: f64, seed: u64, stream: u32) -> Vec<Scalar> {
    let seed = (seed as u32) ^ stream.wrapping_mul(0x9e37_79b9);
    let base_px = law::base_cell_px(scale, pitch_um);
    let gain = law::VALUE_FIELD_SCALE;
    let mut out = vec![ZERO; w * h];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, cell) in row.iter_mut().enumerate() {
            let value = value_noise(x as f64 / base_px, y as f64 / base_px, seed);
            *cell = from_f64(value * gain);
        }
    });
    out
}

#[cfg(test)]
pub(crate) fn field(
    w: usize,
    h: usize,
    pitch_um: f64,
    scale: f64,
    seed: u64,
    stream: u32,
) -> Vec<Scalar> {
    field_one(w, h, pitch_um, scale, seed, stream)
}

/// One unit-variance field with an optional microcontrast component.
/// The two components are independent and mixed by variance: the base keeps
/// `1 - fine_weight`, the fine component carries `fine_weight`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn field_with_fine(
    w: usize,
    h: usize,
    pitch_um: f64,
    scale: f64,
    seed: u64,
    stream: u32,
    fine_scale: f64,
    fine_weight: f64,
) -> Vec<Scalar> {
    let mut base = field_one(w, h, pitch_um, scale, seed, stream);
    if fine_weight <= 0.0 {
        return base;
    }
    let fine_seed = (seed as u32) ^ (stream ^ 0x4000_0000).wrapping_mul(0x9e37_79b9);
    let fine_base_px = law::base_cell_px(scale * fine_scale, pitch_um);
    let gain = law::VALUE_FIELD_SCALE;
    let a = (1.0 - fine_weight).sqrt();
    let b = fine_weight.sqrt();
    base.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, cell) in row.iter_mut().enumerate() {
            let fine =
                value_noise(x as f64 / fine_base_px, y as f64 / fine_base_px, fine_seed) * gain;
            *cell = from_f64(a * to_f64(*cell) + b * fine);
        }
    });
    base
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_math::precision::to_f64;

    #[test]
    fn field_is_near_unit_variance() {
        let v = field(256, 256, 4.86, 1.0, 7, 0);
        let mean = v.iter().map(|&x| to_f64(x)).sum::<f64>() / v.len() as f64;
        let var = v.iter().map(|&x| (to_f64(x) - mean).powi(2)).sum::<f64>() / v.len() as f64;
        assert!((mean).abs() < 0.02, "mean {mean}");
        assert!((var.sqrt() - 1.0).abs() < 0.03, "sd {}", var.sqrt());
    }

    fn variance_at(pitch_um: f64, scale: f64) -> (f64, f64) {
        let v = field(256, 256, pitch_um, scale, 7, 0);
        let mean = v.iter().map(|&x| to_f64(x)).sum::<f64>() / v.len() as f64;
        let var = v.iter().map(|&x| (to_f64(x) - mean).powi(2)).sum::<f64>() / v.len() as f64;
        (mean, var.sqrt())
    }

    #[test]
    fn commensurate_cell_sizes_keep_unit_variance() {
        for (pitch_um, scale) in [
            (7.3, 1.0),
            (7.3 / 0.4, 1.0),
            (7.3, 2.0),
            (7.3, 4.0),
            (14.6, 1.0),
            (3.65, 1.0),
            (7.3 / 2.001, 1.0),
        ] {
            let (mean, sd) = variance_at(pitch_um, scale);
            assert!(
                (mean).abs() < 0.03,
                "pitch {pitch_um} scale {scale} mean {mean}"
            );
            assert!(
                (sd - 1.0).abs() < 0.08,
                "pitch {pitch_um} scale {scale} sd {sd}"
            );
        }
    }

    #[test]
    fn microcontrast_keeps_unit_variance() {
        let v = field_with_fine(256, 256, 4.86, 1.0, 7, 0, 0.5, 0.35);
        let mean = v.iter().map(|&x| to_f64(x)).sum::<f64>() / v.len() as f64;
        let var = v.iter().map(|&x| (to_f64(x) - mean).powi(2)).sum::<f64>() / v.len() as f64;
        assert!((mean).abs() < 0.03, "mean {mean}");
        assert!((var.sqrt() - 1.0).abs() < 0.05, "sd {}", var.sqrt());
    }

    #[test]
    fn same_seed_replays() {
        assert_eq!(field(48, 48, 5.0, 1.0, 7, 0), field(48, 48, 5.0, 1.0, 7, 0));
    }
}
