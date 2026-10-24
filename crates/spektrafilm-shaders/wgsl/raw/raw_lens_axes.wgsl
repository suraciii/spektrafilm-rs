// Two serial recurrences reproduce the full-frame call, not row-local starts.
@group(0) @binding(1) var<storage, read_write> axes: array<f32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = id.x + id.y * groups.x * 256u;
    if i > 1u { return; }
    let width = lens.dimensions.x;
    let count = select(width, lens.dimensions.y, i == 1u);
    let offset = select(0u, width, i == 1u);
    let center = select(lens.center.xy, lens.center.zw, i == 1u);
    var value = lens_round(-center);
    for (var n = 0u; n < count; n++) {
        axes[offset + n] = value;
        value = lens_round(lens_ds_add(vec2(value, 0.0), lens.normalization.xy));
    }
}
