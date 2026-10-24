// LibRaw 0.22 aspect_ratio.cpp. Fuji rotation precedes convert_to_rgb;
// stretch follows it. stretch's frac is explicitly int(...), hence zero.
struct GeometryPass {
    dimensions: vec4<u32>, // input width/height, output width/height
    controls: vec4<u32>, // mode (0 Fuji, 1 stretch), Fuji diagonal, colors, axis
    step_bits: vec4<u32>, // exact IEEE754 double stretch increment; unused pads
    step: vec4<f32>, // sqrt(.5) split high/low
};
@group(0) @binding(0) var<uniform> geometry: GeometryPass;
@group(0) @binding(1) var<storage, read> source: array<u32>;
@group(0) @binding(2) var<storage, read_write> destination: array<u32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let iw = geometry.dimensions.x;
    let ih = geometry.dimensions.y;
    let ow = geometry.dimensions.z;
    let oh = geometry.dimensions.w;
    if (geometry.controls.x == 1u) {
        // One invocation per stretched scanline; binary64 recurrence is local,
        // not a host-produced per-pixel correction raster.
        let lane = id.x + id.y * grid.x * 256u;
        let vertical = geometry.controls.w == 1u;
        if (lane >= select(ih, iw, vertical)) { return; }
        var rc = vec2<u32>(0u);
        let length = select(ow, oh, vertical);
        for (var p = 0u; p < length; p++) {
            let coordinate = min(floor_double(rc), select(iw, ih, vertical) - 1u);
            let si = select(lane * iw + coordinate, coordinate * iw + lane, vertical);
            let ti = select(lane * ow + p, p * ow + lane, vertical);
            for (var c = 0u; c < geometry.controls.z; c++) { destination[ti * 4u + c] = source[si * 4u + c]; }
            rc = add_double(rc, geometry.step_bits.xy);
        }
        return;
    }
    let index = id.x + id.y * grid.x * 256u;
    if (index >= ow * oh) { return; }
    let x = index % ow;
    let y = index / ow;
    let rr = ds_add(vec2<f32>(f32(geometry.controls.y), 0.0), ds_mul_int(geometry.step.xy, f32(i32(y) - i32(x))));
    let cc = ds_mul_int(geometry.step.xy, f32(y + x));
    let r = rr.x + rr.y;
    let col = cc.x + cc.y;
    if (r < 0.0 || col < 0.0) { return; }
    let ur = u32(r);
    let uc = u32(col);
    if (ur > ih - 2u || uc > iw - 2u) { return; }
    let fr = r - f32(ur);
    let fc = col - f32(uc);
    let base = (ur * iw + uc) * 4u;
    for (var c = 0u; c < geometry.controls.z; c++) {
        let top = f32(source[base + c]) * (1.0 - fc) + f32(source[base + 4u + c]) * fc;
        let bottom = f32(source[base + iw * 4u + c]) * (1.0 - fc) + f32(source[base + (iw + 1u) * 4u + c]) * fc;
        destination[index * 4u + c] = u32(top * (1.0 - fr) + bottom * fr);
    }
}
