//! CIE L*a*b* under the sRGB (D65) white point and CIEDE2000, in f64.
//!
//! The LUT QA metrics and the benchmark's difference maps compare the same
//! perceptual quantities, so the conversion and the colour-difference formula
//! live here once instead of being restated per crate. The D50 variants in
//! `spektrafilm-core::magazine_print_color` are a different white point and
//! stay separate.

use crate::colorspace::{SRGB_TO_XYZ_F64, srgb_decode_f64};

/// sRGB (D65) white point in XYZ at Y = 1 — `0.3127 / 0.329` and its blue.
pub const SRGB_WHITE_XYZ: [f64; 3] = [0.3127 / 0.329, 1.0, (1.0 - 0.3127 - 0.329) / 0.329];

/// CIE L*a*b* from XYZ under the sRGB white point.
pub fn xyz_to_lab(xyz: [f64; 3]) -> [f64; 3] {
    let f: [f64; 3] = std::array::from_fn(|i| {
        let t = xyz[i] / SRGB_WHITE_XYZ[i];
        if t > (6f64 / 29.).powi(3) {
            t.cbrt()
        } else {
            t / (3. * (6f64 / 29.).powi(2)) + 4. / 29.
        }
    });
    [
        116. * f[1] - 16.,
        500. * (f[0] - f[1]),
        200. * (f[1] - f[2]),
    ]
}

/// Encoded sRGB (clamped to its 0..1 display range) to CIE L*a*b*.
pub fn srgb_encoded_to_lab(encoded: [f64; 3]) -> [f64; 3] {
    let linear = encoded.map(|v| srgb_decode_f64(v.clamp(0., 1.)));
    let mut xyz = [0.0f64; 3];
    for i in 0..3 {
        xyz[i] = SRGB_TO_XYZ_F64[i][0] * linear[0]
            + SRGB_TO_XYZ_F64[i][1] * linear[1]
            + SRGB_TO_XYZ_F64[i][2] * linear[2];
    }
    xyz_to_lab(xyz)
}

/// CIE 142:2001 CIEDE2000, unit weights.
pub fn delta_e_2000(a: [f64; 3], b: [f64; 3]) -> f64 {
    let [l1, a1, b1] = a;
    let [l2, a2, b2] = b;
    let c1 = a1.hypot(b1);
    let c2 = a2.hypot(b2);
    let cm = (c1 + c2) / 2.;
    let g = 0.5 * (1. - (cm.powi(7) / (cm.powi(7) + 25f64.powi(7))).sqrt());
    let ap1 = (1. + g) * a1;
    let ap2 = (1. + g) * a2;
    let cp1 = ap1.hypot(b1);
    let cp2 = ap2.hypot(b2);
    let h1 = b1.atan2(ap1).to_degrees().rem_euclid(360.);
    let h2 = b2.atan2(ap2).to_degrees().rem_euclid(360.);
    let dl = l2 - l1;
    let dc = cp2 - cp1;
    let mut dh = h2 - h1;
    if cp1 * cp2 == 0. {
        dh = 0.;
    } else if dh > 180. {
        dh -= 360.;
    } else if dh < -180. {
        dh += 360.;
    }
    let d_h = 2. * (cp1 * cp2).sqrt() * (dh / 2.).to_radians().sin();
    let lm = (l1 + l2) / 2.;
    let cm = (cp1 + cp2) / 2.;
    let hm = if cp1 * cp2 == 0. {
        h1 + h2
    } else if (h1 - h2).abs() <= 180. {
        (h1 + h2) / 2.
    } else if h1 + h2 < 360. {
        (h1 + h2 + 360.) / 2.
    } else {
        (h1 + h2 - 360.) / 2.
    };
    let t = 1. - 0.17 * (hm - 30.).to_radians().cos()
        + 0.24 * (2. * hm).to_radians().cos()
        + 0.32 * (3. * hm + 6.).to_radians().cos()
        - 0.20 * (4. * hm - 63.).to_radians().cos();
    let sl = 1. + 0.015 * (lm - 50.).powi(2) / (20. + (lm - 50.).powi(2)).sqrt();
    let sc = 1. + 0.045 * cm;
    let sh = 1. + 0.015 * cm * t;
    let rt = -2.
        * (cm.powi(7) / (cm.powi(7) + 25f64.powi(7))).sqrt()
        * (60. * (-((hm - 275.) / 25.).powi(2)).exp())
            .to_radians()
            .sin();
    let x = dl / sl;
    let y = dc / sc;
    let z = d_h / sh;
    (x * x + y * y + z * z + rt * y * z).max(0.).sqrt()
}
