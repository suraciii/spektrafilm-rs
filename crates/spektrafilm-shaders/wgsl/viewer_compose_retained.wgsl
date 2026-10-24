// Native viewer composition: interpolation, split, crossfade and RGB8 packing.
struct Params {
 width:u32, height:u32, sw:u32, sh:u32,
 pw:u32, ph:u32, ow:u32, oh:u32, lw:u32, lh:u32,
 mode:u32, full:u32, output_layer:u32, pad0:u32, pad1:u32, pad2:u32,
 rect:vec4<f32>, border:vec4<f32>, paper_rect:vec4<f32>, image_rect:vec4<f32>,
 gray:vec3<f32>, split:f32, fade:f32, pad3:u32, pad4:u32, pad5:u32,
 windows:array<vec4<u32>,4>, first:u32,count:u32,pad6:u32,pad7:u32,
}
@group(0) @binding(0) var<uniform> params:Params;
@group(0) @binding(1) var<storage,read> source:array<f32>;
@group(0) @binding(2) var<storage,read> paper:array<f32>;
@group(0) @binding(3) var<storage,read> previous:array<f32>;
@group(0) @binding(4) var<storage,read> left:array<f32>;
@group(0) @binding(5) var<storage,read_write> out:array<u32>;
fn radius(mode: u32) -> i32 {
    if mode <= 1u {
        return 1; // nearest, linear
    }
    if mode <= 3u {
        return 2; // cubic, spline16
    }
    if mode == 4u {
        return 3; // spline36
    }
    return 4; // lanczos, blackman
}

// Per-mode kernel weight, matching the host's `Interpolation::weight`.
fn weight(mode: u32, x: f32) -> f32 {
    let a = abs(x);
    if a >= f32(radius(mode)) {
        return 0.0;
    }
    if mode == 0u {
        // nearest
        if a < 0.5 {
            return 1.0;
        }
        return 0.0;
    }
    if mode == 1u {
        return 1.0 - a; // linear
    }
    if mode == 2u {
        // cubic
        if a < 1.0 {
            return (4.0 - 6.0 * a * a + 3.0 * a * a * a) / 6.0;
        }
        let y = 2.0 - a;
        return y * y * y / 6.0;
    }
    if mode == 3u {
        // spline16
        if a < 1.0 {
            return ((a - 1.8) * a - 0.2) * a + 1.0;
        }
        let y = a - 1.0;
        return ((-y / 3.0 + 0.8) * y - 7.0 / 15.0) * y;
    }
    if mode == 4u {
        // spline36
        if a < 1.0 {
            return ((13.0 / 11.0 * a - 453.0 / 209.0) * a - 3.0 / 209.0) * a + 1.0;
        }
        if a < 2.0 {
            let y = a - 1.0;
            return ((-6.0 / 11.0 * y + 270.0 / 209.0) * y - 156.0 / 209.0) * y;
        }
        let y = a - 2.0;
        return ((y / 11.0 - 45.0 / 209.0) * y + 26.0 / 209.0) * y;
    }
    // lanczos (5) and blackman (6)
    if a < 1e-7 {
        return 1.0;
    }
    let p = 3.141592653589793 * a;
    let q = p / 4.0;
    let window = select(
        0.42 + 0.5 * cos(q) + 0.08 * cos(2.0 * q),
        sin(q) / q,
        mode == 5u,
    );
    return sin(p) / p * window;
}


fn contains(r:vec4<f32>, p:vec2<f32>)->bool { return all(p>=r.xy) && all(p<r.zw); }
fn pixel(which:u32,x:i32,y:i32,w:u32,h:u32)->vec3<f32> {
 let win=params.windows[which];let i=((u32(clamp(y,0,i32(h)-1))-win.y)*win.z+u32(clamp(x,0,i32(w)-1))-win.x)*3u;
 if which==1u { return vec3<f32>(paper[i],paper[i+1u],paper[i+2u]); }
 if which==2u { return vec3<f32>(previous[i],previous[i+1u],previous[i+2u]); }
 if which==3u { return vec3<f32>(left[i],left[i+1u],left[i+2u]); }
 return vec3<f32>(source[i],source[i+1u],source[i+2u]);
}
fn sample(which:u32,p:vec2<f32>,rect:vec4<f32>,w:u32,h:u32,mode:u32)->vec3<f32> {
 let q=(p-rect.xy)/(rect.zw-rect.xy)*vec2<f32>(f32(w),f32(h))-vec2<f32>(0.5);
 if mode==0u { return pixel(which,i32(floor(q.x+0.5)),i32(floor(q.y+0.5)),w,h); }
 let r=radius(mode); let b=vec2<i32>(floor(q)); var rgb=vec3<f32>(0.0);
 for(var j=0;j<2*r;j++){for(var i=0;i<2*r;i++){
 let x=b.x+i-r+1;let y=b.y+j-r+1;
 let weight_xy=weight(mode,q.x-f32(x))*weight(mode,q.y-f32(y));
 rgb+=weight_xy*pixel(which,x,y,w,h);
 }}return rgb;
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>){
 let local=gid.x+gid.y*grid.x*256u;if local>=params.count{return;}let i=local+params.first;
 let p=params.rect.xy+(vec2<f32>(f32(i%params.width),f32(i/params.width))+vec2<f32>(0.5))*(params.rect.zw-params.rect.xy)/vec2<f32>(f32(params.width),f32(params.height));
 var rgb=params.gray;if contains(params.border,p){rgb=vec3<f32>(1.0);}
 if params.pw>0u && contains(params.paper_rect,p) && !contains(params.image_rect,p){rgb=sample(1u,p,params.paper_rect,params.pw,params.ph,4u);}
 if params.sw>0u && contains(params.image_rect,p){
 if params.full!=0u {rgb=pixel(0u,i32(i%params.width),i32(i/params.width),params.sw,params.sh);}else{rgb=sample(0u,p,params.image_rect,params.sw,params.sh,params.mode);}
 if params.output_layer!=0u && params.lw>0u && p.x<params.image_rect.x+(params.image_rect.z-params.image_rect.x)*clamp(params.split,0.0,1.0){rgb=sample(3u,p,params.image_rect,params.lw,params.lh,params.mode);}
 if params.output_layer!=0u && params.ow>0u && params.lw==0u {let old=sample(2u,p,params.image_rect,params.ow,params.oh,params.mode);rgb=clamp(old,vec3<f32>(0.0),vec3<f32>(1.0))*(1.0-params.fade)+clamp(rgb,vec3<f32>(0.0),vec3<f32>(1.0))*params.fade;}
 }
 let b=vec3<u32>(floor(clamp(rgb,vec3<f32>(0.0),vec3<f32>(1.0))*255.0+vec3<f32>(0.5)));
 out[local*3u]=b.x|(b.y<<8u)|(b.z<<16u)|0xff000000u;
}
