// Native border_interpolate(8), indexed by full-image CFA coordinates.
struct RawWindow { x:u32, y:u32, width:u32, height:u32 };
@group(0) @binding(0) var<uniform> frame:RawFrame;
@group(0) @binding(1) var<uniform> window:RawWindow;
@group(0) @binding(2) var<storage,read_write> image:array<u32>;
@group(0) @binding(3) var<storage,read> cfa:array<u32>;
fn color(y:u32,x:u32)->u32{return raw_color_from_code(cfa[raw_cfa_index(x,y,frame)],frame);}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>){
    let p=gid.x+gid.y*grid.x*256u;if(p>=window.width*window.height){return;}
    let x=window.x+p%window.width;let y=window.y+p/window.width;
    if(x>=8u && y>=8u && x+8u<frame.width && y+8u<frame.height){return;}
    let f=color(y,x);var sums:array<u32,4>;var weights:array<u32,4>;
    for(var dy=-1;dy<=1;dy++){for(var dx=-1;dx<=1;dx++){
        let xx=i32(x)+dx;let yy=i32(y)+dy;
        if(xx<0 || yy<0 || xx>=i32(frame.width) || yy>=i32(frame.height)){continue;}
        // Gathered-edge invocations are discarded by the copy's core crop.
        if(xx<i32(window.x) || yy<i32(window.y) || xx>=i32(window.x+window.width) || yy>=i32(window.y+window.height)){continue;}
        let c=color(u32(yy),u32(xx));let q=(u32(yy)-window.y)*window.width+u32(xx)-window.x;
        sums[c]+=image[q*4u+c];weights[c]++;
    }}
    for(var c=0u;c<frame.colors;c++){if(c!=f && weights[c]!=0u){image[p*4u+c]=sums[c]/weights[c];}}
}
