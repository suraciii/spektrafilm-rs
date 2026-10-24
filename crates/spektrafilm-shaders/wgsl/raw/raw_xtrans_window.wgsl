// Transport only: native X-Trans keeps full-image stride, CFA and tile phase.
struct RawWindow { x:u32, y:u32, width:u32, height:u32 };
@group(0) @binding(8) var<uniform> window:RawWindow;
fn image_index(p:i32)->u32 {
    return (u32(p/i32(frame.width))-window.y)*window.width+u32(p%i32(frame.width))-window.x;
}
