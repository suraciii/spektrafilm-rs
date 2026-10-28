// Exact sampled PSF, accumulated in full row-major tap order across pages.
// Host gathers reflected global source coordinates; page edges are not edges.
struct Params {
    header: vec4<u32>, // output count, tap count, first tap block, final block
    controls: vec4<f32>, // scatter
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> halo: array<f32>;
@group(0) @binding(2) var<storage, read> taps: array<f32>;
@group(0) @binding(3) var<storage, read_write> accumulation: array<f32>;
@group(0) @binding(4) var<storage, read> centers: array<f32>;
@group(0) @binding(5) var<storage, read_write> output: array<f32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let pixel = gid.x + gid.y * grid.x * 256u;
    if pixel >= p.header.x { return; }
    for (var c = 0u; c < 3u; c++) {
        let index = pixel*3u+c;
        var total = 0.0;
        if p.header.z == 0u { total = accumulation[index]; }
        for (var tap = 0u; tap < p.header.y; tap++) {
            total += taps[c*p.header.y+tap] * halo[(pixel+tap)*3u+c];
        }
        accumulation[index] = total;
        if p.header.w != 0u {
            output[index] = (1.0-p.controls.x)*centers[index]+p.controls.x*total;
        }
    }
}
