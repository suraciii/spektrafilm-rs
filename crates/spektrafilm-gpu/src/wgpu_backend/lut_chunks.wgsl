struct Params { n: u32, size: u32, first: u32, len: u32 }
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> input: array<f32>;
@group(0) @binding(2) var<storage, read> page: array<f32>;
@group(0) @binding(3) var<storage, read_write> corners: array<f32>;
@group(0) @binding(4) var<storage, read_write> output: array<f32>;

@compute @workgroup_size(256)
fn gather(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let i = gid.x + gid.y * grid.x * 256u;
    if i >= p.n { return; }
    let upper = f32(p.size - 1u);
    let q = min(clamp(vec3<f32>(input[i*3u], input[i*3u+1u], input[i*3u+2u]), vec3<f32>(0.0), vec3<f32>(1.0))*upper, vec3<f32>(upper));
    let lo = vec3<u32>(q);
    let hi = min(lo + vec3<u32>(1u), vec3<u32>(p.size-1u));
    for (var corner = 0u; corner < 8u; corner++) {
        let x = select(lo.x, hi.x, (corner & 1u) != 0u);
        let y = select(lo.y, hi.y, (corner & 2u) != 0u);
        let z = select(lo.z, hi.z, (corner & 4u) != 0u);
        let base = ((x*p.size+y)*p.size+z)*3u;
        for (var c=0u; c<3u; c++) {
            let j = base+c;
            if j >= p.first && j-p.first < p.len {
                corners[i*24u+corner*3u+c] = page[j-p.first];
            }
        }
    }
}

@compute @workgroup_size(256)
fn finish(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let i = gid.x + gid.y * grid.x * 256u;
    if i >= p.n { return; }
    let upper = f32(p.size-1u);
    let q = min(clamp(vec3<f32>(input[i*3u], input[i*3u+1u], input[i*3u+2u]), vec3<f32>(0.0), vec3<f32>(1.0))*upper, vec3<f32>(upper));
    let f = q-vec3<f32>(vec3<u32>(q));
    let b = i*24u;
    for (var c=0u; c<3u; c++) {
        let c00 = corners[b+c] + f.x*(corners[b+3u+c]-corners[b+c]);
        let c10 = corners[b+6u+c] + f.x*(corners[b+9u+c]-corners[b+6u+c]);
        let c01 = corners[b+12u+c] + f.x*(corners[b+15u+c]-corners[b+12u+c]);
        let c11 = corners[b+18u+c] + f.x*(corners[b+21u+c]-corners[b+18u+c]);
        let c0 = c00 + f.y*(c10-c00);
        let c1 = c01 + f.y*(c11-c01);
        output[i*3u+c] = c0 + f.z*(c1-c0);
    }
}
