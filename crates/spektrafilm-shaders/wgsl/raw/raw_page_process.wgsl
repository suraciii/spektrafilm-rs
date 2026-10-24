struct PageProcess { region:vec4<u32>, control:vec4<u32> }; // first,count,windowwidth,windowheight; mode,filters,globalx,globaly
@group(0) @binding(0) var<uniform> frame:RawFrame;
@group(0) @binding(1) var<uniform> page:PageProcess;
@group(0) @binding(2) var<storage,read_write> image:array<u32>;
@group(0) @binding(3) var<storage,read> scale:array<f32>;
@group(0) @binding(4) var<storage,read> black:array<u32>;
@group(0) @binding(5) var<storage,read> repeat:array<u32>;
@group(0) @binding(6) var<storage,read_write> maximum:array<atomic<u32>>;
fn color(y:u32,x:u32)->u32{return (page.control.y >> ((((y<<1u)&14u)|(x&1u))<<1u))&3u;}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>){
    let local=id.x+id.y*grid.x*256u;
    if(page.control.x==0u){
        if(local!=0u){return;}
        let row=page.region.x;let begin=page.region.y;let end=page.region.z;
        for(var x=begin;x<end;x++){
            let ch=color(page.control.w+row,page.control.z+x);let i=(row*page.region.w+x)*4u+ch;
            if(image[i]!=0u){continue;}
            var sum=0u;var n=0u;
            for(var r=i32(row)-2;r<=i32(row)+2;r++){for(var c=i32(x)-2;c<=i32(x)+2;c++){
                if(r<0||c<0||r>=i32(arrayLength(&image)/(4u*page.region.w))||c>=i32(page.region.w)){continue;}
                if(color(page.control.w+u32(r),page.control.z+u32(c))!=ch){continue;}
                let v=image[(u32(r)*page.region.w+u32(c))*4u+ch];if(v!=0u){sum+=v;n++;}
            }}
            if(n!=0u){image[i]=sum/n;}
        }return;
    }
    if(local>=page.region.y){return;}
    let index=page.region.x+local;let x=index%frame.width;let y=index/frame.width;
    var offset=0u;if(frame.has_black_map!=0u){offset=repeat[(y%frame.black_rows)*frame.black_columns+x%frame.black_columns];}
    for(var c=0u;c<4u;c++){
        var value=i32(image[local*4u+c]);
        if(page.control.x==1u){value-=i32(black[c])+i32(offset);image[local*4u+c]=raw_clip(value);atomicMax(&maximum[0],raw_clip(value));}
        else{
            if((frame.mix_green&2u)!=0u&&value>32767){value=0;}
            if(frame.subtract_black_map==0u&&frame.has_black_map!=0u){if(value!=0){value=i32(f32(value-i32(offset))*scale[c]);}}
            else{value=i32(f32(value)*scale[c]);}
            image[local*4u+c]=raw_clip(value);
        }
    }
    if(page.control.x==2u&&raw_is_merge_site(x,y,frame)){image[local*4u+1u]=image[local*4u+3u];}
}
