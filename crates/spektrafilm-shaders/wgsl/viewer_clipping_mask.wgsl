// OR native endpoint classifications into a viewport-sized mask, one page at a time.
struct Params {
 sw:u32, count:u32, page_first:u32, page_count:u32,
 shadow:f32, highlight:f32, first:u32, width:u32,
 x0:u32, y0:u32, x_len:u32, block_start:u32,
 block_count:u32, pad0:u32, pad1:u32, pad2:u32,
}
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read> source:array<f32>;
@group(0) @binding(2) var<storage,read> geometry:array<vec2<u32>>;
@group(0) @binding(3) var<storage,read_write> mask:array<atomic<u32>>;
fn finite(v:f32)->bool { return (bitcast<u32>(v)&0x7f800000u)!=0x7f800000u; }
// Number of rectangle pixels preceding a linear native image boundary.
fn selected_before(boundary:u32, x:vec2<u32>, y:vec2<u32>)->u32 {
 let row=boundary/p.sw;
 let col=boundary%p.sw;
 let w=x.y-x.x;
 let rows=min(max(row,y.x)-y.x,y.y-y.x);
 var partial=0u;
 if row>=y.x && row<y.y { partial=min(max(col,x.x)-x.x,w); }
 return rows*w+partial;
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>, @builtin(num_workgroups) grid:vec3<u32>) {
 let invocation=gid.x+gid.y*grid.x*256u;
 if invocation>=p.count*p.block_count { return; }
 let local=invocation%p.count;
 let block=p.block_start+invocation/p.count;
 let i=p.first+local;
 let x=geometry[i%p.width-p.x0];
 let y=geometry[p.x_len+i/p.width-p.y0];
 if x.x==0xffffffffu || y.x==0xffffffffu || x.x>=x.y || y.x>=y.y { return; }
 let begin=selected_before(p.page_first,x,y);
 let end=selected_before(p.page_first+p.page_count,x,y);
 let offset=block*256u;
 if offset>=end-begin { return; }
 let count=min(256u,end-begin-offset);
 let w=x.y-x.x;
 var flags=0u;
 for(var j=0u; j<count; j++) {
  let selected=begin+offset+j;
  let source_pixel=(y.x+selected/w)*p.sw+x.x+selected%w;
  let k=(source_pixel-p.page_first)*3u;
  let rgb=vec3<f32>(source[k],source[k+1u],source[k+2u]);
  if finite(rgb.x) && finite(rgb.y) && finite(rgb.z) {
   if all(rgb<=vec3<f32>(p.shadow)) { flags|=1u; }
   if any(rgb>=vec3<f32>(p.highlight)) { flags|=2u; }
   if flags==3u { break; }
  }
 }
 if flags!=0u { atomicOr(&mask[local],flags); }
}
