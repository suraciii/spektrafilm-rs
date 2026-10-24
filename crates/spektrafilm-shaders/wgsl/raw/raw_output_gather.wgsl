// Requests are device-produced absolute pixel indices. Each source page only
// services its intersections; sample values never cross the host boundary.
struct Gather { first:u32, count:u32, channels:u32, requests:u32 };
@group(0) @binding(0) var<uniform> p:Gather;
@group(0) @binding(1) var<storage,read> indices:array<u32>;
@group(0) @binding(2) var<storage,read> source:array<u32>;
@group(0) @binding(3) var<storage,read_write> values:array<vec4<u32>>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
 let i=id.x+id.y*grid.x*256u;
 if i>=p.requests { return; }
 let index=indices[i];
 if index<p.first || index-p.first>=p.count { return; }
 var value=vec4<u32>(0u);
 for(var c=0u;c<p.channels;c++){ value[c]=source[(index-p.first)*p.channels+c]; }
 values[i]=value;
}
