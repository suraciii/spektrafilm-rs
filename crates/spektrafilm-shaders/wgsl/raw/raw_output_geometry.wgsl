// Numerical stage order is unchanged: Fuji ushort interpolation, camera
// ushort conversion, native integer-frac stretch, orientation/normalization.
struct Tile { dimensions:vec4<u32>, tile:vec4<u32>, controls:vec4<u32>, step:vec4<f32> };
@group(0) @binding(0) var<uniform> p:Tile;
@group(0) @binding(1) var<storage,read_write> requests:array<u32>;
@group(0) @binding(2) var<storage,read> values:array<vec4<u32>>;
@group(0) @binding(3) var<storage,read_write> destination:array<u32>;
@group(0) @binding(4) var<storage,read_write> bounds:array<atomic<u32>>;
fn request(slot:u32,index:u32) {
 requests[slot]=index; atomicMin(&bounds[0],index); atomicMax(&bounds[1],index);
}
fn fuji_coordinate(index:u32)->vec2<f32> {
 let x=index%p.dimensions.z; let y=index/p.dimensions.z;
 let rr=ds_add(vec2<f32>(f32(p.controls.x),0.0),ds_mul_int(p.step.xy,f32(i32(y)-i32(x))));
 let cc=ds_mul_int(p.step.xy,f32(y+x));
 return vec2<f32>(rr.x+rr.y,cc.x+cc.y);
}
fn oriented(index:u32)->vec2<u32> {
 var row=index/p.dimensions.z; var col=index%p.dimensions.z;
 if (p.controls.x&4u)!=0u { let swap=row;row=col;col=swap; }
 if (p.controls.x&2u)!=0u { row=p.dimensions.y-1u-row; }
 if (p.controls.x&1u)!=0u { col=p.dimensions.x-1u-col; }
 return vec2<u32>(col,row);
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
 let i=id.x+id.y*grid.x*256u; if i>=p.tile.y {return;}
 let index=p.tile.x+i; let mode=p.tile.z;
 if mode==0u {
  let rc=fuji_coordinate(index);let r=u32(max(rc.x,0.0));let c=u32(max(rc.y,0.0));
  let valid=rc.x>=0.0&&rc.y>=0.0&&r<=p.dimensions.y-2u&&c<=p.dimensions.x-2u;
  for(var k=0u;k<4u;k++){requests[i*4u+k]=0xffffffffu;}
  if valid { let base=r*p.dimensions.x+c;request(i*4u,base);request(i*4u+1u,base+1u);request(i*4u+2u,base+p.dimensions.x);request(i*4u+3u,base+p.dimensions.x+1u); }
 } else if mode==1u {
  let rc=fuji_coordinate(index); let r=u32(max(rc.x,0.0));let c=u32(max(rc.y,0.0));
  if rc.x<0.0||rc.y<0.0||r>p.dimensions.y-2u||c>p.dimensions.x-2u {return;}
  let fr=rc.x-f32(r);let fc=rc.y-f32(c);
  for(var channel=0u;channel<p.controls.y;channel++){
   let top=f32(values[i*4u][channel])*(1.0-fc)+f32(values[i*4u+1u][channel])*fc;
   let bottom=f32(values[i*4u+2u][channel])*(1.0-fc)+f32(values[i*4u+3u][channel])*fc;
   destination[i*4u+channel]=u32(top*(1.0-fr)+bottom*fr);
  }
 } else if mode==2u {
  let xy=oriented(index);request(i,select(xy.x,xy.y,p.controls.y==1u));
 } else if mode==3u {
  var xy=oriented(index);
  if p.controls.y==1u {xy.y=values[i].x;}else {xy.x=values[i].x;}
  request(i,xy.y*p.controls.z+xy.x);
 } else if mode==4u {
  let xy=oriented(index);request(i,xy.y*p.dimensions.x+xy.x);
 } else {
  for(var c=0u;c<3u;c++){destination[i*3u+c]=bitcast<u32>(f32(values[i][c])/65535.0);}
 }
}
