// Procedural grain: independent deterministic noise, native encoded RGB composition.
// Uniform layout: dimensions [width,height,phase_bits,mode], controls [amount,shadows,
// midtones,highlights], geometry [raw_scale,cluster_size,rotation,color], flags
// [resolution_factor,resolution_type,colored,clustered], window [first,count,row_base,rows],
// canvas [cells_wide,cells_high,chunk_cell_base,row_base], generator_geometry
// [analogue_mult_x,analogue_mult_y,analogue_aspect,analogue_texel].
struct Params { dimensions:vec4<u32>, controls:vec4<f32>, geometry:vec4<f32>, flags:vec4<f32>, window:vec4<u32>, canvas:vec4<u32>, generator_geometry:vec4<f32> }
@group(0) @binding(0) var<uniform> p:Params;
// A window of the unblurred encoded source rows, which the canvas pass samples.
@group(0) @binding(1) var<storage,read> source_rgb:array<f32>;
// Two packed half pairs per canvas cell: (r,g) then (b,0).
@group(0) @binding(2) var<storage,read_write> canvas_cells:array<u32>;
// The Film Resolution result the composition reads.
@group(0) @binding(3) var<storage,read> working_rgb:array<f32>;
// The scan's exact grain mean, written by the reduction pass.
@group(0) @binding(4) var<storage,read> mean_rgb:array<f32>;
@group(0) @binding(5) var<storage,read_write> output_rgb:array<f32>;
// The local WGPU HAL uses this named function to select strict Metal math.
fn spektrafilm_strict_float(v:f32)->f32{return v;}
// Vulkan selects this module only; Spektra/MT19937 retain their own math contract.
fn spektrafilm_procedural_no_contraction(v:f32)->f32{return v;}
fn hash(x0:u32)->u32 {
 var x=x0; x+=x<<10u; x^=x>>6u; x+=x<<3u; x^=x>>11u; x+=x<<15u; return x;
}
fn random4(v:vec4<f32>)->f32 {
 let b=bitcast<vec4<u32>>(v);
 let h=hash(b.x^hash(b.y)^hash(b.z)^hash(b.w));
 return bitcast<f32>((h&0x007fffffu)|0x3f800000u)-1.;
}
fn grad4(j:f32,ip:vec4<f32>)->vec4<f32>{
 let px=floor(fract(j*ip.x)*7.)*ip.z-1.;
 let py=floor(fract(j*ip.y)*7.)*ip.z-1.;
 let pz=floor(fract(j*ip.z)*7.)*ip.z-1.;
 return vec4(px,py,pz,1.5-(abs(px)+abs(py)+abs(pz)));
}
fn snoise(timer:f32,v:vec4<f32>)->vec4<f32> {
 // The integer hash consumes float bits. Materialize both products so Metal
 // cannot reassociate or fuse the CPU's separately rounded interpolation.
 let r=random4(v);let t=frexp(timer*v);
 let weighted=frexp((vec4(timer)-vec4(r))*ldexp(t.fract,t.exp));
 let ip=random4(vec4(r)+ldexp(weighted.fract,weighted.exp));
 let q=floor(fract(0.5*ip)*7.)*ip-1.;
 return vec4(q,q,q,1.5-(abs(q)+abs(q)+abs(q)));
}
// Independent Taylor trig and mathematical 2/pi reduction, shared with procedural.rs.
// Split constants avoid depending on device-specific FMA fusion below 8192.
// The integer fallback covers all finite binary32 magnitudes.
// The sine hash amplifies a one-ULP trig change into a different permutation.
// Keep the CPU's binary32 product boundaries through Metal contraction.
fn trig_product(a:f32,b:f32)->f32 {
 let split=frexp(spektrafilm_procedural_no_contraction(spektrafilm_strict_float(a*b)));return ldexp(split.fract,split.exp);
}
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
 let r=trig_product(hi,1.570796251296997);
 let tail=fma(hi,1.570796251296997,-r);
 let tail2=fma(hi,7.549789415861596e-8,tail);
 return vec2(fma(lo,1.5707963267948966,tail2)+r,f32(quadrant));
}
fn trig_polynomial(r:f32,cosine:bool)->f32 {
 let z=trig_product(r,r);
 if(cosine){
  let p0=trig_product(z,1./479001600.)-1./3628800.;
  let p1=trig_product(p0,z)+1./40320.;let p2=trig_product(p1,z)-1./720.;
  let p3=trig_product(p2,z)+1./24.;let p4=trig_product(p3,z)-0.5;
  return trig_product(z,p4)+1.;
 }
 let p0=trig_product(z,1./6227020800.)-1./39916800.;
 let p1=trig_product(p0,z)+1./362880.;let p2=trig_product(p1,z)-1./5040.;
 let p3=trig_product(p2,z)+1./120.;let p4=trig_product(p3,z)-1./6.;
 return trig_product(trig_product(r,z),p4)+r;
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
 // A contracted dot product changes the sine-hash permutation as well.
 let phase_x=frexp((tc.x+timer)*12.9898);
 let phase_y=frexp((tc.y+timer)*78.233);
 let phase=ldexp(phase_x.fract,phase_x.exp)+ldexp(phase_y.fract,phase_y.exp);
 let n=grain_sin(phase)*43758.5453;
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
fn rotate(pos:vec2<f32>,angle:f32,aspect:f32)->vec2<f32>{let x=(pos.x*2.-1.)*aspect;let y=pos.y*2.-1.;let sine=grain_sin(angle);let cosine=grain_cos(angle);return vec2<f32>(rounded_normal_divide(x*cosine-y*sine,aspect)*0.5+0.5,(y*cosine+x*sine)*0.5+0.5);}
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
// Correctly rounded binary32 division for finite normal operands and a
// finite normal result (zero numerator is also supported). At each step
// rem < n < 2^24; doubled rem and q fit in u32. Callers establish the domain.
fn divide_normal_f32(a:f32,b:f32)->f32{
 let aa=bitcast<u32>(a);let bb=bitcast<u32>(b);let sign=(aa^bb)&0x80000000u;
 if((aa&0x7fffffffu)==0u){return bitcast<f32>(sign);}
 var m=(aa&0x7fffffu)|0x800000u;let n=(bb&0x7fffffu)|0x800000u;
 var e=i32((aa>>23u)&255u)-i32((bb>>23u)&255u)+127;
 if(m<n){m<<=1u;e-=1;}
 var rem=m-n;var q=1u;
 for(var i=0u;i<23u;i++){rem<<=1u;q<<=1u;if(rem>=n){rem-=n;q|=1u;}}
 let twice=rem<<1u;
 if(twice>n || (twice==n && (q&1u)!=0u)){q+=1u;}
 if(q==0x1000000u){q>>=1u;e+=1;}
 return bitcast<f32>(sign|(u32(e)<<23u)|(q&0x7fffffu));
}
// Rotation can cancel to zero or approach the subnormal/overflow boundaries.
// Use the integer path only when exponent bounds guarantee a normal result;
// otherwise retain the existing device operation. This is not an IEEE library.
fn rounded_normal_divide(a:f32,b:f32)->f32 {
 let ea=(bitcast<u32>(a)>>23u)&255u;let eb=(bitcast<u32>(b)>>23u)&255u;
 let e=i32(ea)-i32(eb)+127;
 if(ea>0u && ea<255u && eb>0u && eb<255u && e>1 && e<254){return divide_normal_f32(a,b);}
 return a/b;
}

fn generator(pos:vec2<f32>,size:vec2<f32>,luma:f32,rgb:vec3<f32>,digital:bool)->vec3<f32>{
 var timer=bitcast<f32>(p.dimensions.z);let scale=clamp(p.geometry.x,0.5,1.4);
 let scaled_size=size*scale;var coords=pos;var mult=size;var aspect=size.x/size.y;
 // Integer cell coordinates are zero or |pos| >= 1. Host dimensions bound
 // size below 2^43; finite scale is clamped to [.5,1.4]. These quotients are
 // normal and finite, so the unchecked private divider is valid here.
 if(!digital){coords=vec2(divide_normal_f32(pos.x,scaled_size.x),divide_normal_f32(pos.y,scaled_size.y));mult=p.generator_geometry.xy;aspect=p.generator_geometry.z;}
 var angles=vec3<f32>(1.425,3.892,5.835)*p.geometry.z*scale;
 if(p.flags.w!=0. && !digital){angles=snoise(timer,vec4(coords.x,timer,coords.y,timer)).xyz*p.geometry.z;}
 if(digital){
  let sn=snoise(timer,vec4(rgb,1.));timer=timer*0.01+0.99*(sn.x*0.25+sn.y*0.25+sn.z*0.25+sn.w*0.25);
  let den=(1.+(p.geometry.x-1.)/47.)*2.4*max(size.x/1920.,size.y/1080.);mult=size/den;
 }
 var n=vec3<f32>(0.);
 for(var c=0u;c<3u;c++){
  var angle=angles[c];if(digital){angle=timer+vec3<f32>(1.425,3.892,5.835)[c];}
  let q=rotate(coords,angle,aspect);let v=vec3<f32>(q*mult,f32(c));let texel=select(p.generator_geometry.w,p.geometry.y/256.,digital);n[c]=pn(v,timer,texel);
  if(c==0u && !digital){n[c]=noise_mix(n[c],pn(vec3<f32>(v.xy,1.),timer*0.5,texel),luma);}
 }
 let color=select(0.,clamp(p.geometry.w,0.,1.),p.flags.z!=0.);
 n.y=noise_mix(n.x,n.y,color);n.z=noise_mix(n.x,n.z,color);return n+0.5;
}
fn effective_control(v:f32)->f32 {let t=clamp(v,0.,1.);return 0.12*t*t+0.68*t+0.2;}
fn overlay(b:f32,g:f32)->f32 {return clamp(select(1.-2.*(1.-b)*(1.-g),2.*b*g,b<0.5),0.,1.);}
fn opacity(v:f32,c:f32)->f32 {let d=(v-c)*5.;return exp(-0.5*d*d);}
// Bilinear read of the source window at one canvas cell. The window holds whole
// rows from `p.window.z`, so rows index with the full image width.
fn source_corner(base:vec2<i32>,o:vec2<i32>)->vec3<f32>{
 let xy=vec2<u32>(clamp(base+o,vec2<i32>(0),vec2<i32>(p.dimensions.xy)-1));
 let i=((xy.y-p.window.z)*p.dimensions.x+xy.x)*3u;
 return vec3(source_rgb[i],source_rgb[i+1u],source_rgb[i+2u]);
}
fn source_at(coord:vec2<i32>,size:vec2<f32>)->vec3<f32>{
 let dims=vec2<f32>(p.dimensions.xy);
 let pos=select(vec2<f32>(coord)/(size-1.)*(dims-1.)-0.1,vec2<f32>(coord),all(size==dims));
 let base=vec2<i32>(floor(pos));let f=fract(pos);
 return mix(mix(source_corner(base,vec2<i32>(0,0)),source_corner(base,vec2<i32>(1,0)),f.x),
            mix(source_corner(base,vec2<i32>(0,1)),source_corner(base,vec2<i32>(1,1)),f.x),f.y);
}
// `canvas.w` is the first row this buffer stores: the canvas is split into row
// bands so it can exceed one binding, and a band keeps one row of overlap on
// each side for the 3x3 average.
fn cell_at(coord:vec2<i32>)->vec3<f32>{
 let c=clamp(coord,vec2<i32>(0),vec2<i32>(p.canvas.xy)-1i);
 let i=((u32(c.y)-p.canvas.w)*p.canvas.x+u32(c.x))*2u;
 return vec3(unpack2x16float(canvas_cells[i]),unpack2x16float(canvas_cells[i+1u]).x);
}
// The canvas pass: one invocation per cell of the current row band.
@compute @workgroup_size(256)
fn canvas_main(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>){
 let local=gid.x+gid.y*grid.x*256u;
 if(local>=p.window.y){return;}
 let cell=p.window.x+local;
 let width=p.dimensions.x;let height=p.dimensions.y;
 let gsf=max(5200./f32(width),3100./f32(height));let size=floor(vec2<f32>(f32(width),f32(height))*gsf);
 let coord=vec2<i32>(i32(cell%p.canvas.x),i32(cell/p.canvas.x));
 let rgb=source_at(coord,size);let luma=dot(rgb,vec3(0.2125,0.7154,0.0721));
 let value=generator(vec2<f32>(coord),size,luma,rgb,false);
 // `canvas.z` is this chunk's first cell inside the band buffer.
 let write=local+p.canvas.z;
 canvas_cells[write*2u]=pack2x16float(vec2(half_round(value.x),half_round(value.y)));
 canvas_cells[write*2u+1u]=pack2x16float(vec2(half_round(value.z),0.));
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>){
 let local=gid.x+gid.y*grid.x*256u;let width=p.dimensions.x;let height=p.dimensions.y;if(local>=p.window.y){return;}let idx=p.window.x+local;let i=local*3u;
 let x=idx%width;let y=idx/width;let raw_a=clamp(p.controls.x,0.,1.);let effective_a=effective_control(raw_a);let a=effective_a*select(1.,0.5,p.dimensions.w!=0u);let gsf=max(5200./f32(width),3100./f32(height));
 let rgb=vec3(working_rgb[i],working_rgb[i+1u],working_rgb[i+2u]);let luma=rgb.x*0.2125+rgb.y*0.7154+rgb.z*0.0721;
 let uv=vec2<f32>(f32(x),f32(y))*vec2(1./f32(max(width-1u,1u)),1./f32(max(height-1u,1u)));let size=floor(vec2<f32>(f32(width),f32(height))*gsf);var g=vec3<f32>(0.);
 if(p.dimensions.w!=0u){g=generator(uv,vec2(f32(width),f32(height)),luma,rgb,true);}else{
  let at=vec2<i32>(uv*(size/(1.+(p.geometry.x-1.)/47.*1.5)));
  for(var dy=-1;dy<=1;dy++){for(var dx=-1;dx<=1;dx++){g+=cell_at(at+vec2<i32>(dx,dy));}}g*=1./9.;
 }
 let ws=effective_control(p.controls.y)*a*opacity(luma,0.)*2.;let wm=effective_control(p.controls.z)*a*opacity(luma,0.5);let wh=effective_control(p.controls.w)*a*opacity(luma,1.)*2.;
 for(var c=0u;c<3u;c++){
  let b=rgb[c];
  let s=mix(b,overlay(pow(max(b,0.),0.8),g[c]),ws);let m=mix(s,overlay(s,g[c]),wm);let h=mix(m,overlay(m-0.2,g[c]),wh);
  let stored=half_round(clamp(h*0.5+b*0.5,0.,1.));
  // Remove the displacement the zoned bases produce at the grain mean.
  // Digital grain has no canvas, so its mean is the neutral half.
  let mu=select(mean_rgb[c],0.5,p.dimensions.w!=0u);
  let ds=mix(b,overlay(pow(max(b,0.),0.8),mu),ws);let dm=mix(ds,overlay(ds,mu),wm);let dh=mix(dm,overlay(dm-0.2,mu),wh);
  output_rgb[i+c]=half_round(clamp(stored-(dh*0.5+b*0.5-b),0.,1.));
 }
}
