// Grain V2 working-domain resolution passes.
// dimensions: width, height, axis (0/1; 2 = native encoded input), optical.
struct Params { dimensions:vec4<u32>, taps:vec4<u32> }
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read> input:array<f32>;
@group(0) @binding(2) var<storage,read> weights:array<vec2<f32>>;
@group(0) @binding(3) var<storage,read_write> output:array<f32>;
fn half_round(v:f32)->f32 {
 let bits=bitcast<u32>(v);let sign=bits&0x80000000u;let exponent=(bits>>23u)&255u;let mantissa=bits&0x7fffffu;
 if(exponent==255u){return v;}
 if(exponent>=143u){return bitcast<f32>(sign|0x7f800000u);}
 if(exponent>=113u){
  let rounded=(bits&0x7fffffffu)+4095u+((mantissa>>13u)&1u);
  if(rounded>=0x47800000u){return bitcast<f32>(sign|0x7f800000u);}
  return bitcast<f32>(sign|(rounded&0x7fffe000u));
 }
 if(exponent<102u){return bitcast<f32>(sign);}
 let shift=126u-exponent;let significand=mantissa|0x800000u;
 let truncated=significand>>shift;let remainder=significand&((1u<<shift)-1u);let midpoint=1u<<(shift-1u);
 let rounded=truncated+select(0u,1u,remainder>midpoint || (remainder==midpoint && (truncated&1u)!=0u));
 return bitcast<f32>(sign|bitcast<u32>(f32(rounded)*0.000000059604644775390625));
}
fn at(pos:vec2<i32>)->vec3<f32> {
 let xy=vec2<u32>(clamp(pos,vec2<i32>(0),vec2<i32>(p.dimensions.xy)-1));
 let i=(xy.y*p.dimensions.x+xy.x)*3u;return vec3(input[i],input[i+1u],input[i+2u]);
}
fn bilinear(pos:vec2<f32>)->vec3<f32> {
 let q=pos-0.1;let base=vec2<i32>(floor(q));let t=fract(q);
 let a=at(base);let b=at(base+vec2(1,0));let c=at(base+vec2(0,1));let d=at(base+vec2(1,1));
 let ab=a+(b-a)*t.x;let cd=c+(d-c)*t.x;return ab+(cd-ab)*t.y;
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
 let idx=gid.x+gid.y*grid.x*256u;let width=p.dimensions.x;let height=p.dimensions.y;
 if(idx>=width*height){return;}let i=idx*3u;let xy=vec2<i32>(i32(idx%width),i32(idx/width));
 var result=vec3<f32>(0.);
 if(p.dimensions.z==2u) {
  for(var c=0u;c<3u;c++){output[i+c]=half_round(input[i+c]);}return;
 }
 let direction=select(vec2<i32>(1,0),vec2<i32>(0,1),p.dimensions.z==1u);
 if(p.dimensions.w!=0u) {
  let half_size=p.taps.x/2u;
  for(var t=0u;t<half_size;t++) {
   result+=at(xy+direction*i32(t))*weights[t+half_size].x+at(xy+direction*(i32(t)-i32(half_size)))*weights[t].x;
  }
 } else {
  let center=vec2<f32>(xy);let high=vec2<f32>(p.dimensions.xy)-1.;
  for(var t=0u;t<p.taps.x;t++) {
   let delta=vec2<f32>(direction)*weights[t].y;
   var a=center+delta;var b=center-delta;
   a=select(a,center,(a<vec2(0.))|(a>high));b=select(b,center,(b<vec2(0.))|(b>high));
   result+=weights[t].x*(bilinear(a)+bilinear(b));
  }
 }
 for(var c=0u;c<3u;c++){var v=result[c];if(p.dimensions.w==0u || p.dimensions.z==1u){v=half_round(v);}output[i+c]=v;}
}
