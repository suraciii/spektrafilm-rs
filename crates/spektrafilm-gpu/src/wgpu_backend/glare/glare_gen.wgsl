// Sequential NumPy legacy normal stream. One invocation owns the generator
// and its cached pair. Bounded dispatches preserve rejection consumption.
struct Params {
    count: u32,
    first: u32,
    seed: u32,
    pad: u32,
    mu: f32,
    sigma: f32,
    pad_tail: vec2<u32>,
}
struct MtSlot {
    idx: u32,
    seeded: u32,
    has_gauss: u32,
    pad: u32,
    gauss: f32,
    state: array<u32, 624>,
}
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> glare_buf: array<f32>;
@group(0) @binding(2) var<storage, read_write> mt: array<MtSlot>;

// Keep centered integer numerators until the normal transform splits them.
fn rk_signed_uniform() -> vec2<i32> {
    let a = i32(mt_next(0u) >> 5u) - 67108864;
    let b = i32(mt_next(0u) >> 6u);
    return vec2<i32>(a, b);
}

fn rk_gauss() -> f32 {
    if mt[0].has_gauss != 0u {
        let value = mt[0].gauss;
        mt[0].has_gauss = 0u;
        mt[0].gauss = 0.0;
        return value;
    }
    loop {
        let x = rk_signed_uniform();
        let y = rk_signed_uniform();
        // Split the 27-bit numerator on a 2^-12 grid before conversion.
        // The coarse squared distance is exact in i32 (magnitude <= 2^24).
        // Subtract small f32 residuals to retain distance to the unit circle.
        let ix = x.x / 16384;
        let iy = y.x / 16384;
        let coarse_x = f32(ix) * (1.0 / 4096.0);
        let coarse_y = f32(iy) * (1.0 / 4096.0);
        let fine_x = f32(x.x - ix * 16384) * (1.0 / 67108864.0) + f32(x.y) * (1.0 / 4503599627370496.0);
        let fine_y = f32(y.x - iy * 16384) * (1.0 / 67108864.0) + f32(y.y) * (1.0 / 4503599627370496.0);
        let distance = f32(16777216 - ix * ix - iy * iy) * (1.0 / 16777216.0)
            - (2.0 * (coarse_x * fine_x + coarse_y * fine_y)
                + fine_x * fine_x + fine_y * fine_y);
        let radius = (coarse_x + fine_x) * (coarse_x + fine_x) + (coarse_y + fine_y) * (coarse_y + fine_y);
        if distance > 0.0 && radius > 0.0 {
            var log_radius = log(radius);
            if abs(distance) < 0.01 {
                log_radius = -distance * (1.0 + distance * (0.5 + distance * (1.0 / 3.0 + distance * 0.25)));
            }
            let factor = sqrt(-2.0 * log_radius / radius);
            mt[0].gauss = factor * (coarse_x + fine_x);
            mt[0].has_gauss = 1u;
            return factor * (coarse_y + fine_y);
        }
    }
    return 0.0;
}

@compute @workgroup_size(1)
fn main() {
    // Every job submission starts a new realization, even on reused state.
    if params.first == 0u {
        mt_reseed(0u, params.seed);
        mt[0].has_gauss = 0u;
        mt[0].gauss = 0.0;
    }
    for (var i = 0u; i < params.count; i = i + 1u) {
        var exponent = params.mu;
        if params.sigma >= 1e-6 {
            exponent = fma(params.sigma, rk_gauss(), exponent);
        }
        let value = exp(exponent);
        let base = i * 3u;
        glare_buf[base] = value;
        glare_buf[base + 1u] = value;
        glare_buf[base + 2u] = value;
    }
}
