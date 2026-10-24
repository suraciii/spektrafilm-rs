// LibRaw 0.22.2 xtrans_demosaic.cpp: allhex and native green bounds scan.
struct DemosaicPass { colors:u32, stage:u32, top:u32, left:u32 };
@group(0) @binding(0) var<uniform> frame:RawFrame;
@group(0) @binding(1) var<uniform> settings:DemosaicPass;
@group(0) @binding(2) var<storage,read_write> image:array<u32>;
@group(0) @binding(3) var<storage,read> cfa:array<u32>;
// [0,144): allhex[3][3][2][8], [144,146): solitary green, [146]: invalid.
@group(0) @binding(4) var<storage,read_write> metadata:array<i32>;
const ORTH:array<i32,12> = array<i32,12>(1,0,0,1,-1,0,0,-1,1,0,0,1);
const PATT:array<i32,32> = array<i32,32>(0,1,0,-1,2,0,-1,0,1,1,1,-1,0,0,0,0,0,1,0,-2,1,0,-2,0,1,1,-2,-2,1,-1,-1,1);
fn color(y:i32,x:i32)->u32{return cfa[u32((y+6)%6)*6u+u32((x+6)%6)];}
fn hx(y:i32,x:i32,k:i32)->i32{return metadata[u32(((y%3)*3+x%3)*16+k)];}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>) {
    if(gid.x!=0u){return;}
    var counts:array<i32,4>;
    for(var y=0;y<6;y++){for(var x=0;x<6;x++){counts[color(y,x)]++;}}
    metadata[146]=i32(counts[0]<6 || counts[0]>10 || counts[1]<16 || counts[1]>24 || counts[2]<6 || counts[2]>10 || counts[3]!=0);
    for(var i=0u;i<144u;i++){metadata[i]=32700;}
    var minv=0;var maxv=0;var minh=0;var maxh=0;
    for(var y=0;y<3;y++){for(var x=0;x<3;x++){
        var ng=0;
        for(var d=0;d<10;d+=2){
            let g=i32(color(y,x)==1u);
            if(color(y+ORTH[u32(d)],x+ORTH[u32(d+2)])==1u){ng=0;}else{ng++;}
            if(ng==4){metadata[144]=y;metadata[145]=x;}
            if(ng==g+1){for(var c=0;c<8;c++){
                let v=ORTH[u32(d)]*PATT[u32(g*16+c*2)]+ORTH[u32(d+1)]*PATT[u32(g*16+c*2+1)];
                let h=ORTH[u32(d+2)]*PATT[u32(g*16+c*2)]+ORTH[u32(d+3)]*PATT[u32(g*16+c*2+1)];
                minv=min(v,minv);maxv=max(v,maxv);minh=min(h,minh);maxh=max(h,maxh);
                let k=u32((y*3+x)*16+(c^((g*2)&d)));
                // Native allhex stores short, including the frame-stride offset.
                metadata[k]=(h+v*i32(frame.width))<<16>>16;
                metadata[k+8u]=h+v*512;
            }}
        }
    }}
    for(var i=0u;i<144u;i++){if(metadata[i]>maxh+maxv*i32(frame.width)+1 || metadata[i]<minh+minv*i32(frame.width)-1){metadata[146]=1;}}
    if(metadata[146]!=0){return;}
    // Paged execution builds the same allhex first, then resumes the bounds
    // scan on device windows. No sensor values are needed for this stage.
    if(settings.stage==1u){return;}
    if(settings.stage!=2u){
    var retries=0u;
    var y=2;
    loop {
        if(y>=i32(frame.height)-2){break;}
        var x=2;var low=65535u;var high=0u;
        loop {
            if(x>=i32(frame.width)-2){break;}
            if(color(y,x)==1u){low=65535u;high=0u;x++;continue;}
            let p=y*i32(frame.width)+x;
            if(high==0u){for(var c=0;c<6;c++){let value=image[u32(p+hx(y,x,c))*4u+1u];low=min(low,value);high=max(high,value);}}
            image[u32(p)*4u+1u]=low;image[u32(p)*4u+3u]=high;
            switch((y-metadata[144])%3){
                case 1:{if(y<i32(frame.height)-3){y++;x--;}}
                case 2:{low=65535u;high=0u;x+=2;if(x<i32(frame.width)-3 && y>2){y--;retries++;if(retries>raw_pixel_count(frame)){metadata[146]=1;return;}}}
                default:{}
            }
            x++;
        }
        y++;
    }
    }
    for(var row=3;row<9 && row<i32(frame.height)-3;row++){
        for(var col=3;col<9 && col<i32(frame.width)-3;col++){
            if(color(row,col)==1u){continue;}
            for(var c=0;c<2;c++){
                let p=row*i32(frame.width)+col;let offset=3*hx(row,col,4+c);
                if(p+offset<0 || p+offset>=i32(raw_pixel_count(frame)) || p-offset<0 || p-offset>=i32(raw_pixel_count(frame))){metadata[146]=1;}
            }
        }
    }
}
