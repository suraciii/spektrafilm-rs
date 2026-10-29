// Range-reduced binary32 series, matching the scanner's spectral math.
fn transmission_exp(x: f32) -> f32 {
    if (bitcast<u32>(x) & 0x7fffffffu) > 0x7f800000u { return x; }
    if x < -103.97208 { return 0.0; }
    if x > 88.72284 { return bitcast<f32>(0x7f800000u); }
    let scaled = x * 1.44269504089;
    let exponent = i32(select(ceil(scaled - 0.5), floor(scaled + 0.5), scaled >= 0.0));
    let r = (x - f32(exponent) * 0.693145751953125) - f32(exponent) * 0.00000142860682;
    var term = 1.0;
    var sum = 1.0;
    for (var n = 1u; n <= 10u; n++) { term *= r / f32(n); sum += term; }
    if exponent > 0i { for (var n = 0i; n < exponent; n++) { sum *= 2.0; } }
    else { for (var n = 0i; n > exponent; n--) { sum *= 0.5; } }
    return sum;
}
// Binary32 PCHIP over device-generated scanner/enlarger spectral lattices.
// Same prepared axis slopes, Hermite walk, and convex-hull clamp as
// spektrafilm_math::pchip3d::pchip_interp. Scanner colour reconstruction
// remains in binary32 until the final f32 RGB image boundary.

