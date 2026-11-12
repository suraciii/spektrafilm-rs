//! Isotropic value-noise field for the Dye grain law.
//!
//! The field is a coordinate-addressed three-octave value-noise sum with
//! smoothstep interpolation. It replaces the Gaussian-filtered field: the
//! octave weights, not a high-pass, control the large-scale content, and the
//! process has no low-frequency Gaussian lobes to read as dark blotches.
//!
//! The realization is deterministic at a film coordinate. The same hash,
//! stream, octave sizes and normalization are evaluated by the CPU reference
//! and the device field pass, so a crop or band split samples one film.

use rayon::prelude::*;
use spektrafilm_backend::dye_params::law;
use spektrafilm_math::precision::{Scalar, ZERO, from_f64};

/// Offset between octave lattices, in cells. It keeps the three octaves from
/// sharing lattice corners without changing the field's stationarity.
const OCTAVE_OFFSET: f64 = 37.0;

fn hash(mut x: u32, y: u32, seed: u32, octave: u32) -> f64 {
    x = x
        .wrapping_mul(0x9e37_79b1)
        .wrapping_add(y.wrapping_mul(0x85eb_ca6b))
        .wrapping_add(seed.wrapping_mul(0xc2b2_ae35))
        .wrapping_add(octave.wrapping_mul(0x27d4_eb2f));
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

fn value_noise(x: f64, y: f64, seed: u32, octave: u32) -> f64 {
    let ix = x.floor();
    let iy = y.floor();
    let fx = smoothstep(x - ix);
    let fy = smoothstep(y - iy);
    let x0 = ix as i64 as u32;
    let y0 = iy as i64 as u32;
    let x1 = (ix as i64 + 1) as u32;
    let y1 = (iy as i64 + 1) as u32;
    let a = hash(x0, y0, seed, octave);
    let b = hash(x1, y0, seed, octave);
    let c = hash(x0, y1, seed, octave);
    let d = hash(x1, y1, seed, octave);
    let top = a + (b - a) * fx;
    let bottom = c + (d - c) * fx;
    top + (bottom - top) * fy
}

/// Build one zero-mean, unit-variance field on the source film lattice.
///
/// `stream` selects the shared field (`0`) or one record's own field
/// (`c + 1`). `scale` is the recipe's relative correlation length.
pub(crate) fn field(
    w: usize,
    h: usize,
    pitch_um: f64,
    scale: f64,
    seed: u64,
    stream: u32,
) -> Vec<Scalar> {
    let seed = (seed as u32) ^ stream.wrapping_mul(0x9e37_79b9);
    let base_px = law::base_cell_px(scale, pitch_um);
    let norm = law::VALUE_WEIGHTS.iter().map(|w| w * w).sum::<f64>().sqrt();
    let gain = law::VALUE_FIELD_SCALE / norm;
    let mut out = vec![ZERO; w * h];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, cell) in row.iter_mut().enumerate() {
            let mut sum = 0.0;
            for (octave, weight) in law::VALUE_WEIGHTS.iter().enumerate() {
                let size = base_px / (1u32 << octave) as f64;
                let offset = octave as f64 * OCTAVE_OFFSET;
                sum += weight
                    * value_noise(
                        x as f64 / size + offset,
                        y as f64 / size + offset,
                        seed,
                        octave as u32,
                    );
            }
            *cell = from_f64(sum * gain);
        }
    });
    out
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
    fn same_seed_replays() {
        assert_eq!(field(48, 48, 5.0, 1.0, 7, 0), field(48, 48, 5.0, 1.0, 7, 0));
    }
}
