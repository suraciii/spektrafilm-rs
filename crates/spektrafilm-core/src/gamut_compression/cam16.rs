use super::mat_vec;
use spektrafilm_math::colorspace::spow;

// ─────────────────────────────────────────────────────────────────────────
// CAM16-UCS (upstream's default `cam16ucs` method).
//
// Fixed L_A = 64 cd/m², Y_b = 20, Average surround. Adaptation and white
// response depend on the destination whitepoint; other coefficients are fixed.
// XYZ here is at Y=1; CIECAM16 works in the Y=100 domain, hence the ×/÷100.
// ─────────────────────────────────────────────────────────────────────────
const CAM16_M16: [[f64; 3]; 3] = [
    [0.401288, 0.650173, -0.051461],
    [-0.250268, 1.204414, 0.045854],
    [-0.002079, 0.048952, 0.953127],
];
const CAM16_M16I: [[f64; 3]; 3] = [
    [1.8620678550872327, -1.0112546305316843, 0.14918677544445175],
    [
        0.3875265432361372,
        0.6214474419314753,
        -0.008973985167612516,
    ],
    [
        -0.015841498849333863,
        -0.03412293802851557,
        1.0499644368778496,
    ],
];
const CAM16_F_L: f64 = 0.6839903845696502;
const CAM16_N: f64 = 0.2;
const CAM16_N_BB: f64 = 1.0003040045593807;
const CAM16_N_CB: f64 = 1.0003040045593807;
const CAM16_Z: f64 = 1.9272135954999579;
const CAM16_C: f64 = 0.69;
const CAM16_N_C: f64 = 1.0;
// Luo 2006 CAM16-UCS coefficients (K_L=1.0, c1, c2).
const UCS_C1: f64 = 0.007;
const UCS_C2: f64 = 0.0228;

#[derive(Clone, Copy)]
pub(super) struct Cam16Viewing {
    d_rgb: [f64; 3],
    a_w: f64,
}

impl Cam16Viewing {
    pub(super) fn new(white: [f64; 3]) -> Self {
        let rgb_w = mat_vec(&CAM16_M16, white.map(|v| v * 100.0));
        let d = (1.0 - (1.0 / 3.6) * ((-64.0f64 - 42.0) / 92.0).exp()).clamp(0.0, 1.0);
        let d_rgb = rgb_w.map(|v| d * 100.0 / v + 1.0 - d);
        let rgb_wc = std::array::from_fn(|i| rgb_w[i] * d_rgb[i]);
        let [r, g, b] = cam16_padc_forward(rgb_wc);
        Self {
            d_rgb,
            a_w: (2.0 * r + g + b / 20.0 - 0.305) * CAM16_N_BB,
        }
    }
}

#[inline]
fn atan2_deg(y: f64, x: f64) -> f64 {
    y.atan2(x).to_degrees().rem_euclid(360.0)
}

#[inline]
fn cam16_padc_forward(rgb: [f64; 3]) -> [f64; 3] {
    let mut out = [0.0f64; 3];
    for i in 0..3 {
        let flr = (CAM16_F_L * rgb[i].abs() / 100.0).powf(0.42);
        out[i] = 400.0 * rgb[i].signum() * flr / (27.13 + flr) + 0.1;
    }
    out
}

#[inline]
fn cam16_padc_inverse(rgb: [f64; 3]) -> [f64; 3] {
    let mut out = [0.0f64; 3];
    for i in 0..3 {
        let d = rgb[i] - 0.1;
        let base = (27.13 * d.abs()) / (400.0 - d.abs());
        out[i] = d.signum() * 100.0 / CAM16_F_L * base.powf(1.0 / 0.42);
    }
    out
}

/// XYZ (Y=1) → CAM16-UCS `(Jp, ap, bp)`.
pub(super) fn xyz_to_cam16ucs(xyz: [f64; 3], cam: &Cam16Viewing) -> [f64; 3] {
    let xyz100 = [xyz[0] * 100.0, xyz[1] * 100.0, xyz[2] * 100.0];
    let rgb = mat_vec(&CAM16_M16, xyz100);
    let rgb_c = [
        rgb[0] * cam.d_rgb[0],
        rgb[1] * cam.d_rgb[1],
        rgb[2] * cam.d_rgb[2],
    ];
    let [ra, ga, ba] = cam16_padc_forward(rgb_c);
    let a = ra - 12.0 * ga / 11.0 + ba / 11.0;
    let b = (ra + ga - 2.0 * ba) / 9.0;
    let h = atan2_deg(b, a);
    let e_t = 0.25 * ((2.0 + h * std::f64::consts::PI / 180.0).cos() + 3.8);
    let a_resp = (2.0 * ra + ga + ba / 20.0 - 0.305) * CAM16_N_BB;
    let jj = 100.0 * spow(a_resp / cam.a_w, CAM16_C * CAM16_Z);
    let denom = ra + ga + 21.0 * ba / 20.0;
    let t = if denom != 0.0 {
        (50000.0 / 13.0) * CAM16_N_C * CAM16_N_CB * (e_t * (a * a + b * b).sqrt()) / denom
    } else {
        0.0
    };
    let cc = spow(t, 0.9) * spow(jj / 100.0, 0.5) * (1.64 - 0.29f64.powf(CAM16_N)).powf(0.73);
    let m = cc * CAM16_F_L.powf(0.25);
    let jp = (1.0 + 100.0 * UCS_C1) * jj / (1.0 + UCS_C1 * jj);
    let mp = (1.0 / UCS_C2) * (1.0 + UCS_C2 * m).ln();
    let hr = h * std::f64::consts::PI / 180.0;
    [jp, mp * hr.cos(), mp * hr.sin()]
}

