// Output gamut compression — GPU port of
// `spektrafilm-core/src/gamut_compression.rs` (`OutputGamutCompress::compress`).
//
// Runs in-place on the unclipped linear scan output RGB. Compression needs
// negative and above-one values preserved by scan_spectral. Mode selects the
// algorithm; the perceptual modes
// share the baked `C_max(L, h)` table uploaded by the CPU:
//   0 = aces_rgc   (per-channel Reinhard knee, no table)
//   1 = oklch      (OkLab chroma reduction)
//   2 = oklrab     (OkLab coords, table indexed by Ottosson Lr lightness)
//   3 = cam16ucs   (full CIECAM16 + Luo 2006 UCS)
//   4 = jzazbz     (Safdar 2017, binary64 forward/inverse transform)
//
// JzAzBz runs its forward/inverse transform in binary64 (see the
// `jzazbz.wgsl` section below). Its `b` row is [0.199076, 1.096799,
// -1.295875] over cone responses near 0.3, so the three terms cancel by
// ~105x to leave a value near 0.0075. An f32 rounding at the input becomes
// ~3e-6 at the output, which rotates the hue by ~1.5e-4 rad and moves the
// C_max lookup by ~0.012 of a row. The JzAzBz C_max table has a hard
// discontinuity between hue bins 692 and 693 (0.0234 versus 0.0), so that
// shift is a ~9% table error and ~1.6e-3 of output RGB, against a 1e-4
// budget — measured, not estimated. f64 keeps that mode on the device,
// matching the CPU reference: both the transform and the lookup index.
//
// Every destination-specific constant arrives as a uniform: the CPU resolves
// the RGB↔XYZ matrices from `RgbColorSpace::matrix_rgb_to_xyz` /
// `matrix_xyz_to_rgb` and the CIECAM16 viewing conditions from
// `Cam16Viewing::new(destination.whitepoint_xyz())`. Nothing about the
// destination may be baked here — sRGB's D65 matrices and viewing conditions
// were once hardcoded and confined the pass to that one space.
//
// f32 throughout, except the JzAzBz transform (mode 4), which is binary64.
// The pass runs on the device for every mode; exports run the same kernel
// through the resident chain.

struct Params {
    n_pixels: u32,
    mode: u32,
    n_l: u32,
    n_h: u32,
    // x=threshold, y=limit, z=power (Reinhard knee on normalized chroma).
    knee: vec4<f32>,
    // x=l_min, y=l_max (table L-axis bounds).
    lspace: vec4<f32>,
    // Destination linear RGB → XYZ, at the destination whitepoint. Columns of
    // the CPU's row-major matrix, so `m * rgb` is the matrix-vector product.
    rgb_to_xyz: mat3x3<f32>,
    // Destination XYZ → linear RGB (the stored inverse, not a recomputed one).
    xyz_to_rgb: mat3x3<f32>,
    // CIECAM16 viewing conditions for the destination white:
    // xyz = degree-of-adaptation diagonal, w = achromatic response A_w.
    cam16: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> cmax: array<f32>;       // [n_l*n_h]
@group(0) @binding(2) var<storage, read_write> img: array<f32>;  // [H*W*3]

const PI: f32 = 3.141592653589793;

// OkLab (Ottosson) matrices.
const M1_R0 = vec3<f32>(0.8189330101, 0.3618667424, -0.1288597137);
const M1_R1 = vec3<f32>(0.0329845436, 0.9293118715, 0.0361456387);
const M1_R2 = vec3<f32>(0.0482003018, 0.2643662691, 0.633851707);
const M1I_R0 = vec3<f32>(1.2270138511, -0.5577999807, 0.2812561490);
const M1I_R1 = vec3<f32>(-0.0405801784, 1.1122568696, -0.0716766787);
const M1I_R2 = vec3<f32>(-0.0763812845, -0.4214819784, 1.5861632204);
const M2_R0 = vec3<f32>(0.2104542553, 0.793617785, -0.0040720468);
const M2_R1 = vec3<f32>(1.9779984951, -2.428592205, 0.4505937099);
const M2_R2 = vec3<f32>(0.0259040371, 0.7827717662, -0.808675766);
const M2I_R0 = vec3<f32>(1.0, 0.3963377922, 0.2158037581);
const M2I_R1 = vec3<f32>(1.0, -0.1055613423, -0.0638541748);
const M2I_R2 = vec3<f32>(1.0, -0.0894841821, -1.2914855379);

// CAM16 viewing-condition constants (L_A = 64 cd/m², Y_b = 20, Average).
// M16 and its inverse are whitepoint-independent; D_rgb and A_w are not, so
// those two arrive per destination in `params.cam16`.
const CAM16_M16_R0 = vec3<f32>(0.401288, 0.650173, -0.051461);
const CAM16_M16_R1 = vec3<f32>(-0.250268, 1.204414, 0.045854);
const CAM16_M16_R2 = vec3<f32>(-0.002079, 0.048952, 0.953127);
const CAM16_M16I_R0 = vec3<f32>(1.8620678551, -1.0112546305, 0.1491867754);
const CAM16_M16I_R1 = vec3<f32>(0.3875265432, 0.6214474419, -0.0089739852);
const CAM16_M16I_R2 = vec3<f32>(-0.0158414988, -0.0341229380, 1.0499644369);
const CAM16_F_L: f32 = 0.6839903846;
const CAM16_N_BB: f32 = 1.0003040046;
const CAM16_CZ: f32 = 0.69 * 1.9272135955; // c * z
// (50000/13) * N_c * N_cb with N_c = 1, N_cb = N_BB.
const CAM16_ECC: f32 = 3846.1538461538 * 1.0003040046;
// (1.64 - 0.29^n)^0.73 and F_L^0.25 — const-folded at shader compile.
const CAM16_CC_F: f32 = pow(1.64 - pow(0.29, 0.2), 0.73);
const CAM16_F_L_4: f32 = pow(0.6839903846, 0.25);
const UCS_C1: f32 = 0.007;
const UCS_C2: f32 = 0.0228;

// Ottosson Lr lightness rebase (oklrab table index).
const OKLRAB_K1: f32 = 0.206;
const OKLRAB_K2: f32 = 0.03;
const OKLRAB_K3: f32 = (1.0 + 0.206) / (1.0 + 0.03);

fn mv(r0: vec3<f32>, r1: vec3<f32>, r2: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(r0, v), dot(r1, v), dot(r2, v));
}

