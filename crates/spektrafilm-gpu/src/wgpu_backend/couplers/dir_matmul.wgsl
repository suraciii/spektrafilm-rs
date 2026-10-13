// DIR coupler per-pixel matrix multiply.
//
// CPU equivalent: in `compute_exposure_correction`:
//   density_silver = positive ? (density_max - density_cmy) : density_cmy
//   if donor_k: density_silver = langmuir(density_silver, donor_k, donor_ref)
//   correction[m] = sum_c density_silver[c] * couplers_matrix[c][m]
//
// `couplers_matrix` is already pre-scaled by `amount` on the CPU side.
// `positive` is encoded as 0 (negative) or 1 (positive film) in params.
// `donor_mask` is 1 on channels where the donor saturation applies; a
// channel with a non-finite knee stays linear (CPU `k.is_infinite()`
// short-circuit) and its sanitized `donor_k` is never read.

struct Params {
    n_pixels: u32,
    positive: u32,
    _pad0: u32,
    _pad1: u32,
    density_max: vec4<f32>, // .xyz used, .w padding
    // Amplitude-matched Langmuir knee, reference and activity per
    // channel (`.w` padding).
    donor_k: vec4<f32>,
    donor_ref: vec4<f32>,
    donor_mask: vec4<f32>,
    // Row-major couplers matrix [3][3], packed as three vec4s
    // (.w padding to satisfy std140 alignment).
    m_row0: vec4<f32>,
    m_row1: vec4<f32>,
    m_row2: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> density_cmy: array<f32>;
@group(0) @binding(2) var<storage, read_write> correction: array<f32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let idx = gid.x + gid.y * grid.x * 256u;
    if idx >= params.n_pixels { return; }
    let base = idx * 3u;

    var silver: vec3<f32> = vec3<f32>(
        density_cmy[base],
        density_cmy[base + 1u],
        density_cmy[base + 2u],
    );
    if params.positive != 0u {
        silver = params.density_max.xyz - silver;
    }

    // g(D) = D (K + D_ref) / (K + D) — amplitude-matched at D_ref. The
    // saturated term is evaluated unconditionally but only selected on
    // active channels, so a degenerate knee can never leak into a
    // masked-off channel.
    let k = params.donor_k.xyz;
    let r = params.donor_ref.xyz;
    let saturated = silver * (k + r) / (k + silver);
    let mask = params.donor_mask.xyz;
    silver = vec3<f32>(
        select(silver.x, saturated.x, mask.x > 0.5),
        select(silver.y, saturated.y, mask.y > 0.5),
        select(silver.z, saturated.z, mask.z > 0.5),
    );

    // correction[m] = silver[0]*M[0][m] + silver[1]*M[1][m] + silver[2]*M[2][m]
    let c0 = silver.x * params.m_row0.x + silver.y * params.m_row1.x + silver.z * params.m_row2.x;
    let c1 = silver.x * params.m_row0.y + silver.y * params.m_row1.y + silver.z * params.m_row2.y;
    let c2 = silver.x * params.m_row0.z + silver.y * params.m_row1.z + silver.z * params.m_row2.z;

    correction[base] = c0;
    correction[base + 1u] = c1;
    correction[base + 2u] = c2;
}
