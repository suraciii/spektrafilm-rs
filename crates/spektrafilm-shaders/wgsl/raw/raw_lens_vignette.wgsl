// RGB makes Lensfun's SSE colour callback use its scalar fallback. Its r²
// recurrence must run left-to-right; replacing it with x*x+y*y changes pixels.
@group(0) @binding(1) var<storage, read> axes: array<f32>;
@group(0) @binding(2) var<storage, read> source: array<f32>;
@group(0) @binding(3) var<storage, read_write> corrected: array<f32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let row = id.x + id.y * groups.x * 256u;
    let width = lens.dimensions.x;
    if row >= lens.dimensions.y { return; }
    var x = axes[0] * lens.mapping.x;
    let y = axes[width + row] * lens.mapping.x;
    var r2 = x * x + y * y;
    let step = lens.vignette.w;
    let d1 = 2.0 * step;
    let d2 = step * step;
    for (var col = 0u; col < width; col++) {
        let r4 = r2 * r2;
        let r6 = r4 * r2;
        // C++'s 1.0 promotes these additions to double, after each float product.
        var denominator_pair = lens_ds_add(vec2(1.0, 0.0), vec2(lens.vignette.x * r2, 0.0));
        denominator_pair = lens_ds_add(denominator_pair, vec2(lens.vignette.y * r4, 0.0));
        denominator_pair = lens_ds_add(denominator_pair, vec2(lens.vignette.z * r6, 0.0));
        let denominator = lens_round(denominator_pair);
        let gain = 1.0 / denominator;
        let index = (row * width + col) * 3u;
        for (var c = 0u; c < 3u; c++) {
            var value = source[index + c];
            if lens.flags.z != 0u { value = max(value * gain, 0.0); }
            corrected[index + c] = value;
        }
        r2 += d1 * x + d2;
        x += step;
    }
}
