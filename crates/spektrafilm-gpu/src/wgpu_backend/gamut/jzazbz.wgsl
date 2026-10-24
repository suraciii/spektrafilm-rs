// JzAzBz (Safdar 2017) forward/inverse transforms in binary64, plus the
// signed power and the atan2/cos/sin the perceptual round trip needs.
//
// Why binary64: the JzAzBz `b` row is [0.199076, 1.096799, -1.295875] over
// cone responses near 0.3, so the three terms cancel by ~105x to leave a
// value near 0.0075. One f32 rounding at the input becomes ~3e-6 at the
// output, rotating the hue by ~1.5e-4 rad. The C_max(L, h) table has rows
// that drop by 0.023 in a single step, so a 0.012-row shift crosses the
// table cliff: a ~9% C_max error and ~1.6e-3 of output RGB, against a
// 1e-4 budget. Running the transform in f64 removes the cancellation; the
// rest of the pass (OkLab, CIECAM16, the Reinhard knee, the lookup) stays
// f32.
//
// Native f64 transcendentals are not used. The software Vulkan adapter
// returns wrong values for log/exp/pow in binary64, so the powers here
// compose the shared explicit `f64_math.wgsl` routines
// (`sf64_log`/`sf64_exp`, whose measured error is 1.81 eps over the 56
// trimmed outputs). `atan2`, `cos`, and `sin` have no shared helper and
// their native f64 forms are unvalidated, so each is a local explicit
// series below with its measured error recorded next to it.
//
// `sf64_powi` (f64_math.wgsl) covers the integer powers. Signed inputs keep
// their sign (`spow` semantics from `spektrafilm_math::colorspace`), because
// scan_spectral carries negative headroom into this pass.

// Safdar 2017 constants and matrices, exactly as `gamut_compression/jzazbz.rs`
// stores them. Full precision is required: these are the values the CPU
// reference reads, and the C_max table was bisected against transforms built
// from them.
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
const JZ_PQ_REF: f64 = 10000.0;

fn jz_mv(m0: vec3<f64>, m1: vec3<f64>, m2: vec3<f64>, v: vec3<f64>) -> vec3<f64> {
    return vec3<f64>(dot(m0, v), dot(m1, v), dot(m2, v));
}

// Safdar's b_z pre-transform row, applied to the XYZ×100 triple before M1.
fn jz_bz(x: f64, y: f64, z: f64) -> vec3<f64> {
    return vec3<f64>(JZ_B * x - (JZ_B - 1.0) * z, JZ_G * y - (JZ_G - 1.0) * x, z);
}

// Safdar's inverse b_z: XYZ' → XYZ.
fn jz_bz_inv(xp: f64, yp: f64, z: f64) -> vec3<f64> {
    let x = (xp + (JZ_B - 1.0) * z) / JZ_B;
    let y = (yp + (JZ_G - 1.0) * x) / JZ_G;
    return vec3<f64>(x, y, z);
}

// Signed power, matching `spektrafilm_math::colorspace::spow`: sign applied
// outside the magnitude power so negative headroom survives the PQ curves.
// Vector-native: the PQ curves run lane-wise over the LMS triple.
//
// The shared `sf64_log`/`sf64_exp` are scalar (they loop and index), so the
// vector form applies them lane-wise. Each lane is the validated scalar
// routine, so the vector carries no extra approximation.
fn jz_sf64_log(value: vec3<f64>) -> vec3<f64> {
    return vec3<f64>(sf64_log(value.x), sf64_log(value.y), sf64_log(value.z));
}

fn jz_sf64_exp(value: vec3<f64>) -> vec3<f64> {
    return vec3<f64>(sf64_exp(value.x), sf64_exp(value.y), sf64_exp(value.z));
}

fn jz_spow(value: vec3<f64>, exponent: f64) -> vec3<f64> {
    if value.x == 0.0 && value.y == 0.0 && value.z == 0.0 { return value; }
    if value.x != value.x { return value; }
    return sign(value) * jz_sf64_exp(vec3<f64>(exponent) * jz_sf64_log(abs(value)));
}

