// OR native endpoint classifications into a viewport-sized mask, one page at a time.
struct Params {
 sw:u32, count:u32, page_first:u32, page_count:u32,
 shadow:u32, highlight:u32, first:u32, width:u32,
 x0:u32, y0:u32, x_len:u32, block_start:u32,
 block_count:u32, binary64:u32, shadow_hi:u32, highlight_hi:u32,
}
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read> source:array<u32>;
@group(0) @binding(2) var<storage,read> geometry:array<vec2<u32>>;
@group(0) @binding(3) var<storage,read_write> mask:array<atomic<u32>>;
fn finite(v:f32)->bool { return (bitcast<u32>(v)&0x7f800000u)!=0x7f800000u; }
// Ordered IEEE binary64 comparison, with positive and negative zero equal.
fn less64(a:vec2<u32>, b:vec2<u32>)->bool {
 let az=(a.y&0x7fffffffu)==0u && a.x==0u;
 let bz=(b.y&0x7fffffffu)==0u && b.x==0u;
 if az && bz { return false; }
 let an=(a.y&0x80000000u)!=0u;
 let bn=(b.y&0x80000000u)!=0u;
 if an!=bn { return an; }
 let lt=a.y<b.y || (a.y==b.y && a.x<b.x);
 let gt=a.y>b.y || (a.y==b.y && a.x>b.x);
 return select(lt,gt,an);
}
fn finite64(a:vec2<u32>)->bool { return (a.y&0x7ff00000u)!=0x7ff00000u; }
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
  if p.binary64!=0u {
   let k=(source_pixel-p.page_first)*6u;
   let r=vec2<u32>(source[k],source[k+1u]);
   let g=vec2<u32>(source[k+2u],source[k+3u]);
   let b=vec2<u32>(source[k+4u],source[k+5u]);
   let shadow=vec2<u32>(p.shadow,p.shadow_hi);
   let highlight=vec2<u32>(p.highlight,p.highlight_hi);
   if finite64(r) && finite64(g) && finite64(b) {
    if !less64(shadow,r) && !less64(shadow,g) && !less64(shadow,b) { flags|=1u; }
    if !less64(r,highlight) || !less64(g,highlight) || !less64(b,highlight) { flags|=2u; }
   }
  } else {
   let k=(source_pixel-p.page_first)*3u;
   let rgb=vec3<f32>(bitcast<f32>(source[k]),bitcast<f32>(source[k+1u]),bitcast<f32>(source[k+2u]));
   if finite(rgb.x) && finite(rgb.y) && finite(rgb.z) {
    if all(rgb<=vec3<f32>(bitcast<f32>(p.shadow))) { flags|=1u; }
    if any(rgb>=vec3<f32>(bitcast<f32>(p.highlight))) { flags|=2u; }
   }
  }
  if flags==3u { break; }
 }
 if flags!=0u { atomicOr(&mask[local],flags); }
}
