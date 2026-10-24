// LibRaw 0.22.2 misc_demosaic.cpp: ppg_interpolate, three ordered passes.
struct DemosaicPass { colors:u32, stage:u32, top:u32, left:u32 };
@group(0) @binding(0) var<uniform> frame:RawFrame;
@group(0) @binding(1) var<uniform> settings:DemosaicPass;
@group(0) @binding(2) var<storage,read_write> image:array<u32>;
@group(0) @binding(3) var<storage,read> cfa:array<u32>;
fn color(y:u32,x:u32)->u32 { return raw_color_from_code(cfa[raw_cfa_index(x,y,frame)],frame); }
fn sample(p:i32,c:u32)->i32 { return i32(image[u32(p)*4u+c]); }
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>, @builtin(num_workgroups) grid:vec3<u32>) {
    let p=gid.x+gid.y*grid.x*256u; if(p>=raw_pixel_count(frame)){return;}
    let y=p/frame.width; let x=p%frame.width; let f=color(y,x); let q=i32(p); let w=i32(frame.width);
    if(settings.stage==0u) {
        if(x<3u || y<3u || x+3u>=frame.width || y+3u>=frame.height || (f&1u)!=0u){return;}
        var guess:array<i32,2>; var diff:array<i32,2>;
        for(var i=0u;i<2u;i++) {
            let d=select(1,w,i==1u);
            guess[i]=(sample(q-d,1u)+sample(q,f)+sample(q+d,1u))*2-sample(q-2*d,f)-sample(q+2*d,f);
            diff[i]=(abs(sample(q-2*d,f)-sample(q,f))+abs(sample(q+2*d,f)-sample(q,f))+abs(sample(q-d,1u)-sample(q+d,1u)))*3+(abs(sample(q+3*d,1u)-sample(q+d,1u))+abs(sample(q-3*d,1u)-sample(q-d,1u)))*2;
        }
        let i=u32(diff[0]>diff[1]); let d=select(1,w,i==1u);
        image[p*4u+1u]=raw_ulim(guess[i]>>2, sample(q+d,1u),sample(q-d,1u));
    } else if(settings.stage==1u) {
        if(x<1u || y<1u || x+1u>=frame.width || y+1u>=frame.height || f!=1u){return;}
        var c=color(y,x+1u);
        for(var i=0u;i<2u;i++) {
            let d=select(1,w,i==1u);
            image[p*4u+c]=raw_clip((sample(q-d,c)+sample(q+d,c)+2*sample(q,1u)-sample(q-d,1u)-sample(q+d,1u))>>1);
            c=2u-c;
        }
    } else {
        if(x<1u || y<1u || x+1u>=frame.width || y+1u>=frame.height || (f&1u)!=0u){return;}
        let c=2u-f; var diff:array<i32,2>; var guess:array<i32,2>;
        for(var i=0u;i<2u;i++) {
            let d=select(w+1,w-1,i==1u);
            diff[i]=abs(sample(q-d,c)-sample(q+d,c))+abs(sample(q-d,1u)-sample(q,1u))+abs(sample(q+d,1u)-sample(q,1u));
            guess[i]=sample(q-d,c)+sample(q+d,c)+2*sample(q,1u)-sample(q-d,1u)-sample(q+d,1u);
        }
        var result=(guess[0]+guess[1])>>2;
        if(diff[0]!=diff[1]){result=guess[u32(diff[0]>diff[1])]>>1;}
        image[p*4u+c]=raw_clip(result);
    }
}
