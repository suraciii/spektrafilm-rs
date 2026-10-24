// LibRaw0.22.2 fp_dng.cpp:537-558, then utils_libraw.cpp:486-548.
// One invocation owns LibRaw's bounded cblack metadata, never an image plane.
struct FloatMetadata {
    geometry:vec4<u32>, // filters, repeating rows, repeating columns, is_float
    levels:vec4<u32>, // global black, maximum, fixed maximum, auto WB
    channels:vec4<u32>,
    white:vec4<f32>,
    control:vec4<f32>, // fmaximum, adjust_maximum threshold, reserved
};
struct FloatScaleParams { white:vec4<f32>, maximum:u32, fixed_maximum:u32, threshold:f32, auto_white_balance:u32 };
@group(0) @binding(0) var<uniform> metadata:FloatMetadata;
@group(0) @binding(1) var<storage,read> source_repeat:array<u32>;
@group(0) @binding(2) var<storage,read_write> black:array<u32>;
@group(0) @binding(3) var<storage,read_write> repeat:array<u32>;
@group(0) @binding(4) var<storage,read_write> scale:FloatScaleParams;
@group(0) @binding(5) var<storage,read_write> conversion:RawFloatConversion;
fn native_fc(row:u32,col:u32)->u32 {
    return (metadata.geometry.x >> ((((row<<1u)&14u)|(col&1u))<<1u))&3u;
}
@compute @workgroup_size(1)
fn main(){
    let floating=metadata.geometry.w!=0u;
    let tmax=max(max(f32(max(metadata.levels.y,1u)),metadata.control.x),1.0);
    let rescale=floating&&(tmax<4096.0||tmax>32767.0);
    var multiplier=1.0;
    var maximum=metadata.levels.y;
    var global_black=metadata.levels.x;
    var channels=metadata.channels;
    let map_count=metadata.geometry.y*metadata.geometry.z;
    if(rescale){
        multiplier=16383.0/tmax;
        maximum=16383u;
        global_black=u32(f32(global_black)*multiplier);
        for(var c=0u;c<4u;c++){channels[c]=u32(f32(channels[c])*multiplier);}
    }
    for(var i=0u;i<map_count;i++){
        var value=source_repeat[i];
        if(rescale){value=u32(f32(value)*multiplier);}
        repeat[i]=value;
    }
    if(floating){
        let rows=metadata.geometry.y;let columns=metadata.geometry.z;
        if(metadata.geometry.x>1000u&&(rows+1u)/2u==1u&&(columns+1u)/2u==1u){
            var colors:array<u32,4>;
            var last_green=0u;var green_count=0u;
            for(var c=0u;c<4u;c++){
                colors[c]=native_fc(c/2u,c%2u);
                if(colors[c]==1u){green_count++;last_green=c;}
            }
            if(green_count>1u){colors[last_green]=3u;}
            for(var c=0u;c<4u;c++){
                channels[colors[c]]+=repeat[(c/2u%rows)*columns+c%2u%columns];
            }
            for(var i=0u;i<map_count;i++){repeat[i]=0u;}
        }else if(metadata.geometry.x<=1000u&&rows==1u&&columns==1u){
            for(var c=0u;c<4u;c++){channels[c]+=repeat[0];}
            repeat[0]=0u;
        }
        let common_offset=min(min(channels.x,channels.y),min(channels.z,channels.w));
        channels-=vec4(common_offset);global_black+=common_offset;
        if(map_count!=0u){
            var map_common=repeat[0];
            for(var i=1u;i<map_count;i++){map_common=min(map_common,repeat[i]);}
            for(var i=0u;i<map_count;i++){repeat[i]-=map_common;}
            global_black+=map_common;
        }
        channels+=vec4(global_black);
    }
    for(var c=0u;c<4u;c++){black[c]=channels[c];}
    scale.white=metadata.white;
    // Match the resolved processing control's nonnegative saturation range.
    scale.maximum=maximum-min(maximum,global_black);
    scale.fixed_maximum=metadata.levels.z;
    scale.threshold=metadata.control.y;
    scale.auto_white_balance=metadata.levels.w;
    conversion=RawFloatConversion(multiplier,metadata.geometry.w,global_black,maximum);
}
