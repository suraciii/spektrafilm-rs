// The scanner optics integrate transmission, never density.
// Count is pixels; each invocation keeps its three channels in one page.
struct Params { count: u32, inverse: u32, _pad: vec2<u32> }
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read_write> image: array<f32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let i = gid.x + gid.y * grid.x * 256u;
    if i >= p.count { return; }
    for(var c=0u;c<3u;c++) {
        let sample=i*3u+c;
        if p.inverse == 0u { image[sample] = f32(sf64_exp(-f64(image[sample]) * 2.302585092994046)); }
        else { image[sample] = f32(-sf64_log(f64(image[sample])) / 2.302585092994046); }
    }
}
