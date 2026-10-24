// `scale_colors_loop`: the reference's integer white-balance scale.
//
// LibRaw truncates `int(val * scale_mul[c])` and clamps to 0..65535. After
// `raw2image_ex` the per-channel black offsets are already folded into the
// samples, so the remaining subtraction is the repeating black map when the
// decoder provided one. `pre_interpolate`'s green merge follows the scale,
// exactly as it follows `scale_colors` in `dcraw_process`.

@group(0) @binding(0) var<uniform> frame: RawFrame;
@group(0) @binding(1) var<storage, read> scale: array<f32>;
@group(0) @binding(2) var<storage, read> cfa: array<u32>;
@group(0) @binding(3) var<storage, read> black_map: array<u32>;
@group(0) @binding(4) var<storage, read_write> image: array<u32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let index = id.x + id.y * grid.x * 256u;
    if (index >= raw_pixel_count(frame)) {
        return;
    }
    let x = index % frame.width;
    let y = index / frame.width;
    for (var color = 0u; color < 4u; color++) {
        var value = i32(image[index * 4u + color]);
        if ((frame.mix_green & 2u) != 0u && value > 32767) { value = 0; }
        if (frame.has_black_map != 0u && frame.subtract_black_map == 0u) {
            if (value != 0) {
                let offset = i32(black_map[(y % frame.black_rows) * frame.black_columns + (x % frame.black_columns)]);
                value = i32(f32(value - offset) * scale[color]);
            }
        } else {
            value = i32(f32(value) * scale[color]);
        }
        image[index * 4u + color] = raw_clip(value);
    }
    if (raw_is_merge_site(x, y, frame)) {
        // Channel 3 is zero everywhere the merge loop is not a real second
        // green, so this copy reproduces the reference even then.
        image[index * 4u + 1u] = image[index * 4u + 3u];
    }
}
