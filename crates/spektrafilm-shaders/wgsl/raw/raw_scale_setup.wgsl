// LibRaw0.22.2 scale_colors. Concatenate raw_common + raw_scale_helpers.
@group(0) @binding(0) var<uniform> p:ScaleParams;
@group(0) @binding(1) var<storage,read> measured:array<u32>;
@group(0) @binding(2) var<storage,read_write> scales:array<f32>;
@group(0) @binding(3) var<uniform> frame:RawFrame;
@group(0) @binding(4) var<storage,read> block_sums:array<u32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id:vec3<u32>){
    if(id.x!=0u){return;}
    let maximum=raw_adjusted_maximum(p,measured[0]);
    var white=p.white;
    if(p.auto_white_balance!=0u){
        var totals:array<vec2<u32>,8>;
        let blocks=((frame.width+7u)/8u)*((frame.height+7u)/8u);
        for(var block=0u;block<blocks;block++){
            for(var c=0u;c<8u;c++){totals[c]=add64(totals[c],vec2(block_sums[block*8u+c],0u));}
        }
        for(var c=0u;c<4u;c++){if(nonzero64(totals[c])){white[c]=native_ratio(totals[c+4u],totals[c]);}}
    }
    let result=raw_resolve_scales(white,maximum,frame.colors);
    for(var c=0u;c<4u;c++){scales[c]=result[c];}
}
