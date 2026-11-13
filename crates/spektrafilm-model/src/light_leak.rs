//! Camera light leak: stray light that reaches a film record during exposure.
//!
//! A leak enters through a seal gap — a camera-back gasket, a door hinge, a
//! film-door edge, a failing light trap — and exposes the record alongside the
//! image. It is therefore an additive term on **record exposure**, evaluated
//! before development, and never a display-domain overlay.
//!
//! Geometry: one aperture segment on the frame perimeter, spread inward with a
//! compact falloff whose length is measured in millimetres on the film, and
//! modulated by a seeded streak field elongated along the aperture. Every
//! length converts through the working pixel pitch, so the leak keeps its
//! physical size across crop, resize, and film format.
//!
//! Both executors evaluate this module's field. The lattice hash is 32-bit
//! integer arithmetic, which WGPU implements with the same wrapping semantics
//! as Rust, so the two realizations share the field rather than merely
//! resembling it.

use rayon::prelude::*;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{Scalar, from_f64};

/// Light-leak controls. All lengths are millimetres on the film.
#[derive(Debug, Clone, PartialEq)]
pub struct LightLeakParams {
    /// Aperture exposure in stops relative to `reference_exposure`, the
    /// exposure that produces mid density on the selected film. Zero is an
    /// exact bypass.
    pub exposure_ev: f32,
    /// Aperture center position around the frame perimeter, `[0, 1)`,
    /// measured clockwise from the top-left corner.
    pub position: f32,
    /// Aperture width along the perimeter, millimetres on the film.
    pub width_mm: f32,
    /// Inward e-folding length of the leak, millimetres on the film. The
    /// falloff is Gaussian, so the leak is confined to a few spreads: a leak
    /// through one seal cannot fog the opposite corner of the frame.
    pub spread_mm: f32,
    /// Aperture tilt relative to the perimeter tangent, degrees.
    pub angle_deg: f32,
    /// Per-record added-exposure ratio, in cyan, magenta, yellow record order.
    /// The default is the common warm path: light that crossed the film base
    /// and lost blue on the way, so the rim reads orange-red and a strong leak
    /// still burns its core to the film's white. A red-only ratio (green and
    /// blue equal) gives the red rim that some measured leaks show instead.
    pub leak_spectrum: [f32; 3],
    /// Strength of the seeded streak structure over the smooth falloff.
    pub texture_amount: f32,
    /// Spatial scale of the streak structure, millimetres on the film.
    pub texture_scale_mm: f32,
}

impl Default for LightLeakParams {
    fn default() -> Self {
        Self {
            exposure_ev: 0.0,
            position: 0.0,
            width_mm: 8.0,
            spread_mm: 4.0,
            angle_deg: 0.0,
            leak_spectrum: [1.0, 0.4, 0.15],
            texture_amount: 0.25,
            texture_scale_mm: 6.0,
        }
    }
}

/// The resolved aperture, in pixel coordinates, plus every derived length the
/// field needs. Both executors evaluate the same record, so the geometry is
/// resolved once on the host and carried to the device.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightLeakGeometry {
    /// Aperture center, in pixels.
    pub center: [f32; 2],
    /// Unit aperture direction.
    pub direction: [f32; 2],
    /// Half the aperture width, in pixels.
    pub half_px: f32,
    /// Inward e-folding length, in pixels.
    pub spread_px: f32,
    /// Streak lattice scale, in pixels.
    pub texture_scale_px: f32,
}

/// Fold the render seed into the 32-bit lattice seed both executors use.
///
/// The seed is a `u64` render parameter; the lattice hash is 32-bit. Folding
/// both halves keeps the full seed entropy in the field and gives the device
/// the same value the host evaluates.
pub fn fold_seed(seed: u64) -> u32 {
    (seed as u32) ^ ((seed >> 32) as u32).rotate_left(16)
}

/// Pixels per millimetre at the working pitch.
fn px_per_mm(pixel_size_um: f32) -> f32 {
    if pixel_size_um > 0.0 {
        1000.0 / pixel_size_um
    } else {
        0.0
    }
}

