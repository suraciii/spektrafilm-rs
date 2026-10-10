use super::mat_vec;
use spektrafilm_math::colorspace::spow;

// Safdar 2017 constants and matrices from the pinned colour-science runtime.
const JZ_M1: [[f64; 3]; 3] = [
    [0.41478972, 0.579999, 0.014648],
    [-0.20151, 1.120649, 0.0531008],
    [-0.0166008, 0.2648, 0.6684799],
];
const JZ_M1_INV: [[f64; 3]; 3] = [
    [
        1.9242264357876069,
        -1.0047923125953655,
        0.037651404030617994,
    ],
    [
        0.35031676209499907,
        0.7264811939316552,
        -0.06538442294808501,
    ],
    [
        -0.09098281098284754,
        -0.3127282905230739,
        1.5227665613052603,
    ],
];
const JZ_M2: [[f64; 3]; 3] = [
    [0.5, 0.5, 0.0],
    [3.524, -4.066708, 0.542708],
    [0.199076, 1.096799, -1.295875],
];
const JZ_M2_INV: [[f64; 3]; 3] = [
    [1.0, 0.1386050432715393, 0.058047316156118994],
    [
        0.9999999999999999,
        -0.13860504327153927,
        -0.05804731615611874,
    ],
    [
        0.9999999999999998,
        -0.09601924202631892,
        -0.8118918960560387,
    ],
];
const JZ_B: f64 = 1.15;
const JZ_G: f64 = 0.66;
const JZ_D: f64 = -0.56;
const JZ_D0: f64 = 1.6295499532821565e-11;
const JZ_M_1: f64 = 0.1593017578125;
const JZ_M_2: f64 = 134.03437499999998;
const JZ_C1: f64 = 0.8359375;
const JZ_C2: f64 = 18.8515625;
const JZ_C3: f64 = 18.6875;
const JZ_Y_WHITE: f64 = 100.0;

pub(super) fn xyz_to_jzazbz(xyz: [f64; 3]) -> [f64; 3] {
    let [x, y, z] = xyz.map(|v| v * JZ_Y_WHITE);
    let lms = mat_vec(
        &JZ_M1,
        [JZ_B * x - (JZ_B - 1.0) * z, JZ_G * y - (JZ_G - 1.0) * x, z],
    );
    let pq = lms.map(|v| {
        let yp = spow(v / 10000.0, JZ_M_1);
        spow((JZ_C1 + JZ_C2 * yp) / (1.0 + JZ_C3 * yp), JZ_M_2)
    });
    let [iz, az, bz] = mat_vec(&JZ_M2, pq);
    [(1.0 + JZ_D) * iz / (1.0 + JZ_D * iz) - JZ_D0, az, bz]
}

pub(super) fn jzazbz_to_xyz(jab: [f64; 3]) -> [f64; 3] {
    let [jz, az, bz] = jab;
    let iz = (jz + JZ_D0) / (1.0 + JZ_D - JZ_D * (jz + JZ_D0));
    let pq = mat_vec(&JZ_M2_INV, [iz, az, bz]);
    let lms = pq.map(|v| {
        let vp = spow(v, 1.0 / JZ_M_2);
        10000.0 * spow((vp - JZ_C1).max(0.0) / (JZ_C2 - JZ_C3 * vp), 1.0 / JZ_M_1)
    });
    let [xp, yp, z] = mat_vec(&JZ_M1_INV, lms);
    let x = (xp + (JZ_B - 1.0) * z) / JZ_B;
    let y = (yp + (JZ_G - 1.0) * x) / JZ_G;
    [x / JZ_Y_WHITE, y / JZ_Y_WHITE, z / JZ_Y_WHITE]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jzazbz_forward_inverse_match_fresh_python_constants() {
        let cases = [
            (
                [0.0, 0.0, 0.0],
                [0.0, -4.33057640838877e-27, -4.96277727979887e-28],
                [0.0, 0.0, 0.0],
            ),
            (
                [0.9504559270516716, 1.0, 1.0890577507598784],
                [
                    0.16717342769906365,
                    -0.0001403351730878002,
                    -0.00010225282099334996,
                ],
                [0.9504559270517058, 1.0000000000000124, 1.089057750759923],
            ),
            (
                [0.2, 0.1, -0.01],
                [0.0705700543382176, 0.08128666063727495, 0.09067046006630623],
                [
                    0.20000000000001084,
                    0.09999999999999104,
                    -0.009999999999997755,
                ],
            ),
        ];
        for (xyz, jab, inverse) in cases {
            let forward = xyz_to_jzazbz(xyz);
            let back = jzazbz_to_xyz(jab);
            for i in 0..3 {
                assert!((forward[i] - jab[i]).abs() < 1e-12, "{xyz:?}: {forward:?}");
                assert!((back[i] - inverse[i]).abs() < 1e-11, "{jab:?}: {back:?}");
            }
        }
    }
}