/// CAM16-UCS `(Jp, ap, bp)` → XYZ (Y=1).
pub(super) fn cam16ucs_to_xyz(jab: [f64; 3], cam: &Cam16Viewing) -> [f64; 3] {
    let [jp, ap, bp] = jab;
    let mp = (ap * ap + bp * bp).sqrt();
    let h = atan2_deg(bp, ap);
    let jj = jp / ((1.0 + 100.0 * UCS_C1) - UCS_C1 * jp);
    let m = ((UCS_C2 * mp).exp() - 1.0) / UCS_C2;
    let cc = m / CAM16_F_L.powf(0.25);
    let j_prime = jj.max(f64::EPSILON);
    let t = spow(
        cc / ((j_prime / 100.0).sqrt() * (1.64 - 0.29f64.powf(CAM16_N)).powf(0.73)),
        1.0 / 0.9,
    );
    let e_t = 0.25 * ((2.0 + h * std::f64::consts::PI / 180.0).cos() + 3.8);
    let a_resp = cam.a_w * spow(jj / 100.0, 1.0 / (CAM16_C * CAM16_Z));
    let p1 = if t != 0.0 {
        (50000.0 / 13.0) * CAM16_N_C * CAM16_N_CB * e_t / t
    } else {
        0.0
    };
    let p2 = a_resp / CAM16_N_BB + 0.305;
    let p3 = 21.0 / 20.0;
    let (mut a, mut b) = cam16_opponent_inverse(p1, p2, p3, h);
    // Achromatic guard, matching colour's `ab * np.where(t == 0, 0, 1)`:
    // when t == 0 the hue is undefined and the opponent inverse must be zeroed.
    if t == 0.0 {
        a = 0.0;
        b = 0.0;
    }
    let ra = (460.0 * p2 + 451.0 * a + 288.0 * b) / 1403.0;
    let ga = (460.0 * p2 - 891.0 * a - 261.0 * b) / 1403.0;
    let ba = (460.0 * p2 - 220.0 * a - 6300.0 * b) / 1403.0;
    let rgb_c = cam16_padc_inverse([ra, ga, ba]);
    let rgb = [
        rgb_c[0] / cam.d_rgb[0],
        rgb_c[1] / cam.d_rgb[1],
        rgb_c[2] / cam.d_rgb[2],
    ];
    let xyz100 = mat_vec(&CAM16_M16I, rgb);
    [xyz100[0] / 100.0, xyz100[1] / 100.0, xyz100[2] / 100.0]
}

/// Inverse opponent dimensions (CIECAM02/16 `opponent_colour_dimensions_inverse`).
fn cam16_opponent_inverse(p1: f64, p2: f64, p3: f64, h: f64) -> (f64, f64) {
    let hr = h * std::f64::consts::PI / 180.0;
    let s = hr.sin();
    let c = hr.cos();
    let nn = p2 * (2.0 + p3) * (460.0 / 1403.0);
    if s.abs() >= c.abs() {
        let p4 = if s != 0.0 { p1 / s } else { 0.0 };
        let b = nn
            / (p4 + (2.0 + p3) * (220.0 / 1403.0) * (c / s) - (27.0 / 1403.0)
                + p3 * (6300.0 / 1403.0));
        (b * (c / s), b)
    } else {
        let p5 = if c != 0.0 { p1 / c } else { 0.0 };
        let a = nn
            / (p5 + (2.0 + p3) * (220.0 / 1403.0)
                - ((27.0 / 1403.0) - p3 * (6300.0 / 1403.0)) * (s / c));
        (a, a * (s / c))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_math::colorspace::resolve;

    #[test]
    fn cam16_adapts_to_destination_whitepoint() {
        let references = [
            (
                "sRGB",
                [1.0228770275436545, 0.9852074782801457, 0.9285450586783286],
                37.16907530221132,
            ),
            (
                "DCI-P3",
                [1.0379655182795853, 0.978366094618728, 1.04121606038871],
                37.156162885738674,
            ),
            (
                "ProPhoto RGB",
                [1.0048858995308803, 0.9991651597854243, 1.1823902986704795],
                37.17349142011997,
            ),
            (
                "ACES2065-1",
                [1.0181013144461637, 0.9889551394254175, 0.9923024021726741],
                37.169941806273705,
            ),
        ];
        for (destination, d_rgb, a_w) in references {
            let cs = resolve(destination).unwrap();
            let cam = Cam16Viewing::new(cs.whitepoint_xyz());
            for i in 0..3 {
                assert!((cam.d_rgb[i] - d_rgb[i]).abs() < 1e-14);
            }
            assert!((cam.a_w - a_w).abs() < 1e-12);
            let xyz = cs.whitepoint_xyz();
            let back = cam16ucs_to_xyz(xyz_to_cam16ucs(xyz, &cam), &cam);
            for i in 0..3 {
                assert!((back[i] - xyz[i]).abs() < 1e-12);
            }
        }
    }
}
