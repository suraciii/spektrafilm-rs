// Per-pixel 3D LUT lattice evaluation — GPU port of
// `spektrafilm_math::lut::trilinear_3d` (the calibrated runtime's pointwise
// lattice lookup).
//
// Per pixel: clamp each RGB channel to [0, 1], scale by the lattice index
// range `size - 1`, then trilinear-interpolate the size^3 RGB cube.
// Data layout matches the CPU reference and the `.cube` reader:
//     lut_data[(r*size*size + g*size + b)*3 + c]
//
// The scaled coordinate is `min(clamp(x, 0, 1) * max_idx, max_idx)` so a
// channel at or above 1.0 lands on the last lattice plane with a zero
// fraction instead of reading one plane past the cube.

struct Params {
    width: u32,
    height: u32,
    size: u32, // lattice side length (>= 1)
    _pad: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> input: array<f32>;         // [H*W*3]
@group(0) @binding(2) var<storage, read> lut_data: array<f32>;      // [size^3*3]
@group(0) @binding(3) var<storage, read_write> output: array<f32>;  // [H*W*3]

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let pixel_idx = gid.x + gid.y * grid.x * 256u;
    if pixel_idx >= params.width * params.height {
        return;
    }

    let base = pixel_idx * 3u;
    let size = params.size;
    let max_idx = f32(size - 1u);
    let size2 = size * size;

    // Integer conversion truncates toward zero, exactly like the Rust
    // `as usize` cast applied to the already clamped, already scaled value.
    let rf = min(clamp(input[base], 0.0, 1.0) * max_idx, max_idx);
    let gf = min(clamp(input[base + 1u], 0.0, 1.0) * max_idx, max_idx);
    let bf = min(clamp(input[base + 2u], 0.0, 1.0) * max_idx, max_idx);

    let r0 = u32(rf);
    let g0 = u32(gf);
    let b0 = u32(bf);
    // The upper corner repeats the lower one on the last plane, which keeps
    // the read inside the cube when the fraction is zero.
    let r1 = min(r0 + 1u, size - 1u);
    let g1 = min(g0 + 1u, size - 1u);
    let b1 = min(b0 + 1u, size - 1u);

    let fr = rf - f32(r0);
    let fg = gf - f32(g0);
    let fb = bf - f32(b0);

    // Corner element offsets, shared by the three channel lookups.
    let i000 = (r0 * size2 + g0 * size + b0) * 3u;
    let i100 = (r1 * size2 + g0 * size + b0) * 3u;
    let i010 = (r0 * size2 + g1 * size + b0) * 3u;
    let i110 = (r1 * size2 + g1 * size + b0) * 3u;
    let i001 = (r0 * size2 + g0 * size + b1) * 3u;
    let i101 = (r1 * size2 + g0 * size + b1) * 3u;
    let i011 = (r0 * size2 + g1 * size + b1) * 3u;
    let i111 = (r1 * size2 + g1 * size + b1) * 3u;

    for (var c = 0u; c < 3u; c++) {
        let c000 = lut_data[i000 + c];
        let c100 = lut_data[i100 + c];
        let c010 = lut_data[i010 + c];
        let c110 = lut_data[i110 + c];
        let c001 = lut_data[i001 + c];
        let c101 = lut_data[i101 + c];
        let c011 = lut_data[i011 + c];
        let c111 = lut_data[i111 + c];

        // Three-level lerp: R, then G, then B. The association order mirrors
        // `trilinear_3d` so both sides agree within the f32 budget.
        let c00 = c000 + fr * (c100 - c000);
        let c10 = c010 + fr * (c110 - c010);
        let c01 = c001 + fr * (c101 - c001);
        let c11 = c011 + fr * (c111 - c011);

        let c0 = c00 + fg * (c10 - c00);
        let c1 = c01 + fg * (c11 - c01);

        output[base + c] = c0 + fb * (c1 - c0);
    }
}
