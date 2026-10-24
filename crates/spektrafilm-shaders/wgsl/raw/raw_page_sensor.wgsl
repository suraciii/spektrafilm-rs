struct SensorPage { region:vec4<u32>, control:vec4<u32> }; // first,count,sourcefirst,mode; filters,zero_bad,0,0
@group(0) @binding(0) var<uniform> frame:RawFrame;
@group(0) @binding(1) var<uniform> page:SensorPage;
@group(0) @binding(2) var<storage,read> input:array<f32>;
@group(0) @binding(3) var<storage,read> cfa:array<u32>;
@group(0) @binding(4) var<storage,read> black:array<u32>;
@group(0) @binding(5) var<storage,read_write> output:array<u32>;
@group(0) @binding(6) var<storage,read_write> maximum:array<atomic<u32>>;
@group(0) @binding(7) var<storage,read> repeat:array<u32>;
@group(0) @binding(8) var<storage,read> conversion:RawFloatConversion;
fn inverse_fuji(x:i32,y:i32)->vec2<i32>{
    let delta=y-i32(frame.fuji_width)+1;
    if(frame.fuji_layout!=0u){
        let row=x+delta;
        return vec2<i32>(x-((row+1)>>1),row);
    }
    let col=x-delta;
    return vec2<i32>(col,x-((col+1)>>1));
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>){
    let local=id.x+id.y*grid.x*256u;if(local>=page.region.y){return;}
    let index=page.region.x+local;let x=index%frame.width;let y=index/frame.width;
    var source=index;
    if(frame.fuji_width!=0u){
        let point=inverse_fuji(i32(x),i32(y));
        if(any(point<vec2<i32>(0))||point.x>=i32(frame.source_width)||point.y>=i32(frame.source_height)){return;}
        source=u32(point.y)*frame.source_width+u32(point.x);
    }
    if(source<page.region.z||source>=page.region.z+page.control.z){return;}
    let site=cfa[raw_cfa_index(x,y,frame)];
    for(var c=0u;c<frame.channels;c++){
        let channel=select(c,site,frame.channels==1u);
        var value=i32(raw_float_sample(input[(source-page.region.z)*frame.channels+c],conversion));
        if(page.control.y==0u){
            value-=i32(select(black[channel],black[channel]&65535u,frame.channels==1u));
            if(frame.channels>1u&&frame.has_black_map!=0u){value-=i32(repeat[(y%frame.black_rows)*frame.black_columns+x%frame.black_columns]);}
        }
        let result=raw_clip(value);output[local*4u+channel]=result;
        if(page.control.y==0u){atomicMax(&maximum[0],result);}
    }
}
