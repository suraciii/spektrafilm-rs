// Bounded LibRaw auto-WB reduction. Concatenate raw_common + raw_scale_helpers.
// Mode0 consumes one native8x8 block (8u32); mode1 publishes global scales.
// Totals start zero and every accumulation is a separate ordered dispatch.
struct WbTotalsPass { mode:u32, pad0:u32, pad1:u32, pad2:u32 };
@group(0) @binding(0) var<uniform> p:ScaleParams;
@group(0) @binding(1) var<storage,read> measured:array<u32>;
@group(0) @binding(2) var<storage,read_write> scales:array<f32>;
@group(0) @binding(3) var<uniform> frame:RawFrame;
@group(0) @binding(4) var<storage,read> block_sums:array<u32>;
@group(0) @binding(5) var<storage,read_write> totals:array<vec2<u32>,8>;
@group(0) @binding(6) var<uniform> settings:WbTotalsPass;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id:vec3<u32>){
    if(id.x!=0u){return;}
    if(settings.mode==0u){
        for(var c=0u;c<8u;c++){totals[c]=add64(totals[c],vec2(block_sums[c],0u));}
        return;
    }
    var white=p.white;
    if(p.auto_white_balance!=0u){
        for(var c=0u;c<4u;c++){if(nonzero64(totals[c])){white[c]=native_ratio(totals[c+4u],totals[c]);}}
    }
    let result=raw_resolve_scales(white,raw_adjusted_maximum(p,measured[0]),frame.colors);
    for(var c=0u;c<4u;c++){scales[c]=result[c];}
}
