// Derived from darktable data/kernels/demosaic_markesteijn.cl
// Copyright (c) 2016 Ulrich Pegelow and darktable contributors.
// SPDX-License-Identifier: GPL-3.0-or-later
// Port: explicit storage snapshots replace OpenCL local-memory staging.
// Three-pass Markesteijn, following darktable's OpenCL stage order.
// Every image stage is one invocation per pixel. Spatial stages read a snapshot.
// Integer sensor samples enter/leave here; exact 2^-16 scaling keeps the
// homogeneity epsilon in normalized units without reciprocal rounding.
struct Params {
    width: u32,
    height: u32,
    stage: u32,
    refinement: u32,
    direction: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
};
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read_write> image:array<vec4<u32>>;
@group(0) @binding(2) var<storage,read> cfa:array<u32>;
// 72 xy hex offsets, solitary green xy, validation status.
@group(0) @binding(3) var<storage,read_write> metadata:array<i32>;
@group(0) @binding(4) var<storage,read> src:array<vec4<f32>>;
@group(0) @binding(5) var<storage,read_write> dst:array<vec4<f32>>;
// First N entries: green bounds. Second N: solitary-green carry.
@group(0) @binding(6) var<storage,read_write> aux:array<vec4<f32>>;
// Eight derivative planes (float bits), eight homogeneity planes, eight sums.
@group(0) @binding(7) var<storage,read_write> scores:array<u32>;
const ORTH:array<i32,12> = array<i32,12>(1,0,0,1,-1,0,0,-1,1,0,0,1);
const PATT:array<i32,32> = array<i32,32>(0,1,0,-1,2,0,-1,0,1,1,1,-1,0,0,0,0,0,1,0,-2,1,0,-2,0,1,1,-2,-2,1,-1,-1,1);
const DIR:array<vec2<i32>,4> = array<vec2<i32>,4>(vec2(1,0),vec2(0,1),vec2(1,1),vec2(-1,1));
fn color(q:vec2<i32>)->u32 {
    let v=(q%vec2(6)+vec2(6))%vec2(6);
    return cfa[u32(v.y*6+v.x)];
}
fn inside(q:vec2<i32>,b:i32)->bool {
    return all(q>=vec2(b)) && all(q<vec2(i32(p.width)-b,i32(p.height)-b));
}
fn index(q:vec2<i32>)->u32 {
    return u32(q.y)*p.width+u32(q.x);
}
fn at(d:u32,q:vec2<i32>)->vec4<f32> {
    if(!inside(q,0)) {
        return vec4(0.0);
    }
    return src[d*p.width*p.height+index(q)];
}
fn hex(q:vec2<i32>,k:u32)->vec2<i32> {
    let v=(q%vec2(3)+vec2(3))%vec2(3);
    let j=u32(v.y*3+v.x)*16u+k*2u;
    return vec2(metadata[j],metadata[j+1u]);
}
fn yuv(d:u32,q:vec2<i32>)->vec3<f32> {
    if(!inside(q,13)) {
        return vec3(0.0);
    }
    let v=at(d,q);
    let y=0.2627*v.x+0.678*v.y+0.0593*v.z;
    return vec3(y,(v.z-y)*0.56433,(v.x-y)*0.67815);
}
fn tables() {
    var counts:array<u32,3>;
    metadata[146]=0;
    for(var i=0u;i<36u;i++) {
        let c=cfa[i];
        if(c>2u) {
            metadata[146]=1;
            return;
        }
        counts[c]++;
    }
    if(counts[0]!=8u || counts[1]!=20u || counts[2]!=8u) {
        metadata[146]=1;
        return;
    }
    for(var i=0u;i<144u;i++) {
        metadata[i]=0;
    }
    var solitary=0u;
    for(var y=0;y<3;y++) {
        for(var x=0;x<3;x++) {
            var ng=0;
            let g=i32(color(vec2(x,y))==1u);
            for(var d=0u;d<10u;d+=2u) {
                if(color(vec2(x+ORTH[d+2u],y+ORTH[d]))==1u) {
                    ng=0;
                }
                else {
                    ng++;
                }
                if(ng==4) {
                    metadata[144]=x;
                    metadata[145]=y;
                    solitary++;
                }
                if(ng==g+1) {
                    for(var c=0u;c<8u;c++) {
                        let v=ORTH[d]*PATT[u32(g)*16u+c*2u]+ORTH[d+1u]*PATT[u32(g)*16u+c*2u+1u];
                        let h=ORTH[d+2u]*PATT[u32(g)*16u+c*2u]+ORTH[d+3u]*PATT[u32(g)*16u+c*2u+1u];
                        let j=u32(y*3+x)*16u+(c^(u32(g*2)&d))*2u;
                        metadata[j]=h;
                        metadata[j+1u]=v;
                    }
                }
            }
        }
    }
    if(solitary!=1u) {
        metadata[146]=1;
        return;
    }
    for(var y=0;y<6;y++) {
        for(var x=0;x<6;x++) {
            let q=vec2(x,y);
            if(color(q)==1u) {
                continue;
            }
            for(var k=0u;k<6u;k++) {
                if(color(q+hex(q,k))!=1u) {
                    metadata[146]=1;
                }
            }
        }
    }
}
@compute @workgroup_size(256) fn main(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
    let i=gid.x+gid.y*grid.x*256u;
    let n=p.width*p.height;
    if(p.stage==0u) {
        if(i==0u) {
            tables();
        }
        return;
    }
    if(i>=n || metadata[146]!=0) {
        return;
    }
    let q=vec2(i32(i%p.width),i32(i/p.width));
    let f=color(q);
    let sg=vec2(metadata[144],metadata[145]);
    let flip=u32((q.y-sg.y)%3==0);
    if(p.stage==1u) {
        var v=vec4(0.0);
        v[f]=f32(image[i][f])/65536.0;
        for(var d=0u;d<8u;d++) {
            dst[d*n+i]=v;
        }
        return;
    }
    if(p.stage==2u) {
        if(!inside(q,3) || f==1u) {
            return;
        }
        var lo=3.402823466e38;
        var hi=0.0;
        for(var k=0u;k<6u;k++) {
            let v=at(0u,q+hex(q,k)).y;
            lo=min(lo,v);
            hi=max(hi,v);
        }
        var other=q;
        for(var k=0u;k<4u;k++) {
            let o=vec2(ORTH[2u*k],ORTH[2u*k+1u]);
            if(color(q+o)!=1u) {
                other=q+o;
            }
        }
        for(var k=0u;k<6u;k++) {
            let v=at(0u,other+hex(other,k)).y;
            lo=min(lo,v);
            hi=max(hi,v);
        }
        aux[i]=vec4(lo,hi,0.0,0.0);
        return;
    }
    if(p.stage>=9u) {
        if(p.stage==9u) {
            if(!inside(q,13)) {
                return;
            }
            for(var d=0u;d<8u;d++) {
                let o=DIR[d&3u];
                let v=2.0*yuv(d,q)-yuv(d,q+o)-yuv(d,q-o);
                scores[d*n+i]=bitcast<u32>(v.x*v.x+v.y*v.y+v.z*v.z);
            }
            return;
        }
        if(p.stage==10u) {
            if(!inside(q,15)) {
                return;
            }
            var threshold=3.402823466e38;
            for(var d=0u;d<8u;d++) {
                threshold=min(threshold,bitcast<f32>(scores[d*n+i]));
            }
            threshold=max(8.0*threshold,1e-10);
            for(var d=0u;d<8u;d++) {
                var h=0u;
                for(var y=-1;y<=1;y++) {
                    for(var x=-1;x<=1;x++) {
                        h+=u32(bitcast<f32>(scores[d*n+index(q+vec2(x,y))])<=threshold);
                    }
                }
                scores[(8u+d)*n+i]=h;
            }
            return;
        }
        if(p.stage==11u) {
            if(!inside(q,17)) {
                return;
            }
            for(var d=0u;d<8u;d++) {
                var h=0u;
                for(var y=-2;y<=2;y++) {
                    for(var x=-2;x<=2;x++) {
                        h+=scores[(8u+d)*n+index(q+vec2(x,y))];
                    }
                }
                scores[(16u+d)*n+i]=h;
            }
            return;
        }
        if(p.stage==12u) {
            if(!inside(q,17)) {
                return;
            }
            var hm:array<u32,8>;
            var maximum=0u;
            for(var d=0u;d<8u;d++) {
                hm[d]=scores[(16u+d)*n+i];
                maximum=max(maximum,hm[d]);
            }
            maximum-=maximum>>3u;
            for(var d=0u;d<4u;d++) {
                let a=hm[d];
                let b=hm[d+4u];
                if(a<b) {
                    hm[d]=0u;
                }
                if(a>b) {
                    hm[d+4u]=0u;
                }
            }
            var sum=vec3(0.0);
            var weight=0.0;
            for(var d=0u;d<8u;d++) {
                if(hm[d]>=maximum) {
                    sum+=at(d,q).xyz;
                    weight+=1.0;
                }
            }
            let v=clamp(sum/max(weight,1.0),vec3(0.0),vec3(65535.0/65536.0));
            image[i]=vec4(vec3<u32>(round(v*65536.0)),0u);
            return;
        }
    }
    // All spatial updates copy unchanged samples too. Alternate source and
    // destination allocations after each dispatch; never read another write.
    for(var d=0u;d<8u;d++) {
        var v=at(d,q);
        let enabled=(p.refinement==0u && d<4u)||(p.refinement>0u && d>=4u);
        let local=d&3u;
        if(p.stage==4u) {
            if(d>=4u) {
                v=at(d-4u,q);
            }
        }
        if(p.stage==3u && d<4u && inside(q,3) && f!=1u) {
            let h0=hex(q,0u);
            let h1=hex(q,1u);
            let h2=hex(q,2u);
            let h3=hex(q,3u);
            var values:array<f32,4>;
            values[0]=0.6796875*(at(0u,q+h1).y+at(0u,q+h0).y)-0.1796875*(at(0u,q+2*h1).y+at(0u,q+2*h0).y);
            values[1]=0.87109375*at(0u,q+h3).y+0.13*at(0u,q+h2).y+0.359375*(v[f]-at(0u,q-h2)[f]);
            for(var k=0u;k<2u;k++) {
                let h=hex(q,4u+k);
                values[2u+k]=0.640625*at(0u,q+h).y+0.359375*at(0u,q-2*h).y+0.12890625*(2.0*v[f]-at(0u,q+3*h)[f]-at(0u,q-3*h)[f]);
            }
            v.y=clamp(values[d^flip],aux[i].x,aux[i].y);
        }
        if(p.stage==5u && enabled && inside(q,6) && f!=1u) {
            for(var k=3u;k<6u;k++) {
                if(((k-2u)^flip)!=local) {
                    continue;
                }
                let h=hex(q,k);
                let a=at(d,q-2*h);
                let b=at(d,q+h);
                v.y=clamp((a.y+2.0*b.y-a[f]-2.0*b[f]+3.0*v[f])/3.0,aux[i].x,aux[i].y);
            }
        }
        if(p.stage==6u && enabled && inside(q,5) && all((q-sg)%vec2(3)==vec2(0))) {
            let k=p.direction;
            var destination=k;
            if(k>=2u) {
                destination=(k+2u)/2u;
            }
            if(local==destination) {
                var old=aux[n+i];
                if(k==0u) {
                    old=vec4(0.0);
                }
                var cv=old;
                var diff=0.0;
                let axis=1u^(k&1u);
                let o=vec2(i32(axis),i32(axis^1u));
                var h=color(q+vec2(1,0))^((k&1u)*2u);
                for(var c=0u;c<2u;c++) {
                    let offset=o*(1i<<c);
                    let a=at(d,q+offset);
                    let b=at(d,q-offset);
                    let g=2.0*v.y-a.y-b.y;
                    cv[h]=g+a[h]+b[h];
                    if(k>1u) {
                        let t=a.y-b.y-a[h]+b[h];
                        diff+=t*t+g*g;
                    }
                    h^=2u;
                }
                cv.w=diff;
                if(k>1u && (k&1u)!=0u && old.w<diff) {
                    cv.x=old.x;
                    cv.z=old.z;
                }
                if(k<2u || (k&1u)!=0u) {
                    v.x=cv.x*0.5;
                    v.z=cv.z*0.5;
                }
                aux[n+i]=cv;
            }
        }
        if(p.stage==7u && enabled && inside(q,5) && f!=1u) {
            let horizontal=(q.y-sg.y)%3==0;
            let near=select(vec2(0,1),vec2(1,0),horizontal);
            let far=select(vec2(3,0),vec2(0,3),horizontal);
            let gn=abs(v.y-at(d,q+near).y)+abs(v.y-at(d,q-near).y);
            let gf=abs(v.y-at(d,q+far).y)+abs(v.y-at(d,q-far).y);
            let o=select(far,near,local>1u || (local==0u && horizontal) || (local==1u && !horizontal) || gn<2.0*gf);
            let a=at(d,q+o);
            let b=at(d,q-o);
            let c=2u-f;
            v[c]=(a[c]+b[c]+2.0*v.y-a.y-b.y)*0.5;
        }
        if(p.stage==8u && enabled && inside(q,4) && all((q-sg)%vec2(3)!=vec2(0))) {
            let a=at(d,q+hex(q,local*2u));
            let b=at(d,q+hex(q,local*2u+1u));
            let skew=any(hex(q,local*2u)+hex(q,local*2u+1u)!=vec2(0));
            let m=select(vec4(2.0,-1.0,-1.0,1.0),vec4(3.0,-2.0,-1.0,2.0),skew);
            let scale=select(0.5,1.0/3.0,skew);
            let g=m.x*v.y+m.y*a.y+m.z*b.y;
            v.x=(g+m.w*a.x+b.x)*scale;
            v.z=(g+m.w*a.z+b.z)*scale;
        }
        dst[d*n+i]=v;
    }
}
fn spektrafilm_raw_no_contraction(v:f32)->f32 {
    return v;
}
// Metal uses the existing strict-math opt-in; Vulkan uses the RAW marker above.
fn spektrafilm_strict_float(v:f32)->f32 {
    return v;
}