struct Params {
    n_pixels: u32,
    size: u32,
    size_m1: u32,
    _pad: u32,
    _align: vec4<u32>,
    // Normalisation: axis coordinate = (value - data_min) * data_scale.
    data_min: vec3<f32>,
    data_scale: vec3<f32>,
    origin: vec4<u32>,
    extent: vec4<u32>,
    cat: array<vec4<f32>,3>,
    rgb: array<vec4<f32>,3>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> input: array<f32>;      // [N*3]
@group(0) @binding(2) var<storage, read> lut: array<f32>;        // [size^3*3]
@group(0) @binding(3) var<storage, read> slope_x: array<f32>;    // [size^3*3]
@group(0) @binding(4) var<storage, read> slope_y: array<f32>;
@group(0) @binding(5) var<storage, read> slope_z: array<f32>;
@group(0) @binding(6) var<storage, read> cell_min: array<f32>;   // [(size-1)^3*3]
@group(0) @binding(7) var<storage, read> cell_max: array<f32>;
@group(0) @binding(8) var<storage, read_write> output: array<f32>;

// `((i * size + j) * size + k) * 3 + c`
fn idx4(i: u32, j: u32, k: u32, c: u32) -> u32 {
    return (((i - params.origin.x) * params.extent.y + j - params.origin.y) * params.extent.z + k - params.origin.z) * 3u + c;
}

// `((i * (size-1) + j) * (size-1) + k) * 3 + c`
fn idx4_cell(i: u32, j: u32, k: u32, c: u32) -> u32 {
    return (((i - params.origin.x) * (params.extent.y - 1u) + j - params.origin.y) * (params.extent.z - 1u) + k - params.origin.z) * 3u + c;
}

// Cubic Hermite basis, matching `hermite` in the reference.
fn hermite(y0: f32, y1: f32, m0: f32, m1: f32, t: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h10 = t3 - 2.0 * t2 + t;
    let h01 = -2.0 * t3 + 3.0 * t2;
    let h11 = t3 - t2;
    return h00 * y0 + h10 * m0 + h01 * y1 + h11 * m1;
}

fn linear_mix(v0: f32, v1: f32, t: f32) -> f32 {
    return v0 + t * (v1 - v0);
}

fn bilinear_mix(v00: f32, v10: f32, v01: f32, v11: f32, tx: f32, ty: f32) -> f32 {
    let vx0 = linear_mix(v00, v10, tx);
    let vx1 = linear_mix(v01, v11, tx);
    return linear_mix(vx0, vx1, ty);
}

// `cubic_base_fraction`: clamp the coordinate to [0, size-1] and return the
// base node index. The caller takes the fractional offset as
// `clamp(coord - base, 0, 1)`, matching the reference's `c - base`.
fn base_index(coord: f32) -> u32 {
    let upper = f32(params.size_m1);
    var c = coord;
    if c <= 0.0 {
        c = 0.0;
    } else if c > upper {
        c = upper;
    }
    if c >= upper {
        // The last interval runs to the final node at t = 1.
        return params.size_m1 - 1u;
    }
    return u32(floor(c));
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let p = gid.x + gid.y * grid.x * 256u;
    if p >= params.n_pixels {
        return;
    }
    let src = p * 3u;
    let r = (f32(input[src]) - params.data_min.x) * params.data_scale.x;
    let g = (f32(input[src + 1u]) - params.data_min.y) * params.data_scale.y;
    let b = (f32(input[src + 2u]) - params.data_min.z) * params.data_scale.z;

    // The base node index is resolved per axis; the fraction is
    // `clamp(coord - base, 0, 1)`, which reproduces the reference's
    // `c - base` for every clamped and interior coordinate.
    let i = base_index(r);
    let j = base_index(g);
    let k = base_index(b);
    if i < params.origin.x || j < params.origin.y || k < params.origin.z ||
       i - params.origin.x >= params.extent.x - 1u ||
       j - params.origin.y >= params.extent.y - 1u ||
       k - params.origin.z >= params.extent.z - 1u { return; }
    let tr = clamp(r - f32(i), f32(0.0), f32(1.0));
    let tg = clamp(g - f32(j), f32(0.0), f32(1.0));
    let tb = clamp(b - f32(k), f32(0.0), f32(1.0));

    var result = vec3<f32>(0.0);
    for (var c = 0u; c < 3u; c++) {
        // Hermite along x at the four (y, z) corners of the cell.
        let v000 = hermite(lut[idx4(i, j, k, c)],
                           lut[idx4(i + 1u, j, k, c)],
                           slope_x[idx4(i, j, k, c)],
                           slope_x[idx4(i + 1u, j, k, c)], tr);
        let v010 = hermite(lut[idx4(i, j + 1u, k, c)],
                           lut[idx4(i + 1u, j + 1u, k, c)],
                           slope_x[idx4(i, j + 1u, k, c)],
                           slope_x[idx4(i + 1u, j + 1u, k, c)], tr);
        let v001 = hermite(lut[idx4(i, j, k + 1u, c)],
                           lut[idx4(i + 1u, j, k + 1u, c)],
                           slope_x[idx4(i, j, k + 1u, c)],
                           slope_x[idx4(i + 1u, j, k + 1u, c)], tr);
        let v011 = hermite(lut[idx4(i, j + 1u, k + 1u, c)],
                           lut[idx4(i + 1u, j + 1u, k + 1u, c)],
                           slope_x[idx4(i, j + 1u, k + 1u, c)],
                           slope_x[idx4(i + 1u, j + 1u, k + 1u, c)], tr);

        // Orthogonal-axis slopes, mixed linearly along x.
        let sy00 = linear_mix(slope_y[idx4(i, j, k, c)],
                              slope_y[idx4(i + 1u, j, k, c)], tr);
        let sy10 = linear_mix(slope_y[idx4(i, j + 1u, k, c)],
                              slope_y[idx4(i + 1u, j + 1u, k, c)], tr);
        let sy01 = linear_mix(slope_y[idx4(i, j, k + 1u, c)],
                              slope_y[idx4(i + 1u, j, k + 1u, c)], tr);
        let sy11 = linear_mix(slope_y[idx4(i, j + 1u, k + 1u, c)],
                              slope_y[idx4(i + 1u, j + 1u, k + 1u, c)], tr);

        // Hermite along y at both z positions.
        let vz0 = hermite(v000, v010, sy00, sy10, tg);
        let vz1 = hermite(v001, v011, sy01, sy11, tg);

        // Bilinear mix of slope_z to the (tr, tg) point at both z corners.
        let sz0 = bilinear_mix(slope_z[idx4(i, j, k, c)],
                               slope_z[idx4(i + 1u, j, k, c)],
                               slope_z[idx4(i, j + 1u, k, c)],
                               slope_z[idx4(i + 1u, j + 1u, k, c)], tr, tg);
        let sz1 = bilinear_mix(slope_z[idx4(i, j, k + 1u, c)],
                               slope_z[idx4(i + 1u, j, k + 1u, c)],
                               slope_z[idx4(i, j + 1u, k + 1u, c)],
                               slope_z[idx4(i + 1u, j + 1u, k + 1u, c)], tr, tg);

        // Final Hermite along z, then the per-cell convex-hull clamp.
        let interp = hermite(vz0, vz1, sz0, sz1, tb);
        result[c] = clamp(interp,
                                cell_min[idx4_cell(i, j, k, c)],
                                cell_max[idx4_cell(i, j, k, c)]);
    }
    if params._pad != 0u {
        let xyz=vec3<f32>(transmission_exp(result.x*2.302585092994046),transmission_exp(result.y*2.302585092994046),transmission_exp(result.z*2.302585092994046));
        let a=vec3<f32>(
            params.cat[0].x*xyz.x+params.cat[0].y*xyz.y+params.cat[0].z*xyz.z,
            params.cat[1].x*xyz.x+params.cat[1].y*xyz.y+params.cat[1].z*xyz.z,
            params.cat[2].x*xyz.x+params.cat[2].y*xyz.y+params.cat[2].z*xyz.z);
        result=vec3<f32>(
            params.rgb[0].x*a.x+params.rgb[0].y*a.y+params.rgb[0].z*a.z,
            params.rgb[1].x*a.x+params.rgb[1].y*a.y+params.rgb[1].z*a.z,
            params.rgb[2].x*a.x+params.rgb[2].y*a.y+params.rgb[2].z*a.z);
    }
    output[src]=f32(result.x);output[src+1u]=f32(result.y);output[src+2u]=f32(result.z);
}
