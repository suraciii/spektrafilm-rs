// LibRaw 0.22 raw2image_ex/copy_fuji_uncropped and subtract_black_internal.
@group(0) @binding(0) var<uniform> frame: RawFrame;
@group(0) @binding(1) var<storage, read> samples: array<f32>;
@group(0) @binding(2) var<storage, read> cfa: array<u32>;
@group(0) @binding(3) var<storage, read> channel_black: array<u32>;
@group(0) @binding(4) var<storage, read_write> image: array<u32>;
@group(0) @binding(5) var<storage, read_write> max_value: array<atomic<u32>>;
@group(0) @binding(6) var<storage, read> black_map: array<u32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let index = id.x + id.y * grid.x * 256u;
    if (index >= frame.source_width * frame.source_height) { return; }
    let x = index % frame.source_width;
    let y = index / frame.source_width;
    var row = i32(y);
    var col = i32(x);
    if (frame.fuji_width != 0u) {
        if (frame.fuji_layout != 0u) {
            row = i32(frame.fuji_width) - 1 - i32(x) + i32(y >> 1u);
            col = i32(x + ((y + 1u) >> 1u));
        } else {
            row = i32(frame.fuji_width) - 1 + i32(y) - i32(x >> 1u);
            col = i32(y + ((x + 1u) >> 1u));
        }
    }
    if (row < 0 || col < 0 || row >= i32(frame.height) || col >= i32(frame.width)) { return; }
    let destination = u32(row) * frame.width + u32(col);
    let site_color = cfa[raw_cfa_index(u32(col), u32(row), frame)];
    for (var c = 0u; c < frame.channels; c++) {
        let color = select(c, site_color, frame.channels == 1u);
        var value = i32(samples[index * frame.channels + c]);
        // copy_bayer uses ushort normalized black; interleaved sensors use
        // subtract_black_internal's full-width offsets and repeating map.
        value -= i32(select(channel_black[color], channel_black[color] & 65535u, frame.channels == 1u));
        if (frame.subtract_black_map != 0u && frame.has_black_map != 0u && frame.channels > 1u) {
            value -= i32(black_map[(u32(row) % frame.black_rows) * frame.black_columns + u32(col) % frame.black_columns]);
        }
        let clipped = raw_clip(value);
        image[destination * 4u + color] = clipped;
        atomicMax(&max_value[0], clipped);
    }
}
