// `ahd_interpolate_r_and_b_in_rgb_and_convert_to_cielab`, one homogeneity
// direction per dispatch. Direction 0 reads the horizontal green candidates,
// direction 1 the vertical ones; LibRaw runs the same loop twice.
//
// The reference addresses its per-tile buffers with flattened literals
// (`pix[-4 + c]`, `rix[-512 - 1]`). The same expressions here use the image
// row stride, because the tile-local arrays only exist to bound host memory;
// the arithmetic and the read set are identical.


fn green_at(x: u32, y: u32, frame: RawFrame, pass_uniform: AhdPass) -> i32 {
    if (pass_uniform.direction == 0u) {
        return i32(green_h[y * frame.width + x]);
    }
    return i32(green_v[y * frame.width + x]);
}

@group(0) @binding(0) var<uniform> frame: RawFrame;
@group(0) @binding(1) var<uniform> pass_uniform: AhdPass;
@group(0) @binding(2) var<storage, read> image: array<u32>;
@group(0) @binding(3) var<storage, read> cfa: array<u32>;
@group(0) @binding(4) var<storage, read> green_h: array<u32>;
@group(0) @binding(5) var<storage, read> green_v: array<u32>;
@group(0) @binding(6) var<storage, read_write> out_rgb: array<u32>;

/// This stage's read of `image`; the index rule stays in `raw_sample_index`.
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
    if (x < 3u || y < 3u || x + 3u >= frame.width || y + 3u >= frame.height) {
        return;
    }
    let color = raw_color_from_code(cfa[raw_cfa_index(x, y, frame)], frame);
    var red: u32 = 0u;
    var green: u32 = 0u;
    var blue: u32 = 0u;
    if (color == 1u) {
        let vertical = raw_color_from_code(cfa[raw_cfa_index(x, y + 1u, frame)], frame);
        let other = 2u - vertical;
        let center_green = sample_at(x, y, 1u);
        let horizontal_value = center_green
            + ((sample_at(x - 1u, y, other) + sample_at(x + 1u, y, other)
                - green_at(x - 1u, y, frame, pass_uniform) - green_at(x + 1u, y, frame, pass_uniform)) >> 1);
        let vertical_value = center_green
            + ((sample_at(x, y - 1u, vertical) + sample_at(x, y + 1u, vertical)
                - green_at(x, y - 1u, frame, pass_uniform) - green_at(x, y + 1u, frame, pass_uniform)) >> 1);
        if (other == 0u) {
            red = raw_clip(horizontal_value);
            blue = raw_clip(vertical_value);
        } else {
            blue = raw_clip(horizontal_value);
            red = raw_clip(vertical_value);
        }
        green = u32(center_green);
    } else {
        green = u32(green_at(x, y, frame, pass_uniform));
        let other = 2u - color;
        let diagonal = green_at(x, y, frame, pass_uniform)
            + ((sample_at(x - 1u, y - 1u, other) + sample_at(x + 1u, y - 1u, other)
                + sample_at(x - 1u, y + 1u, other) + sample_at(x + 1u, y + 1u, other)
                - green_at(x - 1u, y - 1u, frame, pass_uniform) - green_at(x + 1u, y - 1u, frame, pass_uniform)
                - green_at(x - 1u, y + 1u, frame, pass_uniform) - green_at(x + 1u, y + 1u, frame, pass_uniform) + 1) >> 2);
        let value = raw_clip(diagonal);
        if (other == 0u) {
            red = value;
        } else {
            blue = value;
        }
        if (color == 0u) {
            red = u32(sample_at(x, y, 0u));
        } else {
            blue = u32(sample_at(x, y, 2u));
        }
    }
    out_rgb[index * 3u + 0u] = red;
    out_rgb[index * 3u + 1u] = green;
    out_rgb[index * 3u + 2u] = blue;
}