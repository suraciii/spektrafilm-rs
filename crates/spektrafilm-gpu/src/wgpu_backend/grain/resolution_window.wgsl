struct Params { dimensions:vec4<u32>, tap:vec4<u32>, page:vec4<u32> }
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read> source:array<f32>;
@group(0) @binding(2) var<storage,read> weights:array<vec4<f32>>;
@group(0) @binding(3) var<storage,read_write> samples:array<f32>;
@group(0) @binding(4) var<storage,read_write> output:array<f32>;
fn half_round(v:f32)->f32 {
 let bits=bitcast<u32>(v);let sign=bits&0x80000000u;let exponent=(bits>>23u)&255u;let mantissa=bits&0x7fffffu;
 if(exponent==255u){return v;}if(exponent>=143u){return bitcast<f32>(sign|0x7f800000u);}
 if(exponent>=113u){let rounded=(bits&0x7fffffffu)+4095u+((mantissa>>13u)&1u);if(rounded>=0x47800000u){return bitcast<f32>(sign|0x7f800000u);}return bitcast<f32>(sign|(rounded&0x7fffe000u));}
 if(exponent<102u){return bitcast<f32>(sign);}let shift=126u-exponent;let significand=mantissa|0x800000u;let truncated=significand>>shift;let remainder=significand&((1u<<shift)-1u);let midpoint=1u<<(shift-1u);let rounded=truncated+select(0u,1u,remainder>midpoint || (remainder==midpoint && (truncated&1u)!=0u));return bitcast<f32>(sign|bitcast<u32>(f32(rounded)*0.000000059604644775390625));
}
fn sample(t:u32,corner:u32)->vec3<f32>{let i=(t*8u+corner)*3u;return vec3(samples[i],samples[i+1u],samples[i+2u]);}
fn interpolate(t:u32,side:u32,f:vec2<f32>)->vec3<f32>{let a=sample(t,side*4u);let b=sample(t,side*4u+1u);let c=sample(t,side*4u+2u);let d=sample(t,side*4u+3u);let ab=a+(b-a)*f.x;let cd=c+(d-c)*f.x;return ab+(cd-ab)*f.y;}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>) {
 let t=gid.x;let xy=vec2<i32>(i32(p.tap.x%p.dimensions.x),i32(p.tap.x/p.dimensions.x));let direction=select(vec2<i32>(1,0),vec2<i32>(0,1),p.dimensions.z==1u);let high=vec2<i32>(p.dimensions.xy)-1;
 if(p.page.z==0u){
  if(t>=p.tap.z){return;}
  for(var side=0u;side<2u;side++){
   var pos=vec2<f32>(xy);
   if(p.dimensions.w!=0u){pos=vec2<f32>(clamp(xy+direction*(i32(p.tap.y+t)-select(0,i32(p.tap.w),side==1u)),vec2<i32>(0),high));}
   else {let delta=vec2<f32>(direction)*weights[t].y;pos=vec2<f32>(xy)+select(delta,-delta,side==1u);pos=select(pos,vec2<f32>(xy),(pos<vec2(0.))|(pos>vec2<f32>(high)));pos-=0.1;}
   let base=vec2<i32>(floor(pos));
   for(var corner=0u;corner<4u;corner++){
    let at=vec2<u32>(clamp(base+vec2<i32>(i32(corner&1u),i32(corner>>1u)),vec2<i32>(0),high));let pixel=at.y*p.dimensions.x+at.x;
    if(pixel>=p.page.x && pixel-p.page.x<p.page.y){for(var c=0u;c<3u;c++){samples[(t*8u+side*4u+corner)*3u+c]=source[(pixel-p.page.x)*3u+c];}}
   }
  }
  return;
 }
 if(t!=0u){return;}var result=vec3<f32>(0.);if(p.tap.y!=0u){result=vec3(output[0],output[1],output[2]);}
 for(var k=0u;k<p.tap.z;k++){
  if(p.dimensions.w!=0u){result+=sample(k,0u)*weights[k].x+sample(k,4u)*weights[k].y;}
  else {let delta=vec2<f32>(direction)*weights[k].y;var a=vec2<f32>(xy)+delta;var b=vec2<f32>(xy)-delta;a=select(a,vec2<f32>(xy),(a<vec2(0.))|(a>vec2<f32>(high)));b=select(b,vec2<f32>(xy),(b<vec2(0.))|(b>vec2<f32>(high)));result+=weights[k].x*(interpolate(k,0u,fract(a-0.1))+interpolate(k,1u,fract(b-0.1)));}
 }
 for(var c=0u;c<3u;c++){var v=result[c];if(p.page.w!=0u && (p.dimensions.w==0u || p.dimensions.z==1u)){v=half_round(v);}output[c]=v;}
}
