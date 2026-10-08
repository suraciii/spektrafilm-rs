// Grain V2: independent gradient hash and documented RGB composition semantics.
// Uniform layout: dimensions [width,height,seed,mode], controls [amount,shadows,
// midtones,highlights], geometry [raw_scale,cluster_size,rotation,color], flags
// [resolution_factor,resolution_type,colored,clustered]. 64 bytes total.
struct Params { dimensions:vec4<u32>, controls:vec4<f32>, geometry:vec4<f32>, flags:vec4<f32> }
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read> input_rgb:array<f32>;
@group(0) @binding(2) var<storage,read_write> output_rgb:array<f32>;
fn hash(x0:u32)->u32 {var x=x0^(x0>>16u);x*=0x7feb352du;x^=x>>15u;x*=0x846ca68bu;return x^(x>>16u);}
fn unit(x:u32)->f32{return f32(x>>8u)/16777216.;}
fn fade(t:vec3<f32>)->vec3<f32>{return t*t*t*(t*(t*6.-15.)+10.);}
fn pn(q:vec3<f32>)->f32 {
 let cell=vec3<i32>(floor(q));let f=fract(q);let u=fade(f);var sum=0.;
 for(var z=0;z<2;z++){for(var y=0;y<2;y++){for(var x=0;x<2;x++){
 let h=hash(bitcast<u32>(cell.x+x)^(bitcast<u32>(cell.y+y)*0x9e3779b9u)^(bitcast<u32>(cell.z+z)*0x85ebca6bu)^p.dimensions.z);
 let g=vec3<f32>(unit(h),unit(hash(h)),unit(hash(h^0x51ed270bu)))*2.-1.;
 let d=dot(g,f-vec3<f32>(f32(x),f32(y),f32(z)));
 sum+=d*select(1.-u.x,u.x,x==1)*select(1.-u.y,u.y,y==1)*select(1.-u.z,u.z,z==1);
 }}}return sum;
}
fn rotate(pos:vec2<f32>,angle:f32,aspect:f32)->vec2<f32>{let x=(pos.x-0.5)*aspect;let y=pos.y-0.5;return vec2<f32>((x*cos(angle)-y*sin(angle))/aspect+0.5,x*sin(angle)+y*cos(angle)+0.5);}
fn rescale()->f32{return 1.+(p.geometry.x-1.)/47.*1.5;}
fn generator(pos:vec2<f32>,size:vec2<f32>,luma:f32,digital:bool)->vec3<f32>{
 var timer=f32(p.dimensions.z&65535u)/65536.;let scale=clamp(p.geometry.x,0.5,1.4);
 var angles=vec3<f32>(1.425,3.892,5.835)*scale*p.geometry.z;
 if(p.flags.w!=0.){for(var c=0u;c<3u;c++){angles[c]=pn(vec3<f32>(pos*8.,timer+f32(c)))*p.geometry.z;}}
 if(digital){timer=timer*0.01+pn(vec3<f32>(luma,pos))*0.99;}
 var n=vec3<f32>(0.);
 for(var c=0u;c<3u;c++){
 var angle=angles[c];if(digital){angle=timer+vec3<f32>(1.425,3.892,5.835)[c];}
 let q=rotate(pos,angle,size.x/size.y);var den=max(p.geometry.y,0.01)*scale;if(digital){den=rescale();}
 let v=vec3<f32>(q*size/den,timer+f32(c));n[c]=pn(v);
 if(c==0u && !digital){n[c]=mix(n[c],pn(vec3<f32>(v.xy,timer*0.5+1.)),luma);}
 }
 let color=select(0.,clamp(p.geometry.w,0.,1.),p.flags.z!=0.);
 n.y=mix(n.x,n.y,color);n.z=mix(n.x,n.z,color);return n+0.5;
}
fn effective_control(v:f32)->f32 {let t=clamp(v,0.,1.);return select(0.,0.12*t*t+0.68*t+0.2,t>0.);}
fn overlay(b:f32,g:f32)->f32 {return clamp(select(1.-2.*(1.-b)*(1.-g),2.*b*g,b<0.5),0.,1.);}
fn opacity(v:f32,c:f32)->f32 {let d=(v-c)*5.;return exp(-0.5*d*d);}
fn weight(d:i32,r:f32,optical:bool)->f32 {if(optical){let v=f32(d)/max(r,0.001);return exp(-0.5*v*v);}return clamp(r+1.-f32(abs(d)),0.,1.);}
fn source(x:u32,y:u32,r:f32)->vec3<f32>{
 if(r<=0.){let i=(y*p.dimensions.x+x)*3u;return vec3<f32>(input_rgb[i],input_rgb[i+1u],input_rgb[i+2u]);}
 let optical=p.flags.y!=1.;var reach=i32(ceil(r));if(optical){reach=i32(ceil(r*3.));}
 var sum=vec3<f32>(0.);var total=0.;
 for(var dy=-reach;dy<=reach;dy++){for(var dx=-reach;dx<=reach;dx++){
 let xx=u32(clamp(i32(x)+dx,0,i32(p.dimensions.x)-1));let yy=u32(clamp(i32(y)+dy,0,i32(p.dimensions.y)-1));let i=(yy*p.dimensions.x+xx)*3u;
 let w=weight(dx,r,optical)*weight(dy,r,optical);sum+=vec3<f32>(input_rgb[i],input_rgb[i+1u],input_rgb[i+2u])*w;total+=w;
 }}return sum/total;
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>){
 let idx=gid.x+gid.y*grid.x*256u;let width=p.dimensions.x;let height=p.dimensions.y;if(idx>=width*height){return;}let i=idx*3u;if(p.controls.x==0.){for(var c=0u;c<3u;c++){output_rgb[i+c]=input_rgb[i+c];}return;}
 let x=idx%width;let y=idx/width;let raw_a=clamp(p.controls.x,0.,1.);let a=effective_control(raw_a);
 let gsf=max(5200./f32(width),3100./f32(height));let k=select(1.6,1.2,p.flags.y==1.);
 let base_radius=(1.+(p.geometry.x-1.)/47.)*(1.-clamp(p.flags.x,0.,100.)/100.)/gsf*k*(0.7*raw_a*raw_a+0.3*raw_a+0.05);
 let radius=select(base_radius,0.,p.dimensions.w!=0u);
 let rgb=source(x,y,radius);let luma=dot(rgb,vec3<f32>(0.2125,0.7154,0.0721));
 let uv=vec2<f32>(f32(x)/f32(max(width-1u,1u)),f32(y)/f32(max(height-1u,1u)));
 var g=vec3<f32>(0.);let size=vec2<f32>(f32(width),f32(height))*gsf;
 if(p.dimensions.w!=0u){g=generator(uv,vec2<f32>(f32(width),f32(height)),luma,true);}else{
 let at=vec2<i32>(uv*size/rescale());for(var dy=-1;dy<=1;dy++){for(var dx=-1;dx<=1;dx++){
 let coord=clamp(at+vec2<i32>(dx,dy),vec2<i32>(0),vec2<i32>(size)-1);g+=generator(vec2<f32>(coord)/size,size,luma,false)/9.;
 }} }
 let ws=effective_control(p.controls.y)*a*opacity(luma,0.)*2.;let wm=effective_control(p.controls.z)*a*opacity(luma,0.5);let wh=effective_control(p.controls.w)*a*opacity(luma,1.)*2.;
 for(var c=0u;c<3u;c++){let b=rgb[c];let s=mix(b,overlay(pow(max(b,0.),0.8),g[c]),ws);let m=mix(s,overlay(s,g[c]),wm);let h=mix(m,overlay(m-0.2,g[c]),wh);output_rgb[i+c]=mix(b,h,0.5);}
}
