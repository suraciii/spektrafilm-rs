const LENS_PI: f32 = 3.14159265358979323846;

// The fixed target is rectilinear. Lensfun special-cases equidistant,
// panoramic and equirectangular; other source projections go via equirect.
fn lens_project(p: vec2<f32>) -> vec2<f32> {
    let kind = lens.flags.x;
    let dist = lens.mapping.y;
    let inv = lens.mapping.z;
    if lens.flags.y == 0u { return p; }
    if kind == 2u {
        let r = sqrt(p.x * p.x + p.y * p.y) * inv;
        if r == 0.0 { return p; }
        return p * (atan(r) / r);
    }
    if kind == 3u {
        let x = dist * atan(p.x * inv);
        return vec2(x, p.y * cos(x * inv));
    }
    let equi = vec2(dist * atan2(p.x, dist), dist * atan2(p.y, sqrt(dist * dist + p.x * p.x)));
    if kind == 4u { return equi; }
    if kind == 6u {
        let lon = equi.x * inv;
        let lat = equi.y * inv;
        let cosphi = cos(lat);
        let ksp = dist * 2.0 / (1.0 + cosphi * cos(lon));
        return vec2(ksp * cosphi * sin(lon), ksp * sin(lat));
    }
    if kind == 7u {
        let lambda = equi.x / dist;
        let phi = equi.y / dist;
        let denominator = cos(phi) * cos(lambda) + 1.0;
        if abs(denominator) <= 1e-10 { return vec2(1.6e16); }
        let k = sqrt(2.0 / denominator);
        return vec2(dist * k * cos(phi) * sin(lambda), dist * k * sin(phi));
    }
    var phi = equi.x * inv;
    var theta = -equi.y * inv + LENS_PI / 2.0;
    if theta < 0.0 { theta = -theta; phi += LENS_PI; }
    if theta > LENS_PI { theta = 2.0 * LENS_PI - theta; phi += LENS_PI; }
    let s = sin(theta);
    let vx = s * sin(phi);
    let vy = cos(theta);
    theta = atan2(sqrt(vx * vx + vy * vy), s * cos(phi));
    phi = atan2(vy, vx);
    var rho = dist * sin(theta);
    if kind == 8u { rho = 1.47 * dist * sin(theta * 0.713); }
    return vec2(rho * cos(phi), rho * sin(phi));
}
fn lens_distort(p: vec2<f32>) -> vec2<f32> {
    let terms = lens.distortion;
    let r2 = p.x * p.x + p.y * p.y;
    switch lens.dimensions.z {
        case 1u: { return p * ((1.0 - terms.x) + terms.x * r2); }
        case 2u: {
            let sum = lens_ds_add(vec2(1.0, 0.0), vec2(terms.x * r2, 0.0));
            return p * lens_round(lens_ds_add(sum, vec2(terms.y * r2 * r2, 0.0)));
        }
        case 3u: {
            let r = sqrt(r2);
            let d0 = lens_ds_add(vec2(1.0, 0.0), vec2(-terms.x, 0.0));
            let d1 = lens_ds_add(d0, vec2(-terms.y, 0.0));
            let d = lens_round(lens_ds_add(d1, vec2(-terms.z, 0.0)));
            return p * (terms.x * r2 * r + terms.y * r2 + terms.z * r + d);
        }
        default: { return p; }
    }
}
// Combined RGB geometry sends three coordinate pairs per pixel to the native
// SSE callback. It computes four radii but multiplies the two interleaved
// vectors directly: [x0,y0,x1,y1] and [x2,y2,x3,y3] both use [s0,s1,s2,s3].
// Preserve that consumer-visible lane association, including scalar row tails
// and unaligned odd-width rows, rather than applying one radius to each pair.
fn lens_combined_simd(col: u32, row: u32, c: u32) -> bool {
    let pair = col * 3u + c;
    return lens.flags.w != 0u && (lens.dimensions.z == 1u || lens.dimensions.z == 3u)
        && (row * lens.dimensions.x) % 2u == 0u
        && pair < (lens.dimensions.x * 3u / 4u) * 4u;
}
fn lens_combined_scale(p: vec2<f32>) -> f32 {
    let t = lens.distortion;
    let r2 = p.x * p.x + p.y * p.y;
    if lens.dimensions.z == 1u { return r2 * t.x + (1.0 - t.x); }
    let r = sqrt(r2);
    let d = 1.0 - (t.x + (t.y + t.z));
    let ab = (t.x * r2) * r;
    let bc = r2 * t.y + r * t.z;
    return bc + (ab + d);
}
fn lens_combined_distort(p: vec2<f32>, previous: vec2<f32>, next: vec2<f32>, col: u32, row: u32, c: u32) -> vec2<f32> {
    if !lens_combined_simd(col, row, c) { return lens_distort(p); }
    let pair = col * 3u + c;
    let first = pair / 4u * 4u + (pair % 2u) * 2u;
    let px = first / 3u;
    let py = (first + 1u) / 3u;
    let a = select(select(p, previous, px < col), next, px > col);
    let b = select(select(p, previous, py < col), next, py > col);
    return p * vec2(lens_combined_scale(a), lens_combined_scale(b));
}
fn lens_tca(p: vec2<f32>, c: u32) -> vec2<f32> {
    if c == 1u || lens.dimensions.w == 0u { return p; }
    let k = select(0u, 1u, c == 2u);
    if lens.dimensions.w == 1u { return p * lens.tca[k]; }
    let r2 = p.x * p.x + p.y * p.y;
    let v = lens.tca[k];
    let cc = lens.tca[k + 2u];
    let b = lens.tca_tail[k];
    // The optimized branch depends on BOTH channels, not the current channel.
    if lens.tca.z == 0.0 && lens.tca.w == 0.0 { return p * (b * r2 + v); }
    return p * (b * r2 + cc * sqrt(r2) + v);
}
