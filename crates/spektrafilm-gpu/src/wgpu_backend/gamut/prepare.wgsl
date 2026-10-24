// Cold C_max preparation. Every bisection decision is binary64; only the
// retained consumer table is narrowed to f32. Shared Jz helpers supply explicit
// transcendentals, avoiding unvalidated native f64 log/exp/pow/trigonometry.
struct PrepareParams {
    mode: u32,
    n_l: u32,
    n_h: u32,
    n_bisect: u32,
    first: u32,
    count: u32,
    _pad0: u32,
    _pad1: u32,
    geometry: vec4<f64>,
    xyz_to_rgb: array<vec4<f64>, 3>,
    cam16: vec4<f64>,
}
@group(0) @binding(0) var<uniform> prep: PrepareParams;
@group(0) @binding(1) var<storage, read_write> table: array<f32>;

fn prepare_oklab_to_xyz(lab: vec3<f64>) -> vec3<f64> {
    let lms = jz_mv(
        vec3<f64>(0.9999999984505196, 0.3963377921737678, 0.21580375806075877),
        vec3<f64>(1.0000000088817607, -0.10556134232365633, -0.0638541747717059),
        vec3<f64>(1.0000000546724108, -0.08948418209496574, -1.2914855378640917), lab);
    // CPU powi(3): preserve its multiplication order.
    let cubed = lms * (lms * lms);
    return jz_mv(
        vec3<f64>(1.2270138511035211, -0.5577999806518222, 0.2812561489664678),
        vec3<f64>(-0.0405801784232806, 1.11225686961683, -0.07167667866560119),
        vec3<f64>(-0.0763812845057069, -0.4214819784180127, 1.5861632204407947), cubed);
}

// Exact fixed viewing constants and inverse from core gamut_compression/cam16.rs.
const PREP_CAM_FL: f64 = 0.6839903845696502;
const PREP_CAM_NBB: f64 = 1.0003040045593807;
const PREP_CAM_Z: f64 = 1.9272135954999579;
const PREP_CAM_C: f64 = 0.69;
const PREP_UCS_C1: f64 = 0.007;
const PREP_UCS_C2: f64 = 0.0228;

fn prepare_cam_opponent(p1: f64, p2: f64, h: f64) -> vec2<f64> {
    let p3 = f64(21.0 / 20.0);
    let s = jz_sin(h);
    let c = jz_cos(h);
    let nn = p2 * (2.0 + p3) * (460.0 / 1403.0);
    if abs(s) >= abs(c) {
        var p4: f64 = 0.0;
        if s != 0.0 { p4 = p1 / s; }
        let b = nn / (p4 + (2.0 + p3) * (220.0 / 1403.0) * (c / s)
            - (27.0 / 1403.0) + p3 * (6300.0 / 1403.0));
        return vec2<f64>(b * (c / s), b);
    }
    var p5: f64 = 0.0;
    if c != 0.0 { p5 = p1 / c; }
    let a = nn / (p5 + (2.0 + p3) * (220.0 / 1403.0)
        - ((27.0 / 1403.0) - p3 * (6300.0 / 1403.0)) * (s / c));
    return vec2<f64>(a, a * (s / c));
}

