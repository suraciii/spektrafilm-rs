// LibRaw's pow(float,float) must round to binary32 before Lab quantization.
// Native driver pow differs by an ULP at discrete AHD direction thresholds;
// evaluate the same binary32 input/exponent with shared binary64 elementary
// functions, then round once to f32. No host numerical LUT preparation.
@group(0) @binding(0) var<storage,read_write> table:array<f32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id:vec3<u32>, @builtin(num_workgroups) grid:vec3<u32>) {
    let i=id.x+id.y*grid.x*256u;
    if (i>=65536u) { return; }
    let r=f32(i)/65535.0;
    var value=7.787*r+16.0/116.0;
    if (r>0.008856) { value=f32(sf64_exp(f64(f32(1.0/3.0))*sf64_log(f64(r)))); }
    table[i]=value;
}
