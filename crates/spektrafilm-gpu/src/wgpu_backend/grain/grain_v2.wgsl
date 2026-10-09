// Grain V2: independent deterministic noise, native encoded RGB composition.
// Uniform layout: dimensions [width,height,phase_bits,mode], controls [amount,shadows,
// midtones,highlights], geometry [raw_scale,cluster_size,rotation,color], flags
// [resolution_factor,film_type,colored,clustered]. 64 bytes total.
struct Params { dimensions:vec4<u32>, controls:vec4<f32>, geometry:vec4<f32>, flags:vec4<f32> }
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read> input_rgb:array<f32>;
@group(0) @binding(2) var<storage,read_write> output_rgb:array<f32>;

@group(0) @binding(3) var<storage,read> working_rgb:array<f32>;
fn hash(x0:u32)->u32 {
 var x=x0; x+=x<<10u; x^=x>>6u; x+=x<<3u; x^=x>>11u; x+=x<<15u; return x;
}
fn random4(v:vec4<f32>)->f32 {
 let b=bitcast<vec4<u32>>(v);
 let h=hash(b.x^hash(b.y)^hash(b.z)^hash(b.w));
 return bitcast<f32>((h&0x007fffffu)|0x3f800000u)-1.;
}
fn snoise(timer:f32,v:vec4<f32>)->vec4<f32> {
 let r=random4(v); let ip=random4(vec4(r)+(vec4(timer)-vec4(r))*(timer*v));
 let q=floor(fract(0.5*ip)*7.)*ip-1.;
 return vec4(q,q,q,1.5-(abs(q)+abs(q)+abs(q)));
}
// Independent Taylor trig and mathematical 2/pi reduction, shared with v2.rs.
// Split constants avoid depending on device-specific FMA fusion below 8192.
// The integer fallback covers all finite binary32 magnitudes.
fn trig_word(words:vec3<u32>,bit:u32)->u32 {
 let i=bit/32u;let offset=bit%32u;
 let low=select(select(words.x,words.y,i==1u),words.z,i==2u);
 let high=select(select(words.y,words.z,i==1u),0u,i==2u);
 if(offset==0u){return low;}
 return (low>>offset)|(high<<(32u-offset));
}
fn trig_reduce(value:f32)->vec2<f32> {
 let x=abs(value);
 if(x<=0.7853981633974483){return vec2(x,0.);}
 if(x<8192.){
  let q=floor(fma(x,0.6366197723675814,0.5));
  let r=(x-q*1.5703125)-q*0.00048351287841796875;
  let s=r-q*2.384185791015625e-7;
  return vec2(s-q*7.549789415861596e-8,f32(u32(q)&3u));
 }
 let bits=bitcast<u32>(x);
 if((bits&0x7f800000u)==0x7f800000u){return vec2(bitcast<f32>(0x7fc00000u),0.);}
 let m=(bits&0x007fffffu)|0x00800000u;
 // Only the 50 bits surrounding the phase boundary are consumed below.
 // Retain their three words while streaming the full significand product.
 let shift=406u-(bits>>23u);
 let first=(shift-48u)/32u;
 var words=vec3<u32>(0u);var carry=0u;
 for(var i=0u;i<8u;i++){
  // Scalar selection avoids a failing WGPU 24/llvmpipe compilation path
  // for dynamically indexed constant arrays inside this reduction loop.
  var digit=0u;
  switch i {
   case 0u: {digit=0xdebbc561u;}
   case 1u: {digit=0xfe5163abu;}
   case 2u: {digit=0x3c439041u;}
   case 3u: {digit=0xdb629599u;}
   case 4u: {digit=0xf534ddc0u;}
   case 5u: {digit=0xfc2757d1u;}
   case 6u: {digit=0x4e441529u;}
   case 7u: {digit=0xa2f9836eu;}
   default: {}
  }
  let a=m&65535u;let b=m>>16u;let c=digit&65535u;let d=digit>>16u;
  let t0=a*c;let t1=b*c+(t0>>16u);let t2=a*d+(t1&65535u);
  let low=(t2<<16u)|(t0&65535u);let high=b*d+(t1>>16u)+(t2>>16u);
  let sum=low+carry;carry=high+select(0u,1u,sum<low);
  if(i==first){words.x=sum;}
  if(i==first+1u){words.y=sum;}
  if(i==first+2u){words.z=sum;}
 }
 if(first+2u==8u){words.z=carry;}
 let phase=shift-first*32u;
 let round_up=trig_word(words,phase-1u)&1u;
 let quadrant=((trig_word(words,phase)&3u)+round_up)&3u;
 let hi=f32(trig_word(words,phase-24u)&0x00ffffffu)*5.960464477539063e-8-f32(round_up);
 let lo=f32(trig_word(words,phase-48u)&0x00ffffffu)*3.552713678800501e-15;
 let r=hi*1.570796251296997;
 let tail=fma(hi,1.570796251296997,-r);
 let tail2=fma(hi,7.549789415861596e-8,tail);
 return vec2(fma(lo,1.5707963267948966,tail2)+r,f32(quadrant));
}
fn trig_polynomial(r:f32,cosine:bool)->f32 {
 let z=r*r;
 if(cosine){
  let p0=z*(1./479001600.)-1./3628800.;
  let p1=p0*z+1./40320.;let p2=p1*z-1./720.;
  let p3=p2*z+1./24.;let p4=p3*z-0.5;
  return z*p4+1.;
 }
 let p0=z*(1./6227020800.)-1./39916800.;
 let p1=p0*z+1./362880.;let p2=p1*z-1./5040.;
 let p3=p2*z+1./120.;let p4=p3*z-1./6.;
 return (r*z)*p4+r;
}
fn grain_sin(value:f32)->f32 {
 let reduced=trig_reduce(value);let q=u32(reduced.y);
 let result=trig_polynomial(reduced.x,(q&1u)!=0u);
 return select(result,-result,((q&2u)!=0u)!=((bitcast<u32>(value)&0x80000000u)!=0u));
}
fn grain_cos(value:f32)->f32 {
 let reduced=trig_reduce(value);let q=u32(reduced.y);
 let result=trig_polynomial(reduced.x,(q&1u)==0u);
 return select(result,-result,((q+1u)&2u)!=0u);
}
fn noise_mix(a:f32,b:f32,t:f32)->f32{return a+(b-a)*t;}
fn rnm(tc:vec2<f32>,timer:f32)->vec4<f32> {
 let n=grain_sin((tc.x+timer)*12.9898+(tc.y+timer)*78.233)*43758.5453;
 // Exact exponent decomposition materializes n before the next product;
 // reassociating both multipliers changes the sine-hash permutation.
 let split=frexp(n);let rounded=ldexp(split.fract,split.exp);
 return fract(vec4(rounded,rounded*1.2154,rounded*1.3453,rounded*1.3647))*2.-1.;
}
fn fade(t:vec3<f32>)->vec3<f32>{return t*t*t*(t*(t*6.-15.)+10.);}
fn pn(q:vec3<f32>,timer:f32,texel:f32)->f32 {
 // Preserve the binary32 multiply before adding the half-texel offset.
 // Contracting this expression changes the sine-hash cell permutation.
 let scaled=frexp(texel*floor(q));let pi=ldexp(scaled.fract,scaled.exp)+0.5*texel;let f=fract(q);let u=fade(f);
 var n:array<f32,8>;
 for(var x=0u;x<2u;x++){for(var y=0u;y<2u;y++){
  let perm=rnm(pi.xy+vec2(f32(x),f32(y))*texel,timer).w;
  for(var z=0u;z<2u;z++){
   let g=rnm(vec2(perm,pi.z+f32(z)*texel),timer).xyz*4.-1.;
   let d=f-vec3(f32(x),f32(y),f32(z));
   n[x*4u+y*2u+z]=g.x*d.x+g.y*d.y+g.z*d.z;
  }
 }}
 let nx=vec4(n[0],n[1],n[2],n[3])+(vec4(n[4],n[5],n[6],n[7])-vec4(n[0],n[1],n[2],n[3]))*u.x;
 let nxy=nx.xy+(nx.zw-nx.xy)*u.y;
 return nxy.x+(nxy.y-nxy.x)*u.z;
}
fn rotate(pos:vec2<f32>,angle:f32,aspect:f32)->vec2<f32>{let x=(pos.x*2.-1.)*aspect;let y=pos.y*2.-1.;let sine=grain_sin(angle);let cosine=grain_cos(angle);return vec2<f32>((x*cosine-y*sine)/aspect*0.5+0.5,(y*cosine+x*sine)*0.5+0.5);}
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
fn generator(pos:vec2<f32>,size:vec2<f32>,luma:f32,rgb:vec3<f32>,digital:bool)->vec3<f32>{
 var timer=bitcast<f32>(p.dimensions.z);let scale=clamp(p.geometry.x,0.5,1.4);
 let scaled_size=size*scale;var coords=pos;var mult=size;var aspect=size.x/size.y;
 if(!digital){coords=pos/scaled_size;mult=scaled_size/p.geometry.y/scale;aspect=scaled_size.x/scaled_size.y;}
 var angles=vec3<f32>(1.425,3.892,5.835)*p.geometry.z*scale;
 if(p.flags.w!=0. && !digital){angles=snoise(timer,vec4(coords.x,timer,coords.y,timer)).xyz*p.geometry.z;}
 if(digital){
  let sn=snoise(timer,vec4(rgb,1.));timer=timer*0.01+0.99*(sn.x*0.25+sn.y*0.25+sn.z*0.25+sn.w*0.25);
  let den=(1.+(p.geometry.x-1.)/47.)*2.4*max(size.x/1920.,size.y/1080.);mult=size/den;
 }
 var n=vec3<f32>(0.);
 for(var c=0u;c<3u;c++){
  var angle=angles[c];if(digital){angle=timer+vec3<f32>(1.425,3.892,5.835)[c];}
  let q=rotate(coords,angle,aspect);let v=vec3<f32>(q*mult,f32(c));let texel=select(1./256./p.geometry.y,p.geometry.y/256.,digital);n[c]=pn(v,timer,texel);
  if(c==0u && !digital){n[c]=noise_mix(n[c],pn(vec3<f32>(v.xy,1.),timer*0.5,texel),luma);}
 }
 let color=select(0.,clamp(p.geometry.w,0.,1.),p.flags.z!=0.);
 n.y=noise_mix(n.x,n.y,color);n.z=noise_mix(n.x,n.z,color);return n+0.5;
}
fn effective_control(v:f32)->f32 {let t=clamp(v,0.,1.);return 0.12*t*t+0.68*t+0.2;}
fn overlay(b:f32,g:f32)->f32 {return clamp(select(1.-2.*(1.-b)*(1.-g),2.*b*g,b<0.5),0.,1.);}
fn opacity(v:f32,c:f32)->f32 {let d=(v-c)*5.;return exp(-0.5*d*d);}
fn source(x:u32,y:u32,r:f32)->vec3<f32>{
 let i=(y*p.dimensions.x+x)*3u;
 return vec3(input_rgb[i],input_rgb[i+1u],input_rgb[i+2u]);
}
fn grain_source(coord:vec2<i32>,size:vec2<f32>,radius:f32)->vec3<f32> {
 let dims=p.dimensions.xy;
 if(all(size==vec2<f32>(dims))){return source(u32(coord.x),u32(coord.y),radius);}
 let pos=vec2<f32>(coord)/(size-1.)*(vec2<f32>(dims)-1.)-0.1;
 let at=vec2<i32>(floor(pos));let f=fract(pos);let hi=vec2<i32>(dims)-1;
 let a=vec2<u32>(clamp(at,vec2<i32>(0),hi));
 let b=vec2<u32>(clamp(at+vec2(1,0),vec2<i32>(0),hi));
 let c=vec2<u32>(clamp(at+vec2(0,1),vec2<i32>(0),hi));
 let d=vec2<u32>(clamp(at+vec2(1,1),vec2<i32>(0),hi));
 return mix(mix(source(a.x,a.y,radius),source(b.x,b.y,radius),f.x),mix(source(c.x,c.y,radius),source(d.x,d.y,radius),f.x),f.y);
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>){
 let idx=gid.x+gid.y*grid.x*256u;let width=p.dimensions.x;let height=p.dimensions.y;if(idx>=width*height){return;}let i=idx*3u;
 let x=idx%width;let y=idx/width;let raw_a=clamp(p.controls.x,0.,1.);let effective_a=effective_control(raw_a);let a=effective_a*select(1.,0.5,p.dimensions.w!=0u);let gsf=max(5200./f32(width),3100./f32(height));
 let rgb=vec3(working_rgb[i],working_rgb[i+1u],working_rgb[i+2u]);let luma=rgb.x*0.2125+rgb.y*0.7154+rgb.z*0.0721;
 let uv=vec2<f32>(f32(x),f32(y))*vec2(1./f32(max(width-1u,1u)),1./f32(max(height-1u,1u)));let size=floor(vec2<f32>(f32(width),f32(height))*gsf);var g=vec3<f32>(0.);
 if(p.dimensions.w!=0u){g=generator(uv,vec2(f32(width),f32(height)),luma,rgb,true);}else{
  let at=vec2<i32>(uv*(size/(1.+(p.geometry.x-1.)/47.*1.5)));
  for(var dy=-1;dy<=1;dy++){for(var dx=-1;dx<=1;dx++){
   let coord=clamp(at+vec2<i32>(dx,dy),vec2<i32>(0),vec2<i32>(size)-1);
   let grgb=grain_source(coord,size,0.);let gluma=dot(grgb,vec3(0.2125,0.7154,0.0721));
   let generated=generator(vec2<f32>(coord),size,gluma,grgb,false);
   g+=vec3(half_round(generated.x),half_round(generated.y),half_round(generated.z));
  }}g*=1./9.;
 }
 let ws=effective_control(p.controls.y)*a*opacity(luma,0.)*2.;let wm=effective_control(p.controls.z)*a*opacity(luma,0.5);let wh=effective_control(p.controls.w)*a*opacity(luma,1.)*2.;
 for(var c=0u;c<3u;c++){let b=rgb[c];let s=mix(b,overlay(pow(max(b,0.),0.8),g[c]),ws);let m=mix(s,overlay(s,g[c]),wm);let h=mix(m,overlay(m-0.2,g[c]),wh);output_rgb[i+c]=half_round(clamp(h*0.5+b*0.5,0.,1.));}
}