fn prepare_cam16_to_xyz(jab: vec3<f64>) -> vec3<f64> {
    let mp = sqrt(jab.y * jab.y + jab.z * jab.z);
    // Match CPU atan2_deg -> rem_euclid(360) -> radians, including the
    // intermediate degree rounding rather than reusing the grid hue.
    let degrees = jz_atan2(jab.z, jab.y) * f64(180.0 / 3.14159265358979323846);
    var hue = degrees % 360.0lf;
    if hue < 0.0 { hue = hue + 360.0; }
    let h = hue * 3.14159265358979323846lf / 180.0lf;
    let jj = jab.x / ((1.0 + 100.0 * PREP_UCS_C1) - PREP_UCS_C1 * jab.x);
    let m = (sf64_exp(PREP_UCS_C2 * mp) - 1.0) / PREP_UCS_C2;
    let cc = m / sf64_pow(PREP_CAM_FL, 0.25);
    let j_prime = max(jj, 2.220446049250313e-16lf);
    let t = sf64_pow(cc / (sqrt(j_prime / 100.0)
        * sf64_pow(1.64 - sf64_pow(0.29, 0.2), 0.73)), 1.0 / 0.9);
    let e_t = 0.25 * (jz_cos(2.0 + h) + 3.8);
    let a_resp = prep.cam16.w * sf64_pow(jj / 100.0, 1.0 / (PREP_CAM_C * PREP_CAM_Z));
    var p1: f64 = 0.0;
    if t != 0.0 { p1 = (50000.0 / 13.0) * 1.0003040045593807 * e_t / t; }
    let p2 = a_resp / PREP_CAM_NBB + 0.305;
    var ab = prepare_cam_opponent(p1, p2, h);
    if t == 0.0 { ab = vec2<f64>(0.0); }
    let responses = vec3<f64>(
        (460.0 * p2 + 451.0 * ab.x + 288.0 * ab.y) / 1403.0,
        (460.0 * p2 - 891.0 * ab.x - 261.0 * ab.y) / 1403.0,
        (460.0 * p2 - 220.0 * ab.x - 6300.0 * ab.y) / 1403.0);
    var rgb: vec3<f64>;
    for (var i = 0u; i < 3u; i = i + 1u) {
        let d = responses[i] - 0.1;
        let base = (27.13 * abs(d)) / (400.0 - abs(d));
        // CPU padc_inverse uses unsigned powf here: a negative base must
        // remain NaN (out of gamut), not become a signed-power response.
        let adapted = sign(d) * 100.0 / PREP_CAM_FL
            * sf64_exp((1.0 / 0.42) * sf64_log(base));
        rgb[i] = adapted / prep.cam16[i];
    }
    return jz_mv(
        vec3<f64>(1.8620678550872327, -1.0112546305316843, 0.14918677544445175),
        vec3<f64>(0.3875265432361372, 0.6214474419314753, -0.008973985167612516),
        vec3<f64>(-0.015841498849333863, -0.03412293802851557, 1.0499644368778496), rgb) / 100.0;
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>,
        @builtin(num_workgroups) groups: vec3<u32>) {
    let local = gid.x + gid.y * groups.x * 256u;
    if local >= prep.count { return; }
    let index = prep.first + local;
    let row = index / prep.n_h;
    let column = index % prep.n_h;
    let lookup_l = prep.geometry.x + (prep.geometry.y - prep.geometry.x)
        * f64(row) / f64(prep.n_l - 1u);
    var l = lookup_l;
    if prep.mode == 2u {
        let k3 = f64((1.0 + 0.206) / (1.0 + 0.03));
        l = (lookup_l * (lookup_l + 0.206)) / (k3 * (lookup_l + 0.03));
    }
    let h = f64(-3.14159265358979323846) + f64(2.0 * 3.14159265358979323846)
        * f64(column) / f64(prep.n_h);
    let cos_h = jz_cos(h);
    let sin_h = jz_sin(h);
    var lo: f64 = 0.0;
    var hi = prep.geometry.z;
    for (var step = 0u; step < prep.n_bisect; step = step + 1u) {
        let mid = (lo + hi) * 0.5;
        let lab = vec3<f64>(l, mid * cos_h, mid * sin_h);
        var xyz: vec3<f64>;
        if prep.mode == 4u {
            xyz = jzazbz_to_xyz(lab);
        } else if prep.mode == 3u {
            xyz = prepare_cam16_to_xyz(lab);
        } else {
            xyz = prepare_oklab_to_xyz(lab);
        }
        let rgb = jz_mv(prep.xyz_to_rgb[0].xyz, prep.xyz_to_rgb[1].xyz,
            prep.xyz_to_rgb[2].xyz, xyz);
        if all(rgb >= vec3<f64>(-1e-6)) && all(rgb <= vec3<f64>(1.0 + 1e-6)) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    table[local] = f32(lo);
}
