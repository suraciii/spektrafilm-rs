/// Color space conversion matrices and utilities.
///
/// All matrices are row-major: output[i] = dot(matrix[i], input).
/// Source: IEC 61966-2-1 (sRGB), ICC profiles, CIE standards.

/// sRGB → CIE XYZ (D65), assuming linear input.
/// Matches Python colour-science 4-digit precision for exact parity.
pub const SRGB_TO_XYZ: [[f32; 3]; 3] = [
    [0.4124, 0.3576, 0.1805],
    [0.2126, 0.7152, 0.0722],
    [0.0193, 0.1192, 0.9505],
];

/// f64 variants for the calibration chain (Python parity).
pub const SRGB_TO_XYZ_F64: [[f64; 3]; 3] = [
    [0.4124, 0.3576, 0.1805],
    [0.2126, 0.7152, 0.0722],
    [0.0193, 0.1192, 0.9505],
];
pub const PROPHOTO_TO_XYZ_F64: [[f64; 3]; 3] = [
    [0.7976749, 0.1351917, 0.0313534],
    [0.2880402, 0.7118741, 0.0000857],
    [0.0000000, 0.0000000, 0.8252100],
];
pub const REC2020_TO_XYZ_F64: [[f64; 3]; 3] = [
    [
        0.63695804830129099,
        0.14461690358620841,
        0.16888097516417205,
    ],
    [
        0.26270021201126692,
        0.67799807151887115,
        0.059301716469861938,
    ],
    [0.0, 0.028072693049087445, 1.0609850577107907],
];
// Bit-exact match with colour-science (Python `repr` decimals
// round-trip to the same f64 bits in Rust's correctly-rounded parser).
pub const ACES_TO_XYZ_F64: [[f64; 3]; 3] = [
    [0.9525523959, 0.0, 9.36786e-05],
    [0.3439664498, 0.7281660966, -0.0721325464],
    [0.0, 0.0, 1.0088251844],
];

/// f64 XYZ → RGB matrices (inverses of the *_TO_XYZ_F64 matrices),
/// matching colour-science exactly. colour-science stores the sRGB
/// matrix at the IEC 61966-2-1 standard 4-decimal precision — using
/// the high-precision inverse here produces ~1e-5 of drift in the
/// scan stage.
pub const XYZ_TO_SRGB_F64: [[f64; 3]; 3] = [
    [3.2406, -1.5372, -0.4986],
    [-0.9689, 1.8758, 0.0415],
    [0.0557, -0.204, 1.057],
];
pub const XYZ_TO_PROPHOTO_F64: [[f64; 3]; 3] = [
    [1.346, -0.2556, -0.0511],
    [-0.5446, 1.5082, 0.0205],
    [0.0, 0.0, 1.2123],
];
pub const XYZ_TO_REC2020_F64: [[f64; 3]; 3] = [
    [
        1.7166511879712687,
        -0.35567078377639255,
        -0.25336628137365996,
    ],
    [
        -0.66668435183248886,
        1.6164812366349386,
        0.015768545813911142,
    ],
    [
        0.017639857445310794,
        -0.042770613257808537,
        0.94210312123547413,
    ],
];
pub const XYZ_TO_ACES_F64: [[f64; 3]; 3] = [
    [1.0498110175, 0.0, -9.74845e-05],
    [-0.4959030231, 1.3733130458, 0.0982400361],
    [0.0, 0.0, 0.9912520182],
];

/// CIE XYZ (D65) → sRGB linear.
/// Exact inverse of the 4-digit SRGB_TO_XYZ above.
pub const XYZ_TO_SRGB: [[f32; 3]; 3] = [
    [3.2406255, -1.5372080, -0.4986286],
    [-0.9689307, 1.8757561, 0.0415175],
    [0.0557101, -0.2040211, 1.0569959],
];

/// ProPhoto RGB → CIE XYZ (D50).
pub const PROPHOTO_TO_XYZ: [[f32; 3]; 3] = [
    [0.7976749, 0.1351917, 0.0313534],
    [0.2880402, 0.7118741, 0.0000857],
    [0.0000000, 0.0000000, 0.8252100],
];

/// CIE XYZ (D50) → ProPhoto RGB.
pub const XYZ_TO_PROPHOTO: [[f32; 3]; 3] = [
    [1.3459433, -0.2556075, -0.0511118],
    [-0.5445989, 1.5081673, 0.0205351],
    [0.0000000, 0.0000000, 1.2118128],
];

/// Rec. 2020 → CIE XYZ (D65).
pub const REC2020_TO_XYZ: [[f32; 3]; 3] = [
    [0.6369580, 0.1446169, 0.1688810],
    [0.2627002, 0.6779981, 0.0593017],
    [0.0000000, 0.0280727, 1.0609851],
];

/// CIE XYZ (D65) → Rec. 2020.
pub const XYZ_TO_REC2020: [[f32; 3]; 3] = [
    [1.7166512, -0.3556708, -0.2533663],
    [-0.6666844, 1.6164812, 0.0157685],
    [0.0176399, -0.0427706, 0.9421031],
];

/// ACES AP0 (ACES2065-1) → CIE XYZ (D60-ish, ACES white).
pub const ACES_TO_XYZ: [[f32; 3]; 3] = [
    [0.9525524, 0.0000000, 0.0000937],
    [0.3439664, 0.7281661, -0.0721325],
    [0.0000000, 0.0000000, 1.0088252],
];

/// CIE XYZ → ACES AP0.
pub const XYZ_TO_ACES: [[f32; 3]; 3] = [
    [1.0498110, 0.0000000, -0.0000974],
    [-0.4959030, 1.3733131, 0.0982400],
    [0.0000000, 0.0000000, 0.9912520],
];

/// CAT02 forward matrix (XYZ → LMS).
pub const CAT02_FORWARD: [[f32; 3]; 3] = [
    [0.7328, 0.4296, -0.1624],
    [-0.7036, 1.6975, 0.0061],
    [0.0030, 0.0136, 0.9834],
];

/// CAT02 inverse matrix (LMS → XYZ).
pub const CAT02_INVERSE: [[f32; 3]; 3] = [
    [1.096124, -0.278869, 0.182745],
    [0.454369, 0.473533, 0.072098],
    [-0.009628, -0.005698, 1.015326],
];

/// CIE D50 white point (XYZ, Y=1).
pub const D50_XYZ: [f32; 3] = [0.96422, 1.0, 0.82521];
/// CIE D55 white point (XYZ, Y=1).
pub const D55_XYZ: [f32; 3] = [0.95682, 1.0, 0.92149];
/// CIE D65 white point (XYZ, Y=1).
/// Derived from xy=(0.3127, 0.3290): X=0.3127/0.3290, Z=(1-0.3127-0.3290)/0.3290
pub const D65_XYZ: [f32; 3] = [0.95047, 1.0, 1.08883];

