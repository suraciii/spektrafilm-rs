// darktable `markesteijn_initial_copy`, data-parallel: one invocation per pixel.
// Keeps the CFA sample, zeroes the other channels, clamps negatives. The working
// layout is the native one: four interleaved floats per pixel (R,G,B,aux).
struct InitParams { width:u32, height:u32, cfa_rows:u32, cfa_columns:u32 };
@group(0) @binding(0) var<uniform> p:InitParams;
@group(0) @binding(1) var<storage,read> image:array<u32>;
@group(0) @binding(2) var<storage,read> cfa:array<u32>;
@group(0) @binding(3) var<storage,read_write> rgb:array<f32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>){
    let i=gid.x;
    if(i>=p.width*p.height){return;}
    let x=i%p.width;
    let y=i/p.width;
    let f=cfa[(y%p.cfa_rows)*p.cfa_columns+(x%p.cfa_columns)];
    let v=max(f32(image[i*4u+f]),0.0);
    for(var c=0u;c<3u;c++){
        rgb[i*4u+c]=select(0.0,v,c==f);
    }
}
