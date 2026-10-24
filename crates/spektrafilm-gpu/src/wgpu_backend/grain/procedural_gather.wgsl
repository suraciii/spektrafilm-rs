struct Params { dimensions:vec4<u32>, page:vec4<u32>, controls:vec4<f32> }
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read> source:array<f32>;
@group(0) @binding(2) var<storage,read_write> samples:array<f32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
 let local=gid.x+gid.y*grid.x*256u;if(local>=p.dimensions.w){return;}
 let width=p.dimensions.x;let height=p.dimensions.y;let pixel=p.dimensions.z+local;
 let uv=vec2<f32>(f32(pixel%width),f32(pixel/width))*vec2(1./f32(max(width-1u,1u)),1./f32(max(height-1u,1u)));
 let gsf=max(5200./f32(width),3100./f32(height));let size=floor(vec2<f32>(f32(width),f32(height))*gsf);
 let center=vec2<i32>(uv*(size/(1.+(p.controls.x-1.)/47.*1.5)));
 for(var dy=-1;dy<=1;dy++){for(var dx=-1;dx<=1;dx++){
  let slot=u32((dy+1)*3+dx+1);let coord=clamp(center+vec2<i32>(dx,dy),vec2<i32>(0),vec2<i32>(size)-1);
  var pos=vec2<f32>(coord)/(size-1.)*(vec2<f32>(p.dimensions.xy)-1.)-0.1;
  if(all(size==vec2<f32>(p.dimensions.xy))){pos=vec2<f32>(coord);}
  let base=vec2<i32>(floor(pos));
  for(var corner=0u;corner<4u;corner++){
   let xy=vec2<u32>(clamp(base+vec2<i32>(i32(corner&1u),i32(corner>>1u)),vec2<i32>(0),vec2<i32>(p.dimensions.xy)-1));
   let index=xy.y*width+xy.x;
   if(index>=p.page.x && index-p.page.x<p.page.y){
    for(var c=0u;c<3u;c++){samples[((local*9u+slot)*4u+corner)*3u+c]=source[(index-p.page.x)*3u+c];}
   }
  }
 }}
}