/// sRGB whitepoint as xy chromaticity (matches Python colour-science).
pub const SRGB_WHITE_XY: (f32, f32) = (0.3127, 0.329);
/// Convert xy chromaticity to XYZ (Y=1).
pub fn xy_to_xyz(x: f32, y: f32) -> [f32; 3] {
    if y <= 0.0 {
        return [0.0, 1.0, 0.0];
    }
    [x / y, 1.0, (1.0 - x - y) / y]
}

/// Convert xy chromaticity to XYZ (Y=1) in f64.
pub fn xy_to_xyz_f64(x: f64, y: f64) -> [f64; 3] {
    if y <= 0.0 {
        return [0.0, 1.0, 0.0];
    }
    [x / y, 1.0, (1.0 - x - y) / y]
}

/// Apply 3x3 matrix to RGB triple.
#[inline]
pub fn mat3_mul(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// Chromatic adaptation from source white to destination white using CAT02.
/// Computes in f64 internally for precision, returns f32.
pub fn chromatic_adaptation_matrix(src_white: [f32; 3], dst_white: [f32; 3]) -> [[f32; 3]; 3] {
    // High-precision CAT02 matrices (f64)
    const FWD: [[f64; 3]; 3] = [
        [0.7328, 0.4296, -0.1624],
        [-0.7036, 1.6975, 0.0061],
        [0.003, 0.0136, 0.9834],
    ];
    const INV: [[f64; 3]; 3] = [
        [1.096123820835514, -0.278869000218287, 0.182745179382773],
        [0.454369041975359, 0.473533154307412, 0.072097803717229],
        [-0.009627608738429, -0.005698031216113, 1.015325639954543],
    ];

    let sw = [
        src_white[0] as f64,
        src_white[1] as f64,
        src_white[2] as f64,
    ];
    let dw = [
        dst_white[0] as f64,
        dst_white[1] as f64,
        dst_white[2] as f64,
    ];

    // src/dst → LMS
    let mut src_lms = [0.0f64; 3];
    let mut dst_lms = [0.0f64; 3];
    for i in 0..3 {
        src_lms[i] = FWD[i][0] * sw[0] + FWD[i][1] * sw[1] + FWD[i][2] * sw[2];
        dst_lms[i] = FWD[i][0] * dw[0] + FWD[i][1] * dw[1] + FWD[i][2] * dw[2];
    }

    let gain = [
        dst_lms[0] / src_lms[0],
        dst_lms[1] / src_lms[1],
        dst_lms[2] / src_lms[2],
    ];

    // M_adapt = INV * diag(gain) * FWD
    let mut result = [[0.0f32; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            let sum = INV[i][0] * gain[0] * FWD[0][j]
                + INV[i][1] * gain[1] * FWD[1][j]
                + INV[i][2] * gain[2] * FWD[2][j];
            result[i][j] = sum as f32;
        }
    }
    result
}

/// Chromatic adaptation in full f64 precision, returning f64 matrix.
pub fn chromatic_adaptation_matrix_f64(src_white: [f64; 3], dst_white: [f64; 3]) -> [[f64; 3]; 3] {
    const FWD: [[f64; 3]; 3] = [
        [0.7328, 0.4296, -0.1624],
        [-0.7036, 1.6975, 0.0061],
        [0.003, 0.0136, 0.9834],
    ];
    const INV: [[f64; 3]; 3] = [
        [1.096123820835514, -0.278869000218287, 0.182745179382773],
        [0.454369041975359, 0.473533154307412, 0.072097803717229],
        [-0.009627608738429, -0.005698031216113, 1.015325639954543],
    ];
    von_kries(src_white, dst_white, &FWD, &INV)
}

/// CAT16 (Li et al. 2017) von Kries chromatic adaptation. Same form as
/// `chromatic_adaptation_matrix_f64` (CAT02) but with the CIECAM16 cone
/// matrix `M16`, which avoids CAT02's cone-primary instabilities around
/// blue/violet. Matches colour-science's
/// `matrix_chromatic_adaptation_VonKries(..., transform="CAT16")`.
pub fn chromatic_adaptation_matrix_cat16_f64(
    src_white: [f64; 3],
    dst_white: [f64; 3],
) -> [[f64; 3]; 3] {
    const FWD: [[f64; 3]; 3] = [
        [0.401288, 0.650173, -0.051461],
        [-0.250268, 1.204414, 0.045854],
        [-0.002079, 0.048952, 0.953127],
    ];
    const INV: [[f64; 3]; 3] = [
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
    von_kries(src_white, dst_white, &FWD, &INV)
}

/// Shared von Kries adaptation: `inv · diag(dst_lms/src_lms) · fwd`, where
/// `fwd`/`inv` are the cone matrix and its inverse for the chosen transform.
fn von_kries(
    src_white: [f64; 3],
    dst_white: [f64; 3],
    fwd: &[[f64; 3]; 3],
    inv: &[[f64; 3]; 3],
) -> [[f64; 3]; 3] {
    let mut src_lms = [0.0f64; 3];
    let mut dst_lms = [0.0f64; 3];
    for i in 0..3 {
        src_lms[i] = fwd[i][0] * src_white[0] + fwd[i][1] * src_white[1] + fwd[i][2] * src_white[2];
        dst_lms[i] = fwd[i][0] * dst_white[0] + fwd[i][1] * dst_white[1] + fwd[i][2] * dst_white[2];
    }
    let gain = [
        dst_lms[0] / src_lms[0],
        dst_lms[1] / src_lms[1],
        dst_lms[2] / src_lms[2],
    ];

    let mut result = [[0.0f64; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            result[i][j] = inv[i][0] * gain[0] * fwd[0][j]
                + inv[i][1] * gain[1] * fwd[1][j]
                + inv[i][2] * gain[2] * fwd[2][j];
        }
    }
    result
}

/// Build a full RGB→RGB conversion matrix:
/// src_RGB → src_XYZ → adapted_XYZ → dst_XYZ → dst_RGB
pub fn rgb_to_rgb_matrix(
    src_to_xyz: &[[f32; 3]; 3],
    src_white: [f32; 3],
    xyz_to_dst: &[[f32; 3]; 3],
    dst_white: [f32; 3],
) -> [[f32; 3]; 3] {
    let adapt = chromatic_adaptation_matrix(src_white, dst_white);

    // Combined = xyz_to_dst * adapt * src_to_xyz
    let mut tmp = [[0.0f32; 3]; 3];
    mat3_mul3(&adapt, src_to_xyz, &mut tmp);

    let mut result = [[0.0f32; 3]; 3];
    mat3_mul3(xyz_to_dst, &tmp, &mut result);
    result
}

/// Multiply two 3x3 matrices: out = a * b
fn mat3_mul3(a: &[[f32; 3]; 3], b: &[[f32; 3]; 3], out: &mut [[f32; 3]; 3]) {
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// RGB colour-space registry — the single authority for the seven spaces
// upstream 0.3.4 exposes (colour-science `RGB_COLOURSPACES` names). Every
// matrix / whitepoint / transfer function below is the value colour-science
// stores (0.4.6/0.4.7 — identical for these entries), dumped from the
// pinned Python environment. Unknown names MUST fail through `resolve`;
// no caller may fall back to a default matrix.
// ─────────────────────────────────────────────────────────────────────────

/// Transfer function attached to an RGB colour space (colour-science
/// `cctf_decoding`/`cctf_encoding` of the space).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cctf {
    /// Scene-linear (ACES2065-1) — identity.
    Linear,
    /// ACEScct logarithmic encoding used by the ACEScct RGB transport space.
    Acescct,
    /// IEC 61966-2-1 sRGB piecewise curve (sRGB, Display P3).
    Srgb,
    /// ROMM/ProPhoto: gamma 1.8 with a 1/32 encoded toe (`x < 16·E_t` on
    /// decode, `E_t > x` on encode, `E_t = 16^(1.8/(1-1.8)) = 1/512`).
    ProPhoto,
    /// ITU-R BT.2020 OETF (colour-science's default 10-bit constants:
    /// α = 1.099, β = 0.018).
    Rec2020,
    /// Adobe RGB (1998): pure power 563/256 (negative → NaN, matching
    /// colour-science's `gamma_function`).
    AdobeRgb1998,
    /// DCI-P3: pure power 2.6 (negative → NaN, as above).
    Gamma2_6,
}

/// One registered RGB colour space: colour-science's stored matrices,
/// whitepoint chromaticity and transfer function.
#[derive(Clone, Copy, Debug)]
pub struct RgbColorSpace {
    /// Canonical colour-science name (params carry these verbatim).
    pub name: &'static str,
    /// Linear RGB → CIE XYZ at the space's own whitepoint.
    pub matrix_rgb_to_xyz: [[f64; 3]; 3],
    /// CIE XYZ at the space's own whitepoint → linear RGB.
    pub matrix_xyz_to_rgb: [[f64; 3]; 3],
    /// Whitepoint chromaticity (x, y).
    pub whitepoint_xy: (f64, f64),
    /// Transfer function.
    pub cctf: Cctf,
}

impl RgbColorSpace {
    /// Whitepoint as XYZ at Y = 1 — upstream `_xy_to_xyz_unit_y`.
    pub fn whitepoint_xyz(&self) -> [f64; 3] {
        let (x, y) = self.whitepoint_xy;
        let safe_y = y.max(1e-12);
        [x / safe_y, 1.0, (1.0 - x - y) / safe_y]
    }

    /// The near-identity matrix colour's `RGB_to_RGB(cs, cs, 'CAT02')`
    /// computes — `M_xyz_to_rgb @ M_rgb_to_xyz`. The 4-decimal-rounded
    /// spaces (sRGB, ProPhoto, Adobe RGB) are not exact inverses, so this
    /// product has ~1e-5 off-diagonals that the scan-stage encode pass
    /// reproduces for Python parity.
    pub fn rgb_to_rgb_identity(&self) -> [[f64; 3]; 3] {
        let mut m = [[0.0f64; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] = self.matrix_xyz_to_rgb[i][0] * self.matrix_rgb_to_xyz[0][j]
                    + self.matrix_xyz_to_rgb[i][1] * self.matrix_rgb_to_xyz[1][j]
                    + self.matrix_xyz_to_rgb[i][2] * self.matrix_rgb_to_xyz[2][j];
            }
        }
        m
    }
}

/// sRGB — IEC 61966-2-1 4-decimal matrices as stored by colour-science.
pub static SRGB: RgbColorSpace = RgbColorSpace {
    name: "sRGB",
    matrix_rgb_to_xyz: SRGB_TO_XYZ_F64,
    matrix_xyz_to_rgb: XYZ_TO_SRGB_F64,
    whitepoint_xy: (0.3127, 0.329),
    cctf: Cctf::Srgb,
};

/// DCI-P3 (theatrical, DCI white ≈ 6300 K, gamma 2.6).
pub static DCI_P3: RgbColorSpace = RgbColorSpace {
    name: "DCI-P3",
    matrix_rgb_to_xyz: [
        [0.44516981556455232, 0.27713440920677762, 0.1722826698155645],
        [
            0.20949167791273049,
            0.72159525416104364,
            0.068913067926225799,
        ],
        [0.0, 0.047060560053981147, 0.9073553943619731],
    ],
    matrix_xyz_to_rgb: [
        [
            2.7253940304917332,
            -1.0180030062271852,
            -0.44016319519003655,
        ],
        [
            -0.79516802580876422,
            1.6897320548436241,
            0.022647190608477474,
        ],
        [
            0.041241891395700045,
            -0.087639019215862396,
            1.1009293786463223,
        ],
    ],
    whitepoint_xy: (0.314, 0.351),
    cctf: Cctf::Gamma2_6,
};

/// Display P3 — P3 primaries at D65 with the sRGB curve.
pub static DISPLAY_P3: RgbColorSpace = RgbColorSpace {
    name: "Display P3",
    matrix_rgb_to_xyz: [
        [0.486570948648216, 0.2656676931690931, 0.19821728523436247],
        [0.2289745640697487, 0.6917385218365064, 0.07928691409374498],
        [0.0, 0.04511338185890264, 1.0439443689009757],
    ],
    matrix_xyz_to_rgb: [
        [
            2.4934969119414263,
            -0.9313836179191244,
            -0.40271078445071706,
        ],
        [
            -0.8294889695615748,
            1.7626640603183463,
            0.023624685841943605,
        ],
        [
            0.035845830243784474,
            -0.07617238926804183,
            0.9568845240076874,
        ],
    ],
    whitepoint_xy: (0.3127, 0.329),
    cctf: Cctf::Srgb,
};

/// Adobe RGB (1998) — D65, pure power 563/256.
pub static ADOBE_RGB_1998: RgbColorSpace = RgbColorSpace {
    name: "Adobe RGB (1998)",
    matrix_rgb_to_xyz: [
        [0.57667, 0.18556, 0.18823],
        [0.29734, 0.62736, 0.07529],
        [0.02703, 0.07069, 0.99134],
    ],
    matrix_xyz_to_rgb: [
        [2.04159, -0.56501, -0.34473],
        [-0.96924, 1.87597, 0.04156],
        [0.01344, -0.11836, 1.01517],
    ],
    whitepoint_xy: (0.3127, 0.329),
    cctf: Cctf::AdobeRgb1998,
};

/// ITU-R BT.2020 — D65, BT.2020 OETF.
pub static ITU_R_BT2020: RgbColorSpace = RgbColorSpace {
    name: "ITU-R BT.2020",
    matrix_rgb_to_xyz: REC2020_TO_XYZ_F64,
    matrix_xyz_to_rgb: XYZ_TO_REC2020_F64,
    whitepoint_xy: (0.3127, 0.329),
    cctf: Cctf::Rec2020,
};

/// ProPhoto RGB (ROMM) — D50, 4-decimal standard matrices as stored by
/// colour-science (NOT the high-precision derived variants).
pub static PROPHOTO_RGB: RgbColorSpace = RgbColorSpace {
    name: "ProPhoto RGB",
    matrix_rgb_to_xyz: [
        [0.7977, 0.1352, 0.0313],
        [0.2880, 0.7119, 0.0001],
        [0.0, 0.0, 0.8249],
    ],
    matrix_xyz_to_rgb: [
        [1.3460, -0.2556, -0.0511],
        [-0.5446, 1.5082, 0.0205],
        [0.0, 0.0, 1.2123],
    ],
    whitepoint_xy: (0.3457, 0.3585),
    cctf: Cctf::ProPhoto,
};

/// ACES2065-1 (AP0) — ACES white (~D60), scene-linear.
pub static ACES2065_1: RgbColorSpace = RgbColorSpace {
    name: "ACES2065-1",
    matrix_rgb_to_xyz: ACES_TO_XYZ_F64,
    matrix_xyz_to_rgb: XYZ_TO_ACES_F64,
    whitepoint_xy: (0.32168, 0.33767),
    cctf: Cctf::Linear,
};

/// Resolve a colour-space name against the registry. Unknown names are an
/// actionable error — the caller must fail before producing any artifact.
pub fn resolve(name: &str) -> Result<&'static RgbColorSpace, String> {
    match name {
        "sRGB" => Ok(&SRGB),
        "DCI-P3" => Ok(&DCI_P3),
        "Display P3" => Ok(&DISPLAY_P3),
        "Adobe RGB (1998)" => Ok(&ADOBE_RGB_1998),
        "ITU-R BT.2020" => Ok(&ITU_R_BT2020),
        "ProPhoto RGB" => Ok(&PROPHOTO_RGB),
        "ACES2065-1" => Ok(&ACES2065_1),
        other => crate::lut_primaries::resolve(other)
            .ok_or_else(|| format!("unknown RGB colour space {other:?}")),
    }
}

