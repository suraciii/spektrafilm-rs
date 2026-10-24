// `convert_to_rgb` plus the loader's own post-decode stages: apply
// `out_cam`, quantize to the reference's 16-bit samples, normalize, apply the
// requested CAT02 adaptation, apply the custom green tint, and place the
// result through LibRaw's orientation mapping.
//
// `camera_to_aces` is `convert_to_rgb`'s `out_cam` for ACES AP0 output; the
// two quantization steps (the matrix result and the 16-bit sample) are the
// reference's own, so the device output lands in the same input domain.

struct ColorPass {
    out_width: u32,
    out_height: u32,
    iwidth: u32,
    iheight: u32,
    flip: u32,
    colors: u32,
    adaptation_active: u32,
    _pad0: u32,
    tint: f32,
    _pad1: f32,
    _pad2: f32,
    _pad3: f32,
    camera_to_aces: array<vec4<f32>, 3>,
    adaptation: array<vec4<f32>, 3>,
    aces_to_xyz: array<vec4<f32>, 3>,
    xyz_to_aces: array<vec4<f32>, 3>,
};


@group(0) @binding(0) var<uniform> pass_uniform: ColorPass;
@group(0) @binding(1) var<storage, read> image: array<u32>;
@group(0) @binding(2) var<storage, read_write> output: array<f32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let index = id.x + id.y * grid.x * 256u;
    if (index >= pass_uniform.out_width * pass_uniform.out_height) {
        return;
    }
    let ox = index % pass_uniform.out_width;
    let oy = index / pass_uniform.out_width;
    // `flip_index(row, col)`: bit 4 swaps the axes, bit 2 flips rows, bit 1
    // flips columns. `iwidth`/`iheight` are the pre-swap active dimensions.
    var row = oy;
    var col = ox;
    if ((pass_uniform.flip & 4u) != 0u) {
        let swap = row;
        row = col;
        col = swap;
    }
    if ((pass_uniform.flip & 2u) != 0u) {
        row = pass_uniform.iheight - 1u - row;
    }
    if ((pass_uniform.flip & 1u) != 0u) {
        col = pass_uniform.iwidth - 1u - col;
    }
    let source = row * pass_uniform.iwidth + col;
    // Camera conversion has already quantized to ushort before stretch.
    var rgb = vec3<f32>(
        f32(image[source * 4u + 0u]) / 65535.0,
        f32(image[source * 4u + 1u]) / 65535.0,
        f32(image[source * 4u + 2u]) / 65535.0,
    );
    if (pass_uniform.adaptation_active != 0u) {
        let xyz = vec3<f32>(
            dot(pass_uniform.aces_to_xyz[0].xyz, rgb),
            dot(pass_uniform.aces_to_xyz[1].xyz, rgb),
            dot(pass_uniform.aces_to_xyz[2].xyz, rgb),
        );
        let adapted = vec3<f32>(
            dot(pass_uniform.adaptation[0].xyz, xyz),
            dot(pass_uniform.adaptation[1].xyz, xyz),
            dot(pass_uniform.adaptation[2].xyz, xyz),
        );
        rgb = vec3<f32>(
            dot(pass_uniform.xyz_to_aces[0].xyz, adapted),
            dot(pass_uniform.xyz_to_aces[1].xyz, adapted),
            dot(pass_uniform.xyz_to_aces[2].xyz, adapted),
        );
    }
    output[index * 3u + 0u] = rgb.x;
    output[index * 3u + 1u] = rgb.y;
    output[index * 3u + 2u] = rgb.z;
}