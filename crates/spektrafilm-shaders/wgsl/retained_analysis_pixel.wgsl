@group(0) @binding(2) var<storage, read_write> result: array<f32>;
@compute @workgroup_size(1)
fn main() {
    let sample = analysis_sample(p.start);
    result[0] = sample.rgb.x;
    result[1] = sample.rgb.y;
    result[2] = sample.rgb.z;
    result[3] = sample.tone;
    result[4] = sample.ab.x;
    result[5] = sample.ab.y;
    result[6] = select(0.0, 1.0, sample.valid);
}
