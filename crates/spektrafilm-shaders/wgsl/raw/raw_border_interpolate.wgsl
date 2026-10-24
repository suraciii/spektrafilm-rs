// `border_interpolate(5)`: fill every channel AHD will not compute.
//
// The reference runs this before AHD and only walks the five-pixel border band
// (it skips `col == border` for interior rows by jumping to `width - border`).
// Each fill averages the neighbouring CFA samples of the missing channel, so
// only CFA-sampled values are ever read; the writes never touch a neighbour's
// own channel.


@group(0) @binding(0) var<uniform> frame: RawFrame;
@group(0) @binding(1) var<storage, read_write> image: array<u32>;
@group(0) @binding(2) var<storage, read> cfa: array<u32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let index = id.x + id.y * grid.x * 256u;
    if (index >= raw_pixel_count(frame)) {
        return;
    }
    let x = index % frame.width;
    let y = index / frame.width;
    let border = 5u;
    let interior_row = y >= border && y + border < frame.height;
    let interior_col = x >= border && x + border < frame.width;
    if (interior_row && interior_col) {
        return;
    }
    var sums = vec3<u32>(0u, 0u, 0u);
    var counts = vec3<u32>(0u, 0u, 0u);
    for (var dy: u32 = 0u; dy < 3u; dy = dy + 1u) {
        for (var dx: u32 = 0u; dx < 3u; dx = dx + 1u) {
            let nx = x + dx - 1u;
            let ny = y + dy - 1u;
            if (nx < frame.width && ny < frame.height) {
                let colour = raw_color_from_code(cfa[raw_cfa_index(nx, ny, frame)], frame);
                let sample = image[(ny * frame.width + nx) * 4u + colour];
                if (colour == 0u) {
                    sums.x = sums.x + sample;
                    counts.x = counts.x + 1u;
                } else if (colour == 1u) {
                    sums.y = sums.y + sample;
                    counts.y = counts.y + 1u;
                } else if (colour == 2u) {
                    sums.z = sums.z + sample;
                    counts.z = counts.z + 1u;
                }
            }
        }
    }
    let own = raw_color_from_code(cfa[raw_cfa_index(x, y, frame)], frame);
    if (own != 0u && counts.x != 0u) {
        image[index * 4u + 0u] = sums.x / counts.x;
    }
    if (own != 1u && counts.y != 0u) {
        image[index * 4u + 1u] = sums.y / counts.y;
    }
    if (own != 2u && counts.z != 0u) {
        image[index * 4u + 2u] = sums.z / counts.z;
    }
}