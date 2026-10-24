// Working-geometry rotation and exact pixel crop on retained RGB samples.
//
// The host resolves the quarter turns and the crop rectangle with the same
// rules as `crate::resizing` (`requested_crop_rect`), so this pass only maps a
// destination pixel back to its source pixel. Rotation matches
// `ImageBuf::rotated_quarter_turns` (positive turns are counter-clockwise) and
// the crop copies samples verbatim, exactly like `crop_image`.

struct Params {
    src_width: u32,
    src_height: u32,
    dst_width: u32,
    dst_height: u32,
    // Crop origin inside the rotated frame.
    x0: u32,
    y0: u32,
    // Counter-clockwise quarter turns, 0..=3.
    turns: u32,
    _pad: u32,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> source: array<f32>;
@group(0) @binding(2) var<storage, read_write> out: array<f32>;

@compute @workgroup_size(256)
fn main(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) grid: vec3<u32>,
) {
    let idx = gid.x + gid.y * grid.x * 256u;
    if idx >= p.dst_width * p.dst_height {
        return;
    }
    let dx = idx % p.dst_width;
    let dy = idx / p.dst_width;
    // Rotated-frame coordinate of this destination pixel.
    let x = p.x0 + dx;
    let y = p.y0 + dy;
    var sx = x;
    var sy = y;
    if p.turns == 1u {
        sx = p.src_width - 1u - y;
        sy = x;
    } else if p.turns == 2u {
        sx = p.src_width - 1u - x;
        sy = p.src_height - 1u - y;
    } else if p.turns == 3u {
        sx = y;
        sy = p.src_height - 1u - x;
    }
    let source_index = (sy * p.src_width + sx) * 3u;
    let to = idx * 3u;
    out[to] = source[source_index];
    out[to + 1u] = source[source_index + 1u];
    out[to + 2u] = source[source_index + 2u];
}
