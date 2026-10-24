// LibRaw remove_zeroes is an in-place row-major dependency, not a parallel
// neighborhood average. One device invocation preserves that scan exactly.
struct ZeroParams { black:vec4<u32>, filters:u32, count:u32, pad0:u32, pad1:u32 };
@group(0) @binding(0) var<uniform> frame:RawFrame;
@group(0) @binding(1) var<uniform> p:ZeroParams;
@group(0) @binding(2) var<storage,read_write> image:array<u32>;
@group(0) @binding(3) var<storage,read_write> maximum:array<atomic<u32>>;
@group(0) @binding(4) var<storage,read> black_map:array<u32>;
fn color(y:u32,x:u32)->u32 {return (p.filters >> ((((y<<1u)&14u)|(x&1u))<<1u))&3u;}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    if(id.x!=0u){return;}
    for(var y=0u;y<frame.height;y++){for(var x=0u;x<frame.width;x++){
        let channel=color(y,x);let i=(y*frame.width+x)*4u+channel;
        if(image[i]!=0u){continue;}
        var sum=0u;var n=0u;
        for(var r=i32(y)-2;r<=i32(y)+2;r++){for(var c=i32(x)-2;c<=i32(x)+2;c++){
            if(r<0||c<0||r>=i32(frame.height)||c>=i32(frame.width)){continue;}
            if(color(u32(r),u32(c))!=channel){continue;}
            let sample=image[(u32(r)*frame.width+u32(c))*4u+channel];
            if(sample!=0u){sum+=sample;n++;}
        }}
        if(n!=0u){image[i]=sum/n;}
    }}
    var dmax=0u;
    for(var i=0u;i<p.count;i++){
        let y=i/frame.width;let x=i%frame.width;
        var repeat=0u;
        if(frame.has_black_map!=0u){repeat=black_map[(y%frame.black_rows)*frame.black_columns+x%frame.black_columns];}
        for(var c=0u;c<4u;c++){
            let value=raw_clip(i32(image[i*4u+c])-i32(p.black[c])-i32(repeat));
            image[i*4u+c]=value;dmax=max(dmax,value);
        }
    }
    atomicStore(&maximum[0],dmax);
}
