// The same knee in binary64, for the JzAzBz path. The native `pow` above is
// not used here: the software Vulkan adapter returns zero for ordinary
// arguments such as pow(0.97, 2), and this knee is the only place the
// above-white pixels (where the knee engages) depend on a power. Composing
// the validated `sf64_exp`/`sf64_log` keeps those pixels on the device
// reference instead of collapsing them toward zero.
fn reinhard_knee_f64(d: f64, threshold: f64, limit: f64, power: f64) -> f64 {
    if d <= threshold {
        return d;
    }
    let scale = limit - threshold;
    let x = (d - threshold) / scale;
    let y = x / sf64_pow(1.0 + sf64_pow(x, power), 1.0 / power);
    return threshold + scale * y;
}

// Bilinear C_max(L, h) lookup in binary64 — the JzAzBz variant.
//
// The JzAzBz C_max table has a hard discontinuity: bins 692 and 693 hold
// 0.0234 and 0.0. The hue index spans 0..720, where one f32 ulp is ~8.6e-5
// rows, so carrying `h_idx` in f32 reintroduces a cliff-straddle risk of the
// same magnitude as the ~1.5e-4 rad hue error the f64 transform removed.
// Resolving the index in f64 keeps the lookup bin identical to the CPU
// reference; the four table entries are still f32 storage values, widened to
// f64 for the interpolation. Narrowing the interpolated result back to f32
// costs ~1e-9 absolute on a ~0.027 entry, and the knee then divides by it —
// enough to move the compressed RGB by ~1e-4, the exact budget this mode
// exists to protect.
fn c_max_lookup_f64(l_in: f64, h: f64) -> f64 {
    let l_min = f64(params.lspace.x);
    let l_max = f64(params.lspace.y);
    let n_l = i32(params.n_l);
    let n_h = i32(params.n_h);
    let l = clamp(l_in, l_min, l_max);

    let h_step = 6.283185307179586476925286766559lf / f64(n_h);
    // `h_grid(0) = -PI`: the hue axis starts at -PI, so the bin index is
    // `(h + PI) / h_step`. Using PI/2 here shifts every hue by 180 bins and
    // returns the wrong `C_max` outright.
    let h_idx = (h + 3.14159265358979323846lf) / h_step;
    let h_floor = floor(h_idx);
    let h_lo = ((i32(h_floor) % n_h) + n_h) % n_h;
    let h_hi = (h_lo + 1) % n_h;
    let h_frac = h_idx - h_floor;

    let l_idx = (l - l_min) / (l_max - l_min) * f64(n_l - 1);
    let l_lo = clamp(i32(floor(l_idx)), 0, n_l - 2);
    let l_hi = l_lo + 1;
    let l_frac = l_idx - f64(l_lo);

    let v00 = f64(cmax[l_lo * n_h + h_lo]);
    let v01 = f64(cmax[l_lo * n_h + h_hi]);
    let v10 = f64(cmax[l_hi * n_h + h_lo]);
    let v11 = f64(cmax[l_hi * n_h + h_hi]);
    return v00 * (1.0 - l_frac) * (1.0 - h_frac)
        + v01 * (1.0 - l_frac) * h_frac
        + v10 * l_frac * (1.0 - h_frac)
        + v11 * l_frac * h_frac;
}

// JzAzBz (Safdar 2017) chroma reduction. The perceptual transform runs in
// binary64 because its `b` row cancels ~105x (see the file header), so the
// chroma/hue split — and with it the C_max lookup — is only f32-safe in
// binary64. The knee, the C_max lookup, and the inverse are shared with the
// other perceptual modes: only the forward/inverse transform is f64.
//
// The C_max table stays an f32 storage buffer. It is bisected on the host in
// f64 and uploaded at f32 width, so its entries are f32 values; the lookup
// is a bilinear interpolation of f32 table entries either way, and widening
// the table to f64 storage would not change any entry's value.
fn compress_jzazbz(rgb: vec3<f32>) -> vec3<f32> {
    // The uniform stores each matrix as three vec4 *columns*
    // (`pack_matrix_columns` in `wgpu_backend/mod.rs`), so `params.rgb_to_xyz
    // * rgb` is the CPU's row-major `mat_vec` with no transposition — the
    // same expression the f32 modes use. Read `params.rgb_to_xyz[0]` as row 0
    // instead, and the destination matrix is silently transposed, which is a
    // large error on every destination. Compute the product, then widen the
    // XYZ to f64, so the ~105x `b`-row cancellation never sees an f32
    // rounding.
    let xyz = params.rgb_to_xyz * rgb;
    let jab = xyz_to_jzazbz(vec3<f64>(f64(xyz.x), f64(xyz.y), f64(xyz.z)));
    let l = jab.x;
    let a = jab.y;
    let b = jab.z;
    let c = sqrt(a * a + b * b);
    // Guard h at c == 0: atan2(0, 0) is indeterminate in WGSL and would
    // propagate NaN through cos/sin (Rust's f64 atan2(0,0) returns 0).
    // `var h: f64 = 0.0` — the explicit annotation is required, since a bare
    // `var h = 0.0` infers f32 and `jz_atan2` returns f64.
    var h: f64 = 0.0;
    if c > 0.0 {
        h = jz_atan2(b, a);
    }
    // The lookup index resolves in f64: the hue index spans 0..720, where
    // one f32 ulp is ~8.6e-5 rows, the same order as the ~1.5e-4 rad hue
    // error the f64 transform removes — and the table has a hard cliff
    // between hue bins 692 and 693.
    let safe = max(c_max_lookup_f64(l, h), 1e-9lf);
    // The whole conditioning chain stays in f64: the ratio, the knee, and the
    // restored chroma. Narrowing `safe` or `d_comp` to f32 before the product
    // reintroduces the rounding this mode exists to avoid.
    let d_comp = reinhard_knee_f64(c / safe, f64(params.knee.x), f64(params.knee.y), f64(params.knee.z));
    let c_new = d_comp * safe;
    let jab_new = vec3<f64>(l, c_new * jz_cos(h), c_new * jz_sin(h));
    let xyz_new = jzazbz_to_xyz(jab_new);
    // Narrow once at the very end: the inverse transform runs in f64 so the
    // reconstruction of the original RGB keeps its precision, and only the
    // stored result is f32.
    return params.xyz_to_rgb * vec3<f32>(f32(xyz_new.x), f32(xyz_new.y), f32(xyz_new.z));
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let pixel_idx = gid.x + gid.y * grid.x * 256u;
    if pixel_idx >= params.n_pixels {
        return;
    }
    let base = pixel_idx * 3u;
    let rgb = vec3<f32>(img[base], img[base + 1u], img[base + 2u]);
    var out: vec3<f32>;
    if params.mode == 0u {
        out = compress_aces_rgc(rgb);
    } else if params.mode == 4u {
        out = compress_jzazbz(rgb);
    } else {
        out = compress_perceptual(rgb);
    }
    img[base] = out.x;
    img[base + 1u] = out.y;
    img[base + 2u] = out.z;
}
