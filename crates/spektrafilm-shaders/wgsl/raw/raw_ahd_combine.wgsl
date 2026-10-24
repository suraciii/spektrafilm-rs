// `ahd_interpolate_combine_homogeneous_pixels`: pick the direction whose 3×3
// homogeneity sum wins, or average the two candidates on a tie.
//
// The chosen values replace the CFA plane's RGB channels in place; the border
// band keeps `border_interpolate`'s fill, because the combine region starts at
// five pixels and the reference never writes outside its own tile interiors.


@group(0) @binding(0) var<uniform> frame: RawFrame;
@group(0) @binding(1) var<storage, read> rgb_h: array<u32>;
@group(0) @binding(2) var<storage, read> rgb_v: array<u32>;
@group(0) @binding(3) var<storage, read> homogeneity: array<u32>;
@group(0) @binding(4) var<storage, read_write> image: array<u32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let index = id.x + id.y * grid.x * 256u;
    if (index >= raw_pixel_count(frame)) {
        return;
    }
    let x = index % frame.width;
    let y = index / frame.width;
    if (x < 5u || y < 5u || x + 5u >= frame.width || y + 5u >= frame.height) {
        return;
    }
    var sum_h = 0u;
    var sum_v = 0u;
    for (var dy: u32 = 0u; dy < 3u; dy = dy + 1u) {
        for (var dx: u32 = 0u; dx < 3u; dx = dx + 1u) {
            let neighbour = homogeneity[(y + dy - 1u) * frame.width + (x + dx - 1u)];
            sum_h += neighbour & 0xFFu;
            sum_v += (neighbour >> 8u) & 0xFFu;
        }
    }
    var red: u32 = 0u;
    var green: u32 = 0u;
    var blue: u32 = 0u;
    if (sum_h != sum_v) {
        if (sum_v > sum_h) {
            red = rgb_v[index * 3u + 0u];
            green = rgb_v[index * 3u + 1u];
            blue = rgb_v[index * 3u + 2u];
        } else {
            red = rgb_h[index * 3u + 0u];
            green = rgb_h[index * 3u + 1u];
            blue = rgb_h[index * 3u + 2u];
        }
    } else {
        red = (rgb_h[index * 3u + 0u] + rgb_v[index * 3u + 0u]) >> 1u;
        green = (rgb_h[index * 3u + 1u] + rgb_v[index * 3u + 1u]) >> 1u;
        blue = (rgb_h[index * 3u + 2u] + rgb_v[index * 3u + 2u]) >> 1u;
    }
    image[index * 4u + 0u] = red;
    image[index * 4u + 1u] = green;
    image[index * 4u + 2u] = blue;
}