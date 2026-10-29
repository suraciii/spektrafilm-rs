// Four independent output sums per invocation. Each component retains the
// original ascending tap order, with no rolling updates or reassociation.
struct Params {
  count:u32, lines:u32, stride:u32, channel:u32,
  radius:u32, mode:u32, initialize:u32, axis_major:u32,
  a:f32, b1:f32, b2:f32, b3:f32,
}
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read> source:array<f32>;
@group(0) @binding(2) var<storage,read_write> destination:array<f32>;
@group(0) @binding(3) var<storage,read> kernel:array<f32>;
@group(0) @binding(4) var<storage,read_write> state:array<f32>;
fn load4(i:u32,offset:u32,last:u32)->vec4<f32>{
 return vec4(source[i],source[i+min(1u,last)*offset],source[i+min(2u,last)*offset],source[i+last*offset]);
}
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>){
 let idx=gid.x+gid.y*grid.x*64u;
 let tiles=(p.count+3u)/4u;
 if(idx>=tiles*p.lines){return;}
 var line=idx/tiles;
 var j=(idx%tiles)*4u;
 if(p.axis_major!=0u){line=idx%p.lines;j=(idx/p.lines)*4u;}
 let span=p.count+2u*p.radius;
 var start=line*span+j;
 var step=1u;
 if(p.axis_major!=0u){start=j*p.lines+line;step=p.lines;}
 var sum=vec4<f32>(0.0);
 let offset=step*p.stride;
 let last=min(3u,p.count-1u-j);
 let taps=2u*p.radius+1u;
 var k=0u;
 for(;k+3u<taps;k+=4u){
   let i=(start+k*step)*p.stride+p.channel;
   sum=sum+load4(i,offset,last);
   sum=sum+load4(i+offset,offset,last);
   sum=sum+load4(i+2u*offset,offset,last);
   sum=sum+load4(i+3u*offset,offset,last);
 }
 for(;k<taps;k++){
   sum=sum+load4((start+k*step)*p.stride+p.channel,offset,last);
 }
 let result=sum*kernel[0];
 for(var c=0u;c<min(4u,p.count-j);c++){
   let output=select(line*p.count+j+c,(j+c)*p.lines+line,p.axis_major!=0u);
   destination[output*p.stride+p.channel]=result[c];
 }
}
