// LibRaw 0.22.2 postprocessing_utils_dcrdefs.cpp:123-159.
// Each native 8x8 block is accepted atomically: one saturated channel rejects
// all of its samples. Entries0..3 are unsigned sums,4..7 sample counts.
@group(0) @binding(0) var<uniform> frame:RawFrame;
@group(0) @binding(1) var<uniform> p:ScaleParams;
@group(0) @binding(2) var<storage,read> measured:array<u32>;
@group(0) @binding(3) var<storage,read> image:array<u32>;
@group(0) @binding(4) var<storage,read> cfa:array<u32>;
@group(0) @binding(5) var<storage,read_write> block_sums:array<u32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>, @builtin(num_workgroups) grid:vec3<u32>){
    let block=gid.x+gid.y*grid.x*256u;let columns=(frame.width+7u)/8u;let rows=(frame.height+7u)/8u;
    if(block>=columns*rows){return;}
    let top=(block/columns)*8u;let left=(block%columns)*8u;var sums:array<u32,8>;
    var reject=false;let maximum=i32(raw_adjusted_maximum(p,measured[0]))-25;
    for(var y=top;y<min(top+8u,frame.height);y++){
        for(var x=left;x<min(left+8u,frame.width);x++){
            let pixel=(y*frame.width+x)*4u;
            if(frame.channels==1u){
                // Auto WB precedes pre_interpolate: do not merge code3.
                let c=cfa[raw_cfa_index(x,y,frame)];let value=image[pixel+c];
                if(i32(value)>maximum){reject=true;}
                sums[c]+=value;sums[c+4u]++;
            }else{
                // Native FORC4 visits even the unused fourth image slot.
                for(var c=0u;c<4u;c++){
                    var value=image[pixel+c];
                    if((frame.mix_green&2u)!=0u&&value>32767u){value=0u;}
                    if(i32(value)>maximum){reject=true;}
                    sums[c]+=value;sums[c+4u]++;
                }
            }
        }
    }
    for(var c=0u;c<8u;c++){block_sums[block*8u+c]=select(sums[c],0u,reject);}
}
