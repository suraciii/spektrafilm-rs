// darktable `markesteijn_interpolate_green`, data-parallel: one invocation per pixel.
// Four directional green estimates per non-green site. Reads come from a single
// source buffer (native stages rgb_0 into local memory before writing), writes go
// to the four direction buffers `c ^ flip`.
struct GP { width:u32, height:u32, border:u32, cfa_rows:u32, cfa_columns:u32, sgx:i32, sgy:i32, pad:u32 };
@group(0) @binding(0) var<uniform> p:GP;
@group(0) @binding(1) var<storage,read> src:array<f32>;
@group(0) @binding(2) var<storage,read> cfa:array<u32>;
@group(0) @binding(3) var<storage,read> allhex:array<vec2<i32>>;
@group(0) @binding(4) var<storage,read> gminmax:array<f32>;
@group(0) @binding(5) var<storage,read_write> o0:array<f32>;
@group(0) @binding(6) var<storage,read_write> o1:array<f32>;
@group(0) @binding(7) var<storage,read_write> o2:array<f32>;
@group(0) @binding(8) var<storage,read_write> o3:array<f32>;

fn fcn(y:i32,x:i32)->u32 {
    let ry=u32(((y % i32(p.cfa_rows)) + i32(p.cfa_rows)) % i32(p.cfa_rows));
    let rx=u32(((x % i32(p.cfa_columns)) + i32(p.cfa_columns)) % i32(p.cfa_columns));
    return cfa[ry*p.cfa_columns+rx];
}
fn at(x:i32,y:i32,c:u32)->f32 {
    if(x<0 || y<0 || x>=i32(p.width) || y>=i32(p.height)){return 0.0;}
    return src[(u32(y)*p.width+u32(x))*4u+c];
}
fn hex_at(row:i32,col:i32,k:u32)->vec2<i32> {
    let idx=(u32(((row%3)+3)%3)*3u+u32(((col%3)+3)%3))*8u+k;
    return allhex[idx];
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>){
    let i=gid.x;
    if(i>=p.width*p.height){return;}
    let x=i32(i%p.width);
    let y=i32(i/p.width);
    let b=i32(p.border);
    if(x<b || y<b || x>=i32(p.width)-b || y>=i32(p.height)-b){return;}
    let f=fcn(y,x);
    if(f==1u){return;}

    let base=i*2u;
    let gmin=gminmax[base];
    let gmax=gminmax[base+1u];
    let h0=hex_at(y,x,0u); let h1=hex_at(y,x,1u); let h2=hex_at(y,x,2u);
    let h3=hex_at(y,x,3u); let h4=hex_at(y,x,4u); let h5=hex_at(y,x,5u);
    let vf=at(x,y,f);

    var color:array<f32,4>;
    color[0]=0.6796875*(at(x+h1.x,y+h1.y,1u)+at(x+h0.x,y+h0.y,1u))
           - 0.1796875*(at(x+2*h1.x,y+2*h1.y,1u)+at(x+2*h0.x,y+2*h0.y,1u));
    color[1]=0.87109375*at(x+h3.x,y+h3.y,1u)+0.13*at(x+h2.x,y+h2.y,1u)
           + 0.359375*(vf-at(x-h2.x,y-h2.y,f));
    for(var c=0u;c<2u;c++){
        let h=select(h4,h5,c==1u);
        color[2u+c]=0.640625*at(x+h.x,y+h.y,1u)+0.359375*at(x-2*h.x,y-2*h.y,1u)
            +0.12890625*(2.0*vf-at(x+3*h.x,y+3*h.y,f)-at(x-3*h.x,y-3*h.y,f));
    }

    let flip=select(0u,1u,((y-p.sgy)%3)==0);
    let gi=i*4u+1u;
    if(flip==0u){
        o0[gi]=clamp(color[0],gmin,gmax); o1[gi]=clamp(color[1],gmin,gmax);
        o2[gi]=clamp(color[2],gmin,gmax); o3[gi]=clamp(color[3],gmin,gmax);
    } else {
        o1[gi]=clamp(color[0],gmin,gmax); o0[gi]=clamp(color[1],gmin,gmax);
        o3[gi]=clamp(color[2],gmin,gmax); o2[gi]=clamp(color[3],gmin,gmax);
    }
}
