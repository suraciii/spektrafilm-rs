//! f64 primitives for the lab spectral roundtrip evaluation.
//!
//! The product render path works from RGB; it never integrates a measured
//! reflectance. `spektrafilm-lab spectral evaluate` reports the roundtrip
//! error between a conditional truth integral over measured spectra and the
//! product reconstruction, so the f64 building blocks that evaluation needs
//! live here: the illuminant chromaticity integration, the scene
//! spectrum-to-RGB conversion, and the film sensitivity integrals. The lab
//! command composes them; the render path is untouched.

use spektrafilm_math::colorspace;
use spektrafilm_math::spectral::{self, CMF_X_F64, CMF_Y_F64, CMF_Z_F64};

/// The mid-gray reflectance the chain anchors at (Python `0.184`).
pub const MIDGRAY_REFLECTANCE: f64 = 0.184;

/// CIE xy chromaticity of an illuminant SPD on the shared spectral grid,
/// integrating the f64 CMF tables (the calibration method's
/// `_illuminant_to_xy`).
pub fn illuminant_xy_f64(illuminant: &[f64]) -> (f64, f64) {
    let n = illuminant.len().min(spectral::N_WAVELENGTHS);
    let mut xyz = [0.0f64; 3];
    for i in 0..n {
        xyz[0] += illuminant[i] * CMF_X_F64[i];
        xyz[1] += illuminant[i] * CMF_Y_F64[i];
        xyz[2] += illuminant[i] * CMF_Z_F64[i];
    }
    spectral::xyz_to_xy_f64(xyz)
}

/// XYZ of a reflectance under an illuminant, normalized so the illuminant
/// integrates to Y = 1: `((R·E) @ CMF) / (E @ CMF_Y)`.
pub fn reflectance_xyz_f64(reflectance: &[f64], illuminant: &[f64]) -> [f64; 3] {
    let n = reflectance
        .len()
        .min(illuminant.len())
        .min(spectral::N_WAVELENGTHS);
    let mut xyz = [0.0f64; 3];
    let mut y = 0.0f64;
    for i in 0..n {
        let re = reflectance[i] * illuminant[i];
        xyz[0] += re * CMF_X_F64[i];
        xyz[1] += re * CMF_Y_F64[i];
        xyz[2] += re * CMF_Z_F64[i];
        y += illuminant[i] * CMF_Y_F64[i];
    }
    [xyz[0] / y, xyz[1] / y, xyz[2] / y]
}

/// Linear RGB of a colour space from XYZ expressed under `src_xy`'s white,
/// adapting with CAT16. This is the inverse composition of the product
/// path's RGB → adapted-XYZ projection.
pub fn xyz_to_rgb_cat16_f64(
    xyz: [f64; 3],
    src_xy: (f64, f64),
    color_space: &str,
) -> Result<[f64; 3], String> {
    let space = colorspace::resolve(color_space)?;
    let (x, y) = src_xy;
    if !x.is_finite() || !y.is_finite() || y <= 0.0 {
        return Err(format!(
            "illuminant chromaticity ({x}, {y}) has no unit-Y XYZ"
        ));
    }
    let src_white = [x / y, 1.0, (1.0 - x - y) / y];
    let dst_white = space.whitepoint_xyz();
    let adapt = colorspace::chromatic_adaptation_matrix_cat16_f64(src_white, dst_white);
    let adapted = apply_matrix_f64(&adapt, xyz);
    Ok(apply_matrix_f64(&space.matrix_xyz_to_rgb, adapted))
}

fn apply_matrix_f64(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// Measured reflectance under a scene illuminant → linear RGB.
pub fn reflectance_to_rgb_f64(
    reflectance: &[f64],
    illuminant: &[f64],
    color_space: &str,
) -> Result<[f64; 3], String> {
    let xyz = reflectance_xyz_f64(reflectance, illuminant);
    let xy = illuminant_xy_f64(illuminant);
    xyz_to_rgb_cat16_f64(xyz, xy, color_space)
}

/// Film-layer sensitivity integral of a spectrum:
/// `Σ_λ spectrum[λ] · illuminant[λ] · sensitivity[λ][c]`.
pub fn sensitivity_integral_f64(
    spectrum: &[f64],
    illuminant: &[f64],
    sensitivity: &[[f64; 3]],
) -> [f64; 3] {
    let n = spectrum.len().min(illuminant.len()).min(sensitivity.len());
    let mut out = [0.0f64; 3];
    for i in 0..n {
        let se = spectrum[i] * illuminant[i];
        for c in 0..3 {
            out[c] += se * sensitivity[i][c];
        }
    }
    out
}

/// Conditional mid-gray anchor: `gray · Σ_λ illuminant[λ] · sensitivity[λ][c]`.
pub fn midgray_anchor_f64(illuminant: &[f64], sensitivity: &[[f64; 3]], gray: f64) -> [f64; 3] {
    let n = illuminant.len().min(sensitivity.len());
    let mut raw = [0.0f64; 3];
    for i in 0..n {
        for c in 0..3 {
            raw[c] += illuminant[i] * sensitivity[i][c];
        }
    }
    [gray * raw[0], gray * raw[1], gray * raw[2]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spectral_service::select_illuminant_f64;

    fn portra_sensitivity() -> Vec<[f64; 3]> {
        let film = crate::profile::load_profile_by_name(
            &crate::test_support::data_dir(),
            "kodak_portra_400",
        )
        .unwrap();
        film.sensitivity_f64()
    }

    #[test]
    fn equal_energy_illuminant_xy_is_neutral() {
        let e = vec![1.0f64; 81];
        let (x, y) = illuminant_xy_f64(&e);
        assert!((x - 1.0 / 3.0).abs() < 1e-3, "x={x}");
        assert!((y - 1.0 / 3.0).abs() < 1e-3, "y={y}");
    }

    #[test]
    fn flat_midgray_reflectance_is_the_anchor_condition() {
        let sensitivity = portra_sensitivity();
        let t = select_illuminant_f64("T");
        let flat = vec![MIDGRAY_REFLECTANCE; 81];
        let truth = sensitivity_integral_f64(&flat, &t, &sensitivity);
        let anchor = midgray_anchor_f64(&t, &sensitivity, MIDGRAY_REFLECTANCE);
        for (c, (truth, anchor)) in truth.iter().zip(anchor).enumerate() {
            let relative = truth / anchor;
            assert!((relative - 1.0).abs() < 1e-12, "channel {c}: {relative}");
        }
    }

    #[test]
    fn flat_midgray_photographs_as_neutral_aces() {
        let t = select_illuminant_f64("T");
        let flat = vec![MIDGRAY_REFLECTANCE; 81];
        let rgb = reflectance_to_rgb_f64(&flat, &t, "ACES2065-1").unwrap();
        for channel in rgb {
            assert!(
                (channel - MIDGRAY_REFLECTANCE).abs() < 1e-9,
                "expected neutral {MIDGRAY_REFLECTANCE}, got {rgb:?}"
            );
        }
    }
}
