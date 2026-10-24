// Binary64 spectral conditioning and contraction; only the final runtime
// table narrows to f32. The Gaussian reflects edge samples (scipy reflect),
// whereas Mitchell sampling mirrors about the grid's endpoint coordinates.
struct Params {
    cells:u32, size:u32, wavelengths:u32, used:u32,
    first:u32, kernel_len:u32, reflectance:u32, window:u32,
    surface:u32, coordinates:u32, pad0:u32, pad1:u32,
}
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read> source:array<f32>;
@group(0) @binding(2) var<storage,read> sensitivity:array<f64>;
@group(0) @binding(3) var<storage,read> window:array<f64>;
@group(0) @binding(4) var<storage,read> coefficients:array<f64>;
@group(0) @binding(5) var<storage,read_write> scratch:array<f64>;
@group(0) @binding(6) var<storage,read_write> result64:array<f64>;
@group(0) @binding(7) var<storage,read_write> result32:array<f32>;

fn linear(gid:vec3<u32>, grid:vec3<u32>)->u32 {
    return gid.x+gid.y*grid.x*256u;
}
fn mitchell(t:f64)->f64 {
    let x=abs(t);
    let B=f64(1.0/3.0);
    let C=f64(1.0/3.0);
    if x<1.0 {
        return f64(1.0/6.0)*((12.0-9.0*B-6.0*C)*x*x*x
            +(-18.0+12.0*B+6.0*C)*x*x+(6.0-2.0*B));
    }
    if x<2.0 {
        return f64(1.0/6.0)*((f64(0.0)-B-6.0*C)*x*x*x+(6.0*B+30.0*C)*x*x
            +(-12.0*B-48.0*C)*x+(8.0*B+24.0*C));
    }
    return f64(0.0);
}
fn position(which:u32)->vec2<f64> {
    return clamp(vec2<f64>(coefficients[p.coordinates+which*2u],
        coefficients[p.coordinates+which*2u+1u]),vec2<f64>(0.0),vec2<f64>(f64(p.size-1u)));
}
fn base_position(which:u32)->vec2<u32> {
    return min(vec2<u32>(position(which)),vec2<u32>(p.size-2u));
}
fn sample_weight(which:u32, corner:u32)->f64 {
    let f=position(which)-vec2<f64>(base_position(which));
    return mitchell(f.x-f64(corner/4u)+1.0)*mitchell(f.y-f64(corner%4u)+1.0);
}
fn weight_total(which:u32)->f64 {
    var total:f64=0.0;
    for(var corner=0u;corner<16u;corner++){ total+=sample_weight(which,corner); }
    return total;
}
fn mirror(i:i32)->u32 {
    if i<0 { return u32(-i); }
    if i>=i32(p.size) { return u32(2*(i32(p.size)-1)-i); }
    return u32(i);
}
fn sampled_cell(which:u32,corner:u32)->u32 {
    let b=vec2<i32>(base_position(which));
    return mirror(b.x+i32(corner/4u)-1)*p.size+mirror(b.y+i32(corner%4u)-1);
}
fn blurred(cell:u32,wl:u32)->f64 {
    var value:f64=0.0;
    let n=i32(p.wavelengths);
    let radius=i32(p.kernel_len/2u);
    for(var k=0u;k<p.kernel_len;k++) {
        let q=i32(wl)+i32(k)-radius;
        let modulo=((q%(2*n))+2*n)%(2*n);
        let j=select(modulo,2*n-1-modulo,modulo>=n);
        value+=coefficients[p.wavelengths+k]*f64(source[cell*p.wavelengths+u32(j)]);
    }
    return value;
}
// Recursive-halving reduction with its tree held in device storage. Avoid
// by-value local arrays: dynamic indexing those arrays produces invalid
// pointer operands in the Vulkan SPIR-V lowering used by this backend.
fn normalization_term(wl:u32,c:u32,windowed:bool)->f64 {
    let si=sensitivity[wl*3u+c]*scratch[wl];
    if windowed { return si*window[wl*3u+c]; }
    return si;
}
fn pairwise(c:u32,windowed:bool)->f64 {
    if p.used==0u { return f64(0.0); }
    let tree=p.wavelengths+54u;
    for(var node=127i;node>=1i;node--) {
        let index=u32(node);
        var bit=1u;
        loop {
            if bit*2u>index { break; }
            bit*=2u;
        }
        var start=0u;
        var len=p.used;
        var valid=true;
        bit/=2u;
        loop {
            if bit==0u { break; }
            if len<=2u { valid=false; break; }
            let half=len/2u;
            if (index&bit)!=0u { start+=half; len-=half; }
            else { len=half; }
            bit/=2u;
        }
        if !valid { continue; }
        if len>2u { scratch[tree+index]=scratch[tree+2u*index]+scratch[tree+2u*index+1u]; }
        else if len==2u { scratch[tree+index]=normalization_term(start,c,windowed)+normalization_term(start+1u,c,windowed); }
        else { scratch[tree+index]=normalization_term(start,c,windowed); }
    }
    return scratch[tree+1u];
}
@compute @workgroup_size(256)
fn reference(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
    let wl=linear(gid,grid);
    if wl>=p.wavelengths { return; }
    var value=scratch[wl];
    for(var cell=0u;cell<p.cells;cell++) {
        value+=sample_weight(0u,p.first+cell)*blurred(cell,wl);
    }
    scratch[wl]=value;
}
@compute @workgroup_size(256)
fn normalize(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
    if linear(gid,grid)!=0u { return; }
    let total=weight_total(0u);
    for(var wl=0u;wl<p.wavelengths;wl++) {
        var value=scratch[wl];
        if total!=0.0 { value/=total; }
        if p.reflectance!=0u { value=f64(f32(value)); }
        scratch[wl]=value;
    }
    for(var c=0u;c<3u;c++) {
        // The two reductions reuse the bounded device-storage tree.
        var response:f64=0.0;
        for(var wl=0u;wl<p.used;wl++) {
            // Scene-neutral reflectance normalization keeps the reference order.
            response+=scratch[wl]*coefficients[wl]*sensitivity[wl*3u+c];
        }
        var factor:f64=1.0;
        let num=pairwise(c,true);
        let den=pairwise(c,false);
        if p.window!=0u && num>1e-10 && den>1e-10 { factor=num/den; }
        scratch[p.wavelengths+c]=factor;
        scratch[p.wavelengths+3u+c]=response;
    }
}
fn surface_factor(cell:u32,c:u32)->f64 {
    if p.surface==0u { return f64(1.0); }
    let step=f64(1.0)/f64(p.size-1u);
    let x=f64(cell/p.size)*step-coefficients[p.coordinates+4u];
    let y=f64(cell%p.size)*step-coefficients[p.coordinates+5u];
    let x2=x*x; let y2=y*y; let xy=x*y; let x3=x2*x; let y3=y2*y;
    let b=p.wavelengths+p.kernel_len+c*15u;
    let z=coefficients[b+1u]*x+coefficients[b+2u]*y
        +coefficients[b+3u]*x2+coefficients[b+4u]*y2+coefficients[b+5u]*xy
        +coefficients[b+6u]*x3+coefficients[b+7u]*y3
        +coefficients[b+8u]*(x2*y)+coefficients[b+9u]*(x*y2)
        +coefficients[b+10u]*(x2*x2)+coefficients[b+11u]*(y2*y2)
        +coefficients[b+12u]*(x3*y)+coefficients[b+13u]*(x2*y2)
        +coefficients[b+14u]*(x*y3);
    let bounded=z/sqrt(1.0+(z/2.0)*(z/2.0));
    return sf64_exp(bounded*0.693147180559945309417232121458176568);
}
@compute @workgroup_size(256)
fn contract(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
    let cell=linear(gid,grid);
    if cell>=p.cells { return; }
    var red=f64(0.0);var green=f64(0.0);var blue=f64(0.0);
    for(var wl=0u;wl<p.used;wl++) {
        var value=blurred(cell,wl);
        if p.reflectance!=0u { value*=coefficients[wl]; }
        var wr=sensitivity[wl*3u];
        var wg=sensitivity[wl*3u+1u];
        var wb=sensitivity[wl*3u+2u];
        if p.window!=0u {
            wr*=window[wl*3u]/scratch[p.wavelengths];
            wg*=window[wl*3u+1u]/scratch[p.wavelengths+1u];
            wb*=window[wl*3u+2u]/scratch[p.wavelengths+2u];
        }
        red+=value*wr;green+=value*wg;blue+=value*wb;
    }
    for(var c=0u;c<3u;c++) {
        var value=red;
        if c==1u { value=green; }
        if c==2u { value=blue; }
        if p.reflectance!=0u {
            let neutral=scratch[p.wavelengths+3u+c];
            if abs(neutral)>1e-15 { value/=neutral; }
        }
        value*=surface_factor(p.first+cell,c);
        result64[cell*3u+c]=value;
        result32[cell*3u+c]=f32(value);
    }
}
@compute @workgroup_size(256)
fn gather_neutral(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
    let i=linear(gid,grid);
    if i>=48u { return; }
    let cell=sampled_cell(1u,i/3u);
    if cell>=p.first && cell-p.first<p.cells {
        scratch[p.wavelengths+6u+i]=result64[(cell-p.first)*3u+i%3u];
    }
}
@compute @workgroup_size(256)
fn finish_neutral(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
    let c=linear(gid,grid);
    if c>=3u { return; }
    var value:f64=0.0;
    for(var corner=0u;corner<16u;corner++) {
        value+=sample_weight(1u,corner)*scratch[p.wavelengths+6u+corner*3u+c];
    }
    let total=weight_total(1u);
    if total!=0.0 { value/=total; }
    result64[c]=value;
}
