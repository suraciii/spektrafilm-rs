// Shared LibRaw0.22.2 scale_colors arithmetic for resident and paged RAW.
struct ScaleParams { white:vec4<f32>, maximum:u32, fixed_maximum:u32, threshold:f32, auto_white_balance:u32 };
// Native double sums contain exact integers. Limbs retain them without f64.
fn add64(a:vec2<u32>,b:vec2<u32>)->vec2<u32>{let low=a.x+b.x;return vec2(low,a.y+b.y+u32(low<a.x));}
fn sub64(a:vec2<u32>,b:vec2<u32>)->vec2<u32>{return vec2(a.x-b.x,a.y-b.y-u32(a.x<b.x));}
fn shl64(a:vec2<u32>)->vec2<u32>{return vec2(a.x<<1u,(a.y<<1u)|(a.x>>31u));}
fn less64(a:vec2<u32>,b:vec2<u32>)->bool{return a.y<b.y || (a.y==b.y && a.x<b.x);}
fn equal64(a:vec2<u32>,b:vec2<u32>)->bool{return all(a==b);}
fn nonzero64(a:vec2<u32>)->bool{return any(a!=vec2(0u));}
// Correctly round native double(count/sum), then its float cast. Converting
// both totals to f32 before division would discard native precision.
fn native_ratio(count:vec2<u32>,sum:vec2<u32>)->f32{
    if(!nonzero64(count)){return 0.0;}
    var numerator=count;var denominator=sum;var exponent=0;
    loop {if(!less64(numerator,denominator)){break;}numerator=shl64(numerator);exponent--;}
    loop {let twice=shl64(denominator);if(less64(numerator,twice)){break;}denominator=twice;exponent++;}
    var remainder=numerator;var significand=vec2(0u);
    for(var bit=0u;bit<53u;bit++){
        significand=shl64(significand);
        if(!less64(remainder,denominator)){remainder=sub64(remainder,denominator);significand.x|=1u;}
        remainder=shl64(remainder);
    }
    if(less64(denominator,remainder) || (equal64(denominator,remainder) && (significand.x&1u)!=0u)){
        significand=add64(significand,vec2(1u,0u));
    }
    var result=(significand.y<<3u)|(significand.x>>29u);
    let discarded=significand.x&0x1fffffffu;
    if(discarded>0x10000000u || (discarded==0x10000000u && (result&1u)!=0u)){result++;}
    return ldexp(f32(result),exponent-23);
}
fn raw_adjusted_maximum(p:ScaleParams,value:u32)->u32{
    var maximum=p.maximum;
    if(p.fixed_maximum==0u && value>0u && value<maximum && f32(value)>f32(maximum)*p.threshold){maximum=value;}
    return maximum;
}
fn raw_resolve_scales(input_white:vec4<f32>,maximum:u32,colors:u32)->vec4<f32>{
    var white=input_white;
    // Native defaults follow auto/camera selection, not precede it.
    if(white[1]==0.0){white[1]=1.0;}
    if(white[3]==0.0){white[3]=select(1.0,white[1],colors<4u);}
    var dmin=white.x;
    for(var c=1u;c<4u;c++){if(white[c]<dmin){dmin=white[c];}}
    var result=vec4<f32>(1.0);
    for(var c=0u;c<4u;c++){
        // Native dmax is double for min/max only; arithmetic casts to f32.
        if(dmin>0.00001 && maximum!=0u){result[c]=(white[c]/dmin)*65535.0/f32(maximum);}
    }
    return result;
}
