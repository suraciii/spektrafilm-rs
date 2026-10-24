// Fixed application of CPU-prepared LittleCMS RGB8 data. No profile parser,
// curve solver, optimizer, transcendental, or profile-specific code is compiled.
struct Params { count: u32, kind: u32, pad0: u32, pad1: u32 }
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> source: array<f32>;
@group(0) @binding(2) var<storage, read> table: array<u32>;
@group(0) @binding(3) var<storage, read_write> output: array<f32>;

fn byte_at(base: u32, index: u32) -> u32 {
    return (table[base + index / 4u] >> ((index % 4u) * 8u)) & 255u;
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let pixel = id.x + id.y * grid.x * 256u;
    if pixel >= params.count { return; }
    var input: vec3<u32>;
    for (var channel = 0u; channel < 3u; channel++) {
        let value = source[pixel * 3u + channel];
        // Rust's RGB8 cast maps NaN to zero as well as clamping finite values.
        // Do not use select: it evaluates the undefined NaN-to-u32 cast too.
        if value != value {
            input[channel] = 0u;
        } else {
            input[channel] = u32(clamp(value, 0.0, 1.0) * 255.0);
        }
    }
    var result = input;
    if params.kind == 1u {
        for (var channel = 0u; channel < 3u; channel++) {
            result[channel] = byte_at(0u, channel * 256u + input[channel]);
        }
    } else if params.kind == 2u {
        var shaped: vec3<i32>;
        for (var channel = 0u; channel < 3u; channel++) {
            shaped[channel] = bitcast<i32>(table[12u + channel * 256u + input[channel]]);
        }
        for (var channel = 0u; channel < 3u; channel++) {
            let row = channel * 3u;
            // Preserve LCMS's wrapping RGB8 Q14 arithmetic and endpoint clamp.
            let value = (bitcast<i32>(table[row]) * shaped.x
                + bitcast<i32>(table[row + 1u]) * shaped.y
                + bitcast<i32>(table[row + 2u]) * shaped.z
                + bitcast<i32>(table[9u + channel]) + 8192i) >> 14u;
            let cell = u32(clamp(value, 0i, 16384i));
            result[channel] = byte_at(780u, channel * 16385u + cell);
        }
    } else if params.kind == 3u {
        let index = (input.x * 65536u + input.y * 256u + input.z) * 3u;
        result = vec3<u32>(byte_at(0u, index), byte_at(0u, index + 1u), byte_at(0u, index + 2u));
    }
    for (var channel = 0u; channel < 3u; channel++) {
        output[pixel * 3u + channel] = f32(result[channel]) / 255.0;
    }
}