/// Resolve the aperture and the derived pixel lengths for one frame.
pub fn light_leak_geometry(
    width: u32,
    height: u32,
    pixel_size_um: f32,
    p: &LightLeakParams,
) -> LightLeakGeometry {
    let scale = px_per_mm(pixel_size_um);
    let (center, tangent) = perimeter_point(width, height, p.position);
    let angle = p.angle_deg.to_radians();
    let (sin, cos) = (angle.sin(), angle.cos());
    LightLeakGeometry {
        center,
        direction: [
            tangent[0] * cos - tangent[1] * sin,
            tangent[0] * sin + tangent[1] * cos,
        ],
        half_px: (p.width_mm * 0.5 * scale).max(0.0),
        spread_px: (p.spread_mm * scale).max(1.0),
        texture_scale_px: (p.texture_scale_mm * scale).max(1.0),
    }
}

/// Walk the frame perimeter clockwise from the top-left corner, returning the
/// point and the unit tangent there.
fn perimeter_point(width: u32, height: u32, t: f32) -> ([f32; 2], [f32; 2]) {
    let w = width as f32;
    let h = height as f32;
    let perimeter = 2.0 * (w + h);
    let mut d = t.rem_euclid(1.0) * perimeter;
    if d < w {
        ([d, 0.0], [1.0, 0.0])
    } else {
        d -= w;
        if d < h {
            ([w, d], [0.0, 1.0])
        } else {
            d -= h;
            if d < w {
                ([w - d, h], [-1.0, 0.0])
            } else {
                d -= w;
                ([0.0, h - d], [0.0, -1.0])
            }
        }
    }
}

/// Distance from a point to the aperture segment, in pixels.
#[inline]
fn distance_to_segment(px: f32, py: f32, g: &LightLeakGeometry) -> f32 {
    let ax = px - g.center[0];
    let ay = py - g.center[1];
    let along = (ax * g.direction[0] + ay * g.direction[1]).clamp(-g.half_px, g.half_px);
    let ex = ax - along * g.direction[0];
    let ey = ay - along * g.direction[1];
    (ex * ex + ey * ey).sqrt()
}

/// 32-bit lattice hash. Wrapping integer arithmetic only, so the host and the
/// device evaluate bit-identical lattice values.
#[inline]
pub fn hash2(x: u32, y: u32, seed: u32) -> f32 {
    let mut h = seed ^ 0x9E37_79B9;
    h = (h ^ x.wrapping_mul(0xC2B2_AE3D)).rotate_left(15);
    h = h.wrapping_mul(0x85EB_CA77);
    h = (h ^ y.wrapping_mul(0x27D4_EB2F)).rotate_left(13);
    h = h.wrapping_mul(0x1656_67B1);
    h ^= h >> 15;
    h = h.wrapping_mul(0x9E37_7F1B);
    h ^= h >> 13;
    // 24 mantissa bits convert exactly to f32 on both executors.
    (h >> 8) as f32 / ((1u32 << 24) as f32)
}

#[inline]
fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Value noise with smoothstep interpolation, in `[0, 1)`.
pub fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let x0 = x.floor();
    let y0 = y.floor();
    let fx = smoothstep(x - x0);
    let fy = smoothstep(y - y0);
    let (ix, iy) = (x0 as i32, y0 as i32);
    // Rotated sampling coordinates are signed: wrapping keeps the two's
    // complement reinterpretation the device's u32 conversion performs.
    let x0 = ix as u32;
    let y0 = iy as u32;
    let n00 = hash2(x0, y0, seed);
    let n10 = hash2(x0.wrapping_add(1), y0, seed);
    let n01 = hash2(x0, y0.wrapping_add(1), seed);
    let n11 = hash2(x0.wrapping_add(1), y0.wrapping_add(1), seed);
    let top = n00 + (n10 - n00) * fx;
    let bottom = n01 + (n11 - n01) * fx;
    top + (bottom - top) * fy
}

/// Three-octave fractal value noise, normalized to `[0, 1)`.
pub fn streak_noise(x: f32, y: f32, seed: u32) -> f32 {
    let mut sum = 0.0;
    let mut norm = 0.0;
    let mut amp = 1.0;
    let (mut fx, mut fy) = (x, y);
    for octave in 0..3u32 {
        sum += amp * value_noise(fx, fy, seed.wrapping_add(octave));
        norm += amp;
        amp *= 0.5;
        fx *= 2.0;
        fy *= 2.0;
    }
    sum / norm
}

