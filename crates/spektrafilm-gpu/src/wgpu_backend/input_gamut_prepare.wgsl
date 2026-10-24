// Input-gamut preparation stays binary64 until the retained runtime table is narrowed.
struct Params {
    counts: vec4<u32>, // first, count, size, FFT stage
    flags: vec4<u32>,  // algorithm, boundary, source page first, source page length
    white: vec2<f64>,
    knee: vec4<f64>,
    neutral: vec2<f64>,
    detail: f64,
    pad: f64,
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> locus: array<vec2<f64>>;
@group(0) @binding(2) var<storage, read> source: array<f64>;
@group(0) @binding(3) var<storage, read_write> prepared: array<f64>;
@group(0) @binding(4) var<storage, read_write> scratch: array<f64>;
@group(0) @binding(5) var<storage, read_write> narrowed: array<f32>;
const IG_PI: f64 = 3.1415926535897932384626433832795lf;
fn ig_index(gid: vec3<u32>, grid: vec3<u32>) -> u32 { return gid.x + gid.y * grid.x * 256u; }
fn ig_ray(direction: vec2<f64>, hull: bool) -> f64 {
    var nearest: f64 = 1.0lf / p.pad;
    let edges = select(65u, 1024u, hull);
    for (var i=0u; i<edges; i++) {
        var a: vec2<f64>; var b: vec2<f64>;
        if hull {
            a=vec2<f64>(source[i*2u],source[i*2u+1u]);
            let j=(i+1u)%1024u;
            b=vec2<f64>(source[j*2u],source[j*2u+1u]);
        } else { a=locus[i]; b=locus[i+1u]; }
        let edge=b-a;
        let denom=direction.x*edge.y-direction.y*edge.x;
        if abs(denom)<=1e-12 { continue; }
        let delta=p.white-a;
        let t=(-delta.x*edge.y+delta.y*edge.x)/denom;
        let s=(-delta.x*direction.y+delta.y*direction.x)/denom;
        if t>1e-9 && s>=0.0 && s<=1.0 && t<nearest { nearest=t; }
    }
    return nearest;
}
fn ig_inside(xy: vec2<f64>) -> bool {
    var inside=false;
    for(var i=0u;i<65u;i++) {
        let a=locus[i];let b=locus[i+1u];
        if (a.y>xy.y)!=(b.y>xy.y) {
            let crossing=a.x+(xy.y-a.y)*(b.x-a.x)/(b.y-a.y);
            if xy.x<crossing { inside=!inside; }
        }
    }
    return inside;
}
fn ig_inverse(lab: vec3<f64>) -> vec3<f64> {
    let v=vec3<f64>(
        0.9999999984505196lf*lab.x+0.3963377921737678lf*lab.y+0.21580375806075877lf*lab.z,
        1.0000000088817607lf*lab.x-0.10556134232365633lf*lab.y-0.0638541747717059lf*lab.z,
        1.0000000546724108lf*lab.x-0.08948418209496574lf*lab.y-1.2914855378640917lf*lab.z);
    let c=v*v*v;
    return vec3<f64>(
        1.2270138511035211lf*c.x-0.5577999806518222lf*c.y+0.2812561489664678lf*c.z,
        f64(-0.0405801784232806)*c.x+1.11225686961683lf*c.y-0.07167667866560119lf*c.z,
        f64(-0.0763812845057069)*c.x-0.4214819784180127lf*c.y+1.5861632204407947lf*c.z);
}
fn ig_cbrt(v:f64)->f64 {
    if v==0.0 {return v;}
    var c=sign(v)*sf64_exp(sf64_log(abs(v))/3.0);
    c=(2.0*c+v/(c*c))/3.0;
    return c;
}
fn ig_forward(xy:vec2<f64>)->vec3<f64> {
    let sy=max(xy.y,f64(1e-12));
    let xyz=vec3<f64>(xy.x/sy,1.0,(1.0-xy.x-xy.y)/sy);
    let v=vec3<f64>(
        ig_cbrt(0.8189330101lf*xyz.x+0.3618667424lf*xyz.y-0.1288597137lf*xyz.z),
        ig_cbrt(0.0329845436lf*xyz.x+0.9293118715lf*xyz.y+0.0361456387lf*xyz.z),
        ig_cbrt(0.0482003018lf*xyz.x+0.2643662691lf*xyz.y+0.633851707lf*xyz.z));
    return vec3<f64>(0.2104542553lf*v.x+0.793617785lf*v.y-0.0040720468lf*v.z,
        1.9779984951lf*v.x-2.428592205lf*v.y+0.4505937099lf*v.z,
        0.0259040371lf*v.x+0.7827717662lf*v.y-0.808675766lf*v.z);
}
fn ig_xy(xyz:vec3<f64>)->vec2<f64> { return xyz.xy/max(xyz.x+xyz.y+xyz.z,f64(1e-12)); }
fn ig_knee(d:f64)->f64 {
    if d<=p.knee.x {return d;}
    let scale=p.knee.y-p.knee.x;let x=(d-p.knee.x)/scale;
    return p.knee.x+scale*x/sf64_pow(1.0+sf64_pow(x,p.knee.z),1.0/p.knee.z);
}
fn ig_cell_xy(cell:u32)->vec2<f64> {
    let inv=1.0/f64(p.counts.z-1u);
    let sq=sqrt(f64(cell/p.counts.z)*inv);
    return vec2<f64>(1.0-sq,f64(cell%p.counts.z)*inv*sq);
}
fn ig_tc(xy:vec2<f64>)->vec2<f64> {
    let one=1.0-xy.x;
    return clamp(vec2<f64>(one*one,xy.y/max(one,f64(1e-10))),vec2<f64>(0.0),vec2<f64>(1.0));
}
// Reach samples and bit-reversed FFT input. Source is not read on this entrypoint.
@compute @workgroup_size(256)
fn reach(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
    let i=ig_index(gid,grid);if i>=1024u{return;}
    let theta=2.0*IG_PI*f64(i)/1024.0;
    let r=ig_ray(vec2<f64>(jz_cos(theta),jz_sin(theta)),false);
    scratch[i]=r;
    let rev=reverseBits(i)>>22u;
    prepared[2u*rev]=r;prepared[2u*rev+1u]=0.0;
}
// Ten radix-2 stages, then Gaussian frequency weighting and a second FFT.
@compute @workgroup_size(256)
fn fft(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
    let i=ig_index(gid,grid);if i>=512u{return;}
    let half=1u<<p.counts.w;let block=i/half;let j=i%half;
    let a=block*half*2u+j;let b=a+half;
    let angle=select(f64(-1.0),f64(1.0),p.flags.x!=0u)*IG_PI*f64(j)/f64(half);
    let w=vec2<f64>(jz_cos(angle),jz_sin(angle));
    let av=vec2<f64>(source[a*2u],source[a*2u+1u]);
    let bv=vec2<f64>(source[b*2u],source[b*2u+1u]);
    let t=vec2<f64>(bv.x*w.x-bv.y*w.y,bv.x*w.y+bv.y*w.x);
    prepared[a*2u]=av.x+t.x;prepared[a*2u+1u]=av.y+t.y;
    prepared[b*2u]=av.x-t.x;prepared[b*2u+1u]=av.y-t.y;
}
@compute @workgroup_size(256)
fn frequency_filter(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
    let i=ig_index(gid,grid);if i>=1024u{return;}
    let mode=f64(min(i,1024u-i))/p.detail;
    let weight=sf64_exp(-0.5*mode*mode);let rev=reverseBits(i)>>22u;
    prepared[2u*rev]=source[2u*i]*weight;prepared[2u*rev+1u]=source[2u*i+1u]*weight;
}
@compute @workgroup_size(256)
fn hull_scale(@builtin(global_invocation_id) gid:vec3<u32>) {
    if gid.x!=0u{return;}
    var scale:f64=1.0lf/p.pad;
    for(var j=0u;j<1024u;j++) {scale=min(scale,scratch[j]/max(source[j*2u]/1024.0,f64(1e-12)));}
    if scale>1.7976931348623157e308lf {scale=1.0;}
    prepared[0]=scale;
}
@compute @workgroup_size(256)
fn hull(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
    let i=ig_index(gid,grid);if i>=1024u{return;}
    let radius=max(source[i*2u]/1024.0,f64(1e-12))*scratch[0];
    let theta=2.0*IG_PI*f64(i)/1024.0;
    prepared[i*2u]=p.white.x+radius*jz_cos(theta);prepared[i*2u+1u]=p.white.y+radius*jz_sin(theta);
}
@compute @workgroup_size(256)
fn cmax(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
    let local=ig_index(gid,grid);if local>=p.counts.y{return;}let i=p.counts.x+local;
    let l=f64(0.05)+f64(1.0-0.05)*f64(i/720u)/63.0;
    let h=f64(0.0)-IG_PI+2.0*IG_PI*f64(i%720u)/720.0;
    let direction=vec2<f64>(jz_cos(h),jz_sin(h));
    var lo:f64=0.0;var hi:f64=0.5;
    for(var step=0u;step<18u;step++) {
        let mid=(lo+hi)*0.5;
        if ig_inside(ig_xy(ig_inverse(vec3<f64>(l,mid*direction)))) {lo=mid;}else{hi=mid;}
    }
    prepared[local]=lo;
}
// Per-cell Oklch coordinates, or radial-compressed source grid coordinates.
@compute @workgroup_size(256)
fn coordinates(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
    let i=ig_index(gid,grid);if i>=p.counts.y{return;}
    let xy=ig_cell_xy(p.counts.x+i);
    if p.flags.x==1u {
        let lab=ig_forward(xy);prepared[i*4u]=lab.x;prepared[i*4u+1u]=sqrt(lab.y*lab.y+lab.z*lab.z);
        prepared[i*4u+2u]=jz_atan2(lab.z,lab.y);return;
    }
    let delta=xy-p.white;let dist=sqrt(delta.x*delta.x+delta.y*delta.y);var compressed=xy;
    if dist>=1e-9 {
        let direction=delta/max(dist,f64(1e-12));let boundary=ig_ray(direction,p.flags.y==1u);
        compressed=p.white+direction*(ig_knee(dist/max(boundary,f64(1e-12)))*boundary);
    }
    let tc=ig_tc(compressed)*f64(p.counts.z-1u);prepared[i*4u]=tc.x;prepared[i*4u+1u]=tc.y;
}
fn ig_cmax_nodes(l:f64,h:f64)->vec4<u32> {
    // The CPU subtracts the first two grid values, rather than using 2pi/N directly.
    let h0=f64(0.0)-IG_PI;let h1=h0+(2.0*IG_PI)/720.0;let idx=(h-h0)/(h1-h0);
    let hf=floor(idx);let hlo=u32((i32(hf)%720i+720i)%720i);let hhi=(hlo+1u)%720u;
    let li=(clamp(l,f64(0.05),f64(1.0))-0.05lf)/f64(1.0-0.05)*63.0;
    let llo=u32(clamp(i32(floor(li)),0i,62i));
    return vec4<u32>(llo*720u+hlo,llo*720u+hhi,(llo+1u)*720u+hlo,(llo+1u)*720u+hhi);
}
// Gather four Cmax nodes or four RGB corners, by bounded source page.
@compute @workgroup_size(256)
fn gather_cmax(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
    let i=ig_index(gid,grid);if i>=p.counts.y{return;}
    let nodes=ig_cmax_nodes(scratch[i*4u],scratch[i*4u+2u]);
    for(var j=0u;j<4u;j++){let index=nodes[j];if index>=p.flags.z && index-p.flags.z<p.flags.w {prepared[i*4u+j]=source[index-p.flags.z];}}
}
@compute @workgroup_size(256)
fn oklch_coordinates(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
    let i=ig_index(gid,grid);if i>=p.counts.y{return;}
    let l=scratch[i*4u];let c=scratch[i*4u+1u];let h=scratch[i*4u+2u];
    let h0=f64(0.0)-IG_PI;let h1=h0+(2.0*IG_PI)/720.0;let hi=(h-h0)/(h1-h0);let hf=hi-floor(hi);
    let li=(clamp(l,f64(0.05),f64(1.0))-0.05lf)/f64(1.0-0.05)*63.0;let lf=li-f64(clamp(i32(floor(li)),0i,62i));
    let b=i*4u;
    let cm=source[b]*(1.0-lf)*(1.0-hf)+source[b+1u]*(1.0-lf)*hf+source[b+2u]*lf*(1.0-hf)+source[b+3u]*lf*hf;
    let safe=max(cm,f64(1e-9));let cn=ig_knee(c/safe)*safe;
    let tc=ig_tc(ig_xy(ig_inverse(vec3<f64>(l,cn*jz_cos(h),cn*jz_sin(h)))))*f64(p.counts.z-1u);
    prepared[b]=tc.x;prepared[b+1u]=tc.y;
}
@compute @workgroup_size(256)
fn gather_rgb(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
    let i=ig_index(gid,grid);if i>=p.counts.y{return;}
    let ci=scratch[i*4u];let cj=scratch[i*4u+1u];let bi=i32(floor(ci));let bj=i32(floor(cj));
    for(var j=0u;j<4u;j++) {
        let row=u32(clamp(bi+i32(j/2u),0i,i32(p.counts.z)-1i));let col=u32(clamp(bj+i32(j%2u),0i,i32(p.counts.z)-1i));
        for(var c=0u;c<3u;c++) {let index=(row*p.counts.z+col)*3u+c;
            if index>=p.flags.z && index-p.flags.z<p.flags.w {prepared[i*12u+j*3u+c]=source[index-p.flags.z];}
        }
    }
}
@compute @workgroup_size(256)
fn interpolate(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
    let i=ig_index(gid,grid);if i>=p.counts.y{return;}
    let fi=scratch[i*4u]-floor(scratch[i*4u]);let fj=scratch[i*4u+1u]-floor(scratch[i*4u+1u]);
    for(var c=0u;c<3u;c++) {
        let b=i*12u+c;let value=source[b]*(1.0-fi)*(1.0-fj)+source[b+3u]*(1.0-fi)*fj+source[b+6u]*fi*(1.0-fj)+source[b+9u]*fi*fj;
        prepared[i*3u+c]=value;narrowed[i*3u+c]=f32(value);
    }
}
fn ig_reflect(i:i32)->u32 {if i<0i{return u32(-i);}if i>=i32(p.counts.z){return u32(2i*(i32(p.counts.z)-1i)-i);}return u32(i);}
fn ig_mitchell(t:f64)->f64 {
    let x=abs(t);let b=f64(1.0/3.0);let c=f64(1.0/3.0);
    if x<1.0 {return f64(1.0/6.0)*((12.0-9.0*b-6.0*c)*x*x*x+(-18.0+12.0*b+6.0*c)*x*x+(6.0-2.0*b));}
    if x<2.0 {return f64(1.0/6.0)*((f64(0.0)-b-6.0*c)*x*x*x+(6.0*b+30.0*c)*x*x+(-12.0*b-48.0*c)*x+(8.0*b+24.0*c));}
    return 0.0lf;
}
@compute @workgroup_size(256)
fn neutral_gather(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
    let i=ig_index(gid,grid);if i>=48u{return;}
    let xy=clamp(p.neutral*f64(p.counts.z-1u),vec2<f64>(0.0),vec2<f64>(f64(p.counts.z-1u)));
    let base=min(vec2<u32>(xy),vec2<u32>(p.counts.z-2u));let corner=i/3u;
    let row=ig_reflect(i32(base.x)+i32(corner/4u)-1i);let col=ig_reflect(i32(base.y)+i32(corner%4u)-1i);
    let index=(row*p.counts.z+col)*3u+i%3u;
    if index>=p.flags.z && index-p.flags.z<p.flags.w {prepared[i]=source[index-p.flags.z];}
}
@compute @workgroup_size(256)
fn neutral_reduce(@builtin(global_invocation_id) gid:vec3<u32>) {
    if gid.x>=3u{return;}
    let xy=clamp(p.neutral*f64(p.counts.z-1u),vec2<f64>(0.0),vec2<f64>(f64(p.counts.z-1u)));
    let base=min(vec2<u32>(xy),vec2<u32>(p.counts.z-2u));let f=xy-vec2<f64>(base);
    var value:f64=0.0;var total:f64=0.0;
    for(var i=0u;i<4u;i++){for(var j=0u;j<4u;j++){
        let weight=ig_mitchell(f.x+1.0-f64(i))*ig_mitchell(f.y+1.0-f64(j));
        total=total+weight;value=value+weight*source[(i*4u+j)*3u+gid.x];
    }}
    if total!=0.0 {value=value/total;}prepared[gid.x]=value;
}