/// Sign-preserving power — colour-science's `spow` (NaN where the base is
/// negative and the exponent is fractional-but-integral... no: `spow` is
/// sign-preserving by construction and only NaN on NaN input).
#[inline]
pub fn spow(x: f64, p: f64) -> f64 {
    x.signum() * x.abs().powf(p)
}

/// sRGB encoded → linear, exactly colour's `eotf_sRGB` (threshold computed
/// as `12.92 · 0.0031308`, linear branch on `>=`).
#[inline]
pub fn srgb_decode_f64(v: f64) -> f64 {
    if 12.92 * 0.0031308 >= v {
        v / 12.92
    } else {
        spow((v + 0.055) / 1.055, 2.4)
    }
}

/// Linear → sRGB encoded, exactly colour's `eotf_inverse_sRGB`.
#[inline]
pub fn srgb_encode_f64(v: f64) -> f64 {
    if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * spow(v, 1.0 / 2.4) - 0.055
    }
}

/// Apply the space's decoding transfer function (encoded → linear).
/// Mirrors colour-science exactly, including NaN for negative inputs in the
/// pure-power spaces (`gamma_function`) and sign-preserving `spow` powers
/// in the piecewise spaces.
#[inline]
pub fn cctf_decode(v: f64, cctf: Cctf) -> f64 {
    match cctf {
        Cctf::Linear => v,
        Cctf::Acescct => {
            if v > 0.155251141552511 {
                (v * 17.52 - 9.72).exp2()
            } else {
                (v - 0.0729055341958355) / 10.5402377416545
            }
        }
        Cctf::Srgb => srgb_decode_f64(v),
        Cctf::ProPhoto => {
            // E_t = 16^(1.8/(1-1.8)) = 1/512; linear below 16·E_t = 1/32.
            if v < 16.0 * (1.0 / 512.0) {
                v / 16.0
            } else {
                spow(v, 1.8)
            }
        }
        Cctf::Rec2020 => {
            const A: f64 = 1.099;
            const B: f64 = 0.018;
            if v < A * B.powf(0.45) - (A - 1.0) {
                v / 4.5
            } else {
                spow((v + (A - 1.0)) / A, 1.0 / 0.45)
            }
        }
        Cctf::AdobeRgb1998 => v.powf(563.0 / 256.0),
        Cctf::Gamma2_6 => v.powf(2.6),
    }
}

