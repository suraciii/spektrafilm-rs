// `cielab()` from LibRaw's AHD: the homogeneity statistic AHD compares.
//
// LibRaw accumulates in f32, truncates each XYZ coordinate to an integer LUT
// index, and scales Lab to `short` units. The cube-root table is uploaded as a
// storage buffer because LibRaw builds the same 65536-entry table once and
// indexes it by the computed coordinate.
//
// `xyz_camera` and `cbrt` are the homogeneity stage's own module-scope
// bindings, read directly rather than passed in: naga rejects `ptr<storage>`
// as a function parameter, and this file is only ever concatenated with that
// stage.

fn raw_cielab(
    rgb0: i32,
    rgb1: i32,
    rgb2: i32,
    colors: u32,
) -> vec3<i32> {
    var xyz = vec3<f32>(0.5, 0.5, 0.5);
    let samples = vec3<f32>(f32(rgb0), f32(rgb1), f32(rgb2));
    for (var c: u32 = 0u; c < colors; c = c + 1u) {
        let row0 = xyz_camera[0u][c];
        let row1 = xyz_camera[1u][c];
        let row2 = xyz_camera[2u][c];
        xyz = xyz + vec3<f32>(row0, row1, row2) * samples[c];
    }
    let x = cbrt[raw_clip(i32(xyz.x))];
    let y = cbrt[raw_clip(i32(xyz.y))];
    let z = cbrt[raw_clip(i32(xyz.z))];
    return vec3<i32>(
        i32(64.0 * (116.0 * y - 16.0)),
        i32(64.0 * 500.0 * (x - y)),
        i32(64.0 * 200.0 * (y - z)),
    );
}