fn cbrt(x: f32) -> f32 {
    return sign(x) * pow(abs(x), 1.0 / 3.0);
}

// Smooth Reinhard knee: identity below threshold, asymptotic at limit.
fn reinhard_knee(d: f32, threshold: f32, limit: f32, power: f32) -> f32 {
    if d <= threshold {
        return d;
    }
    let scale = limit - threshold;
    let x = (d - threshold) / scale;
    let y = x / pow(1.0 + pow(x, power), 1.0 / power);
    return threshold + scale * y;
}

fn xyz_to_oklab(xyz: vec3<f32>) -> vec3<f32> {
    let lms = mv(M1_R0, M1_R1, M1_R2, xyz);
    let lms_ = vec3<f32>(cbrt(lms.x), cbrt(lms.y), cbrt(lms.z));
    return mv(M2_R0, M2_R1, M2_R2, lms_);
}

fn oklab_to_xyz(lab: vec3<f32>) -> vec3<f32> {
    let lms_ = mv(M2I_R0, M2I_R1, M2I_R2, lab);
    let lms = lms_ * lms_ * lms_;
    return mv(M1I_R0, M1I_R1, M1I_R2, lms);
}

fn oklab_l_to_lr(l: f32) -> f32 {
    let t = OKLRAB_K3 * l - OKLRAB_K1;
    return 0.5 * (t + sqrt(t * t + 4.0 * OKLRAB_K2 * OKLRAB_K3 * l));
}

// atan2 in degrees, wrapped to ~[0, 360) (f32 rounding can yield exactly
// 360 for tiny negative angles — harmless, every consumer is periodic).
// atan2(0, 0) is indeterminate in WGSL and would propagate NaN through the
// whole CAM16 chain (the t == 0 rescue doesn't catch NaN); Rust's f64
// atan2(0, 0) returns 0, so match that explicitly.
fn atan2_deg(y: f32, x: f32) -> f32 {
    if y == 0.0 && x == 0.0 {
        return 0.0;
    }
    let d = degrees(atan2(y, x));
    return select(d, d + 360.0, d < 0.0);
}

