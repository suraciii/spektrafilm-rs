// LibRaw 0.22.2 xtrans_demosaic.cpp, Markesteijn three-pass interpolation.
// One invocation owns a native 512-square tile. The reference's in-place
// row-major refinements are intentionally serial, not Jacobi approximations.
// Adjacent top rows are also ordered: native green bounds in image[][1]
// (xtrans_demosaic.cpp:127) are read by LIM at :232/:257 and overwritten
// by final RGB writeback at :423. LibRaw's OpenMP top loop at :178 races
// those reads/writes; this kernel preserves the deterministic serial order,
// not a schedule-dependent snapshot of independently executing top rows.
struct DemosaicPass { colors:u32, stage:u32, top:u32, left:u32 };
@group(0) @binding(0) var<uniform> frame:RawFrame;
@group(0) @binding(1) var<uniform> settings:DemosaicPass;
@group(0) @binding(2) var<storage,read_write> image:array<u32>;
@group(0) @binding(3) var<storage,read> cfa:array<u32>;
@group(0) @binding(4) var<storage,read> metadata:array<i32>;
@group(0) @binding(5) var<storage,read_write> scratch:array<u32>;
@group(0) @binding(6) var<storage,read> xyz_camera:array<vec4<f32>,3>;
@group(0) @binding(7) var<storage,read> cbrt:array<f32>;
const N:i32=262144;
const LAB:i32=6291456;
const DRV:i32=7077888;
const HOM:i32=9175040;
fn color(y:i32,x:i32)->i32{return i32(cfa[u32((y+6)%6)*6u+u32((x+6)%6)]);}
fn hex(y:i32,x:i32,tile:bool,k:i32)->i32{return metadata[u32(((y%3)*3+x%3)*16+select(0,8,tile)+k)];}
fn src(p:i32,c:i32)->i32{return i32(image[image_index(p)*4u+u32(c)]);}
fn rgb(d:i32,p:i32,c:i32)->i32{return i32(scratch[u32((d*N+p)*3+c)]);}
fn put(d:i32,p:i32,c:i32,v:i32){scratch[u32((d*N+p)*3+c)]=u32(v)&65535u;}
fn lab(p:i32,c:i32)->i32{return bitcast<i32>(scratch[u32(LAB+p*3+c)]);}
fn deriv(d:i32,p:i32)->f32{return bitcast<f32>(scratch[u32(DRV+d*N+p)]);}
fn homo(d:i32,p:i32)->i32{return i32(scratch[u32(HOM+d*N+p)]);}
fn cielab(p:i32,d:i32){
    var xyz=vec3<f32>(0.5);let samples=vec3<f32>(f32(rgb(d,p,0)),f32(rgb(d,p,1)),f32(rgb(d,p,2)));
    for(var c=0u;c<3u;c++){xyz+=vec3<f32>(xyz_camera[0][c],xyz_camera[1][c],xyz_camera[2][c])*samples[c];}
    let x=cbrt[raw_clip(i32(xyz.x))];let y=cbrt[raw_clip(i32(xyz.y))];let z=cbrt[raw_clip(i32(xyz.z))];
    let value=vec3<i32>(i32(64.0*(116.0*y-16.0)),i32(64.0*500.0*(x-y)),i32(64.0*200.0*(y-z)));
    for(var c=0;c<3;c++){scratch[u32(LAB+p*3+c)]=bitcast<u32>(value[c]<<16>>16);}
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>){
    if(gid.x!=0u || metadata[146]!=0){return;}
    let top=i32(settings.top);let left=i32(settings.left);let w=i32(frame.width);let height=i32(frame.height);
    var mr=min(top+512,height-3);var mc=min(left+512,w-3);let sy=metadata[144];let sx=metadata[145];
    // Copy the measured RGB planes; retain tile padding just as native scratch.
    for(var y=top;y<mr;y++){for(var x=left;x<mc;x++){
        let p=(y-top)*512+x-left;let q=y*w+x;
        for(var d=0;d<4;d++){for(var c=0;c<3;c++){if(settings.stage==0u || c!=1 || color(y,x)==1){put(d,p,c,src(q,c));}}}
    }}
    if(settings.stage==0u){
    for(var y=top;y<mr;y++){for(var x=left;x<mc;x++){
        let f=color(y,x);if(f==1){continue;}let q=y*w+x;let p=(y-top)*512+x-left;
        var h:array<i32,8>;for(var c=0;c<8;c++){h[c]=hex(y,x,false,c);}
        var values:array<i32,4>;
        values[0]=174*(src(q+h[1],1)+src(q+h[0],1))-46*(src(q+2*h[1],1)+src(q+2*h[0],1));
        values[1]=223*src(q+h[3],1)+33*src(q+h[2],1)+92*(src(q,f)-src(q-h[2],f));
        for(var c=0;c<2;c++){values[2+c]=164*src(q+h[4+c],1)+92*src(q-2*h[4+c],1)+33*(2*src(q,f)-src(q+3*h[4+c],f)-src(q-3*h[4+c],f));}
        for(var c=0;c<4;c++){put(c^i32((y-sy)%3==0),p,1,max(src(q,1),min(values[c]>>8,src(q,3))));}
    }}
    }
    var base=0;
    for(var iteration=0;iteration<3;iteration++){
        if(iteration==1){for(var i=0;i<4*N*3;i++){scratch[u32(4*N*3+i)]=scratch[u32(i)];}base=4;}
        if(iteration!=0){
            for(var y=top+2;y<mr-2;y++){for(var x=left+2;x<mc-2;x++){
                let f=color(y,x);if(f==1){continue;}let q=y*w+x;let p=(y-top)*512+x-left;
                for(var d=3;d<6;d++){
                    let dd=base+((d-2)^i32((y-sy)%3==0));let h=hex(y,x,true,d);
                    let value=rgb(dd,p-2*h,1)+2*rgb(dd,p+h,1)-rgb(dd,p-2*h,f)-2*rgb(dd,p+h,f)+3*rgb(dd,p,f);
                    put(dd,p,1,max(src(q,1),min(value/3,src(q,3))));
                }
            }}
        }
        // Solitary green pixels: each directional pair selects its own model.
        for(var y=(top-sy+4)/3*3+sy;y<mr-2;y+=3){
            for(var x=(left-sx+4)/3*3+sx;x<mc-2;x+=3){
                var h=color(y,x+1);if(h==1){break;}
                let p=(y-top)*512+x-left;var values:array<i32,18>;var diff:array<f32,6>;
                for(var d=0;d<6;d++){diff[d]=0.0;}
                var i=1;var dd=base;
                for(var d=0;d<6;d++){
                    for(var c=0;c<2;c++){
                        let offset=i<<u32(c);let g=2*rgb(dd,p,1)-rgb(dd,p+offset,1)-rgb(dd,p-offset,1);
                        values[h*6+d]=g+rgb(dd,p+offset,h)+rgb(dd,p-offset,h);
                        if(d>1){let delta=f32(rgb(dd,p+offset,1))-f32(rgb(dd,p-offset,1))-f32(rgb(dd,p+offset,h))+f32(rgb(dd,p-offset,h));diff[d]+=delta*delta+f32(g)*f32(g);}
                        h=h^2;
                    }
                    if(d>1 && (d&1)!=0 && diff[d-1]<diff[d]){for(var c=0;c<2;c++){values[c*12+d]=values[c*12+d-1];}}
                    if(d<2 || (d&1)!=0){for(var c=0;c<2;c++){put(dd,p,c*2,i32(raw_clip(values[c*12+d]/2)));}dd++;}
                    i=i^513;h=h^2;
                }
            }
        }
        // Opposite chroma at red/blue sites.
        for(var y=top+3;y<mr-3;y++){for(var x=left+3;x<mc-3;x++){
            let f=2-color(y,x);if(f==1){continue;}let p=(y-top)*512+x-left;
            let c=select(1,512,(y-sy)%3!=0);let h=3*(c^513);
            for(var d=0;d<4;d++){
                let dd=base+d;var i=h;
                if(d>1 || ((d^c)&1)!=0 || abs(rgb(dd,p,1)-rgb(dd,p+c,1))+abs(rgb(dd,p,1)-rgb(dd,p-c,1))<2*(abs(rgb(dd,p,1)-rgb(dd,p+h,1))+abs(rgb(dd,p,1)-rgb(dd,p-h,1)))){i=c;}
                put(dd,p,f,i32(raw_clip((rgb(dd,p+i,f)+rgb(dd,p-i,f)+2*rgb(dd,p,1)-rgb(dd,p+i,1)-rgb(dd,p-i,1))/2)));
            }
        }}
        // The 2x2 green blocks use the native green hexagon pairs.
        for(var y=top+2;y<mr-2;y++){
            if((y-sy)%3==0){continue;}
            for(var x=left+2;x<mc-2;x++){
                if((x-sx)%3==0){continue;}let p=(y-top)*512+x-left;
                for(var d=0;d<8;d+=2){
                    let dd=base+d/2;let h=hex(y,x,true,d);let k=hex(y,x,true,d+1);
                    if(h+k!=0){let g=3*rgb(dd,p,1)-2*rgb(dd,p+h,1)-rgb(dd,p+k,1);for(var c=0;c<4;c+=2){put(dd,p,c,i32(raw_clip((g+2*rgb(dd,p+h,c)+rgb(dd,p+k,c))/3)));}}
                    else{let g=2*rgb(dd,p,1)-rgb(dd,p+h,1)-rgb(dd,p+k,1);for(var c=0;c<4;c+=2){put(dd,p,c,i32(raw_clip((g+rgb(dd,p+h,c)+rgb(dd,p+k,c))/2)));}}
                }
            }
        }
    }
    mr-=top;mc-=left;
    // CIELab signed-short quantization and native integer second derivative.
    let dirs=array<i32,4>(1,512,513,511);
    for(var d=0;d<8;d++){
        for(var y=2;y<mr-2;y++){for(var x=2;x<mc-2;x++){cielab(y*512+x,d);}}
        let f=dirs[d&3];
        for(var y=3;y<mr-3;y++){for(var x=3;x<mc-3;x++){
            let p=y*512+x;let g=2*lab(p,0)-lab(p+f,0)-lab(p-f,0);
            let a=2*lab(p,1)-lab(p+f,1)-lab(p-f,1)+g*500/232;
            let b=2*lab(p,2)-lab(p+f,2)-lab(p-f,2)-g*500/580;
            scratch[u32(DRV+d*N+p)]=bitcast<u32>(f32(g*g+a*a+b*b));
        }}
    }
    for(var i=0;i<8*N;i++){scratch[u32(HOM+i)]=0u;}
    for(var y=4;y<mr-4;y++){for(var x=4;x<mc-4;x++){
        let p=y*512+x;var threshold=3.402823466e38;
        for(var d=0;d<8;d++){threshold=min(threshold,deriv(d,p));}threshold*=8.0;
        for(var d=0;d<8;d++){var count=0u;for(var v=-1;v<=1;v++){for(var h=-1;h<=1;h++){if(deriv(d,p+v*512+h)<=threshold){count++;}}}scratch[u32(HOM+d*N+p)]=count;}
    }}
    if(height-top<516){mr=height-top+2;}if(w-left<516){mc=w-left+2;}
    for(var y=min(top,8);y<mr-8;y++){for(var x=min(left,8);x<mc-8;x++){
        let p=y*512+x;var hm:array<i32,8>;
        for(var d=0;d<8;d++){hm[d]=0;}
        for(var d=0;d<8;d++){for(var v=-2;v<=2;v++){for(var h=-2;h<=2;h++){hm[d]+=homo(d,p+v*512+h);}}}
        for(var d=0;d<4;d++){if(hm[d]<hm[d+4]){hm[d]=0;}else if(hm[d]>hm[d+4]){hm[d+4]=0;}}
        var high=hm[0];for(var d=1;d<8;d++){high=max(high,hm[d]);}high-=high>>3;
        var avg:array<i32,4>;
        for(var c=0;c<4;c++){avg[c]=0;}
        for(var d=0;d<8;d++){if(hm[d]>=high){for(var c=0;c<3;c++){avg[c]+=rgb(d,p,c);}avg[3]++;}}
        for(var c=0;c<3;c++){image[image_index((y+top)*w+x+left)*4u+u32(c)]=u32(avg[c]/avg[3]);}
    }}
}