/// Apply the space's encoding transfer function (linear → encoded).
#[inline]
pub fn cctf_encode(v: f64, cctf: Cctf) -> f64 {
    match cctf {
        Cctf::Linear => v,
        Cctf::Acescct => {
            if v <= 0.0078125 {
                10.5402377416545 * v + 0.0729055341958355
            } else {
                (v.log2() + 9.72) / 17.52
            }
        }
        Cctf::Srgb => srgb_encode_f64(v),
        Cctf::ProPhoto => {
            // E_t = 1/512; linear when E_t > v, else gamma 1/1.8.
            if 1.0 / 512.0 > v {
                v * 16.0
            } else {
                spow(v, 1.0 / 1.8)
            }
        }
        Cctf::Rec2020 => {
            const A: f64 = 1.099;
            const B: f64 = 0.018;
            if v < B {
                v * 4.5
            } else {
                A * spow(v, 0.45) - (A - 1.0)
            }
        }
        Cctf::AdobeRgb1998 => v.powf(1.0 / (563.0 / 256.0)),
        Cctf::Gamma2_6 => v.powf(1.0 / 2.6),
    }
}

/// Encode the same-space RGB roundtrip used by upstream scanning. Floating
/// output keeps negative and super-white values; integer writers own clipping.
pub fn encode_rgb(rgb: [f64; 3], space: &RgbColorSpace) -> [f64; 3] {
    let m = space.rgb_to_rgb_identity();
    let mut out = [0.0; 3];
    for i in 0..3 {
        let linear = m[i][0] * rgb[0] + m[i][1] * rgb[1] + m[i][2] * rgb[2];
        out[i] = cctf_encode(linear, space.cctf);
    }
    out
}

/// Convert native output to encoded sRGB for a bounded display surface.
/// Native image exports retain their chosen colour space and floating range.
pub fn display_matrix(space: &RgbColorSpace) -> [[f64; 3]; 3] {
    conversion_matrix(space, &SRGB)
}

/// Compose the stored destination inverse, CAT02, and source matrix once.
pub fn conversion_matrix(source: &RgbColorSpace, destination: &RgbColorSpace) -> [[f64; 3]; 3] {
    let cat =
        chromatic_adaptation_matrix_f64(source.whitepoint_xyz(), destination.whitepoint_xyz());
    let mut adapted = [[0.0; 3]; 3];
    let mut result = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            for k in 0..3 {
                adapted[i][j] += cat[i][k] * source.matrix_rgb_to_xyz[k][j];
            }
        }
    }
    for i in 0..3 {
        for j in 0..3 {
            for k in 0..3 {
                result[i][j] += destination.matrix_xyz_to_rgb[i][k] * adapted[k][j];
            }
        }
    }
    result
}

pub fn display_rgb(
    rgb: [f64; 3],
    space: &RgbColorSpace,
    encoded: bool,
    matrix: &[[f64; 3]; 3],
) -> [f64; 3] {
    convert_rgb(rgb, space, encoded, &SRGB, true, matrix)
}

