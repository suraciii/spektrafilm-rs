// Shared retained-analysis conversion. The source remains in its native carrier.
struct Params {
    page_first: u32, start: u32, count: u32, width: u32,
    x0: u32, y0: u32, x1: u32, y1: u32,
    mode: u32, zoom: u32, spatial: u32, decode: u32,
    cctf: u32, column_offset: u32, totals_offset: u32, _pad: u32,
    row0: vec4<f32>, row1: vec4<f32>, row2: vec4<f32>, y_row: vec4<f32>,
};
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> source: array<f32>;

struct Sample { rgb: vec3<f32>, tone: f32, ab: vec2<f32>, linear: vec3<f32>, valid: bool };
fn finite(v: f32) -> bool { return (bitcast<u32>(v) & 0x7f800000u) != 0x7f800000u; }
fn finite3(v: vec3<f32>) -> bool { return finite(v.x) && finite(v.y) && finite(v.z); }
fn cbrt_signed(v: f32) -> f32 { return sign(v) * pow(abs(v), 1.0 / 3.0); }
fn analysis_sample(i: u32) -> Sample {
    var rgb = vec3<f32>(source[i * 3u], source[i * 3u + 1u], source[i * 3u + 2u]);
    if !finite3(rgb) { return Sample(vec3<f32>(0.0), 0.0, vec2<f32>(0.0), rgb, false); }
    if p.decode != 0u { rgb = vec3<f32>(decode_one(rgb.x, p.cctf), decode_one(rgb.y, p.cctf), decode_one(rgb.z, p.cctf)); }
    let linear = vec3<f32>(dot(p.row0.xyz, rgb), dot(p.row1.xyz, rgb), dot(p.row2.xyz, rgb));
    let signals = vec3<f32>(encode_one(linear.x, 2u), encode_one(linear.y, 2u), encode_one(linear.z, 2u));
    let tone = encode_one(dot(p.y_row.xyz, linear), 2u);
    let l = cbrt_signed(dot(linear, vec3<f32>(0.4122214708, 0.5363325363, 0.0514459929)));
    let m = cbrt_signed(dot(linear, vec3<f32>(0.2119034982, 0.6806995451, 0.1073969566)));
    let s = cbrt_signed(dot(linear, vec3<f32>(0.0883024619, 0.2817188376, 0.6299787005)));
    let ab = vec2<f32>(1.9779984951*l - 2.4285922050*m + 0.4505937099*s,
                       0.0259040371*l + 0.7827717662*m - 0.8086757660*s);
    let valid = finite3(linear) && finite3(signals) && finite(tone) && finite(ab.x) && finite(ab.y);
    return Sample(signals, tone, ab, linear, valid);
}
fn signal_bin(v: f32) -> u32 { return min(255u, u32(floor(clamp(v, 0.0, 1.0) * 256.0))); }
