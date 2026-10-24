// LibRaw0.22.2 src/decoders/fp_dng.cpp:537-563. The conversion record is
// device-produced; no sample or normalization coefficient returns to the host.
struct RawFloatConversion {
    multiplier:f32,
    is_float:u32,
    black:u32,
    maximum:u32,
};
fn raw_float_sample(value:f32, conversion:RawFloatConversion)->u32 {
    if(conversion.is_float==0u){return u32(value);}
    // Native casts the nonnegative product to ushort, rather than rounding or
    // saturating it. Valid decoded values are bounded by fmaximum.
    return u32(max(value,0.0)*conversion.multiplier)&65535u;
}
