// darktable `markesteijn_green_minmax`, data-parallel: one invocation per pixel.
// Per non-green site, min/max of the green samples on its own hexagon and on the
// hexagon of the last non-green site along the four axis directions.
struct MinMaxParams { width:u32, height:u32, border:u32, cfa_rows:u32, cfa_columns:u32 };
@group(0) @binding(0) var<uniform> p:MinMaxParams;
@group(0) @binding(1) var<storage,read> rgb:array<f32>;
@group(0) @binding(2) var<storage,read> cfa:array<u32>;
@group(0) @binding(3) var<storage,read> allhex:array<vec2<i32>>;
@group(0) @binding(4) var<storage,read_write> gminmax:array<f32>;

fn fcn(y:i32,x:i32)->u32 {
    let ry=u32(((y % i32(p.cfa_rows)) + i32(p.cfa_rows)) % i32(p.cfa_rows));
    let rx=u32(((x % i32(p.cfa_columns)) + i32(p.cfa_columns)) % i32(p.cfa_columns));
    return cfa[ry*p.cfa_columns+rx];
}
fn green_at(x:i32,y:i32)->f32 {
    if(x<0 || y<0 || x>=i32(p.width) || y>=i32(p.height)){return 0.0;}
    return rgb[(u32(y)*p.width+u32(x))*4u+1u];
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
    if(fcn(y,x)==1u){return;}

    var gmin=3.402823466e38;
    var gmax=0.0;
    for(var k=0u;k<6u;k++){
        let o=hex_at(y,x,k);
        let v=green_at(x+o.x,y+o.y);
        gmin=min(gmin,v);
        gmax=max(gmax,v);
    }
    var last=vec2<i32>(0,0);
    var found=false;
    for(var n=0u;n<4u;n++){
        var dx=0; var dy=0;
        if(n==0u){dx=1;} else if(n==1u){dy=1;} else if(n==2u){dx=-1;} else {dy=-1;}
        if(fcn(y+dy,x+dx)==1u){continue;}
        last=vec2<i32>(x+dx,y+dy);
        found=true;
    }
    if(found){
        for(var k=0u;k<6u;k++){
            let o=hex_at(last.y,last.x,k);
            let v=green_at(last.x+o.x,last.y+o.y);
            gmin=min(gmin,v);
            gmax=max(gmax,v);
        }
    }
    gminmax[i*2u]=gmin;
    gminmax[i*2u+1u]=gmax;
}