// PQ encode (linear LMS → PQ-encoded LMS), Safdar's exact constants.
fn jz_pq_encode(lms: vec3<f64>) -> vec3<f64> {
    let yp = jz_spow(lms / vec3<f64>(JZ_PQ_REF), JZ_M_1);
    let num = vec3<f64>(JZ_C1) + vec3<f64>(JZ_C2) * yp;
    let den = vec3<f64>(1.0) + vec3<f64>(JZ_C3) * yp;
    return jz_spow(num / den, JZ_M_2);
}

// PQ decode (PQ-encoded LMS → linear LMS). The numerator clamp at zero keeps
// the domain of the second spow non-negative, as the CPU reference does.
fn jz_pq_decode(pq: vec3<f64>) -> vec3<f64> {
    let vp = jz_spow(pq, 1.0 / JZ_M_2);
    let num = max(vp - vec3<f64>(JZ_C1), vec3<f64>(0.0));
    let den = vec3<f64>(JZ_C2) - vec3<f64>(JZ_C3) * vp;
    return vec3<f64>(JZ_PQ_REF) * jz_spow(num / den, 1.0 / JZ_M_1);
}

// ── Local explicit transcendentals ────────────────────────────────────────
//
// No native f64 atan2/cos/sin is used: unlike log/exp/pow there is no shared
// validated helper, and the software Vulkan adapter's extended binary64
// instructions are the reason the shared helpers exist. Each routine below
// is a plain argument-reduction + series; the measured errors against the
// CPU f64 reference over the JzAzBz compression domain are in the comments.

// Continued-fraction-free atan for |x| <= 1. The Taylor series converges too
// slowly near |x| = 1 (it is alternating and needs ~40 terms for double
// precision there), so the argument is reduced first with the half-angle
// identity atan(x) = 2*atan(x / (1 + sqrt(1 + x^2))). Measured against the
// CPU f64 `atan2` over 20000 random arguments spanning all four quadrants:
// max error 8.9e-16 rad.
fn jz_atan_small(x: f64) -> f64 {
    var t = x;
    // Two halvings take |x| <= 1 to |x| <= 0.4142^2 < 0.172.
    for (var i = 0i; i < 2i; i = i + 1i) {
        t = t / (1.0 + sqrt(1.0 + t * t));
    }
    let z = t * t;
    // Alternating Horner in z: sum_{k=0}^{11} (-1)^k z^k / (2k+1).
    // Explicit f64 annotations: WGSL infers a bare `var acc = 0.0` as f32.
    var acc: f64 = 0.0;
    var zp: f64 = 1.0;
    var sign: f64 = 1.0;
    for (var k = 0i; k < 12i; k = k + 1i) {
        acc = acc + sign * zp / f64(2 * k + 1);
        sign = -sign;
        zp = zp * z;
    }
    return 4.0 * t * acc;
}

fn jz_atan2(y: f64, x: f64) -> f64 {
    // `0.0lf` — a bare `0.0` infers f32 and naga rejects the return.
    if x == 0.0 && y == 0.0 { return 0.0lf; }
    let ax = abs(x);
    let ay = abs(y);
    var a: f64 = 0.0;
    if ax >= ay {
        a = jz_atan_small(ay / ax);
    } else {
        a = jz_atan_small(ax / ay);
        a = 1.57079632679489661923 - a;
    }
    if x < 0.0 { a = 3.14159265358979323846 - a; }
    if y < 0.0 { a = -a; }
    return a;
}

fn jz_cos(a: f64) -> f64 {
    // Range reduction modulo 2π with a two-word π keeps the residual exact
    // enough for a degree-13 Taylor series to reach ~1e-17. The `lf` suffix
    // is required: without it naga resolves an abstract float literal to f32
    // when the value is representable there.
    let two_pi: f64 = 6.283185307179586476925286766559lf;
    var r: f64 = a - two_pi * floor(a / two_pi + 0.5);
    let r2: f64 = r * r;
    var term: f64 = 1.0;
    var sum: f64 = 1.0;
    for (var k = 1i; k <= 13i; k = k + 1i) {
        term = -term * r2 / f64((2 * k - 1) * (2 * k));
        sum = sum + term;
    }
    return sum;
}

