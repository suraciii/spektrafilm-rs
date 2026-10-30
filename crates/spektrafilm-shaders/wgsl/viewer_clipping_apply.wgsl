// Apply warnings only inside the photograph, to already-packed presentation pixels.
struct Params { count:u32, mode:u32, blue:u32, red:u32,
 first:u32, width:u32, height:u32, pad0:u32,
 rect:vec4<f32>, image_rect:vec4<f32>, }
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read> mask:array<u32>;
@group(0) @binding(2) var<storage,read_write> pixels:array<u32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>, @builtin(num_workgroups) grid:vec3<u32>) {
 let local=gid.x+gid.y*grid.x*256u;
 if local>=p.count { return; }
 let i=p.first+local;
 let center=p.rect.xy+(vec2<f32>(f32(i%p.width),f32(i/p.width))+vec2<f32>(0.5))*(p.rect.zw-p.rect.xy)/vec2<f32>(f32(p.width),f32(p.height));
 if !all(center>=p.image_rect.xy) || !all(center<p.image_rect.zw) { return; }
 let flags=mask[local];
 if p.mode==4u { pixels[local]=select(0xffffffffu,0xff000000u,(flags&1u)!=0u); }
 else if p.mode==5u { pixels[local]=select(0xff000000u,0xffffffffu,(flags&2u)!=0u); }
 else if (p.mode&2u)!=0u && (flags&2u)!=0u { pixels[local]=p.red; }
 else if (p.mode&1u)!=0u && (flags&1u)!=0u { pixels[local]=p.blue; }
}