fn cam16_padc_forward(rgb: vec3<f32>) -> vec3<f32> {
    let flr = pow(CAM16_F_L * abs(rgb) / 100.0, vec3<f32>(0.42));
    return 400.0 * sign(rgb) * flr / (27.13 + flr) + 0.1;
}

fn cam16_padc_inverse(rgb: vec3<f32>) -> vec3<f32> {
    let d = rgb - 0.1;
    let base = (27.13 * abs(d)) / (400.0 - abs(d));
    return sign(d) * 100.0 / CAM16_F_L * pow(base, vec3<f32>(1.0 / 0.42));
}

// XYZ (Y=1) → CAM16-UCS (Jp, ap, bp).
fn xyz_to_cam16ucs(xyz: vec3<f32>) -> vec3<f32> {
    let d_rgb = params.cam16.xyz;
    let a_w = params.cam16.w;
    let rgb = mv(CAM16_M16_R0, CAM16_M16_R1, CAM16_M16_R2, xyz * 100.0);
    let p = cam16_padc_forward(rgb * d_rgb);
    let a = p.x - 12.0 * p.y / 11.0 + p.z / 11.0;
    let b = (p.x + p.y - 2.0 * p.z) / 9.0;
    let h = atan2_deg(b, a);
    let e_t = 0.25 * (cos(2.0 + radians(h)) + 3.8);
    let a_resp = (2.0 * p.x + p.y + p.z / 20.0 - 0.305) * CAM16_N_BB;
    let jj = 100.0 * pow(a_resp / a_w, CAM16_CZ);
    let denom = p.x + p.y + 21.0 * p.z / 20.0;
    var t = 0.0;
    if denom != 0.0 {
        t = CAM16_ECC * (e_t * sqrt(a * a + b * b)) / denom;
    }
    let cc = pow(t, 0.9) * sqrt(jj / 100.0) * CAM16_CC_F;
    let m = cc * CAM16_F_L_4;
    let jp = (1.0 + 100.0 * UCS_C1) * jj / (1.0 + UCS_C1 * jj);
    let mp = (1.0 / UCS_C2) * log(1.0 + UCS_C2 * m);
    let hr = radians(h);
    return vec3<f32>(jp, mp * cos(hr), mp * sin(hr));
}

// CAM16-UCS (Jp, ap, bp) → XYZ (Y=1).
fn cam16ucs_to_xyz(jab: vec3<f32>) -> vec3<f32> {
    let d_rgb = params.cam16.xyz;
    let a_w = params.cam16.w;
    let jp = jab.x;
    let mp = length(jab.yz);
    let h = atan2_deg(jab.z, jab.y);
    let jj = jp / ((1.0 + 100.0 * UCS_C1) - UCS_C1 * jp);
    let m = (exp(UCS_C2 * mp) - 1.0) / UCS_C2;
    let cc = m / CAM16_F_L_4;
    let j_prime = max(jj, 1.1920929e-7);
    let t = pow(cc / (sqrt(j_prime / 100.0) * CAM16_CC_F), 1.0 / 0.9);
    let e_t = 0.25 * (cos(2.0 + radians(h)) + 3.8);
    let a_resp = a_w * pow(jj / 100.0, 1.0 / CAM16_CZ);
    var p1 = 0.0;
    if t != 0.0 {
        p1 = CAM16_ECC * e_t / t;
    }
    let p2 = a_resp / CAM16_N_BB + 0.305;
    // Inverse opponent dimensions (p3 = 21/20).
    let hr = radians(h);
    let sh = sin(hr);
    let ch = cos(hr);
    let nn = p2 * (2.0 + 21.0 / 20.0) * (460.0 / 1403.0);
    var a = 0.0;
    var b = 0.0;
    if abs(sh) >= abs(ch) {
        var p4 = 0.0;
        if sh != 0.0 {
            p4 = p1 / sh;
        }
        b = nn
            / (p4 + (2.0 + 21.0 / 20.0) * (220.0 / 1403.0) * (ch / sh) - (27.0 / 1403.0)
                + (21.0 / 20.0) * (6300.0 / 1403.0));
        a = b * (ch / sh);
    } else {
        var p5 = 0.0;
        if ch != 0.0 {
            p5 = p1 / ch;
        }
        a = nn
            / (p5 + (2.0 + 21.0 / 20.0) * (220.0 / 1403.0)
                - ((27.0 / 1403.0) - (21.0 / 20.0) * (6300.0 / 1403.0)) * (sh / ch));
        b = a * (sh / ch);
    }
    // Achromatic guard: hue is undefined at t == 0.
    if t == 0.0 {
        a = 0.0;
        b = 0.0;
    }
    let ra = (460.0 * p2 + 451.0 * a + 288.0 * b) / 1403.0;
    let ga = (460.0 * p2 - 891.0 * a - 261.0 * b) / 1403.0;
    let ba = (460.0 * p2 - 220.0 * a - 6300.0 * b) / 1403.0;
    let rgb = cam16_padc_inverse(vec3<f32>(ra, ga, ba)) / d_rgb;
    return mv(CAM16_M16I_R0, CAM16_M16I_R1, CAM16_M16I_R2, rgb) / 100.0;
}