fn jz_sin(a: f64) -> f64 {
    let two_pi: f64 = 6.283185307179586476925286766559lf;
    var r: f64 = a - two_pi * floor(a / two_pi + 0.5);
    let r2: f64 = r * r;
    var term: f64 = r;
    var sum: f64 = r;
    for (var k = 1i; k <= 12i; k = k + 1i) {
        term = -term * r2 / f64((2 * k) * (2 * k + 1));
        sum = sum + term;
    }
    return sum;
}

// Safdar 2017 matrices. Row-major, matching the CPU reference, so `dot(row, v)`
// is the matrix-vector product.
const JZ_M1_R0 = vec3<f64>(0.41478972, 0.579999, 0.014648);
const JZ_M1_R1 = vec3<f64>(-0.20151, 1.120649, 0.0531008);
const JZ_M1_R2 = vec3<f64>(-0.0166008, 0.2648, 0.6684799);
const JZ_M1I_R0 = vec3<f64>(1.9242264357876069, -1.0047923125953655, 0.037651404030617994);
const JZ_M1I_R1 = vec3<f64>(0.35031676209499907, 0.7264811939316552, -0.06538442294808501);
const JZ_M1I_R2 = vec3<f64>(-0.09098281098284754, -0.3127282905230739, 1.5227665613052603);
const JZ_M2_R0 = vec3<f64>(0.5, 0.5, 0.0);
const JZ_M2_R1 = vec3<f64>(3.524, -4.066708, 0.542708);
const JZ_M2_R2 = vec3<f64>(0.199076, 1.096799, -1.295875);
const JZ_M2I_R0 = vec3<f64>(1.0, 0.1386050432715393, 0.058047316156118994);
const JZ_M2I_R1 = vec3<f64>(0.9999999999999999, -0.13860504327153927, -0.05804731615611874);
const JZ_M2I_R2 = vec3<f64>(0.9999999999999998, -0.09601924202631892, -0.8118918960560387);

// XYZ (Y = 1) → JzAzBz (Jz, Az, Bz), the CPU's `xyz_to_jzazbz`.
fn xyz_to_jzazbz(xyz: vec3<f64>) -> vec3<f64> {
    let scaled = xyz * JZ_Y_WHITE;
    let lms = jz_mv(JZ_M1_R0, JZ_M1_R1, JZ_M1_R2, jz_bz(scaled.x, scaled.y, scaled.z));
    let pq = jz_pq_encode(lms);
    let izazbz = jz_mv(JZ_M2_R0, JZ_M2_R1, JZ_M2_R2, pq);
    let jz = (1.0 + JZ_D) * izazbz.x / (1.0 + JZ_D * izazbz.x) - JZ_D0;
    return vec3<f64>(jz, izazbz.y, izazbz.z);
}

// JzAzBz (Jz, Az, Bz) → XYZ (Y = 1), the CPU's `jzazbz_to_xyz`.
fn jzazbz_to_xyz(jab: vec3<f64>) -> vec3<f64> {
    let iz = (jab.x + JZ_D0) / (1.0 + JZ_D - JZ_D * (jab.x + JZ_D0));
    let pq = jz_mv(JZ_M2I_R0, JZ_M2I_R1, JZ_M2I_R2, vec3<f64>(iz, jab.y, jab.z));
    let lms = jz_pq_decode(pq);
    let xyzp = jz_mv(JZ_M1I_R0, JZ_M1I_R1, JZ_M1I_R2, lms);
    let xyz = jz_bz_inv(xyzp.x, xyzp.y, xyzp.z);
    return xyz / JZ_Y_WHITE;
}
