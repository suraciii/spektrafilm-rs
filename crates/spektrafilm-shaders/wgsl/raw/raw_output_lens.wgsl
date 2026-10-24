struct Tile { first:u32, count:u32, mode:u32, pad:u32 };
@group(0) @binding(1) var<uniform> tile:Tile;
@group(0) @binding(2) var<storage,read_write> requests:array<u32>;
@group(0) @binding(3) var<storage,read> values:array<vec4<u32>>;
@group(0) @binding(4) var<storage,read_write> coords:array<vec4<f32>>;
@group(0) @binding(5) var<storage,read_write> output:array<f32>;
@group(0) @binding(6) var<storage,read_write> bounds:array<atomic<u32>>;
@group(0) @binding(7) var<storage,read_write> state:array<f32>;
fn request(slot:u32,index:u32){requests[slot]=index;atomicMin(&bounds[0],index);atomicMax(&bounds[1],index);}
fn weight(value:f32,x:vec2<f32>,y:vec2<f32>)->vec2<f32>{return lens_ds_mul(lens_ds_mul(vec2(value,0.0),x),y);}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>){
 let i=id.x+id.y*grid.x*256u;if i>=tile.count{return;}
 let index=tile.first+i;let width=lens.dimensions.x;
 let simd=lens.flags.w!=0u&&(lens.dimensions.z==1u||lens.dimensions.z==3u);
 let stride=select(2u,4u,simd);
 if tile.mode==0u {
  let x=index%width;
  request(i*stride,x);request(i*stride+1u,width+index/width);
  if simd {request(i*stride+2u,max(x,1u)-1u);request(i*stride+3u,min(x+1u,width-1u));}
 }else if tile.mode==1u {
  let initial=vec2(bitcast<f32>(values[i*stride].x),bitcast<f32>(values[i*stride+1u].x));
  let p=lens_project(initial*lens.mapping.w);
  var previous=p;var next=p;
  if simd {
   previous=lens_project(vec2(bitcast<f32>(values[i*stride+2u].x),initial.y)*lens.mapping.w);
   next=lens_project(vec2(bitcast<f32>(values[i*stride+3u].x),initial.y)*lens.mapping.w);
  }
  for(var c=0u;c<3u;c++){
   let distorted=lens_combined_distort(p,previous,next,index%width,index/width,c);
   let raw=lens_pixel(lens_tca(distorted,c));let slot=i*3u+c;
   let valid=raw.x==raw.x&&raw.y==raw.y;
   let q=clamp(raw,vec2(0.0),vec2<f32>(lens.dimensions.xy-vec2(1u)));
   coords[slot]=vec4(q,f32(valid),0.0);
   let x=u32(q.x);let y=u32(q.y);let x1=min(x+1u,width-1u);let y1=min(y+1u,lens.dimensions.y-1u);
   for(var k=0u;k<4u;k++){requests[slot*4u+k]=0xffffffffu;}
   if valid {request(slot*4u,y*width+x);request(slot*4u+1u,y*width+x1);request(slot*4u+2u,y1*width+x);request(slot*4u+3u,y1*width+x1);}
  }
 }else if tile.mode==2u {
  for(var c=0u;c<3u;c++){
   let slot=i*3u+c;let q=coords[slot];var result=0.0;
   if q.z!=0.0 {
    let dx=lens_ds_add(vec2(q.x,0.0),vec2(-f32(u32(q.x)),0.0));let dy=lens_ds_add(vec2(q.y,0.0),vec2(-f32(u32(q.y)),0.0));
    let nx=lens_ds_add(vec2(1.0,0.0),-dx);let ny=lens_ds_add(vec2(1.0,0.0),-dy);
    var sum=weight(bitcast<f32>(values[slot*4u][c]),nx,ny);
    sum=lens_ds_add(sum,weight(bitcast<f32>(values[slot*4u+1u][c]),dx,ny));
    sum=lens_ds_add(sum,weight(bitcast<f32>(values[slot*4u+2u][c]),nx,dy));
    result=lens_round(lens_ds_add(sum,weight(bitcast<f32>(values[slot*4u+3u][c]),dx,dy)));
   }
   output[i*3u+c]=result;
  }
 }else if tile.mode==3u {
  request(i*2u,0u);request(i*2u+1u,width+index/width);
 }else if tile.mode==4u {
  // One lane owns each bounded run, carrying recurrence state between runs.
  if i!=0u{return;}
  var x=state[0];var r2=state[1];let step=lens.vignette.w;let d1=2.0*step;let d2=step*step;
  for(var n=0u;n<tile.count;n++){
   if (tile.first+n)%width==0u {
    x=coords[n].x*lens.mapping.x;let y=coords[n].y*lens.mapping.x;r2=x*x+y*y;
   }
   let r4=r2*r2;let r6=r4*r2;
   var denominator=lens_ds_add(vec2(1.0,0.0),vec2(lens.vignette.x*r2,0.0));
   denominator=lens_ds_add(denominator,vec2(lens.vignette.y*r4,0.0));
   denominator=lens_ds_add(denominator,vec2(lens.vignette.z*r6,0.0));let gain=1.0/lens_round(denominator);
   for(var c=0u;c<3u;c++){var v=bitcast<f32>(values[n][c]);if lens.flags.z!=0u{v=max(v*gain,0.0);}output[n*3u+c]=v;}
   r2+=d1*x+d2;x+=step;
  }
  state[0]=x;state[1]=r2;
 }else {
  coords[i]=vec4(bitcast<f32>(values[i*2u].x),bitcast<f32>(values[i*2u+1u].x),0.0,0.0);request(i,index);
 }
}
