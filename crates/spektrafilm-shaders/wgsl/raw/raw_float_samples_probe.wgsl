// Test boundary for the same inline conversion used by bounded page ingress.
@group(0) @binding(0) var<storage,read> input:array<f32>;
@group(0) @binding(1) var<storage,read> conversion:RawFloatConversion;
@group(0) @binding(2) var<storage,read_write> output:array<u32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id:vec3<u32>){
    if(id.x<arrayLength(&input)){output[id.x]=raw_float_sample(input[id.x],conversion);}
}
