// RGB_8 application of device-prepared LCMS default optimization tables.
// Concatenated after viewer_icc_prepare.wgsl: bindings, constants and exact
// u16 interpolation are shared rather than duplicated across entrypoints.
fn icc_opt_apply_matrix(input: vec3<u32>) -> vec3<u32> {
    var shaped: vec3<i32>;
    for (var channel = 0u; channel < 3u; channel++) {
        shaped[channel] = bitcast<i32>(scratch[ICC_OPT_FIRST + channel * 256u + input[channel]]);
    }
    var result: vec3<u32>;
    for (var row = 0u; row < 3u; row++) {
        // i32 overflow wraps as in LCMS's CMS_NO_SANITIZE integer evaluator.
        let position = (bitcast<i32>(scratch[16u + row * 3u]) * shaped.x
            + bitcast<i32>(scratch[17u + row * 3u]) * shaped.y
            + bitcast<i32>(scratch[18u + row * 3u]) * shaped.z
            + bitcast<i32>(scratch[25u + row]) + 8192i) >> 14u;
        let cell = u32(clamp(position, 0i, 16384i));
        result[row] = scratch[ICC_OPT_SECOND + row * 16385u + cell];
    }
    return result;
}

@compute @workgroup_size(64)
fn apply_main(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    // Match the backend's two-dimensional bounded dispatch convention.
    let pixel = id.x + id.y * grid.x * 64u;
    if pixel >= params.n { return; }
    var bytes: vec3<u32>;
    for (var channel = 0u; channel < 3u; channel++) {
        bytes[channel] = u32(floor(clamp(source[pixel * 3u + channel], 0.0, 1.0) * 255.0));
    }
    let mode = scratch[0u];
    var result = bytes * 257u;
    if mode == 1u {
        for (var channel = 0u; channel < 3u; channel++) {
            result[channel] = icc_opt_table16(ICC_OPT_RAMP + channel * 4096u, bytes[channel] * 257u);
        }
    } else if mode == 2u {
        result = icc_opt_apply_matrix(bytes);
    } else if mode == 3u || mode == 4u {
        var input = bytes * 257u;
        if mode == 3u {
            for (var channel = 0u; channel < 3u; channel++) {
                input[channel] = icc_opt_table16(ICC_OPT_RAMP + channel * 4096u, input[channel]);
            }
        }
        result = icc_opt_tetra(input);
    }
    for (var channel = 0u; channel < 3u; channel++) {
        output[pixel * 3u + channel] = f32(icc_opt_byte(result[channel])) / 255.0;
    }
}