/// The leak's spatial weight at one pixel, in `[0, 1]`.
///
/// `field = exp(-(distance / spread)^2) * window(distance) * (1 - t + t * streaks)`
///
/// The falloff is Gaussian and then windowed to zero at two and a half spreads.
/// A seal lets light into one part of the film and the camera's cavity shadows
/// the rest, so the leak has a finite reach: beyond the gap's penumbra the
/// exposure is zero. An unbounded tail keeps adding a little exposure to the
/// far side of the frame, and in the shadows that little is a visible fog.
///
/// The streaks are sampled in the aperture's own frame and stretched along it,
/// so the structure follows the seal rather than floating over the frame as
/// round blobs.
#[inline]
pub fn leak_weight(px: f32, py: f32, g: &LightLeakGeometry, texture_amount: f32, seed: u32) -> f32 {
    let distance = distance_to_segment(px, py, g);
    let q = distance / g.spread_px;
    // One spread of penumbra beyond one and a half spreads, then nothing.
    let window = 1.0 - smoothstep(q - 1.5);
    let falloff = (-q * q).exp() * window;
    let ax = px - g.center[0];
    let ay = py - g.center[1];
    let along = ax * g.direction[0] + ay * g.direction[1];
    let across = -ax * g.direction[1] + ay * g.direction[0];
    let scale = g.texture_scale_px;
    let streaks = streak_noise(along / (scale * 3.0), across / scale, seed);
    falloff * (1.0 - texture_amount + texture_amount * streaks)
}

/// The leak's spatial weight field over one frame, in `[0, 1]`.
pub fn light_leak_field(
    width: u32,
    height: u32,
    pixel_size_um: f32,
    p: &LightLeakParams,
    seed: u64,
) -> Vec<f32> {
    let g = light_leak_geometry(width, height, pixel_size_um, p);
    let lattice = fold_seed(seed);
    let t = p.texture_amount.clamp(0.0, 1.0);
    let mut field = vec![0.0f32; (width as usize) * (height as usize)];
    for y in 0..height {
        let fy = y as f32;
        for x in 0..width {
            let i = (y as usize) * (width as usize) + (x as usize);
            field[i] = leak_weight(x as f32, fy, &g, t, lattice);
        }
    }
    field
}

