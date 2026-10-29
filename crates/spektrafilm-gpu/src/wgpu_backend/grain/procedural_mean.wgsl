// Exact mean of the grain a Procedural scan composes.
//
// The scan averages nine canvas cells per pixel, so the mean it needs is the
// mean of those averages, not the mean of the cells. `mean_main` walks a pixel
// window and writes one partial per workgroup; `reduce_main` folds the partials
// into the three channels the composition subtracts.
//
// Uniform layout: dimensions [pixel_width,pixel_height,first_pixel,count],
// canvas [cells_wide,cells_high,first_partial,workgroups],
// scale [resampler_scale,first_canvas_row,canvas_width,canvas_height].
struct Params { dimensions:vec4<u32>, canvas:vec4<u32>, scale:vec4<f32> }
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read> canvas_cells:array<u32>;
@group(0) @binding(2) var<storage,read_write> partials:array<f32>;

// `canvas.z` is the first partial this dispatch writes and `scale.y` the first
// canvas row the band buffer stores.
fn cell_at(coord:vec2<i32>)->vec3<f32>{
 let c=clamp(coord,vec2<i32>(0),vec2<i32>(p.canvas.xy)-1i);
 let i=((u32(c.y)-u32(p.scale.y))*p.canvas.x+u32(c.x))*2u;
 return vec3(unpack2x16float(canvas_cells[i]),unpack2x16float(canvas_cells[i+1u]).x);
}

var<workgroup> workgroup_sum:array<vec4<f32>,256>;

@compute @workgroup_size(256)
fn mean_main(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>,@builtin(local_invocation_index) lane:u32,@builtin(workgroup_id) group:vec3<u32>){
 let local=gid.x+gid.y*grid.x*256u;
 let width=p.dimensions.x;let height=p.dimensions.y;
 var acc=vec4<f32>(0.);
 if(local<p.dimensions.w){
  let idx=p.dimensions.z+local;
  let x=idx%width;let y=idx/width;
  let size=p.scale.zw;
  let uv=vec2<f32>(f32(x),f32(y))*vec2(1./f32(max(width-1u,1u)),1./f32(max(height-1u,1u)));
  let centre=vec2<i32>(uv*(size/p.scale.x));
  var sum=vec3<f32>(0.);
  for(var dy=-1;dy<=1;dy++){for(var dx=-1;dx<=1;dx++){sum+=cell_at(centre+vec2<i32>(dx,dy));}}
  acc=vec4<f32>(sum*(1./9.),1.);
 }
 workgroup_sum[lane]=acc;
 workgroupBarrier();
 for(var stride=128u;stride>0u;stride>>=1u){
  if(lane<stride){workgroup_sum[lane]+=workgroup_sum[lane+stride];}
  workgroupBarrier();
 }
 if(lane==0u){
  let base=(p.canvas.z+group.x+group.y*grid.x)*4u;
  partials[base]=workgroup_sum[0].x;partials[base+1u]=workgroup_sum[0].y;partials[base+2u]=workgroup_sum[0].z;partials[base+3u]=workgroup_sum[0].w;
 }
}

@compute @workgroup_size(256)
fn reduce_main(@builtin(local_invocation_index) lane:u32){
 var acc=vec4<f32>(0.);
 let groups=p.canvas.w;
 for(var i=lane;i<groups;i+=256u){
  let base=i*4u;
  acc+=vec4<f32>(partials[base],partials[base+1u],partials[base+2u],partials[base+3u]);
 }
 workgroup_sum[lane]=acc;
 workgroupBarrier();
 for(var stride=128u;stride>0u;stride>>=1u){
  if(lane<stride){workgroup_sum[lane]+=workgroup_sum[lane+stride];}
  workgroupBarrier();
 }
 if(lane==0u){
  let n=max(workgroup_sum[0].w,1.);
  partials[0]=workgroup_sum[0].x/n;partials[1]=workgroup_sum[0].y/n;partials[2]=workgroup_sum[0].z/n;
 }
}
