// Lensfun 0.3.4 modifier.cpp, mod-{coord,subpix,color}.cpp.
// Coordinates stay on device. Float pairs retain Lensfun's double normalization
// constants while every recurrent axis element is rounded to float, as upstream.
struct LensParams {
    dimensions: vec4<u32>, flags: vec4<u32>,
    normalization: vec4<f32>, center: vec4<f32>, distortion: vec4<f32>,
    tca: vec4<f32>, tca_tail: vec4<f32>, vignette: vec4<f32>, mapping: vec4<f32>,
}
@group(0) @binding(0) var<uniform> lens: LensParams;
fn lens_ds_add(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    let s = a.x + b.x;
    let v = s - a.x;
    let e = ((a.x - (s - v)) + (b.x - v)) + a.y + b.y;
    let hi = s + e;
    return vec2(hi, e - (hi - s));
}
fn lens_ds_mul(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    let p = a.x * b.x;
    let ac = 4097.0 * a.x;
    let ah = ac - (ac - a.x);
    let al = a.x - ah;
    let bc = 4097.0 * b.x;
    let bh = bc - (bc - b.x);
    let bl = b.x - bh;
    let e = (((ah * bh - p) + ah * bl) + al * bh) + al * bl;
    return lens_ds_add(vec2(p, e), vec2(a.x * b.y + a.y * b.x, 0.0));
}
fn lens_round(a: vec2<f32>) -> f32 { return a.x + a.y; }
fn lens_pixel(p: vec2<f32>) -> vec2<f32> {
    return vec2(
        lens_round(lens_ds_mul(lens_ds_add(vec2(p.x, 0.0), lens.center.xy), lens.normalization.zw)),
        lens_round(lens_ds_mul(lens_ds_add(vec2(p.y, 0.0), lens.center.zw), lens.normalization.zw)));
}
