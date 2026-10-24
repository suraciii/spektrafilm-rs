// Exact native first-green predictions with sparse, signed-short source reads.
struct PredictionPass { top:u32,left:u32,first:u32,count:u32,stage:u32,_a:u32,_b:u32,_c:u32 };
@group(0) @binding(0) var<uniform> frame:RawFrame;
@group(0) @binding(1) var<uniform> settings:PredictionPass;
@group(0) @binding(2) var<storage,read> metadata:array<i32>;
@group(0) @binding(3) var<storage,read> cfa:array<u32>;
@group(0) @binding(4) var<storage,read_write> indices:array<u32>;
@group(0) @binding(5) var<storage,read_write> bounds:array<atomic<u32>>;
@group(0) @binding(6) var<storage,read_write> values:array<vec4<u32>>;
@group(0) @binding(7) var<storage,read_write> scratch:array<u32>;
fn color(y:u32,x:u32)->u32{return cfa[(y%6u)*6u+x%6u];}
fn hex(y:u32,x:u32,k:u32)->i32{return metadata[((y%3u)*3u+x%3u)*16u+k];}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>){
    let i=gid.x+gid.y*grid.x*256u;if(i>=settings.count){return;}
    let p=settings.first+i;let y=settings.top+p/512u;let x=settings.left+p%512u;
    let valid_pixel=y<min(settings.top+512u,frame.height-3u) && x<min(settings.left+512u,frame.width-3u);
    let f=color(y,x);
    if(settings.stage==0u){
        var offsets:array<i32,16>;
        let a=hex(y,x,0u);let b=hex(y,x,1u);let c=hex(y,x,2u);let d=hex(y,x,3u);let e=hex(y,x,4u);let g=hex(y,x,5u);
        offsets=array<i32,16>(0,b,a,2*b,2*a,d,c,-c,e,-2*e,3*e,-3*e,g,-2*g,3*g,-3*g);
        for(var k=0u;k<16u;k++){
            var q=0xffffffffu;
            if(valid_pixel && f!=1u){q=u32(i32(y*frame.width+x)+offsets[k]);atomicMin(&bounds[0],q);atomicMax(&bounds[1],q);}
            indices[i*16u+k]=q;
        }
        return;
    }
    if(!valid_pixel || f==1u){return;}
    var samples:array<vec4<i32>,16>;for(var k=0u;k<16u;k++){samples[k]=vec4<i32>(values[i*16u+k]);}
    var guesses:array<i32,4>;
    guesses[0]=174*(samples[1][1]+samples[2][1])-46*(samples[3][1]+samples[4][1]);
    guesses[1]=223*samples[5][1]+33*samples[6][1]+92*(samples[0][f]-samples[7][f]);
    for(var c=0u;c<2u;c++){let k=8u+c*4u;guesses[2u+c]=164*samples[k][1]+92*samples[k+1u][1]+33*(2*samples[0][f]-samples[k+2u][f]-samples[k+3u][f]);}
    for(var c=0u;c<4u;c++){let d=c^u32((i32(y)-metadata[144])%3==0);scratch[(d*262144u+p)*3u+1u]=u32(max(samples[0][1],min(guesses[c]>>8,samples[0][3])))&65535u;}
}
