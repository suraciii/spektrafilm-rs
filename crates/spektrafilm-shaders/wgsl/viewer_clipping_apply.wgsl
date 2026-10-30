// Apply warnings only inside the photograph, to already-packed presentation pixels.
struct Params { count:u32, mode:u32, blue:u32, red:u32,
 first:u32, width:u32, x0:u32, y0:u32, x_len:u32, pad0:u32, pad1:u32, pad2:u32, }
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read> mask:array<u32>;
@group(0) @binding(2) var<storage,read> geometry:array<vec2<u32>>;
@group(0) @binding(3) var<storage,read_write> pixels:array<u32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>, @builtin(num_workgroups) grid:vec3<u32>) {
 let local=gid.x+gid.y*grid.x*256u;
 if local>=p.count { return; }
 let i=p.first+local;
 if geometry[i%p.width-p.x0].x==0xffffffffu || geometry[p.x_len+i/p.width-p.y0].x==0xffffffffu { return; }
 let flags=mask[local];
 if p.mode==4u { pixels[local]=select(0xffffffffu,0xff000000u,(flags&1u)!=0u); }
 else if p.mode==5u { pixels[local]=select(0xff000000u,0xffffffffu,(flags&2u)!=0u); }
 else if (p.mode&2u)!=0u && (flags&2u)!=0u { pixels[local]=p.red; }
 else if (p.mode&1u)!=0u && (flags&1u)!=0u { pixels[local]=p.blue; }
}
