// `ahd_interpolate_green_h_and_v`: the horizontal and vertical green
// candidates at every non-green site. LibRaw evaluates the same expressions on
// its 512-pixel tile grid; the candidates depend only on the pixel and its
// immediate neighbours, so a whole-image dispatch is the identical arithmetic.


@group(0) @binding(0) var<uniform> frame: RawFrame;
@group(0) @binding(1) var<storage, read> image: array<u32>;
@group(0) @binding(2) var<storage, read> cfa: array<u32>;
@group(0) @binding(3) var<storage, read_write> green_h: array<u32>;
@group(0) @binding(4) var<storage, read_write> green_v: array<u32>;

/// This stage's read of `image`. The index rule itself stays in
/// `raw_sample_index`; only the buffer identifier is local, because
/// `raw_common.wgsl` is shared with stages that declare no `image`.
fn sample_at(x: u32, y: u32, channel: u32) -> i32 {
    return i32(image[raw_sample_index(x, y, frame, channel)]);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let index = id.x + id.y * grid.x * 256u;
    if (index >= raw_pixel_count(frame)) {
        return;
    }
    let x = index % frame.width;
    let y = index / frame.width;
    if (x < 2u || y < 2u || x + 2u >= frame.width || y + 2u >= frame.height) {
        return;
    }
    let color = raw_color_from_code(cfa[raw_cfa_index(x, y, frame)], frame);
    if (color == 1u) {
        return;
    }
    let center = sample_at(x, y, color);
    let horizontal = ((sample_at(x - 1u, y, 1u) + center
        + sample_at(x + 1u, y, 1u)) * 2
        - sample_at(x - 2u, y, color)
        - sample_at(x + 2u, y, color)) >> 2;
    green_h[index] = raw_ulim(
        horizontal,
        sample_at(x - 1u, y, 1u),
        sample_at(x + 1u, y, 1u),
    );
    let vertical = ((sample_at(x, y - 1u, 1u) + center
        + sample_at(x, y + 1u, 1u)) * 2
        - sample_at(x, y - 2u, color)
        - sample_at(x, y + 2u, color)) >> 2;
    green_v[index] = raw_ulim(
        vertical,
        sample_at(x, y - 1u, 1u),
        sample_at(x, y + 1u, 1u),
    );
}