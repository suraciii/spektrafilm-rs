// Merge one chunk maximum into the full-source maximum before applying boost.
struct Params { first: u32, _pad0: u32, _pad1: u32, _pad2: u32 }
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> chunk: array<f32>;
@group(0) @binding(2) var<storage, read_write> maximum: array<f32>;
@compute @workgroup_size(1)
fn main() {
    if params.first != 0u { maximum[0] = chunk[0]; }
    else { maximum[0] = max(maximum[0], chunk[0]); }
}