/// Add the leak to linear film raw exposure.
///
/// `reference_exposure` is the exposure that produces mid density on the
/// selected film, so `exposure_ev` is a material-relative stop count that does
/// not scale with scene brightness.
pub fn apply_light_leak(
    raw: &ImageBuf,
    pixel_size_um: f32,
    reference_exposure: f64,
    p: &LightLeakParams,
    seed: u64,
) -> ImageBuf {
    if p.exposure_ev == 0.0 {
        return raw.clone();
    }
    let g = light_leak_geometry(raw.width, raw.height, pixel_size_um, p);
    let lattice = fold_seed(seed);
    let t = p.texture_amount.clamp(0.0, 1.0);
    let scale = from_f64(reference_exposure * 2.0f64.powf(p.exposure_ev as f64));
    let spectrum: [Scalar; 3] = [
        from_f64(p.leak_spectrum[0] as f64),
        from_f64(p.leak_spectrum[1] as f64),
        from_f64(p.leak_spectrum[2] as f64),
    ];
    let mut out = raw.clone();
    out.data
        .par_chunks_exact_mut(3)
        .enumerate()
        .for_each(|(i, pixel)| {
            let x = (i % raw.width as usize) as f32;
            let y = (i / raw.width as usize) as f32;
            let w = from_f64(leak_weight(x, y, &g, t, lattice) as f64);
            for c in 0..3 {
                pixel[c] += scale * spectrum[c] * w;
            }
        });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_math::precision::to_f64;

    fn params() -> LightLeakParams {
        LightLeakParams {
            exposure_ev: 1.0,
            position: 0.0,
            width_mm: 8.0,
            spread_mm: 12.0,
            angle_deg: 0.0,
            leak_spectrum: [1.0, 0.55, 0.28],
            texture_amount: 0.0,
            texture_scale_mm: 6.0,
        }
    }

    #[test]
    fn zero_exposure_is_an_exact_bypass() {
        let raw = ImageBuf::from_data(8, 8, vec![from_f64(0.5); 8 * 8 * 3]);
        let mut p = params();
        p.exposure_ev = 0.0;
        let out = apply_light_leak(&raw, 10.0, 3.0, &p, 7);
        assert_eq!(out.data, raw.data);
    }

    #[test]
    fn field_peaks_at_the_aperture_and_decays_inward() {
        // 1000 um pitch keeps the millimetre geometry readable in a small
        // frame: 8 mm aperture half-width, 12 mm e-folding spread.
        let field = light_leak_field(64, 48, 1000.0, &params(), 3);
        let w = 64usize;
        let at_aperture = field[w + 2];
        let near = field[12 * w + 2];
        let mid = field[24 * w + 2];
        let far = field[47 * w + 63];
        assert!(at_aperture > 0.9, "aperture peak was {at_aperture}");
        assert!(
            at_aperture > near && near > mid && mid > far,
            "{at_aperture} {near} {mid} {far}"
        );
        assert!(far < 0.05, "far corner was {far}");
    }

    #[test]
    fn field_is_bounded_and_finite() {
        let field = light_leak_field(32, 32, 50.0, &params(), 11);
        assert!(
            field
                .iter()
                .all(|v| v.is_finite() && *v >= 0.0 && *v <= 1.0)
        );
    }

    #[test]
    fn the_same_seed_reproduces_the_field() {
        let a = light_leak_field(24, 24, 40.0, &params(), 5);
        let b = light_leak_field(24, 24, 40.0, &params(), 5);
        assert_eq!(a, b);
    }

    #[test]
    fn a_different_seed_changes_the_streaks_only() {
        // The falloff is seed independent: only the streak structure moves, so
        // the field stays inside the falloff envelope for every seed.
        let mut smooth = params();
        smooth.texture_amount = 1.0;
        let geometry = light_leak_geometry(32, 32, 50.0, &smooth);
        for seed in [1u64, 2, 3] {
            let field = light_leak_field(32, 32, 50.0, &smooth, seed);
            for (index, weight) in field.iter().enumerate() {
                let (x, y) = ((index % 32) as f32, (index / 32) as f32);
                let envelope = leak_weight(x, y, &geometry, 0.0, fold_seed(seed));
                assert!(
                    weight <= &envelope,
                    "seed {seed} at {index}: {weight} exceeds the falloff {envelope}"
                );
            }
        }
        assert_ne!(
            light_leak_field(32, 32, 50.0, &smooth, 1),
            light_leak_field(32, 32, 50.0, &smooth, 2),
            "the streak structure must depend on the seed"
        );
    }

    #[test]
    fn the_leak_adds_exposure_in_the_exposure_domain() {
        let raw = ImageBuf::from_data(4, 4, vec![from_f64(1.0); 4 * 4 * 3]);
        let out = apply_light_leak(&raw, 1000.0, 3.0, &params(), 1);
        // The aperture center sits on the top-left corner, so pixel 0 is at
        // full weight: one stop over a reference of 3.0 is +6.0 exposure.
        let gain = to_f64(out.data[0]) - to_f64(raw.data[0]);
        assert!((gain - 6.0).abs() < 1e-6, "gain was {gain}");
    }

    #[test]
    fn the_aperture_walks_the_perimeter() {
        // Position 0 is the top-left corner with a horizontal tangent.
        let top = light_leak_geometry(100, 60, 1000.0, &params());
        assert_eq!(top.center, [0.0, 0.0]);
        assert_eq!(top.direction, [1.0, 0.0]);
        // 130 px around a 320 px perimeter is thirty pixels down the right edge.
        let mut right = params();
        right.position = 130.0 / 320.0;
        let right = light_leak_geometry(100, 60, 1000.0, &right);
        assert_eq!(right.center, [100.0, 30.0]);
        assert_eq!(right.direction, [0.0, 1.0]);
    }

    #[test]
    fn the_added_exposure_does_not_scale_with_the_scene() {
        // Stray light is absolute: the same leak adds the same exposure to a
        // dark frame and to a frame four times brighter.
        let dark = ImageBuf::from_data(4, 4, vec![from_f64(0.25); 4 * 4 * 3]);
        let bright = ImageBuf::from_data(4, 4, vec![from_f64(1.0); 4 * 4 * 3]);
        let added = |raw: &ImageBuf| {
            let out = apply_light_leak(raw, 1000.0, 3.0, &params(), 1);
            to_f64(out.data[0]) - to_f64(raw.data[0])
        };
        assert_eq!(added(&dark), added(&bright));
    }

    #[test]
    fn the_geometry_survives_a_pitch_change_at_the_same_physical_point() {
        // One physical leak, two working pitches. The aperture is anchored on
        // the top-left corner, so the corner and a point six millimetres along
        // the edge carry the same weight in both resolutions.
        let fine = params();
        let mut coarse = params();
        coarse.position = 0.0;
        let (w_fine, h_fine) = (48u32, 36u32);
        let (w_coarse, h_coarse) = (96u32, 36u32);
        let geometry_fine = light_leak_geometry(w_fine, h_fine, 1000.0, &fine);
        let geometry_coarse = light_leak_geometry(w_coarse, h_coarse, 500.0, &coarse);
        let seed = fold_seed(11);
        // The corner itself.
        let corner_fine = leak_weight(0.0, 0.0, &geometry_fine, 0.0, seed);
        let corner_coarse = leak_weight(0.0, 0.0, &geometry_coarse, 0.0, seed);
        assert!((corner_fine - corner_coarse).abs() < 1e-6);
        // Six millimetres along the top edge, and six millimetres inward.
        for (fine_xy, coarse_xy) in [((6.0, 0.0), (12.0, 0.0)), ((0.0, 6.0), (0.0, 12.0))] {
            let a = leak_weight(fine_xy.0, fine_xy.1, &geometry_fine, 0.0, seed);
            let b = leak_weight(coarse_xy.0, coarse_xy.1, &geometry_coarse, 0.0, seed);
            assert!(
                (a - b).abs() < 1e-6,
                "{fine_xy:?} gave {a}, {coarse_xy:?} gave {b}"
            );
        }
    }

    #[test]
    fn the_falloff_is_confined_to_a_few_spreads() {
        // A seal illuminates the film near the gap; the chamber shadows the
        // rest. Three spreads from the aperture the leak must be gone, or the
        // far side of the frame gains a visible fog in the shadows.
        let geometry = light_leak_geometry(64, 48, 1000.0, &params());
        let seed = fold_seed(3);
        let peak = leak_weight(0.0, 0.0, &geometry, 0.0, seed);
        let three_spreads = leak_weight(0.0, 36.0, &geometry, 0.0, seed);
        assert!((peak - 1.0).abs() < 1e-6, "peak was {peak}");
        assert_eq!(
            three_spreads, 0.0,
            "the cavity shadow must stop the leak, not fade it"
        );
        // Inside the window the falloff still falls smoothly.
        let one_spread = leak_weight(0.0, 12.0, &geometry, 0.0, seed);
        assert!(
            (one_spread - (-1.0f32).exp()).abs() < 1e-6,
            "one spread was {one_spread}"
        );
    }

    #[test]
    fn the_streaks_run_along_the_aperture() {
        // Stretched along the seal, the texture changes three times faster
        // across it. Measure the modulation alone by dividing out the smooth
        // falloff, then compare neighbouring pixels along and across.
        let mut textured = params();
        textured.texture_amount = 1.0;
        let geometry = light_leak_geometry(96, 72, 1000.0, &textured);
        let seed = fold_seed(17);
        let mut along = 0.0;
        let mut across = 0.0;
        let mut count = 0.0;
        for y in 4..34 {
            for x in 4..90 {
                let (x, y) = (x as f32, y as f32);
                let envelope = leak_weight(x, y, &geometry, 0.0, seed);
                if envelope < 0.3 {
                    continue;
                }
                let modulation = |px: f32, py: f32| {
                    let smooth = leak_weight(px, py, &geometry, 0.0, seed);
                    leak_weight(px, py, &geometry, 1.0, seed) / smooth
                };
                let here = modulation(x, y);
                along += (modulation(x + 1.0, y) - here).abs();
                across += (modulation(x, y + 1.0) - here).abs();
                count += 1.0;
            }
        }
        let (along, across) = (along / count, across / count);
        assert!(
            along < across * 0.6,
            "streaks were not elongated: along {along}, across {across}"
        );
    }

    #[test]
    fn the_hash_is_bounded_and_spreads() {
        let values: Vec<f32> = (0..4096u32)
            .map(|i| hash2(i, i.wrapping_mul(7), 99))
            .collect();
        assert!(values.iter().all(|v| *v >= 0.0 && *v < 1.0));
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        assert!((mean - 0.5).abs() < 0.05, "mean was {mean}");
    }
}
