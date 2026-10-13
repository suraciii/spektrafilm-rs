// Per-pixel per-channel 1D table lookup with binary search + linear interpolation.
// Matches CPU interp_1d: binary search, clamp to endpoints, linear interp.
// Bindings: 0=uniform, 1=input ro, 2=table_x ro, 3=table_y ro, 4=output rw.

struct Params {
    width: u32,
    height: u32,
    table_len: u32,
    _pad: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> input: array<f32>;        // [H*W*3]
@group(0) @binding(2) var<storage, read> table_x: array<f32>;      // [table_len]
@group(0) @binding(3) var<storage, read> table_y: array<f32>;      // [table_len*3]
@group(0) @binding(4) var<storage, read_write> output: array<f32>; // [H*W*3]

fn interp_channel(x: f32, channel: u32) -> f32 {
    let n = params.table_len;
    if n == 0u { return x; }
    if n == 1u { return table_y[channel]; }
    if x <= table_x[0] { return table_y[channel]; }
    if x >= table_x[n - 1u] { return table_y[(n - 1u) * 3u + channel]; }

    var lo = 0u;
    var hi = n - 1u;
    while lo + 1u < hi {
        let mid = (lo + hi) / 2u;
        if table_x[mid] <= x {
            lo = mid;
        } else {
            hi = mid;
        }
    }

    let x0 = table_x[lo];
    let x1 = table_x[lo + 1u];
    let y0 = table_y[lo * 3u + channel];
    let y1 = table_y[(lo + 1u) * 3u + channel];
    let t = (x - x0) / (x1 - x0);
    return y0 + t * (y1 - y0);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let pixel_idx = gid.x + gid.y * grid.x * 256u;
    if pixel_idx >= params.width * params.height { return; }

    let base = pixel_idx * 3u;
    output[base]      = interp_channel(input[base], 0u);
    output[base + 1u] = interp_channel(input[base + 1u], 1u);
    output[base + 2u] = interp_channel(input[base + 2u], 2u);
}
