// Device binary64 preparation searches the first positive f32 word whose
// sRGB OETF rounds into the next bin. Binning uses only integer comparisons.
struct Params { pixels: u32, pad0: u32, pad1: u32, pad2: u32 }
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> source: array<u32>;
@group(0) @binding(2) var<storage, read> boundaries: array<u32>;
@group(0) @binding(3) var<storage, read_write> counts: array<atomic<u32>>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let pixel = id.x + id.y * groups.x * 256u;
    if pixel >= params.pixels { return; }
    for (var channel = 0u; channel < 3u; channel++) {
        let bits = source[pixel * 3u + channel];
        var bin = 0u;
        // Negative values and all NaNs cast to the first bin in the CPU path.
        if (bits & 0x80000000u) == 0u && bits <= 0x7f800000u {
            var low = 0u;
            var high = 63u;
            while low < high {
                let middle = (low + high) / 2u;
                if bits >= boundaries[middle] { low = middle + 1u; }
                else { high = middle; }
            }
            bin = low;
        }
        atomicAdd(&counts[channel * 64u + bin], 1u);
    }
}
