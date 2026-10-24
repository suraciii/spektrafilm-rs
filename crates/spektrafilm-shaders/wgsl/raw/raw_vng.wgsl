// LibRaw 0.22.2 misc_demosaic.cpp: VNG's 64 native gradient terms.
const TERMS:array<i32,384> = array<i32,384>(
    -2, -2, 0, -1, 0, 1,
    -2, -2, 0, 0, 1, 1,
    -2, -1, -1, 0, 0, 1,
    -2, -1, 0, -1, 0, 2,
    -2, -1, 0, 0, 0, 3,
    -2, -1, 0, 1, 1, 1,
    -2, 0, 0, -1, 0, 6,
    -2, 0, 0, 0, 1, 2,
    -2, 0, 0, 1, 0, 3,
    -2, 1, -1, 0, 0, 4,
    -2, 1, 0, -1, 1, 4,
    -2, 1, 0, 0, 0, 6,
    -2, 1, 0, 1, 0, 2,
    -2, 2, 0, 0, 1, 4,
    -2, 2, 0, 1, 0, 4,
    -1, -2, -1, 0, 0, -128,
    -1, -2, 0, -1, 0, 1,
    -1, -2, 1, -1, 0, 1,
    -1, -2, 1, 0, 1, 1,
    -1, -1, -1, 1, 0, -120,
    -1, -1, 1, -2, 0, 64,
    -1, -1, 1, -1, 0, 34,
    -1, -1, 1, 0, 0, 51,
    -1, -1, 1, 1, 1, 17,
    -1, 0, -1, 2, 0, 8,
    -1, 0, 0, -1, 0, 68,
    -1, 0, 0, 1, 0, 17,
    -1, 0, 1, -2, 1, 64,
    -1, 0, 1, -1, 0, 102,
    -1, 0, 1, 0, 1, 34,
    -1, 0, 1, 1, 0, 51,
    -1, 0, 1, 2, 1, 16,
    -1, 1, 1, -1, 1, 68,
    -1, 1, 1, 0, 0, 102,
    -1, 1, 1, 1, 0, 34,
    -1, 1, 1, 2, 0, 16,
    -1, 2, 0, 1, 0, 4,
    -1, 2, 1, 0, 1, 4,
    -1, 2, 1, 1, 0, 4,
    0, -2, 0, 0, 1, -128,
    0, -1, 0, 1, 1, -120,
    0, -1, 1, -2, 0, 64,
    0, -1, 1, 0, 0, 17,
    0, -1, 2, -2, 0, 64,
    0, -1, 2, -1, 0, 32,
    0, -1, 2, 0, 0, 48,
    0, -1, 2, 1, 1, 16,
    0, 0, 0, 2, 1, 8,
    0, 0, 2, -2, 1, 64,
    0, 0, 2, -1, 0, 96,
    0, 0, 2, 0, 1, 32,
    0, 0, 2, 1, 0, 48,
    0, 0, 2, 2, 1, 16,
    0, 1, 1, 0, 0, 68,
    0, 1, 1, 2, 0, 16,
    0, 1, 2, -1, 1, 64,
    0, 1, 2, 0, 0, 96,
    0, 1, 2, 1, 0, 32,
    0, 1, 2, 2, 0, 16,
    1, -2, 1, 0, 0, -128,
    1, -1, 1, 1, 0, -120,
    1, 0, 1, 2, 0, 8,
    1, 0, 2, -1, 0, 64,
    1, 0, 2, 1, 0, 16);
const CHOOD:array<vec2<i32>,8> = array<vec2<i32>,8>(vec2(-1,-1),vec2(-1,0),vec2(-1,1),vec2(0,1),vec2(1,1),vec2(1,0),vec2(1,-1),vec2(0,-1));
struct DemosaicPass { colors:u32, stage:u32, top:u32, left:u32 };
@group(0) @binding(0) var<uniform> frame:RawFrame;
@group(0) @binding(1) var<uniform> settings:DemosaicPass;
@group(0) @binding(2) var<storage,read_write> image:array<u32>;
@group(0) @binding(3) var<storage,read> cfa:array<u32>;
@group(0) @binding(4) var<storage,read_write> result:array<u32>;
fn color(y:i32,x:i32)->u32 {
    let yy=u32((y%i32(frame.cfa_rows)+i32(frame.cfa_rows))%i32(frame.cfa_rows));
    let xx=u32((x%i32(frame.cfa_columns)+i32(frame.cfa_columns))%i32(frame.cfa_columns));
    return raw_color_from_code(cfa[yy*frame.cfa_columns+xx],frame);
}
fn sample(y:i32,x:i32,c:u32)->i32 {return i32(image[(u32(y)*frame.width+u32(x))*4u+c]);}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>, @builtin(num_workgroups) grid:vec3<u32>) {
    let p=gid.x+gid.y*grid.x*256u; if(p>=raw_pixel_count(frame)){return;}
    let y=i32(p/frame.width); let x=i32(p%frame.width);
    if(x<2 || y<2 || x+2>=i32(frame.width) || y+2>=i32(frame.height)){return;}
    if(settings.stage==1u){for(var c=0u;c<4u;c++){image[p*4u+c]=result[p*4u+c];}return;}
    var gradients:array<i32,8>;
    for(var t=0u;t<64u;t++) {
        let k=t*6u; let y1=TERMS[k];let x1=TERMS[k+1u];let y2=TERMS[k+2u];let x2=TERMS[k+3u];
        let c=color(y+y1+144,x+x1+144);
        if(color(y+y2+144,x+x2+144)!=c){continue;}
        let diag=select(1,2,color(y,x+1)==c && color(y+1,x)==c);
        if(abs(y1-y2)==diag && abs(x1-x2)==diag){continue;}
        let diff=abs(sample(y+y1,x+x1,c)-sample(y+y2,x+x2,c))<<u32(TERMS[k+4u]);
        for(var g=0u;g<8u;g++){if((u32(TERMS[k+5u])&(1u<<g))!=0u){gradients[g]+=diff;}}
    }
    var lo=gradients[0];var hi=lo;
    for(var g=1u;g<8u;g++){lo=min(lo,gradients[g]);hi=max(hi,gradients[g]);}
    for(var c=0u;c<4u;c++){result[p*4u+c]=image[p*4u+c];}
    if(hi==0){return;}
    let threshold=lo+(hi>>1);let f=color(y,x);var sums:array<i32,4>;var n=0;
    for(var g=0u;g<8u;g++) {
        if(gradients[g]>threshold){continue;}
        let dy=CHOOD[g].x;let dx=CHOOD[g].y;
        let second=color(y+dy+144,x+dx+144)!=f && color(y+2*dy+144,x+2*dx+144)==f;
        for(var c=0u;c<settings.colors;c++){
            if(c==f && second){sums[c]+=(sample(y,x,c)+sample(y+2*dy,x+2*dx,c))>>1;}
            else{sums[c]+=sample(y+dy,x+dx,c);}
        }
        n++;
    }
    for(var c=0u;c<settings.colors;c++){
        var value=sample(y,x,f);if(c!=f){value+=(sums[c]-sums[f])/n;}
        result[p*4u+c]=raw_clip(value);
    }
}
