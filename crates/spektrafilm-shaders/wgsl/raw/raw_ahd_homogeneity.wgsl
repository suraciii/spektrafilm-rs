// `ahd_interpolate_build_homogeneity_map`: compare each candidate direction's
// CIELab neighbourhood and count how many neighbours agree within the two
// directions' own epsilon. The epsilon pair comes from the *cross* directions,
// exactly as the reference's `leps`/`abeps` do.
//
// LibRaw reuses one `lab` tile buffer per direction; here the Lab values are
// re-derived from the retained candidate samples, which is the same function
// of the same inputs.


/// One direction's RGB triple at `index`, as CIELab input.
///
/// Direction 0 is the horizontal candidates, direction 1 the vertical ones; the
/// two `lab_at` call shapes are one implementation with a `u32` selector,
/// because naga rejects `ptr<storage>` as a function parameter.
fn lab_sample(index: u32, channel: u32, vertical: u32) -> i32 {
    if (vertical != 0u) {
        return i32(rgb_v[index * 3u + channel]);
    }
    return i32(rgb_h[index * 3u + channel]);
}

fn lab_at(index: u32, colors: u32, vertical: u32) -> vec3<i32> {
    return raw_cielab(
        lab_sample(index, 0u, vertical),
        lab_sample(index, 1u, vertical),
        lab_sample(index, 2u, vertical),
        colors,
    );
}

/// `(ldiff, abdiff)` for one neighbour, as the reference computes them.
fn difference(center: vec3<i32>, adjacent: vec3<i32>) -> vec2<u32> {
    let dl = abs(center.x - adjacent.x);
    let da = center.y - adjacent.y;
    let db = center.z - adjacent.z;
    // The reference squares signed ints and stores the sum as `unsigned`;
    // WGSL integer arithmetic wraps the same way.
    let a = bitcast<u32>(da * da) + bitcast<u32>(db * db);
    return vec2<u32>(u32(dl), a);
}

fn within(difference: vec2<u32>, leps: u32, abeps: u32) -> u32 {
    if (difference.x <= leps && difference.y <= abeps) {
        return 1u;
    }
    return 0u;
}

@group(0) @binding(0) var<uniform> frame: RawFrame;
@group(0) @binding(1) var<uniform> pass_uniform: AhdPass;
@group(0) @binding(2) var<storage, read> rgb_h: array<u32>;
@group(0) @binding(3) var<storage, read> rgb_v: array<u32>;
@group(0) @binding(4) var<storage, read> xyz_camera: array<vec4<f32>>;
@group(0) @binding(5) var<storage, read> cbrt: array<f32>;
@group(0) @binding(6) var<storage, read_write> homogeneity: array<u32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let index = id.x + id.y * grid.x * 256u;
    if (index >= raw_pixel_count(frame)) {
        return;
    }
    let x = index % frame.width;
    let y = index / frame.width;
    if (x < 4u || y < 4u || x + 4u >= frame.width || y + 4u >= frame.height) {
        return;
    }
    let colors = pass_uniform.colors;
    let left = index - 1u;
    let right = index + 1u;
    let above = index - frame.width;
    let below = index + frame.width;

    let center_h = lab_at(index, colors, 0u);
    let h0 = difference(center_h, lab_at(left, colors, 0u));
    let h1 = difference(center_h, lab_at(right, colors, 0u));
    let h2 = difference(center_h, lab_at(above, colors, 0u));
    let h3 = difference(center_h, lab_at(below, colors, 0u));

    let center_v = lab_at(index, colors, 1u);
    let v0 = difference(center_v, lab_at(left, colors, 1u));
    let v1 = difference(center_v, lab_at(right, colors, 1u));
    let v2 = difference(center_v, lab_at(above, colors, 1u));
    let v3 = difference(center_v, lab_at(below, colors, 1u));

    let leps = min(max(h0.x, h1.x), max(v2.x, v3.x));
    let abeps = min(max(h0.y, h1.y), max(v2.y, v3.y));
    var count_h = 0u;
    var count_v = 0u;
    count_h += within(h0, leps, abeps);
    count_h += within(h1, leps, abeps);
    count_h += within(h2, leps, abeps);
    count_h += within(h3, leps, abeps);
    count_v += within(v0, leps, abeps);
    count_v += within(v1, leps, abeps);
    count_v += within(v2, leps, abeps);
    count_v += within(v3, leps, abeps);
    homogeneity[index] = count_h | (count_v << 8u);
}