@group(0) @binding(0) var<storage, read> counts: array<u32>;
@group(0) @binding(1) var<storage, read_write> bins: array<f32>;
var<workgroup> maximum: array<u32, 256>;
@compute @workgroup_size(256)
fn main(@builtin(local_invocation_id) id: vec3<u32>) {
    let i = id.x;
    maximum[i] = 0u;
    if i < 192u { maximum[i] = counts[i]; }
    workgroupBarrier();
    for (var stride = 128u; stride > 0u; stride /= 2u) {
        if i < stride { maximum[i] = max(maximum[i], maximum[i + stride]); }
        workgroupBarrier();
    }
    if i < 192u {
        bins[i] = 0.0;
        if maximum[0] > 0u { bins[i] = f32(counts[i]) / f32(maximum[0]); }
    }
}