/// Convert a native pixel without clipping or gamut compression. The caller
/// precomputes `conversion_matrix` once for the image's source/destination.
pub fn convert_rgb(
    mut rgb: [f64; 3],
    source: &RgbColorSpace,
    source_encoded: bool,
    destination: &RgbColorSpace,
    destination_encoded: bool,
    matrix: &[[f64; 3]; 3],
) -> [f64; 3] {
    if source_encoded {
        for value in &mut rgb {
            *value = cctf_decode(*value, source.cctf);
        }
    }
    let mut out = [0.0; 3];
    for i in 0..3 {
        let linear = matrix[i][0] * rgb[0] + matrix[i][1] * rgb[1] + matrix[i][2] * rgb[2];
        out[i] = if destination_encoded {
            cctf_encode(linear, destination.cctf)
        } else {
            linear
        };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::precision::{from_f64, srgb_decode, srgb_encode};

    #[test]
    fn piecewise_transfer_boundaries_match_fresh_colour_047() {
        let cases = [
            (
                Cctf::Srgb,
                [0.0031307999, 0.0031308, 0.0031308001],
                [0.040449934708, 0.040449936, 0.04044990875299786],
                [0.0404499359, 0.040449936, 0.0404499361],
                [0.003130799992260062, 0.0031308, 0.00313080225278607],
            ),
            (
                Cctf::Rec2020,
                [0.0179999999, 0.018, 0.0180000001],
                [
                    0.08099999955000001,
                    0.08124794403514049,
                    0.08124794448576036,
                ],
                [0.0809999999, 0.081, 0.0810000001],
                [
                    0.017999999977777778,
                    0.018000000000000002,
                    0.018000000022222223,
                ],
            ),
            (
                Cctf::ProPhoto,
                [0.0019531249, 0.001953125, 0.0019531251],
                [0.0312499984, 0.031249999999999997, 0.03125000088888887],
                [0.0312499999, 0.03125, 0.0312500001],
                [0.00195312499375, 0.0019531249999999998, 0.00195312501125],
            ),
        ];
        for (curve, linear, encoded, encoded_input, decoded) in cases {
            for i in 0..3 {
                assert!((cctf_encode(linear[i], curve) - encoded[i]).abs() < 1e-12);
                assert!((cctf_decode(encoded_input[i], curve) - decoded[i]).abs() < 1e-12);
            }
        }
        assert!(cctf_encode(-0.1, Cctf::AdobeRgb1998).is_nan());
        assert!(cctf_decode(-0.1, Cctf::Gamma2_6).is_nan());
        assert_eq!(cctf_encode(2.0, Cctf::Linear), 2.0);
        assert!(encode_rgb([-0.1, 2.0, 0.18], &SRGB)[0] < 0.0);
        assert!(encode_rgb([-0.1, 2.0, 0.18], &SRGB)[1] > 1.0);
    }
    #[test]
    fn acescct_transfer_matches_transport_curve() {
        for linear in [0.0, 0.001, 0.0078125, 0.18, 1.0, 4.0] {
            let encoded = cctf_encode(linear, Cctf::Acescct);
            let decoded = cctf_decode(encoded, Cctf::Acescct);
            assert!(
                (decoded - linear).abs() < 1e-12,
                "{linear}: {encoded} -> {decoded}"
            );
        }
        assert_eq!(
            cctf_encode(0.18, Cctf::Acescct),
            (0.18f64.log2() + 9.72) / 17.52
        );
    }

    // Generated with fresh colour-science 0.4.7; default stored matrices, CAT02.
    #[test]
    fn seven_spaces_match_fresh_colour_047_reference() {
        let probes = [
            [0.184, 0.184, 0.184],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.9, 0.02, 0.6],
            [-0.1, 0.2, -0.03],
            [1.7, 1.2, 0.4],
        ];
        // Avoid fractional-power sensitivity to roundoff at zero for scan primaries.
        let scan_probes = [
            [0.184, 0.184, 0.184],
            [1.0, 0.01, 0.01],
            [0.01, 1.0, 0.01],
            [0.01, 0.01, 1.0],
            [0.9, 0.02, 0.6],
            [-0.1, 0.2, -0.03],
            [1.7, 1.2, 0.4],
        ];
        // Each probe checks RGB→XYZ, XYZ→RGB, decode, encode, and scan encoding.
        let references = [
            (
                "sRGB",
                [0.9504559270516716, 1.0, 1.0890577507598784],
                [
                    [
                        [0.174892, 0.184, 0.200376],
                        [0.4124, 0.2126, 0.0193],
                        [0.3576, 0.7152, 0.1192],
                        [0.18049999999999997, 0.0722, 0.9505],
                        [0.486612, 0.24896400000000002, 0.590054],
                        [0.024864999999999995, 0.119614, -0.006605000000000001],
                        [1.2024, 1.24854, 0.55605],
                    ],
                    [
                        [0.22168320000000008, 0.1745056, 0.16720079999999998],
                        [3.2406, -0.9689, 0.05569999999999999],
                        [-1.5372, 1.8758, -0.204],
                        [-0.49860000000000004, 0.04150000000000002, 1.057],
                        [2.586636, -0.8095939999999999, 0.6802499999999999],
                        [-0.616542, 0.470805, -0.07808],
                        [3.4649400000000004, 0.6204299999999996, 0.27269000000000004],
                    ],
                    [
                        [
                            0.028336686090670693,
                            0.028336686090670693,
                            0.028336686090670693,
                        ],
                        [1.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0],
                        [0.0, 0.0, 1.0],
                        [
                            0.7874122893956174,
                            0.0015479876160990713,
                            0.31854677812509186,
                        ],
                        [
                            -0.007739938080495357,
                            0.033104766570885055,
                            -0.0023219814241486067,
                        ],
                        [3.3920261118720676, 1.5168374366863642, 0.13286832155381798],
                    ],
                    [
                        [
                            0.46610657463312305,
                            0.46610657463312305,
                            0.46610657463312305,
                        ],
                        [0.9999999999999999, 0.0, 0.0],
                        [0.0, 0.9999999999999999, 0.0],
                        [0.0, 0.0, 0.9999999999999999],
                        [0.9546871718858662, 0.15170371931624205, 0.7977377330312598],
                        [-1.292, 0.48452920448170694, -0.3876],
                        [1.2610513758453459, 1.0832683112047328, 0.6651850846308363],
                    ],
                    [
                        [0.4661098098223811, 0.46611831020243527, 0.46611001609188357],
                        [0.9999964708409027, 0.09999464022460194, 0.09985627267217229],
                        [0.09985378410789228, 1.0000177841449398, 0.09992995964216012],
                        [0.10000162265491591, 0.09980558189017263, 1.0000016145878536],
                        [0.9546901925269963, 0.15177064772940052, 0.797739338071056],
                        [
                            -1.2919983048959998,
                            0.48453610050245605,
                            -0.38757106565999994,
                        ],
                        [1.2610498346447017, 1.0833007381622972, 0.6651973651403305],
                    ],
                ],
            ),
            (
                "DCI-P3",
                [0.8945868945868947, 1.0, 0.9544159544159544],
                [
                    [
                        [0.1646039886039886, 0.18400000000000002, 0.17561253561253562],
                        [
                            0.4451698155645524,
                            0.20949167791273052,
                            -3.701445641729008e-17,
                        ],
                        [0.27713440920677773, 0.7215952541610438, 0.04706056005398116],
                        [0.1722826698155645, 0.06891306792622581, 0.9073553943619733],
                        [0.5095651240815714, 0.24432225596041385, 0.5453544478182636],
                        [
                            0.005741420190433373,
                            0.12130249100314892,
                            -0.017808549820062962,
                        ],
                        [1.158263045434098, 1.2496153846153846, 0.4194148298095667],
                    ],
                    [
                        [0.2331699205497101, 0.168766864414374, 0.1940339341520134],
                        [2.725394030491733, -0.795168025808764, 0.04124189139570003],
                        [-1.018003006227185, 1.6897320548436237, -0.08763901921586238],
                        [
                            -0.44016319519003655,
                            0.022647190608477454,
                            1.1009293786463221,
                        ],
                        [2.1683966502039937, -0.6682682677659286, 0.695922549059606],
                        [
                            -0.46293510843890917,
                            0.41678379783134684,
                            -0.054679874342132145,
                        ],
                        [3.235500966287309, 0.6849516981808403, 0.40531614377218406],
                    ],
                    [
                        [
                            0.012261012630819025,
                            0.012261012630819025,
                            0.012261012630819025,
                        ],
                        [1.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0],
                        [0.0, 0.0, 1.0],
                        [
                            0.7603797186212513,
                            3.825409999160147e-05,
                            0.26496789221441996,
                        ],
                        [f64::NAN, 0.015229231509727027, f64::NAN],
                        [3.9734449052755125, 1.6064648952909515, 0.09233279397806167],
                    ],
                    [
                        [0.5214799608317394, 0.5214799608317394, 0.5214799608317394],
                        [1.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0],
                        [0.0, 0.0, 1.0],
                        [0.9602868133327568, 0.22210073631264332, 0.8216248345594525],
                        [f64::NAN, 0.5384747991560215, f64::NAN],
                        [1.226405813511074, 1.0726408326462522, 0.7029851492201263],
                    ],
                    [
                        [0.5214799608317394, 0.5214799608317393, 0.5214799608317394],
                        [1.0, 0.170125427985259, 0.17012542798525893],
                        [0.17012542798525948, 1.0, 0.170125427985259],
                        [0.17012542798525815, 0.17012542798525898, 1.0],
                        [0.9602868133327567, 0.22210073631264338, 0.8216248345594525],
                        [f64::NAN, 0.5384747991560215, f64::NAN],
                        [1.226405813511074, 1.072640832646252, 0.7029851492201263],
                    ],
                ],
            ),
            (
                "Display P3",
                [0.9504559270516716, 1.0, 1.0890577507598784],
                [
                    [
                        [0.17488389057750756, 0.184, 0.20038662613981761],
                        [
                            0.486570948648216,
                            0.2289745640697487,
                            -4.0456829153654276e-17,
                        ],
                        [0.2656676931690931, 0.6917385218365064, 0.04511338185890264],
                        [0.19821728523436247, 0.079286914093745, 1.0439443689009757],
                        [0.5621575787873937, 0.267484026555751, 0.6272688889777635],
                        [
                            -0.001470074788033855,
                            0.11307164053751408,
                            -0.02229565469524874,
                        ],
                        [1.2252587585986239, 1.2510577507598786, 0.4717138057910734],
                    ],
                    [
                        [
                            0.21333006176117167,
                            0.17605115889416356,
                            0.16864666555695113,
                        ],
                        [
                            2.4934969119414268,
                            -0.8294889695615748,
                            0.035845830243784474,
                        ],
                        [
                            -0.9313836179191246,
                            1.7626640603183463,
                            -0.07617238926804183,
                        ],
                        [
                            -0.4027107844507172,
                            0.023624685841943622,
                            0.9568845240076874,
                        ],
                        [1.9838930777184713, -0.6971119798938842, 0.6048685138386576],
                        [
                            -0.42354509124444606,
                            0.4347729684445684,
                            -0.04752559659821744,
                        ],
                        [2.960200095017189, 0.7145154984641159, 0.3522848538958584],
                    ],
                    [
                        [
                            0.028336686090670693,
                            0.028336686090670693,
                            0.028336686090670693,
                        ],
                        [1.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0],
                        [0.0, 0.0, 1.0],
                        [
                            0.7874122893956174,
                            0.0015479876160990713,
                            0.31854677812509186,
                        ],
                        [
                            -0.007739938080495357,
                            0.033104766570885055,
                            -0.0023219814241486067,
                        ],
                        [3.3920261118720676, 1.5168374366863642, 0.13286832155381798],
                    ],
                    [
                        [
                            0.46610657463312305,
                            0.46610657463312305,
                            0.46610657463312305,
                        ],
                        [0.9999999999999999, 0.0, 0.0],
                        [0.0, 0.9999999999999999, 0.0],
                        [0.0, 0.0, 0.9999999999999999],
                        [0.9546871718858662, 0.15170371931624205, 0.7977377330312598],
                        [-1.292, 0.48452920448170694, -0.3876],
                        [1.2610513758453459, 1.0832683112047328, 0.6651850846308363],
                    ],
                    [
                        [
                            0.46610657463312305,
                            0.46610657463312305,
                            0.46610657463312305,
                        ],
                        [0.9999999999999999, 0.0998528227341283, 0.09985282273412832],
                        [0.09985282273412863, 0.9999999999999999, 0.09985282273412832],
                        [0.09985282273412824, 0.09985282273412846, 0.9999999999999999],
                        [0.9546871718858662, 0.15170371931624207, 0.7977377330312598],
                        [-1.2920000000000003, 0.48452920448170694, -0.3876],
                        [1.2610513758453459, 1.0832683112047328, 0.6651850846308363],
                    ],
                ],
            ),
            (
                "Adobe RGB (1998)",
                [0.9504559270516716, 1.0, 1.0890577507598784],
                [
                    [
                        [0.17488464, 0.18399816000000002, 0.20038704],
                        [0.57667, 0.29734, 0.02703],
                        [0.18555999999999997, 0.62736, 0.07069],
                        [0.18822999999999998, 0.07529, 0.99134],
                        [0.6356522, 0.3253272, 0.6205448],
                        [-0.02620190000000001, 0.0934793, -0.018305199999999997],
                        [1.278303, 1.288426, 0.527315],
                    ],
                    [
                        [
                            0.20826039999999996,
                            0.17448535999999995,
                            0.16748599999999997,
                        ],
                        [2.04159, -0.96924, 0.013439999999999997],
                        [-0.5650100000000001, 1.87597, -0.11836],
                        [-0.34473000000000004, 0.04156000000000002, 1.01517],
                        [1.6192927999999998, -0.8098606, 0.6188307999999999],
                        [-0.30681909999999996, 0.47087120000000005, -0.0554711],
                        [2.6547989999999997, 0.6200799999999997, 0.28688400000000003],
                    ],
                    [
                        [
                            0.024164263991590273,
                            0.024164263991590273,
                            0.024164263991590273,
                        ],
                        [1.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0],
                        [0.0, 0.0, 1.0],
                        [
                            0.7931754593999576,
                            0.00018348193467219382,
                            0.3251667049779579,
                        ],
                        [f64::NAN, 0.029027662219974663, f64::NAN],
                        [3.2122370637626414, 1.4932649827486288, 0.1333039049218824],
                    ],
                    [
                        [
                            0.46313496660583076,
                            0.46313496660583076,
                            0.46313496660583076,
                        ],
                        [1.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0],
                        [0.0, 0.0, 1.0],
                        [0.9532213304117194, 0.16883658916733524, 0.7927273402455737],
                        [f64::NAN, 0.48103147826312764, f64::NAN],
                        [1.2728778257900653, 1.0864362831342662, 0.6592557425266524],
                    ],
                    [
                        [0.46313783646174383, 0.4631325918727544, 0.46313420978431225],
                        [1.0000025738541307, 0.12315166402779015, 0.12317800925850116],
                        [0.12321488417359791, 0.9999973563401929, 0.12320372278842925],
                        [0.12321733203076213, 0.12320252946374977, 0.9999986847664512],
                        [0.9532250121183967, 0.16881498004918596, 0.7927248787844696],
                        [f64::NAN, 0.4810309619162586, f64::NAN],
                        [1.2728831872312345, 1.0864285991450213, 0.6592532390896595],
                    ],
                ],
            ),
            (
                "ITU-R BT.2020",
                [0.9504559270516716, 1.0, 1.0890577507598784],
                [
                    [
                        [
                            0.17488389057750758,
                            0.18400000000000002,
                            0.20038662613981761,
                        ],
                        [
                            0.6369580483012912,
                            0.262700212011267,
                            4.9002699185383275e-17,
                        ],
                        [
                            0.14461690358620838,
                            0.6779980715188711,
                            0.028072693049087445,
                        ],
                        [0.16888097516417205, 0.05930171646986195, 1.0609850577107909],
                        [0.6774831666413895, 0.2855711821224349, 0.6371524884874563],
                        [
                            -0.03983885336781261,
                            0.10755054160855167,
                            -0.02621501312150624,
                        ],
                        [1.3239213564813141, 1.2839087328297438, 0.45808125474322137],
                    ],
                    [
                        [0.2038009985991037, 0.17766403923341043, 0.16872291523782762],
                        [1.7166511879712678, -0.6666843518324888, 0.01763985744531079],
                        [
                            -0.3556707837763925,
                            1.6164812366349386,
                            -0.04277061325780853,
                        ],
                        [
                            -0.25336628137365985,
                            0.01576854581391113,
                            0.9421031212354739,
                        ],
                        [1.3858528846744174, -0.5582251644281944, 0.5802823321769078],
                        [-0.2351982871111955, 0.3894916261358193, -0.038581202033157],
                        [2.3901555664700203, 0.8127215041722597, 0.35550427024184766],
                    ],
                    [
                        [
                            0.049050298124240166,
                            0.049050298124240166,
                            0.049050298124240166,
                        ],
                        [1.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0],
                        [0.0, 0.0, 1.0],
                        [0.8089625839151658, 0.0044444444444444444, 0.365837095782347],
                        [
                            -0.022222222222222223,
                            0.055426681811288486,
                            -0.006666666666666666,
                        ],
                        [2.9897159981443773, 1.4499692663588764, 0.1729841613701762],
                    ],
                    [
                        [0.4140571127064271, 0.4140571127064271, 0.4140571127064271],
                        [1.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0],
                        [0.0, 0.0, 1.0],
                        [0.9491098963971183, 0.08999973292453692, 0.7743046147819846],
                        [-0.45, 0.4336736212880552, -0.135],
                        [1.2964031757616288, 1.093969260201581, 0.6286536103906286],
                    ],
                    [
                        [0.4140571127064271, 0.4140571127064271, 0.414057112706427],
                        [1.0, 0.044999999999999894, 0.045],
                        [0.04499999999999985, 1.0, 0.04499999999999999],
                        [0.04500000000000003, 0.04500000000000004, 0.9999999999999998],
                        [0.9491098963971183, 0.08999973292453686, 0.7743046147819846],
                        [-0.45, 0.4336736212880552, -0.13499999999999998],
                        [1.2964031757616288, 1.093969260201581, 0.6286536103906286],
                    ],
                ],
            ),
            (
                "ProPhoto RGB",
                [0.9642956764295677, 1.0, 0.8251046025104605],
                [
                    [
                        [0.17741279999999998, 0.18399999999999997, 0.1517816],
                        [0.7977, 0.288, -1.1472736502440068e-18],
                        [0.13519999999999996, 0.7119, -6.456940471330267e-19],
                        [0.03129999999999999, 0.00010000000000000009, 0.8249],
                        [0.739414, 0.273498, 0.49493999999999994],
                        [-0.05366900000000001, 0.113577, -0.024746999999999998],
                        [1.53085, 1.34392, 0.32996000000000003],
                    ],
                    [
                        [0.19123120000000002, 0.1810744, 0.2230632],
                        [1.346, -0.5446, -1.4457073492523394e-18],
                        [-0.25560000000000005, 1.5082, -8.2499685309798e-19],
                        [-0.05110000000000003, 0.02050000000000001, 1.2123],
                        [1.1756280000000001, -0.44767599999999996, 0.7273799999999999],
                        [-0.18418700000000002, 0.35548500000000005, -0.036369],
                        [1.9610400000000001, 0.8922199999999999, 0.48492],
                    ],
                    [
                        [
                            0.047497645458177196,
                            0.047497645458177196,
                            0.047497645458177196,
                        ],
                        [1.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0],
                        [0.0, 0.0, 1.0],
                        [0.8272495069561094, 0.00125, 0.3987238835693843],
                        [-0.00625, 0.05518918645844859, -0.001875],
                        [2.599010646182191, 1.388437205763783, 0.192179909437029],
                    ],
                    [
                        [
                            0.39045002702761455,
                            0.39045002702761455,
                            0.39045002702761455,
                        ],
                        [1.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0],
                        [0.0, 0.0, 1.0],
                        [0.9431465314595465, 0.11379620405527816, 0.7529232265121797],
                        [-1.6, 0.4089623530229582, -0.48],
                        [1.3428489952192484, 1.1065972360403842, 0.6010660762800317],
                    ],
                    [
                        [0.3904632175527825, 0.3904515736411044, 0.39045572539557677],
                        [1.0000506068089172, 0.07714597750736601, 0.07742749825634634],
                        [0.07750373145742814, 1.0000317522078308, 0.07742749825634634],
                        [0.07722372743089957, 0.07749176457403711, 1.0000145943592464],
                        [0.9431778069480611, 0.11364151261829827, 0.7529342149442324],
                        [-1.6000669360000004, 0.408982409165724, -0.4800126096],
                        [1.3429179758669865, 1.1065784924103925, 0.6010748484542798],
                    ],
                ],
            ),
            (
                "ACES2065-1",
                [0.9526460745698463, 1.0, 1.0088251843515854],
                [
                    [
                        [0.17528687770799997, 0.184, 0.1856238339296],
                        [0.9525523959, 0.3439664498, -1.3700265923920138e-18],
                        [
                            -2.1301316561213198e-17,
                            0.7281660966,
                            -4.955330679102859e-19,
                        ],
                        [9.367859999998151e-05, -0.0721325464, 1.0088251844],
                        [0.85735336347, 0.280853598912, 0.60529511064],
                        [-0.095258049948, 0.113400550732, -0.030264755532],
                        [1.61937654447, 1.42968926202, 0.40353007376000005],
                    ],
                    [
                        [0.193147290072, 0.1795196108192, 0.18239037134879998],
                        [1.0498110175, -0.4959030231, -1.1821004105196352e-18],
                        [-3.071045000699565e-17, 1.3733130458, -6.745688325018738e-19],
                        [-9.748450000002144e-05, 0.09824003610000001, 0.9912520182],
                        [0.94477142505, -0.359902438214, 0.5947512109199999],
                        [-0.104978177215, 0.321305710387, -0.029737560546],
                        [1.7846397359499997, 0.84423653013, 0.39650080728000003],
                    ],
                    [
                        [0.184, 0.184, 0.184],
                        [1.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0],
                        [0.0, 0.0, 1.0],
                        [0.9, 0.02, 0.6],
                        [-0.1, 0.2, -0.03],
                        [1.7, 1.2, 0.4],
                    ],
                    [
                        [0.184, 0.184, 0.184],
                        [1.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0],
                        [0.0, 0.0, 1.0],
                        [0.9, 0.02, 0.6],
                        [-0.1, 0.2, -0.03],
                        [1.7, 1.2, 0.4],
                    ],
                    [
                        [
                            0.18399999999439481,
                            0.18400000000830713,
                            0.18400000000873762,
                        ],
                        [0.9999999999619187, 0.01000000003994535, 0.01000000000047487],
                        [
                            0.009999999999695348,
                            0.9999999999707941,
                            0.01000000000047487,
                        ],
                        [
                            0.010000000007313733,
                            0.010000000035310926,
                            1.0000000000474871,
                        ],
                        [0.8999999999702748, 0.020000000056431325, 0.6000000000284923],
                        [
                            -0.09999999999641504,
                            0.199999999988963,
                            -0.030000000001424613,
                        ],
                        [1.6999999999382092, 1.200000000045954, 0.4000000000189949],
                    ],
                ],
            ),
        ];
        let assert_reference = |actual: f64, expected: f64, label: &str| {
            if expected.is_nan() {
                assert!(actual.is_nan(), "{label}: expected NaN, got {actual}");
            } else {
                assert!(
                    (actual - expected).abs() <= 2e-12 * expected.abs().max(1.0),
                    "{label}: got {actual}, expected {expected}"
                );
            }
        };
        for (name, white, expected) in references {
            let space = resolve(name).unwrap();
            for channel in 0..3 {
                assert_reference(space.whitepoint_xyz()[channel], white[channel], name);
            }
            for (index, probe) in probes.iter().copied().enumerate() {
                let multiply = |matrix: &[[f64; 3]; 3]| {
                    matrix.map(|row| row[0] * probe[0] + row[1] * probe[1] + row[2] * probe[2])
                };
                let actual = [
                    multiply(&space.matrix_rgb_to_xyz),
                    multiply(&space.matrix_xyz_to_rgb),
                    probe.map(|v| cctf_decode(v, space.cctf)),
                    probe.map(|v| cctf_encode(v, space.cctf)),
                    encode_rgb(scan_probes[index], space),
                ];
                for operation in 0..5 {
                    for channel in 0..3 {
                        let label = format!(
                            "{name}, probe {index}, operation {operation}, channel {channel}"
                        );
                        assert_reference(
                            actual[operation][channel],
                            expected[operation][index][channel],
                            &label,
                        );
                    }
                }
            }
        }
    }

    /// CAT16 adaptation matches colour-science's
    /// `matrix_chromatic_adaptation_VonKries(transform="CAT16")` for the
    /// ProPhoto-white → D55 case used by the input projection.
    #[test]
    fn cat16_matches_colour() {
        let src = [0.9642956764295677, 1.0, 0.8251046025104605]; // ProPhoto white
        let dst = [0.9567982961086806, 1.0, 0.9213965001151279]; // D55
        let got = chromatic_adaptation_matrix_cat16_f64(src, dst);
        let want = [
            [
                0.9955740273098072,
                -0.016544957254590615,
                0.01613798204777648,
            ],
            [
                -0.0022206862456965637,
                1.0026392099167525,
                -0.0006033317106774956,
            ],
            [
                -0.00013585839424725028,
                0.0054854466982645,
                1.1102132484680078,
            ],
        ];
        for i in 0..3 {
            for j in 0..3 {
                assert!(
                    (got[i][j] - want[i][j]).abs() < 1e-12,
                    "[{i}][{j}] got {} want {}",
                    got[i][j],
                    want[i][j]
                );
            }
        }
    }

    #[test]
    fn test_srgb_roundtrip() {
        for &v in &[0.0_f64, 0.001, 0.01, 0.1, 0.18, 0.5, 0.9, 1.0] {
            let s = from_f64(v);
            let encoded = srgb_encode(s);
            let decoded = srgb_decode(encoded);
            assert!(
                (s - decoded).abs() < from_f64(1e-5),
                "roundtrip failed for {v}: encoded={encoded}, decoded={decoded}"
            );
        }
    }

    #[test]
    fn test_identity_adaptation() {
        let m = chromatic_adaptation_matrix(D65_XYZ, D65_XYZ);
        for i in 0..3 {
            for j in 0..3 {
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!(
                    (m[i][j] - expected).abs() < 1e-4,
                    "identity adaptation [{i}][{j}]: expected {expected}, got {}",
                    m[i][j]
                );
            }
        }
    }

    #[test]
    fn test_srgb_xyz_roundtrip() {
        let rgb = [0.5, 0.3, 0.8];
        let xyz = mat3_mul(&SRGB_TO_XYZ, rgb);
        let rgb2 = mat3_mul(&XYZ_TO_SRGB, xyz);
        for i in 0..3 {
            assert!(
                (rgb[i] - rgb2[i]).abs() < 1e-5,
                "RGB roundtrip failed channel {i}: {:.6} vs {:.6}",
                rgb[i],
                rgb2[i]
            );
        }
    }
}