// Bilinear C_max(L, h) lookup — L clamped to the table bounds, h wraps.
fn c_max_lookup(l_in: f32, h: f32) -> f32 {
    let l_min = params.lspace.x;
    let l_max = params.lspace.y;
    let n_l = i32(params.n_l);
    let n_h = i32(params.n_h);
    let l = clamp(l_in, l_min, l_max);

    let h_step = 2.0 * PI / f32(n_h);
    let h_idx = (h + PI) / h_step;
    let h_floor = floor(h_idx);
    let h_lo = ((i32(h_floor) % n_h) + n_h) % n_h;
    let h_hi = (h_lo + 1) % n_h;
    let h_frac = h_idx - h_floor;

    let l_idx = (l - l_min) / (l_max - l_min) * f32(n_l - 1);
    let l_lo = clamp(i32(floor(l_idx)), 0, n_l - 2);
    let l_hi = l_lo + 1;
    let l_frac = l_idx - f32(l_lo);

    let v00 = cmax[l_lo * n_h + h_lo];
    let v01 = cmax[l_lo * n_h + h_hi];
    let v10 = cmax[l_hi * n_h + h_lo];
    let v11 = cmax[l_hi * n_h + h_hi];
    return v00 * (1.0 - l_frac) * (1.0 - h_frac)
        + v01 * (1.0 - l_frac) * h_frac
        + v10 * l_frac * (1.0 - h_frac)
        + v11 * l_frac * h_frac;
}

// ACES RGC v1.3: per-channel Reinhard knee on achromatic distance.
fn compress_aces_rgc(rgb: vec3<f32>) -> vec3<f32> {
    let ach = max(rgb.x, max(rgb.y, rgb.z));
    if ach <= 1e-12 {
        return rgb;
    }
    let d = (ach - rgb) / ach;
    let dc = vec3<f32>(
        reinhard_knee(d.x, params.knee.x, params.knee.y, params.knee.z),
        reinhard_knee(d.y, params.knee.x, params.knee.y, params.knee.z),
        reinhard_knee(d.z, params.knee.x, params.knee.y, params.knee.z),
    );
    return ach * (1.0 - dc);
}

fn compress_perceptual(rgb: vec3<f32>) -> vec3<f32> {
    let xyz = params.rgb_to_xyz * rgb;
    var lab: vec3<f32>;
    if params.mode == 3u {
        lab = xyz_to_cam16ucs(xyz);
    } else {
        lab = xyz_to_oklab(xyz);
    }
    let l = lab.x;
    let c = length(lab.yz);
    // Guard h at c == 0: atan2(0, 0) is indeterminate in WGSL and would
    // propagate NaN through cos/sin (Rust's f64 atan2(0,0) returns 0).
    var h = 0.0;
    if c > 0.0 {
        h = atan2(lab.z, lab.y);
    }
    // oklrab indexes the table by Ottosson's rebased Lr lightness.
    var lookup_l = l;
    if params.mode == 2u {
        lookup_l = oklab_l_to_lr(l);
    }
    let safe = max(c_max_lookup(lookup_l, h), 1e-9);
    let d_comp = reinhard_knee(c / safe, params.knee.x, params.knee.y, params.knee.z);
    let c_new = d_comp * safe;
    let lab_new = vec3<f32>(l, c_new * cos(h), c_new * sin(h));
    var xyz_new: vec3<f32>;
    if params.mode == 3u {
        xyz_new = cam16ucs_to_xyz(lab_new);
    } else {
        xyz_new = oklab_to_xyz(lab_new);
    }
    return params.xyz_to_rgb * xyz_new;
}
