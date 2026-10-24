struct Params { size:u32,n:u32,pad0:u32,pad1:u32,origin:vec4<u32>,extent:vec4<u32>,source_origin:vec4<u32>,source_extent:vec4<u32> }
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read> source:array<f64>;
@group(0) @binding(2) var<storage,read_write> lut:array<f64>;
@group(0) @binding(3) var<storage,read_write> sx:array<f64>;
@group(0) @binding(4) var<storage,read_write> sy:array<f64>;
@group(0) @binding(5) var<storage,read_write> sz:array<f64>;
@group(0) @binding(6) var<storage,read_write> cell_min:array<f64>;
@group(0) @binding(7) var<storage,read_write> cell_max:array<f64>;
fn sample(q:vec3<u32>,c:u32)->f64 {
    let v=q-p.source_origin.xyz;
    return source[((v.x*p.source_extent.y+v.y)*p.source_extent.z+v.z)*3u+c];
}
fn delta(q:vec3<u32>,axis:u32,pos:u32,c:u32)->f64 {
    var a=q;var b=q;a[axis]=pos;b[axis]=pos+1u;
    return sample(b,c)-sample(a,c);
}
fn slope(q:vec3<u32>,axis:u32,c:u32)->f64 {
    let pos=q[axis];
    if p.size==2u{return delta(q,axis,0u,c);}
    if pos==0u || pos==p.size-1u {
        var d0=delta(q,axis,0u,c);var d1=delta(q,axis,1u,c);
        if pos!=0u {d0=delta(q,axis,p.size-2u,c);d1=delta(q,axis,p.size-3u,c);}
        var v=f64(0.5)*(f64(3.0)*d0-d1);
        if v*d0<=f64(0.0){v=f64(0.0);}else if d0*d1<f64(0.0) && abs(v)>abs(f64(3.0)*d0){v=f64(3.0)*d0;}
        return v;
    }
    let dp=delta(q,axis,pos-1u,c);let dn=delta(q,axis,pos,c);
    if dp==f64(0.0)||dn==f64(0.0)||dp*dn<=f64(0.0){return f64(0.0);}
    return f64(2.0)*dp*dn/(dp+dn);
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
    let i=gid.x+gid.y*grid.x*256u;
    if i>=p.n{return;}
    let local=vec3<u32>(i/(p.extent.y*p.extent.z),(i/p.extent.z)%p.extent.y,i%p.extent.z);
    let q=p.origin.xyz+local;
    for(var c=0u;c<3u;c++) {
        lut[i*3u+c]=sample(q,c);sx[i*3u+c]=slope(q,0u,c);sy[i*3u+c]=slope(q,1u,c);sz[i*3u+c]=slope(q,2u,c);
        if all(local<p.extent.xyz-vec3<u32>(1u)) {
            var lo=sample(q,c);var hi=lo;
            for(var x=0u;x<2u;x++){for(var y=0u;y<2u;y++){for(var z=0u;z<2u;z++){
                let v=sample(q+vec3<u32>(x,y,z),c);if v<lo{lo=v;}else if v>hi{hi=v;}
            }}}
            let b=((local.x*(p.extent.y-1u)+local.y)*(p.extent.z-1u)+local.z)*3u+c;
            cell_min[b]=lo;cell_max[b]=hi;
        }
    }
}
